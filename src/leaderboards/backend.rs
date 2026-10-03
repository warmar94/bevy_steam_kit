//! The leaderboard half of the backend seam: [`LeaderboardBackend`], reached through
//! [`SteamBackend::leaderboards`](crate::SteamBackend::leaderboards), and the types it uses.

/// Steam's limit on the details (extra `i32`s) stored with one leaderboard entry
/// (`k_cLeaderboardDetailsMax`).
pub const MAX_LEADERBOARD_DETAILS: usize = 64;

/// How a leaderboard ranks its scores. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LeaderboardSort {
    /// The lowest score is best (times).
    Ascending,
    /// The highest score is best (points).
    Descending,
}

/// How Steam shows a leaderboard's scores on the Steam Community site. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LeaderboardDisplay {
    /// A plain number.
    Numeric,
    /// The score is a time in seconds.
    TimeSeconds,
    /// The score is a time in milliseconds.
    TimeMilliseconds,
}

/// How an upload treats the player's existing entry. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UploadMethod {
    /// Keep the better of the old and the new score (by the board's sort).
    KeepBest,
    /// Always replace the old score.
    ForceUpdate,
}

/// Which entries to download. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScoreRange {
    /// Global ranks `first..=last` (1-based; `first >= 1`, `last >= first`).
    Global {
        /// First rank (1 = the best).
        first: u32,
        /// Last rank, inclusive.
        last: u32,
    },
    /// The local player's entry with `before` entries above and `after` below. Empty when the
    /// player has no entry.
    AroundUser {
        /// Entries above the player.
        before: u32,
        /// Entries below the player.
        after: u32,
    },
    /// The local player and their Steam friends (every one of them that has an entry).
    Friends,
}

/// One downloaded leaderboard entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderboardEntry {
    /// Global rank (1 = the best).
    pub rank: i32,
    /// The player's SteamID64.
    pub steam_id: u64,
    /// The score.
    pub score: i32,
    /// The details stored with the score (at most the number requested).
    pub details: Vec<i32>,
}

/// Everything the leaderboards feature needs from Steam. Implementations must never panic.
///
/// Asynchronous calls return whether the call was started (`false`: refused, e.g. an unknown
/// handle); their outcome is returned as a [`BackendEvent`](crate::BackendEvent) carrying the same
/// `op` by a following [`SteamBackend::pump`](crate::SteamBackend::pump). An implementation must NEVER
/// call Steam from inside a call-result closure (steamworks holds its own lock there: a nested
/// call deadlocks); closures only queue events.
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added to this trait comes with a default implementation.
pub trait LeaderboardBackend {
    /// Find a leaderboard by name, creating it with `create` when given and missing. Outcome:
    /// `LeaderboardFound`, `LeaderboardNotFound` or `LeaderboardIoFailure` with this `op`.
    fn find_leaderboard(&self, op: u64, name: &str, create: Option<(LeaderboardSort, LeaderboardDisplay)>) -> bool;
    /// Upload a score (at most [`MAX_LEADERBOARD_DETAILS`] details) to a found board. Outcome:
    /// `LeaderboardScoreUploaded`, `LeaderboardUploadRejected` or `LeaderboardIoFailure`.
    fn upload_score(&self, op: u64, board: u64, method: UploadMethod, score: i32, details: &[i32]) -> bool;
    /// Download entries of a found board (always with room for [`MAX_LEADERBOARD_DETAILS`]
    /// details per entry). Outcome: `LeaderboardScoresDownloaded` or `LeaderboardIoFailure`.
    fn download_scores(&self, op: u64, board: u64, range: ScoreRange) -> bool;
    /// A found board's sort method (`None` when unknown).
    fn leaderboard_sort(&self, board: u64) -> Option<LeaderboardSort>;
    /// A found board's display type (`None` when unknown).
    fn leaderboard_display(&self, board: u64) -> Option<LeaderboardDisplay>;
    /// A found board's entry count as of the last find / upload / download (0 when unknown).
    fn leaderboard_entry_count(&self, board: u64) -> i32;
}
