//! Stats and achievements on real Steam with Valve's public test app 480 ("Spacewar"), which
//! defines the stats `NumGames` / `NumWins` / `NumLosses` (INT) and `FeetTraveled` /
//! `MaxFeetTraveled` (FLOAT) and the achievements `ACH_WIN_ONE_GAME`, `ACH_WIN_100_GAMES`,
//! `ACH_TRAVEL_FAR_ACCUM`, `ACH_TRAVEL_FAR_SINGLE`.
//!
//! Needs the `steam` and `stats` features and a running, logged-in Steam client. Everything it
//! changes shows on your own Spacewar profile. It never prints your SteamID or name.
//!
//! ```text
//! cargo run --example stats_480 --features steam,stats                   # read and print only
//! cargo run --example stats_480 --features steam,stats -- --play         # NumGames +1, FeetTraveled +10.5, unlock ACH_WIN_ONE_GAME, store
//! cargo run --example stats_480 --features steam,stats -- --round-trip   # --play, then try to put every value back and store again
//! cargo run --example stats_480 --features steam,stats -- --progress     # show the ACH_WIN_100_GAMES 5/100 progress popup
//! cargo run --example stats_480 --features steam,stats -- --reset-stats  # reset every Spacewar STAT to 0 (achievements kept)
//! cargo run --example stats_480 --features steam,stats -- --reset        # reset ALL Spacewar stats and achievements (development)
//! ```
//!
//! Spacewar's stats only go up: Steam refuses a `SetStat` that lowers one (a `Refused` error), so
//! `--round-trip` can put the achievement back but not the stats; `--reset-stats` sets every stat
//! back to 0 (and so also wipes any stat you had before). The example prints every Spacewar stat
//! and achievement at the start and at the end, and quits by itself when done, or after 90 seconds.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const GAMES: &str = "NumGames";
const FEET: &str = "FeetTraveled";
const WIN_ONE: &str = "ACH_WIN_ONE_GAME";
const WIN_100: &str = "ACH_WIN_100_GAMES";
const INT_STATS: [&str; 3] = ["NumGames", "NumWins", "NumLosses"];
const FLOAT_STATS: [&str; 3] = ["FeetTraveled", "MaxFeetTraveled", "AverageSpeed"];
const ACHIEVEMENTS: [&str; 4] = ["ACH_WIN_ONE_GAME", "ACH_WIN_100_GAMES", "ACH_TRAVEL_FAR_ACCUM", "ACH_TRAVEL_FAR_SINGLE"];
const TIME_LIMIT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Status,
    Play,
    RoundTrip,
    Progress,
    ResetStats,
    Reset,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    WaitReady,
    WaitFirstStore,
    WaitRestoreStore,
    WaitProgress,
    Done,
}

/// What the stats were when the example started.
#[derive(Clone, Copy, Debug)]
struct Snapshot {
    games: Option<i32>,
    feet: Option<f32>,
    win_one: Option<bool>,
}

#[derive(Resource)]
struct Run {
    mode: Mode,
    phase: Phase,
    before: Option<Snapshot>,
}

fn main() {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let mode = match std::env::args().nth(1).as_deref() {
        Some("--play") => Mode::Play,
        Some("--round-trip") => Mode::RoundTrip,
        Some("--progress") => Mode::Progress,
        Some("--reset-stats") => Mode::ResetStats,
        Some("--reset") => Mode::Reset,
        _ => Mode::Status,
    };
    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return;
        }
    };
    println!("Steam is up: app {app_id}, mode {mode:?}");

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))),
            LogPlugin::default(),
            // Probe readiness with a stat Spacewar defines.
            SteamKitPlugin::default().with_stats(StatsSettings { probe: Some(GAMES.into()), ..Default::default() }),
        ))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(Run { mode, phase: Phase::WaitReady, before: None })
        .add_systems(Update, (drive, print_facts).before(SteamKitSystems::Requests))
        .add_systems(Update, time_limit)
        .run();
}

/// Every Spacewar stat and achievement, on one line.
fn print_all(backend: &SteamBackendRes, t: f32, when: &str) {
    let Some(stats) = backend.0.stats() else { return };
    let mut line = format!("[{t:6.2}s] {when}:");
    for name in INT_STATS {
        line.push_str(&format!(" {name}={:?}", stats.get_stat(name, StatKind::I32)));
    }
    for name in FLOAT_STATS {
        line.push_str(&format!(" {name}={:?}", stats.get_stat(name, StatKind::F32)));
    }
    for name in ACHIEVEMENTS {
        line.push_str(&format!(" {name}={:?}", stats.achievement(name)));
    }
    println!("{line}");
}

fn snapshot(backend: &SteamBackendRes) -> Snapshot {
    let stats = backend.0.stats();
    Snapshot {
        games: stats.and_then(|s| s.get_stat(GAMES, StatKind::I32)).and_then(|v| if let StatValue::I32(v) = v { Some(v) } else { None }),
        feet: stats.and_then(|s| s.get_stat(FEET, StatKind::F32)).and_then(|v| if let StatValue::F32(v) = v { Some(v) } else { None }),
        win_one: stats.and_then(|s| s.achievement(WIN_ONE)),
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct Facts<'w, 's> {
    ready: MessageReader<'w, 's, StatsReady>,
    stored: MessageReader<'w, 's, StatsStored>,
    progress: MessageReader<'w, 's, AchievementProgress>,
}

fn drive(
    mut run: ResMut<Run>,
    backend: Res<SteamBackendRes>,
    mut facts: Facts,
    mut req: MessageWriter<StatsRequest>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    let was_ready = facts.ready.read().count() > 0;
    let was_stored = facts.stored.read().count() > 0;
    let saw_progress = facts.progress.read().count() > 0;
    match run.phase {
        Phase::WaitReady if was_ready => {
            let before = snapshot(&backend);
            println!("[{t:6.2}s] ready. {GAMES} = {:?}, {FEET} = {:?}, {WIN_ONE} unlocked = {:?}", before.games, before.feet, before.win_one);
            run.before = Some(before);
            print_all(&backend, t, "all at start");
            match run.mode {
                Mode::Status => run.phase = Phase::Done,
                Mode::Play | Mode::RoundTrip => {
                    req.write(StatsRequest::AddStat { name: GAMES.into(), delta: StatValue::I32(1) });
                    req.write(StatsRequest::AddStat { name: FEET.into(), delta: StatValue::F32(10.5) });
                    req.write(StatsRequest::UnlockAchievement { name: WIN_ONE.into() });
                    req.write(StatsRequest::StoreStats);
                    println!("[{t:6.2}s] sent: {GAMES} +1, {FEET} +10.5, unlock {WIN_ONE}, store");
                    run.phase = Phase::WaitFirstStore;
                }
                Mode::Progress => {
                    req.write(StatsRequest::IndicateAchievementProgress { name: WIN_100.into(), current: 5, max: 100 });
                    println!("[{t:6.2}s] sent: progress {WIN_100} 5/100");
                    run.phase = Phase::WaitProgress;
                }
                Mode::ResetStats => {
                    req.write(StatsRequest::ResetAllStats { achievements_too: false });
                    println!("[{t:6.2}s] sent: reset every stat (achievements kept)");
                    run.phase = Phase::WaitFirstStore;
                }
                Mode::Reset => {
                    req.write(StatsRequest::ResetAllStats { achievements_too: true });
                    println!("[{t:6.2}s] sent: reset ALL stats and achievements");
                    run.phase = Phase::WaitFirstStore;
                }
            }
        }
        Phase::WaitFirstStore if was_stored => {
            let now = snapshot(&backend);
            println!("[{t:6.2}s] stored. {GAMES} = {:?}, {FEET} = {:?}, {WIN_ONE} unlocked = {:?}", now.games, now.feet, now.win_one);
            let before = run.before;
            match (run.mode, before) {
                (Mode::RoundTrip, Some(before)) => {
                    if let Some(games) = before.games {
                        req.write(StatsRequest::SetStat { name: GAMES.into(), value: StatValue::I32(games) });
                    }
                    if let Some(feet) = before.feet {
                        req.write(StatsRequest::SetStat { name: FEET.into(), value: StatValue::F32(feet) });
                    }
                    if before.win_one == Some(false) {
                        req.write(StatsRequest::ClearAchievement { name: WIN_ONE.into() });
                    }
                    req.write(StatsRequest::StoreStats);
                    println!("[{t:6.2}s] sent: restore the starting values, store (after the minimum store gap)");
                    run.phase = Phase::WaitRestoreStore;
                }
                _ => run.phase = Phase::Done,
            }
        }
        Phase::WaitRestoreStore if was_stored => {
            let now = snapshot(&backend);
            println!("[{t:6.2}s] restored and stored. {GAMES} = {:?}, {FEET} = {:?}, {WIN_ONE} unlocked = {:?}", now.games, now.feet, now.win_one);
            if let Some(before) = run.before {
                let same = now.games == before.games && now.feet == before.feet && now.win_one == before.win_one;
                println!("[{t:6.2}s] round trip {}", if same { "OK: every value is back as it was" } else { "MISMATCH: values differ from the start" });
            }
            run.phase = Phase::Done;
        }
        Phase::WaitProgress if saw_progress => run.phase = Phase::Done,
        _ => {}
    }
    if run.phase == Phase::Done {
        print_all(&backend, t, "all at end");
        println!("[{t:6.2}s] done");
        exit.write(AppExit::Success);
    }
}

fn print_facts(
    mut unlocked: MessageReader<AchievementUnlocked>,
    mut progress: MessageReader<AchievementProgress>,
    mut errors: MessageReader<StatsError>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    for ev in unlocked.read() {
        println!("[{t:6.2}s] achievement unlocked (confirmed by Steam): {}", ev.name);
    }
    for ev in progress.read() {
        println!("[{t:6.2}s] achievement progress shown: {} {}/{}", ev.name, ev.current, ev.max);
    }
    for err in errors.read() {
        println!("[{t:6.2}s] stats error {:?} {:?}: {}", err.kind, err.name, err.message);
    }
}

fn time_limit(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>, mut said: Local<bool>) {
    if time.elapsed() > TIME_LIMIT && !*said {
        *said = true;
        println!("time limit reached - quitting");
        exit.write(AppExit::error());
    }
}
