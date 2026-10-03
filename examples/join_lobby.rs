//! Join a real Steam lobby: either the lobby id given on the command line, or whatever lobby a
//! friend's "Join Game" / invite points at (including a cold launch with `+connect_lobby <id>`).
//! On entering, it prints the lobby data the host set and the member count; a real game would now
//! connect its networking to the host. Stop it with Ctrl+C.
//!
//! Needs the `steam` feature, a running and logged-in Steam client, and (the usual development
//! setup) a `steam_appid.txt` in the working directory containing your app id (`480` = Valve's test
//! app "Spacewar" for development). The app id is read from the `STEAM_APP_ID` environment
//! variable (default 480) and passed to `Client::init_app`. With app 480,
//! "Join Game" only reaches a copy of this example that is ALREADY RUNNING: Steam would otherwise
//! try to launch Spacewar itself.
//!
//! ```text
//! echo 480 > steam_appid.txt
//! cargo run --example join_lobby --features steam              # wait for "Join Game" / an invite
//! cargo run --example join_lobby --features steam -- <lobby id> # join a lobby directly
//! ```

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

/// The lobby id from the command line, joined on startup.
#[derive(Resource)]
struct JoinOnStart(Option<u64>);

fn main() -> AppExit {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    // A bare id; a `+connect_lobby <id>` from Steam is picked up by the kit itself.
    let lobby = std::env::args().nth(1).and_then(|a| a.trim().parse::<u64>().ok());

    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return AppExit::error();
        }
    };
    // Warm up the relay network now, so a P2P connection to the host does not wait for it.
    client.networking_utils().init_relay_network_access();
    println!("Steam is up: app {app_id}, you are {} ({})", client.friends().name(), client.user().steam_id().raw());
    if lobby.is_none() {
        println!("Waiting for a join request: use \"Join Game\" on a friend who runs host_lobby.");
    }

    App::new()
        .add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))), LogPlugin::default(), SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(JoinOnStart(lobby))
        .add_systems(Startup, join_on_start)
        .add_systems(Update, (on_join_requested, on_entered, on_error).before(SteamKitSystems::Requests))
        .run()
}

fn join_on_start(start: Res<JoinOnStart>, mut join: MessageWriter<JoinLobby>) {
    if let Some(lobby) = start.0 {
        println!("Joining lobby {lobby}...");
        join.write(JoinLobby { lobby });
    }
}

/// The game decides: this example always accepts.
fn on_join_requested(mut requests: MessageReader<JoinRequested>, backend: Res<SteamBackendRes>, mut join: MessageWriter<JoinLobby>) {
    for req in requests.read() {
        let name = if req.from == 0 { "(unknown)".to_string() } else { backend.0.friend_name(req.from) };
        println!("Join request via {:?}: lobby {} from {name} -> joining", req.source, req.lobby);
        join.write(JoinLobby { lobby: req.lobby });
    }
}

fn on_entered(mut entered: MessageReader<LobbyEntered>, backend: Res<SteamBackendRes>) {
    for ev in entered.read() {
        let Some(b) = backend.0.lobby() else { continue };
        let host = b.lobby_data(ev.lobby, "host");
        let version = b.lobby_data(ev.lobby, "version");
        println!(
            "In lobby {}: {} member(s), host = {host:?}, version = {version:?}. A game would now connect to the host.",
            ev.lobby,
            b.lobby_member_count(ev.lobby)
        );
    }
}

fn on_error(mut errors: MessageReader<LobbyError>) {
    for err in errors.read() {
        eprintln!("Lobby error {:?}: {}", err.kind, err.message);
    }
}
