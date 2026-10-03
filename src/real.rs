//! [`RealSteamBackend`] (feature `steam`): the backend over `steamworks` 0.12.2. The feature
//! halves live in `<feature>/real.rs`.
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
//! - `Client::register_callback` keeps ONE closure per callback type for the whole process
//!   (callback.rs:178-185: a second registration replaces the first). Every feature reads its
//!   callbacks from the handler of the one `process_callbacks` (lib.rs:139-152 runs registered
//!   closures AND that handler), so a game's own registrations keep working. The kit registers
//!   exactly three callback types itself, the guards below, and nothing else. Their handles are
//!   `mem::forget`-ten: `CallbackHandle::drop` removes the entry for its id whoever registered it
//!   last (callback.rs:143-156), so a dropped old backend would remove a newer one's guards.
//!
//! Steam quitting (the shutdown guard, verified against the same source):
//! - `SteamServersDisconnected` (id 103) and `SteamServerConnectFailure` (id 102) convert
//!   `m_eResult` with `From<EResult> for SteamError` (user.rs:389-410), which PANICS on
//!   `k_EResultOK` (error.rs:390). Steam sends `SteamServersDisconnected` with OK when the Steam
//!   client quits, and `CallbackResult::from_raw` (callback.rs:51) converts every callback it
//!   knows before the handler sees it: without a guard, the one pump panics when Steam quits.
//! - Pumping through `SteamAPI_ManualDispatch_*` ourselves is not possible without breaking call
//!   results: `run_callbacks_raw` (lib.rs:154-189) hands each `SteamAPICallCompleted_t` to the
//!   closure stored in the PRIVATE `Inner::callbacks.call_results` map (filled by steamworks'
//!   `create_lobby`, `find_leaderboard`, ...), and a callback taken from the pipe cannot be put
//!   back. So the kit keeps `process_callbacks`.
//! - Inside `process_callbacks` the registered closure of a callback id runs on the raw callback
//!   memory BEFORE `CallbackResult::from_raw` reads the same memory (lib.rs:142-149). The kit
//!   registers its own `Callback` types for ids 102 and 103 ([`DisconnectGuard`],
//!   [`ConnectFailureGuard`]; the trait is public and implementable, callback.rs:126-129) whose
//!   `from_raw` rewrites an `m_eResult` of OK into `k_EResultNoConnection` in place. The
//!   conversion that follows then cannot panic: no panic happens, also with `panic = "abort"`.
//!   Every other callback, every call result and every other registration is untouched. A game
//!   that registers either of these two types itself replaces the guard (one closure per type),
//!   one registered before `new` is replaced by the guard, and dropping that game handle later
//!   removes the guard; a typed closure for them panics on Steam's OK with or without the kit.
//!   The rewrite relies on steam_api's callback buffer being writable (`m_pubParam` is a
//!   non-const `uint8 *` in the SDK; Valve does not document writing to it).
//! - The same way, [`RichPresenceJoinGuard`] (id 337) makes `GameRichPresenceJoinRequested` safe:
//!   steamworks converts `m_rgchConnect` with `CStr::from_bytes_until_nul(..).expect(..)` and
//!   `.to_str().expect(..)` (friends.rs:292-301), so a connect string without NUL or with bytes
//!   that are not UTF-8 (any friend can send one) panicked inside the pump. The guard puts a NUL in
//!   the last byte when there is none and replaces every invalid byte with `?` ([`sanitize_connect`];
//!   the length never changes). A game registering this type itself replaces the guard.
//! - **No Steam call after Steam's shutdown callback**: after the 103-OK the client's pipe is
//!   closing, while `SteamAPI_IsSteamRunning` still answers true.
//!   The guard that sees the ORIGINAL OK (102 or 103) raises the process-wide
//!   [`STEAM_SHUTTING_DOWN`] at once. Only the result OK does that; any other result code changes
//!   nothing. From that
//!   moment `gone()` is true: every feature accessor is `None`, `local_id` / `launch_command_line`
//!   answer from values read in `new`, `friend_name` is `""`, and the pump returns
//!   `BackendEvent::SteamLost` once (that frame's other events are dropped) and then nothing,
//!   without calling Steam. The `process_callbacks` run that delivered the 103 cannot be stopped
//!   from inside: steamworks finishes its dispatch (`GetNextCallback` / `FreeLastCallback`, the
//!   call results queued after it via `GetAPICallResult` and their closures, e.g.
//!   `GetDownloadedLeaderboardEntry`, and the game's own registered closures).
//! - `SteamAPI_Shutdown` is NOT called by the kit: steamworks calls it when the last `Client`
//!   clone drops (`Manager::drop`, lib.rs:565-572); the game usually holds its own clone, so an
//!   early shutdown by the kit would make every later call through that clone a use after
//!   shutdown, and the drop would shut down a second time. Once Steam is gone the kit LEAKS its
//!   own clone when the backend drops (`ManuallyDrop`), so the steamworks reference count never
//!   reaches zero and `SteamAPI_Shutdown` (a call on the dead pipe) never runs at exit.
//! - A client that ends WITHOUT the callback (killed, crashed):
//!   `crate::steam_process` asks the OPERATING SYSTEM (registry pid +
//!   process handle on Windows, `steam.pid` + `/proc` on Linux) in every `gone()`, i.e. before
//!   every pump and every feature access; ended -> the process-wide `STEAM_PROCESS_ENDED`,
//!   `SteamLost { SteamProcessEnded }`. Where that check is off (macOS, Flatpak / Snap, no pid):
//!   `SteamAPI_IsSteamRunning` (a process check) at most once a second, BEFORE the pump; false
//!   after true -> the same
//!   `SteamLost`. A `false` on the very first check switches it off (a sandbox where the process
//!   check cannot see the client must not read as "Steam quit").
//! - A pump that panics (only in an unwinding build) raises the process-wide
//!   [`STEAM_DISPATCH_BROKEN`] before the panic goes on to the core: steamworks is left in the
//!   middle of a callback, so no `RealSteamBackend` of this process pumps or calls Steam again.
//!   The lobby feature's `LobbyLeft` after such a `SteamLost` therefore does not leave the Steam
//!   lobby or clear the rich presence at Steam.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use steamworks::{sys, CallbackResult, Client, SteamId};

use crate::backend::{BackendEvent, SteamBackend, SteamLostReason};

pub(crate) type Queue = Arc<Mutex<Vec<BackendEvent>>>;

/// Push a call-result outcome for the next [`SteamBackend::pump`] to return. Used by every
/// feature with call results (`lobby`, `leaderboards`); features without one never call it.
#[cfg_attr(not(any(feature = "lobby", feature = "leaderboards")), allow(dead_code))]
pub(crate) fn push(queue: &Queue, ev: BackendEvent) {
    queue.lock().unwrap_or_else(|p| p.into_inner()).push(ev);
}

/// `true` when `s` contains a NUL byte: steamworks builds `CString::new(..).unwrap()` from its
/// string inputs, so such a string would PANIC inside steamworks. Every string input is checked.
/// Used by `lobby`, `auth`, `friends` and `overlay`.
#[cfg_attr(not(any(feature = "lobby", feature = "auth", feature = "friends", feature = "overlay")), allow(dead_code))]
pub(crate) fn has_nul(s: &str) -> bool {
    s.contains('\0')
}

/// How often the backend asks Steam whether the Steam client still runs.
const STEAM_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Set (never cleared) when Steam sent `SteamServersDisconnected` / `SteamServerConnectFailure`
/// with result OK: the Steam client is shutting down and its pipe is about to die. From then on
/// the kit makes NO Steam call of any kind (one Steam per process, so the flag is process-wide).
pub(crate) static STEAM_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Set (never cleared) when the operating system says the Steam client process has ended (killed
/// or crashed: no shutdown callback). From then on the kit makes NO Steam call of any kind.
pub(crate) static STEAM_PROCESS_ENDED: AtomicBool = AtomicBool::new(false);

/// Set (never cleared) when this backend's pump panicked: steamworks' dispatch is left in the
/// middle of a callback (no `FreeLastCallback`), so no `RealSteamBackend` of this process pumps or
/// calls Steam again.
pub(crate) static STEAM_DISPATCH_BROKEN: AtomicBool = AtomicBool::new(false);

/// Rewrite an `EResult` of `k_EResultOK` into `k_EResultNoConnection`, in place, and raise
/// `shutting_down` (the guards pass [`STEAM_SHUTTING_DOWN`]). Returns whether it was OK.
///
/// # Safety
/// `field` points to a readable and writable 4-byte `EResult` (any alignment).
unsafe fn ok_to_no_connection(field: *mut i32, shutting_down: &AtomicBool) -> bool {
    // SAFETY: the caller's contract. Read as a plain `i32` (the bindgen enum is `repr(i32)` on
    // Windows and `repr(u32)` elsewhere, values 0..=126: the same 4 bytes), so a value Steam adds
    // later is never materialised as an out-of-range Rust enum here.
    unsafe {
        if field.read_unaligned() == sys::EResult::k_EResultOK as i32 {
            field.write_unaligned(sys::EResult::k_EResultNoConnection as i32);
            shutting_down.store(true, Ordering::SeqCst);
            return true;
        }
    }
    false
}

/// Steam is gone for the whole process (shutting down, process ended, or a broken dispatch).
fn steam_gone_process_wide() -> bool {
    STEAM_SHUTTING_DOWN.load(Ordering::SeqCst) || STEAM_PROCESS_ENDED.load(Ordering::SeqCst) || STEAM_DISPATCH_BROKEN.load(Ordering::SeqCst)
}

/// The guard's work on a `SteamServersDisconnected_t` (tests pass their own flag).
///
/// # Safety
/// `raw` points to a live, writable `SteamServersDisconnected_t`.
unsafe fn guard_disconnected(raw: *mut c_void, shutting_down: &AtomicBool) -> bool {
    // SAFETY: the caller's contract; `addr_of_mut!` takes the field's address without a reference.
    unsafe { ok_to_no_connection(std::ptr::addr_of_mut!((*raw.cast::<sys::SteamServersDisconnected_t>()).m_eResult).cast::<i32>(), shutting_down) }
}

/// The guard's work on a `SteamServerConnectFailure_t` (tests pass their own flag).
///
/// # Safety
/// `raw` points to a live, writable `SteamServerConnectFailure_t`.
unsafe fn guard_connect_failure(raw: *mut c_void, shutting_down: &AtomicBool) -> bool {
    // SAFETY: as above.
    unsafe { ok_to_no_connection(std::ptr::addr_of_mut!((*raw.cast::<sys::SteamServerConnectFailure_t>()).m_eResult).cast::<i32>(), shutting_down) }
}

/// The guard for `SteamServersDisconnected` (id 103): see the module docs.
pub(crate) struct DisconnectGuard;

// SAFETY: `ID` is the id of `SteamServersDisconnected_t`, so steamworks calls `from_raw` only with
// a pointer to Steam's callback memory of that struct, valid and writable for the call (it is the
// buffer `SteamAPI_ManualDispatch_GetNextCallback` handed out, freed only after the dispatch).
unsafe impl steamworks::Callback for DisconnectGuard {
    const ID: i32 = sys::SteamServersDisconnected_t_k_iCallback as i32;

    unsafe fn from_raw(raw: *mut c_void) -> Self {
        // SAFETY: see the impl.
        unsafe { guard_disconnected(raw, &STEAM_SHUTTING_DOWN) };
        Self
    }
}

/// The guard for `SteamServerConnectFailure` (id 102): see the module docs.
pub(crate) struct ConnectFailureGuard;

// SAFETY: as for `DisconnectGuard`, with `SteamServerConnectFailure_t`.
unsafe impl steamworks::Callback for ConnectFailureGuard {
    const ID: i32 = sys::SteamServerConnectFailure_t_k_iCallback as i32;

    unsafe fn from_raw(raw: *mut c_void) -> Self {
        // SAFETY: see the impl.
        unsafe { guard_connect_failure(raw, &STEAM_SHUTTING_DOWN) };
        Self
    }
}

/// Make a connect-string buffer safe for steamworks' conversion, in place: a NUL inside it (the
/// last byte when there is none, cutting the string to 255 bytes), and every byte before the NUL
/// that is not part of valid UTF-8 replaced with `?`. The length never changes. Returns `true`
/// when something was changed.
pub(crate) fn sanitize_connect(buf: &mut [u8]) -> bool {
    let Some(last) = buf.len().checked_sub(1) else { return false };
    let mut changed = false;
    let end = match buf.iter().position(|&b| b == 0) {
        Some(end) => end,
        None => {
            buf[last] = 0;
            changed = true;
            last
        }
    };
    let mut pos = 0;
    while pos < end {
        match std::str::from_utf8(&buf[pos..end]) {
            Ok(_) => break,
            Err(e) => {
                let bad_start = pos + e.valid_up_to();
                let bad_len = e.error_len().unwrap_or(end - bad_start);
                buf[bad_start..bad_start + bad_len].fill(b'?');
                changed = true;
                pos = bad_start + bad_len;
            }
        }
    }
    changed
}

/// The guard for `GameRichPresenceJoinRequested` (id 337): see the module docs.
pub(crate) struct RichPresenceJoinGuard;

// SAFETY: `ID` is the id of `GameRichPresenceJoinRequested_t`; steamworks calls `from_raw` only
// with a pointer to Steam's callback memory of that struct, valid and writable for the call.
unsafe impl steamworks::Callback for RichPresenceJoinGuard {
    const ID: i32 = sys::GameRichPresenceJoinRequested_t_k_iCallback as i32;

    unsafe fn from_raw(raw: *mut c_void) -> Self {
        // SAFETY: see the impl; the field is a `[c_char; 256]` (alignment 1), so a byte slice over
        // it is valid; `addr_of_mut!` takes its address without a reference to the struct.
        unsafe {
            let field = std::ptr::addr_of_mut!((*raw.cast::<sys::GameRichPresenceJoinRequested_t>()).m_rgchConnect);
            let len = std::mem::size_of_val(&*field);
            sanitize_connect(std::slice::from_raw_parts_mut(field.cast::<u8>(), len));
        }
        Self
    }
}

/// The "is the Steam client still running" watch.
#[derive(Debug, Default)]
struct SteamWatch {
    last_check: Option<Instant>,
    /// `SteamAPI_IsSteamRunning` returned `true` at least once.
    seen_running: bool,
    /// The first check returned `false`: the check cannot see the client here, so it is off.
    check_off: bool,
}

/// The real Steam backend. Holds a `steamworks::Client` clone (which also keeps Steam alive).
pub struct RealSteamBackend {
    /// `ManuallyDrop`: once Steam is gone this clone is leaked on drop, so the kit never causes
    /// steamworks' `SteamAPI_Shutdown` (run when the last clone drops) on a dead Steam pipe.
    pub(crate) client: ManuallyDrop<Client>,
    pub(crate) queue: Queue,
    /// Found leaderboards by raw handle (steamworks' `Leaderboard` has no public constructor).
    #[cfg(feature = "leaderboards")]
    pub(crate) boards: crate::leaderboards::real::Boards,
    /// Requested Web API tickets by handle (steamworks' `AuthTicket` has no raw accessor).
    #[cfg(feature = "auth")]
    pub(crate) tickets: crate::auth::real::Tickets,
    /// Steam is gone and `SteamLost` was reported: never call Steam again.
    lost: AtomicBool,
    watch: Mutex<SteamWatch>,
    /// The Steam client process, watched through the operating system (`None`: no check here).
    process: Option<crate::steam_process::SteamProcess>,
    /// Read at creation, answered from here once Steam is gone (no Steam call then).
    local_id: u64,
    launch_command_line: String,
}

impl RealSteamBackend {
    /// Wrap an initialised Steam client. The caller keeps its own clone for other uses (and must
    /// never pump it: the kit pumps).
    ///
    /// This registers three steamworks callbacks, the kit's guards against panics inside
    /// steamworks 0.12.2: `SteamServersDisconnected` and `SteamServerConnectFailure` (they panic
    /// when Steam quits) and `GameRichPresenceJoinRequested` (it panics on a connect string that is
    /// not UTF-8; the guard replaces such bytes with `?`). The guards stay registered for the rest
    /// of the process (also after this backend drops). Do not register those three types yourself:
    /// steamworks keeps one closure per type, so a registration of yours made later replaces the
    /// kit's guard, one made earlier is replaced by the kit's (silently), and dropping your handle of
    /// it later removes the kit's guard. A typed closure for them panics in those cases anyway.
    ///
    /// When Steam reports that its client is shutting down, its process ends, or a pump panicked,
    /// this backend makes no Steam call any more, and neither does a `RealSteamBackend` created later
    /// in the same process (it makes no Steam call even in `new` and reports `SteamLost` on its first
    /// pump): see [`crate::SteamLost`].
    pub fn new(client: Client) -> Self {
        // Not Steam calls (steamworks' own closure map). The handles are forgotten: dropping one
        // removes the entry for its id whoever registered it last, so a dropped old backend would
        // remove a newer backend's guards. The closures are stateless no-ops; a later registration
        // replaces them with identical ones.
        std::mem::forget(client.register_callback(|_: DisconnectGuard| {}));
        std::mem::forget(client.register_callback(|_: ConnectFailureGuard| {}));
        std::mem::forget(client.register_callback(|_: RichPresenceJoinGuard| {}));
        if steam_gone_process_wide() {
            // Steam is already gone in this process: no Steam call, not even here.
            tracing::warn!(">>> STEAM: a RealSteamBackend was created after Steam was lost in this process - it stays inert");
            return Self::with(client, 0, String::new(), None);
        }
        let local_id = client.user().steam_id().raw();
        let launch_command_line = client.apps().launch_command_line();
        let process = match crate::steam_process::SteamProcess::find() {
            Ok(p) => Some(p),
            Err(why) => {
                tracing::warn!(">>> STEAM: no operating-system check of the Steam client process ({why}): a killed Steam client can hang the game");
                None
            }
        };
        Self::with(client, local_id, launch_command_line, process)
    }

    fn with(client: Client, local_id: u64, launch_command_line: String, process: Option<crate::steam_process::SteamProcess>) -> Self {
        Self {
            client: ManuallyDrop::new(client),
            queue: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "leaderboards")]
            boards: crate::leaderboards::real::Boards::default(),
            #[cfg(feature = "auth")]
            tickets: crate::auth::real::Tickets::default(),
            lost: AtomicBool::new(false),
            watch: Mutex::new(SteamWatch::default()),
            process,
            local_id,
            launch_command_line,
        }
    }

    /// No Steam call may be made: Steam is shutting down (the guard saw OK), its process has ended
    /// (asked from the operating system right now: no Steam call, never blocks), or it was found
    /// gone. Every Steam access of this backend asks this first.
    fn gone(&self) -> bool {
        if self.lost.load(Ordering::SeqCst) || steam_gone_process_wide() {
            return true;
        }
        if self.process.as_ref().is_some_and(|p| p.has_ended()) {
            STEAM_PROCESS_ENDED.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// The `IsSteamRunning` FALLBACK (a Steam client that ended without sending its shutdown
    /// callback, e.g. killed): `true` once the client that was running is gone. Asks at most once
    /// per [`STEAM_CHECK_INTERVAL`]; checked BEFORE the pump, so a dead client is not pumped.
    fn steam_process_gone(&self) -> bool {
        if self.process.is_some() {
            // The operating-system check (in `gone`) covers it, without any Steam call.
            return false;
        }
        let mut w = self.watch.lock().unwrap_or_else(|p| p.into_inner());
        if w.check_off {
            return false;
        }
        let now = Instant::now();
        if w.last_check.is_some_and(|t| now.duration_since(t) < STEAM_CHECK_INTERVAL) {
            return false;
        }
        w.last_check = Some(now);
        // SAFETY: a plain flat-API query without arguments (a process check, no pipe I/O).
        let running = unsafe { sys::SteamAPI_IsSteamRunning() };
        if running {
            w.seen_running = true;
            false
        } else if w.seen_running {
            true
        } else {
            w.check_off = true;
            tracing::warn!(
                ">>> STEAM: Steam reports its client as not running while the kit is connected - the Steam-exit fallback check is off in this environment"
            );
            false
        }
    }

    /// Mark Steam as lost (once) and return the event that reports it.
    fn report_lost(&self, why: &str, reason: SteamLostReason) -> Vec<BackendEvent> {
        if self.lost.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        tracing::warn!(">>> STEAM: {why} - the kit makes no Steam call from now on");
        vec![BackendEvent::SteamLost { reason }]
    }
}

impl Drop for RealSteamBackend {
    fn drop(&mut self) {
        if self.gone() {
            // Leak this clone: dropping the last one runs `SteamAPI_Shutdown`, a Steam call on a
            // dead pipe. The process is past Steam anyway.
            return;
        }
        // SAFETY: dropped exactly once, here, and never used afterwards.
        unsafe { ManuallyDrop::drop(&mut self.client) };
    }
}

impl SteamBackend for RealSteamBackend {
    fn local_id(&self) -> u64 {
        if self.gone() {
            return self.local_id;
        }
        self.client.user().steam_id().raw()
    }

    fn friend_name(&self, id: u64) -> String {
        if self.gone() {
            return String::new();
        }
        self.client.friends().get_friend(SteamId::from_raw(id)).name()
    }

    fn launch_command_line(&self) -> String {
        if self.gone() {
            return self.launch_command_line.clone();
        }
        self.client.apps().launch_command_line()
    }

    fn pump(&self) -> Vec<BackendEvent> {
        if self.lost.load(Ordering::SeqCst) {
            return Vec::new();
        }
        if STEAM_SHUTTING_DOWN.load(Ordering::SeqCst) {
            return self.report_lost("Steam is shutting down", SteamLostReason::SteamExited);
        }
        if STEAM_DISPATCH_BROKEN.load(Ordering::SeqCst) {
            return self.report_lost("an earlier pump of this process panicked", SteamLostReason::PumpPanicked);
        }
        if self.gone() || self.steam_process_gone() {
            STEAM_PROCESS_ENDED.store(true, Ordering::SeqCst);
            return self.report_lost("the Steam client process has ended (killed or crashed)", SteamLostReason::SteamProcessEnded);
        }
        // THE one pump. Call-result closures run INSIDE this call and push to `queue`, so the
        // queue must not be locked while it runs.
        let mut out = Vec::new();
        let mut servers_lost = false;
        // Read before the pump (never call Steam inside it): stats callbacks of another app
        // are ignored.
        #[cfg(feature = "stats")]
        let app_id = self.client.utils().app_id().0;
        let dispatch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.client.process_callbacks(|cb| {
                // Already made safe by the guard (an OK result reads as `NoConnection` here).
                if matches!(cb, CallbackResult::SteamServersDisconnected(_) | CallbackResult::SteamServerConnectFailure(_)) {
                    servers_lost = true;
                }
                // Every feature's mapper sees every callback (by reference). Mappers only convert;
                // they never call Steam.
                #[cfg(feature = "lobby")]
                out.extend(crate::lobby::real::map_callback(&cb));
                #[cfg(feature = "stats")]
                out.extend(crate::stats::real::map_callback(&cb, app_id));
                #[cfg(feature = "auth")]
                out.extend(crate::auth::real::map_callback(&cb, &self.tickets));
                #[cfg(feature = "friends")]
                out.extend(crate::friends::real::map_callback(&cb));
                #[cfg(feature = "overlay")]
                out.extend(crate::overlay::real::map_callback(&cb));
                #[cfg(not(any(feature = "lobby", feature = "stats", feature = "auth", feature = "friends", feature = "overlay")))]
                drop(cb);
            })
        }));
        if let Err(panic) = dispatch {
            // steamworks stopped in the middle of a callback: never dispatch again in this process.
            STEAM_DISPATCH_BROKEN.store(true, Ordering::SeqCst);
            self.lost.store(true, Ordering::SeqCst);
            std::panic::resume_unwind(panic);
        }
        if STEAM_SHUTTING_DOWN.load(Ordering::SeqCst) {
            // The guard saw Steam's shutdown in this very pump: this frame's events are dropped
            // (applying them would call Steam) and nothing else is touched.
            return self.report_lost("Steam is shutting down (its servers-disconnected callback said OK)", SteamLostReason::SteamExited);
        }
        if servers_lost {
            tracing::info!(">>> STEAM: Steam reported its servers disconnected (or unreachable); the Steam client still runs");
        }
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        out.append(&mut q);
        out
    }
    #[cfg(feature = "lobby")]
    fn lobby(&self) -> Option<&dyn crate::LobbyBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "stats")]
    fn stats(&self) -> Option<&dyn crate::StatsBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "leaderboards")]
    fn leaderboards(&self) -> Option<&dyn crate::LeaderboardBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "auth")]
    fn auth(&self) -> Option<&dyn crate::AuthBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "friends")]
    fn friends(&self) -> Option<&dyn crate::FriendsBackend> {
        (!self.gone()).then_some(self)
    }

    #[cfg(feature = "overlay")]
    fn overlay(&self) -> Option<&dyn crate::OverlayBackend> {
        (!self.gone()).then_some(self)
    }
}

#[cfg(test)]
mod tests {
    //! The shutdown guard on steamworks' own conversion (`CallbackResult::from_raw`), on callback
    //! memory built here: no Steam client is involved.

    use std::panic::{catch_unwind, AssertUnwindSafe};

    use steamworks::{Callback, SteamError, SteamServerConnectFailure, SteamServersDisconnected};

    use super::*;

    fn disconnected(result: sys::EResult) -> sys::SteamServersDisconnected_t {
        sys::SteamServersDisconnected_t { m_eResult: result }
    }

    #[test]
    fn the_guards_have_the_ids_of_the_two_callbacks() {
        assert_eq!(DisconnectGuard::ID, 103);
        assert_eq!(DisconnectGuard::ID, SteamServersDisconnected::ID);
        assert_eq!(ConnectFailureGuard::ID, 102);
        assert_eq!(ConnectFailureGuard::ID, SteamServerConnectFailure::ID);
    }

    #[test]
    fn without_the_guard_steamworks_panics_on_steams_shutdown_callback() {
        // The trap itself (steamworks 0.12.2): Steam sends `SteamServersDisconnected` with OK when
        // the client quits. If this test fails, steamworks stopped panicking here.
        let mut raw = disconnected(sys::EResult::k_EResultOK);
        let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
        // SAFETY: `p` points to a live `SteamServersDisconnected_t`, the struct of this id.
        let r = catch_unwind(AssertUnwindSafe(|| unsafe { CallbackResult::from_raw(DisconnectGuard::ID, p) }));
        assert!(r.is_err(), "steamworks 0.12.2 panics on k_EResultOK");
    }

    #[test]
    fn with_the_guard_the_shutdown_callback_converts_without_panicking() {
        let mut raw = disconnected(sys::EResult::k_EResultOK);
        let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
        // SAFETY: as above. The guard runs first on the same memory, as in `process_callbacks`.
        // A flag of the test's own: the process-wide one stays untouched in this test binary.
        let flag = AtomicBool::new(false);
        let cb = unsafe {
            assert!(guard_disconnected(p, &flag));
            CallbackResult::from_raw(DisconnectGuard::ID, p)
        };
        assert!(matches!(cb, Some(CallbackResult::SteamServersDisconnected(SteamServersDisconnected { reason: SteamError::NoConnection }))), "{cb:?}");
        assert!(flag.load(Ordering::SeqCst), "the OK raised the no-more-Steam-calls flag");
        assert!(!STEAM_SHUTTING_DOWN.load(Ordering::SeqCst));

        let mut raw = sys::SteamServerConnectFailure_t { m_eResult: sys::EResult::k_EResultOK, m_bStillRetrying: true };
        let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
        // SAFETY: `p` points to a live `SteamServerConnectFailure_t`, the struct of this id.
        let flag = AtomicBool::new(false);
        let cb = unsafe {
            assert!(guard_connect_failure(p, &flag));
            CallbackResult::from_raw(ConnectFailureGuard::ID, p)
        };
        assert!(
            matches!(cb, Some(CallbackResult::SteamServerConnectFailure(SteamServerConnectFailure { reason: SteamError::NoConnection, still_retrying: true }))),
            "{cb:?}"
        );
    }

    #[test]
    fn the_guard_leaves_every_other_result_as_steam_sent_it() {
        for (result, expected) in [
            (sys::EResult::k_EResultNoConnection, "NoConnection"),
            (sys::EResult::k_EResultLoggedInElsewhere, "LoggedInElsewhere"),
            (sys::EResult::k_EResultFail, "Generic"),
        ] {
            let mut raw = disconnected(result);
            let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
            // SAFETY: as above.
            let flag = AtomicBool::new(false);
            let cb = unsafe {
                assert!(!guard_disconnected(p, &flag), "not OK: no flag");
                CallbackResult::from_raw(DisconnectGuard::ID, p)
            };
            let Some(CallbackResult::SteamServersDisconnected(SteamServersDisconnected { reason })) = cb else { panic!("{cb:?}") };
            assert_eq!(format!("{reason:?}"), expected);
        }
        // An unknown value (not an `EResult`) is not touched either.
        let mut raw: i32 = 9_999;
        // SAFETY: a live, writable 4-byte value.
        let flag = AtomicBool::new(false);
        unsafe { ok_to_no_connection(&mut raw, &flag) };
        assert!(!flag.load(Ordering::SeqCst));
        assert_eq!(raw, 9_999);
    }

    fn join_request(connect: &[u8]) -> sys::GameRichPresenceJoinRequested_t {
        // SAFETY: an all-zero `GameRichPresenceJoinRequested_t` is valid (plain integers and bytes).
        let mut raw: sys::GameRichPresenceJoinRequested_t = unsafe { std::mem::zeroed() };
        for (dst, src) in raw.m_rgchConnect.iter_mut().zip(connect) {
            *dst = *src as std::ffi::c_char;
        }
        raw
    }

    #[test]
    fn without_the_guard_steamworks_panics_on_a_connect_string_that_is_not_utf8() {
        let mut raw = join_request(b"+connect \xff\xfe 1");
        let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
        // SAFETY: `p` points to a live `GameRichPresenceJoinRequested_t`, the struct of this id.
        let r = catch_unwind(AssertUnwindSafe(|| unsafe { CallbackResult::from_raw(RichPresenceJoinGuard::ID, p) }));
        assert!(r.is_err(), "steamworks 0.12.2 panics on non-UTF-8 connect strings");
    }

    #[test]
    fn with_the_guard_a_bad_connect_string_converts_with_question_marks() {
        assert_eq!(RichPresenceJoinGuard::ID, 337);
        assert_eq!(RichPresenceJoinGuard::ID, steamworks::GameRichPresenceJoinRequested::ID);
        for (bytes, expected) in [
            (&b"+connect \xff\xfe 1"[..], "+connect ?? 1".to_string()),
            (&b"ok \xc3\xa9 \xe2\x82"[..], "ok \u{e9} ??".to_string()),
            (&[b'a'; 256][..], "a".repeat(255)),
            (&b"+connect_lobby 42"[..], "+connect_lobby 42".to_string()),
        ] {
            let mut raw = join_request(bytes);
            let p = std::ptr::addr_of_mut!(raw).cast::<c_void>();
            // SAFETY: as above; the guard runs first on the same memory, as in `process_callbacks`.
            let cb = unsafe {
                RichPresenceJoinGuard::from_raw(p);
                CallbackResult::from_raw(RichPresenceJoinGuard::ID, p)
            };
            let Some(CallbackResult::GameRichPresenceJoinRequested(j)) = cb else { panic!("{cb:?}") };
            assert_eq!(j.connect, expected);
        }
    }

    #[test]
    fn sanitize_connect_keeps_valid_strings_and_the_length() {
        let mut ok = *b"+connect_lobby 7\0garbage\xff";
        assert!(!sanitize_connect(&mut ok), "valid up to the NUL: untouched");
        assert_eq!(&ok, b"+connect_lobby 7\0garbage\xff");
        let mut bad = *b"\xff\0";
        assert!(sanitize_connect(&mut bad));
        assert_eq!(&bad, b"?\0");
        let mut no_nul = *b"ab";
        assert!(sanitize_connect(&mut no_nul));
        assert_eq!(&no_nul, b"a\0");
        assert!(!sanitize_connect(&mut []));
    }
}
