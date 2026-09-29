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
//! - `steam`: the real backend, `RealSteamBackend`, over `steamworks` 0.12.2. Without it the
//!   crate still builds (no Steam SDK runtime needed) and [`FakeSteamBackend`] drives everything.
//!
//! The kit is inert until a [`SteamBackendRes`] exists: insert
//! `SteamBackendRes(Box::new(RealSteamBackend::new(client)))` (feature `steam`) when Steam
//! initialised, or a [`FakeSteamBackend`] in tests. The game initialises Steam itself, keeps its own
//! `steamworks::Client` clone for any other Steam API, and never pumps it: exactly one
//! `process_callbacks` per frame for the whole process, and that is the kit's.
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod backend;
mod fake;
#[cfg(feature = "leaderboards")]
pub mod leaderboards;
#[cfg(feature = "lobby")]
pub mod lobby;
#[cfg(any(feature = "stats", feature = "leaderboards"))]
mod names;
#[cfg(feature = "steam")]
mod real;
#[cfg(feature = "stats")]
pub mod stats;
mod steam_id;
#[cfg(test)]
mod tests;

/// Every Rust example in the README compiles (checked by `cargo test --all-features`; the README
/// uses every feature and `RealSteamBackend`, so the check needs all of them).
#[cfg(all(doctest, feature = "steam", feature = "lobby", feature = "stats", feature = "leaderboards"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub use backend::{BackendEvent, SteamBackend, SteamBackendRes};
pub use fake::{FakeCall, FakeSteamBackend};
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
}

/// The kit's system sets. `#[non_exhaustive]`: a later version may add a set.
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
    /// frame are then dropped (with a warning). Remove it anywhere else (e.g. in `Update`).
    Callbacks,
    /// `Update`: the features handle their request messages. `Last`: they clean up on `AppExit`
    /// (the lobby is left, stats are stored a last time, waiting leaderboard requests are
    /// answered). Write `AppExit` anywhere before `Last` for that to happen.
    Requests,
}

/// Everything the backend returned from this frame's pump. Written only by `pump_steam`; every
/// feature reads its own variants in [`SteamKitSystems::Callbacks`] of the same frame.
#[derive(Resource, Default, Debug)]
pub(crate) struct PumpedEvents(pub(crate) Vec<BackendEvent>);

impl Plugin for SteamKitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PumpedEvents>()
            .configure_sets(First, (SteamKitSystems::Pump, SteamKitSystems::Callbacks).chain().before(MessageUpdateSystems))
            .configure_sets(Update, SteamKitSystems::Requests)
            .configure_sets(Last, SteamKitSystems::Requests)
            .add_systems(First, pump_steam.in_set(SteamKitSystems::Pump));

        #[cfg(feature = "lobby")]
        lobby::build(app, &self.lobby);
        #[cfg(feature = "stats")]
        stats::build(app, &self.stats);
        #[cfg(feature = "leaderboards")]
        leaderboards::build(app, &self.leaderboards);
    }
}

/// `First`: THE callback pump. Replaces last frame's events with this frame's (none without a
/// backend), so a feature never sees an event twice.
fn pump_steam(backend: Option<Res<SteamBackendRes>>, mut pumped: ResMut<PumpedEvents>) {
    pumped.0.clear();
    if let Some(backend) = backend {
        pumped.0 = backend.0.pump();
    }
}
