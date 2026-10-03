//! Feature `auth`: Steam Web API tickets, so a game server can learn who the player is.
//!
//! The game sends [`AuthRequest::WebApiTicket`] with an [`AuthRequestId`] of its choice and an
//! identity (the name of the service that checks the ticket, agreed with that service). Steam
//! answers asynchronously; the kit writes exactly one answer per accepted request:
//! [`WebApiTicketReady`] (the ticket, as bytes or lowercase hex) or [`AuthError`]. The one
//! exception, as for leaderboards: a request whose id is still in use is rejected with
//! [`AuthErrorKind::DuplicateId`] and the earlier request keeps its own answer.
//!
//! A delivered ticket stays valid at Steam until it is cancelled ([`AuthRequest::Cancel`], usually
//! right after the server answered) or the session ends. On `AppExit` the kit cancels every live
//! ticket ([`AuthSettings::cancel_on_exit`]) and answers waiting requests with
//! [`AuthErrorKind::Exiting`].
//!
//! **Secrets:** the ticket is a credential. [`WebApiTicket`]'s `Debug` prints only its length, it
//! has no `Display`, and the kit's log lines name the identity and the length, never the bytes.
//!
//! Configured with [`SteamKitPlugin::with_auth`](crate::SteamKitPlugin::with_auth)
//! ([`AuthSettings`]). Inert until a [`SteamBackendRes`] whose backend supports auth exists; until
//! then requests are answered with [`AuthErrorKind::NoBackend`].
//!
//! Schedules: tickets arriving from Steam are applied in [`SteamKitSystems::Callbacks`] (`First`);
//! requests and timeouts in [`SteamKitSystems::Requests`] (`Update`); exit handling in
//! [`SteamKitSystems::Requests`] (`Last`). Timeouts use `Time<Real>`; without Bevy's `TimePlugin`
//! there are none.

mod backend;
pub(crate) mod fake;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use backend::{AuthBackend, WebApiTicket};
pub use fake::FakeAuthFailure;

use std::collections::HashMap;
use std::time::Duration;

use bevy_app::{App, AppExit, First, Last, Update};
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_time::{Real, Time};
use tracing::{info, warn};

use crate::{BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

// ---------------------------------------------------------------------------------------------
// Settings + state
// ---------------------------------------------------------------------------------------------

/// The auth feature's settings, given with
/// [`SteamKitPlugin::with_auth`](crate::SteamKitPlugin::with_auth) and also inserted as a resource
/// by the plugin (read-only). Build it with `..Default::default()`.
#[derive(Resource, Clone, Debug)]
pub struct AuthSettings {
    /// A ticket request without an answer after this long is given up and cancelled
    /// ([`AuthErrorKind::TimedOut`]). Default 30 s.
    pub timeout: Duration,
    /// Cancel every live ticket on `AppExit`. Default `true`. A ticket can only be cancelled
    /// through the backend: when [`SteamBackendRes`] is removed the kit forgets its live tickets
    /// without cancelling them (they stay valid at Steam until the Steam session ends), so cancel
    /// them before removing the backend.
    pub cancel_on_exit: bool,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(30), cancel_on_exit: true }
    }
}

/// The game's id for one ticket request; the answer carries the same id, and
/// [`AuthRequest::Cancel`] names the ticket by it.
///
/// Get one from [`SteamAuth::next_id`] (unique among the ids in use), or pick it by hand: keep it
/// unique while its ticket is pending or live, and when mixing with `next_id` stay at or above
/// [`AuthRequestId::FIRST_MANUAL`] (the kit never issues those).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AuthRequestId(pub u64);

impl AuthRequestId {
    /// The first id the kit never issues (`2^63`).
    pub const FIRST_MANUAL: u64 = 1 << 63;
}

/// A ticket request waiting for Steam.
#[derive(Debug)]
struct Pending {
    op: u64,
    identity: String,
    started: Option<Duration>,
}

/// The auth feature's state. Written only by the kit; read it through its methods.
#[derive(Resource, Debug, Default)]
pub struct SteamAuth {
    next_op: u64,
    pending: HashMap<AuthRequestId, Pending>,
    /// Delivered tickets not cancelled yet: id -> op.
    live: HashMap<AuthRequestId, u64>,
    last_issued: u64,
    /// ONE read position in the request stream, shared by the `Update` and the `Last` system.
    cursor: MessageCursor<AuthRequest>,
}

impl SteamAuth {
    /// A request with this id is waiting for Steam's answer.
    pub fn is_pending(&self, id: AuthRequestId) -> bool {
        self.pending.contains_key(&id)
    }

    /// The ticket of this id was delivered and is not cancelled.
    pub fn is_live(&self, id: AuthRequestId) -> bool {
        self.live.contains_key(&id)
    }

    /// Delivered tickets that are not cancelled.
    pub fn live_tickets(&self) -> usize {
        self.live.len()
    }

    /// A fresh request id: increasing, in `1..FIRST_MANUAL` (wrapping back to 1 at the top), never
    /// an id that is pending or live. Call it from a system ordered
    /// `.before(SteamKitSystems::Requests)` (it takes `ResMut<SteamAuth>`).
    pub fn next_id(&mut self) -> AuthRequestId {
        loop {
            self.last_issued = if self.last_issued + 1 >= AuthRequestId::FIRST_MANUAL { 1 } else { self.last_issued + 1 };
            let id = AuthRequestId(self.last_issued);
            if !self.in_use(id) {
                return id;
            }
        }
    }

    fn in_use(&self, id: AuthRequestId) -> bool {
        self.pending.contains_key(&id) || self.live.contains_key(&id)
    }

    fn op(&mut self) -> u64 {
        self.next_op = self.next_op.wrapping_add(1).max(1);
        self.next_op
    }
}

// ---------------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------------

/// An auth request; one ordered stream (a request and a cancel written in one frame are applied
/// in that order). `#[non_exhaustive]`: build the variants as usual, match with a `_` arm.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthRequest {
    /// Ask Steam for a Web API ticket. Answer: [`WebApiTicketReady`] or [`AuthError`].
    WebApiTicket {
        /// Request id.
        id: AuthRequestId,
        /// The service that checks the ticket (agreed with it, for example `"my-game-server"`).
        /// Must not contain a NUL byte. Valve recommends one identity per service.
        identity: String,
    },
    /// Cancel the ticket of `id`: a live ticket is cancelled at Steam (no answer); a pending
    /// request is cancelled and answered with [`AuthErrorKind::Cancelled`]; an unknown id is
    /// ignored.
    Cancel {
        /// The id of the request whose ticket to cancel.
        id: AuthRequestId,
    },
}

impl AuthRequest {
    /// [`AuthRequest::WebApiTicket`].
    pub fn web_api_ticket(id: AuthRequestId, identity: impl Into<String>) -> Self {
        Self::WebApiTicket { id, identity: identity.into() }
    }
    /// [`AuthRequest::Cancel`].
    pub fn cancel(id: AuthRequestId) -> Self {
        Self::Cancel { id }
    }
    /// This request's id.
    pub fn id(&self) -> AuthRequestId {
        match self {
            Self::WebApiTicket { id, .. } | Self::Cancel { id } => *id,
        }
    }
}

/// A ticket arrived. `#[non_exhaustive]`: read it, the kit writes it. Its `Debug` shows the
/// ticket's length only.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebApiTicketReady {
    /// The request's id.
    pub id: AuthRequestId,
    /// The identity it was requested for.
    pub identity: String,
    /// The ticket: send `ticket.to_hex()` to your server.
    pub ticket: WebApiTicket,
}

/// What went wrong in an [`AuthError`]. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AuthErrorKind {
    /// No [`SteamBackendRes`] (or its backend does not support auth), or it was removed while the
    /// request waited.
    NoBackend,
    /// The identity contains a NUL byte (`steamworks` would panic on it).
    InvalidIdentity,
    /// A request with this id is still pending or its ticket is live. This error answers only
    /// the rejected request.
    DuplicateId,
    /// Steam answered with a failure (its text is in `message`), or the backend refused the call.
    Failed,
    /// No answer within [`AuthSettings::timeout`]; the request was cancelled at Steam.
    TimedOut,
    /// The game cancelled the request ([`AuthRequest::Cancel`]) before the ticket arrived.
    Cancelled,
    /// The app exited while the request waited.
    Exiting,
}

/// A ticket request failed. `#[non_exhaustive]`: read it, the kit writes it.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AuthError {
    /// The request's id.
    pub id: AuthRequestId,
    /// Category.
    pub kind: AuthErrorKind,
    /// Human-readable detail.
    pub message: String,
}

// ---------------------------------------------------------------------------------------------
// Plugin wiring + systems
// ---------------------------------------------------------------------------------------------

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App, settings: &AuthSettings) {
    app.insert_resource(settings.clone())
        .init_resource::<SteamAuth>()
        .add_message::<AuthRequest>()
        .add_message::<WebApiTicketReady>()
        .add_message::<AuthError>()
        .add_systems(First, apply_auth_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_auth_requests.in_set(SteamKitSystems::Requests))
        .add_systems(Last, auth_on_exit.in_set(SteamKitSystems::Requests));
}

#[derive(SystemParam)]
struct AuthOut<'w> {
    ready: MessageWriter<'w, WebApiTicketReady>,
    error: MessageWriter<'w, AuthError>,
}

impl AuthOut<'_> {
    fn fail(&mut self, id: AuthRequestId, kind: AuthErrorKind, message: impl Into<String>) {
        let message = message.into();
        warn!(">>> STEAM: web API ticket request {}: {kind:?} ({message})", id.0);
        self.error.write(AuthError { id, kind, message });
    }
}

fn auth_api<'a>(backend: &'a Option<Res<'_, SteamBackendRes>>) -> Option<&'a dyn AuthBackend> {
    backend.as_ref().and_then(|b| b.0.auth())
}

/// `First` ([`SteamKitSystems::Callbacks`]): tickets that arrived -> answers. Reads no clock.
fn apply_auth_events(backend: Option<Res<SteamBackendRes>>, pumped: Res<PumpedEvents>, mut state: ResMut<SteamAuth>, mut out: AuthOut) {
    let Some(api) = auth_api(&backend) else {
        let lost = pumped.0.iter().filter(|e| matches!(e, BackendEvent::WebApiTicket { .. })).count();
        if lost > 0 && backend.is_none() {
            // The requests they answer are failed with `NoBackend` in `Update`.
            warn!(">>> STEAM: backend removed before {lost} web API ticket(s) were applied - dropped");
        }
        return;
    };
    for ev in &pumped.0 {
        // Irrefutable when `auth` is the only feature with events.
        #[allow(irrefutable_let_patterns)]
        let BackendEvent::WebApiTicket { op, result } = ev
        else {
            continue;
        };
        let Some(id) = state.pending.iter().find(|(_, p)| p.op == *op).map(|(id, _)| *id) else {
            // A ticket the game gave up on (cancelled / timed out): it was already cancelled.
            info!(">>> STEAM: late web API ticket for a given-up request - dropped");
            continue;
        };
        let Some(p) = state.pending.remove(&id) else { continue };
        match result {
            Ok(ticket) => {
                info!(">>> STEAM: web API ticket for identity {:?} ready ({} bytes)", p.identity, ticket.len());
                state.live.insert(id, p.op);
                out.ready.write(WebApiTicketReady { id, identity: p.identity, ticket: ticket.clone() });
            }
            Err(message) => {
                api.cancel_auth_ticket(p.op);
                out.fail(id, AuthErrorKind::Failed, format!("Steam: {message}"));
            }
        }
    }
}

/// `Update` ([`SteamKitSystems::Requests`]): timeouts, then new requests in order.
fn handle_auth_requests(
    backend: Option<Res<SteamBackendRes>>,
    time: Option<Res<Time<Real>>>,
    settings: Res<AuthSettings>,
    messages: Res<Messages<AuthRequest>>,
    mut state: ResMut<SteamAuth>,
    mut out: AuthOut,
) {
    let requests: Vec<AuthRequest> = state.cursor.read(&messages).cloned().collect();
    let Some(api) = auth_api(&backend) else {
        // The backend is gone (or never came): nothing pending will be answered by Steam.
        let mut gone: Vec<AuthRequestId> = state.pending.drain().map(|(id, _)| id).collect();
        gone.sort();
        for id in gone {
            out.fail(id, AuthErrorKind::NoBackend, "Steam backend removed while this request waited");
        }
        state.live.clear();
        for req in requests {
            if let AuthRequest::WebApiTicket { id, .. } = req {
                out.fail(id, AuthErrorKind::NoBackend, "auth: Steam is not available");
            }
        }
        return;
    };
    let now = time.as_ref().map(|t| t.elapsed());

    if let Some(now) = now {
        for p in state.pending.values_mut() {
            p.started.get_or_insert(now);
        }
        let mut expired: Vec<AuthRequestId> =
            state.pending.iter().filter(|(_, p)| p.started.is_some_and(|t| now.saturating_sub(t) >= settings.timeout)).map(|(id, _)| *id).collect();
        expired.sort();
        for id in expired {
            if let Some(p) = state.pending.remove(&id) {
                api.cancel_auth_ticket(p.op);
                out.fail(id, AuthErrorKind::TimedOut, "no ticket from Steam in time (cancelled)");
            }
        }
    }

    for req in requests {
        apply_request(api, now, &mut state, &mut out, req);
    }
}

fn apply_request(api: &dyn AuthBackend, now: Option<Duration>, state: &mut SteamAuth, out: &mut AuthOut, req: AuthRequest) {
    match req {
        AuthRequest::WebApiTicket { id, identity } => {
            if state.in_use(id) {
                out.fail(id, AuthErrorKind::DuplicateId, "rejected: this id is still pending or its ticket is live (that one keeps its own answer)");
                return;
            }
            if identity.contains('\0') {
                out.fail(id, AuthErrorKind::InvalidIdentity, "the identity contains a NUL byte");
                return;
            }
            let op = state.op();
            if api.request_web_api_ticket(op, &identity) {
                info!(">>> STEAM: web API ticket requested for identity {identity:?}");
                state.pending.insert(id, Pending { op, identity, started: now });
            } else {
                out.fail(id, AuthErrorKind::Failed, "the backend refused the ticket request");
            }
        }
        AuthRequest::Cancel { id } => {
            if let Some(op) = state.live.remove(&id) {
                api.cancel_auth_ticket(op);
                info!(">>> STEAM: web API ticket {} cancelled", id.0);
            } else if let Some(p) = state.pending.remove(&id) {
                api.cancel_auth_ticket(p.op);
                out.fail(id, AuthErrorKind::Cancelled, "cancelled before the ticket arrived");
            }
        }
    }
}

/// `Last` ([`SteamKitSystems::Requests`]): on `AppExit`, answer every waiting request (also those
/// written this frame after the `Update` set) with [`AuthErrorKind::Exiting`], cancelling them,
/// and cancel every live ticket when [`AuthSettings::cancel_on_exit`].
fn auth_on_exit(
    mut exit: MessageReader<AppExit>,
    backend: Option<Res<SteamBackendRes>>,
    settings: Res<AuthSettings>,
    messages: Res<Messages<AuthRequest>>,
    mut state: ResMut<SteamAuth>,
    mut out: AuthOut,
) {
    if exit.read().count() == 0 {
        return;
    }
    let late: Vec<AuthRequest> = state.cursor.read(&messages).cloned().collect();
    let api = auth_api(&backend);
    // Late cancels still apply (a ticket the game is done with); late requests are not sent.
    for req in late {
        match req {
            AuthRequest::WebApiTicket { id, .. } => {
                let kind = if api.is_some() { AuthErrorKind::Exiting } else { AuthErrorKind::NoBackend };
                out.fail(id, kind, "the app exited before this request was sent");
            }
            AuthRequest::Cancel { id } => {
                if let Some(api) = api {
                    apply_request(api, None, &mut state, &mut out, AuthRequest::Cancel { id });
                }
            }
        }
    }
    let mut pending: Vec<(AuthRequestId, u64)> = state.pending.drain().map(|(id, p)| (id, p.op)).collect();
    pending.sort();
    for (id, op) in pending {
        if let Some(api) = api {
            api.cancel_auth_ticket(op);
        }
        out.fail(id, AuthErrorKind::Exiting, "the app exited before Steam answered");
    }
    if settings.cancel_on_exit {
        let live: Vec<u64> = state.live.drain().map(|(_, op)| op).collect();
        if let Some(api) = api {
            for op in &live {
                api.cancel_auth_ticket(*op);
            }
        }
        if !live.is_empty() {
            info!(">>> STEAM: {} web API ticket(s) cancelled on exit", live.len());
        }
    }
}
