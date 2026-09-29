//! The stats feature's public surface from a game's point of view: the kit in a strict headless
//! app (ambiguity detection = Error on every main schedule), the game's own systems ordered
//! around the public sets, real time driven by hand, and the in-memory `FakeSteamBackend`.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_steam_kit::*;

/// What the game saw.
#[derive(Resource, Default)]
struct Seen {
    ready: usize,
    stored: usize,
    unlocked: Vec<String>,
    errors: Vec<StatsErrorKind>,
    /// `NumGames` read on every `StatsStored`.
    games_on_store: Vec<Option<StatValue>>,
}

/// The game wins a match: count it and unlock the first-win achievement.
#[derive(Resource)]
struct WonAMatch(bool);

fn play(mut won: ResMut<WonAMatch>, mut stats: MessageWriter<StatsRequest>) {
    if std::mem::take(&mut won.0) {
        stats.write(StatsRequest::add_stat("NumGames", StatValue::I32(1)));
        stats.write(StatsRequest::unlock_achievement("ACH_WIN_ONE_GAME"));
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct Facts<'w, 's> {
    ready: MessageReader<'w, 's, StatsReady>,
    stored: MessageReader<'w, 's, StatsStored>,
    unlocked: MessageReader<'w, 's, AchievementUnlocked>,
    errors: MessageReader<'w, 's, StatsError>,
}

fn record(mut facts: Facts, backend: Option<Res<SteamBackendRes>>, mut seen: ResMut<Seen>) {
    seen.ready += facts.ready.read().count();
    for _ in facts.stored.read() {
        let games = backend.as_ref().and_then(|b| b.0.stats()).and_then(|s| s.get_stat("NumGames", StatKind::I32));
        seen.games_on_store.push(games);
        seen.stored += 1;
    }
    seen.unlocked.extend(facts.unlocked.read().map(|u| u.name.clone()));
    seen.errors.extend(facts.errors.read().map(|e| e.kind));
}

fn game(fake: &FakeSteamBackend) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default().with_stats(StatsSettings { probe: Some("NumGames".into()), ..Default::default() })))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<Seen>()
        .insert_resource(WonAMatch(false))
        .add_systems(Update, play.before(SteamKitSystems::Requests))
        .add_systems(Last, record.after(SteamKitSystems::Requests));
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app
}

fn spacewar() -> FakeSteamBackend {
    let fake = FakeSteamBackend::new();
    fake.define_stat("NumGames", StatValue::I32(0));
    fake.define_achievement("ACH_WIN_ONE_GAME", false);
    fake
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

#[test]
fn a_win_is_counted_unlocked_and_stored_once() {
    let fake = spacewar();
    let mut app = game(&fake);
    frames(&mut app, 2);
    assert_eq!(app.world().resource::<Seen>().ready, 1);

    app.insert_resource(WonAMatch(true));
    frames(&mut app, 15);
    let seen = app.world().resource::<Seen>();
    assert_eq!(seen.unlocked, vec!["ACH_WIN_ONE_GAME".to_string()]);
    assert_eq!(seen.stored, 1);
    assert_eq!(seen.games_on_store, vec![Some(StatValue::I32(1))]);
    assert!(seen.errors.is_empty(), "{:?}", seen.errors);
    assert_eq!(fake.calls().iter().filter(|c| matches!(c, FakeCall::StoreStats)).count(), 1);
    assert!(!app.world().resource::<SteamStats>().has_unsaved());
}

#[test]
fn a_win_before_stats_are_loaded_is_not_lost() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = game(&fake);
    app.insert_resource(WonAMatch(true));
    frames(&mut app, 5);
    assert_eq!(app.world().resource::<SteamStats>().queued(), 2);

    fake.set_stats_ready(true);
    frames(&mut app, 30);
    let seen = app.world().resource::<Seen>();
    assert_eq!(seen.ready, 1);
    assert_eq!(seen.unlocked, vec!["ACH_WIN_ONE_GAME".to_string()]);
    assert_eq!(fake.stat("NumGames"), Some(StatValue::I32(1)));
}

#[test]
fn the_exit_store_catches_the_last_changes() {
    let fake = spacewar();
    let mut app = game(&fake);
    frames(&mut app, 2);
    app.world_mut().write_message(StatsRequest::AddStat { name: "NumGames".into(), delta: StatValue::I32(4) });
    frames(&mut app, 1);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(fake.calls().iter().filter(|c| matches!(c, FakeCall::StoreStats)).count(), 1);
    assert!(!app.world().resource::<SteamStats>().has_unsaved());
}
