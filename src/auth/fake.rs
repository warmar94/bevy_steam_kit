//! The auth half of [`FakeSteamBackend`] (feature `auth`).

use std::collections::BTreeMap;

use super::backend::{AuthBackend, WebApiTicket};
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};

/// How the fake's next ticket request goes wrong (see
/// [`FakeSteamBackend::fail_next_auth_ticket`]). `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FakeAuthFailure {
    /// The backend refuses the request (`request_web_api_ticket` returns `false`).
    Refused,
    /// Steam answers with a failure.
    Failed,
    /// The request is made but no answer ever arrives.
    NoAnswer,
}

/// The fake's auth state, stored inside the shared fake state.
#[derive(Debug, Default)]
pub(crate) struct FakeAuthState {
    /// Tickets requested and not cancelled: op -> identity.
    live: BTreeMap<u64, String>,
    next_failure: Option<FakeAuthFailure>,
}

/// Auth controls (feature `auth`).
impl FakeSteamBackend {
    /// The bytes of the fake ticket for backend operation `op`: `b"FAKE-TICKET-<op>"`. The kit
    /// numbers its ticket requests 1, 2, 3, ... in the order it sends them.
    pub fn fake_web_api_ticket(op: u64) -> Vec<u8> {
        format!("FAKE-TICKET-{op}").into_bytes()
    }

    /// Make the next ticket request fail in the given way.
    pub fn fail_next_auth_ticket(&self, failure: FakeAuthFailure) {
        self.lock().auth.next_failure = Some(failure);
    }

    /// The identities of every ticket requested and not cancelled (pending or delivered), in
    /// request order.
    pub fn live_auth_tickets(&self) -> Vec<String> {
        self.lock().auth.live.values().cloned().collect()
    }
}

impl AuthBackend for FakeSteamBackend {
    fn request_web_api_ticket(&self, op: u64, identity: &str) -> bool {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::RequestWebApiTicket { identity: identity.to_string() });
        if identity.contains('\0') {
            return false;
        }
        match s.auth.next_failure.take() {
            Some(FakeAuthFailure::Refused) => return false,
            Some(FakeAuthFailure::NoAnswer) => {}
            Some(FakeAuthFailure::Failed) => s.queued.push(BackendEvent::WebApiTicket { op, result: Err("Fail".into()) }),
            None => s.queued.push(BackendEvent::WebApiTicket { op, result: Ok(WebApiTicket::new(Self::fake_web_api_ticket(op))) }),
        }
        s.auth.live.insert(op, identity.to_string());
        true
    }

    fn cancel_auth_ticket(&self, op: u64) {
        let mut s = self.lock();
        s.calls.push(FakeCall::CancelAuthTicket { op });
        s.auth.live.remove(&op);
    }
}
