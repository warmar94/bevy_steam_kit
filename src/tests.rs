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

#[derive(Resource, Default)]
struct LostSeen(Vec<SteamLost>);

fn record_lost(mut r: MessageReader<SteamLost>, mut seen: ResMut<LostSeen>) {
    seen.0.extend(r.read().cloned());
}

/// A game system that takes the backend as `Res` (it would fail if the resource disappeared).
fn needs_backend(backend: Res<SteamBackendRes>) {
    let _ = backend.0.local_id();
}

fn lost_app(fake: &FakeSteamBackend) -> App {
    let mut app = strict_app();
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<LostSeen>()
        .add_systems(Update, (record_lost, needs_backend).before(SteamKitSystems::Requests));
    app
}

#[test]
fn steam_exiting_makes_the_backend_inert_and_reports_steam_lost_once() {
    let fake = FakeSteamBackend::new();
    let mut app = lost_app(&fake);
    frames(&mut app, 2);
    fake.simulate_steam_exit();
    frames(&mut app, 1);
    let res = app.world().resource::<SteamBackendRes>();
    assert_eq!(res.0.local_id(), fake.local_id(), "the resource stays; identity still answers");
    assert!(res.0.pump().is_empty());
    assert_eq!(app.world().resource::<LostSeen>().0, vec![SteamLost { reason: SteamLostReason::SteamExited }], "readable in that frame's Update");
    assert!(app.world().resource::<PumpedEvents>().0.is_empty(), "the core's event is not handed to features");
    frames(&mut app, 5);
    assert_eq!(fake.pump_count(), 3, "never pumped again");
    assert_eq!(app.world().resource::<LostSeen>().0.len(), 1);
}

#[test]
fn a_panic_in_the_pump_is_caught_and_treated_as_steam_lost() {
    let fake = FakeSteamBackend::new();
    let mut app = lost_app(&fake);
    frames(&mut app, 1);
    #[cfg(feature = "lobby")]
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 3, from: 0 });
    fake.panic_in_next_pump();
    // The panic message is printed by the panic hook; the app keeps running.
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<SteamBackendRes>().0.local_id(), fake.local_id());
    assert_eq!(app.world().resource::<LostSeen>().0, vec![SteamLost { reason: SteamLostReason::PumpPanicked }]);
    assert!(app.world().resource::<PumpedEvents>().0.is_empty(), "that frame's events are dropped");
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 2);
    assert_eq!(app.world().resource::<LostSeen>().0.len(), 1);

    // A game may install a backend again (here a fresh fake): the kit pumps it.
    let again = FakeSteamBackend::new();
    app.insert_resource(SteamBackendRes(Box::new(again.clone())));
    frames(&mut app, 2);
    assert_eq!(again.pump_count(), 2);
}

#[cfg(feature = "lobby")]
#[test]
fn events_pumped_with_steam_lost_are_still_applied() {
    #[derive(Resource, Default)]
    struct Joins(Vec<JoinRequested>);
    let fake = FakeSteamBackend::new();
    let mut app = lost_app(&fake);
    let record_joins = |mut r: MessageReader<JoinRequested>, mut j: ResMut<Joins>| j.0.extend(r.read().cloned());
    app.init_resource::<Joins>().add_systems(Update, record_joins.after(SteamKitSystems::Requests));
    frames(&mut app, 1);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 3, from: 0 });
    fake.simulate_steam_exit();
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<Joins>().0.len(), 1, "applied before the backend was made inert");
    assert_eq!(app.world().resource::<LostSeen>().0.len(), 1);
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
    // With `friends`, a name change queues Steam's `PersonaChanged`; nothing else is queued.
    #[cfg(not(feature = "friends"))]
    assert!(fake.pump().is_empty());
    #[cfg(feature = "friends")]
    assert_eq!(fake.pump(), vec![BackendEvent::PersonaChanged { steam_id: 76_561_197_960_265_730, flags: 1 }]);
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
