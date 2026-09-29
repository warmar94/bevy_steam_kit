//! Feature `lobby`: Steam lobbies, rich presence, invites and friend-join requests.
//!
//! Game-agnostic by design: it knows Steam and Bevy, nothing else. The game sends requests
//! ([`CreateLobby`], [`JoinLobby`], [`LeaveLobby`], [`InviteFriend`], [`SetRichPresence`],
//! [`ClearRichPresence`]) and reacts to facts ([`LobbyCreated`], [`LobbyEntered`],
//! [`JoinRequested`], [`LobbyLeft`], [`InviteSent`], [`LobbyError`]). What a join request MEANS
//! (open a menu, connect a transport) is the game's decision.
//!
//! Configured through [`SteamKitPlugin::with_lobby`](crate::SteamKitPlugin::with_lobby) ([`LobbySettings`]).
//! Inert until a [`SteamBackendRes`] whose backend supports lobbies
//! ([`SteamBackend::lobby`](crate::SteamBackend::lobby) returns `Some`) exists; until then requests
//! are answered with [`LobbyErrorKind::NoBackend`].
//!
//! Schedules: the pumped lobby events are applied in [`SteamKitSystems::Callbacks`] (`First`);
//! requests are handled in [`SteamKitSystems::Requests`] in `Update`, and the lobby is left on
//! `AppExit` in [`SteamKitSystems::Requests`] in `Last`.

mod backend;
pub(crate) mod fake;
mod parse;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use backend::{JoinSource, LobbyBackend, LobbyKind};
pub use parse::{connect_string, parse_connect_lobby};

use bevy_app::{App, AppExit, First, Last, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use tracing::{info, warn};

use crate::{is_individual_steam_id64, BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

/// Steam's hard cap on lobby members (`steamworks` asserts on more).
pub const MAX_LOBBY_MEMBERS: u32 = 250;

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/// The lobby feature's settings, given with [`SteamKitPlugin::with_lobby`](crate::SteamKitPlugin::with_lobby) and
/// also inserted as a resource by the plugin (read-only).
#[derive(Resource, Clone, Debug)]
pub struct LobbySettings {
    /// The connect-string prefix used for rich presence, invites and launch-arg parsing.
    /// Default `"+connect_lobby"` (the string Steam passes on a cold launch).
    pub connect_prefix: String,
    /// When a lobby this process created is ready, set rich presence
    /// `connect = "<connect_prefix> <lobby>"` so friends get "Join Game". Default `true`.
    pub set_connect_presence: bool,
    /// On the first frame a lobby-capable backend exists, look for `<connect_prefix> <lobby>` in
    /// the process arguments and in Steam's launch command line and report it as a
    /// [`JoinRequested`] with [`JoinSource::LaunchArgs`]. Default `true`.
    pub check_launch_args: bool,
}

impl Default for LobbySettings {
    fn default() -> Self {
        Self { connect_prefix: "+connect_lobby".to_string(), set_connect_presence: true, check_launch_args: true }
    }
}

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App, settings: &LobbySettings) {
    app.insert_resource(settings.clone())
        .init_resource::<SteamLobby>()
        .init_resource::<LobbyInternals>()
        .add_message::<CreateLobby>()
        .add_message::<JoinLobby>()
        .add_message::<LeaveLobby>()
        .add_message::<InviteFriend>()
        .add_message::<SetRichPresence>()
        .add_message::<ClearRichPresence>()
        .add_message::<LobbyCreated>()
        .add_message::<LobbyEntered>()
        .add_message::<JoinRequested>()
        .add_message::<LobbyLeft>()
        .add_message::<InviteSent>()
        .add_message::<LobbyError>()
        .add_systems(First, apply_lobby_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_requests.in_set(SteamKitSystems::Requests))
        .add_systems(Last, leave_on_exit.in_set(SteamKitSystems::Requests));
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// The lobby this process is in. Written only by the kit; read it, never write it.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SteamLobby {
    /// The lobby we are a member of (created or joined), if any.
    pub current: Option<u64>,
    /// A [`CreateLobby`] is in flight.
    pub pending_create: bool,
    /// A [`JoinLobby`] is in flight for this lobby.
    pub pending_join: Option<u64>,
    /// Bumped on every change of `current` / a pending request (wrapping). Cheap change check.
    pub generation: u32,
}

/// Private bookkeeping.
#[derive(Resource, Default, Debug)]
struct LobbyInternals {
    /// Lobby data of the create in flight, applied when it completes.
    pending_data: Vec<(String, String)>,
    /// Creates abandoned by a leave while in flight: their completion is left immediately.
    orphaned_creates: u32,
    /// This process set rich presence (connect or via [`SetRichPresence`]) -> clear it on leave.
    presence_set: bool,
    /// The launch-args check has run.
    launch_checked: bool,
}

// ---------------------------------------------------------------------------------------------
// Messages IN
// ---------------------------------------------------------------------------------------------

/// Create a lobby and become its owner. Refused ([`LobbyErrorKind::AlreadyInLobby`]) while a
/// lobby is current or a create/join is in flight.
#[derive(Message, Clone, Debug)]
pub struct CreateLobby {
    /// Visibility.
    pub kind: LobbyKind,
    /// Member cap including the owner; clamped to `1..=250`.
    pub max_members: u32,
    /// Lobby metadata set as soon as the lobby exists (e.g. a host id, a version tag).
    pub data: Vec<(String, String)>,
}

/// Join a lobby. Leaves the current lobby (and abandons any request in flight) first.
#[derive(Message, Clone, Debug)]
pub struct JoinLobby {
    /// Raw lobby id.
    pub lobby: u64,
}

/// Leave the current lobby, abandon any request in flight, clear rich presence the kit set.
#[derive(Message, Clone, Debug, Default)]
pub struct LeaveLobby;

/// Invite a user to the current lobby with a Steam game invite carrying the connect string.
#[derive(Message, Clone, Debug)]
pub struct InviteFriend {
    /// The invited user's SteamID64 (must be an individual account).
    pub steam_id: u64,
}

/// Set (`Some`) or remove (`None`) one rich-presence key.
#[derive(Message, Clone, Debug)]
pub struct SetRichPresence {
    /// Key (e.g. `"status"`).
    pub key: String,
    /// Value; `None` removes the key.
    pub value: Option<String>,
}

/// Clear every rich-presence key.
#[derive(Message, Clone, Debug, Default)]
pub struct ClearRichPresence;

// ---------------------------------------------------------------------------------------------
// Messages OUT
// ---------------------------------------------------------------------------------------------

/// A lobby this process created is ready (data, joinable and connect presence already set).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct LobbyCreated {
    /// Raw lobby id.
    pub lobby: u64,
}

/// This process joined a lobby after [`JoinLobby`]. Read its data via the backend.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct LobbyEntered {
    /// Raw lobby id.
    pub lobby: u64,
}

/// Someone asked this process to join a lobby. The kit does NOT join by itself.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct JoinRequested {
    /// Raw lobby id.
    pub lobby: u64,
    /// The friend it came from (SteamID64), `0` when unknown (cold launch, non-friend).
    pub from: u64,
    /// How the request arrived.
    pub source: JoinSource,
}

/// This process left `lobby` (on [`LeaveLobby`], a new [`JoinLobby`] or `AppExit`).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct LobbyLeft {
    /// Raw lobby id.
    pub lobby: u64,
}

/// The outcome of an [`InviteFriend`].
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct InviteSent {
    /// Invited user.
    pub steam_id: u64,
    /// The lobby the invite points at.
    pub lobby: u64,
    /// Whether Steam accepted the call.
    pub ok: bool,
}

/// What went wrong in a [`LobbyError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LobbyErrorKind {
    /// Steam failed to create the lobby.
    CreateFailed,
    /// A create was refused: a lobby is current or a request is in flight.
    AlreadyInLobby,
    /// Steam failed to join the lobby.
    JoinFailed,
    /// The request needs a current lobby and there is none.
    NoLobby,
    /// No [`SteamBackendRes`] (or its backend does not support lobbies): Steam is not available in
    /// this process.
    NoBackend,
    /// Not an individual public SteamID64.
    InvalidSteamId,
    /// A request field was unusable (`JoinLobby` with lobby id 0). A string with a NUL byte is
    /// not this kind: Steam cannot take it, so it fails like a refused call (`PresenceFailed` for
    /// rich presence, `InviteSent { ok: false }` for an invite, a logged warning for lobby data).
    InvalidRequest,
    /// Steam refused a rich-presence update.
    PresenceFailed,
}

/// A request failed. The game decides how (or whether) to tell the user.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct LobbyError {
    /// Category.
    pub kind: LobbyErrorKind,
    /// Human-readable detail (ASCII).
    pub message: String,
}

impl LobbyError {
    fn new(kind: LobbyErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

/// Every OUT writer, bundled so the systems stay far below Bevy's parameter cap.
#[derive(SystemParam)]
struct LobbyOut<'w> {
    created: MessageWriter<'w, LobbyCreated>,
    entered: MessageWriter<'w, LobbyEntered>,
    join_requested: MessageWriter<'w, JoinRequested>,
    left: MessageWriter<'w, LobbyLeft>,
    invite_sent: MessageWriter<'w, InviteSent>,
    error: MessageWriter<'w, LobbyError>,
}

/// Every IN reader.
#[derive(SystemParam)]
struct LobbyIn<'w, 's> {
    create: MessageReader<'w, 's, CreateLobby>,
    join: MessageReader<'w, 's, JoinLobby>,
    leave: MessageReader<'w, 's, LeaveLobby>,
    invite: MessageReader<'w, 's, InviteFriend>,
    set_presence: MessageReader<'w, 's, SetRichPresence>,
    clear_presence: MessageReader<'w, 's, ClearRichPresence>,
}

// ---------------------------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------------------------

fn bump(state: &mut SteamLobby) {
    state.generation = state.generation.wrapping_add(1);
}

/// The lobby half of the installed backend, if there is a backend and it supports lobbies.
fn lobby_api<'a>(backend: &'a Option<Res<'_, SteamBackendRes>>) -> Option<&'a dyn LobbyBackend> {
    backend.as_ref().and_then(|b| b.0.lobby())
}

/// Leave everything: the current lobby, any create/join in flight, and our rich presence.
fn leave_all(backend: &dyn LobbyBackend, state: &mut SteamLobby, internals: &mut LobbyInternals, left: &mut MessageWriter<LobbyLeft>) {
    let mut changed = false;
    if let Some(lobby) = state.current.take() {
        backend.leave_lobby(lobby);
        left.write(LobbyLeft { lobby });
        info!(">>> STEAM: left lobby {lobby}");
        changed = true;
    }
    if state.pending_create {
        state.pending_create = false;
        internals.orphaned_creates = internals.orphaned_creates.saturating_add(1);
        internals.pending_data.clear();
        info!(">>> STEAM: abandoned a lobby create in flight (it will be left when it completes)");
        changed = true;
    }
    if let Some(lobby) = state.pending_join.take() {
        info!(">>> STEAM: abandoned joining lobby {lobby} (it will be left if the join completes)");
        changed = true;
    }
    if internals.presence_set {
        backend.clear_rich_presence();
        internals.presence_set = false;
    }
    if changed {
        bump(state);
    }
}

/// `First` ([`SteamKitSystems::Callbacks`]): check the launch arguments once, then turn this
/// frame's pumped lobby events into state changes + OUT messages. Never pumps by itself.
fn apply_lobby_events(
    backend: Option<Res<SteamBackendRes>>,
    pumped: Res<PumpedEvents>,
    settings: Res<LobbySettings>,
    mut state: ResMut<SteamLobby>,
    mut internals: ResMut<LobbyInternals>,
    mut out: LobbyOut,
) {
    let Some(backend) = backend.as_ref() else {
        if !pumped.0.is_empty() {
            // Only possible when the game removed the backend between `Pump` and `Callbacks`
            // (documented on `SteamKitSystems::Callbacks`): applying needs the backend.
            warn!(">>> STEAM: backend removed before {} pumped event(s) were applied - dropped", pumped.0.len());
        }
        return;
    };
    let Some(lobby_api) = backend.0.lobby() else { return };

    if settings.check_launch_args && !internals.launch_checked {
        internals.launch_checked = true;
        let mut text: String = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
        text.push(' ');
        text.push_str(&backend.0.launch_command_line());
        if let Some(lobby) = parse_connect_lobby(&text, &settings.connect_prefix) {
            info!(">>> STEAM: launched to join lobby {lobby}");
            out.join_requested.write(JoinRequested { lobby, from: 0, source: JoinSource::LaunchArgs });
        }
    }

    for ev in &pumped.0 {
        apply_event(ev, lobby_api, &settings, &mut state, &mut internals, &mut out);
    }
}

/// One pumped event. Events of other features are ignored here.
fn apply_event(
    ev: &BackendEvent,
    backend: &dyn LobbyBackend,
    settings: &LobbySettings,
    state: &mut SteamLobby,
    internals: &mut LobbyInternals,
    out: &mut LobbyOut,
) {
    match ev {
        &BackendEvent::LobbyCreated { lobby } => {
            if internals.orphaned_creates > 0 {
                internals.orphaned_creates -= 1;
                backend.leave_lobby(lobby);
                info!(">>> STEAM: lobby {lobby} created after it was abandoned - left it");
            } else if !state.pending_create {
                backend.leave_lobby(lobby);
                warn!(">>> STEAM: unexpected lobby {lobby} created - left it");
            } else {
                state.pending_create = false;
                for (key, value) in std::mem::take(&mut internals.pending_data) {
                    if !backend.set_lobby_data(lobby, &key, &value) {
                        warn!(">>> STEAM: lobby {lobby}: could not set data {key:?}");
                    }
                }
                if !backend.set_lobby_joinable(lobby, true) {
                    warn!(">>> STEAM: lobby {lobby}: could not make it joinable");
                }
                if settings.set_connect_presence {
                    let connect = connect_string(&settings.connect_prefix, lobby);
                    if backend.set_rich_presence("connect", Some(&connect)) {
                        internals.presence_set = true;
                    } else {
                        warn!(">>> STEAM: lobby {lobby}: could not set rich presence connect");
                    }
                }
                state.current = Some(lobby);
                bump(state);
                info!(">>> STEAM: lobby {lobby} open");
                out.created.write(LobbyCreated { lobby });
            }
        }
        BackendEvent::LobbyCreateFailed { message } => {
            if internals.orphaned_creates > 0 {
                internals.orphaned_creates -= 1;
                info!(">>> STEAM: abandoned lobby create failed: {message}");
            } else if state.pending_create {
                state.pending_create = false;
                internals.pending_data.clear();
                bump(state);
                warn!(">>> STEAM: lobby create FAILED: {message}");
                out.error.write(LobbyError::new(LobbyErrorKind::CreateFailed, format!("lobby create failed: {message}")));
            }
        }
        &BackendEvent::LobbyEntered { lobby } => {
            if state.pending_join == Some(lobby) {
                state.pending_join = None;
                state.current = Some(lobby);
                bump(state);
                info!(">>> STEAM: joined lobby {lobby}");
                out.entered.write(LobbyEntered { lobby });
            } else if state.current != Some(lobby) {
                backend.leave_lobby(lobby);
                info!(">>> STEAM: lobby {lobby} joined after it was abandoned - left it");
            }
        }
        &BackendEvent::LobbyJoinFailed { lobby } => {
            if state.pending_join == Some(lobby) {
                state.pending_join = None;
                bump(state);
                warn!(">>> STEAM: joining lobby {lobby} FAILED");
                out.error.write(LobbyError::new(LobbyErrorKind::JoinFailed, format!("could not join lobby {lobby}")));
            }
        }
        &BackendEvent::LobbyJoinRequested { lobby, from } => {
            if lobby == 0 {
                warn!(">>> STEAM: ignored a lobby join request with lobby id 0");
                return;
            }
            let from = if is_individual_steam_id64(from) { from } else { 0 };
            info!(">>> STEAM: join requested (lobby invite) lobby {lobby} from {from}");
            out.join_requested.write(JoinRequested { lobby, from, source: JoinSource::LobbyInvite });
        }
        BackendEvent::RichPresenceJoinRequested { from, connect } => match parse_connect_lobby(connect, &settings.connect_prefix) {
            Some(lobby) => {
                let from = if is_individual_steam_id64(*from) { *from } else { 0 };
                info!(">>> STEAM: join requested (rich presence) lobby {lobby} from {from}");
                out.join_requested.write(JoinRequested { lobby, from, source: JoinSource::RichPresence });
            }
            None => warn!(">>> STEAM: ignored rich-presence join with connect {connect:?}"),
        },
        // Another feature's event (stats, leaderboards); this arm keeps the lobby code unchanged when one
        // is added).
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// `Update`: handle every request message.
fn handle_requests(
    backend: Option<Res<SteamBackendRes>>,
    settings: Res<LobbySettings>,
    mut state: ResMut<SteamLobby>,
    mut internals: ResMut<LobbyInternals>,
    mut input: LobbyIn,
    mut out: LobbyOut,
) {
    let Some(backend) = lobby_api(&backend) else {
        let no_backend = |what: &str| LobbyError::new(LobbyErrorKind::NoBackend, format!("{what}: Steam is not available"));
        for _ in input.create.read() {
            out.error.write(no_backend("create lobby"));
        }
        for _ in input.join.read() {
            out.error.write(no_backend("join lobby"));
        }
        for _ in input.invite.read() {
            out.error.write(no_backend("invite"));
        }
        for _ in input.set_presence.read() {
            out.error.write(no_backend("rich presence"));
        }
        // Leaving / clearing with no Steam is a harmless no-op, not an error: a game calls these
        // unconditionally on teardown and must not receive a `LobbyError` in a build without Steam.
        input.leave.clear();
        input.clear_presence.clear();
        return;
    };

    // Leave first, so "leave + create" in one frame re-creates cleanly.
    if input.leave.read().count() > 0 {
        leave_all(backend, &mut state, &mut internals, &mut out.left);
    }

    for req in input.join.read() {
        if req.lobby == 0 {
            out.error.write(LobbyError::new(LobbyErrorKind::InvalidRequest, "lobby id 0"));
            continue;
        }
        if state.current == Some(req.lobby) {
            out.entered.write(LobbyEntered { lobby: req.lobby });
            continue;
        }
        if state.pending_join == Some(req.lobby) {
            continue;
        }
        leave_all(backend, &mut state, &mut internals, &mut out.left);
        info!(">>> STEAM: joining lobby {}", req.lobby);
        state.pending_join = Some(req.lobby);
        bump(&mut state);
        backend.join_lobby(req.lobby);
    }

    for req in input.create.read() {
        if state.current.is_some() || state.pending_create || state.pending_join.is_some() {
            warn!(">>> STEAM: create lobby refused - already in or joining a lobby");
            out.error.write(LobbyError::new(LobbyErrorKind::AlreadyInLobby, "already in a lobby (or a lobby request is in flight)"));
            continue;
        }
        let max = req.max_members.clamp(1, MAX_LOBBY_MEMBERS);
        if max != req.max_members {
            warn!(">>> STEAM: lobby max_members {} clamped to {max}", req.max_members);
        }
        info!(">>> STEAM: creating {:?} lobby (max {max})", req.kind);
        state.pending_create = true;
        internals.pending_data = req.data.clone();
        bump(&mut state);
        backend.create_lobby(req.kind, max);
    }

    for req in input.invite.read() {
        if !is_individual_steam_id64(req.steam_id) {
            out.error.write(LobbyError::new(LobbyErrorKind::InvalidSteamId, format!("not a SteamID64: {}", req.steam_id)));
            continue;
        }
        let Some(lobby) = state.current else {
            out.error.write(LobbyError::new(LobbyErrorKind::NoLobby, "not in a lobby - nothing to invite to"));
            continue;
        };
        let connect = connect_string(&settings.connect_prefix, lobby);
        let ok = backend.invite_to_game(req.steam_id, &connect);
        info!(">>> STEAM: invite sent to {} (lobby {lobby}) ok={ok}", req.steam_id);
        out.invite_sent.write(InviteSent { steam_id: req.steam_id, lobby, ok });
    }

    for req in input.set_presence.read() {
        if backend.set_rich_presence(&req.key, req.value.as_deref()) {
            if req.value.is_some() {
                internals.presence_set = true;
            }
        } else {
            out.error.write(LobbyError::new(LobbyErrorKind::PresenceFailed, format!("could not set rich presence {:?}", req.key)));
        }
    }

    if input.clear_presence.read().count() > 0 {
        backend.clear_rich_presence();
        internals.presence_set = false;
    }
}

/// `Last`: on `AppExit`, leave the lobby and clear rich presence before the process ends.
fn leave_on_exit(
    mut exit: MessageReader<AppExit>,
    backend: Option<Res<SteamBackendRes>>,
    mut state: ResMut<SteamLobby>,
    mut internals: ResMut<LobbyInternals>,
    mut left: MessageWriter<LobbyLeft>,
) {
    if exit.read().count() == 0 {
        return;
    }
    let Some(backend) = lobby_api(&backend) else { return };
    leave_all(backend, &mut state, &mut internals, &mut left);
}
