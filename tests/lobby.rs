//! The lobby feature's public surface from a game's point of view: the kit in a strict headless
//! app (ambiguity detection = Error on every main schedule), the game's own systems ordered around the public
//! sets, and the in-memory `FakeSteamBackend` — no test ever talks to a real Steam client.

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_steam_kit::*;

/// Obviously made-up individual SteamID64s.
const FRIEND: u64 = 76_561_197_960_265_730;
const HOST: u64 = 76_561_197_960_265_731;

/// Everything the game saw, in order.
#[derive(Resource, Default)]
struct Seen {
    created: Vec<LobbyCreated>,
    entered: Vec<LobbyEntered>,
    requested: Vec<JoinRequested>,
    left: Vec<LobbyLeft>,
    invites: Vec<InviteSent>,
    errors: Vec<LobbyError>,
    /// `host` data read from each entered lobby.
    host_data: Vec<Option<String>>,
}

/// The game's reaction to a join request: accept it.
#[derive(Resource)]
struct AutoAccept(bool);

/// Every fact message the lobby feature writes.
#[derive(SystemParam)]
struct Facts<'w, 's> {
    created: MessageReader<'w, 's, LobbyCreated>,
    entered: MessageReader<'w, 's, LobbyEntered>,
    requested: MessageReader<'w, 's, JoinRequested>,
    left: MessageReader<'w, 's, LobbyLeft>,
    invites: MessageReader<'w, 's, InviteSent>,
    errors: MessageReader<'w, 's, LobbyError>,
}

fn record(mut facts: Facts, backend: Option<Res<SteamBackendRes>>, mut seen: ResMut<Seen>) {
    seen.created.extend(facts.created.read().cloned());
    for ev in facts.entered.read() {
        let host = backend.as_ref().and_then(|b| b.0.lobby().and_then(|l| l.lobby_data(ev.lobby, "host")));
        seen.host_data.push(host);
        seen.entered.push(ev.clone());
    }
    seen.requested.extend(facts.requested.read().cloned());
    seen.left.extend(facts.left.read().cloned());
    seen.invites.extend(facts.invites.read().cloned());
    seen.errors.extend(facts.errors.read().cloned());
}

fn accept(accept: Res<AutoAccept>, mut requests: MessageReader<JoinRequested>, mut join: MessageWriter<JoinLobby>) {
    for req in requests.read() {
        if accept.0 {
            join.write(JoinLobby { lobby: req.lobby });
        }
    }
}

fn game(backend: Option<&FakeSteamBackend>) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .init_resource::<Seen>()
        .insert_resource(AutoAccept(true))
        // Requests are written before the kit handles them (acted on in the same frame) ...
        .add_systems(Update, accept.before(SteamKitSystems::Requests))
        // ... and facts are read after it: in `Last`, after the exit leave, sees every fact of the frame.
        .add_systems(Last, record.after(SteamKitSystems::Requests));
    if let Some(fake) = backend {
        app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    }
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

fn seen(app: &App) -> &Seen {
    app.world().resource::<Seen>()
}

fn lobby(app: &App) -> SteamLobby {
    app.world().resource::<SteamLobby>().clone()
}

fn host_request() -> CreateLobby {
    CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: vec![("host".into(), HOST.to_string())] }
}

#[test]
fn host_lifecycle_create_presence_leave() {
    let fake = FakeSteamBackend::new();
    let mut app = game(Some(&fake));
    app.world_mut().write_message(host_request());
    frames(&mut app, 3);

    assert_eq!(seen(&app).created, vec![LobbyCreated { lobby: 1000 }]);
    assert_eq!(lobby(&app).current, Some(1000));
    assert_eq!(fake.lobby_data(1000, "host"), Some(HOST.to_string()));
    assert_eq!(fake.rich_presence("connect").as_deref(), Some("+connect_lobby 1000"));
    assert!(fake.calls().contains(&FakeCall::CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4 }));

    // A game-set key sticks next to `connect`; leaving clears both.
    app.world_mut().write_message(SetRichPresence { key: "status".into(), value: Some("Hosting".into()) });
    frames(&mut app, 1);
    assert_eq!(fake.rich_presence("status").as_deref(), Some("Hosting"));

    app.world_mut().write_message(LeaveLobby);
    frames(&mut app, 2);
    assert_eq!(seen(&app).left, vec![LobbyLeft { lobby: 1000 }]);
    assert_eq!(lobby(&app).current, None);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(1000)));
    assert_eq!((fake.rich_presence("connect"), fake.rich_presence("status")), (None, None));
    assert!(seen(&app).errors.is_empty(), "{:?}", seen(&app).errors);
}

#[test]
fn a_join_request_is_only_a_fact_until_the_game_sends_join_lobby() {
    let fake = FakeSteamBackend::new();
    fake.put_lobby_data(77, "host", &HOST.to_string());
    let mut app = game(Some(&fake));
    app.insert_resource(AutoAccept(false));
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 77, from: FRIEND });
    frames(&mut app, 3);

    assert_eq!(seen(&app).requested, vec![JoinRequested { lobby: 77, from: FRIEND, source: JoinSource::LobbyInvite }]);
    assert!(!fake.calls().iter().any(|c| matches!(c, FakeCall::JoinLobby(_))), "the kit never joins by itself");
    assert!(seen(&app).entered.is_empty());

    // The game decides to accept the next one.
    app.insert_resource(AutoAccept(true));
    fake.push_event(BackendEvent::RichPresenceJoinRequested { from: FRIEND, connect: "+connect_lobby 77".into() });
    frames(&mut app, 3);

    assert_eq!(seen(&app).requested.last().map(|r| r.source), Some(JoinSource::RichPresence));
    assert!(fake.calls().contains(&FakeCall::JoinLobby(77)));
    assert_eq!(seen(&app).entered, vec![LobbyEntered { lobby: 77 }]);
    assert_eq!(seen(&app).host_data, vec![Some(HOST.to_string())]);
    assert_eq!(lobby(&app).current, Some(77));
}

#[test]
fn a_cold_launch_argument_becomes_a_join_request() {
    let fake = FakeSteamBackend::new();
    fake.set_launch_command_line("+connect_lobby 31");
    let mut app = game(Some(&fake));
    app.insert_resource(AutoAccept(false));
    frames(&mut app, 3);
    assert_eq!(seen(&app).requested, vec![JoinRequested { lobby: 31, from: 0, source: JoinSource::LaunchArgs }]);
}

#[test]
fn a_failed_join_is_an_error_and_leaves_no_lobby() {
    let fake = FakeSteamBackend::new();
    fake.set_join_succeeds(false);
    let mut app = game(Some(&fake));
    app.world_mut().write_message(JoinLobby { lobby: 55 });
    frames(&mut app, 3);
    assert_eq!(seen(&app).errors.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![LobbyErrorKind::JoinFailed]);
    assert_eq!(lobby(&app), SteamLobby { generation: lobby(&app).generation, ..default() });
}

#[test]
fn invite_needs_a_lobby_then_carries_the_connect_string() {
    let fake = FakeSteamBackend::new();
    let mut app = game(Some(&fake));
    app.world_mut().write_message(InviteFriend { steam_id: FRIEND });
    frames(&mut app, 1);
    assert_eq!(seen(&app).errors.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![LobbyErrorKind::NoLobby]);
    assert!(seen(&app).invites.is_empty());

    app.world_mut().write_message(host_request());
    frames(&mut app, 3);
    app.world_mut().write_message(InviteFriend { steam_id: FRIEND });
    frames(&mut app, 1);
    assert_eq!(seen(&app).invites, vec![InviteSent { steam_id: FRIEND, lobby: 1000, ok: true }]);
    assert!(fake.calls().contains(&FakeCall::InviteToGame { friend: FRIEND, connect: "+connect_lobby 1000".into() }));
}

#[test]
fn without_a_backend_requests_are_answered_with_no_backend() {
    let mut app = game(None);
    app.world_mut().write_message(host_request());
    app.world_mut().write_message(JoinLobby { lobby: 9 });
    app.world_mut().write_message(InviteFriend { steam_id: FRIEND });
    app.world_mut().write_message(SetRichPresence { key: "status".into(), value: None });
    // Leaving and clearing are silent no-ops: a game calls them unconditionally on teardown.
    app.world_mut().write_message(LeaveLobby);
    app.world_mut().write_message(ClearRichPresence);
    frames(&mut app, 2);
    let kinds: Vec<_> = seen(&app).errors.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec![LobbyErrorKind::NoBackend; 4]);
    assert_eq!(lobby(&app), SteamLobby::default());
}

#[test]
fn app_exit_leaves_the_lobby_and_clears_presence() {
    let fake = FakeSteamBackend::new();
    let mut app = game(Some(&fake));
    app.world_mut().write_message(host_request());
    frames(&mut app, 3);
    assert_eq!(lobby(&app).current, Some(1000));

    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(seen(&app).left, vec![LobbyLeft { lobby: 1000 }]);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(1000)));
    assert_eq!(fake.rich_presence("connect"), None);
    assert_eq!(lobby(&app).current, None);
}

/// The builder is the way to give settings; `JoinSource` is `#[non_exhaustive]` (a `_` arm).
#[test]
fn the_plugin_takes_lobby_settings_through_its_builder() {
    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default().with_lobby(LobbySettings { set_connect_presence: false, ..default() })))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())));
    app.world_mut().write_message(host_request());
    frames(&mut app, 3);
    assert!(!app.world().resource::<LobbySettings>().set_connect_presence);
    assert_eq!(lobby(&app).current, Some(1000));
    assert_eq!(fake.rich_presence("connect"), None);
    let label = match JoinSource::LaunchArgs {
        JoinSource::LaunchArgs => "cold launch",
        _ => "running",
    };
    assert_eq!(label, "cold launch");
}
