//! The testable seam between the kit's systems and Steam.
//!
//! The systems never touch `steamworks` directly: they talk to a [`SteamBackend`] stored in
//! [`SteamBackendRes`]. The real implementation is [`crate::RealSteamBackend`] (feature `steam`);
//! tests and builds without Steam use [`crate::FakeSteamBackend`].
//!
//! The trait is split by feature so a build compiles only what it enabled: [`SteamBackend`] holds
//! what every feature shares (identity, launch command line, the one callback pump), and each
//! feature adds its own trait, reached through an accessor with a `None` default (for example
//! `SteamBackend::lobby` with feature `lobby`). A backend written against a smaller feature set
//! therefore keeps compiling when another crate in the build enables more features; it simply
//! reports that it does not support them.

use bevy_ecs::prelude::Resource;

/// A raw event produced by a backend's [`SteamBackend::pump`]. The kit collects them once per
/// frame in [`crate::SteamKitSystems::Pump`] and every compiled feature turns its own variants into
/// public messages in [`crate::SteamKitSystems::Callbacks`].
///
/// `#[non_exhaustive]`: variants are added by features (and by future versions), so a `match`
/// outside this crate needs a `_` arm. `PartialEq` only (no `Eq`), so a later variant may carry a
/// float.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BackendEvent {
    /// A `create_lobby` call completed successfully.
    #[cfg(feature = "lobby")]
    LobbyCreated {
        /// The new lobby's raw id.
        lobby: u64,
    },
    /// A `create_lobby` call failed.
    #[cfg(feature = "lobby")]
    LobbyCreateFailed {
        /// Human-readable reason from Steam.
        message: String,
    },
    /// A `join_lobby` call completed successfully.
    #[cfg(feature = "lobby")]
    LobbyEntered {
        /// The joined lobby's raw id.
        lobby: u64,
    },
    /// A `join_lobby` call failed.
    #[cfg(feature = "lobby")]
    LobbyJoinFailed {
        /// The lobby that could not be joined.
        lobby: u64,
    },
    /// Steam `GameLobbyJoinRequested` (lobby invite accepted / "Join Game" on a friend).
    #[cfg(feature = "lobby")]
    LobbyJoinRequested {
        /// The lobby to join.
        lobby: u64,
        /// The friend it came from (raw SteamID64).
        from: u64,
    },
    /// Steam `GameRichPresenceJoinRequested`: the raw `connect` string, parsed by the lobby feature.
    #[cfg(feature = "lobby")]
    RichPresenceJoinRequested {
        /// The friend it came from (raw SteamID64; may be invalid when not from a friend).
        from: u64,
        /// The rich-presence connect string.
        connect: String,
    },
    /// Steam `UserStatsReceived`: stats of `user` were (re)loaded.
    #[cfg(feature = "stats")]
    StatsReceived {
        /// Whose stats (raw SteamID64). The stats feature acts only on the local user's.
        user: u64,
        /// `false` when Steam reported a failure.
        ok: bool,
    },
    /// Steam `UserStatsStored` with success: a `store_stats` completed.
    #[cfg(feature = "stats")]
    StatsStored,
    /// Steam `UserStatsStored` with `InvalidParameter`: at least one stat broke a constraint set on
    /// the partner site; Steam restores the server's values.
    #[cfg(feature = "stats")]
    StatsStoreRejected,
    /// Steam `UserStatsStored` with any other failure.
    #[cfg(feature = "stats")]
    StatsStoreFailed {
        /// Human-readable reason from Steam.
        message: String,
    },
    /// Steam `UserAchievementStored`: an achievement was stored (`current == 0 && max == 0`: it
    /// is unlocked) or its progress was shown (`IndicateAchievementProgress`).
    #[cfg(feature = "stats")]
    AchievementStored {
        /// Achievement API name.
        name: String,
        /// Progress shown (0 for an unlock).
        current: u32,
        /// Progress maximum (0 for an unlock).
        max: u32,
    },
    /// A leaderboard find completed: the board exists (or was created).
    #[cfg(feature = "leaderboards")]
    LeaderboardFound {
        /// The operation this answers.
        op: u64,
        /// The board's raw handle.
        board: u64,
    },
    /// A leaderboard find completed: no such board.
    #[cfg(feature = "leaderboards")]
    LeaderboardNotFound {
        /// The operation this answers.
        op: u64,
    },
    /// A score upload completed.
    #[cfg(feature = "leaderboards")]
    LeaderboardScoreUploaded {
        /// The operation this answers.
        op: u64,
        /// The score that was uploaded.
        score: i32,
        /// The board's entry for the player changed.
        changed: bool,
        /// The player's global rank now.
        rank_new: i32,
        /// The player's global rank before (0: no entry before).
        rank_previous: i32,
    },
    /// A score upload was refused by Steam (Steam gives no reason).
    #[cfg(feature = "leaderboards")]
    LeaderboardUploadRejected {
        /// The operation this answers.
        op: u64,
    },
    /// A download completed.
    #[cfg(feature = "leaderboards")]
    LeaderboardScoresDownloaded {
        /// The operation this answers.
        op: u64,
        /// The entries, best first.
        entries: Vec<crate::LeaderboardEntry>,
    },
    /// A leaderboard call completed with Steam's `IOFailure` (the only failure steamworks 0.12.2
    /// reports for leaderboards).
    #[cfg(feature = "leaderboards")]
    LeaderboardIoFailure {
        /// The operation this answers.
        op: u64,
    },
}

/// What the kit's core needs from Steam, plus one accessor per optional feature. Implementations
/// must never panic; a failure is a `false` / `None` / an error event.
///
/// Asynchronous calls return nothing: their outcome is queued and returned by a later
/// [`pump`](Self::pump).
pub trait SteamBackend: Send + Sync + 'static {
    /// The local user's SteamID64.
    fn local_id(&self) -> u64;
    /// A user's persona name ("" when unknown).
    fn friend_name(&self, id: u64) -> String;
    /// The command line Steam launched the game with ("" when none).
    fn launch_command_line(&self) -> String;
    /// Pump Steam callbacks once and return everything that arrived since the last pump
    /// (callbacks + completed call results). Called by the kit exactly once per frame, in
    /// [`crate::SteamKitSystems::Pump`]; never call it yourself.
    fn pump(&self) -> Vec<BackendEvent>;

    /// The lobby / rich presence / invite half of this backend (feature `lobby`). The default is
    /// `None`: the lobby feature then treats Steam as unavailable and answers requests with
    /// [`crate::LobbyErrorKind::NoBackend`].
    #[cfg(feature = "lobby")]
    fn lobby(&self) -> Option<&dyn crate::LobbyBackend> {
        None
    }

    /// The stats / achievements half of this backend (feature `stats`). The default is `None`:
    /// the stats feature then treats Steam as unavailable and answers requests with
    /// [`crate::StatsErrorKind::NoBackend`].
    #[cfg(feature = "stats")]
    fn stats(&self) -> Option<&dyn crate::StatsBackend> {
        None
    }

    /// The leaderboard half of this backend (feature `leaderboards`). The default is `None`: the
    /// leaderboards feature then treats Steam as unavailable and answers requests with
    /// [`crate::LeaderboardErrorKind::NoBackend`].
    #[cfg(feature = "leaderboards")]
    fn leaderboards(&self) -> Option<&dyn crate::LeaderboardBackend> {
        None
    }
}

/// The active backend. Insert it to make the kit live; without it every system is inert (and
/// feature requests are answered with a `NoBackend` error).
#[derive(Resource)]
pub struct SteamBackendRes(pub Box<dyn SteamBackend>);
