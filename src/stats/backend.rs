//! The stats half of the backend seam: [`StatsBackend`], reached through
//! [`SteamBackend::stats`](crate::SteamBackend::stats), and the value types it uses.

/// The type of a stat as configured on the Steamworks partner site. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatKind {
    /// An `INT` stat.
    I32,
    /// A `FLOAT` stat.
    F32,
}

/// A stat value. The variant must match the stat's type on the partner site (an `INT` stat is
/// written with `I32`, a `FLOAT` stat with `F32`); a mismatch is refused by Steam.
/// `#[non_exhaustive]`; `PartialEq` only (it holds a float).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum StatValue {
    /// An `INT` stat value.
    I32(i32),
    /// A `FLOAT` stat value (must be finite).
    F32(f32),
}

impl StatValue {
    /// The kind of this value.
    pub fn kind(&self) -> StatKind {
        match self {
            StatValue::I32(_) => StatKind::I32,
            StatValue::F32(_) => StatKind::F32,
        }
    }
}

/// Everything the stats feature needs from Steam. Implementations must never panic; a failure is
/// a `false` / `None`. Names reaching a backend are already validated by the kit
/// ([`is_valid_api_name`](crate::is_valid_api_name)), but an implementation must stay safe on any
/// input.
///
/// Reads and writes change Steam's in-memory copy only; [`store_stats`](Self::store_stats) sends
/// them, and its outcome arrives later as a [`BackendEvent`](crate::BackendEvent) from
/// [`SteamBackend::pump`](crate::SteamBackend::pump).
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added in a later version comes with a default implementation.
pub trait StatsBackend {
    /// Are the local user's stats loaded? `probe` is a stat or achievement name to test with
    /// (`None`: the backend picks its own test). Must be cheap: called about once a second until
    /// it returns `true`.
    fn is_ready(&self, probe: Option<&str>) -> bool;
    /// Read a stat of the given kind. `None`: unknown name, wrong kind, or not loaded.
    fn get_stat(&self, name: &str, kind: StatKind) -> Option<StatValue>;
    /// Write a stat (in memory). `false`: unknown name, wrong kind, not loaded, or not finite.
    fn set_stat(&self, name: &str, value: StatValue) -> bool;
    /// Is the achievement unlocked? `None`: unknown name or not loaded.
    fn achievement(&self, name: &str) -> Option<bool>;
    /// Unlock an achievement (in memory; the popup shows on the next store). `false` on failure.
    fn unlock_achievement(&self, name: &str) -> bool;
    /// Lock an achievement again (in memory; meant for development). `false` on failure.
    fn clear_achievement(&self, name: &str) -> bool;
    /// Show the "current / max" progress popup of an achievement. Sets and unlocks nothing.
    /// `false` on failure.
    fn indicate_achievement_progress(&self, name: &str, current: u32, max: u32) -> bool;
    /// Send the in-memory changes to Steam. `true` = the store was started; its outcome arrives
    /// later. `false` = refused locally (nothing to store for this app, stats not loaded).
    fn store_stats(&self) -> bool;
    /// Reset every stat (and, optionally, every achievement) of the local user and store it.
    /// Meant for development. `false` on failure.
    fn reset_all_stats(&self, achievements_too: bool) -> bool;
}
