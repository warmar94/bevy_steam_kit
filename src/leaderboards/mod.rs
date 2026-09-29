//! Feature `leaderboards`: find (or create) Steam leaderboards, upload scores, download entries.
//!
//! The game sends [`LeaderboardRequest`]s, each with a [`LeaderboardRequestId`] of its choice.
//! Every accepted request gets exactly one answer carrying its id: [`LeaderboardFound`],
//! [`ScoreUploaded`], [`ScoresDownloaded`] or [`LeaderboardError`]. The one exception is a
//! request sent with an id that is still pending: it is rejected with
//! [`LeaderboardErrorKind::DuplicateId`] (carrying that same id) and the pending request still
//! gets its own answer, so the id then sees two messages. Keep ids unique while pending. Boards are
//! addressed by NAME; the kit finds each board once and caches its handle
//! ([`SteamLeaderboards`]).
//!
//! Configured with [`SteamKitPlugin::with_leaderboards`](crate::SteamKitPlugin::with_leaderboards)
//! ([`LeaderboardSettings`]). Inert until a [`SteamBackendRes`] whose backend supports
//! leaderboards exists; until then requests are answered with [`LeaderboardErrorKind::NoBackend`].
//! When the backend is removed, every request still waiting is answered with `NoBackend`; on
//! `AppExit`, with [`LeaderboardErrorKind::Exiting`] (write `AppExit` before `Last`).
//! Independent of the `stats` feature: leaderboards need no stats readiness and no store.
//!
//! - **Uploads** run one at a time (Valve: "one outstanding call at a time") and at most
//!   [`LeaderboardSettings::uploads_per_window`] per [`LeaderboardSettings::upload_window`]
//!   (Valve: 10 per 10 minutes); the rest wait in a bounded FIFO.
//! - **Downloads** run at once, several at a time.
//! - Every Steam call is given up after [`LeaderboardSettings::timeout`]
//!   ([`LeaderboardErrorKind::TimedOut`]); a late answer is dropped.
//! - Failures are reported as Steam reports them: steamworks 0.12.2 gives `IOFailure` or, for an
//!   upload, "not successful" without a reason ([`LeaderboardErrorKind::UploadRejected`]).
//! - Not supported (not wrapped by steamworks 0.12.2): downloading entries of chosen users, and
//!   attaching user-generated content to an entry.
//!
//! Schedules: pumped results are applied in [`SteamKitSystems::Callbacks`] (`First`); requests,
//! timeouts and the upload queue in [`SteamKitSystems::Requests`] (`Update`); the exit answers in
//! [`SteamKitSystems::Requests`] (`Last`). Timing uses `Time<Real>` (Bevy's `TimePlugin`); without
//! it there are no timeouts and no upload window (still one upload at a time), and calls started
//! before a clock appears are timed from the moment it does.

mod backend;
pub(crate) mod fake;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use crate::names::{is_valid_leaderboard_name, MAX_LEADERBOARD_NAME_BYTES};
pub use backend::{LeaderboardBackend, LeaderboardDisplay, LeaderboardEntry, LeaderboardSort, ScoreRange, UploadMethod, MAX_LEADERBOARD_DETAILS};
pub use fake::FakeLeaderboardFailure;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use bevy_app::{App, AppExit, First, Last, Update};
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_time::{Real, Time};
use tracing::{info, warn};

use crate::{BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

/// The highest usable row count: Steam takes ranks and window sizes as a C `int`.
const MAX_ROWS_LIMIT: u32 = i32::MAX as u32;

// ---------------------------------------------------------------------------------------------
// Settings + state
// ---------------------------------------------------------------------------------------------

/// The leaderboards feature's settings, given with
/// [`SteamKitPlugin::with_leaderboards`](crate::SteamKitPlugin::with_leaderboards) and also
/// inserted as a resource by the plugin (read-only). Build it with `..Default::default()`.
#[derive(Resource, Clone, Debug)]
pub struct LeaderboardSettings {
    /// A Steam call without an answer after this long is given up. Default 30 s.
    pub timeout: Duration,
    /// At most this many uploads start per [`upload_window`](Self::upload_window). Default 10.
    pub uploads_per_window: u32,
    /// The upload rate window. Default 600 s (Valve: 10 uploads per 10 minutes).
    pub upload_window: Duration,
    /// Uploads waiting their turn; more are refused with [`LeaderboardErrorKind::QueueFull`].
    /// Default 64.
    pub max_queued_uploads: usize,
    /// The most entries one download may ask for (capped at `i32::MAX`). Default 500.
    pub max_download_rows: u32,
}

impl Default for LeaderboardSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            uploads_per_window: 10,
            upload_window: Duration::from_secs(600),
            max_queued_uploads: 64,
            max_download_rows: 500,
        }
    }
}

/// The game's id for one request; its answer carries the same id.
///
/// The recommended way to get one is [`SteamLeaderboards::next_id`]: kit-issued ids are unique
/// among pending requests. Ids may also be chosen by hand; keep them unique while pending (an id
/// may be reused once its answer arrived). To MIX both, keep hand-picked ids at or above
/// [`LeaderboardRequestId::FIRST_MANUAL`]: the kit never issues those, so the two can never
/// collide (a hand-picked id below it may collide with a kit id that is written but not yet
/// handled, which is then rejected as `DuplicateId`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LeaderboardRequestId(pub u64);

impl LeaderboardRequestId {
    /// The first id the kit never issues (`2^63`): hand-picked ids from here up never collide with
    /// kit-issued ones.
    pub const FIRST_MANUAL: u64 = 1 << 63;
}

/// A found leaderboard. `#[non_exhaustive]`: read it, the kit builds it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LeaderboardInfo {
    /// The name it was found by.
    pub name: String,
    /// Steam's raw handle (valid for this game session only).
    pub handle: u64,
    /// Sort method (`None` when Steam did not say).
    pub sort: Option<LeaderboardSort>,
    /// Display type (`None` when Steam did not say).
    pub display: Option<LeaderboardDisplay>,
    /// Entry count as of the last find / upload / download.
    pub entry_count: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct UploadJob {
    id: LeaderboardRequestId,
    board: String,
    score: i32,
    details: Vec<i32>,
    method: UploadMethod,
}

#[derive(Clone, Debug, PartialEq)]
struct DownloadJob {
    id: LeaderboardRequestId,
    board: String,
    range: ScoreRange,
    max_details: usize,
}

/// Work that needs a found board.
#[derive(Clone, Debug, PartialEq)]
enum Job {
    Upload(UploadJob),
    Download(DownloadJob),
}

impl Job {
    fn id(&self) -> LeaderboardRequestId {
        match self {
            Job::Upload(j) => j.id,
            Job::Download(j) => j.id,
        }
    }
    fn board(&self) -> &str {
        match self {
            Job::Upload(j) => &j.board,
            Job::Download(j) => &j.board,
        }
    }
}

/// A call running at Steam. `started` is `None` while no clock exists (set when one appears).
#[derive(Debug)]
enum Op {
    Find { name: String, create: bool, waiters: Vec<LeaderboardRequestId>, jobs: Vec<Job>, started: Option<Duration> },
    Upload { job: UploadJob, started: Option<Duration> },
    Download { job: DownloadJob, started: Option<Duration> },
}

impl Op {
    fn started(&self) -> Option<Duration> {
        match self {
            Op::Find { started, .. } | Op::Upload { started, .. } | Op::Download { started, .. } => *started,
        }
    }
    fn started_mut(&mut self) -> &mut Option<Duration> {
        match self {
            Op::Find { started, .. } | Op::Upload { started, .. } | Op::Download { started, .. } => started,
        }
    }
    /// Every (id, board) waiting on this call.
    fn waiting(self) -> Vec<(LeaderboardRequestId, String)> {
        match self {
            Op::Find { name, waiters, jobs, .. } => waiters.into_iter().chain(jobs.iter().map(Job::id)).map(|id| (id, name.clone())).collect(),
            Op::Upload { job, .. } => vec![(job.id, job.board)],
            Op::Download { job, .. } => vec![(job.id, job.board)],
        }
    }
}

/// The leaderboards feature's state. Written only by the kit; read it through its methods.
#[derive(Resource, Debug, Default)]
pub struct SteamLeaderboards {
    boards: HashMap<String, LeaderboardInfo>,
    next_op: u64,
    ops: HashMap<u64, Op>,
    /// Jobs whose board was found in `First`, dispatched in `Update`.
    ready_jobs: Vec<Job>,
    upload_queue: VecDeque<UploadJob>,
    upload_in_flight: Option<u64>,
    upload_starts: VecDeque<Duration>,
    pending: HashSet<LeaderboardRequestId>,
    /// The last id issued by `next_id`.
    last_issued: u64,
    /// ONE read position in the request stream, shared by the `Update` and the `Last` system.
    cursor: MessageCursor<LeaderboardRequest>,
}

impl SteamLeaderboards {
    /// A board found earlier in this session, by name.
    pub fn board(&self, name: &str) -> Option<&LeaderboardInfo> {
        self.boards.get(name)
    }
    /// A request with this id is waiting for its answer.
    pub fn is_pending(&self, id: LeaderboardRequestId) -> bool {
        self.pending.contains(&id)
    }
    /// Uploads waiting their turn (not counting the one in flight).
    pub fn uploads_queued(&self) -> usize {
        self.upload_queue.len()
    }
    /// An upload was started and its answer has not arrived yet.
    pub fn upload_in_flight(&self) -> bool {
        self.upload_in_flight.is_some()
    }

    /// A fresh request id: increasing, in `1..FIRST_MANUAL` (wrapping back to 1 after the top),
    /// and never an id that is pending right now. Take `ResMut<SteamLeaderboards>` in a system
    /// ordered `.before(SteamKitSystems::Requests)`, call this, and write the request.
    pub fn next_id(&mut self) -> LeaderboardRequestId {
        loop {
            self.last_issued = if self.last_issued + 1 >= LeaderboardRequestId::FIRST_MANUAL { 1 } else { self.last_issued + 1 };
            let id = LeaderboardRequestId(self.last_issued);
            if !self.pending.contains(&id) {
                return id;
            }
        }
    }

    fn op(&mut self) -> u64 {
        self.next_op = self.next_op.wrapping_add(1);
        self.next_op
    }

    fn has_work(&self) -> bool {
        !self.ops.is_empty() || !self.ready_jobs.is_empty() || !self.upload_queue.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------------

/// A leaderboard request. `#[non_exhaustive]`: later versions may add kinds (the variants are
/// built as usual).
#[derive(Message, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LeaderboardRequest {
    /// Find an existing board. Answer: [`LeaderboardFound`] or a `NotFound` error.
    Find {
        /// Request id.
        id: LeaderboardRequestId,
        /// Board name.
        name: String,
    },
    /// Find a board, creating it (with this sort and display) when missing. A board created this
    /// way is not shown on the Steam Community site until it is configured on the partner site.
    FindOrCreate {
        /// Request id.
        id: LeaderboardRequestId,
        /// Board name.
        name: String,
        /// Sort method for a new board.
        sort: LeaderboardSort,
        /// Display type for a new board.
        display: LeaderboardDisplay,
    },
    /// Upload the local player's score (the board is found first if needed; never created).
    UploadScore {
        /// Request id.
        id: LeaderboardRequestId,
        /// Board name.
        board: String,
        /// Score.
        score: i32,
        /// Up to [`MAX_LEADERBOARD_DETAILS`] extra values stored with the score.
        details: Vec<i32>,
        /// Keep the best or always replace.
        method: UploadMethod,
    },
    /// Download entries (the board is found first if needed; never created).
    DownloadScores {
        /// Request id.
        id: LeaderboardRequestId,
        /// Board name.
        board: String,
        /// Which entries.
        range: ScoreRange,
        /// How many details to keep per entry (at most [`MAX_LEADERBOARD_DETAILS`]).
        max_details: usize,
    },
}

impl LeaderboardRequest {
    /// [`LeaderboardRequest::Find`].
    pub fn find(id: LeaderboardRequestId, name: impl Into<String>) -> Self {
        Self::Find { id, name: name.into() }
    }
    /// [`LeaderboardRequest::FindOrCreate`].
    pub fn find_or_create(id: LeaderboardRequestId, name: impl Into<String>, sort: LeaderboardSort, display: LeaderboardDisplay) -> Self {
        Self::FindOrCreate { id, name: name.into(), sort, display }
    }
    /// [`LeaderboardRequest::UploadScore`] without details.
    pub fn upload(id: LeaderboardRequestId, board: impl Into<String>, score: i32, method: UploadMethod) -> Self {
        Self::UploadScore { id, board: board.into(), score, details: Vec::new(), method }
    }
    /// [`LeaderboardRequest::DownloadScores`] without details.
    pub fn download(id: LeaderboardRequestId, board: impl Into<String>, range: ScoreRange) -> Self {
        Self::DownloadScores { id, board: board.into(), range, max_details: 0 }
    }
    /// This request's id.
    pub fn id(&self) -> LeaderboardRequestId {
        match self {
            Self::Find { id, .. } | Self::FindOrCreate { id, .. } | Self::UploadScore { id, .. } | Self::DownloadScores { id, .. } => *id,
        }
    }
    /// The board this request names.
    pub fn board(&self) -> &str {
        match self {
            Self::Find { name, .. } | Self::FindOrCreate { name, .. } => name,
            Self::UploadScore { board, .. } | Self::DownloadScores { board, .. } => board,
        }
    }
}

/// A `Find` / `FindOrCreate` answer. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LeaderboardFound {
    /// The request's id.
    pub id: LeaderboardRequestId,
    /// The board.
    pub info: LeaderboardInfo,
}

/// An `UploadScore` answer. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScoreUploaded {
    /// The request's id.
    pub id: LeaderboardRequestId,
    /// Board name.
    pub board: String,
    /// The score that was uploaded (not necessarily the one kept).
    pub score: i32,
    /// The player's entry changed (always with `ForceUpdate`; with `KeepBest` only when better).
    pub changed: bool,
    /// The player's global rank now.
    pub rank_new: i32,
    /// The player's global rank before (0: no entry before).
    pub rank_previous: i32,
}

/// A `DownloadScores` answer. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScoresDownloaded {
    /// The request's id.
    pub id: LeaderboardRequestId,
    /// Board name.
    pub board: String,
    /// The requested range.
    pub range: ScoreRange,
    /// The entries, best first (details cut to the requested count).
    pub entries: Vec<LeaderboardEntry>,
    /// The board's total entry count.
    pub entry_count: i32,
}

/// What went wrong in a [`LeaderboardError`]. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LeaderboardErrorKind {
    /// No [`SteamBackendRes`] (or its backend does not support leaderboards), or it was removed
    /// while this request waited.
    NoBackend,
    /// Empty, longer than [`MAX_LEADERBOARD_NAME_BYTES`], or containing a NUL byte.
    InvalidName,
    /// More than [`MAX_LEADERBOARD_DETAILS`] details.
    TooManyDetails,
    /// A download range that is empty, starts below rank 1, goes past `i32::MAX`, or asks for
    /// more than [`LeaderboardSettings::max_download_rows`] entries.
    InvalidRange,
    /// A request with this id is still pending. This error answers only the REJECTED duplicate;
    /// the pending request with the same id still gets its own answer.
    DuplicateId,
    /// No board with this name.
    NotFound,
    /// Steam did not accept the upload and gave no reason (for example a "trusted" board that
    /// only takes scores from a server; whether Steam's rate limit shows up this way is not
    /// verified).
    UploadRejected,
    /// Steam answered with `IOFailure` (the only failure steamworks 0.12.2 reports).
    IoFailure,
    /// No answer within [`LeaderboardSettings::timeout`].
    TimedOut,
    /// Too many uploads waiting ([`LeaderboardSettings::max_queued_uploads`]).
    QueueFull,
    /// The backend refused to start the call.
    Refused,
    /// The app exited while this request waited (answered in `Last` on `AppExit`).
    Exiting,
}

/// A request failed. The game decides how (or whether) to tell the user. `#[non_exhaustive]`:
/// read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LeaderboardError {
    /// The request's id.
    pub id: LeaderboardRequestId,
    /// Board name.
    pub board: String,
    /// Category.
    pub kind: LeaderboardErrorKind,
    /// Human-readable detail (ASCII).
    pub message: String,
}

// ---------------------------------------------------------------------------------------------
// Plugin wiring + systems
// ---------------------------------------------------------------------------------------------

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App, settings: &LeaderboardSettings) {
    app.insert_resource(settings.clone())
        .init_resource::<SteamLeaderboards>()
        .add_message::<LeaderboardRequest>()
        .add_message::<LeaderboardFound>()
        .add_message::<ScoreUploaded>()
        .add_message::<ScoresDownloaded>()
        .add_message::<LeaderboardError>()
        .add_systems(First, apply_leaderboard_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_leaderboard_requests.in_set(SteamKitSystems::Requests))
        .add_systems(Last, answer_on_exit.in_set(SteamKitSystems::Requests));
}

#[derive(SystemParam)]
struct BoardsOut<'w> {
    found: MessageWriter<'w, LeaderboardFound>,
    uploaded: MessageWriter<'w, ScoreUploaded>,
    downloaded: MessageWriter<'w, ScoresDownloaded>,
    error: MessageWriter<'w, LeaderboardError>,
}

impl BoardsOut<'_> {
    fn fail(&mut self, state: &mut SteamLeaderboards, id: LeaderboardRequestId, board: &str, kind: LeaderboardErrorKind, message: impl Into<String>) {
        let message = message.into();
        state.pending.remove(&id);
        warn!(">>> STEAM: leaderboard {board:?} request {}: {kind:?} ({message})", id.0);
        self.error.write(LeaderboardError { id, board: board.to_string(), kind, message });
    }
}

/// Answer every request the kit still holds (running, waiting for a board, queued) with `kind`.
fn fail_all(state: &mut SteamLeaderboards, out: &mut BoardsOut, kind: LeaderboardErrorKind, message: &str) {
    let mut waiting: Vec<(LeaderboardRequestId, String)> = Vec::new();
    for (_, op) in std::mem::take(&mut state.ops) {
        waiting.extend(op.waiting());
    }
    waiting.extend(std::mem::take(&mut state.ready_jobs).into_iter().map(|j| (j.id(), j.board().to_string())));
    waiting.extend(std::mem::take(&mut state.upload_queue).into_iter().map(|j| (j.id, j.board)));
    state.upload_in_flight = None;
    for (id, board) in waiting {
        out.fail(state, id, &board, kind, message);
    }
    state.pending.clear();
}

fn info_of(api: &dyn LeaderboardBackend, name: &str, handle: u64) -> LeaderboardInfo {
    LeaderboardInfo {
        name: name.to_string(),
        handle,
        sort: api.leaderboard_sort(handle),
        display: api.leaderboard_display(handle),
        entry_count: api.leaderboard_entry_count(handle),
    }
}

fn is_leaderboard_event(ev: &BackendEvent) -> bool {
    matches!(
        ev,
        BackendEvent::LeaderboardFound { .. }
            | BackendEvent::LeaderboardNotFound { .. }
            | BackendEvent::LeaderboardScoreUploaded { .. }
            | BackendEvent::LeaderboardUploadRejected { .. }
            | BackendEvent::LeaderboardScoresDownloaded { .. }
            | BackendEvent::LeaderboardIoFailure { .. }
    )
}

/// `First` ([`SteamKitSystems::Callbacks`]): this frame's pumped leaderboard results -> answers.
/// Never pumps; reads no clock.
fn apply_leaderboard_events(backend: Option<Res<SteamBackendRes>>, pumped: Res<PumpedEvents>, mut state: ResMut<SteamLeaderboards>, mut out: BoardsOut) {
    let Some(api) = backend.as_ref().and_then(|b| b.0.leaderboards()) else {
        let lost = pumped.0.iter().filter(|e| is_leaderboard_event(e)).count();
        if lost > 0 && backend.is_none() {
            // The requests they answer are failed with `NoBackend` in `Update`.
            warn!(">>> STEAM: backend removed before {lost} pumped leaderboard event(s) were applied - dropped");
        }
        return;
    };
    for ev in &pumped.0 {
        match ev {
            &BackendEvent::LeaderboardFound { op, board } => match state.ops.remove(&op) {
                Some(Op::Find { name, waiters, jobs, .. }) => {
                    let info = info_of(api, &name, board);
                    info!(">>> STEAM: leaderboard {name:?} found ({} entries)", info.entry_count);
                    state.boards.insert(name.clone(), info.clone());
                    for id in waiters {
                        state.pending.remove(&id);
                        out.found.write(LeaderboardFound { id, info: info.clone() });
                    }
                    state.ready_jobs.extend(jobs);
                }
                other => late(&mut state, op, other),
            },
            &BackendEvent::LeaderboardNotFound { op } => match state.ops.remove(&op) {
                Some(Op::Find { name, waiters, jobs, .. }) => {
                    for id in waiters.into_iter().chain(jobs.iter().map(Job::id)) {
                        out.fail(&mut state, id, &name, LeaderboardErrorKind::NotFound, "no leaderboard with this name");
                    }
                }
                other => late(&mut state, op, other),
            },
            &BackendEvent::LeaderboardScoreUploaded { op, score, changed, rank_new, rank_previous } => match state.ops.remove(&op) {
                Some(Op::Upload { job, .. }) => {
                    state.upload_in_flight = None;
                    state.pending.remove(&job.id);
                    if let Some(info) = state.boards.get_mut(&job.board) {
                        info.entry_count = api.leaderboard_entry_count(info.handle);
                    }
                    info!(">>> STEAM: leaderboard {:?}: score {score} uploaded (changed {changed}, rank {rank_previous} -> {rank_new})", job.board);
                    out.uploaded.write(ScoreUploaded { id: job.id, board: job.board, score, changed, rank_new, rank_previous });
                }
                other => late(&mut state, op, other),
            },
            &BackendEvent::LeaderboardUploadRejected { op } => match state.ops.remove(&op) {
                Some(Op::Upload { job, .. }) => {
                    state.upload_in_flight = None;
                    out.fail(&mut state, job.id, &job.board, LeaderboardErrorKind::UploadRejected, "Steam did not accept the upload (no reason given)");
                }
                other => late(&mut state, op, other),
            },
            BackendEvent::LeaderboardScoresDownloaded { op, entries } => match state.ops.remove(op) {
                Some(Op::Download { job, .. }) => {
                    let entries: Vec<LeaderboardEntry> =
                        entries.iter().map(|e| LeaderboardEntry { details: e.details.iter().copied().take(job.max_details).collect(), ..e.clone() }).collect();
                    let entry_count = state.boards.get(&job.board).map_or(0, |i| api.leaderboard_entry_count(i.handle));
                    if let Some(info) = state.boards.get_mut(&job.board) {
                        info.entry_count = entry_count;
                    }
                    state.pending.remove(&job.id);
                    out.downloaded.write(ScoresDownloaded { id: job.id, board: job.board, range: job.range, entries, entry_count });
                }
                other => late(&mut state, *op, other),
            },
            &BackendEvent::LeaderboardIoFailure { op } => match state.ops.remove(&op) {
                Some(o) => {
                    if matches!(o, Op::Upload { .. }) {
                        state.upload_in_flight = None;
                    }
                    for (id, board) in o.waiting() {
                        out.fail(&mut state, id, &board, LeaderboardErrorKind::IoFailure, "Steam answered IOFailure");
                    }
                }
                None => late(&mut state, op, None),
            },
            // Another feature's event.
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}

/// An answer for an operation that is not (or no longer) waiting: it timed out, or it answers a
/// different kind of call. Put a mismatched op back; drop an unknown one.
fn late(state: &mut SteamLeaderboards, op: u64, other: Option<Op>) {
    match other {
        Some(o) => {
            state.ops.insert(op, o);
        }
        None => info!(">>> STEAM: late leaderboard answer for a given-up call - dropped"),
    }
}

/// Validate a request (errors are answered at once).
fn validate(req: &LeaderboardRequest, settings: &LeaderboardSettings, state: &mut SteamLeaderboards, out: &mut BoardsOut) -> bool {
    let (id, board) = (req.id(), req.board().to_string());
    // An id still pending is refused without touching the pending one (which still answers).
    if state.pending.contains(&id) {
        warn!(">>> STEAM: leaderboard request id {} is still pending", id.0);
        out.error.write(LeaderboardError {
            id,
            board,
            kind: LeaderboardErrorKind::DuplicateId,
            message: "rejected: a request with this id is still pending (that one still gets its own answer)".into(),
        });
        return false;
    }
    if !is_valid_leaderboard_name(&board) {
        out.fail(
            state,
            id,
            &board,
            LeaderboardErrorKind::InvalidName,
            format!("invalid leaderboard name (empty, over {MAX_LEADERBOARD_NAME_BYTES} bytes, or a NUL byte)"),
        );
        return false;
    }
    match req {
        LeaderboardRequest::UploadScore { details, .. } if details.len() > MAX_LEADERBOARD_DETAILS => {
            out.fail(state, id, &board, LeaderboardErrorKind::TooManyDetails, format!("{} details (max {MAX_LEADERBOARD_DETAILS})", details.len()));
            false
        }
        LeaderboardRequest::DownloadScores { max_details, .. } if *max_details > MAX_LEADERBOARD_DETAILS => {
            out.fail(state, id, &board, LeaderboardErrorKind::TooManyDetails, format!("max_details {max_details} (max {MAX_LEADERBOARD_DETAILS})"));
            false
        }
        LeaderboardRequest::DownloadScores { range, .. } if !range_ok(*range, settings.max_download_rows) => {
            out.fail(state, id, &board, LeaderboardErrorKind::InvalidRange, format!("unusable range {range:?}"));
            false
        }
        _ => true,
    }
}

/// A download range that Steam can take: `Global` from rank 1 on, not reversed, every rank within
/// `i32::MAX` (Steam's C `int`), and every range within `max_rows` entries (itself capped at
/// `i32::MAX`, so an around-user start never wraps).
pub(crate) fn range_ok(range: ScoreRange, max_rows: u32) -> bool {
    let max = u64::from(max_rows.clamp(1, MAX_ROWS_LIMIT));
    match range {
        ScoreRange::Global { first, last } => first >= 1 && last >= first && last <= MAX_ROWS_LIMIT && u64::from(last - first) < max,
        ScoreRange::AroundUser { before, after } => u64::from(before) + u64::from(after) < max,
        ScoreRange::Friends => true,
    }
}

/// Who waits for a find: a `Find` request, or a job that needs the board.
enum FindFor {
    Request(LeaderboardRequestId),
    Job(Job),
}

/// Start (or join) a find for `name`.
fn find(
    api: &dyn LeaderboardBackend,
    now: Option<Duration>,
    state: &mut SteamLeaderboards,
    out: &mut BoardsOut,
    name: &str,
    create: Option<(LeaderboardSort, LeaderboardDisplay)>,
    who: FindFor,
) {
    let (waiter, job) = match who {
        FindFor::Request(id) => (Some(id), None),
        FindFor::Job(job) => (None, Some(job)),
    };
    let joined = state.ops.values_mut().find_map(|op| match op {
        Op::Find { name: n, create: c, waiters, jobs, .. } if n == name && *c == create.is_some() => Some((waiters, jobs)),
        _ => None,
    });
    if let Some((waiters, jobs)) = joined {
        waiters.extend(waiter);
        jobs.extend(job);
        return;
    }
    let op = state.op();
    if api.find_leaderboard(op, name, create) {
        state.ops.insert(
            op,
            Op::Find { name: name.to_string(), create: create.is_some(), waiters: waiter.into_iter().collect(), jobs: job.into_iter().collect(), started: now },
        );
    } else {
        for id in waiter.into_iter().chain(job.map(|j| j.id())) {
            out.fail(state, id, name, LeaderboardErrorKind::Refused, "the backend refused the find");
        }
    }
}

/// A job whose board is known: queue an upload, start a download.
fn dispatch(api: &dyn LeaderboardBackend, now: Option<Duration>, settings: &LeaderboardSettings, state: &mut SteamLeaderboards, out: &mut BoardsOut, job: Job) {
    let Some(handle) = state.boards.get(job.board()).map(|i| i.handle) else {
        out.fail(state, job.id(), job.board(), LeaderboardErrorKind::NotFound, "board not found");
        return;
    };
    match job {
        Job::Upload(up) => {
            if state.upload_queue.len() >= settings.max_queued_uploads.max(1) {
                out.fail(state, up.id, &up.board, LeaderboardErrorKind::QueueFull, "too many uploads waiting");
            } else {
                state.upload_queue.push_back(up);
            }
        }
        Job::Download(down) => {
            let op = state.op();
            if api.download_scores(op, handle, down.range) {
                state.ops.insert(op, Op::Download { job: down, started: now });
            } else {
                out.fail(state, down.id, &down.board, LeaderboardErrorKind::Refused, "the backend refused the download");
            }
        }
    }
}

/// `Update` ([`SteamKitSystems::Requests`]): timeouts, jobs whose board was found, new requests,
/// and the upload queue.
fn handle_leaderboard_requests(
    backend: Option<Res<SteamBackendRes>>,
    time: Option<Res<Time<Real>>>,
    settings: Res<LeaderboardSettings>,
    messages: Res<Messages<LeaderboardRequest>>,
    mut state: ResMut<SteamLeaderboards>,
    mut out: BoardsOut,
) {
    let requests: Vec<LeaderboardRequest> = state.cursor.read(&messages).cloned().collect();
    let Some(api) = backend.as_ref().and_then(|b| b.0.leaderboards()) else {
        // The backend is gone (or never came): nothing it held will ever be answered by Steam.
        if state.has_work() {
            fail_all(&mut state, &mut out, LeaderboardErrorKind::NoBackend, "Steam backend removed while this request waited");
        }
        for req in requests {
            out.error.write(LeaderboardError {
                id: req.id(),
                board: req.board().to_string(),
                kind: LeaderboardErrorKind::NoBackend,
                message: "leaderboards: Steam is not available".into(),
            });
        }
        return;
    };
    let now = time.as_ref().map(|t| t.elapsed());

    if let Some(now) = now {
        // Calls started without a clock are timed from now on.
        for op in state.ops.values_mut() {
            op.started_mut().get_or_insert(now);
        }
        // Give up on calls without an answer.
        let expired: Vec<u64> =
            state.ops.iter().filter(|(_, op)| op.started().is_some_and(|t| now.saturating_sub(t) >= settings.timeout)).map(|(k, _)| *k).collect();
        for op in expired {
            let Some(o) = state.ops.remove(&op) else { continue };
            if matches!(o, Op::Upload { .. }) {
                state.upload_in_flight = None;
            }
            for (id, board) in o.waiting() {
                out.fail(&mut state, id, &board, LeaderboardErrorKind::TimedOut, "no answer from Steam in time");
            }
        }
    }

    for job in std::mem::take(&mut state.ready_jobs) {
        dispatch(api, now, &settings, &mut state, &mut out, job);
    }

    for req in requests {
        if !validate(&req, &settings, &mut state, &mut out) {
            continue;
        }
        let id = req.id();
        state.pending.insert(id);
        match req {
            LeaderboardRequest::Find { name, .. } | LeaderboardRequest::FindOrCreate { name, .. } if state.boards.contains_key(&name) => {
                let info = state.boards[&name].clone();
                state.pending.remove(&id);
                out.found.write(LeaderboardFound { id, info });
            }
            LeaderboardRequest::Find { name, .. } => find(api, now, &mut state, &mut out, &name, None, FindFor::Request(id)),
            LeaderboardRequest::FindOrCreate { name, sort, display, .. } => {
                find(api, now, &mut state, &mut out, &name, Some((sort, display)), FindFor::Request(id))
            }
            LeaderboardRequest::UploadScore { board, score, details, method, .. } => {
                let job = Job::Upload(UploadJob { id, board: board.clone(), score, details, method });
                if state.boards.contains_key(&board) {
                    dispatch(api, now, &settings, &mut state, &mut out, job);
                } else {
                    find(api, now, &mut state, &mut out, &board, None, FindFor::Job(job));
                }
            }
            LeaderboardRequest::DownloadScores { board, range, max_details, .. } => {
                let job = Job::Download(DownloadJob { id, board: board.clone(), range, max_details });
                if state.boards.contains_key(&board) {
                    dispatch(api, now, &settings, &mut state, &mut out, job);
                } else {
                    find(api, now, &mut state, &mut out, &board, None, FindFor::Job(job));
                }
            }
        }
    }

    // The upload queue: one in flight, at most N starts per window.
    while state.upload_in_flight.is_none() {
        if let Some(now) = now {
            while state.upload_starts.front().is_some_and(|t| now.saturating_sub(*t) >= settings.upload_window) {
                state.upload_starts.pop_front();
            }
            if state.upload_starts.len() >= settings.uploads_per_window.max(1) as usize {
                break;
            }
        }
        let Some(job) = state.upload_queue.pop_front() else { break };
        let Some(handle) = state.boards.get(&job.board).map(|i| i.handle) else {
            out.fail(&mut state, job.id, &job.board, LeaderboardErrorKind::NotFound, "board not found");
            continue;
        };
        let op = state.op();
        if api.upload_score(op, handle, job.method, job.score, &job.details) {
            state.upload_in_flight = Some(op);
            if let Some(now) = now {
                state.upload_starts.push_back(now);
            }
            state.ops.insert(op, Op::Upload { job, started: now });
        } else {
            out.fail(&mut state, job.id, &job.board, LeaderboardErrorKind::Refused, "the backend refused the upload");
        }
    }
}

/// `Last` ([`SteamKitSystems::Requests`]): on `AppExit`, answer every request still waiting
/// (running, waiting for a board, queued, or written this frame after the `Update` set) with
/// [`LeaderboardErrorKind::Exiting`].
fn answer_on_exit(
    mut exit: MessageReader<AppExit>,
    backend: Option<Res<SteamBackendRes>>,
    messages: Res<Messages<LeaderboardRequest>>,
    mut state: ResMut<SteamLeaderboards>,
    mut out: BoardsOut,
) {
    if exit.read().count() == 0 {
        return;
    }
    let late: Vec<LeaderboardRequest> = state.cursor.read(&messages).cloned().collect();
    let has_backend = backend.as_ref().and_then(|b| b.0.leaderboards()).is_some();
    if state.has_work() {
        warn!(">>> STEAM: exiting with leaderboard requests still waiting - answered Exiting");
        fail_all(&mut state, &mut out, LeaderboardErrorKind::Exiting, "the app exited before Steam answered");
    }
    for req in late {
        let kind = if has_backend { LeaderboardErrorKind::Exiting } else { LeaderboardErrorKind::NoBackend };
        out.error.write(LeaderboardError { id: req.id(), board: req.board().to_string(), kind, message: "the app exited before this request was sent".into() });
    }
}
