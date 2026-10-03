//! Feature `friends`: the local user's friends list (names, persona states, games, the
//! rich-presence `connect` of friends in this game), game invites carrying any connect string,
//! raw join requests, user info for non-friends and avatars as RGBA bytes.
//!
//! The kit reads the list from Steam when a friends-capable backend appears and keeps the
//! [`SteamFriends`] resource current: a friend's `PersonaStateChange` callback (through the kit's
//! one pump) re-reads that friend in the same frame's `Update`, and the whole list is re-read
//! every [`FriendsSettings::refresh_interval`] (rich presence changes send no callback) and on
//! [`RefreshFriends`]. Every difference is one [`FriendsChanged`] message.
//!
//! [`InviteToGame`] sends a Steam game invite with any connect string (the `lobby` feature's
//! `InviteFriend` invites to the kit's lobby only). Accepting it on the other side, or "Join Game"
//! on a friend whose rich presence has `connect`, arrives as [`ConnectRequested`] with the raw
//! string, whatever it is (for a friend in a Steam lobby, Steam reports "Join Game" as a lobby
//! join instead: the `lobby` feature's `JoinRequested` with `LobbyInvite`, no `ConnectRequested`);
//! a cold launch is found in the launch arguments with
//! [`FriendsSettings::launch_connect_prefix`]. With the `lobby` feature also compiled, a
//! `+connect_lobby <id>` join is reported by both features (a `JoinRequested` and a
//! `ConnectRequested`).
//!
//! Configured with [`SteamKitPlugin::with_friends`](crate::SteamKitPlugin::with_friends)
//! ([`FriendsSettings`]). Inert until a [`SteamBackendRes`] whose backend supports friends exists;
//! requests are answered with [`FriendsErrorKind::NoBackend`] until then. Names are personal data:
//! the kit logs counts and ids, never names.
//!
//! Schedules: pumped events in [`SteamKitSystems::Callbacks`] (`First`): [`ConnectRequested`],
//! and which friends to re-read; reads, [`FriendsChanged`], avatars, user info and invites in
//! [`SteamKitSystems::Requests`] (`Update`); on `AppExit`, every [`RequestUserInfo`] still
//! waiting is answered with [`FriendsErrorKind::Exiting`] in [`SteamKitSystems::Requests`]
//! (`Last`; write `AppExit` before `Last`). The refresh interval uses `Time<Real>`; without
//! Bevy's `TimePlugin` the list is re-read only on callbacks and [`RefreshFriends`].

mod backend;
pub(crate) mod fake;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use backend::{AvatarSize, FriendGame, FriendsBackend, PersonaState, MAX_CONNECT_BYTES};

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

use bevy_app::{App, AppExit, First, Last, Update};
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_time::{Real, Time};
use tracing::{debug, info, warn};

use crate::{is_individual_steam_id64, BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

/// Steam's `EPersonaChange` bits the kit acts on.
const CHANGE_AVATAR: u32 = 0x0040;
const CHANGE_RELATIONSHIP: u32 = 0x0200;

/// Avatars read per frame at most (a large one is 135 KB).
const AVATARS_PER_FRAME: usize = 16;

// ---------------------------------------------------------------------------------------------
// Settings + state
// ---------------------------------------------------------------------------------------------

/// The friends feature's settings, given with
/// [`SteamKitPlugin::with_friends`](crate::SteamKitPlugin::with_friends) and also inserted as a
/// resource by the plugin (read-only). Build it with `..Default::default()`.
#[derive(Resource, Clone, Debug)]
pub struct FriendsSettings {
    /// Re-read the whole list this often, also without a callback (a friend's rich presence
    /// changes send none). Default 5 s. `Duration::ZERO` re-reads it every frame.
    pub refresh_interval: Duration,
    /// Read the rich-presence `connect` of friends playing this game. Default `true`.
    pub read_connect: bool,
    /// On the first frame a friends-capable backend exists, look for this token in the process
    /// arguments, then in Steam's launch command line, and report the text from it to the end as
    /// a [`ConnectRequested`] with [`ConnectSource::LaunchArgs`] (a cold launch from an invite).
    /// Default `None` (no check).
    pub launch_connect_prefix: Option<String>,
    /// Read avatars of this size ([`FriendAvatar`] messages). Default `None` (no avatars).
    pub avatars: Option<AvatarSize>,
    /// A [`RequestUserInfo`] Steam has not answered after this long is answered with
    /// [`FriendsErrorKind::TimedOut`]. Default 10 s (needs Bevy's `TimePlugin`; without a clock
    /// there is no timeout).
    pub user_info_timeout: Duration,
}

impl Default for FriendsSettings {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_secs(5),
            read_connect: true,
            launch_connect_prefix: None,
            avatars: None,
            user_info_timeout: Duration::from_secs(10),
        }
    }
}

/// One user as the friends list shows them. `#[non_exhaustive]`: read it, the kit builds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendInfo {
    /// SteamID64.
    pub steam_id: u64,
    /// Persona name.
    pub name: String,
    /// The nickname the local user gave them, if any.
    pub nickname: Option<String>,
    /// Online status.
    pub state: PersonaState,
    /// The game they are playing, if any.
    pub game: Option<FriendGame>,
    /// Their rich-presence `connect`, read only while they play this game and when set
    /// ([`FriendsSettings::read_connect`]).
    pub connect: Option<String>,
}

impl FriendInfo {
    /// The nickname if set, otherwise the persona name.
    pub fn display_name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.name)
    }
    /// Reachable ([`PersonaState::is_online`]).
    pub fn is_online(&self) -> bool {
        self.state.is_online()
    }
    /// Playing the game with this app id.
    pub fn plays(&self, app_id: u32) -> bool {
        self.game.as_ref().is_some_and(|g| g.app_id == app_id)
    }
}

/// The friends list. Written only by the kit; read it through its methods (order a reading system
/// against [`SteamKitSystems::Requests`] in `Update`, where it changes).
#[derive(Resource, Debug, Default)]
pub struct SteamFriends {
    loaded: bool,
    me: Option<FriendInfo>,
    list: Vec<FriendInfo>,
    app_id: u32,
    generation: u32,
}

impl SteamFriends {
    /// The list was read from Steam (and a friends-capable backend is installed).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
    /// The local user (name and state; no game, no connect). The state is the one friends see
    /// (`Online`, `Away`, `Invisible`, ...), re-read on the local user's persona change and on
    /// every full re-read of the list.
    pub fn me(&self) -> Option<&FriendInfo> {
        self.me.as_ref()
    }
    /// Every friend, sorted by SteamID64.
    pub fn list(&self) -> &[FriendInfo] {
        &self.list
    }
    /// One friend.
    pub fn get(&self, steam_id: u64) -> Option<&FriendInfo> {
        self.list.binary_search_by_key(&steam_id, |f| f.steam_id).ok().map(|i| &self.list[i])
    }
    /// Friends that are reachable now.
    pub fn online(&self) -> impl Iterator<Item = &FriendInfo> {
        self.list.iter().filter(|f| f.is_online())
    }
    /// Friends playing this game.
    pub fn playing_this_game(&self) -> impl Iterator<Item = &FriendInfo> {
        let app = self.app_id;
        self.list.iter().filter(move |f| f.plays(app))
    }
    /// The running app's id, as Steam reported it with the last read (0 before the first).
    pub fn app_id(&self) -> u32 {
        self.app_id
    }
    /// Bumped (wrapping) on every change of the list or of [`me`](Self::me).
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Private bookkeeping between the `First` and the `Update` system.
#[derive(Resource, Debug, Default)]
struct FriendsInternals {
    dirty: HashSet<u64>,
    relist: bool,
    me_dirty: bool,
    last_full: Option<Duration>,
    launch_checked: bool,
    /// `RequestUserInfo`s waiting for the user's `PersonaChanged`: one start time per request
    /// (`None` until a clock exists), so duplicates are answered once each.
    user_info_waiting: BTreeMap<u64, Vec<Option<Duration>>>,
    /// Answers due in `Update`: user -> number of requests to answer.
    user_info_due: BTreeMap<u64, usize>,
    avatar_due: BTreeSet<u64>,
    /// Avatars Steam had none of (not loaded yet, or none set): read again on the next full refresh.
    avatar_retry: BTreeSet<u64>,
    /// The ONE reader of [`RequestUserInfo`], shared by `Update` and the exit system in `Last`
    /// (a request written after the `Update` set in the exit frame is answered once, there).
    user_info_cursor: MessageCursor<RequestUserInfo>,
}

// ---------------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------------

/// Re-read the whole friends list now.
#[derive(Message, Clone, Debug, Default)]
pub struct RefreshFriends;

/// Send a Steam game invite carrying `connect`. When the friend accepts, their game gets the
/// string: as a [`ConnectRequested`] if it runs, or on its command line if Steam starts it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct InviteToGame {
    /// The invited user's SteamID64 (an individual account).
    pub steam_id: u64,
    /// The connect string: not empty, at most [`MAX_CONNECT_BYTES`] bytes, no NUL byte.
    pub connect: String,
}

/// Ask Steam for a user's persona (for example a leaderboard row's player who is not a friend).
/// Exactly one answer per request: [`UserInfoReady`] once Steam has it, or a [`FriendsError`]
/// (`InvalidSteamId`, `NoBackend`, `TimedOut` after [`FriendsSettings::user_info_timeout`],
/// `Exiting` when the app exits first).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct RequestUserInfo {
    /// The user's SteamID64.
    pub steam_id: u64,
    /// Only the name is needed (no avatar).
    pub name_only: bool,
}

/// The friends list changed (written at most once per frame). `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendsChanged {
    /// New friends (all of them on the first read), sorted.
    pub added: Vec<u64>,
    /// Friends no longer in the list (all of them when the backend is removed), sorted.
    pub removed: Vec<u64>,
    /// Friends whose [`FriendInfo`] changed, sorted; the local user's id when
    /// [`SteamFriends::me`] changed.
    pub changed: Vec<u64>,
}

/// The outcome of an [`InviteToGame`]. `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameInviteSent {
    /// Invited user.
    pub steam_id: u64,
    /// The connect string sent.
    pub connect: String,
    /// Steam's own result of the call. Not a delivery receipt: Steam returns `true` also for an
    /// invisible friend and for an id that is not a friend.
    pub ok: bool,
}

/// How a [`ConnectRequested`] arrived. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectSource {
    /// While the game runs: an accepted invite, or "Join Game" on a friend with a rich-presence
    /// `connect` who is not in a Steam lobby (Steam's `GameRichPresenceJoinRequested`).
    RichPresence,
    /// The game was started with the connect string ([`FriendsSettings::launch_connect_prefix`]).
    LaunchArgs,
}

/// Someone asked this process to connect, with a connect string the kit does not interpret.
/// `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectRequested {
    /// The connect string as Steam delivered it (launch args: from the prefix to the end).
    pub connect: String,
    /// The friend it came from (SteamID64), `0` when unknown.
    pub from: u64,
    /// How it arrived.
    pub source: ConnectSource,
}

/// A user's persona is available after [`RequestUserInfo`]. `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInfoReady {
    /// The user.
    pub steam_id: u64,
    /// Their persona name.
    pub name: String,
}

/// A user's avatar (with [`FriendsSettings::avatars`]), once per user and again when it changes.
/// `#[non_exhaustive]`.
#[derive(Message, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendAvatar {
    /// The user (a friend, or the local user).
    pub steam_id: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, row by row.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for FriendAvatar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FriendAvatar {{ steam_id: {}, width: {}, height: {}, rgba: {} bytes }}", self.steam_id, self.width, self.height, self.rgba.len())
    }
}

/// What went wrong in a [`FriendsError`]. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FriendsErrorKind {
    /// No [`SteamBackendRes`], or its backend does not support friends.
    NoBackend,
    /// Not an individual SteamID64.
    InvalidSteamId,
    /// An empty connect string, one longer than [`MAX_CONNECT_BYTES`], or one with a NUL byte.
    InvalidConnect,
    /// Steam did not deliver the user's persona within [`FriendsSettings::user_info_timeout`].
    TimedOut,
    /// The app exited (`AppExit`) before Steam answered the [`RequestUserInfo`].
    Exiting,
}

/// Which request a [`FriendsError`] answers. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FriendsRequestKind {
    /// An [`InviteToGame`].
    Invite,
    /// A [`RequestUserInfo`].
    UserInfo,
}

/// A friends request failed. `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FriendsError {
    /// The request that failed.
    pub request: FriendsRequestKind,
    /// The SteamID64 the request named.
    pub steam_id: u64,
    /// Category.
    pub kind: FriendsErrorKind,
    /// Human-readable detail.
    pub message: String,
}

impl FriendsOut<'_> {
    fn fail(&mut self, request: FriendsRequestKind, steam_id: u64, kind: FriendsErrorKind, message: impl Into<String>) {
        let message = message.into();
        warn!(">>> STEAM: friends {request:?} request: {kind:?} ({message})");
        self.error.write(FriendsError { request, steam_id, kind, message });
    }
}

// ---------------------------------------------------------------------------------------------
// Plugin wiring + systems
// ---------------------------------------------------------------------------------------------

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App, settings: &FriendsSettings) {
    app.insert_resource(settings.clone())
        .init_resource::<SteamFriends>()
        .init_resource::<FriendsInternals>()
        .add_message::<RefreshFriends>()
        .add_message::<InviteToGame>()
        .add_message::<RequestUserInfo>()
        .add_message::<FriendsChanged>()
        .add_message::<GameInviteSent>()
        .add_message::<ConnectRequested>()
        .add_message::<UserInfoReady>()
        .add_message::<FriendAvatar>()
        .add_message::<FriendsError>()
        .add_systems(First, apply_friends_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_friends.in_set(SteamKitSystems::Requests))
        .add_systems(Last, friends_on_exit.in_set(SteamKitSystems::Requests));
}

/// The text from the first whitespace-separated token that is `prefix` (or starts with
/// `prefix=`, quotes around the token allowed) to the end of `text`, trimmed.
pub(crate) fn find_launch_connect(text: &str, prefix: &str) -> Option<String> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    let mut pos = 0;
    for tok in text.split_whitespace() {
        let start = pos + text[pos..].find(tok)?;
        pos = start + tok.len();
        let bare = tok.trim_matches('"');
        if bare == prefix || bare.strip_prefix(prefix).is_some_and(|r| r.starts_with('=')) {
            let from = start + tok.find(prefix)?;
            let found = text[from..].trim().trim_end_matches('"').trim();
            return (!found.is_empty()).then(|| found.to_string());
        }
    }
    None
}

/// `First` ([`SteamKitSystems::Callbacks`]): the launch-args check once, raw join requests ->
/// [`ConnectRequested`], persona changes -> who to re-read in `Update`. Reads no clock.
fn apply_friends_events(
    backend: Option<Res<SteamBackendRes>>,
    pumped: Res<PumpedEvents>,
    settings: Res<FriendsSettings>,
    state: Res<SteamFriends>,
    mut internals: ResMut<FriendsInternals>,
    mut connects: MessageWriter<ConnectRequested>,
) {
    let Some(backend) = backend.as_ref().filter(|b| b.0.friends().is_some()) else {
        let lost = pumped.0.iter().filter(|e| matches!(e, BackendEvent::PersonaChanged { .. } | BackendEvent::ConnectRequested { .. })).count();
        if lost > 0 && backend.is_none() {
            warn!(">>> STEAM: backend removed before {lost} pumped friends event(s) were applied - dropped");
        }
        return;
    };

    if !internals.launch_checked {
        internals.launch_checked = true;
        if let Some(prefix) = settings.launch_connect_prefix.as_deref() {
            let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
            let found = find_launch_connect(&args.join(" "), prefix).or_else(|| find_launch_connect(&backend.0.launch_command_line(), prefix));
            if let Some(connect) = found {
                info!(">>> STEAM: launched with a connect string ({} bytes)", connect.len());
                connects.write(ConnectRequested { connect, from: 0, source: ConnectSource::LaunchArgs });
            }
        }
    }

    let me = backend.0.local_id();
    for ev in &pumped.0 {
        match ev {
            &BackendEvent::PersonaChanged { steam_id, flags } => {
                if flags & CHANGE_RELATIONSHIP != 0 {
                    internals.relist = true;
                }
                if steam_id == me {
                    internals.me_dirty = true;
                } else if state.get(steam_id).is_some() {
                    internals.dirty.insert(steam_id);
                }
                if let Some(waiting) = internals.user_info_waiting.remove(&steam_id) {
                    *internals.user_info_due.entry(steam_id).or_default() += waiting.len();
                }
                if flags & CHANGE_AVATAR != 0 && settings.avatars.is_some() && (steam_id == me || state.get(steam_id).is_some()) {
                    internals.avatar_due.insert(steam_id);
                }
            }
            BackendEvent::ConnectRequested { from, connect } => {
                let from = if is_individual_steam_id64(*from) { *from } else { 0 };
                info!(">>> STEAM: connect requested ({} bytes, from a friend: {})", connect.len(), from != 0);
                connects.write(ConnectRequested { connect: connect.clone(), from, source: ConnectSource::RichPresence });
            }
            // Another feature's event.
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}

#[derive(SystemParam)]
struct FriendsIn<'w, 's> {
    refresh: MessageReader<'w, 's, RefreshFriends>,
    invite: MessageReader<'w, 's, InviteToGame>,
    /// Read through `FriendsInternals::user_info_cursor`.
    user_info: Res<'w, Messages<RequestUserInfo>>,
}

#[derive(SystemParam)]
struct FriendsOut<'w> {
    changed: MessageWriter<'w, FriendsChanged>,
    invite_sent: MessageWriter<'w, GameInviteSent>,
    user_info: MessageWriter<'w, UserInfoReady>,
    avatar: MessageWriter<'w, FriendAvatar>,
    error: MessageWriter<'w, FriendsError>,
}

/// Read one user from the backend.
fn read_friend(api: &dyn FriendsBackend, settings: &FriendsSettings, app_id: u32, id: u64) -> FriendInfo {
    let game = api.game_played(id);
    let connect = if settings.read_connect && game.as_ref().is_some_and(|g| g.app_id == app_id) {
        api.friend_rich_presence(id, "connect").filter(|c| !c.is_empty())
    } else {
        None
    };
    FriendInfo { steam_id: id, name: api.persona_name(id), nickname: api.persona_nickname(id), state: api.persona_state(id), game, connect }
}

fn read_me(api: &dyn FriendsBackend, me: u64) -> FriendInfo {
    let (name, state) = api.local_persona();
    FriendInfo { steam_id: me, name, state, ..Default::default() }
}

/// `Update` ([`SteamKitSystems::Requests`]): re-read what is due, report the differences, then
/// avatars, user info and invites.
fn handle_friends(
    backend: Option<Res<SteamBackendRes>>,
    time: Option<Res<Time<Real>>>,
    settings: Res<FriendsSettings>,
    mut state: ResMut<SteamFriends>,
    mut internals: ResMut<FriendsInternals>,
    mut input: FriendsIn,
    mut out: FriendsOut,
) {
    let internals = &mut *internals;
    let user_info_requests: Vec<RequestUserInfo> = internals.user_info_cursor.read(&input.user_info).cloned().collect();
    let Some((api, me)) = backend.as_ref().and_then(|b| b.0.friends().map(|f| (f, b.0.local_id()))) else {
        // Every user-info request still waiting gets its answer.
        let waiting = std::mem::take(&mut internals.user_info_waiting).into_iter().map(|(id, v)| (id, v.len()));
        let due = std::mem::take(&mut internals.user_info_due);
        for (id, n) in waiting.chain(due) {
            for _ in 0..n {
                out.fail(FriendsRequestKind::UserInfo, id, FriendsErrorKind::NoBackend, "Steam backend removed while this request waited");
            }
        }
        if state.loaded {
            let removed: Vec<u64> = state.list.iter().map(|f| f.steam_id).collect();
            let generation = state.generation.wrapping_add(1);
            *state = SteamFriends { generation, ..Default::default() };
            *internals = FriendsInternals {
                launch_checked: internals.launch_checked,
                user_info_cursor: std::mem::take(&mut internals.user_info_cursor),
                ..Default::default()
            };
            info!(">>> STEAM: friends list cleared (backend removed)");
            out.changed.write(FriendsChanged { removed, ..Default::default() });
        }
        input.refresh.clear();
        for req in input.invite.read() {
            out.fail(FriendsRequestKind::Invite, req.steam_id, FriendsErrorKind::NoBackend, "invite: Steam is not available");
        }
        for req in &user_info_requests {
            out.fail(FriendsRequestKind::UserInfo, req.steam_id, FriendsErrorKind::NoBackend, "user info: Steam is not available");
        }
        return;
    };
    let now = time.as_ref().map(|t| t.elapsed());

    // 1. Re-read what is due.
    let asked = input.refresh.read().count() > 0;
    let interval_due = match (now, internals.last_full) {
        (Some(now), Some(last)) => now.saturating_sub(last) >= settings.refresh_interval,
        (Some(_), None) => true,
        (None, _) => false,
    };
    let full = !state.loaded || internals.relist || asked || interval_due;
    let mut changes = FriendsChanged::default();
    let first_load = !state.loaded;
    if full {
        internals.relist = false;
        internals.dirty.clear();
        internals.last_full = now.or(internals.last_full);
        state.app_id = api.current_app_id();
        let app_id = state.app_id;
        let mut ids = api.friend_ids();
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|&id| id != me);
        let new: Vec<FriendInfo> = ids.iter().map(|&id| read_friend(api, &settings, app_id, id)).collect();
        let (mut i, mut j) = (0, 0);
        while i < state.list.len() || j < new.len() {
            match (state.list.get(i), new.get(j)) {
                (Some(a), Some(b)) if a.steam_id == b.steam_id => {
                    if a != b {
                        changes.changed.push(b.steam_id);
                    }
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(b)) if a.steam_id < b.steam_id => {
                    changes.removed.push(a.steam_id);
                    i += 1;
                }
                (Some(a), None) => {
                    changes.removed.push(a.steam_id);
                    i += 1;
                }
                (_, Some(b)) => {
                    changes.added.push(b.steam_id);
                    j += 1;
                }
                (None, None) => break,
            }
        }
        state.list = new;
        internals.me_dirty = true;
        if settings.avatars.is_some() {
            internals.avatar_due.extend(changes.added.iter().copied());
            let retry = std::mem::take(&mut internals.avatar_retry);
            internals.avatar_due.extend(retry);
            if first_load {
                internals.avatar_due.insert(me);
            }
        }
    } else if !internals.dirty.is_empty() {
        let app_id = state.app_id;
        let mut dirty: Vec<u64> = internals.dirty.drain().collect();
        dirty.sort_unstable();
        for id in dirty {
            let Ok(i) = state.list.binary_search_by_key(&id, |f| f.steam_id) else { continue };
            let fresh = read_friend(api, &settings, app_id, id);
            if state.list[i] != fresh {
                state.list[i] = fresh;
                changes.changed.push(id);
            }
        }
    }
    if internals.me_dirty {
        internals.me_dirty = false;
        let fresh = read_me(api, me);
        if state.me.as_ref() != Some(&fresh) {
            if state.me.is_some() {
                changes.changed.push(me);
            }
            state.me = Some(fresh);
        }
    }
    for id in &changes.removed {
        internals.avatar_due.remove(id);
        internals.avatar_retry.remove(id);
    }
    state.loaded = true;
    if first_load {
        info!(">>> STEAM: friends: {} ({} online, {} playing this game)", state.list.len(), state.online().count(), state.playing_this_game().count());
    }
    if first_load || !changes.added.is_empty() || !changes.removed.is_empty() || !changes.changed.is_empty() {
        state.generation = state.generation.wrapping_add(1);
        debug!(">>> STEAM: friends changed: +{} -{} ~{}", changes.added.len(), changes.removed.len(), changes.changed.len());
        out.changed.write(changes);
    }

    // 2. Avatars, a few per frame.
    if let Some(size) = settings.avatars {
        let due: Vec<u64> = internals.avatar_due.iter().copied().take(AVATARS_PER_FRAME).collect();
        for id in due {
            internals.avatar_due.remove(&id);
            match api.friend_avatar(id, size) {
                Some((width, height, rgba)) if rgba.len() as u64 == u64::from(width) * u64::from(height) * 4 => {
                    out.avatar.write(FriendAvatar { steam_id: id, width, height, rgba });
                }
                Some(_) => warn!(">>> STEAM: an avatar has an unexpected size - skipped"),
                // Not loaded yet (Steam loads large ones on demand; any size of a user whose
                // persona is not loaded) or none set: read again on the next full refresh.
                None => {
                    internals.avatar_retry.insert(id);
                }
            }
        }
    }

    // 3. User info: new requests, answers, timeouts (one answer per request).
    for req in &user_info_requests {
        if !is_individual_steam_id64(req.steam_id) {
            out.fail(FriendsRequestKind::UserInfo, req.steam_id, FriendsErrorKind::InvalidSteamId, "not an individual SteamID64");
            continue;
        }
        if api.request_user_information(req.steam_id, req.name_only) {
            internals.user_info_waiting.entry(req.steam_id).or_default().push(now);
        } else {
            *internals.user_info_due.entry(req.steam_id).or_default() += 1;
        }
    }
    for (id, n) in std::mem::take(&mut internals.user_info_due) {
        let name = api.persona_name(id);
        for _ in 0..n {
            out.user_info.write(UserInfoReady { steam_id: id, name: name.clone() });
        }
    }
    if let Some(now) = now {
        let timeout = settings.user_info_timeout;
        let mut expired: Vec<u64> = Vec::new();
        for (id, starts) in internals.user_info_waiting.iter_mut() {
            for s in starts.iter_mut() {
                s.get_or_insert(now);
            }
            let before = starts.len();
            starts.retain(|s| s.is_some_and(|t| now.saturating_sub(t) < timeout));
            expired.extend(std::iter::repeat_n(*id, before - starts.len()));
        }
        internals.user_info_waiting.retain(|_, starts| !starts.is_empty());
        for id in expired {
            out.fail(FriendsRequestKind::UserInfo, id, FriendsErrorKind::TimedOut, "Steam did not deliver the user's persona in time");
        }
    }

    // 4. Invites.
    for req in input.invite.read() {
        if !is_individual_steam_id64(req.steam_id) {
            out.fail(FriendsRequestKind::Invite, req.steam_id, FriendsErrorKind::InvalidSteamId, "not an individual SteamID64");
            continue;
        }
        if req.connect.is_empty() || req.connect.len() > MAX_CONNECT_BYTES || req.connect.contains('\0') {
            out.fail(
                FriendsRequestKind::Invite,
                req.steam_id,
                FriendsErrorKind::InvalidConnect,
                format!("connect string must be 1..={MAX_CONNECT_BYTES} bytes without NUL (got {} bytes)", req.connect.len()),
            );
            continue;
        }
        let ok = api.invite_user_to_game(req.steam_id, &req.connect);
        info!(">>> STEAM: game invite sent ({} bytes) ok={ok}", req.connect.len());
        out.invite_sent.write(GameInviteSent { steam_id: req.steam_id, connect: req.connect.clone(), ok });
    }
}

/// `Last` ([`SteamKitSystems::Requests`]): on `AppExit`, answer every [`RequestUserInfo`] still
/// waiting (also those written this frame after the `Update` set) with
/// [`FriendsErrorKind::Exiting`] (`NoBackend` for a late one when no friends-capable backend is
/// installed), so each request still gets exactly one answer.
fn friends_on_exit(
    mut exit: MessageReader<AppExit>,
    backend: Option<Res<SteamBackendRes>>,
    messages: Res<Messages<RequestUserInfo>>,
    mut internals: ResMut<FriendsInternals>,
    mut out: FriendsOut,
) {
    if exit.read().count() == 0 {
        return;
    }
    let internals = &mut *internals;
    let late: Vec<u64> = internals.user_info_cursor.read(&messages).map(|r| r.steam_id).collect();
    let waiting = std::mem::take(&mut internals.user_info_waiting).into_iter().map(|(id, v)| (id, v.len()));
    let due = std::mem::take(&mut internals.user_info_due);
    let mut answered = 0usize;
    for (id, n) in waiting.chain(due) {
        for _ in 0..n {
            out.fail(FriendsRequestKind::UserInfo, id, FriendsErrorKind::Exiting, "the app exited before Steam answered");
            answered += 1;
        }
    }
    let late_kind = if backend.as_ref().is_some_and(|b| b.0.friends().is_some()) { FriendsErrorKind::Exiting } else { FriendsErrorKind::NoBackend };
    for id in late {
        out.fail(FriendsRequestKind::UserInfo, id, late_kind, "the app exited before this request was sent");
        answered += 1;
    }
    if answered > 0 {
        warn!(">>> STEAM: exiting with {answered} user-info request(s) still waiting - answered");
    }
}
