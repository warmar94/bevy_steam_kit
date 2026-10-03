//! Features together: one pump per frame, every enabled feature's events delivered in the frame
//! they were pumped, strict ambiguity detection on every main schedule. Each test needs two or
//! more features and is compiled only when they are enabled.

#![cfg(any(
    all(feature = "lobby", feature = "stats"),
    all(feature = "lobby", feature = "leaderboards"),
    all(feature = "stats", feature = "leaderboards"),
    all(feature = "lobby", feature = "friends"),
    all(feature = "lobby", feature = "auth"),
    all(feature = "stats", feature = "friends"),
    all(feature = "friends", feature = "leaderboards"),
    all(feature = "friends", feature = "overlay")
))]

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy_steam_kit::*;

/// What the game read in `PreUpdate`, with the (1-based) frame.
#[derive(Resource, Default)]
struct ReadIn(Vec<(u32, &'static str)>);

fn strict(app: &mut App) {
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
}

fn app(fake: &FakeSteamBackend) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default())).insert_resource(SteamBackendRes(Box::new(fake.clone()))).init_resource::<ReadIn>();
    strict(&mut app);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

#[allow(dead_code)] // not every feature pair reads it
fn read(app: &App) -> Vec<(u32, &'static str)> {
    app.world().resource::<ReadIn>().0.clone()
}

#[cfg(feature = "lobby")]
#[allow(dead_code)] // not every feature pair reads it
fn read_lobby(mut frame: Local<u32>, mut joins: MessageReader<JoinRequested>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in joins.read() {
        seen.0.push((*frame, "join requested"));
    }
}

#[cfg(feature = "stats")]
#[allow(dead_code)] // not every feature pair reads it
fn read_stats(mut frame: Local<u32>, mut stored: MessageReader<StatsStored>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in stored.read() {
        seen.0.push((*frame, "stats stored"));
    }
}

#[cfg(feature = "leaderboards")]
#[allow(dead_code)] // not every feature pair reads it
fn read_boards(mut frame: Local<u32>, mut errors: MessageReader<LeaderboardError>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in errors.read() {
        seen.0.push((*frame, "leaderboard error"));
    }
}

#[cfg(all(feature = "lobby", feature = "stats"))]
#[test]
fn lobby_and_stats_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, (read_lobby, read_stats).chain());
    frames(&mut app, 2);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 12, from: 76_561_197_960_265_730 });
    fake.push_event(BackendEvent::StatsStored);
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 5);
    assert_eq!(read(&app), vec![(3, "join requested"), (3, "stats stored")]);
}

#[cfg(all(feature = "lobby", feature = "leaderboards"))]
#[test]
fn lobby_and_leaderboards_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, (read_lobby, read_boards).chain());
    frames(&mut app, 2);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 12, from: 76_561_197_960_265_730 });
    app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(1), "Nope"));
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 5);
    // The join request is pumped in frame 3; the find starts in frame 3 and its answer is pumped in 4.
    assert_eq!(read(&app), vec![(3, "join requested"), (4, "leaderboard error")]);
}

#[cfg(all(feature = "stats", feature = "leaderboards"))]
#[test]
fn stats_and_leaderboards_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, (read_stats, read_boards).chain());
    frames(&mut app, 2);
    fake.push_event(BackendEvent::StatsStored);
    app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(1), "Nope"));
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 5);
    assert_eq!(read(&app), vec![(3, "stats stored"), (4, "leaderboard error")]);
}

#[cfg(all(feature = "lobby", feature = "stats", feature = "leaderboards"))]
#[test]
fn every_feature_shares_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        SteamKitPlugin::default().with_lobby(LobbySettings::default()).with_stats(StatsSettings::default()).with_leaderboards(LeaderboardSettings::default()),
    ))
    .insert_resource(SteamBackendRes(Box::new(fake.clone())))
    .init_resource::<ReadIn>()
    .add_systems(PreUpdate, (read_lobby, read_stats, read_boards).chain());
    strict(&mut app);
    frames(&mut app, 2);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 12, from: 76_561_197_960_265_730 });
    fake.push_event(BackendEvent::StatsStored);
    app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(1), "Nope"));
    frames(&mut app, 4);
    assert_eq!(fake.pump_count(), 6);
    assert_eq!(read(&app), vec![(3, "join requested"), (3, "stats stored"), (4, "leaderboard error")]);
}

#[cfg(feature = "friends")]
#[allow(dead_code)] // not every feature pair reads it
fn read_connects(mut frame: Local<u32>, mut connects: MessageReader<ConnectRequested>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in connects.read() {
        seen.0.push((*frame, "connect requested"));
    }
}

/// One "Join Game" on a `+connect_lobby` string: the lobby reports a `JoinRequested`, friends the
/// raw `ConnectRequested`, in the same frame; the shared `FakeCall::InviteToGame` serves both.
#[cfg(all(feature = "lobby", feature = "friends"))]
#[test]
fn lobby_and_friends_both_report_a_rich_presence_join() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, (read_lobby, read_connects).chain());
    frames(&mut app, 2);
    fake.push_rich_presence_join(76_561_197_960_265_730, "+connect_lobby 7");
    frames(&mut app, 2);
    assert_eq!(fake.pump_count(), 4);
    assert_eq!(read(&app), vec![(3, "join requested"), (3, "connect requested")]);

    app.world_mut().write_message(InviteToGame { steam_id: 76_561_197_960_265_730, connect: "+connect 1.2.3.4".into() });
    frames(&mut app, 1);
    assert!(fake.calls().contains(&FakeCall::InviteToGame { friend: 76_561_197_960_265_730, connect: "+connect 1.2.3.4".into() }));
}

#[cfg(all(feature = "lobby", feature = "auth"))]
#[test]
fn lobby_and_auth_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, read_lobby);
    frames(&mut app, 1);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 12, from: 76_561_197_960_265_730 });
    app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(1), "svc"));
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 4);
    assert_eq!(read(&app), vec![(2, "join requested")]);
    assert_eq!(app.world().resource::<SteamAuth>().live_tickets(), 1);
}

#[cfg(all(feature = "stats", feature = "friends"))]
#[test]
fn stats_and_friends_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, (read_stats, read_connects).chain());
    frames(&mut app, 2);
    fake.push_event(BackendEvent::StatsStored);
    fake.push_event(BackendEvent::ConnectRequested { from: 0, connect: "x".into() });
    frames(&mut app, 1);
    assert_eq!(read(&app), vec![(3, "stats stored"), (3, "connect requested")]);
}

/// The leaderboards fake's `set_friends` and the friends fake's knobs live side by side (no name
/// collision with both features and `use bevy_steam_kit::*`).
#[cfg(all(feature = "friends", feature = "leaderboards"))]
#[test]
fn friends_and_leaderboards_fake_knobs_do_not_collide() {
    let fake = FakeSteamBackend::new();
    fake.set_friends(&[76_561_197_960_265_730]);
    fake.add_friend(76_561_197_960_265_731, PersonaState::Online);
    let mut app = app(&fake);
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<SteamFriends>().list().len(), 1);
}

#[cfg(all(feature = "friends", feature = "overlay"))]
#[test]
fn friends_and_overlay_share_one_pump() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    app.add_systems(PreUpdate, read_connects);
    frames(&mut app, 1);
    fake.toggle_overlay(true);
    fake.push_event(BackendEvent::ConnectRequested { from: 0, connect: "x".into() });
    frames(&mut app, 1);
    assert!(app.world().resource::<SteamOverlay>().is_active());
    assert_eq!(read(&app), vec![(2, "connect requested")]);
}
