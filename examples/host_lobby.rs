//! Host a real Steam lobby: creates a friends-only lobby, sets rich presence so friends see
//! "Join Game", prints every lobby fact, and (optionally) invites one friend. Stop it with Ctrl+C;
//! the lobby is left and rich presence cleared on exit.
//!
//! Needs the `steam` feature, a running and logged-in Steam client, and (the usual development
//! setup) a `steam_appid.txt` in the working directory containing your app id. For development use
//! `480` (Valve's public test app "Spacewar"); Steam will show "Spacewar" as the game being played.
//! The app id is read from the `STEAM_APP_ID` environment variable (default 480) and passed to
//! `Client::init_app`.
//!
//! ```text
//! echo 480 > steam_appid.txt
//! cargo run --example host_lobby --features steam
//! cargo run --example host_lobby --features steam -- <friend SteamID64 to invite>
//! ```
//!
//! On a second machine (another Steam account that is a friend of this one), run the
//! `join_lobby` example and use "Join Game" in the Steam friends list, or pass the lobby id this
//! example prints.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

/// The friend to invite once the lobby is open (from the command line), if any.
#[derive(Resource)]
struct InviteOnOpen(Option<u64>);

fn main() {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let invite = std::env::args().nth(1).and_then(|a| a.trim().parse::<u64>().ok());

    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return;
        }
    };
    println!("Steam is up: app {app_id}, you are {} ({})", client.friends().name(), client.user().steam_id().raw());

    App::new()
        .add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))), LogPlugin::default(), SteamKitPlugin::default()))
        // The backend keeps a clone of the client alive for as long as the app runs.
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(InviteOnOpen(invite))
        .add_systems(Startup, create)
        .add_systems(Update, (on_created, on_join_requested, on_invite_sent, on_error).before(SteamKitSystems::Requests))
        .run();
}

fn create(mut create: MessageWriter<CreateLobby>, backend: Res<SteamBackendRes>) {
    let me = backend.0.local_id();
    println!("Creating a friends-only lobby...");
    create.write(CreateLobby {
        kind: LobbyKind::FriendsOnly,
        max_members: 4,
        // Your own keys: here, who hosts and which build it is.
        data: vec![("host".into(), me.to_string()), ("version".into(), "1".into())],
    });
}

fn on_created(
    mut created: MessageReader<LobbyCreated>,
    invite: Res<InviteOnOpen>,
    mut presence: MessageWriter<SetRichPresence>,
    mut invites: MessageWriter<InviteFriend>,
) {
    for ev in created.read() {
        println!("Lobby {} is open. Friends now see \"Join Game\"; a joiner can also pass this id.", ev.lobby);
        // `connect` was already set by the kit; add your own keys on top.
        presence.write(SetRichPresence { key: "status".into(), value: Some("Hosting a lobby".into()) });
        if let Some(friend) = invite.0 {
            invites.write(InviteFriend { steam_id: friend });
        }
    }
}

fn on_join_requested(mut requests: MessageReader<JoinRequested>) {
    for req in requests.read() {
        // A host usually ignores these (or asks the player whether to switch lobbies).
        println!("Join request for lobby {} from {} via {:?} (ignored while hosting)", req.lobby, req.from, req.source);
    }
}

fn on_invite_sent(mut sent: MessageReader<InviteSent>) {
    for ev in sent.read() {
        // `ok` means Steam accepted the call; there is no delivery receipt.
        println!("Invite to {} for lobby {}: ok = {}", ev.steam_id, ev.lobby, ev.ok);
    }
}

fn on_error(mut errors: MessageReader<LobbyError>) {
    for err in errors.read() {
        eprintln!("Lobby error {:?}: {}", err.kind, err.message);
    }
}
