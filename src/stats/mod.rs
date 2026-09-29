//! Feature `stats`: Steam user stats and achievements.
//!
//! The game sends [`StatsRequest`]s (set / add to a stat, unlock / clear an achievement, show
//! progress, store, reset) as ONE ordered stream, and reacts to facts ([`StatsReady`],
//! [`StatsStored`], [`AchievementUnlocked`], [`AchievementProgress`], [`StatsError`]). Current
//! values are read synchronously through the backend (`backend.0.stats()`, see [`StatsBackend`]);
//! [`SteamStats`] tells whether stats are ready and whether changes are waiting to be stored.
//!
//! Configured with [`SteamKitPlugin::with_stats`](crate::SteamKitPlugin::with_stats)
//! ([`StatsSettings`]). Inert until a [`SteamBackendRes`] whose backend supports stats
//! ([`SteamBackend::stats`](crate::SteamBackend::stats) returns `Some`) exists; until then
//! writes are answered with [`StatsErrorKind::NoBackend`].
//!
//! - **Order.** Requests are applied in the order they were written, whatever their kind
//!   (`AddStat` then `SetStat` ends with the set value; `UnlockAchievement` then `ResetAllStats`
//!   ends reset).
//! - **Readiness.** There is no "request current stats" call in this Steam SDK: the Steam client
//!   loads them before the game starts. The kit probes readiness (about once a second, every
//!   frame when no clock is available) and holds writes made before that in a bounded queue,
//!   applied in order once ready. A write that falls out of the queue is reported as
//!   [`StatsErrorKind::NotReady`], never dropped silently.
//! - **Storing.** Writes change Steam's in-memory copy; the kit sends them with batched stores:
//!   an achievement change after [`StatsSettings::achievement_store_delay`], stat changes after
//!   [`StatsSettings::stats_store_interval`], never two stores closer than
//!   [`StatsSettings::min_store_gap`], one store in flight at a time,
//!   [`StatsRequest::StoreStats`] = as soon as the gap allows, and a final store on `AppExit`
//!   (requests written in the exit frame after [`SteamKitSystems::Requests`] are applied first).
//!
//! Schedules: pumped stats events are applied in [`SteamKitSystems::Callbacks`] (`First`);
//! requests, readiness and stores in [`SteamKitSystems::Requests`] in `Update`; the exit store in
//! [`SteamKitSystems::Requests`] in `Last`. Timing uses `Time<Real>` (from Bevy's `TimePlugin`,
//! part of `MinimalPlugins` and `DefaultPlugins`). Without it, readiness is probed every frame and
//! only explicit and exit stores run (no batching, no minimum gap, no store timeout).

mod backend;
pub(crate) mod fake;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use crate::names::{is_valid_api_name, MAX_API_NAME_BYTES};
pub use backend::{StatKind, StatValue, StatsBackend};
pub use fake::FakeStoreFailure;

use std::collections::VecDeque;
use std::time::Duration;

use bevy_app::{App, AppExit, First, Last, Update};
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_time::{Real, Time};
use tracing::{info, warn};

use crate::{BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

/// After this many failed readiness probes with a configured `probe`, warn once that the name
/// may be wrong.
const PROBE_WARN_AFTER: u32 = 10;

// ---------------------------------------------------------------------------------------------
// Settings + state
// ---------------------------------------------------------------------------------------------

/// The stats feature's settings, given with
/// [`SteamKitPlugin::with_stats`](crate::SteamKitPlugin::with_stats) and also inserted as a
/// resource by the plugin (read-only). Build it with `..Default::default()`.
#[derive(Resource, Clone, Debug)]
pub struct StatsSettings {
    /// A stat or achievement API name used to detect that Steam has loaded the stats. `None`
    /// (default): ready once the app reports at least one achievement, or once a held write
    /// succeeds. Set it when your app has stats but no achievements.
    pub probe: Option<String>,
    /// How often readiness is probed while not ready. Default 1 s.
    pub probe_interval: Duration,
    /// Stat changes are stored this long after the first unsaved change. Default 60 s (Valve:
    /// store "on the order of minutes, rather than seconds").
    pub stats_store_interval: Duration,
    /// Achievement changes are stored this long after the first one (batches a burst of unlocks
    /// while keeping the unlock popup prompt). Default 1 s.
    pub achievement_store_delay: Duration,
    /// Never two stores closer than this. Default 10 s.
    pub min_store_gap: Duration,
    /// A store whose outcome has not arrived after this long is given up
    /// ([`StatsErrorKind::StoreTimedOut`]) and the changes are stored again later. Default 30 s.
    pub store_timeout: Duration,
    /// Store unsaved changes on `AppExit`. Default `true`.
    pub store_on_exit: bool,
    /// How many writes are held while stats are not ready. Default 256.
    pub max_queued: usize,
}

impl Default for StatsSettings {
    fn default() -> Self {
        Self {
            probe: None,
            probe_interval: Duration::from_secs(1),
            stats_store_interval: Duration::from_secs(60),
            achievement_store_delay: Duration::from_secs(1),
            min_store_gap: Duration::from_secs(10),
            store_timeout: Duration::from_secs(30),
            store_on_exit: true,
            max_queued: 256,
        }
    }
}

/// The stats feature's state. Written only by the kit; read it through its methods.
#[derive(Resource, Debug, Default)]
pub struct SteamStats {
    ready: bool,
    dirty_stats_since: Option<Duration>,
    dirty_achievements_since: Option<Duration>,
    store_requested: bool,
    in_flight_since: Option<Duration>,
    /// The store in flight carried achievement changes (restored as unsaved if it fails).
    in_flight_had_achievements: bool,
    last_store: Option<Duration>,
    last_probe: Option<Duration>,
    probe_failures: u32,
    probe_warned: bool,
    /// A locally refused store was reported; stay quiet until a store starts again.
    refusal_reported: bool,
    /// A store failed (seen in `First`, which has no clock): mark the changes unsaved in `Update`.
    retry_store: bool,
    queued: VecDeque<StatsRequest>,
    stores: u64,
    /// ONE read position in the request stream, shared by the `Update` and the `Last` system.
    cursor: MessageCursor<StatsRequest>,
}

impl SteamStats {
    /// Steam has loaded the local user's stats; writes are applied at once.
    pub fn is_ready(&self) -> bool {
        self.ready
    }
    /// Changes (or held writes, or a requested store) are waiting to be stored.
    pub fn has_unsaved(&self) -> bool {
        self.unsaved_changes() || !self.queued.is_empty()
    }
    /// A store was started and its outcome has not arrived yet.
    pub fn store_in_flight(&self) -> bool {
        self.in_flight_since.is_some()
    }
    /// Writes held until stats are ready.
    pub fn queued(&self) -> usize {
        self.queued.len()
    }
    /// Stores started so far (including the exit store).
    pub fn stores_started(&self) -> u64 {
        self.stores
    }

    fn unsaved_changes(&self) -> bool {
        self.dirty_stats_since.is_some() || self.dirty_achievements_since.is_some() || self.store_requested || self.retry_store
    }
}

// ---------------------------------------------------------------------------------------------
// Messages IN: one ordered stream
// ---------------------------------------------------------------------------------------------

/// A stats request. All kinds travel in this ONE message type, so they are applied in the order
/// they were written. `#[non_exhaustive]`: later versions may add kinds (construct the variants,
/// or use the helper constructors, as usual).
#[derive(Message, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StatsRequest {
    /// Set a stat. The value's variant must match the stat's type (`INT` = `I32`, `FLOAT` = `F32`).
    SetStat {
        /// Stat API name.
        name: String,
        /// New value.
        value: StatValue,
    },
    /// Add to a stat (read, add, write; the delta's variant must match the stat's type). An `I32`
    /// sum saturates.
    AddStat {
        /// Stat API name.
        name: String,
        /// Amount to add.
        delta: StatValue,
    },
    /// Unlock an achievement. Unlocking an unlocked achievement does nothing.
    UnlockAchievement {
        /// Achievement API name.
        name: String,
    },
    /// Lock an achievement again (meant for development and tests).
    ClearAchievement {
        /// Achievement API name.
        name: String,
    },
    /// Show the "current / max" progress popup of an achievement. It sets and unlocks nothing
    /// (not even at `current == max`); `current` is clamped to `max`, `max == 0` is refused.
    IndicateAchievementProgress {
        /// Achievement API name.
        name: String,
        /// Progress to show.
        current: u32,
        /// Progress maximum.
        max: u32,
    },
    /// Store now (as soon as [`StatsSettings::min_store_gap`] allows and no store is in flight).
    /// Without Steam it is a silent no-op.
    StoreStats,
    /// Reset every stat, and optionally every achievement, of the local user (Steam stores it).
    /// Meant for development and tests.
    ResetAllStats {
        /// Lock every achievement too.
        achievements_too: bool,
    },
}

impl StatsRequest {
    /// [`StatsRequest::SetStat`].
    pub fn set_stat(name: impl Into<String>, value: StatValue) -> Self {
        Self::SetStat { name: name.into(), value }
    }
    /// [`StatsRequest::AddStat`].
    pub fn add_stat(name: impl Into<String>, delta: StatValue) -> Self {
        Self::AddStat { name: name.into(), delta }
    }
    /// [`StatsRequest::UnlockAchievement`].
    pub fn unlock_achievement(name: impl Into<String>) -> Self {
        Self::UnlockAchievement { name: name.into() }
    }
    /// [`StatsRequest::ClearAchievement`].
    pub fn clear_achievement(name: impl Into<String>) -> Self {
        Self::ClearAchievement { name: name.into() }
    }
    /// [`StatsRequest::IndicateAchievementProgress`].
    pub fn indicate_achievement_progress(name: impl Into<String>, current: u32, max: u32) -> Self {
        Self::IndicateAchievementProgress { name: name.into(), current, max }
    }

    /// The stat / achievement this request names, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            StatsRequest::SetStat { name, .. }
            | StatsRequest::AddStat { name, .. }
            | StatsRequest::UnlockAchievement { name }
            | StatsRequest::ClearAchievement { name }
            | StatsRequest::IndicateAchievementProgress { name, .. } => Some(name),
            StatsRequest::StoreStats | StatsRequest::ResetAllStats { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Messages OUT
// ---------------------------------------------------------------------------------------------

/// Stats are ready: written the first time readiness is detected, and again whenever Steam
/// reloads the local user's stats (read your values again then). `#[non_exhaustive]`: read it,
/// the kit writes it.
#[derive(Message, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsReady;

/// A store completed successfully. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsStored;

/// Steam confirmed an achievement unlock (after a store). `#[non_exhaustive]`: read it, the kit
/// writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AchievementUnlocked {
    /// Achievement API name.
    pub name: String,
}

/// Steam showed an achievement's progress popup. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AchievementProgress {
    /// Achievement API name.
    pub name: String,
    /// Progress shown.
    pub current: u32,
    /// Progress maximum.
    pub max: u32,
}

/// What went wrong in a [`StatsError`]. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatsErrorKind {
    /// No [`SteamBackendRes`] (or its backend does not support stats).
    NoBackend,
    /// Empty, longer than [`MAX_API_NAME_BYTES`], or containing a NUL byte.
    InvalidName,
    /// A float value (or an `AddStat` result) that is NaN or infinite.
    NotFinite,
    /// A request field was unusable (`IndicateAchievementProgress` with `max == 0`).
    InvalidRequest,
    /// Steam refused the call: unknown name, wrong stat type, or not allowed for this app (for
    /// example lowering a stat that may only increase).
    Refused,
    /// A write held while stats were not ready was dropped (the queue was full, or the app
    /// exited first).
    NotReady,
    /// `store_stats` was refused locally (no stats for this app, or not loaded). Reported once;
    /// the changes stay unsaved and are tried again at the normal cadence.
    StoreRefused,
    /// Steam rejected the store (`InvalidParameter`: a stat broke a constraint set on the partner
    /// site). Steam restored its values: read them again.
    StoreRejected,
    /// The store failed for another reason (the changes are stored again later).
    StoreFailed,
    /// No store outcome arrived within [`StatsSettings::store_timeout`] (the changes are stored
    /// again later).
    StoreTimedOut,
}

/// A stats request or store failed. The game decides how (or whether) to tell the user.
/// `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsError {
    /// Category.
    pub kind: StatsErrorKind,
    /// The stat / achievement concerned, when there is one.
    pub name: Option<String>,
    /// Human-readable detail (ASCII).
    pub message: String,
}

impl StatsError {
    fn new(kind: StatsErrorKind, name: Option<&str>, message: impl Into<String>) -> Self {
        Self { kind, name: name.map(str::to_string), message: message.into() }
    }
}

// ---------------------------------------------------------------------------------------------
// Plugin wiring
// ---------------------------------------------------------------------------------------------

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App, settings: &StatsSettings) {
    app.insert_resource(settings.clone())
        .init_resource::<SteamStats>()
        .add_message::<StatsRequest>()
        .add_message::<StatsReady>()
        .add_message::<StatsStored>()
        .add_message::<AchievementUnlocked>()
        .add_message::<AchievementProgress>()
        .add_message::<StatsError>()
        .add_systems(First, apply_stats_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_stat_requests.in_set(SteamKitSystems::Requests))
        .add_systems(Last, store_on_exit.in_set(SteamKitSystems::Requests));
}

#[derive(SystemParam)]
struct StatsOut<'w> {
    ready: MessageWriter<'w, StatsReady>,
    stored: MessageWriter<'w, StatsStored>,
    unlocked: MessageWriter<'w, AchievementUnlocked>,
    progress: MessageWriter<'w, AchievementProgress>,
    error: MessageWriter<'w, StatsError>,
}

/// The clock: `now` and whether a real clock exists.
#[derive(Clone, Copy)]
struct Clock {
    now: Duration,
    missing: bool,
}

impl Clock {
    fn of(time: &Option<Res<Time<Real>>>) -> Self {
        match time {
            Some(t) => Clock { now: t.elapsed(), missing: false },
            None => Clock { now: Duration::ZERO, missing: true },
        }
    }
    fn waited(&self, since: Option<Duration>, wait: Duration) -> bool {
        !self.missing && since.is_some_and(|t| self.now.saturating_sub(t) >= wait)
    }
}

fn mark_ready(stats: &mut SteamStats, out: &mut StatsOut, why: &str) {
    if !stats.ready {
        info!(">>> STEAM: stats ready ({why})");
    }
    stats.ready = true;
    out.ready.write(StatsReady);
}

/// Mark a failed / timed-out store's changes unsaved again.
fn restore_unsaved(stats: &mut SteamStats, now: Duration) {
    stats.dirty_stats_since.get_or_insert(now);
    if std::mem::take(&mut stats.in_flight_had_achievements) {
        stats.dirty_achievements_since.get_or_insert(now);
    }
}

/// The requests written since the last read (by either stats system).
fn take_new(stats: &mut SteamStats, messages: &Messages<StatsRequest>) -> Vec<StatsRequest> {
    stats.cursor.read(messages).cloned().collect()
}

// ---------------------------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------------------------

/// `First` ([`SteamKitSystems::Callbacks`]): this frame's pumped stats events -> state + OUT
/// messages. Never pumps by itself and reads no clock (that would be ambiguous with Bevy's own
/// time update in `First`).
fn apply_stats_events(backend: Option<Res<SteamBackendRes>>, pumped: Res<PumpedEvents>, mut stats: ResMut<SteamStats>, mut out: StatsOut) {
    let Some(backend) = backend.as_ref() else {
        let lost = pumped.0.iter().filter(|e| is_stats_event(e)).count();
        if lost > 0 {
            warn!(">>> STEAM: backend removed before {lost} pumped stats event(s) were applied - dropped");
        }
        return;
    };
    if backend.0.stats().is_none() {
        return;
    }
    let local = backend.0.local_id();
    for ev in &pumped.0 {
        match ev {
            &BackendEvent::StatsReceived { user, ok } => {
                if user != local {
                    continue;
                }
                if ok {
                    mark_ready(&mut stats, &mut out, "stats received");
                } else {
                    warn!(">>> STEAM: Steam reported a failure loading the local stats");
                }
            }
            BackendEvent::StatsStored => {
                stats.in_flight_since = None;
                stats.in_flight_had_achievements = false;
                info!(">>> STEAM: stats stored");
                out.stored.write(StatsStored);
            }
            BackendEvent::StatsStoreRejected => {
                stats.in_flight_since = None;
                stats.in_flight_had_achievements = false;
                warn!(">>> STEAM: stats store REJECTED (a stat broke a constraint) - Steam restored its values");
                out.error.write(StatsError::new(
                    StatsErrorKind::StoreRejected,
                    None,
                    "store rejected: a stat broke a constraint; Steam restored its values - read them again",
                ));
            }
            BackendEvent::StatsStoreFailed { message } => {
                stats.in_flight_since = None;
                stats.retry_store = true;
                warn!(">>> STEAM: stats store FAILED: {message} (will store again)");
                out.error.write(StatsError::new(StatsErrorKind::StoreFailed, None, format!("store failed: {message}")));
            }
            BackendEvent::AchievementStored { name, current, max } => {
                if *current == 0 && *max == 0 {
                    info!(">>> STEAM: achievement {name} unlocked");
                    out.unlocked.write(AchievementUnlocked { name: name.clone() });
                } else {
                    info!(">>> STEAM: achievement {name} progress {current}/{max}");
                    out.progress.write(AchievementProgress { name: name.clone(), current: *current, max: *max });
                }
            }
            // Another feature's event.
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}

fn is_stats_event(ev: &BackendEvent) -> bool {
    matches!(
        ev,
        BackendEvent::StatsReceived { .. }
            | BackendEvent::StatsStored
            | BackendEvent::StatsStoreRejected
            | BackendEvent::StatsStoreFailed { .. }
            | BackendEvent::AchievementStored { .. }
    )
}

/// Validate a write (errors are written at once, never queued).
fn validate(req: &StatsRequest, out: &mut StatsOut) -> bool {
    if let Some(name) = req.name() {
        if !is_valid_api_name(name) {
            out.error.write(StatsError::new(
                StatsErrorKind::InvalidName,
                Some(name),
                format!("invalid API name (empty, over {MAX_API_NAME_BYTES} bytes, or a NUL byte)"),
            ));
            return false;
        }
    }
    match req {
        StatsRequest::SetStat { value: StatValue::F32(v), name } | StatsRequest::AddStat { delta: StatValue::F32(v), name } if !v.is_finite() => {
            out.error.write(StatsError::new(StatsErrorKind::NotFinite, Some(name), "value is NaN or infinite"));
            false
        }
        StatsRequest::IndicateAchievementProgress { max: 0, name, .. } => {
            out.error.write(StatsError::new(StatsErrorKind::InvalidRequest, Some(name), "progress max is 0"));
            false
        }
        _ => true,
    }
}

/// Apply one validated write. `report`: write errors. Returns whether Steam accepted it.
fn apply(req: &StatsRequest, api: &dyn StatsBackend, now: Duration, stats: &mut SteamStats, out: &mut StatsOut, report: bool) -> bool {
    let refused = |out: &mut StatsOut, name: &str, what: &str| {
        if report {
            warn!(">>> STEAM: {what} {name:?} refused by Steam");
            out.error.write(StatsError::new(StatsErrorKind::Refused, Some(name), format!("{what} refused (unknown name, wrong type, or not allowed)")));
        }
    };
    match req {
        StatsRequest::SetStat { name, value } => {
            if api.set_stat(name, *value) {
                stats.dirty_stats_since.get_or_insert(now);
                true
            } else {
                refused(out, name, "set stat");
                false
            }
        }
        StatsRequest::AddStat { name, delta } => {
            let Some(current) = api.get_stat(name, delta.kind()) else {
                refused(out, name, "add to stat");
                return false;
            };
            let sum = match (current, delta) {
                (StatValue::I32(a), StatValue::I32(b)) => StatValue::I32(a.saturating_add(*b)),
                (StatValue::F32(a), StatValue::F32(b)) => {
                    let s = a + b;
                    if !s.is_finite() {
                        if report {
                            out.error.write(StatsError::new(StatsErrorKind::NotFinite, Some(name), "sum is not finite"));
                        }
                        return false;
                    }
                    StatValue::F32(s)
                }
                _ => {
                    refused(out, name, "add to stat");
                    return false;
                }
            };
            if api.set_stat(name, sum) {
                stats.dirty_stats_since.get_or_insert(now);
                true
            } else {
                refused(out, name, "add to stat");
                false
            }
        }
        StatsRequest::UnlockAchievement { name } => {
            if api.achievement(name) == Some(true) {
                return true;
            }
            if api.unlock_achievement(name) {
                info!(">>> STEAM: achievement {name} set (stored soon)");
                stats.dirty_achievements_since.get_or_insert(now);
                true
            } else {
                refused(out, name, "unlock achievement");
                false
            }
        }
        StatsRequest::ClearAchievement { name } => {
            if api.clear_achievement(name) {
                info!(">>> STEAM: achievement {name} cleared (stored soon)");
                stats.dirty_achievements_since.get_or_insert(now);
                true
            } else {
                refused(out, name, "clear achievement");
                false
            }
        }
        StatsRequest::IndicateAchievementProgress { name, current, max } => {
            if api.indicate_achievement_progress(name, (*current).min(*max), *max) {
                true
            } else {
                refused(out, name, "indicate achievement progress");
                false
            }
        }
        StatsRequest::ResetAllStats { achievements_too } => {
            if api.reset_all_stats(*achievements_too) {
                warn!(">>> STEAM: ALL stats reset (achievements too: {achievements_too})");
                // Steam stores the reset itself.
                stats.dirty_stats_since = None;
                stats.dirty_achievements_since = None;
                stats.in_flight_since = Some(now);
                stats.in_flight_had_achievements = false;
                stats.last_store = Some(now);
                true
            } else {
                if report {
                    out.error.write(StatsError::new(StatsErrorKind::Refused, None, "reset all stats refused"));
                }
                false
            }
        }
        StatsRequest::StoreStats => {
            stats.store_requested = true;
            true
        }
    }
}

/// Hold a validated write until stats are ready (bounded; the oldest is dropped, reported).
fn hold(req: StatsRequest, settings: &StatsSettings, stats: &mut SteamStats, out: &mut StatsOut) {
    if stats.queued.len() >= settings.max_queued.max(1) {
        if let Some(dropped) = stats.queued.pop_front() {
            warn!(">>> STEAM: stats not ready - queue full, dropped the oldest held write");
            out.error.write(StatsError::new(StatsErrorKind::NotReady, dropped.name(), "stats not ready: held write dropped (queue full)"));
        }
    }
    stats.queued.push_back(req);
}

/// Route this frame's requests, in order.
fn route(requests: Vec<StatsRequest>, api: &dyn StatsBackend, now: Duration, settings: &StatsSettings, stats: &mut SteamStats, out: &mut StatsOut) {
    for req in requests {
        if req == StatsRequest::StoreStats {
            stats.store_requested = true;
            continue;
        }
        if !validate(&req, out) {
            continue;
        }
        if stats.ready {
            apply(&req, api, now, stats, out, true);
        } else {
            hold(req, settings, stats, out);
        }
    }
}

/// Probe readiness when due; without an answer, the first held write that succeeds counts.
fn check_ready(api: &dyn StatsBackend, clock: Clock, force: bool, settings: &StatsSettings, stats: &mut SteamStats, out: &mut StatsOut) {
    if stats.ready {
        return;
    }
    // Probe every frame when there is no clock, or when it does not advance.
    let due = force || clock.missing || stats.last_probe.is_none_or(|t| clock.now == t || clock.now.saturating_sub(t) >= settings.probe_interval);
    if !due {
        return;
    }
    stats.last_probe = Some(clock.now);
    let probe = settings.probe.as_deref();
    if let Some(p) = probe {
        if !is_valid_api_name(p) && !stats.probe_warned {
            stats.probe_warned = true;
            warn!(">>> STEAM: the stats probe {p:?} is not a valid API name - readiness comes only from Steam or a held write");
        }
    }
    if api.is_ready(probe) {
        mark_ready(stats, out, "probe");
        return;
    }
    stats.probe_failures = stats.probe_failures.saturating_add(1);
    if let Some(p) = probe {
        if stats.probe_failures >= PROBE_WARN_AFTER && !stats.probe_warned {
            stats.probe_warned = true;
            warn!(">>> STEAM: the stats probe {p:?} has not answered after {PROBE_WARN_AFTER} tries - check that the name exists for this app");
        }
    }
    // A held write that Steam accepts proves the stats are loaded. Try each (a permanently
    // refused one must not block the others).
    for i in 0..stats.queued.len() {
        let Some(req) = stats.queued.get(i).cloned() else { break };
        if apply(&req, api, clock.now, stats, out, false) {
            stats.queued.remove(i);
            mark_ready(stats, out, "a held write succeeded");
            return;
        }
    }
}

/// The store policy: timeout of the store in flight, gap, due.
fn store_policy(api: &dyn StatsBackend, clock: Clock, settings: &StatsSettings, stats: &mut SteamStats, out: &mut StatsOut) {
    let now = clock.now;
    if let Some(since) = stats.in_flight_since {
        if clock.waited(Some(since), settings.store_timeout) {
            stats.in_flight_since = None;
            restore_unsaved(stats, now);
            warn!(">>> STEAM: stats store timed out (will store again)");
            out.error.write(StatsError::new(StatsErrorKind::StoreTimedOut, None, "no store outcome arrived in time"));
        } else {
            return;
        }
    }
    let gap_ok = clock.missing || stats.last_store.is_none_or(|t| now.saturating_sub(t) >= settings.min_store_gap);
    let due = stats.store_requested
        || clock.waited(stats.dirty_achievements_since, settings.achievement_store_delay)
        || clock.waited(stats.dirty_stats_since, settings.stats_store_interval);
    if !(due && gap_ok) {
        return;
    }
    stats.last_store = Some(now);
    stats.stores += 1;
    stats.store_requested = false;
    let had_achievements = stats.dirty_achievements_since.is_some();
    if api.store_stats() {
        stats.in_flight_since = Some(now);
        stats.in_flight_had_achievements = had_achievements;
        stats.dirty_stats_since = None;
        stats.dirty_achievements_since = None;
        stats.refusal_reported = false;
        info!(">>> STEAM: storing stats");
    } else {
        // Keep the changes unsaved and restart their timers: tried again at the normal cadence.
        if stats.dirty_stats_since.is_some() {
            stats.dirty_stats_since = Some(now);
        }
        if stats.dirty_achievements_since.is_some() {
            stats.dirty_achievements_since = Some(now);
        }
        if !stats.refusal_reported {
            stats.refusal_reported = true;
            warn!(">>> STEAM: stats store REFUSED by Steam (will try again; reported once)");
            out.error.write(StatsError::new(StatsErrorKind::StoreRefused, None, "store refused (no stats for this app, or not loaded)"));
        }
    }
}

/// `Update` ([`SteamKitSystems::Requests`]): readiness, the held queue, the request stream, and
/// the store policy.
fn handle_stat_requests(
    backend: Option<Res<SteamBackendRes>>,
    time: Option<Res<Time<Real>>>,
    settings: Res<StatsSettings>,
    messages: Res<Messages<StatsRequest>>,
    mut stats: ResMut<SteamStats>,
    mut out: StatsOut,
) {
    let clock = Clock::of(&time);
    let requests = take_new(&mut stats, &messages);
    let Some(api) = backend.as_ref().and_then(|b| b.0.stats()) else {
        no_backend(&requests, &mut out);
        return;
    };
    if std::mem::take(&mut stats.retry_store) {
        restore_unsaved(&mut stats, clock.now);
    }
    check_ready(api, clock, false, &settings, &mut stats, &mut out);
    if stats.ready {
        while let Some(req) = stats.queued.pop_front() {
            apply(&req, api, clock.now, &mut stats, &mut out, true);
        }
    }
    route(requests, api, clock.now, &settings, &mut stats, &mut out);
    if stats.ready {
        store_policy(api, clock, &settings, &mut stats, &mut out);
    }
}

/// Writes without Steam: `NoBackend` each (a store request is a silent no-op).
fn no_backend(requests: &[StatsRequest], out: &mut StatsOut) {
    for req in requests {
        if *req != StatsRequest::StoreStats {
            out.error.write(StatsError::new(StatsErrorKind::NoBackend, req.name(), "stats: Steam is not available"));
        }
    }
}

/// Writes still held at exit were never applied: report each (also without a backend).
fn report_held_at_exit(stats: &mut SteamStats, out: &mut StatsOut) {
    if !stats.queued.is_empty() {
        warn!(">>> STEAM: exiting with {} stats write(s) never applied (stats were not ready)", stats.queued.len());
        for req in std::mem::take(&mut stats.queued) {
            out.error.write(StatsError::new(StatsErrorKind::NotReady, req.name(), "stats not ready before exit: write not applied"));
        }
    }
}

/// `Last`: on `AppExit` (write it before `Last`), apply the requests written since `Update` (in order), report writes that
/// never got applied, and store unsaved changes once.
fn store_on_exit(
    mut exit: MessageReader<AppExit>,
    backend: Option<Res<SteamBackendRes>>,
    time: Option<Res<Time<Real>>>,
    settings: Res<StatsSettings>,
    messages: Res<Messages<StatsRequest>>,
    mut stats: ResMut<SteamStats>,
    mut out: StatsOut,
) {
    if exit.read().count() == 0 {
        return;
    }
    let clock = Clock::of(&time);
    let requests = take_new(&mut stats, &messages);
    let Some(api) = backend.as_ref().and_then(|b| b.0.stats()) else {
        no_backend(&requests, &mut out);
        report_held_at_exit(&mut stats, &mut out);
        return;
    };
    if std::mem::take(&mut stats.retry_store) {
        restore_unsaved(&mut stats, clock.now);
    }
    check_ready(api, clock, true, &settings, &mut stats, &mut out);
    if stats.ready {
        while let Some(req) = stats.queued.pop_front() {
            apply(&req, api, clock.now, &mut stats, &mut out, true);
        }
    }
    route(requests, api, clock.now, &settings, &mut stats, &mut out);
    report_held_at_exit(&mut stats, &mut out);
    if settings.store_on_exit && stats.ready && stats.unsaved_changes() {
        stats.stores += 1;
        stats.dirty_stats_since = None;
        stats.dirty_achievements_since = None;
        stats.store_requested = false;
        stats.retry_store = false;
        if api.store_stats() {
            info!(">>> STEAM: stats stored on exit");
        } else {
            warn!(">>> STEAM: stats store on exit refused");
        }
    }
}
