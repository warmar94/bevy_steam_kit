//! The auth half of [`RealSteamBackend`] (features `steam` + `auth`), over `steamworks` 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `User::authentication_session_ticket_for_webapi(identity)` (user.rs:142-150) calls
//!   `GetAuthTicketForWebApi` and builds `CString::new(identity).unwrap()`: a NUL byte would
//!   PANIC, so it is refused first. It returns an `AuthTicket` handle (user.rs:241-242:
//!   `Copy + PartialEq`, no constructor, no raw accessor).
//! - The answer is the callback `TicketForWebApiResponse { ticket_handle, result, ticket_len,
//!   ticket }` (user.rs:284-302), which `CallbackResult::from_raw` knows (callback.rs:104-106): the
//!   kit's one pump sees it, no `register_callback`. `ticket` is the WHOLE fixed array
//!   (`m_rgubTicket[2560]`, SDK `isteamuser.h`); only `ticket[..ticket_len]` is the ticket
//!   ([`ticket_bytes`] clamps the length).
//! - Requests are matched to callbacks by `AuthTicket` equality through [`Tickets`] (our own
//!   mutex; the mapper never calls Steam). A callback for a handle the kit did not request (a
//!   ticket the game requested on its own client clone) is ignored, so both can coexist.
//! - `cancel_authentication_ticket` (user.rs:69-73) = `CancelAuthTicket`.
//! - The steamworks callback struct derives `Debug` with the ticket bytes in it: the kit never
//!   formats it.

use std::sync::{Arc, Mutex};

use steamworks::{AuthTicket, CallbackResult};

use super::backend::{AuthBackend, WebApiTicket};
use crate::backend::BackendEvent;
use crate::real::{has_nul, RealSteamBackend};

/// Requested tickets: (op, Steam's handle).
#[derive(Clone, Default)]
pub(crate) struct Tickets(Arc<Mutex<Vec<(u64, AuthTicket)>>>);

impl Tickets {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(u64, AuthTicket)>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn op_of(&self, handle: AuthTicket) -> Option<u64> {
        self.lock().iter().find(|(_, t)| *t == handle).map(|(op, _)| *op)
    }
    fn take(&self, op: u64) -> Option<AuthTicket> {
        let mut v = self.lock();
        let i = v.iter().position(|(o, _)| *o == op)?;
        Some(v.swap_remove(i).1)
    }
}

/// The first `len` bytes of Steam's fixed ticket buffer, `len` clamped to `0..=buffer.len()`.
pub(crate) fn ticket_bytes(buffer: &[u8], len: i32) -> Vec<u8> {
    let n = usize::try_from(len).unwrap_or(0).min(buffer.len());
    buffer[..n].to_vec()
}

/// `true` for the `Debug` text of steamworks' `AuthTicket(k_HAuthTicketInvalid)`.
pub(crate) fn is_invalid_handle(debug: &str) -> bool {
    debug == "AuthTicket(0)"
}

/// The auth callbacks of one `process_callbacks` run, as backend events. Converts only; never
/// calls Steam (it locks only the kit's own ticket table).
pub(crate) fn map_callback(cb: &CallbackResult, tickets: &Tickets) -> Option<BackendEvent> {
    let CallbackResult::TicketForWebApiResponse(r) = cb else { return None };
    let op = tickets.op_of(r.ticket_handle)?;
    let result = match &r.result {
        Ok(()) => Ok(WebApiTicket::new(ticket_bytes(&r.ticket, r.ticket_len))),
        Err(e) => Err(e.to_string()),
    };
    Some(BackendEvent::WebApiTicket { op, result })
}

impl AuthBackend for RealSteamBackend {
    fn request_web_api_ticket(&self, op: u64, identity: &str) -> bool {
        if has_nul(identity) {
            return false;
        }
        let handle = self.client.user().authentication_session_ticket_for_webapi(identity);
        // `k_HAuthTicketInvalid` (0): Steam refused at once (e.g. not logged on); no callback
        // follows. `AuthTicket` has no raw accessor; its derived `Debug` prints the raw handle.
        if is_invalid_handle(&format!("{handle:?}")) {
            return false;
        }
        self.tickets.0.lock().unwrap_or_else(|p| p.into_inner()).push((op, handle));
        true
    }

    fn cancel_auth_ticket(&self, op: u64) {
        if let Some(handle) = self.tickets.take(op) {
            self.client.user().cancel_authentication_ticket(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ticket_bytes;

    #[test]
    fn only_the_first_ticket_len_bytes_are_the_ticket() {
        let buffer: Vec<u8> = (0..2560u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(ticket_bytes(&buffer, 4), vec![0, 1, 2, 3]);
        assert!(ticket_bytes(&buffer, 0).is_empty());
        assert!(ticket_bytes(&buffer, -1).is_empty());
        assert!(ticket_bytes(&buffer, i32::MIN).is_empty());
        assert_eq!(ticket_bytes(&buffer, 2560).len(), 2560);
        assert_eq!(ticket_bytes(&buffer, 2561).len(), 2560);
        assert_eq!(ticket_bytes(&buffer, i32::MAX).len(), 2560);
    }

    /// steamworks' `AuthTicket` is `#[derive(Debug)] struct AuthTicket(HAuthTicket = u32)`.
    #[test]
    fn the_invalid_handle_is_recognised_by_its_debug_text() {
        #[derive(Debug)]
        struct AuthTicket(#[allow(dead_code)] u32);
        assert!(super::is_invalid_handle(&format!("{:?}", AuthTicket(0))));
        assert!(!super::is_invalid_handle(&format!("{:?}", AuthTicket(7))));
    }
}
