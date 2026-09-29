//! The stats half of [`RealSteamBackend`] (features `steam` + `stats`), over `steamworks` 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `Client::user_stats` (lib.rs:440-449) returns a `UserStats` holding a raw pointer
//!   (`!Send`/`!Sync`): fetched fresh per call, never stored.
//! - Every name taking call builds `CString::new(name).unwrap()` (user_stats.rs:353, :376, :395,
//!   :418, :438): a NUL byte would PANIC inside steamworks. Every name is validated here first
//!   ([`is_valid_api_name`]: non-empty, at most 127 bytes, no NUL; the SDK's
//!   `k_cchStatNameMax = 128` includes the terminator).
//! - `get_achievement_names` (user_stats.rs:465-468) `expect`s and panics for an app with 0
//!   achievements: NEVER called. `get_num_achievements` maps 0 to `Err(())` (:450-459).
//! - There is no `request_current_stats` (the SDK 1.62 header comments `RequestCurrentStats` out:
//!   the Steam client loads stats before the game starts). Readiness is probed instead.
//! - `UserStatsReceived` / `UserStatsStored` / `UserAchievementStored` are in
//!   `CallbackResult::from_raw` (callback.rs:107-111): the kit's one pump sees them, no
//!   `register_callback`. `UserStatsStored` with `InvalidParameter` (error.rs:396) = a stat broke
//!   a constraint. `UserAchievementIconFetched` is never delivered that way and is not used.
//! - `IndicateAchievementProgress` has no wrapper; the raw `sys` function is called (bindings:
//!   `(self_: *mut ISteamUserStats, *const c_char, u32, u32) -> bool` on all three platforms).
//! - Floats: non-finite values are refused before reaching Steam (its behaviour is undocumented).

use std::ffi::CString;

use steamworks::{CallbackResult, SteamError};

use super::backend::{StatKind, StatValue, StatsBackend};
use crate::backend::BackendEvent;
use crate::real::RealSteamBackend;
use crate::stats::is_valid_api_name;

/// The stats callbacks of one `process_callbacks` run, as backend events, for the running app
/// (`app_id`) only (the callbacks carry a `game_id`, stat_callback.rs:20, :47, :73). Converts
/// only; never calls Steam.
pub(crate) fn map_callback(cb: &CallbackResult, app_id: u32) -> Option<BackendEvent> {
    match cb {
        CallbackResult::UserStatsReceived(r) if r.game_id.app_id().0 == app_id => {
            Some(BackendEvent::StatsReceived { user: r.steam_id.raw(), ok: r.result.is_ok() })
        }
        CallbackResult::UserStatsStored(r) if r.game_id.app_id().0 == app_id => Some(match &r.result {
            Ok(()) => BackendEvent::StatsStored,
            Err(SteamError::InvalidParameter) => BackendEvent::StatsStoreRejected,
            Err(e) => BackendEvent::StatsStoreFailed { message: e.to_string() },
        }),
        CallbackResult::UserAchievementStored(r) if r.game_id.app_id().0 == app_id => {
            Some(BackendEvent::AchievementStored { name: r.achievement_name.clone(), current: r.current_progress, max: r.max_progress })
        }
        _ => None,
    }
}

impl StatsBackend for RealSteamBackend {
    fn is_ready(&self, probe: Option<&str>) -> bool {
        let us = self.client.user_stats();
        match probe {
            Some(name) => is_valid_api_name(name) && (us.get_stat_i32(name).is_ok() || us.get_stat_f32(name).is_ok() || us.achievement(name).get().is_ok()),
            None => us.get_num_achievements().is_ok(),
        }
    }

    fn get_stat(&self, name: &str, kind: StatKind) -> Option<StatValue> {
        if !is_valid_api_name(name) {
            return None;
        }
        let us = self.client.user_stats();
        match kind {
            StatKind::I32 => us.get_stat_i32(name).ok().map(StatValue::I32),
            StatKind::F32 => us.get_stat_f32(name).ok().map(StatValue::F32),
        }
    }

    fn set_stat(&self, name: &str, value: StatValue) -> bool {
        if !is_valid_api_name(name) {
            return false;
        }
        let us = self.client.user_stats();
        match value {
            StatValue::I32(v) => us.set_stat_i32(name, v).is_ok(),
            StatValue::F32(v) => v.is_finite() && us.set_stat_f32(name, v).is_ok(),
        }
    }

    fn achievement(&self, name: &str) -> Option<bool> {
        if !is_valid_api_name(name) {
            return None;
        }
        self.client.user_stats().achievement(name).get().ok()
    }

    fn unlock_achievement(&self, name: &str) -> bool {
        is_valid_api_name(name) && self.client.user_stats().achievement(name).set().is_ok()
    }

    fn clear_achievement(&self, name: &str) -> bool {
        is_valid_api_name(name) && self.client.user_stats().achievement(name).clear().is_ok()
    }

    fn indicate_achievement_progress(&self, name: &str, current: u32, max: u32) -> bool {
        if !is_valid_api_name(name) || max == 0 {
            return false;
        }
        let Ok(cname) = CString::new(name) else { return false };
        // SAFETY: `self.client` is alive, so the Steam API is initialised and the interface
        // accessor is valid to call (steamworks' own `user_stats()` calls the same function).
        // The pointer is checked for null; `cname` outlives the call; the signature matches the
        // bundled SDK 1.62 bindings on every platform.
        unsafe {
            let us = steamworks::sys::SteamAPI_SteamUserStats_v013();
            if us.is_null() {
                return false;
            }
            steamworks::sys::SteamAPI_ISteamUserStats_IndicateAchievementProgress(us, cname.as_ptr(), current, max)
        }
    }

    fn store_stats(&self) -> bool {
        self.client.user_stats().store_stats().is_ok()
    }

    fn reset_all_stats(&self, achievements_too: bool) -> bool {
        self.client.user_stats().reset_all_stats(achievements_too).is_ok()
    }
}
