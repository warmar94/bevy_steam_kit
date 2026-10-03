//! Steam for Bevy, as opt-in features.
//!
//! The kit's core is one plugin, [`SteamKitPlugin`], that owns the ONE Steam callback pump of the
//! process ([`SteamKitSystems::Pump`], in `First`) and hands what it collected to every compiled
//! feature in the same frame ([`SteamKitSystems::Callbacks`]). Features:
//!
//! - `lobby`: Steam lobbies, rich presence ("Join Game" in the friends list), game invites and
//!   every friend-join path as Bevy messages (module `lobby`, re-exported at the crate root).
//! - `stats`: the local user's stats and achievements as Bevy messages, with batched stores
//!   (module `stats`, re-exported at the crate root).
//! - `leaderboards`: find / create leaderboards, upload scores (queued and rate-limited),
//!   download entries (module `leaderboards`, re-exported at the crate root).
//! - `auth`: Steam Web API tickets for logging in to a game server (module `auth`, re-exported
//!   at the crate root).
//! - `friends`: the friends list with persona states, games and rich-presence `connect`, game
//!   invites with any connect string, raw join requests, avatars (module `friends`, re-exported at
//!   the crate root).
//! - `overlay`: open the Steam overlay (dialogs, store page, web page, invite dialogs) and learn
//!   when it opens and closes (module `overlay`, re-exported at the crate root).
//! - `steam`: the real backend, `RealSteamBackend`, over `steamworks` 0.12.2. Without it the
//!   crate still builds (no Steam SDK runtime needed) and [`FakeSteamBackend`] drives everything.
//!
//! The kit is inert until a [`SteamBackendRes`] exists: insert
//! `SteamBackendRes(Box::new(RealSteamBackend::new(client)))` (feature `steam`) when Steam
//! initialised, or a [`FakeSteamBackend`] in tests. The game initialises Steam itself, keeps its own
//! `steamworks::Client` clone for any other Steam API, and never pumps it: exactly one
//! `process_callbacks` per frame for the whole process, and that is the kit's.
//!
//! When the Steam client quits while the game runs, the game keeps running: the kit writes
//! [`SteamLost`] once, stops pumping Steam and answers every Steam request with its feature's
//! `NoBackend` error from then on.
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "auth")]
pub mod auth;
mod backend;
mod fake;
#[cfg(feature = "friends")]
pub mod friends;
#[cfg(feature = "leaderboards")]
pub mod leaderboards;
#[cfg(feature = "lobby")]
pub mod lobby;
#[cfg(any(feature = "stats", feature = "leaderboards"))]
mod names;
#[cfg(feature = "overlay")]
pub mod overlay;
#[cfg(feature = "steam")]
mod real;
#[cfg(feature = "stats")]
pub mod stats;
mod steam_id;
#[cfg(feature = "steam")]
mod steam_process;
#[cfg(test)]
mod tests;

/// Every Rust example in the README compiles (checked by `cargo test --all-features`; the README
/// uses every feature and `RealSteamBackend`, so the check needs all of them).
#[cfg(all(
    doctest,
    feature = "steam",
    feature = "lobby",
    feature = "stats",
    feature = "leaderboards",
    feature = "auth",
    feature = "friends",
    feature = "overlay"
))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

#[cfg(feature = "auth")]
pub use auth::{AuthBackend, AuthError, AuthErrorKind, AuthRequest, AuthRequestId, AuthSettings, FakeAuthFailure, SteamAuth, WebApiTicket, WebApiTicketReady};
pub use backend::{BackendEvent, SteamBackend, SteamBackendRes, SteamLostReason};
pub use fake::{FakeCall, FakeSteamBackend};
#[cfg(feature = "friends")]
pub use friends::{
    AvatarSize, ConnectRequested, ConnectSource, FriendAvatar, FriendGame, FriendInfo, FriendsBackend, FriendsChanged, FriendsError, FriendsErrorKind,
    FriendsRequestKind, FriendsSettings, GameInviteSent, InviteToGame, PersonaState, RefreshFriends, RequestUserInfo, SteamFriends, UserInfoReady,
    MAX_CONNECT_BYTES,
};
#[cfg(feature = "leaderboards")]
pub use leaderboards::{
    is_valid_leaderboard_name, FakeLeaderboardFailure, LeaderboardBackend, LeaderboardDisplay, LeaderboardEntry, LeaderboardError, LeaderboardErrorKind,
    LeaderboardFound, LeaderboardInfo, LeaderboardRequest, LeaderboardRequestId, LeaderboardSettings, LeaderboardSort, ScoreRange, ScoreUploaded,
    ScoresDownloaded, SteamLeaderboards, UploadMethod, MAX_LEADERBOARD_DETAILS, MAX_LEADERBOARD_NAME_BYTES,
};
#[cfg(feature = "lobby")]
pub use lobby::{
    connect_string, parse_connect_lobby, ClearRichPresence, CreateLobby, InviteFriend, InviteSent, JoinLobby, JoinRequested, JoinSource, LeaveLobby,
    LobbyBackend, LobbyCreated, LobbyEntered, LobbyError, LobbyErrorKind, LobbyKind, LobbyLeft, LobbySettings, SetRichPresence, SteamLobby, MAX_LOBBY_MEMBERS,
};
#[cfg(feature = "overlay")]
pub use overlay::{OpenOverlay, OverlayBackend, OverlayError, OverlayErrorKind, OverlayToggled, SteamOverlay, StoreFlag};
#[cfg(feature = "steam")]
pub use real::RealSteamBackend;
#[cfg(feature = "stats")]
pub use stats::{
    is_valid_api_name, AchievementProgress, AchievementUnlocked, FakeStoreFailure, StatKind, StatValue, StatsBackend, StatsError, StatsErrorKind, StatsReady,
    StatsRequest, StatsSettings, StatsStored, SteamStats, MAX_API_NAME_BYTES,
};
pub use steam_id::is_individual_steam_id64;

use bevy_app::{App, First, Last, Plugin, Update};
use bevy_ecs::message::MessageUpdateSystems;
use bevy_ecs::prelude::*;

// ---------------------------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------------------------

/// The kit's plugin. Add it ONCE; it adds no other plugin and works under `MinimalPlugins`.
///
/// It always installs the callback pump; each compiled feature adds its messages, resources and
/// systems. Feature settings are given with one builder method per feature (`with_lobby` for the
/// lobby feature), never through public fields, so code written for one feature set keeps compiling
/// when another crate in the build enables more features:
///
/// ```
/// # use bevy_app::App;
/// # use bevy_steam_kit::SteamKitPlugin;
/// let mut app = App::new();
/// app.add_plugins(SteamKitPlugin::default());
/// ```
#[derive(Clone, Debug, Default)]
pub struct SteamKitPlugin {
    /// Settings of the lobby feature (feature `lobby`).
    #[cfg(feature = "lobby")]
    lobby: LobbySettings,
    /// Settings of the stats feature (feature `stats`).
    #[cfg(feature = "stats")]
    stats: StatsSettings,
    /// Settings of the leaderboards feature (feature `leaderboards`).
    #[cfg(feature = "leaderboards")]
    leaderboards: LeaderboardSettings,
    /// Settings of the auth feature (feature `auth`).
    #[cfg(feature = "auth")]
    auth: AuthSettings,
    /// Settings of the friends feature (feature `friends`).
    #[cfg(feature = "friends")]
    friends: FriendsSettings,
}

impl SteamKitPlugin {
    /// Use these settings for the lobby feature (feature `lobby`; default: `LobbySettings::default()`).
    ///
    /// ```
    /// # use bevy_steam_kit::{LobbySettings, SteamKitPlugin};
    /// let plugin = SteamKitPlugin::default().with_lobby(LobbySettings { set_connect_presence: false, ..Default::default() });
    /// # let _ = plugin;
    /// ```
    #[cfg(feature = "lobby")]
    pub fn with_lobby(mut self, settings: LobbySettings) -> Self {
        self.lobby = settings;
        self
    }

    /// Use these settings for the stats feature (feature `stats`; default: `StatsSettings::default()`).
    ///
    /// ```
    /// # use bevy_steam_kit::{StatsSettings, SteamKitPlugin};
    /// let plugin = SteamKitPlugin::default().with_stats(StatsSettings { probe: Some("NumGames".into()), ..Default::default() });
    /// # let _ = plugin;
    /// ```
    #[cfg(feature = "stats")]
    pub fn with_stats(mut self, settings: StatsSettings) -> Self {
        self.stats = settings;
        self
    }

    /// Use these settings for the leaderboards feature (feature `leaderboards`; default:
    /// `LeaderboardSettings::default()`).
    ///
    /// ```
    /// # use bevy_steam_kit::{LeaderboardSettings, SteamKitPlugin};
    /// let plugin = SteamKitPlugin::default().with_leaderboards(LeaderboardSettings { max_download_rows: 100, ..Default::default() });
    /// # let _ = plugin;
    /// ```
    #[cfg(feature = "leaderboards")]
    pub fn with_leaderboards(mut self, settings: LeaderboardSettings) -> Self {
        self.leaderboards = settings;
        self
    }

    /// Use these settings for the auth feature (feature `auth`; default: `AuthSettings::default()`).
    ///
    /// ```
    /// # use bevy_steam_kit::{AuthSettings, SteamKitPlugin};
    /// let plugin = SteamKitPlugin::default().with_auth(AuthSettings { timeout: std::time::Duration::from_secs(10), ..Default::default() });
    /// # let _ = plugin;
    /// ```
    #[cfg(feature = "auth")]
    pub fn with_auth(mut self, settings: AuthSettings) -> Self {
        self.auth = settings;
        self
    }

    /// Use these settings for the friends feature (feature `friends`; default:
    /// `FriendsSettings::default()`).
    ///
    /// ```
    /// # use bevy_steam_kit::{FriendsSettings, SteamKitPlugin};
    /// let plugin = SteamKitPlugin::default().with_friends(FriendsSettings { launch_connect_prefix: Some("+connect".into()), ..Default::default() });
    /// # let _ = plugin;
    /// ```
    #[cfg(feature = "friends")]
    pub fn with_friends(mut self, settings: FriendsSettings) -> Self {
        self.friends = settings;
        self
    }
}

/// The kit's system sets. `#[non_exhaustive]`: match with a `_` arm.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SteamKitSystems {
    /// `First`, before `MessageUpdateSystems`: pump Steam exactly once for the frame (the only
    /// `process_callbacks` call of the process) and buffer what arrived.
    Pump,
    /// `First`, after [`Pump`](Self::Pump) and before `MessageUpdateSystems`: every feature turns
    /// its share of the pumped events into state changes and fact messages, readable by this
    /// frame's `PreUpdate` / `Update`.
    ///
    /// Applying an event needs the backend (a late lobby is left, lobby data is written), so a
    /// feature applies this frame's events only while a backend supporting it is still installed.
    /// Do not remove [`SteamBackendRes`] between `Pump` and `Callbacks`: the events pumped in that
    /// frame are then dropped (with a warning). Remove it anywhere else (e.g. in `Update`). In
    /// the frame Steam is lost ([`SteamLost`]) the kit makes its backend inert after this set.
    Callbacks,
    /// `Update`: the features handle their request messages. `Last`: they clean up on `AppExit`
    /// (the lobby is left, stats are stored a last time, waiting leaderboard and auth requests
    /// are answered, live auth tickets are cancelled). Write `AppExit` anywhere before `Last` for
    /// that to happen.
    Requests,
}

/// Steam is gone for this process: the Steam client quit or its process ended (or the backend's
/// pump panicked and the kit caught it). Written once per backend, in `First` after [`SteamKitSystems::Callbacks`] of the frame it
/// happened (readable in that frame's `PreUpdate` / `Update`).
///
/// What the kit does: when Steam sent its shutdown callback, or the operating system reports the
/// Steam client process gone (killed or crashed), the real backend makes no Steam call from that
/// moment on (that pump's other events are dropped); otherwise (a backend reporting
/// `SteamLost` itself, a caught panic) the events pumped in that frame are applied as usual (none
/// after a caught panic). Then the backend inside [`SteamBackendRes`] is replaced by an inert one
/// (the resource stays: `local_id`, `friend_name` and `launch_command_line` still answer, `pump`
/// returns nothing, every feature accessor is `None`) and the backend is never pumped again. From
/// then on every feature behaves as without a backend: requests still waiting and new requests
/// are answered with their feature's `NoBackend` error, the friends list is cleared, an open
/// overlay is reported closed, and the lobby feature (feature `lobby`) clears `SteamLobby` (one
/// `LobbyLeft` for the lobby it was in) and answers a create or join in flight with one
/// `NoBackend` error. The game keeps running; it should stop calling Steam through its own
/// `steamworks::Client` too. The kit does not reconnect to a restarted Steam client (a
/// `RealSteamBackend` created later in the same process stays inert and reports `SteamLost` on its
/// first pump).
///
/// `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SteamLost {
    /// Why.
    pub reason: SteamLostReason,
}

/// Everything the backend returned from this frame's pump. Written only by `pump_steam`; every
/// feature reads its own variants in [`SteamKitSystems::Callbacks`] of the same frame. The second
/// field is set in the frame Steam was lost (the `SteamLost` event is taken out of the list).
#[derive(Resource, Default, Debug)]
pub(crate) struct PumpedEvents(pub(crate) Vec<BackendEvent>, pub(crate) Option<SteamLostReason>);

impl Plugin for SteamKitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PumpedEvents>()
            .add_message::<SteamLost>()
            .configure_sets(First, (SteamKitSystems::Pump, SteamKitSystems::Callbacks).chain().before(MessageUpdateSystems))
            .configure_sets(Update, SteamKitSystems::Requests)
            .configure_sets(Last, SteamKitSystems::Requests)
            .add_systems(First, pump_steam.in_set(SteamKitSystems::Pump))
            .add_systems(First, on_steam_lost.after(SteamKitSystems::Callbacks).before(MessageUpdateSystems));

        #[cfg(feature = "lobby")]
        lobby::build(app, &self.lobby);
        #[cfg(feature = "stats")]
        stats::build(app, &self.stats);
        #[cfg(feature = "leaderboards")]
        leaderboards::build(app, &self.leaderboards);
        #[cfg(feature = "auth")]
        auth::build(app, &self.auth);
        #[cfg(feature = "friends")]
        friends::build(app, &self.friends);
        #[cfg(feature = "overlay")]
        overlay::build(app);
    }
}

/// `First`: THE callback pump. Replaces last frame's events with this frame's (none without a
/// backend), so a feature never sees an event twice.
///
/// A panic inside the backend's pump is caught here (it only can be in a build that unwinds; with
/// `panic = "abort"` the process ends inside the pump): that frame's events are dropped and Steam
/// is treated as lost. The real backend prevents the known panics (Steam's shutdown callback, a
/// connect string that is not UTF-8; see `RealSteamBackend`) before they happen, so this is the
/// second line.
fn pump_steam(backend: Option<Res<SteamBackendRes>>, mut pumped: ResMut<PumpedEvents>) {
    pumped.0.clear();
    pumped.1 = None;
    let Some(backend) = backend else { return };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.0.pump())) {
        Ok(mut events) => {
            // The core's own event: taken out, so features see only theirs.
            events.retain(|ev| match ev {
                BackendEvent::SteamLost { reason } => {
                    pumped.1.get_or_insert(*reason);
                    false
                }
                #[allow(unreachable_patterns)]
                _ => true,
            });
            pumped.0 = events;
        }
        Err(payload) => {
            let what = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("(no message)");
            tracing::error!(">>> STEAM: the Steam callback pump panicked: {what} - this frame's Steam events are dropped and Steam is treated as lost");
            pumped.1 = Some(SteamLostReason::PumpPanicked);
        }
    }
}

/// What [`SteamBackendRes`] holds after Steam was lost: it never pumps, supports no feature (so
/// every feature answers `NoBackend`), and answers the identity queries from the backend it
/// replaced (kept alive, never pumped again).
struct LostBackend(Option<Box<dyn SteamBackend>>);

impl SteamBackend for LostBackend {
    fn local_id(&self) -> u64 {
        self.0.as_ref().map_or(0, |b| b.local_id())
    }
    fn friend_name(&self, id: u64) -> String {
        self.0.as_ref().map(|b| b.friend_name(id)).unwrap_or_default()
    }
    fn launch_command_line(&self) -> String {
        self.0.as_ref().map(|b| b.launch_command_line()).unwrap_or_default()
    }
    fn pump(&self) -> Vec<BackendEvent> {
        Vec::new()
    }
}

/// `First`, after [`SteamKitSystems::Callbacks`]: in the frame Steam was lost, replace the backend
/// inside [`SteamBackendRes`] by an inert one (after the features applied that frame's events) and
/// write [`SteamLost`] once. The resource itself stays, so a game system taking
/// `Res<SteamBackendRes>` keeps running.
fn on_steam_lost(mut commands: Commands, pumped: Res<PumpedEvents>, mut lost: MessageWriter<SteamLost>) {
    let Some(reason) = pumped.1 else { return };
    tracing::error!(
        ">>> STEAM: Steam is gone ({reason:?}): the kit stopped pumping Steam; every Steam request is answered NoBackend from now on, the game keeps running"
    );
    commands.queue(|world: &mut World| {
        if let Some(mut res) = world.get_resource_mut::<SteamBackendRes>() {
            let old = std::mem::replace(&mut res.0, Box::new(LostBackend(None)));
            res.0 = Box::new(LostBackend(Some(old)));
        }
    });
    lost.write(SteamLost { reason });
}
