//! The README's quick start, runnable anywhere: no Steam client, no network. It uses the in-memory
//! `FakeSteamBackend`, so the whole flow is deterministic:
//!
//! 1. host a friends-only lobby with two data keys (`LobbyCreated`),
//! 2. a (simulated) friend invites us to their lobby (`JoinRequested`),
//! 3. the game accepts by sending `JoinLobby` (the kit never joins by itself),
//! 4. we leave our own lobby and enter theirs (`LobbyLeft`, `LobbyEntered`) and read its data.
//!
//! `cargo run --example quick_start`

use bevy::prelude::*;
use bevy_steam_kit::*;

/// An obviously made-up friend (an individual SteamID64 in the public universe).
const FRIEND: u64 = 76_561_197_960_265_730;
/// The friend's lobby.
const FRIEND_LOBBY: u64 = 4242;

/// The test's handle on the fake Steam (every clone shares the same state).
#[derive(Resource)]
struct Fake(FakeSteamBackend);

fn main() {
    let fake = FakeSteamBackend::new();
    // What the friend's game wrote into their lobby.
    fake.put_lobby_data(FRIEND_LOBBY, "host", "76561197960265730");
    fake.put_lobby_data(FRIEND_LOBBY, "version", "3");

    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        // With real Steam this is `RealSteamBackend::new(client)` (feature `steam`).
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .insert_resource(Fake(fake))
        .add_systems(Startup, host)
        .add_systems(Update, (on_created, on_join_requested, on_entered, on_left, on_error).before(SteamKitSystems::Requests))
        .add_systems(Update, stop_after_a_few_frames)
        .run();
}

/// Host a lobby. The data is whatever YOUR game needs a joiner to know.
fn host(mut create: MessageWriter<CreateLobby>) {
    create.write(CreateLobby {
        kind: LobbyKind::FriendsOnly,
        max_members: 4,
        data: vec![("host".into(), "76561197960265729".into()), ("version".into(), "3".into())],
    });
}

fn on_created(mut created: MessageReader<LobbyCreated>, fake: Res<Fake>) {
    for ev in created.read() {
        println!("LobbyCreated: lobby {} (rich presence connect = {:?})", ev.lobby, fake.0.rich_presence("connect"));
        // Simulate a friend's invite arriving from Steam on the next frame.
        fake.0.push_event(BackendEvent::LobbyJoinRequested { lobby: FRIEND_LOBBY, from: FRIEND });
    }
}

/// The game decides what a join request means. Here: accept at once.
fn on_join_requested(mut requests: MessageReader<JoinRequested>, mut join: MessageWriter<JoinLobby>) {
    for req in requests.read() {
        println!("JoinRequested: lobby {} from {} via {:?} -> accepting", req.lobby, req.from, req.source);
        join.write(JoinLobby { lobby: req.lobby });
    }
}

/// In the lobby: read what the host wrote, then connect with your own networking.
fn on_entered(mut entered: MessageReader<LobbyEntered>, backend: Res<SteamBackendRes>) {
    for ev in entered.read() {
        let Some(lobby) = backend.0.lobby() else { continue };
        let host = lobby.lobby_data(ev.lobby, "host");
        let version = lobby.lobby_data(ev.lobby, "version");
        println!("LobbyEntered: lobby {} (host {host:?}, version {version:?}) -> connect your transport here", ev.lobby);
    }
}

fn on_left(mut left: MessageReader<LobbyLeft>) {
    for ev in left.read() {
        println!("LobbyLeft: lobby {}", ev.lobby);
    }
}

fn on_error(mut errors: MessageReader<LobbyError>) {
    for err in errors.read() {
        println!("LobbyError: {:?}: {}", err.kind, err.message);
    }
}

fn stop_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 8 {
        exit.write(AppExit::Success);
    }
}
