//! The leaderboards feature's public surface from a game's point of view: the kit in a strict
//! headless app (ambiguity detection = Error on every main schedule), the game's own systems
//! ordered around the public sets, and the in-memory `FakeSteamBackend`.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_steam_kit::*;

const BOARD: &str = "Quickest Win";

/// The game finished a run in this many seconds.
#[derive(Resource)]
struct Finished(Option<i32>);

/// What the game shows: the top of the board after its own upload.
#[derive(Resource, Default)]
struct Shown {
    rank: Option<i32>,
    top: Vec<(i32, u64, i32)>,
    errors: Vec<LeaderboardErrorKind>,
}

fn post_score(mut finished: ResMut<Finished>, mut boards: MessageWriter<LeaderboardRequest>) {
    if let Some(secs) = finished.0.take() {
        boards.write(LeaderboardRequest::UploadScore {
            id: LeaderboardRequestId(1),
            board: BOARD.into(),
            score: secs,
            details: vec![3, 1],
            method: UploadMethod::KeepBest,
        });
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct Answers<'w, 's> {
    uploaded: MessageReader<'w, 's, ScoreUploaded>,
    downloaded: MessageReader<'w, 's, ScoresDownloaded>,
    errors: MessageReader<'w, 's, LeaderboardError>,
}

/// After our upload, ask for the top 3; show what arrives.
fn show(mut answers: Answers, mut boards: MessageWriter<LeaderboardRequest>, mut shown: ResMut<Shown>) {
    for up in answers.uploaded.read() {
        shown.rank = Some(up.rank_new);
        boards.write(LeaderboardRequest::download(LeaderboardRequestId(2), BOARD, ScoreRange::Global { first: 1, last: 3 }));
    }
    for d in answers.downloaded.read() {
        shown.top = d.entries.iter().map(|e| (e.rank, e.steam_id, e.score)).collect();
    }
    shown.errors.extend(answers.errors.read().map(|e| e.kind));
}

fn game(fake: &FakeSteamBackend) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .insert_resource(Finished(None))
        .init_resource::<Shown>()
        .add_systems(Update, (post_score, show).chain().before(SteamKitSystems::Requests));
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
fn a_run_is_posted_and_the_top_of_the_board_shown() {
    let fake = FakeSteamBackend::new();
    fake.add_leaderboard(BOARD, LeaderboardSort::Ascending, LeaderboardDisplay::TimeSeconds);
    fake.add_leaderboard_entry(BOARD, 76_561_197_960_265_730, 95, &[]);
    fake.add_leaderboard_entry(BOARD, 76_561_197_960_265_731, 140, &[]);
    let mut app = game(&fake);
    app.insert_resource(Finished(Some(120)));
    frames(&mut app, 6);
    let shown = app.world().resource::<Shown>();
    assert_eq!(shown.rank, Some(2));
    assert_eq!(shown.top, vec![(1, 76_561_197_960_265_730, 95), (2, 76_561_197_960_265_729, 120), (3, 76_561_197_960_265_731, 140)]);
    assert!(shown.errors.is_empty(), "{:?}", shown.errors);
}

#[test]
fn posting_to_a_board_that_does_not_exist_is_not_found_and_nothing_is_created() {
    let fake = FakeSteamBackend::new();
    let mut app = game(&fake);
    app.insert_resource(Finished(Some(120)));
    frames(&mut app, 4);
    assert_eq!(app.world().resource::<Shown>().errors, vec![LeaderboardErrorKind::NotFound]);
    assert!(fake.calls().contains(&FakeCall::FindLeaderboard { name: BOARD.into(), create: false }));
    assert!(fake.leaderboard_entries(BOARD).is_empty());
}
