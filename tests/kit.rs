//! Features together: one pump per frame, every enabled feature's events delivered in the frame
//! they were pumped, strict ambiguity detection on every main schedule. Each test needs two or
//! more features and is compiled only when they are enabled.

#![cfg(any(all(feature = "lobby", feature = "stats"), all(feature = "lobby", feature = "leaderboards"), all(feature = "stats", feature = "leaderboards")))]

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

fn read(app: &App) -> Vec<(u32, &'static str)> {
    app.world().resource::<ReadIn>().0.clone()
}

#[cfg(feature = "lobby")]
fn read_lobby(mut frame: Local<u32>, mut joins: MessageReader<JoinRequested>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in joins.read() {
        seen.0.push((*frame, "join requested"));
    }
}

#[cfg(feature = "stats")]
fn read_stats(mut frame: Local<u32>, mut stored: MessageReader<StatsStored>, mut seen: ResMut<ReadIn>) {
    *frame += 1;
    for _ in stored.read() {
        seen.0.push((*frame, "stats stored"));
    }
}

#[cfg(feature = "leaderboards")]
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
