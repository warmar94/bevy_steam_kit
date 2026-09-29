//! Leaderboards feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend), strict ambiguity detection on every main schedule, real time driven by hand
//! (100 ms per frame). No real Steam call is ever made here.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use super::range_ok;
use crate::*;

const BOARD: &str = "Feet Traveled";
const ME: u64 = 76_561_197_960_265_729;
const A: u64 = 76_561_197_960_265_730;
const B: u64 = 76_561_197_960_265_731;

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

fn errors(app: &App) -> Vec<(u64, LeaderboardErrorKind)> {
    seen::<LeaderboardError>(app).iter().map(|e| (e.id.0, e.kind)).collect()
}

fn app_with(backend: Option<&FakeSteamBackend>, settings: LeaderboardSettings) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default().with_leaderboards(settings)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<LeaderboardFound>(&mut app);
    watch::<ScoreUploaded>(&mut app);
    watch::<ScoresDownloaded>(&mut app);
    watch::<LeaderboardError>(&mut app);
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app
}

fn board_fake() -> FakeSteamBackend {
    let fake = FakeSteamBackend::new();
    fake.add_leaderboard(BOARD, LeaderboardSort::Descending, LeaderboardDisplay::Numeric);
    fake.add_leaderboard_entry(BOARD, A, 500, &[1, 2, 3]);
    fake.add_leaderboard_entry(BOARD, B, 300, &[]);
    fake
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn send(app: &mut App, req: LeaderboardRequest) {
    app.world_mut().write_message(req);
}

fn id(n: u64) -> LeaderboardRequestId {
    LeaderboardRequestId(n)
}

fn count(fake: &FakeSteamBackend, f: impl Fn(&FakeCall) -> bool) -> usize {
    fake.calls().iter().filter(|c| f(c)).count()
}

fn finds(fake: &FakeSteamBackend) -> usize {
    count(fake, |c| matches!(c, FakeCall::FindLeaderboard { .. }))
}

fn uploads(fake: &FakeSteamBackend) -> usize {
    count(fake, |c| matches!(c, FakeCall::UploadScore { .. }))
}

#[test]
fn ranges_are_checked() {
    assert!(range_ok(ScoreRange::Global { first: 1, last: 10 }, 500));
    assert!(range_ok(ScoreRange::Global { first: 491, last: 990 }, 500));
    assert!(!range_ok(ScoreRange::Global { first: 491, last: 991 }, 500));
    assert!(!range_ok(ScoreRange::Global { first: 0, last: 10 }, 500));
    assert!(!range_ok(ScoreRange::Global { first: 10, last: 9 }, 500));
    assert!(range_ok(ScoreRange::AroundUser { before: 0, after: 0 }, 500));
    assert!(range_ok(ScoreRange::AroundUser { before: 249, after: 250 }, 500));
    assert!(!range_ok(ScoreRange::AroundUser { before: 250, after: 250 }, 500));
    assert!(!range_ok(ScoreRange::AroundUser { before: u32::MAX, after: u32::MAX }, 500));
    assert!(range_ok(ScoreRange::Friends, 500));
    assert!(is_valid_leaderboard_name(&"x".repeat(MAX_LEADERBOARD_NAME_BYTES)));
    assert!(!is_valid_leaderboard_name(&"x".repeat(MAX_LEADERBOARD_NAME_BYTES + 1)));
}

#[test]
fn find_answers_with_the_board_and_caches_it() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 2);
    let found = seen::<LeaderboardFound>(&app);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id(1));
    assert_eq!(
        (found[0].info.sort, found[0].info.display, found[0].info.entry_count),
        (Some(LeaderboardSort::Descending), Some(LeaderboardDisplay::Numeric), 2)
    );
    assert!(app.world().resource::<SteamLeaderboards>().board(BOARD).is_some());

    // Cached: answered without Steam.
    send(&mut app, LeaderboardRequest::find(id(2), BOARD));
    frames(&mut app, 1);
    assert_eq!(seen::<LeaderboardFound>(&app).len(), 2);
    assert_eq!(finds(&fake), 1);
}

#[test]
fn a_missing_board_is_not_found_and_find_or_create_creates_it() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), "Nope"));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::NotFound)]);

    send(&mut app, LeaderboardRequest::find_or_create(id(2), "Quickest Win", LeaderboardSort::Ascending, LeaderboardDisplay::TimeSeconds));
    frames(&mut app, 2);
    let found = seen::<LeaderboardFound>(&app);
    assert_eq!(found[0].info.sort, Some(LeaderboardSort::Ascending));
    assert!(fake.calls().contains(&FakeCall::FindLeaderboard { name: "Quickest Win".into(), create: true }));
}

#[test]
fn uploads_find_the_board_once_and_run_one_at_a_time() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::upload(id(1), BOARD, 100, UploadMethod::KeepBest));
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 50, UploadMethod::KeepBest));
    frames(&mut app, 1);
    assert_eq!(finds(&fake), 1, "one find for both uploads");
    assert_eq!(uploads(&fake), 0);
    frames(&mut app, 1); // found -> first upload starts
    assert_eq!(uploads(&fake), 1);
    assert!(app.world().resource::<SteamLeaderboards>().upload_in_flight());
    frames(&mut app, 1); // first answered -> second starts
    assert_eq!(uploads(&fake), 2);
    frames(&mut app, 1);
    let up = seen::<ScoreUploaded>(&app);
    assert_eq!(up.len(), 2);
    assert_eq!((up[0].id, up[0].changed, up[0].rank_new, up[0].rank_previous), (id(1), true, 3, 0));
    // KeepBest on a descending board: 50 does not beat 100.
    assert_eq!((up[1].id, up[1].changed, up[1].rank_new, up[1].rank_previous), (id(2), false, 3, 3));
    assert!(errors(&app).is_empty());
}

#[test]
fn force_update_replaces_and_keep_best_keeps_the_better_score() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::UploadScore { id: id(1), board: BOARD.into(), score: 900, details: vec![7; 64], method: UploadMethod::KeepBest });
    frames(&mut app, 4);
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 10, UploadMethod::ForceUpdate));
    frames(&mut app, 3);
    let up = seen::<ScoreUploaded>(&app);
    assert_eq!((up[0].rank_new, up[1].changed, up[1].rank_new, up[1].rank_previous), (1, true, 3, 1));
    let entries = fake.leaderboard_entries(BOARD);
    assert_eq!(entries.iter().find(|e| e.steam_id == ME).map(|e| e.score), Some(10));
}

#[test]
fn downloads_cover_global_around_user_and_friends() {
    let fake = board_fake();
    fake.add_leaderboard_entry(BOARD, ME, 400, &[9, 9]);
    fake.add_leaderboard_entry(BOARD, 76_561_197_960_265_799, 100, &[]);
    fake.set_friends(&[A]);
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::DownloadScores { id: id(1), board: BOARD.into(), range: ScoreRange::Global { first: 1, last: 2 }, max_details: 2 });
    send(&mut app, LeaderboardRequest::download(id(2), BOARD, ScoreRange::AroundUser { before: 1, after: 1 }));
    send(&mut app, LeaderboardRequest::download(id(3), BOARD, ScoreRange::Friends));
    frames(&mut app, 3);
    let got = seen::<ScoresDownloaded>(&app);
    assert_eq!(got.len(), 3);
    let by = |n| got.iter().find(|d| d.id == id(n)).unwrap_or_else(|| panic!("no answer {n}"));
    let ids = |n| by(n).entries.iter().map(|e| (e.rank, e.steam_id)).collect::<Vec<_>>();
    assert_eq!(ids(1), vec![(1, A), (2, ME)]);
    assert_eq!(by(1).entries[0].details, vec![1, 2], "details cut to the 2 asked for");
    assert_eq!(by(1).entry_count, 4);
    assert_eq!(ids(2), vec![(1, A), (2, ME), (3, B)]);
    assert!(by(2).entries.iter().all(|e| e.details.is_empty()));
    assert_eq!(ids(3), vec![(1, A), (2, ME)]);
    assert_eq!(finds(&fake), 1);
}

#[test]
fn around_user_without_an_entry_is_empty() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::download(id(1), BOARD, ScoreRange::AroundUser { before: 3, after: 3 }));
    frames(&mut app, 3);
    assert_eq!(seen::<ScoresDownloaded>(&app)[0].entries, vec![]);
}

#[test]
fn bad_requests_never_reach_steam() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), ""));
    send(&mut app, LeaderboardRequest::find(id(2), "Bad\0Name"));
    send(&mut app, LeaderboardRequest::find(id(3), "x".repeat(MAX_LEADERBOARD_NAME_BYTES + 1)));
    send(&mut app, LeaderboardRequest::UploadScore { id: id(4), board: BOARD.into(), score: 1, details: vec![0; 65], method: UploadMethod::KeepBest });
    send(&mut app, LeaderboardRequest::DownloadScores { id: id(5), board: BOARD.into(), range: ScoreRange::Friends, max_details: 65 });
    send(&mut app, LeaderboardRequest::download(id(6), BOARD, ScoreRange::Global { first: 0, last: 5 }));
    send(&mut app, LeaderboardRequest::download(id(7), BOARD, ScoreRange::AroundUser { before: 400, after: 400 }));
    frames(&mut app, 3);
    assert_eq!(
        errors(&app),
        vec![
            (1, LeaderboardErrorKind::InvalidName),
            (2, LeaderboardErrorKind::InvalidName),
            (3, LeaderboardErrorKind::InvalidName),
            (4, LeaderboardErrorKind::TooManyDetails),
            (5, LeaderboardErrorKind::TooManyDetails),
            (6, LeaderboardErrorKind::InvalidRange),
            (7, LeaderboardErrorKind::InvalidRange),
        ]
    );
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

#[test]
fn a_duplicate_pending_id_is_refused_and_the_first_still_answers() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    send(&mut app, LeaderboardRequest::download(id(1), BOARD, ScoreRange::Friends));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::DuplicateId)]);
    assert_eq!(seen::<LeaderboardFound>(&app).len(), 1);
    assert!(!app.world().resource::<SteamLeaderboards>().is_pending(id(1)));
}

#[test]
fn failures_are_reported_as_steam_reports_them_and_the_queue_moves_on() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 2);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::Rejected);
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 1, UploadMethod::KeepBest));
    send(&mut app, LeaderboardRequest::upload(id(3), BOARD, 2, UploadMethod::KeepBest));
    frames(&mut app, 3);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::IoFailure);
    send(&mut app, LeaderboardRequest::download(id(4), BOARD, ScoreRange::Friends));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(2, LeaderboardErrorKind::UploadRejected), (4, LeaderboardErrorKind::IoFailure)]);
    assert_eq!(seen::<ScoreUploaded>(&app).iter().map(|u| u.id).collect::<Vec<_>>(), vec![id(3)]);
}

#[test]
fn a_call_without_an_answer_times_out_and_a_late_answer_is_dropped() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings { timeout: Duration::from_secs(1), ..Default::default() });
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::upload(id(1), BOARD, 5, UploadMethod::KeepBest));
    frames(&mut app, 5);
    assert!(errors(&app).is_empty());
    frames(&mut app, 8);
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::TimedOut)]);
    // The find's op was 1: a late answer for it changes nothing.
    fake.push_event(BackendEvent::LeaderboardFound { op: 1, board: 7 });
    frames(&mut app, 2);
    assert!(seen::<LeaderboardFound>(&app).is_empty());
    assert!(!app.world().resource::<SteamLeaderboards>().is_pending(id(1)));

    // An upload that never answers frees the queue after the timeout.
    send(&mut app, LeaderboardRequest::find(id(2), BOARD));
    frames(&mut app, 2);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::upload(id(3), BOARD, 5, UploadMethod::KeepBest));
    send(&mut app, LeaderboardRequest::upload(id(4), BOARD, 6, UploadMethod::KeepBest));
    frames(&mut app, 14);
    assert!(errors(&app).contains(&(3, LeaderboardErrorKind::TimedOut)));
    assert_eq!(seen::<ScoreUploaded>(&app).iter().map(|u| u.id).collect::<Vec<_>>(), vec![id(4)]);
}

#[test]
fn uploads_are_limited_per_window() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings { uploads_per_window: 2, upload_window: Duration::from_secs(3), ..Default::default() });
    for n in 1..=3 {
        send(&mut app, LeaderboardRequest::upload(id(n), BOARD, n as i32, UploadMethod::ForceUpdate));
    }
    frames(&mut app, 10);
    assert_eq!(uploads(&fake), 2);
    assert_eq!(app.world().resource::<SteamLeaderboards>().uploads_queued(), 1);
    frames(&mut app, 25); // the window slides past the first upload
    assert_eq!(uploads(&fake), 3);
    assert_eq!(seen::<ScoreUploaded>(&app).len(), 3);
}

#[test]
fn a_full_upload_queue_refuses_more() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings { max_queued_uploads: 2, ..Default::default() });
    for n in 1..=4 {
        send(&mut app, LeaderboardRequest::upload(id(n), BOARD, 1, UploadMethod::KeepBest));
    }
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(3, LeaderboardErrorKind::QueueFull), (4, LeaderboardErrorKind::QueueFull)]);
}

#[test]
fn without_a_backend_requests_get_no_backend() {
    let mut app = app_with(None, LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 1, UploadMethod::KeepBest));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::NoBackend), (2, LeaderboardErrorKind::NoBackend)]);
}

/// Which frame (1-based) each answer was read in, by a game system in `PreUpdate`.
#[derive(Resource, Default)]
struct ReadFrames(Vec<u32>);

fn read_in_pre_update(mut frame: Local<u32>, mut found: MessageReader<LeaderboardFound>, mut seen: ResMut<ReadFrames>) {
    *frame += 1;
    for _ in found.read() {
        seen.0.push(*frame);
    }
}

#[test]
fn an_answer_is_readable_in_the_frame_it_was_pumped() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    app.init_resource::<ReadFrames>().add_systems(PreUpdate, read_in_pre_update);
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 1); // Update: find started (the fake answers on the next pump)
    let pumps = fake.pump_count();
    frames(&mut app, 1);
    assert_eq!(fake.pump_count(), pumps + 1);
    assert_eq!(app.world().resource::<ReadFrames>().0, vec![2]);
}

#[test]
fn settings_come_from_the_kit_plugin_builder() {
    let app = app_with(None, LeaderboardSettings { max_download_rows: 7, ..Default::default() });
    assert_eq!(app.world().resource::<LeaderboardSettings>().max_download_rows, 7);
    assert_eq!(LeaderboardSettings::default().uploads_per_window, 10);
}

#[test]
fn removing_the_backend_answers_everything_waiting_with_no_backend_once() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 2); // cached
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 1, UploadMethod::KeepBest)); // in flight, never answered
    send(&mut app, LeaderboardRequest::upload(id(3), BOARD, 2, UploadMethod::KeepBest)); // queued
    frames(&mut app, 1);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::find(id(4), "Other")); // running find, never answered
    frames(&mut app, 1);
    app.world_mut().remove_resource::<SteamBackendRes>();
    frames(&mut app, 100);
    let mut errs = errors(&app);
    errs.sort_by_key(|e| e.0);
    assert_eq!(errs, vec![(2, LeaderboardErrorKind::NoBackend), (3, LeaderboardErrorKind::NoBackend), (4, LeaderboardErrorKind::NoBackend)]);
    let lb = app.world().resource::<SteamLeaderboards>();
    assert!(!lb.is_pending(id(2)) && !lb.is_pending(id(3)) && !lb.is_pending(id(4)));
    assert_eq!(lb.uploads_queued(), 0);
}

#[test]
fn app_exit_answers_everything_waiting_with_exiting() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 2);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::upload(id(2), BOARD, 1, UploadMethod::KeepBest));
    send(&mut app, LeaderboardRequest::upload(id(3), BOARD, 2, UploadMethod::KeepBest));
    send(&mut app, LeaderboardRequest::upload(id(4), BOARD, 3, UploadMethod::KeepBest));
    frames(&mut app, 1);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    let mut errs = errors(&app);
    errs.sort_by_key(|e| e.0);
    assert_eq!(errs, vec![(2, LeaderboardErrorKind::Exiting), (3, LeaderboardErrorKind::Exiting), (4, LeaderboardErrorKind::Exiting)]);
    frames(&mut app, 2);
    assert_eq!(errors(&app).len(), 3, "never answered twice");
}

/// A game system that writes a request after the kit's Update set and quits.
#[derive(Resource)]
struct LateRequest(bool);

fn late_request(mut late: ResMut<LateRequest>, mut boards: MessageWriter<LeaderboardRequest>, mut exit: MessageWriter<AppExit>) {
    if std::mem::take(&mut late.0) {
        boards.write(LeaderboardRequest::find(LeaderboardRequestId(9), BOARD));
        exit.write(AppExit::Success);
    }
}

#[test]
fn a_request_written_after_the_update_set_in_the_exit_frame_is_answered_exiting() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    app.insert_resource(LateRequest(true)).add_systems(Update, late_request.after(SteamKitSystems::Requests));
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![(9, LeaderboardErrorKind::Exiting)]);
    frames(&mut app, 2);
    assert_eq!(errors(&app).len(), 1, "never answered twice");
    assert!(seen::<LeaderboardFound>(&app).is_empty());
}

#[test]
fn removing_the_backend_between_pump_and_callbacks_warns_and_answers_no_backend() {
    fn remove_backend(world: &mut World) {
        world.remove_resource::<SteamBackendRes>();
    }
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 1); // find started; its answer is pumped next frame
    app.add_systems(First, remove_backend.after(bevy::time::TimeSystems).after(SteamKitSystems::Pump).before(SteamKitSystems::Callbacks));
    frames(&mut app, 2);
    assert!(seen::<LeaderboardFound>(&app).is_empty());
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::NoBackend)]);
}

#[test]
fn calls_started_without_a_clock_are_timed_from_when_one_appears() {
    let fake = board_fake();
    // No MinimalPlugins: no TimePlugin, so no `Time<Real>` at first.
    let mut app = App::new();
    app.add_plugins(SteamKitPlugin::default().with_leaderboards(LeaderboardSettings { timeout: Duration::from_secs(30), ..Default::default() }))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())));
    watch::<LeaderboardError>(&mut app);
    fake.fail_next_leaderboard_call(FakeLeaderboardFailure::NoAnswer);
    send(&mut app, LeaderboardRequest::find(id(1), BOARD));
    frames(&mut app, 3);
    // A clock appears, already 100 s in: the find must not time out at once.
    let mut clock = Time::<Real>::default();
    clock.update_with_duration(Duration::from_secs(100));
    app.insert_resource(clock);
    frames(&mut app, 2);
    assert!(errors(&app).is_empty());
    app.world_mut().resource_mut::<Time<Real>>().update_with_duration(Duration::from_secs(31));
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![(1, LeaderboardErrorKind::TimedOut)]);
}

#[test]
fn download_ranges_are_capped_at_i32_max() {
    assert!(!range_ok(ScoreRange::Global { first: i32::MAX as u32, last: i32::MAX as u32 + 1 }, u32::MAX));
    assert!(range_ok(ScoreRange::Global { first: i32::MAX as u32, last: i32::MAX as u32 }, u32::MAX));
    assert!(!range_ok(ScoreRange::AroundUser { before: i32::MAX as u32, after: 1 }, u32::MAX));
    assert!(range_ok(ScoreRange::AroundUser { before: i32::MAX as u32 - 1, after: 0 }, u32::MAX));
}

#[test]
fn kit_ids_are_unique_skip_pending_ids_wrap_and_never_reach_the_manual_range() {
    let mut lb = SteamLeaderboards::default();
    let ids: Vec<u64> = (0..5).map(|_| lb.next_id().0).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    lb.pending.insert(id(6));
    lb.pending.insert(id(7));
    assert_eq!(lb.next_id(), id(8), "pending ids are skipped");
    lb.last_issued = LeaderboardRequestId::FIRST_MANUAL - 2;
    assert_eq!(lb.next_id().0, LeaderboardRequestId::FIRST_MANUAL - 1);
    lb.pending.insert(id(1));
    assert_eq!(lb.next_id(), id(2), "wraps to 1, skipping the pending 1");
}

/// A game that takes kit ids for two requests and a hand-picked id for a third, in one frame.
fn mixed_ids(mut lb: ResMut<SteamLeaderboards>, mut boards: MessageWriter<LeaderboardRequest>, mut done: Local<bool>) {
    if std::mem::replace(&mut *done, true) {
        return;
    }
    let a = lb.next_id();
    let b = lb.next_id();
    boards.write(LeaderboardRequest::find(a, BOARD));
    boards.write(LeaderboardRequest::download(b, BOARD, ScoreRange::Friends));
    boards.write(LeaderboardRequest::find(LeaderboardRequestId(LeaderboardRequestId::FIRST_MANUAL + 1), "Missing"));
}

#[test]
fn kit_ids_and_manual_ids_mix_without_collisions() {
    let fake = board_fake();
    let mut app = app_with(Some(&fake), LeaderboardSettings::default());
    app.add_systems(Update, mixed_ids.before(SteamKitSystems::Requests));
    frames(&mut app, 4);
    assert_eq!(seen::<LeaderboardFound>(&app).iter().map(|f| f.id).collect::<Vec<_>>(), vec![id(1)]);
    assert_eq!(seen::<ScoresDownloaded>(&app).iter().map(|d| d.id).collect::<Vec<_>>(), vec![id(2)]);
    assert_eq!(errors(&app), vec![(LeaderboardRequestId::FIRST_MANUAL + 1, LeaderboardErrorKind::NotFound)]);
}
