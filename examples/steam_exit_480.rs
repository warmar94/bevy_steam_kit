//! Quitting Steam while a game runs, on real Steam with Valve's public test app 480 ("Spacewar").
//!
//! Needs the `steam`, `auth` and `friends` features and a running, logged-in Steam client. Start
//! the example, then quit Steam (its menu, or `steam://exit`). The example keeps running: it
//! prints when the kit reports `SteamLost` (its systems keep using `Res<SteamBackendRes>`), sends a Web API ticket request and a user-info request
//! after that and prints their answers (`NoBackend`), and quits by itself 10 seconds later with
//! exit code 0. Nothing is sent anywhere; no names, ids or ticket bytes are printed.
//!
//! ```text
//! cargo run --example steam_exit_480 --features steam,auth,friends
//! ```
//!
//! Without a Steam exit it quits after 3 minutes (exit code 1).

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const WAIT_FOR_EXIT: Duration = Duration::from_secs(180);
const AFTER_LOSS: f32 = 10.0;

#[derive(Resource, Default)]
struct Run {
    lost_at: Option<f32>,
    last_alive: f32,
    answers: u32,
}

fn main() -> AppExit {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return AppExit::error();
        }
    };
    println!("Steam is up: app {app_id}. Quit Steam now (its menu, or steam://exit); this app keeps running.");

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))),
            LogPlugin::default(),
            SteamKitPlugin::default().with_auth(AuthSettings::default()).with_friends(FriendsSettings::default()),
        ))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .init_resource::<Run>()
        .add_systems(Update, (on_lost, alive).before(SteamKitSystems::Requests))
        .add_systems(Last, answers.after(SteamKitSystems::Requests))
        .run()
}

fn on_lost(
    mut lost: MessageReader<SteamLost>,
    mut run: ResMut<Run>,
    mut auth: ResMut<SteamAuth>,
    mut tickets: MessageWriter<AuthRequest>,
    mut user_info: MessageWriter<RequestUserInfo>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    for l in lost.read() {
        println!("[{t:6.2}s] SteamLost {:?}: the kit stopped pumping Steam; sending two requests now", l.reason);
        run.lost_at = Some(t);
        let id = auth.next_id();
        tickets.write(AuthRequest::web_api_ticket(id, "bevy_steam_kit_live"));
        // A fabricated id from the test range: any id gets the same answer without Steam.
        user_info.write(RequestUserInfo { steam_id: 76_561_197_960_265_731, name_only: true });
    }
}

fn alive(mut run: ResMut<Run>, backend: Res<SteamBackendRes>, friends: Res<SteamFriends>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    let t = time.elapsed().as_secs_f32();
    if t - run.last_alive >= 5.0 {
        run.last_alive = t;
        println!("[{t:6.2}s] running; Steam features available: {}, friends loaded: {}", backend.0.friends().is_some(), friends.is_loaded());
    }
    match run.lost_at {
        Some(at) if t - at >= AFTER_LOSS => {
            println!("[{t:6.2}s] still running {AFTER_LOSS} s after Steam quit, {} answer(s) received - done", run.answers);
            exit.write(if run.answers == 2 { AppExit::Success } else { AppExit::error() });
        }
        None if time.elapsed() > WAIT_FOR_EXIT => {
            println!("[{t:6.2}s] Steam did not quit within {} s - quitting", WAIT_FOR_EXIT.as_secs());
            exit.write(AppExit::error());
        }
        _ => {}
    }
}

fn answers(mut run: ResMut<Run>, mut auth_errors: MessageReader<AuthError>, mut friends_errors: MessageReader<FriendsError>, time: Res<Time<Real>>) {
    let t = time.elapsed().as_secs_f32();
    for e in auth_errors.read() {
        println!("[{t:6.2}s] ticket request answered: {:?}", e.kind);
        run.answers += 1;
    }
    for e in friends_errors.read() {
        println!("[{t:6.2}s] user-info request answered: {:?}", e.kind);
        run.answers += 1;
    }
}
