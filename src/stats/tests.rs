//! Stats feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend), strict ambiguity detection on every main schedule, real time driven by hand
//! (100 ms per frame). No real Steam call is ever made here.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use crate::*;

const FRAME: Duration = Duration::from_millis(100);
const GAMES: &str = "NumGames";
const FEET: &str = "FeetTraveled";
const WIN: &str = "ACH_WIN_ONE_GAME";

#[derive(Resource)]
struct Seen<T: Message + Clone>(Vec<T>);

fn collect<T: Message + Clone>(mut r: MessageReader<T>, mut seen: ResMut<Seen<T>>) {
    seen.0.extend(r.read().cloned());
}

fn watch<T: Message + Clone>(app: &mut App) {
    app.insert_resource(Seen::<T>(Vec::new())).add_systems(Last, collect::<T>.after(SteamKitSystems::Requests));
}

fn seen<T: Message + Clone>(app: &App) -> Vec<T> {
    app.world().resource::<Seen<T>>().0.clone()
}

fn errors(app: &App) -> Vec<StatsErrorKind> {
    seen::<StatsError>(app).iter().map(|e| e.kind).collect()
}

fn strict(app: &mut App) {
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
}

fn app_with(backend: Option<&FakeSteamBackend>, settings: StatsSettings) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default().with_stats(settings))).insert_resource(TimeUpdateStrategy::ManualDuration(FRAME));
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<StatsReady>(&mut app);
    watch::<StatsStored>(&mut app);
    watch::<AchievementUnlocked>(&mut app);
    watch::<AchievementProgress>(&mut app);
    watch::<StatsError>(&mut app);
    strict(&mut app);
    app
}

fn spacewar() -> FakeSteamBackend {
    let fake = FakeSteamBackend::new();
    fake.define_stat(GAMES, StatValue::I32(3));
    fake.define_stat(FEET, StatValue::F32(1.5));
    fake.define_achievement(WIN, false);
    fake.define_achievement("ACH_WIN_100_GAMES", false);
    fake
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn stores(fake: &FakeSteamBackend) -> usize {
    fake.calls().iter().filter(|c| matches!(c, FakeCall::StoreStats)).count()
}

fn stats(app: &App) -> &SteamStats {
    app.world().resource::<SteamStats>()
}

fn send<M: Message>(app: &mut App, m: M) {
    app.world_mut().write_message(m);
}

/// Long store interval / gap so only the behaviour under test stores.
fn quiet() -> StatsSettings {
    StatsSettings { stats_store_interval: Duration::from_secs(3), min_store_gap: Duration::from_secs(1), ..Default::default() }
}

#[test]
fn api_names_are_validated() {
    assert!(is_valid_api_name("NumGames"));
    assert!(is_valid_api_name(&"x".repeat(MAX_API_NAME_BYTES)));
    assert!(!is_valid_api_name(""));
    assert!(!is_valid_api_name("a\0b"));
    assert!(!is_valid_api_name(&"x".repeat(MAX_API_NAME_BYTES + 1)));
}

#[test]
fn ready_at_once_with_the_fake_and_writes_apply_immediately() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    assert!(stats(&app).is_ready());
    assert_eq!(seen::<StatsReady>(&app).len(), 1);

    send(&mut app, StatsRequest::SetStat { name: FEET.into(), value: StatValue::F32(7.25) });
    send(&mut app, StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::I32(2) });
    frames(&mut app, 1);
    assert_eq!(fake.stat(FEET), Some(StatValue::F32(7.25)));
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(5)));
    assert!(stats(&app).has_unsaved());
    assert!(errors(&app).is_empty());
}

#[test]
fn writes_before_ready_are_held_and_applied_in_order_once_ready() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(10) });
    send(&mut app, StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::I32(1) });
    send(&mut app, StatsRequest::UnlockAchievement { name: WIN.into() });
    frames(&mut app, 3);
    assert!(!stats(&app).is_ready());
    assert_eq!(stats(&app).queued(), 3);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(3)));

    fake.set_stats_ready(true);
    frames(&mut app, 12); // the next probe is at most 1 s away
    assert!(stats(&app).is_ready());
    assert_eq!(stats(&app).queued(), 0);
    assert_eq!(seen::<StatsReady>(&app).len(), 1);
    // Set then add: the order was kept.
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(11)));
    assert_eq!(fake.achieved(WIN), Some(true));
    assert!(errors(&app).is_empty());
}

#[test]
fn without_a_probe_answer_a_held_write_that_succeeds_makes_stats_ready() {
    let fake = spacewar();
    // The probe names something the app does not define: only the fallback can succeed.
    let mut app = app_with(Some(&fake), StatsSettings { probe: Some("NotAStat".into()), ..quiet() });
    frames(&mut app, 2);
    assert!(!stats(&app).is_ready());
    send(&mut app, StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::I32(1) });
    frames(&mut app, 12);
    assert!(stats(&app).is_ready());
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(4)));
}

#[test]
fn a_full_queue_drops_the_oldest_with_a_not_ready_error() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), StatsSettings { max_queued: 2, ..quiet() });
    for v in 0..3 {
        send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(v) });
    }
    frames(&mut app, 1);
    let errs = seen::<StatsError>(&app);
    assert_eq!(errs.len(), 1);
    assert_eq!((errs[0].kind, errs[0].name.as_deref()), (StatsErrorKind::NotReady, Some(GAMES)));
    assert_eq!(stats(&app).queued(), 2);
}

#[test]
fn invalid_names_and_values_never_reach_the_backend() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::SetStat { name: String::new(), value: StatValue::I32(1) });
    send(&mut app, StatsRequest::UnlockAchievement { name: "ACH\0X".into() });
    send(&mut app, StatsRequest::ClearAchievement { name: "x".repeat(MAX_API_NAME_BYTES + 1) });
    send(&mut app, StatsRequest::SetStat { name: FEET.into(), value: StatValue::F32(f32::NAN) });
    send(&mut app, StatsRequest::AddStat { name: FEET.into(), delta: StatValue::F32(f32::INFINITY) });
    send(&mut app, StatsRequest::IndicateAchievementProgress { name: WIN.into(), current: 1, max: 0 });
    frames(&mut app, 2);
    let mut kinds = errors(&app);
    kinds.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(
        kinds,
        vec![
            StatsErrorKind::InvalidName,
            StatsErrorKind::InvalidName,
            StatsErrorKind::InvalidName,
            StatsErrorKind::InvalidRequest,
            StatsErrorKind::NotFinite,
            StatsErrorKind::NotFinite
        ]
    );
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

#[test]
fn unknown_names_and_wrong_types_are_refused_by_steam() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::SetStat { name: "Nope".into(), value: StatValue::I32(1) });
    send(&mut app, StatsRequest::SetStat { name: FEET.into(), value: StatValue::I32(1) });
    send(&mut app, StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::F32(1.0) });
    send(&mut app, StatsRequest::UnlockAchievement { name: "ACH_NOPE".into() });
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![StatsErrorKind::Refused; 4]);
    assert_eq!(fake.stat(FEET), Some(StatValue::F32(1.5)));
    assert!(!stats(&app).has_unsaved());
}

#[test]
fn add_stat_saturates_i32_and_refuses_a_non_finite_sum() {
    let fake = spacewar();
    fake.define_stat("Big", StatValue::I32(i32::MAX - 1));
    fake.define_stat("Huge", StatValue::F32(f32::MAX));
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::AddStat { name: "Big".into(), delta: StatValue::I32(5) });
    send(&mut app, StatsRequest::AddStat { name: "Huge".into(), delta: StatValue::F32(f32::MAX) });
    frames(&mut app, 2);
    assert_eq!(fake.stat("Big"), Some(StatValue::I32(i32::MAX)));
    assert_eq!(fake.stat("Huge"), Some(StatValue::F32(f32::MAX)));
    assert_eq!(errors(&app), vec![StatsErrorKind::NotFinite]);
}

#[test]
fn an_unlock_is_stored_after_the_achievement_delay_and_confirmed() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    send(&mut app, StatsRequest::UnlockAchievement { name: WIN.into() });
    frames(&mut app, 5); // 0.5 s: still batching
    assert_eq!(stores(&fake), 0);
    frames(&mut app, 7); // past the 1 s delay; the outcome arrives on the next pump
    assert_eq!(stores(&fake), 1);
    assert_eq!(seen::<AchievementUnlocked>(&app), vec![AchievementUnlocked { name: WIN.into() }]);
    assert_eq!(seen::<StatsStored>(&app).len(), 1);
    assert!(!stats(&app).has_unsaved());
    assert!(!stats(&app).store_in_flight());

    // Unlocking it again changes nothing and stores nothing.
    send(&mut app, StatsRequest::UnlockAchievement { name: WIN.into() });
    frames(&mut app, 30);
    assert_eq!(stores(&fake), 1);
    assert_eq!(fake.calls().iter().filter(|c| matches!(c, FakeCall::UnlockAchievement(_))).count(), 1);
}

#[test]
fn many_stat_changes_are_batched_into_one_store_after_the_interval() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    for _ in 0..100 {
        send(&mut app, StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::I32(1) });
    }
    frames(&mut app, 10);
    send(&mut app, StatsRequest::AddStat { name: FEET.into(), delta: StatValue::F32(0.5) });
    frames(&mut app, 18); // 2.8 s after the first change
    assert_eq!(stores(&fake), 0);
    frames(&mut app, 4);
    assert_eq!(stores(&fake), 1);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(103)));
    frames(&mut app, 60);
    assert_eq!(stores(&fake), 1, "nothing new to store");
}

#[test]
fn store_requests_respect_the_gap_and_one_store_in_flight() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), StatsSettings { store_timeout: Duration::from_secs(2), ..quiet() });
    frames(&mut app, 1);
    fake.fail_next_store(FakeStoreFailure::NoAnswer);
    send(&mut app, StatsRequest::StoreStats);
    frames(&mut app, 1);
    assert_eq!(stores(&fake), 1);
    assert!(stats(&app).store_in_flight());

    // In flight: a new request waits ...
    send(&mut app, StatsRequest::StoreStats);
    frames(&mut app, 15);
    assert_eq!(stores(&fake), 1);
    // ... until the timeout gives up on the first one; the waiting request then stores.
    frames(&mut app, 6);
    assert!(errors(&app).contains(&StatsErrorKind::StoreTimedOut));
    assert_eq!(stores(&fake), 2);

    // Two requests 0.3 s apart: the second waits for the 1 s gap.
    frames(&mut app, 20);
    send(&mut app, StatsRequest::StoreStats);
    frames(&mut app, 3);
    let before = stores(&fake);
    send(&mut app, StatsRequest::StoreStats);
    frames(&mut app, 3);
    assert_eq!(stores(&fake), before);
    frames(&mut app, 6);
    assert_eq!(stores(&fake), before + 1);
}

#[test]
fn a_rejected_store_reports_and_steam_values_come_back() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(99) });
    send(&mut app, StatsRequest::StoreStats);
    fake.fail_next_store(FakeStoreFailure::Rejected);
    frames(&mut app, 3);
    assert_eq!(errors(&app), vec![StatsErrorKind::StoreRejected]);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(3)), "the fake restores the stored value like Steam");
    frames(&mut app, 60);
    assert_eq!(stores(&fake), 1, "a rejected change is not stored again");
}

#[test]
fn a_failed_or_refused_store_is_retried() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(4) });
    send(&mut app, StatsRequest::StoreStats);
    fake.fail_next_store(FakeStoreFailure::Failed);
    frames(&mut app, 3);
    assert_eq!(errors(&app), vec![StatsErrorKind::StoreFailed]);
    assert!(stats(&app).has_unsaved());
    frames(&mut app, 50); // the stats interval (3 s) after the failure, plus the gap
    assert_eq!(stores(&fake), 2);
    assert_eq!(seen::<StatsStored>(&app).len(), 1);

    fake.fail_next_store(FakeStoreFailure::Refused);
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(5) });
    send(&mut app, StatsRequest::StoreStats);
    frames(&mut app, 2);
    assert_eq!(errors(&app).last(), Some(&StatsErrorKind::StoreRefused));
    frames(&mut app, 35); // the still-unsaved change is stored again after the stats interval
    assert_eq!(stores(&fake), 4);
    assert_eq!(seen::<StatsStored>(&app).len(), 2);
}

#[test]
fn progress_is_clamped_and_reported() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::IndicateAchievementProgress { name: "ACH_WIN_100_GAMES".into(), current: 150, max: 100 });
    frames(&mut app, 2);
    assert!(fake.calls().contains(&FakeCall::IndicateAchievementProgress { name: "ACH_WIN_100_GAMES".into(), current: 100, max: 100 }));
    assert_eq!(seen::<AchievementProgress>(&app), vec![AchievementProgress { name: "ACH_WIN_100_GAMES".into(), current: 100, max: 100 }]);
    assert_eq!(fake.achieved("ACH_WIN_100_GAMES"), Some(false), "progress never unlocks");
    assert_eq!(stores(&fake), 0);
}

#[test]
fn clear_and_reset_are_applied_and_stored() {
    let fake = spacewar();
    fake.define_achievement("ACH_DONE", true);
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::ClearAchievement { name: "ACH_DONE".into() });
    frames(&mut app, 13);
    assert_eq!(fake.achieved("ACH_DONE"), Some(false));
    assert_eq!(stores(&fake), 1);

    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(50) });
    send(&mut app, StatsRequest::ResetAllStats { achievements_too: true });
    frames(&mut app, 2);
    // Applied in the order written: the reset comes after the set.
    assert!(fake.calls().contains(&FakeCall::ResetAllStats { achievements_too: true }));
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(3)));
    assert_eq!(fake.stat(FEET), Some(StatValue::F32(1.5)));
}

#[test]
fn app_exit_stores_unsaved_changes_once_and_nothing_when_clean() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(stores(&fake), 0);

    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(8) });
    frames(&mut app, 1);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(stores(&fake), 1);
    assert!(!stats(&app).has_unsaved());
}

#[test]
fn exiting_with_held_writes_reports_every_one() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(1) });
    send(&mut app, StatsRequest::UnlockAchievement { name: WIN.into() });
    frames(&mut app, 1);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![StatsErrorKind::NotReady; 2]);
    assert_eq!(stats(&app).queued(), 0);
}

#[test]
fn without_a_backend_requests_get_no_backend_and_store_is_silent() {
    let mut app = app_with(None, quiet());
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(1) });
    send(&mut app, StatsRequest::UnlockAchievement { name: WIN.into() });
    send(&mut app, StatsRequest::StoreStats);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![StatsErrorKind::NoBackend; 2]);
    assert!(!stats(&app).is_ready());
}

/// A backend written without the stats half.
struct CoreOnly(FakeSteamBackend);

impl SteamBackend for CoreOnly {
    fn local_id(&self) -> u64 {
        self.0.local_id()
    }
    fn friend_name(&self, id: u64) -> String {
        self.0.friend_name(id)
    }
    fn launch_command_line(&self) -> String {
        self.0.launch_command_line()
    }
    fn pump(&self) -> Vec<BackendEvent> {
        self.0.pump()
    }
}

#[test]
fn a_backend_without_stats_support_is_treated_as_no_steam() {
    let fake = spacewar();
    let mut app = app_with(None, quiet());
    app.insert_resource(SteamBackendRes(Box::new(CoreOnly(fake.clone()))));
    send(&mut app, StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(1) });
    fake.push_event(BackendEvent::StatsStored);
    frames(&mut app, 3);
    assert_eq!(fake.pump_count(), 3);
    assert_eq!(errors(&app), vec![StatsErrorKind::NoBackend]);
    assert!(seen::<StatsStored>(&app).is_empty());
    assert!(fake.calls().is_empty());
}

/// Which frame (1-based) each fact was read in, by a game system in `PreUpdate`.
#[derive(Resource, Default)]
struct ReadFrames(Vec<(u32, &'static str)>);

fn read_in_pre_update(mut frame: Local<u32>, mut ready: MessageReader<StatsReady>, mut stored: MessageReader<StatsStored>, mut seen: ResMut<ReadFrames>) {
    *frame += 1;
    for _ in ready.read() {
        seen.0.push((*frame, "ready"));
    }
    for _ in stored.read() {
        seen.0.push((*frame, "stored"));
    }
}

#[test]
fn stats_received_for_the_local_user_is_ready_in_the_same_frame_and_others_are_ignored() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), quiet());
    app.init_resource::<ReadFrames>().add_systems(PreUpdate, read_in_pre_update);
    frames(&mut app, 2);
    fake.push_event(BackendEvent::StatsReceived { user: 76_561_197_960_265_730, ok: true });
    frames(&mut app, 1);
    assert!(!stats(&app).is_ready(), "another user's stats");
    fake.push_event(BackendEvent::StatsReceived { user: fake.local_id(), ok: true });
    fake.push_event(BackendEvent::StatsStored);
    frames(&mut app, 1);
    assert!(stats(&app).is_ready());
    assert_eq!(app.world().resource::<ReadFrames>().0, vec![(4, "ready"), (4, "stored")]);
}

#[test]
fn removing_the_backend_between_pump_and_callbacks_drops_stats_events_without_panicking() {
    fn remove_backend(world: &mut World) {
        world.remove_resource::<SteamBackendRes>();
    }
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    app.add_systems(First, remove_backend.after(bevy::time::TimeSystems).after(SteamKitSystems::Pump).before(SteamKitSystems::Callbacks));
    fake.push_event(BackendEvent::StatsStored);
    frames(&mut app, 2);
    assert_eq!(fake.pump_count(), 1);
    assert!(seen::<StatsStored>(&app).is_empty());
}

#[test]
fn settings_come_from_the_kit_plugin_builder() {
    let fake = spacewar();
    let app = app_with(Some(&fake), StatsSettings { probe: Some(GAMES.into()), max_queued: 7, ..Default::default() });
    let s = app.world().resource::<StatsSettings>();
    assert_eq!((s.probe.as_deref(), s.max_queued), (Some(GAMES), 7));
    assert_eq!(StatsSettings::default().stats_store_interval, Duration::from_secs(60));
}

#[test]
fn requests_of_different_kinds_apply_in_the_order_written() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    frames(&mut app, 1);
    // Add then set: the set wins.
    send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
    send(&mut app, StatsRequest::set_stat(GAMES, StatValue::I32(0)));
    // Clear then unlock: unlocked.
    send(&mut app, StatsRequest::clear_achievement(WIN));
    send(&mut app, StatsRequest::unlock_achievement(WIN));
    frames(&mut app, 1);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(0)));
    assert_eq!(fake.achieved(WIN), Some(true));

    // Unlock then reset: reset wins.
    send(&mut app, StatsRequest::unlock_achievement("ACH_WIN_100_GAMES"));
    send(&mut app, StatsRequest::ResetAllStats { achievements_too: true });
    frames(&mut app, 1);
    assert_eq!(fake.achieved("ACH_WIN_100_GAMES"), Some(false));
    assert_eq!(fake.achieved(WIN), Some(false));
    assert!(errors(&app).is_empty());
}

/// A game system that writes after the kit's `Update` requests set and asks to quit.
#[derive(Resource)]
struct LateWrite(bool);

fn late_writer(mut late: ResMut<LateWrite>, mut stats: MessageWriter<StatsRequest>, mut exit: MessageWriter<AppExit>) {
    if std::mem::take(&mut late.0) {
        stats.write(StatsRequest::add_stat(GAMES, StatValue::I32(5)));
        stats.write(StatsRequest::unlock_achievement(WIN));
        exit.write(AppExit::Success);
    }
}

#[test]
fn writes_in_the_exit_frame_after_the_requests_set_are_applied_and_stored() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), quiet());
    app.insert_resource(LateWrite(false)).add_systems(Update, late_writer.after(SteamKitSystems::Requests));
    frames(&mut app, 1);
    app.insert_resource(LateWrite(true));
    frames(&mut app, 1);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(8)));
    assert_eq!(fake.achieved(WIN), Some(true));
    assert_eq!(stores(&fake), 1, "the exit store");
    // Never applied twice (the Update system shares the read position).
    frames(&mut app, 1);
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(8)));
}

#[test]
fn exit_frame_writes_before_readiness_are_reported_not_ready() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), quiet());
    app.insert_resource(LateWrite(false)).add_systems(Update, late_writer.after(SteamKitSystems::Requests));
    frames(&mut app, 1);
    app.insert_resource(LateWrite(true));
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![StatsErrorKind::NotReady; 2]);
    assert_eq!(stores(&fake), 0);
}

#[test]
fn without_a_clock_readiness_is_probed_every_frame_and_explicit_stores_work() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    // No MinimalPlugins: no TimePlugin, so no `Time<Real>`.
    let mut app = App::new();
    app.add_plugins(SteamKitPlugin::default()).insert_resource(SteamBackendRes(Box::new(fake.clone())));
    watch::<StatsError>(&mut app);
    strict(&mut app);
    frames(&mut app, 3);
    assert!(!stats(&app).is_ready());
    fake.set_stats_ready(true);
    frames(&mut app, 1);
    assert!(stats(&app).is_ready());

    for _ in 0..3 {
        send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
        send(&mut app, StatsRequest::StoreStats);
        frames(&mut app, 2);
    }
    assert_eq!(stores(&fake), 3, "explicit stores are not held back by a gap measured on a missing clock");
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(6)));
    assert!(errors(&app).is_empty());
}

#[test]
fn a_failed_store_keeps_an_unlock_on_the_short_achievement_delay() {
    let fake = spacewar();
    // Long stats interval: only the achievement delay can make the retry happen soon.
    let mut app =
        app_with(Some(&fake), StatsSettings { stats_store_interval: Duration::from_secs(60), min_store_gap: Duration::from_secs(1), ..Default::default() });
    frames(&mut app, 1);
    fake.fail_next_store(FakeStoreFailure::Failed);
    send(&mut app, StatsRequest::unlock_achievement(WIN));
    frames(&mut app, 13); // stored after 1 s, failed on the next pump
    assert_eq!(errors(&app), vec![StatsErrorKind::StoreFailed]);
    assert_eq!(stores(&fake), 1);
    frames(&mut app, 15); // retried after the 1 s achievement delay (and the 1 s gap), not after 60 s
    assert_eq!(stores(&fake), 2);
    assert_eq!(seen::<AchievementUnlocked>(&app), vec![AchievementUnlocked { name: WIN.into() }]);
}

#[test]
fn a_refused_store_is_reported_once_until_a_store_starts_again() {
    let fake = spacewar();
    let mut app =
        app_with(Some(&fake), StatsSettings { stats_store_interval: Duration::from_secs(1), min_store_gap: Duration::from_secs(1), ..Default::default() });
    frames(&mut app, 1);
    send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
    for _ in 0..5 {
        fake.fail_next_store(FakeStoreFailure::Refused);
        frames(&mut app, 12);
    }
    assert!(stores(&fake) >= 4, "tried again at the normal cadence");
    assert_eq!(errors(&app), vec![StatsErrorKind::StoreRefused], "reported once");
    frames(&mut app, 15); // no failure queued now: the store goes through
    assert_eq!(seen::<StatsStored>(&app).len(), 1);

    // A new refusal after a store started is reported again.
    fake.fail_next_store(FakeStoreFailure::Refused);
    send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
    frames(&mut app, 15);
    assert_eq!(errors(&app), vec![StatsErrorKind::StoreRefused; 2]);
}

#[test]
fn a_permanently_refused_held_write_does_not_block_readiness() {
    let fake = spacewar();
    let mut app = app_with(Some(&fake), StatsSettings { probe: Some("NotAStat".into()), ..quiet() });
    frames(&mut app, 1);
    send(&mut app, StatsRequest::set_stat("NoSuchStat", StatValue::I32(1)));
    send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
    frames(&mut app, 12);
    assert!(stats(&app).is_ready());
    assert_eq!(fake.stat(GAMES), Some(StatValue::I32(4)));
    // The refused one is applied once ready and reported.
    assert_eq!(errors(&app), vec![StatsErrorKind::Refused]);
}

#[test]
fn held_writes_are_reported_at_exit_even_after_the_backend_was_removed() {
    let fake = spacewar();
    fake.set_stats_ready(false);
    let mut app = app_with(Some(&fake), quiet());
    send(&mut app, StatsRequest::add_stat(GAMES, StatValue::I32(1)));
    frames(&mut app, 1);
    assert_eq!(stats(&app).queued(), 1);
    app.world_mut().remove_resource::<SteamBackendRes>();
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![StatsErrorKind::NotReady]);
    assert_eq!(stats(&app).queued(), 0);
}
