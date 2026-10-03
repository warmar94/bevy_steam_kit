//! Steam quitting while the game runs, from a game's point of view: the kit in a strict headless
//! app (ambiguity detection = Error on every main schedule) with the in-memory `FakeSteamBackend`.
//! The game keeps running, reads `SteamLost` once, and every request (waiting or new) gets an
//! answer. Runs with every feature set; each feature's part is compiled when it is enabled.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_steam_kit::*;

/// What the game saw.
#[derive(Resource, Default)]
struct Seen {
    lost: Vec<SteamLostReason>,
    /// Errors of any feature, as `"<feature> <kind>"`.
    errors: Vec<String>,
}

/// The game's error readers (in `Last`, after the kit; their order does not matter).
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct Readers;

fn strict(app: &mut App) {
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
}

/// Like the README's examples, the game takes the backend as `Res`: it keeps running after Steam
/// is lost, because the kit keeps the resource (with an inert backend).
fn read_lost(mut lost: MessageReader<SteamLost>, mut seen: ResMut<Seen>, backend: Res<SteamBackendRes>) {
    for l in lost.read() {
        seen.lost.push(l.reason);
        let _ = backend.0.friend_name(backend.0.local_id());
    }
}

#[cfg(feature = "auth")]
fn read_auth(mut errors: MessageReader<AuthError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("auth {:?}", e.kind)));
}

#[cfg(feature = "leaderboards")]
fn read_boards(mut errors: MessageReader<LeaderboardError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("leaderboards {:?}", e.kind)));
}

#[cfg(feature = "lobby")]
fn read_lobby(mut errors: MessageReader<LobbyError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("lobby {:?}", e.kind)));
}

#[cfg(feature = "stats")]
fn read_stats(mut errors: MessageReader<StatsError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("stats {:?}", e.kind)));
}

#[cfg(feature = "overlay")]
fn read_overlay(mut errors: MessageReader<OverlayError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("overlay {:?}", e.kind)));
}

#[cfg(feature = "friends")]
fn read_friends(mut errors: MessageReader<FriendsError>, mut seen: ResMut<Seen>) {
    seen.errors.extend(errors.read().map(|e| format!("friends {:?}", e.kind)));
}

fn app(fake: &FakeSteamBackend) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<Seen>()
        .configure_sets(Last, Readers.after(SteamKitSystems::Requests))
        .add_systems(Update, read_lost.before(SteamKitSystems::Requests));
    #[cfg(feature = "auth")]
    app.add_systems(Last, read_auth.in_set(Readers).ambiguous_with(Readers));
    #[cfg(feature = "leaderboards")]
    app.add_systems(Last, read_boards.in_set(Readers).ambiguous_with(Readers));
    #[cfg(feature = "lobby")]
    app.add_systems(Last, read_lobby.in_set(Readers).ambiguous_with(Readers));
    #[cfg(feature = "stats")]
    app.add_systems(Last, read_stats.in_set(Readers).ambiguous_with(Readers));
    #[cfg(feature = "overlay")]
    app.add_systems(Last, read_overlay.in_set(Readers).ambiguous_with(Readers));
    #[cfg(feature = "friends")]
    app.add_systems(Last, read_friends.in_set(Readers).ambiguous_with(Readers));
    strict(&mut app);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

#[test]
fn the_game_keeps_running_after_steam_quits_and_every_request_is_answered() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    frames(&mut app, 2);

    // Requests Steam has not answered when it quits.
    #[cfg(feature = "auth")]
    {
        fake.fail_next_auth_ticket(FakeAuthFailure::NoAnswer);
        app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(1), "game-server"));
    }
    #[cfg(feature = "leaderboards")]
    {
        fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
        app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(1), "Quickest Win"));
    }
    frames(&mut app, 2);
    assert!(app.world().resource::<Seen>().errors.is_empty(), "still waiting");

    fake.simulate_steam_exit();
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<Seen>().lost, vec![SteamLostReason::SteamExited]);
    // The resource stays (a game system taking `Res<SteamBackendRes>` keeps running); it is inert.
    assert_eq!(app.world().resource::<SteamBackendRes>().0.local_id(), fake.local_id());
    let waiting: Vec<String> = [(cfg!(feature = "auth"), "auth NoBackend"), (cfg!(feature = "leaderboards"), "leaderboards NoBackend")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, e)| e.to_string())
        .collect();
    let mut got = std::mem::take(&mut app.world_mut().resource_mut::<Seen>().errors);
    got.sort();
    assert_eq!(got, waiting, "answered in the frame Steam was lost");

    // New requests after that: answered NoBackend, the app keeps running.
    #[cfg(feature = "auth")]
    app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(2), "game-server"));
    #[cfg(feature = "leaderboards")]
    app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(2), "Quickest Win"));
    #[cfg(feature = "lobby")]
    app.world_mut().write_message(CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: Vec::new() });
    #[cfg(feature = "stats")]
    app.world_mut().write_message(StatsRequest::UnlockAchievement { name: "ACH_WIN".into() });
    #[cfg(feature = "overlay")]
    app.world_mut().write_message(OpenOverlay::Dialog { dialog: "friends".into() });
    #[cfg(feature = "friends")]
    app.world_mut().write_message(RequestUserInfo { steam_id: 76_561_197_960_265_731, name_only: true });
    frames(&mut app, 30);
    let errors = app.world().resource::<Seen>().errors.clone();
    let expected = [
        cfg!(feature = "auth"),
        cfg!(feature = "leaderboards"),
        cfg!(feature = "lobby"),
        cfg!(feature = "stats"),
        cfg!(feature = "overlay"),
        cfg!(feature = "friends"),
    ]
    .iter()
    .filter(|on| **on)
    .count();
    assert_eq!(errors.len(), expected, "{errors:?}");
    assert!(errors.iter().all(|e| e.ends_with("NoBackend")), "{errors:?}");
    assert_eq!(app.world().resource::<Seen>().lost.len(), 1, "SteamLost once");
    assert_eq!(fake.pump_count(), 5, "never pumped after Steam was lost");
}

#[test]
fn a_panic_inside_the_pump_does_not_end_the_game() {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    frames(&mut app, 1);
    fake.panic_in_next_pump();
    frames(&mut app, 3);
    assert_eq!(app.world().resource::<Seen>().lost, vec![SteamLostReason::PumpPanicked]);
    assert_eq!(app.world().resource::<SteamBackendRes>().0.local_id(), fake.local_id());
}

#[test]
fn after_steams_shutdown_callback_the_kit_makes_no_backend_call_at_all() {
    no_backend_call_after(|f| f.simulate_steam_shutdown(), SteamLostReason::SteamExited);
}

#[test]
fn after_the_steam_process_ended_the_kit_makes_no_backend_call_at_all() {
    // A killed or crashed client (the operating system reports its pid gone; no callback).
    no_backend_call_after(|f| f.simulate_steam_process_ended(), SteamLostReason::SteamProcessEnded);
}

fn no_backend_call_after(lose: impl Fn(&FakeSteamBackend), reason: SteamLostReason) {
    let fake = FakeSteamBackend::new();
    let mut app = app(&fake);
    frames(&mut app, 2);
    // Requests in flight when Steam shuts down.
    #[cfg(feature = "lobby")]
    {
        fake.set_auto_complete_create(false);
        app.world_mut().write_message(CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: Vec::new() });
    }
    #[cfg(feature = "auth")]
    {
        fake.fail_next_auth_ticket(FakeAuthFailure::NoAnswer);
        app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(1), "game-server"));
    }
    #[cfg(feature = "leaderboards")]
    {
        fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
        app.world_mut().write_message(LeaderboardRequest::find(LeaderboardRequestId(1), "Quickest Win"));
    }
    #[cfg(feature = "friends")]
    {
        fake.silence_next_user_info();
        app.world_mut().write_message(RequestUserInfo { steam_id: 76_561_197_960_265_750, name_only: true });
    }
    frames(&mut app, 2);
    let calls_before = fake.calls();
    let pumps_before = fake.pump_count();

    // Steam's shutdown callback (the guard's flag): from here on nothing may reach the backend.
    lose(&fake);
    // A late answer, dropped with that pump.
    #[cfg(feature = "lobby")]
    fake.push_event(BackendEvent::LobbyCreated { lobby: 77 });
    // New requests of every feature in the same frame and later, and the app exiting.
    #[cfg(feature = "lobby")]
    app.world_mut().write_message(LeaveLobby);
    #[cfg(feature = "stats")]
    app.world_mut().write_message(StatsRequest::UnlockAchievement { name: "ACH_WIN".into() });
    #[cfg(feature = "overlay")]
    app.world_mut().write_message(OpenOverlay::Dialog { dialog: "friends".into() });
    #[cfg(feature = "friends")]
    app.world_mut().write_message(InviteToGame { steam_id: 76_561_197_960_265_731, connect: "+x".into() });
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<Seen>().lost, vec![reason], "SteamLost in the same frame");
    frames(&mut app, 3);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);

    assert_eq!(fake.calls(), calls_before, "no backend call after the shutdown callback (exit frame included)");
    assert_eq!(fake.pump_count(), pumps_before + 1, "one last pump returned SteamLost; then the backend is inert");
    let errors = app.world().resource::<Seen>().errors.clone();
    // Exactly one answer per request: in flight (lobby create, ticket, find, user info) and new
    // (stats unlock, overlay, invite; `LeaveLobby` without Steam is a silent no-op).
    for (on, line, count) in [
        (cfg!(feature = "lobby"), "lobby NoBackend", 1),
        (cfg!(feature = "auth"), "auth NoBackend", 1),
        (cfg!(feature = "leaderboards"), "leaderboards NoBackend", 1),
        (cfg!(feature = "friends"), "friends NoBackend", 2),
        (cfg!(feature = "stats"), "stats NoBackend", 1),
        (cfg!(feature = "overlay"), "overlay NoBackend", 1),
    ] {
        let seen = errors.iter().filter(|e| *e == line).count();
        assert_eq!(seen, if on { count } else { 0 }, "{line} in {errors:?}");
    }
    assert!(errors.iter().all(|e| e.ends_with("NoBackend")), "{errors:?}");
    assert_eq!(app.world().resource::<Seen>().lost.len(), 1);
}
