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
/// `#[non_exhaustive]`: the variants depend on the compiled features, so a `match` outside this
/// crate needs a `_` arm. `PartialEq` only (no `Eq`), so a variant may carry a float.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BackendEvent {
    /// Steam is gone for this process (the core's event, compiled with every feature set). After
    /// this frame's events were applied, the kit replaces the backend inside [`SteamBackendRes`]
    /// by an inert one (it is never pumped again) and writes [`crate::SteamLost`]. A backend
    /// returns it once.
    SteamLost {
        /// Why.
        reason: SteamLostReason,
    },
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
    /// Steam `GetTicketForWebApiResponse`: the answer to a Web API ticket request. The ticket's
    /// `Debug` shows its length only.
    #[cfg(feature = "auth")]
    WebApiTicket {
        /// The operation this answers.
        op: u64,
        /// The ticket, or Steam's failure text.
        result: Result<crate::WebApiTicket, String>,
    },
    /// Steam `PersonaStateChange`: something about a user changed (name, status, game, avatar,
    /// the friendship itself, ...).
    #[cfg(feature = "friends")]
    PersonaChanged {
        /// The user (raw SteamID64).
        steam_id: u64,
        /// Steam's raw `EPersonaChange` bits: `0x1` name, `0x2` status, `0x4` came online, `0x8`
        /// went offline, `0x10` game played, `0x20` game server, `0x40` avatar, `0x200`
        /// relationship (friend added or removed), `0x1000` nickname, ...
        flags: u32,
    },
    /// Steam `GameRichPresenceJoinRequested`, raw: the connect string as Steam delivered it (the
    /// `lobby` feature reads the same callback as its own `RichPresenceJoinRequested`).
    #[cfg(feature = "friends")]
    ConnectRequested {
        /// The friend it came from (raw SteamID64; may be invalid when not from a friend).
        from: u64,
        /// The connect string.
        connect: String,
    },
    /// Steam `GameOverlayActivated`: the overlay opened (`true`) or closed (`false`).
    #[cfg(feature = "overlay")]
    OverlayActivated {
        /// The overlay is open now.
        active: bool,
    },
}

/// Why Steam is gone ([`BackendEvent::SteamLost`], [`crate::SteamLost`]). `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SteamLostReason {
    /// The Steam client quit (it sent its shutdown callback).
    SteamExited,
    /// The Steam client process ended without its shutdown callback (killed or crashed), as the
    /// operating system reports it (or Steam's own process check where the kit has none).
    SteamProcessEnded,
    /// The backend's pump panicked (for example inside `steamworks`); the kit caught the panic.
    /// Only possible in a build that unwinds on panic: with `panic = "abort"` the process ends.
    /// After a panic of the real backend's pump, no `RealSteamBackend` of the process pumps or
    /// calls Steam again.
    PumpPanicked,
}

/// What the kit's core needs from Steam, plus one accessor per optional feature. Implementations
/// must never panic; a failure is a `false` / `None` / an error event.
///
/// Asynchronous calls return nothing: their outcome is queued and returned by a following
/// [`pump`](Self::pump).
pub trait SteamBackend: Send + Sync + 'static {
    /// The local user's SteamID64.
    fn local_id(&self) -> u64;
    /// A user's persona name. What an unknown user gives is the backend's: real Steam returns
    /// `"[unknown]"` for a user it knows nothing about (Valve's `GetFriendPersonaName`), the fake
    /// backend returns `""`.
    fn friend_name(&self, id: u64) -> String;
    /// The command line Steam launched the game with ("" when none).
    fn launch_command_line(&self) -> String;
    /// Pump Steam callbacks once and return everything that arrived since the last pump
    /// (callbacks + completed call results). Called by the kit exactly once per frame, in
    /// [`crate::SteamKitSystems::Pump`]; never call it yourself. A backend that learns Steam is
    /// gone returns [`BackendEvent::SteamLost`]; a panic in here is caught by the kit (in a build
    /// that unwinds) and treated the same way ([`SteamLostReason::PumpPanicked`]).
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

    /// The Web API ticket half of this backend (feature `auth`). The default is `None`: the auth
    /// feature then answers requests with [`crate::AuthErrorKind::NoBackend`].
    #[cfg(feature = "auth")]
    fn auth(&self) -> Option<&dyn crate::AuthBackend> {
        None
    }

    /// The friends half of this backend (feature `friends`). The default is `None`: the friends
    /// feature then treats Steam as unavailable ([`crate::FriendsErrorKind::NoBackend`]).
    #[cfg(feature = "friends")]
    fn friends(&self) -> Option<&dyn crate::FriendsBackend> {
        None
    }

    /// The overlay half of this backend (feature `overlay`). The default is `None`: the overlay
    /// feature then answers requests with [`crate::OverlayErrorKind::NoBackend`].
    #[cfg(feature = "overlay")]
    fn overlay(&self) -> Option<&dyn crate::OverlayBackend> {
        None
    }
}

/// The active backend. Insert it to make the kit live; without it every system is inert (and
/// feature requests are answered with a `NoBackend` error). When Steam is gone
/// ([`crate::SteamLost`]) the kit keeps the resource but replaces the backend in it by an inert
/// one (no pump, no feature; the identity queries still answer).
#[derive(Resource)]
pub struct SteamBackendRes(pub Box<dyn SteamBackend>);
