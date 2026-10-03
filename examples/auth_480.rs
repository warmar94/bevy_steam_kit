//! Steam Web API tickets on real Steam with Valve's public test app 480 ("Spacewar").
//!
//! Needs the `steam` and `auth` features and a running, logged-in Steam client. The example asks
//! for a ticket for the identity `bevy_steam_kit_live`, prints only its LENGTH and how long Steam
//! took (never the bytes or the hex), cancels it, asks again and cancels that request before the
//! answer can arrive. Nothing is sent anywhere.
//!
//! ```text
//! cargo run --example auth_480 --features steam,auth
//! ```
//!
//! The example quits by itself when it is done, or after 60 seconds.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const TIME_LIMIT: Duration = Duration::from_secs(60);
const IDENTITY: &str = "bevy_steam_kit_live";

/// The steps of the run.
#[derive(Resource, Default)]
struct Run {
    step: u32,
    first: Option<AuthRequestId>,
    second: Option<AuthRequestId>,
    asked_at: f32,
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
    println!("Steam is up: app {app_id}");

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))),
            LogPlugin::default(),
            SteamKitPlugin::default().with_auth(AuthSettings::default()),
        ))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .init_resource::<Run>()
        .add_systems(Update, step.before(SteamKitSystems::Requests))
        .add_systems(Update, time_limit)
        .run()
}

fn step(
    mut run: ResMut<Run>,
    mut auth: ResMut<SteamAuth>,
    mut requests: MessageWriter<AuthRequest>,
    mut ready: MessageReader<WebApiTicketReady>,
    mut errors: MessageReader<AuthError>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    if run.step == 0 {
        let id = auth.next_id();
        requests.write(AuthRequest::web_api_ticket(id, IDENTITY));
        run.first = Some(id);
        run.asked_at = t;
        run.step = 1;
        println!("[{t:6.2}s] 1. ticket requested for identity {IDENTITY:?}");
    }
    for r in ready.read() {
        // Length and timing only: the ticket is a credential.
        println!("[{t:6.2}s] ticket #{} ready: {} bytes, {} hex chars, after {:.2}s", r.id.0, r.ticket.len(), r.ticket.to_hex().len(), t - run.asked_at);
        if Some(r.id) == run.first && run.step == 1 {
            requests.write(AuthRequest::cancel(r.id));
            println!("[{t:6.2}s] 2. ticket #{} cancelled", r.id.0);
            let id = auth.next_id();
            requests.write(AuthRequest::web_api_ticket(id, IDENTITY));
            requests.write(AuthRequest::cancel(id));
            run.second = Some(id);
            run.step = 2;
            println!("[{t:6.2}s] 3. second ticket requested and cancelled in the same frame");
        }
    }
    for e in errors.read() {
        println!("[{t:6.2}s] ticket #{} error {:?}: {}", e.id.0, e.kind, e.message);
        if Some(e.id) == run.second && e.kind == AuthErrorKind::Cancelled {
            println!("[{t:6.2}s] done (live tickets: {})", auth.live_tickets());
            exit.write(AppExit::Success);
        } else if Some(e.id) == run.first {
            exit.write(AppExit::error());
        }
    }
}

fn time_limit(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>, mut said: Local<bool>) {
    if time.elapsed() > TIME_LIMIT && !*said {
        *said = true;
        println!("time limit reached - quitting");
        exit.write(AppExit::error());
    }
}
