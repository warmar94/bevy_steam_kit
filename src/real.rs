//! [`RealSteamBackend`] (feature `steam`): the backend over `steamworks` 0.12.2. The feature
//! halves live in `lobby/real.rs`, `stats/real.rs` and `leaderboards/real.rs`.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `Client::process_callbacks` (lib.rs:319) runs `Inner::run_callbacks_raw` (lib.rs:154), which
//!   dispatches, in ONE call: (a) call results (the closures passed to async calls such as
//!   `create_lobby` / `join_lobby`, lib.rs:161-181), (b) every callback registered with
//!   `Client::register_callback` (lib.rs:141-146; e.g. a networking transport's), and (c) the
//!   handler closure with each callback `CallbackResult::from_raw` knows. So `process_callbacks`
//!   REPLACES `run_callbacks` (lib.rs:305): exactly one of them, once per frame, for the whole
//!   process. The kit calls `process_callbacks` from [`SteamBackend::pump`] and nothing else may
//!   call either; every feature receives its share through the returned events.
//! - `Matchmaking` / `Friends` / `Apps` hold raw pointers (not `Send`): fetched fresh from the
//!   `Client` every call, never stored.

use std::sync::{Arc, Mutex};

use steamworks::{Client, SteamId};

use crate::backend::{BackendEvent, SteamBackend};

pub(crate) type Queue = Arc<Mutex<Vec<BackendEvent>>>;

/// Push a call-result outcome for the next [`SteamBackend::pump`] to return.
#[cfg_attr(not(any(feature = "lobby", feature = "leaderboards")), allow(dead_code))]
pub(crate) fn push(queue: &Queue, ev: BackendEvent) {
    queue.lock().unwrap_or_else(|p| p.into_inner()).push(ev);
}

/// `true` when `s` contains a NUL byte: steamworks builds `CString::new(..).unwrap()` from its
/// string inputs, so such a string would PANIC inside steamworks. Every string input is checked.
#[cfg_attr(not(feature = "lobby"), allow(dead_code))]
pub(crate) fn has_nul(s: &str) -> bool {
    s.contains('\0')
}

/// The real Steam backend. Holds a `steamworks::Client` clone (which also keeps Steam alive).
pub struct RealSteamBackend {
    pub(crate) client: Client,
    pub(crate) queue: Queue,
    /// Found leaderboards by raw handle (steamworks' `Leaderboard` has no public constructor).
    #[cfg(feature = "leaderboards")]
    pub(crate) boards: crate::leaderboards::real::Boards,
}

impl RealSteamBackend {
    /// Wrap an initialised Steam client. The caller keeps its own clone for other uses (and must
    /// never pump it: the kit pumps).
    pub fn new(client: Client) -> Self {
        Self {
            client,
            queue: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "leaderboards")]
            boards: crate::leaderboards::real::Boards::default(),
        }
    }
}

impl SteamBackend for RealSteamBackend {
    fn local_id(&self) -> u64 {
        self.client.user().steam_id().raw()
    }

    fn friend_name(&self, id: u64) -> String {
        self.client.friends().get_friend(SteamId::from_raw(id)).name()
    }

    fn launch_command_line(&self) -> String {
        self.client.apps().launch_command_line()
    }

    fn pump(&self) -> Vec<BackendEvent> {
        // THE one pump. Call-result closures run INSIDE this call and push to `queue`, so the
        // queue must not be locked while it runs.
        #[cfg_attr(not(any(feature = "lobby", feature = "stats")), allow(unused_mut))]
        let mut out = Vec::new();
        // Read before the pump (never call Steam inside it): stats callbacks of another app
        // are ignored.
        #[cfg(feature = "stats")]
        let app_id = self.client.utils().app_id().0;
        self.client.process_callbacks(|cb| {
            // Every feature's mapper sees every callback (by reference). Mappers only convert;
            // they never call Steam.
            #[cfg(feature = "lobby")]
            out.extend(crate::lobby::real::map_callback(&cb));
            #[cfg(feature = "stats")]
            out.extend(crate::stats::real::map_callback(&cb, app_id));
            #[cfg(not(any(feature = "lobby", feature = "stats")))]
            drop(cb);
        });
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        out.append(&mut q);
        out
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
