//! The Steam overlay on real Steam with Valve's public test app 480 ("Spacewar").
//!
//! Needs the `steam` and `overlay` features and a running, logged-in Steam client. The overlay
//! draws over a game's own window; this example has no window, so whether anything shows from a
//! `cargo run` process is exactly what it checks: it prints whether Steam reports the overlay as
//! available, sends one request after 3 seconds, and prints every open / closed event for 60
//! seconds (press Shift+Tab where the overlay shows).
//!
//! ```text
//! cargo run --example overlay_480 --features steam,overlay                     # the store page of 480
//! cargo run --example overlay_480 --features steam,overlay -- --friends        # the friends dialog
//! cargo run --example overlay_480 --features steam,overlay -- --invite-dialog  # the invite dialog with a connect string
//! cargo run --example overlay_480 --features steam,overlay -- --invite-dialog-lobby <lobby id>  # the lobby invite dialog
//! ```
//!
//! For the lobby invite dialog, start `host_lobby` first and pass the lobby id it prints.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const RUN_FOR: Duration = Duration::from_secs(60);

#[derive(Resource)]
struct Plan {
    request: Option<OpenOverlay>,
    last_enabled: Option<bool>,
    /// An error arrived: exit with an error code.
    failed: bool,
}

fn main() -> AppExit {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lobby = args.iter().position(|a| a == "--invite-dialog-lobby").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let request = if let Some(lobby) = lobby {
        OpenOverlay::invite_dialog(lobby)
    } else if args.iter().any(|a| a == "--friends") {
        OpenOverlay::dialog("friends")
    } else if args.iter().any(|a| a == "--invite-dialog") {
        OpenOverlay::invite_dialog_connect("+bevy_steam_kit_test 1")
    } else {
        OpenOverlay::store(app_id)
    };
    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return AppExit::error();
        }
    };
    println!("Steam is up: app {app_id}; request after 3 s: {request:?}");

    App::new()
        .add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))), LogPlugin::default(), SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(Plan { request: Some(request), last_enabled: None, failed: false })
        .add_systems(Update, run.after(SteamKitSystems::Requests))
        .run()
}

fn run(
    mut plan: ResMut<Plan>,
    overlay: Res<SteamOverlay>,
    mut open: MessageWriter<OpenOverlay>,
    mut toggled: MessageReader<OverlayToggled>,
    mut errors: MessageReader<OverlayError>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time<Real>>,
) {
    let t = time.elapsed().as_secs_f32();
    if plan.last_enabled != Some(overlay.is_enabled()) {
        plan.last_enabled = Some(overlay.is_enabled());
        println!("[{t:6.2}s] Steam reports the overlay as available: {}", overlay.is_enabled());
    }
    if t >= 3.0 {
        if let Some(r) = plan.request.take() {
            println!("[{t:6.2}s] sending {r:?}");
            open.write(r);
        }
    }
    for e in toggled.read() {
        println!("[{t:6.2}s] overlay {}", if e.active { "OPENED" } else { "closed" });
    }
    for e in errors.read() {
        plan.failed = true;
        println!("[{t:6.2}s] error {:?}: {}", e.kind, e.message);
    }
    if time.elapsed() > RUN_FOR {
        println!("[{t:6.2}s] {} open/close events - quitting", overlay.toggles());
        exit.write(if plan.failed { AppExit::error() } else { AppExit::Success });
    }
}
