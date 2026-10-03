//! [`FakeSteamBackend`]: an in-memory Steam for tests and for driving the kit without Steam.
//! It never touches the network or the Steam client. The feature halves live in `<feature>/fake.rs`
//! (`lobby`, `stats`, `leaderboards`, `auth`, `friends`, `overlay`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{BackendEvent, SteamBackend};

/// One recorded call into the fake backend (queries such as `lobby_data` are not recorded).
///
/// `#[non_exhaustive]`: every feature adds the calls it makes. `PartialEq` only (no `Eq`), so a
/// variant may carry a float.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FakeCall {
    /// `create_lobby(kind, max_members)`.
    #[cfg(feature = "lobby")]
    CreateLobby {
        /// Requested visibility.
        kind: crate::LobbyKind,
        /// Requested member cap.
        max_members: u32,
    },
    /// `join_lobby(lobby)`.
    #[cfg(feature = "lobby")]
    JoinLobby(u64),
    /// `leave_lobby(lobby)`.
    #[cfg(feature = "lobby")]
    LeaveLobby(u64),
    /// `set_lobby_data(lobby, key, value)`.
    #[cfg(feature = "lobby")]
    SetLobbyData {
        /// Lobby.
        lobby: u64,
        /// Key.
        key: String,
        /// Value.
        value: String,
    },
    /// `set_lobby_joinable(lobby, joinable)`.
    #[cfg(feature = "lobby")]
    SetLobbyJoinable {
        /// Lobby.
        lobby: u64,
        /// Joinable flag.
        joinable: bool,
    },
    /// `set_rich_presence(key, value)`.
    #[cfg(feature = "lobby")]
    SetRichPresence {
        /// Key.
        key: String,
        /// Value (`None` = remove).
        value: Option<String>,
    },
    /// `clear_rich_presence()`.
    #[cfg(feature = "lobby")]
    ClearRichPresence,
    /// A game invite with a connect string: `invite_to_game(friend, connect)` (lobby) or
    /// `invite_user_to_game(friend, connect)` (friends); the same Steam call.
    #[cfg(any(feature = "lobby", feature = "friends"))]
    InviteToGame {
        /// Invited user.
        friend: u64,
        /// Connect string sent.
        connect: String,
    },
    /// `set_stat(name, value)`.
    #[cfg(feature = "stats")]
    SetStat {
        /// Stat API name.
        name: String,
        /// Value written.
        value: crate::StatValue,
    },
    /// `unlock_achievement(name)`.
    #[cfg(feature = "stats")]
    UnlockAchievement(String),
    /// `clear_achievement(name)`.
    #[cfg(feature = "stats")]
    ClearAchievement(String),
    /// `indicate_achievement_progress(name, current, max)`.
    #[cfg(feature = "stats")]
    IndicateAchievementProgress {
        /// Achievement API name.
        name: String,
        /// Current progress.
        current: u32,
        /// Maximum.
        max: u32,
    },
    /// `store_stats()`.
    #[cfg(feature = "stats")]
    StoreStats,
    /// `reset_all_stats(achievements_too)`.
    #[cfg(feature = "stats")]
    ResetAllStats {
        /// Achievements were reset too.
        achievements_too: bool,
    },
    /// `find_leaderboard(op, name, create)`.
    #[cfg(feature = "leaderboards")]
    FindLeaderboard {
        /// Board name.
        name: String,
        /// Find-or-create.
        create: bool,
    },
    /// `upload_score(op, board, method, score, details)`.
    #[cfg(feature = "leaderboards")]
    UploadScore {
        /// Board handle.
        board: u64,
        /// Method.
        method: crate::UploadMethod,
        /// Score.
        score: i32,
        /// Details.
        details: Vec<i32>,
    },
    /// `download_scores(op, board, range)`.
    #[cfg(feature = "leaderboards")]
    DownloadScores {
        /// Board handle.
        board: u64,
        /// Range.
        range: crate::ScoreRange,
    },
    /// `request_web_api_ticket(op, identity)`.
    #[cfg(feature = "auth")]
    RequestWebApiTicket {
        /// The identity the ticket was requested for.
        identity: String,
    },
    /// `cancel_auth_ticket(op)`.
    #[cfg(feature = "auth")]
    CancelAuthTicket {
        /// The backend operation whose ticket was cancelled.
        op: u64,
    },
    /// `request_user_information(id, name_only)`.
    #[cfg(feature = "friends")]
    RequestUserInformation {
        /// The user.
        id: u64,
        /// Only the name was asked for.
        name_only: bool,
    },
    /// One overlay call (`open_overlay*` / `open_invite_dialog*`), as the request that caused it.
    #[cfg(feature = "overlay")]
    ActivateOverlay(crate::OpenOverlay),
}

#[derive(Debug)]
pub(crate) struct FakeState {
    pub(crate) local_id: u64,
    pumps: u64,
    pub(crate) calls: Vec<FakeCall>,
    pub(crate) queued: Vec<BackendEvent>,
    pub(crate) friend_names: HashMap<u64, String>,
    launch_command_line: String,
    /// The next pump panics (`panic_in_next_pump`).
    panic_next_pump: bool,
    /// `simulate_steam_shutdown`: no feature from now on; `SteamLost` on the next pump.
    steam_gone: Option<crate::SteamLostReason>,
    /// `SteamLost` was returned for `steam_gone`.
    gone_reported: bool,
    /// The running app's id (`set_app_id`; default 480).
    #[cfg_attr(not(feature = "friends"), allow(dead_code))]
    pub(crate) app_id: u32,
    #[cfg(feature = "lobby")]
    pub(crate) lobby: crate::lobby::fake::FakeLobbyState,
    #[cfg(feature = "stats")]
    pub(crate) stats: crate::stats::fake::FakeStatsState,
    #[cfg(feature = "leaderboards")]
    pub(crate) boards: crate::leaderboards::fake::FakeBoardsState,
    #[cfg(feature = "auth")]
    pub(crate) auth: crate::auth::fake::FakeAuthState,
    #[cfg(feature = "friends")]
    pub(crate) friends: crate::friends::fake::FakeFriendsState,
    #[cfg(feature = "overlay")]
    pub(crate) overlay: crate::overlay::fake::FakeOverlayState,
}

/// An in-memory Steam. Cheap to clone: every clone shares the same state, so a test keeps one
/// clone to inspect while the kit owns another inside [`crate::SteamBackendRes`].
///
/// Defaults: local id `76561197960265729`, app id 480; with feature `lobby`, `create_lobby`
/// completes on the next pump with ids `1000, 1001, ...` and `join_lobby` succeeds on the next
/// pump; with feature `stats`, stats are ready, no stat or achievement is defined, and a store
/// succeeds on the next pump; with `auth`, a ticket arrives on the next pump; with `friends`, no
/// friends; with `overlay`, the overlay is enabled. Each feature's knobs are listed with its
/// methods.
#[derive(Clone, Debug)]
pub struct FakeSteamBackend {
    state: Arc<Mutex<FakeState>>,
}

impl Default for FakeSteamBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSteamBackend {
    /// A fresh fake Steam with the defaults described on the type.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                local_id: 76_561_197_960_265_729,
                pumps: 0,
                calls: Vec::new(),
                queued: Vec::new(),
                friend_names: HashMap::new(),
                launch_command_line: String::new(),
                panic_next_pump: false,
                steam_gone: None,
                gone_reported: false,
                app_id: 480,
                #[cfg(feature = "lobby")]
                lobby: crate::lobby::fake::FakeLobbyState::default(),
                #[cfg(feature = "stats")]
                stats: crate::stats::fake::FakeStatsState::default(),
                #[cfg(feature = "leaderboards")]
                boards: crate::leaderboards::fake::FakeBoardsState::default(),
                #[cfg(feature = "auth")]
                auth: crate::auth::fake::FakeAuthState::default(),
                #[cfg(feature = "friends")]
                friends: crate::friends::fake::FakeFriendsState::default(),
                #[cfg(feature = "overlay")]
                overlay: crate::overlay::fake::FakeOverlayState::default(),
            })),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, FakeState> {
        // A poisoned mutex only means a test thread panicked mid-call; the data is still usable.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Every call recorded so far, in order.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.lock().calls.clone()
    }

    /// How many times [`SteamBackend::pump`] has been called (the kit pumps exactly once per
    /// frame while this backend is installed).
    pub fn pump_count(&self) -> u64 {
        self.lock().pumps
    }

    /// Set the local user's SteamID64.
    pub fn set_local_id(&self, id: u64) {
        self.lock().local_id = id;
    }

    /// Queue any raw backend event (e.g. a join request) for the next pump.
    pub fn push_event(&self, event: BackendEvent) {
        self.lock().queued.push(event);
    }

    /// Give a user a persona name.
    /// With feature `friends` this also queues the `PersonaChanged` (name) event Steam sends.
    pub fn set_friend_name(&self, id: u64, name: &str) {
        let mut s = self.lock();
        s.friend_names.insert(id, name.to_string());
        #[cfg(feature = "friends")]
        s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: 0x0001 });
    }

    /// Set what `launch_command_line` returns.
    pub fn set_launch_command_line(&self, text: &str) {
        self.lock().launch_command_line = text.to_string();
    }

    /// Queue what the real backend reports when the Steam client exits while the game runs
    /// ([`BackendEvent::SteamLost`] with [`crate::SteamLostReason::SteamExited`]) for the next
    /// pump: the kit then makes the backend inert (never pumped again) and writes
    /// [`crate::SteamLost`].
    pub fn simulate_steam_exit(&self) {
        self.lock().queued.push(BackendEvent::SteamLost { reason: crate::SteamLostReason::SteamExited });
    }

    /// What the real backend does when Steam sends its shutdown callback: from this call on the
    /// backend supports no feature (every feature accessor is `None`, so the kit cannot reach any
    /// feature call), the next pump drops what was queued and returns
    /// [`BackendEvent::SteamLost`] once, later pumps return nothing. [`calls`](Self::calls)
    /// shows that nothing reaches the backend afterwards.
    pub fn simulate_steam_shutdown(&self) {
        self.lock().steam_gone = Some(crate::SteamLostReason::SteamExited);
    }

    /// What the real backend does when the operating system reports the Steam client process gone
    /// (killed or crashed, no shutdown callback): the same as [`simulate_steam_shutdown`](Self::simulate_steam_shutdown),
    /// with [`crate::SteamLostReason::SteamProcessEnded`].
    pub fn simulate_steam_process_ended(&self) {
        self.lock().steam_gone = Some(crate::SteamLostReason::SteamProcessEnded);
    }

    #[cfg_attr(
        not(any(feature = "lobby", feature = "stats", feature = "leaderboards", feature = "auth", feature = "friends", feature = "overlay")),
        allow(dead_code)
    )]
    fn gone(&self) -> bool {
        self.lock().steam_gone.is_some()
    }

    /// The next pump PANICS (like a panic inside `steamworks`), to test that the kit catches it
    /// (in a build that unwinds): that frame's queued events are dropped and Steam is treated as
    /// lost. Rust's panic hook prints the panic message as usual.
    pub fn panic_in_next_pump(&self) {
        self.lock().panic_next_pump = true;
    }

    /// Set the running app's id (default 480), as the `friends` feature sees it.
    pub fn set_app_id(&self, app_id: u32) {
        self.lock().app_id = app_id;
    }

    /// Queue what real Steam produces for a rich-presence "Join Game" or an accepted invite while
    /// the game runs (`GameRichPresenceJoinRequested`): one event per compiled feature that reads
    /// it (`RichPresenceJoinRequested` for `lobby`, `ConnectRequested` for `friends`), as the real
    /// backend does.
    #[cfg(any(feature = "lobby", feature = "friends"))]
    pub fn push_rich_presence_join(&self, from: u64, connect: &str) {
        let mut s = self.lock();
        #[cfg(feature = "lobby")]
        s.queued.push(BackendEvent::RichPresenceJoinRequested { from, connect: connect.to_string() });
        #[cfg(feature = "friends")]
        s.queued.push(BackendEvent::ConnectRequested { from, connect: connect.to_string() });
    }
}

impl SteamBackend for FakeSteamBackend {
    fn local_id(&self) -> u64 {
        self.lock().local_id
    }

    fn friend_name(&self, id: u64) -> String {
        self.lock().friend_names.get(&id).cloned().unwrap_or_default()
    }

    fn launch_command_line(&self) -> String {
        self.lock().launch_command_line.clone()
    }

    fn pump(&self) -> Vec<BackendEvent> {
        let mut s = self.lock();
        s.pumps = s.pumps.saturating_add(1);
        if let Some(reason) = s.steam_gone {
            s.queued.clear();
            if std::mem::replace(&mut s.gone_reported, true) {
                return Vec::new();
            }
            return vec![BackendEvent::SteamLost { reason }];
        }
        if std::mem::take(&mut s.panic_next_pump) {
            // The queued events are lost with the panic, as a real pump's would be.
            s.queued.clear();
            drop(s);
            panic!("FakeSteamBackend: simulated panic in the pump");
        }
        std::mem::take(&mut s.queued)
    }

    #[cfg(feature = "lobby")]
    fn lobby(&self) -> Option<&dyn crate::LobbyBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "stats")]
    fn stats(&self) -> Option<&dyn crate::StatsBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "leaderboards")]
    fn leaderboards(&self) -> Option<&dyn crate::LeaderboardBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "auth")]
    fn auth(&self) -> Option<&dyn crate::AuthBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "friends")]
    fn friends(&self) -> Option<&dyn crate::FriendsBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "overlay")]
    fn overlay(&self) -> Option<&dyn crate::OverlayBackend> {
        (!self.gone()).then_some(self)
    }
}
