//! [`FakeSteamBackend`]: an in-memory Steam for tests and for driving the kit without Steam.
//! It never touches the network or the Steam client. The feature halves live in `lobby/fake.rs`,
//! `stats/fake.rs` and `leaderboards/fake.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{BackendEvent, SteamBackend};

/// One recorded call into the fake backend (queries such as `lobby_data` are not recorded).
///
/// `#[non_exhaustive]`: every feature adds the calls it makes. `PartialEq` only (no `Eq`), so a
/// later variant may carry a float.
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
    /// `invite_to_game(friend, connect)`.
    #[cfg(feature = "lobby")]
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
}

#[derive(Debug)]
pub(crate) struct FakeState {
    pub(crate) local_id: u64,
    pumps: u64,
    pub(crate) calls: Vec<FakeCall>,
    pub(crate) queued: Vec<BackendEvent>,
    friend_names: HashMap<u64, String>,
    launch_command_line: String,
    #[cfg(feature = "lobby")]
    pub(crate) lobby: crate::lobby::fake::FakeLobbyState,
    #[cfg(feature = "stats")]
    pub(crate) stats: crate::stats::fake::FakeStatsState,
    #[cfg(feature = "leaderboards")]
    pub(crate) boards: crate::leaderboards::fake::FakeBoardsState,
}

/// An in-memory Steam. Cheap to clone: every clone shares the same state, so a test keeps one
/// clone to inspect while the kit owns another inside [`crate::SteamBackendRes`].
///
/// Defaults: local id `76561197960265729`; with feature `lobby`, `create_lobby` completes on the
/// next pump with ids `1000, 1001, ...` and `join_lobby` succeeds on the next pump; with feature
/// `stats`, stats are ready, no stat or achievement is defined, and a store succeeds on the next
/// pump.
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
                #[cfg(feature = "lobby")]
                lobby: crate::lobby::fake::FakeLobbyState::default(),
                #[cfg(feature = "stats")]
                stats: crate::stats::fake::FakeStatsState::default(),
                #[cfg(feature = "leaderboards")]
                boards: crate::leaderboards::fake::FakeBoardsState::default(),
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
    pub fn set_friend_name(&self, id: u64, name: &str) {
        self.lock().friend_names.insert(id, name.to_string());
    }

    /// Set what `launch_command_line` returns.
    pub fn set_launch_command_line(&self, text: &str) {
        self.lock().launch_command_line = text.to_string();
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
        std::mem::take(&mut s.queued)
    }

    #[cfg(feature = "lobby")]
    fn lobby(&self) -> Option<&dyn crate::LobbyBackend> {
        Some(self)
    }

    #[cfg(feature = "stats")]
    fn stats(&self) -> Option<&dyn crate::StatsBackend> {
        Some(self)
    }

    #[cfg(feature = "leaderboards")]
    fn leaderboards(&self) -> Option<&dyn crate::LeaderboardBackend> {
        Some(self)
    }
}
