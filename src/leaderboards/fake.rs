//! The leaderboard half of [`FakeSteamBackend`] (feature `leaderboards`): in-memory boards.
//!
//! Simplifications: ties rank by earlier upload, and an around-user window is clipped at the
//! board's edges; neither is verified against real Steam.

use std::collections::{HashMap, HashSet};

use super::backend::{LeaderboardBackend, LeaderboardDisplay, LeaderboardEntry, LeaderboardSort, ScoreRange, UploadMethod, MAX_LEADERBOARD_DETAILS};
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};

/// How the fake's next leaderboard call goes wrong (see
/// [`FakeSteamBackend::fail_next_leaderboard_call`]). `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FakeLeaderboardFailure {
    /// The call completes with `IOFailure`.
    IoFailure,
    /// An upload completes as refused by Steam (`m_bSuccess == 0`); other calls: `IOFailure`.
    Rejected,
    /// The call is started but its outcome never arrives.
    NoAnswer,
}

#[derive(Debug)]
struct FakeBoard {
    name: String,
    sort: LeaderboardSort,
    display: LeaderboardDisplay,
    /// (player, score, details, upload order)
    entries: Vec<(u64, i32, Vec<i32>, u64)>,
}

impl FakeBoard {
    /// Entries best first; ties by earlier upload.
    fn ranked(&self) -> Vec<LeaderboardEntry> {
        let mut e = self.entries.clone();
        e.sort_by(|a, b| {
            let by_score = match self.sort {
                LeaderboardSort::Ascending => a.1.cmp(&b.1),
                LeaderboardSort::Descending => b.1.cmp(&a.1),
            };
            by_score.then(a.3.cmp(&b.3))
        });
        e.into_iter().enumerate().map(|(i, (id, score, details, _))| LeaderboardEntry { rank: i as i32 + 1, steam_id: id, score, details }).collect()
    }

    fn rank_of(&self, id: u64) -> i32 {
        self.ranked().iter().find(|e| e.steam_id == id).map_or(0, |e| e.rank)
    }

    fn better(&self, new: i32, old: i32) -> bool {
        match self.sort {
            LeaderboardSort::Ascending => new < old,
            LeaderboardSort::Descending => new > old,
        }
    }
}

/// The fake's leaderboard state, stored inside the shared fake state.
#[derive(Debug, Default)]
pub(crate) struct FakeBoardsState {
    boards: HashMap<u64, FakeBoard>,
    next_handle: u64,
    next_seq: u64,
    friends: HashSet<u64>,
    next_failure: Option<FakeLeaderboardFailure>,
}

impl FakeBoardsState {
    fn handle_of(&self, name: &str) -> Option<u64> {
        self.boards.iter().find(|(_, b)| b.name == name).map(|(h, _)| *h)
    }
    fn create(&mut self, name: &str, sort: LeaderboardSort, display: LeaderboardDisplay) -> u64 {
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        let h = 0x0100_0000 + self.next_handle;
        self.boards.insert(h, FakeBoard { name: name.to_string(), sort, display, entries: Vec::new() });
        h
    }
}

/// Leaderboard controls (feature `leaderboards`).
impl FakeSteamBackend {
    /// Create a board, as the partner site would.
    pub fn add_leaderboard(&self, name: &str, sort: LeaderboardSort, display: LeaderboardDisplay) {
        let mut s = self.lock();
        if s.boards.handle_of(name).is_none() {
            s.boards.create(name, sort, display);
        }
    }

    /// Put a player's entry on a board (replacing theirs). Details beyond 64 are dropped.
    pub fn add_leaderboard_entry(&self, board: &str, steam_id: u64, score: i32, details: &[i32]) {
        let mut guard = self.lock();
        let s = &mut guard.boards;
        let Some(h) = s.handle_of(board) else { return };
        s.next_seq += 1;
        let seq = s.next_seq;
        if let Some(b) = s.boards.get_mut(&h) {
            b.entries.retain(|e| e.0 != steam_id);
            b.entries.push((steam_id, score, details.iter().copied().take(MAX_LEADERBOARD_DETAILS).collect(), seq));
        }
    }

    /// The local player's Steam friends (for [`ScoreRange::Friends`]).
    pub fn set_friends(&self, friends: &[u64]) {
        self.lock().boards.friends = friends.iter().copied().collect();
    }

    /// Make the next leaderboard call fail in the given way.
    pub fn fail_next_leaderboard_call(&self, failure: FakeLeaderboardFailure) {
        self.lock().boards.next_failure = Some(failure);
    }

    /// A board's entries, best first (empty for an unknown board).
    pub fn leaderboard_entries(&self, board: &str) -> Vec<LeaderboardEntry> {
        let s = self.lock();
        s.boards.handle_of(board).and_then(|h| s.boards.boards.get(&h)).map(|b| b.ranked()).unwrap_or_default()
    }
}

impl LeaderboardBackend for FakeSteamBackend {
    fn find_leaderboard(&self, op: u64, name: &str, create: Option<(LeaderboardSort, LeaderboardDisplay)>) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::FindLeaderboard { name: name.to_string(), create: create.is_some() });
        let ev = match s.boards.next_failure.take() {
            Some(FakeLeaderboardFailure::NoAnswer) => return true,
            Some(_) => BackendEvent::LeaderboardIoFailure { op },
            None => match (s.boards.handle_of(name), create) {
                (Some(h), _) => BackendEvent::LeaderboardFound { op, board: h },
                (None, Some((sort, display))) => BackendEvent::LeaderboardFound { op, board: s.boards.create(name, sort, display) },
                (None, None) => BackendEvent::LeaderboardNotFound { op },
            },
        };
        s.queued.push(ev);
        true
    }

    fn upload_score(&self, op: u64, board: u64, method: UploadMethod, score: i32, details: &[i32]) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::UploadScore { board, method, score, details: details.to_vec() });
        if details.len() > MAX_LEADERBOARD_DETAILS || !s.boards.boards.contains_key(&board) {
            return false;
        }
        let ev = match s.boards.next_failure.take() {
            Some(FakeLeaderboardFailure::NoAnswer) => return true,
            Some(FakeLeaderboardFailure::Rejected) => BackendEvent::LeaderboardUploadRejected { op },
            Some(_) => BackendEvent::LeaderboardIoFailure { op },
            None => {
                let me = s.local_id;
                s.boards.next_seq += 1;
                let seq = s.boards.next_seq;
                let Some(b) = s.boards.boards.get_mut(&board) else { return false };
                let previous = b.rank_of(me);
                let old = b.entries.iter().find(|e| e.0 == me).map(|e| e.1);
                let changed = match (method, old) {
                    (_, None) | (UploadMethod::ForceUpdate, _) => true,
                    (UploadMethod::KeepBest, Some(old)) => b.better(score, old),
                };
                if changed {
                    b.entries.retain(|e| e.0 != me);
                    b.entries.push((me, score, details.to_vec(), seq));
                }
                BackendEvent::LeaderboardScoreUploaded { op, score, changed, rank_new: b.rank_of(me), rank_previous: previous }
            }
        };
        s.queued.push(ev);
        true
    }

    fn download_scores(&self, op: u64, board: u64, range: ScoreRange) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::DownloadScores { board, range });
        if !s.boards.boards.contains_key(&board) {
            return false;
        }
        let ev = match s.boards.next_failure.take() {
            Some(FakeLeaderboardFailure::NoAnswer) => return true,
            Some(_) => BackendEvent::LeaderboardIoFailure { op },
            None => {
                let Some(b) = s.boards.boards.get(&board) else { return false };
                let ranked = b.ranked();
                let me = s.local_id;
                let entries: Vec<LeaderboardEntry> = match range {
                    ScoreRange::Global { first, last } => ranked.into_iter().filter(|e| e.rank >= first as i32 && e.rank <= last as i32).collect(),
                    // A fake simplification: the window is clipped at the board's edges. What real
                    // Steam returns near the top or bottom of a board is not verified here.
                    ScoreRange::AroundUser { before, after } => match ranked.iter().position(|e| e.steam_id == me) {
                        Some(i) => {
                            let lo = i.saturating_sub(before as usize);
                            let hi = i.saturating_add(after as usize).min(ranked.len().saturating_sub(1));
                            ranked[lo..=hi].to_vec()
                        }
                        None => Vec::new(),
                    },
                    ScoreRange::Friends => ranked.into_iter().filter(|e| e.steam_id == me || s.boards.friends.contains(&e.steam_id)).collect(),
                };
                BackendEvent::LeaderboardScoresDownloaded { op, entries }
            }
        };
        s.queued.push(ev);
        true
    }

    fn leaderboard_sort(&self, board: u64) -> Option<LeaderboardSort> {
        self.lock().boards.boards.get(&board).map(|b| b.sort)
    }

    fn leaderboard_display(&self, board: u64) -> Option<LeaderboardDisplay> {
        self.lock().boards.boards.get(&board).map(|b| b.display)
    }

    fn leaderboard_entry_count(&self, board: u64) -> i32 {
        self.lock().boards.boards.get(&board).map_or(0, |b| b.entries.len() as i32)
    }
}
