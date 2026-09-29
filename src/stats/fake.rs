//! The stats half of [`FakeSteamBackend`] (feature `stats`): in-memory stats and achievements.

use std::collections::HashMap;

use super::backend::{StatKind, StatValue, StatsBackend};
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};

/// How the fake's next `store_stats` goes wrong (see [`FakeSteamBackend::fail_next_store`]).
/// `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FakeStoreFailure {
    /// `store_stats` returns `false` (refused locally).
    Refused,
    /// The store completes with `InvalidParameter`: the stats go back to the last stored values.
    Rejected,
    /// The store completes with another failure; the values are kept.
    Failed,
    /// `store_stats` returns `true` but no outcome ever arrives.
    NoAnswer,
}

/// The fake's stats state, stored inside the shared fake state.
#[derive(Debug)]
pub(crate) struct FakeStatsState {
    ready: bool,
    defaults: HashMap<String, StatValue>,
    stats: HashMap<String, StatValue>,
    stored: HashMap<String, StatValue>,
    achievements: HashMap<String, bool>,
    unlocked_since_store: Vec<String>,
    next_failure: Option<FakeStoreFailure>,
}

impl Default for FakeStatsState {
    fn default() -> Self {
        Self {
            ready: true,
            defaults: HashMap::new(),
            stats: HashMap::new(),
            stored: HashMap::new(),
            achievements: HashMap::new(),
            unlocked_since_store: Vec::new(),
            next_failure: None,
        }
    }
}

/// Stats controls (feature `stats`).
impl FakeSteamBackend {
    /// Define a stat with its type (the variant) and starting value, as the partner site would.
    pub fn define_stat(&self, name: &str, value: StatValue) {
        let mut s = self.lock();
        s.stats.defaults.insert(name.to_string(), value);
        s.stats.stats.insert(name.to_string(), value);
        s.stats.stored.insert(name.to_string(), value);
    }

    /// Define an achievement, locked or already unlocked.
    pub fn define_achievement(&self, name: &str, unlocked: bool) {
        self.lock().stats.achievements.insert(name.to_string(), unlocked);
    }

    /// `false`: every stats call fails as if Steam had not loaded the stats yet (default `true`).
    pub fn set_stats_ready(&self, ready: bool) {
        self.lock().stats.ready = ready;
    }

    /// Make the next `store_stats` fail in the given way.
    pub fn fail_next_store(&self, failure: FakeStoreFailure) {
        self.lock().stats.next_failure = Some(failure);
    }

    /// A stat's current in-memory value.
    pub fn stat(&self, name: &str) -> Option<StatValue> {
        self.lock().stats.stats.get(name).copied()
    }

    /// Whether an achievement is unlocked (in memory).
    pub fn achieved(&self, name: &str) -> Option<bool> {
        self.lock().stats.achievements.get(name).copied()
    }
}

impl StatsBackend for FakeSteamBackend {
    fn is_ready(&self, probe: Option<&str>) -> bool {
        let s = self.lock();
        s.stats.ready && probe.is_none_or(|p| s.stats.stats.contains_key(p) || s.stats.achievements.contains_key(p))
    }

    fn get_stat(&self, name: &str, kind: StatKind) -> Option<StatValue> {
        let s = self.lock();
        if !s.stats.ready {
            return None;
        }
        s.stats.stats.get(name).copied().filter(|v| v.kind() == kind)
    }

    fn set_stat(&self, name: &str, value: StatValue) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::SetStat { name: name.to_string(), value });
        if !s.stats.ready {
            return false;
        }
        if let StatValue::F32(v) = value {
            if !v.is_finite() {
                return false;
            }
        }
        match s.stats.stats.get_mut(name) {
            Some(current) if current.kind() == value.kind() => {
                *current = value;
                true
            }
            _ => false,
        }
    }

    fn achievement(&self, name: &str) -> Option<bool> {
        let s = self.lock();
        if !s.stats.ready {
            return None;
        }
        s.stats.achievements.get(name).copied()
    }

    fn unlock_achievement(&self, name: &str) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::UnlockAchievement(name.to_string()));
        if !s.stats.ready {
            return false;
        }
        match s.stats.achievements.get_mut(name) {
            Some(achieved) => {
                if !*achieved {
                    *achieved = true;
                    s.stats.unlocked_since_store.push(name.to_string());
                }
                true
            }
            None => false,
        }
    }

    fn clear_achievement(&self, name: &str) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::ClearAchievement(name.to_string()));
        if !s.stats.ready {
            return false;
        }
        match s.stats.achievements.get_mut(name) {
            Some(achieved) => {
                *achieved = false;
                s.stats.unlocked_since_store.retain(|n| n != name);
                true
            }
            None => false,
        }
    }

    fn indicate_achievement_progress(&self, name: &str, current: u32, max: u32) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::IndicateAchievementProgress { name: name.to_string(), current, max });
        if !s.stats.ready || max == 0 || !s.stats.achievements.contains_key(name) {
            return false;
        }
        s.queued.push(BackendEvent::AchievementStored { name: name.to_string(), current, max });
        true
    }

    fn store_stats(&self) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::StoreStats);
        if !s.stats.ready {
            return false;
        }
        match s.stats.next_failure.take() {
            Some(FakeStoreFailure::Refused) => false,
            Some(FakeStoreFailure::Rejected) => {
                s.stats.stats = s.stats.stored.clone();
                s.queued.push(BackendEvent::StatsStoreRejected);
                true
            }
            Some(FakeStoreFailure::Failed) => {
                s.queued.push(BackendEvent::StatsStoreFailed { message: "fake store failure".to_string() });
                true
            }
            Some(FakeStoreFailure::NoAnswer) => true,
            None => {
                s.stats.stored = s.stats.stats.clone();
                s.queued.push(BackendEvent::StatsStored);
                for name in std::mem::take(&mut s.stats.unlocked_since_store) {
                    s.queued.push(BackendEvent::AchievementStored { name, current: 0, max: 0 });
                }
                true
            }
        }
    }

    fn reset_all_stats(&self, achievements_too: bool) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::ResetAllStats { achievements_too });
        if !s.stats.ready {
            return false;
        }
        s.stats.stats = s.stats.defaults.clone();
        s.stats.stored = s.stats.defaults.clone();
        if achievements_too {
            s.stats.achievements.values_mut().for_each(|a| *a = false);
            s.stats.unlocked_since_store.clear();
        }
        s.queued.push(BackendEvent::StatsStored);
        true
    }
}
