//! Core: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE backend). They
//! run in every feature combination, including a build without any feature. No real Steam call is
//! ever made here.

use bevy::ecs::message::MessageUpdateSystems;
use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;

use crate::*;

fn strict_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()));
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

#[test]
fn the_kit_pumps_exactly_once_per_frame() {
    let fake = FakeSteamBackend::new();
    let mut app = strict_app();
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    for n in 1..=5 {
        app.update();
        assert_eq!(fake.pump_count(), n);
    }
}

#[test]
fn no_backend_no_pump_and_nothing_panics() {
    let fake = FakeSteamBackend::new();
    let mut app = strict_app();
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 0);

    // Inserted later (a game that starts Steam only in some modes): pumping starts then ...
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    frames(&mut app, 2);
    assert_eq!(fake.pump_count(), 2);

    // ... and stops when it is removed.
    app.world_mut().remove_resource::<SteamBackendRes>();
    frames(&mut app, 2);
    assert_eq!(fake.pump_count(), 2);
}

#[test]
fn the_pump_buffer_holds_only_this_frames_events() {
    let fake = FakeSteamBackend::new();
    let mut app = strict_app();
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    frames(&mut app, 1);
    assert!(app.world().resource::<PumpedEvents>().0.is_empty());

    #[cfg(feature = "lobby")]
    {
        fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 3, from: 0 });
        frames(&mut app, 1);
        assert_eq!(app.world().resource::<PumpedEvents>().0, vec![BackendEvent::LobbyJoinRequested { lobby: 3, from: 0 }]);
        frames(&mut app, 1);
        assert!(app.world().resource::<PumpedEvents>().0.is_empty(), "replaced, never replayed");

        // Removing the backend clears the buffer too.
        fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 4, from: 0 });
        frames(&mut app, 1);
        app.world_mut().remove_resource::<SteamBackendRes>();
        frames(&mut app, 1);
        assert!(app.world().resource::<PumpedEvents>().0.is_empty());
    }
}

/// A game system in `First` ordered around the public sets must not be ambiguous with the kit.
fn game_system_in_first(backend: Option<Res<SteamBackendRes>>) {
    let _ = backend.map(|b| b.0.local_id());
}

#[test]
fn a_strict_app_with_the_kit_and_game_systems_builds_and_runs() {
    let fake = FakeSteamBackend::new();
    let mut app = strict_app();
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .add_systems(First, game_system_in_first.after(SteamKitSystems::Callbacks).before(MessageUpdateSystems))
        .add_systems(Update, game_system_in_first.before(SteamKitSystems::Requests));
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 3);
}

#[test]
fn the_fake_answers_core_queries() {
    let fake = FakeSteamBackend::new();
    assert_eq!(fake.local_id(), 76_561_197_960_265_729);
    assert!(is_individual_steam_id64(fake.local_id()));
    fake.set_local_id(76_561_197_960_265_731);
    assert_eq!(fake.local_id(), 76_561_197_960_265_731);
    assert_eq!(fake.friend_name(76_561_197_960_265_730), "");
    fake.set_friend_name(76_561_197_960_265_730, "Test Friend");
    assert_eq!(fake.friend_name(76_561_197_960_265_730), "Test Friend");
    assert_eq!(fake.launch_command_line(), "");
    fake.set_launch_command_line("-x");
    assert_eq!(fake.launch_command_line(), "-x");
    assert!(fake.pump().is_empty());
    assert_eq!(fake.pump_count(), 1);
    assert!(fake.calls().is_empty(), "queries and pumps are not recorded");
}

#[test]
fn steam_id64_validation() {
    assert!(is_individual_steam_id64(76_561_198_000_000_042)); // fabricated
    assert!(is_individual_steam_id64(76_561_197_960_265_729));
    assert!(!is_individual_steam_id64(0));
    assert!(!is_individual_steam_id64(1));
    assert!(!is_individual_steam_id64(12345));
    // Account id 0 in an otherwise valid individual prefix.
    assert!(!is_individual_steam_id64(76_561_197_960_265_728));
    // A clan/group id (account type 7, instance 0).
    assert!(!is_individual_steam_id64(103_582_791_429_521_408));
    // A lobby / chat id (account type 8).
    assert!(!is_individual_steam_id64(109_775_241_000_000_000));
    // Wrong universe (2 = internal).
    assert!(!is_individual_steam_id64((2u64 << 56) | (1 << 52) | (1 << 32) | 5));
}
