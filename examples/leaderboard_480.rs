//! Leaderboards on real Steam with Valve's public test app 480 ("Spacewar"). Its SDK sample uses
//! the boards "Feet Traveled" and "Quickest Win"; everyone testing with 480 shares them.
//!
//! Needs the `steam` and `leaderboards` features and a running, logged-in Steam client. The
//! example only FINDS boards (it never creates one) and prints ranks and scores; SteamIDs are
//! printed only with `--show-ids`. An upload is public and cannot be deleted by the game: it uses
//! `KeepBest`, so it never replaces a better score of yours.
//!
//! ```text
//! cargo run --example leaderboard_480 --features steam,leaderboards                         # find + download (read only)
//! cargo run --example leaderboard_480 --features steam,leaderboards -- --upload 1           # KeepBest upload of 1, then download around you
//! cargo run --example leaderboard_480 --features steam,leaderboards -- --board "Quickest Win"
//! ```
//!
//! Request ids come from `SteamLeaderboards::next_id` (the recommended way).
//!
//! The example quits by itself when every answer arrived, or after 60 seconds.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const TIME_LIMIT: Duration = Duration::from_secs(60);

#[derive(Resource)]
struct Plan {
    board: String,
    upload: Option<i32>,
    show_ids: bool,
    /// Answers still expected.
    waiting: usize,
    started: bool,
    /// An error answer arrived: exit with an error code.
    failed: bool,
}

fn main() -> AppExit {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let board = arg("--board").unwrap_or_else(|| "Feet Traveled".to_string());
    let upload = arg("--upload").and_then(|v| v.parse::<i32>().ok());
    let show_ids = args.iter().any(|a| a == "--show-ids");

    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return AppExit::error();
        }
    };
    println!("Steam is up: app {app_id}, board {board:?}, upload {upload:?}");

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))),
            LogPlugin::default(),
            SteamKitPlugin::default().with_leaderboards(LeaderboardSettings::default()),
        ))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(Plan { board, upload, show_ids, waiting: 0, started: false, failed: false })
        .add_systems(Update, (start, answers).chain().before(SteamKitSystems::Requests))
        .add_systems(Update, time_limit)
        .run()
}

fn start(mut plan: ResMut<Plan>, mut ids: ResMut<SteamLeaderboards>, mut boards: MessageWriter<LeaderboardRequest>, time: Res<Time<Real>>) {
    if plan.started {
        return;
    }
    plan.started = true;
    let board = plan.board.clone();
    boards.write(LeaderboardRequest::find(ids.next_id(), board.clone()));
    boards.write(LeaderboardRequest::download(ids.next_id(), board.clone(), ScoreRange::Global { first: 1, last: 10 }));
    boards.write(LeaderboardRequest::download(ids.next_id(), board.clone(), ScoreRange::Friends));
    plan.waiting = 3;
    if let Some(score) = plan.upload {
        boards.write(LeaderboardRequest::upload(ids.next_id(), board, score, UploadMethod::KeepBest));
        plan.waiting += 1;
    } else {
        boards.write(LeaderboardRequest::download(ids.next_id(), board, ScoreRange::AroundUser { before: 2, after: 2 }));
        plan.waiting += 1;
    }
    println!("[{:6.2}s] sent {} request(s)", time.elapsed().as_secs_f32(), plan.waiting);
}

#[derive(bevy::ecs::system::SystemParam)]
struct Answers<'w, 's> {
    found: MessageReader<'w, 's, LeaderboardFound>,
    uploaded: MessageReader<'w, 's, ScoreUploaded>,
    downloaded: MessageReader<'w, 's, ScoresDownloaded>,
    errors: MessageReader<'w, 's, LeaderboardError>,
}

fn answers(
    mut plan: ResMut<Plan>,
    mut a: Answers,
    mut boards: MessageWriter<LeaderboardRequest>,
    mut ids: ResMut<SteamLeaderboards>,
    mut exit: MessageWriter<AppExit>,
    backend: Res<SteamBackendRes>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    let me = backend.0.local_id();
    for f in a.found.read() {
        plan.waiting -= 1;
        println!("[{t:6.2}s] #{} found {:?}: sort {:?}, display {:?}, {} entries", f.id.0, f.info.name, f.info.sort, f.info.display, f.info.entry_count);
    }
    for u in a.uploaded.read() {
        plan.waiting -= 1;
        println!("[{t:6.2}s] #{} uploaded {}: changed {}, rank {} -> {}", u.id.0, u.score, u.changed, u.rank_previous, u.rank_new);
        // Then show the neighbourhood.
        boards.write(LeaderboardRequest::download(ids.next_id(), u.board.clone(), ScoreRange::AroundUser { before: 2, after: 2 }));
        plan.waiting += 1;
    }
    for d in a.downloaded.read() {
        plan.waiting -= 1;
        println!("[{t:6.2}s] #{} downloaded {:?}: {} entr(ies) of {}", d.id.0, d.range, d.entries.len(), d.entry_count);
        for e in &d.entries {
            let who = if e.steam_id == me {
                "you".to_string()
            } else if plan.show_ids {
                e.steam_id.to_string()
            } else {
                "a player".to_string()
            };
            println!("           rank {:>6}  score {:>10}  {who}", e.rank, e.score);
        }
    }
    for e in a.errors.read() {
        plan.waiting = plan.waiting.saturating_sub(1);
        plan.failed = true;
        println!("[{t:6.2}s] #{} error {:?}: {}", e.id.0, e.kind, e.message);
    }
    if plan.started && plan.waiting == 0 {
        println!("[{t:6.2}s] done");
        exit.write(if plan.failed { AppExit::error() } else { AppExit::Success });
    }
}

fn time_limit(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>, mut said: Local<bool>) {
    if time.elapsed() > TIME_LIMIT && !*said {
        *said = true;
        println!("time limit reached - quitting");
        exit.write(AppExit::error());
    }
}
