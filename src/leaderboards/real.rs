//! The leaderboard half of [`RealSteamBackend`] (features `steam` + `leaderboards`), over
//! `steamworks` 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source (`user_stats.rs`):
//! - `find_leaderboard` (:15-40) / `find_or_create_leaderboard` (:42-97) build
//!   `CString::new(name).unwrap()` (:20, :51): a NUL byte would PANIC. Names are validated first
//!   (non-empty, at most 127 bytes - `k_cchLeaderboardNameMax` is 128, possibly counting the NUL -
//!   and no NUL).
//! - Results arrive as call-result closures run INSIDE the kit's one `process_callbacks` while
//!   steamworks holds its `call_results` lock (lib.rs:176-181): the closures here only push to
//!   the backend's queue and record a found board in `boards` (our own mutex); they never call
//!   Steam (a nested async call would re-lock and deadlock).
//! - `Leaderboard` (:530-542) has no public constructor: found boards are kept here by `raw()`.
//! - `upload_leaderboard_score` (:99-147) passes `details.len() as _`; more than 64 details makes
//!   Steam fail the upload: refused before. `Ok(None)` (`m_bSuccess == 0`) carries no reason.
//! - `download_leaderboard_entries` (:149-215) allocates `max_details_len` per entry and then
//!   `set_len(m_cDetails)`: a `max_details_len` below an entry's detail count is undefined
//!   behaviour. The kit ALWAYS passes 64 (`k_cLeaderboardDetailsMax`, the most an entry can hold)
//!   and truncates afterwards.
//! - `start` / `end` are `usize` cast with `as _` to the C `int` (:176-177): the around-user
//!   window needs a NEGATIVE start, passed as `(-(before as i64)) as isize as usize` (sign-extended;
//!   the `as _` truncation gives back `-before`) - see [`around_user_start`].
//! - The only failure value is `SteamError::IOFailure` (error.rs:130); there is no `EResult`.
//! - Not wrapped by 0.12.2 and NOT supported: `DownloadLeaderboardEntriesForUsers` and
//!   `AttachLeaderboardUGC` (their results cannot be received: `register_call_result` is
//!   crate-private, callback.rs:193).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use steamworks::{Leaderboard, LeaderboardDataRequest, LeaderboardDisplayType, LeaderboardSortMethod, UploadScoreMethod};

use super::backend::{LeaderboardBackend, LeaderboardDisplay, LeaderboardEntry, LeaderboardSort, ScoreRange, UploadMethod, MAX_LEADERBOARD_DETAILS};
use crate::backend::BackendEvent;
use crate::names::is_valid_leaderboard_name;
use crate::real::{push, Queue, RealSteamBackend};

/// Found boards by raw handle.
#[derive(Clone, Default)]
pub(crate) struct Boards(Arc<Mutex<HashMap<u64, Leaderboard>>>);

impl Boards {
    fn get(&self, handle: u64) -> Option<Leaderboard> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).get(&handle).cloned()
    }
    fn insert(&self, board: Leaderboard) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).insert(board.raw(), board);
    }
}

/// The `start` argument for an around-user download of `before` entries above the player: the
/// negative number `-before`, carried through steamworks' `usize` parameter. On every target the
/// `usize` -> C `int` cast (`as _`) truncates to the low 32 bits, which hold `-before`.
pub(crate) fn around_user_start(before: u32) -> usize {
    (-(before as i64)) as isize as usize
}

/// The `(start, end)` arguments for a range (already validated by the kit).
pub(crate) fn range_args(range: ScoreRange) -> (LeaderboardDataRequest, usize, usize) {
    match range {
        ScoreRange::Global { first, last } => (LeaderboardDataRequest::Global, first as usize, last as usize),
        ScoreRange::AroundUser { before, after } => (LeaderboardDataRequest::GlobalAroundUser, around_user_start(before), after as usize),
        // Steam ignores the range for friends.
        ScoreRange::Friends => (LeaderboardDataRequest::Friends, 1, 1),
    }
}

fn find_closure(queue: Queue, boards: Boards, op: u64) -> impl FnOnce(Result<Option<Leaderboard>, steamworks::SteamError>) + Send + 'static {
    move |res| {
        let ev = match res {
            Ok(Some(board)) => {
                let handle = board.raw();
                boards.insert(board);
                BackendEvent::LeaderboardFound { op, board: handle }
            }
            Ok(None) => BackendEvent::LeaderboardNotFound { op },
            Err(_) => BackendEvent::LeaderboardIoFailure { op },
        };
        push(&queue, ev);
    }
}

impl LeaderboardBackend for RealSteamBackend {
    fn find_leaderboard(&self, op: u64, name: &str, create: Option<(LeaderboardSort, LeaderboardDisplay)>) -> bool {
        if !is_valid_leaderboard_name(name) {
            return false;
        }
        let cb = find_closure(self.queue.clone(), self.boards.clone(), op);
        let us = self.client.user_stats();
        match create {
            None => us.find_leaderboard(name, cb),
            Some((sort, display)) => {
                let sort = match sort {
                    LeaderboardSort::Ascending => LeaderboardSortMethod::Ascending,
                    LeaderboardSort::Descending => LeaderboardSortMethod::Descending,
                };
                let display = match display {
                    LeaderboardDisplay::Numeric => LeaderboardDisplayType::Numeric,
                    LeaderboardDisplay::TimeSeconds => LeaderboardDisplayType::TimeSeconds,
                    LeaderboardDisplay::TimeMilliseconds => LeaderboardDisplayType::TimeMilliSeconds,
                };
                us.find_or_create_leaderboard(name, sort, display, cb)
            }
        }
        true
    }

    fn upload_score(&self, op: u64, board: u64, method: UploadMethod, score: i32, details: &[i32]) -> bool {
        if details.len() > MAX_LEADERBOARD_DETAILS {
            return false;
        }
        let Some(lb) = self.boards.get(board) else { return false };
        let method = match method {
            UploadMethod::KeepBest => UploadScoreMethod::KeepBest,
            UploadMethod::ForceUpdate => UploadScoreMethod::ForceUpdate,
        };
        let queue = self.queue.clone();
        self.client.user_stats().upload_leaderboard_score(&lb, method, score, details, move |res| {
            let ev = match res {
                Ok(Some(u)) => BackendEvent::LeaderboardScoreUploaded {
                    op,
                    score: u.score,
                    changed: u.was_changed,
                    rank_new: u.global_rank_new,
                    rank_previous: u.global_rank_previous,
                },
                Ok(None) => BackendEvent::LeaderboardUploadRejected { op },
                Err(_) => BackendEvent::LeaderboardIoFailure { op },
            };
            push(&queue, ev);
        });
        true
    }

    fn download_scores(&self, op: u64, board: u64, range: ScoreRange) -> bool {
        let Some(lb) = self.boards.get(board) else { return false };
        let (request, start, end) = range_args(range);
        let queue = self.queue.clone();
        // ALWAYS the maximum: steamworks' `set_len(m_cDetails)` is only sound when this is at
        // least the entry's detail count, and no entry holds more than 64.
        self.client.user_stats().download_leaderboard_entries(&lb, request, start, end, MAX_LEADERBOARD_DETAILS, move |res| {
            let ev = match res {
                Ok(entries) => BackendEvent::LeaderboardScoresDownloaded {
                    op,
                    entries: entries
                        .into_iter()
                        .map(|e| LeaderboardEntry { rank: e.global_rank, steam_id: e.user.raw(), score: e.score, details: e.details })
                        .collect(),
                },
                Err(_) => BackendEvent::LeaderboardIoFailure { op },
            };
            push(&queue, ev);
        });
        true
    }

    fn leaderboard_sort(&self, board: u64) -> Option<LeaderboardSort> {
        let lb = self.boards.get(board)?;
        match self.client.user_stats().get_leaderboard_sort_method(&lb)? {
            LeaderboardSortMethod::Ascending => Some(LeaderboardSort::Ascending),
            LeaderboardSortMethod::Descending => Some(LeaderboardSort::Descending),
        }
    }

    fn leaderboard_display(&self, board: u64) -> Option<LeaderboardDisplay> {
        let lb = self.boards.get(board)?;
        match self.client.user_stats().get_leaderboard_display_type(&lb)? {
            LeaderboardDisplayType::Numeric => Some(LeaderboardDisplay::Numeric),
            LeaderboardDisplayType::TimeSeconds => Some(LeaderboardDisplay::TimeSeconds),
            LeaderboardDisplayType::TimeMilliSeconds => Some(LeaderboardDisplay::TimeMilliseconds),
        }
    }

    fn leaderboard_entry_count(&self, board: u64) -> i32 {
        self.boards.get(board).map_or(0, |lb| self.client.user_stats().get_leaderboard_entry_count(&lb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What steamworks does with the `start` argument: `start as _` into a C `int`.
    fn as_c_int(v: usize) -> i32 {
        v as i32
    }

    #[test]
    fn the_around_user_start_reaches_steam_as_a_negative_number() {
        for before in [0u32, 1, 3, 10, 500, i32::MAX as u32] {
            assert_eq!(as_c_int(around_user_start(before)) as i64, -(before as i64));
        }
        assert_eq!(range_args(ScoreRange::AroundUser { before: 3, after: 4 }).2, 4);
        assert_eq!(range_args(ScoreRange::Global { first: 1, last: 10 }).1, 1);
    }
}
