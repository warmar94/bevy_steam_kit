//! The auth half of the backend seam: [`AuthBackend`], reached through
//! [`SteamBackend::auth`](crate::SteamBackend::auth), and the ticket type [`WebApiTicket`].

use std::fmt;

/// A Steam Web API authentication ticket (`GetAuthTicketForWebApi`): the bytes a game server
/// passes to Steam's `ISteamUserAuth/AuthenticateUserTicket` to learn who the player is.
///
/// It is a credential. `Debug` prints only its length (`WebApiTicket(234 bytes)`), there is no
/// `Display`, and the kit never logs it. Send [`to_hex`](Self::to_hex) to your server and nowhere
/// else. The bytes of this value are overwritten with zeros when it is dropped; copies you make
/// (such as the hex string) are yours to handle.
#[derive(Clone, PartialEq, Eq)]
pub struct WebApiTicket(Vec<u8>);

impl WebApiTicket {
    /// Wrap ticket bytes (for custom backends; the kit's backends build it for you).
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The raw ticket bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// The ticket as lowercase hex, two characters per byte: the form Steam's Web API takes
    /// (`ticket=<hex>`).
    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(self.0.len() * 2);
        for b in &self.0 {
            s.push(HEX[usize::from(b >> 4)] as char);
            s.push(HEX[usize::from(b & 0x0f)] as char);
        }
        s
    }

    /// The ticket's length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The ticket has no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for WebApiTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WebApiTicket({} bytes)", self.0.len())
    }
}

impl Drop for WebApiTicket {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: `b` is a valid, aligned, exclusive reference into the vector's buffer.
            // A volatile write keeps the compiler from removing the zeroing as a dead store.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

/// Everything the auth feature needs from Steam. Implementations must never panic.
///
/// [`request_web_api_ticket`](Self::request_web_api_ticket) returns whether the request was made;
/// its outcome arrives as a [`BackendEvent::WebApiTicket`](crate::BackendEvent) carrying the same
/// `op` from a following [`SteamBackend::pump`](crate::SteamBackend::pump). Nothing inside the pump may
/// call Steam.
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added to this trait comes with a default implementation.
pub trait AuthBackend {
    /// Ask Steam for a Web API ticket for `identity` (the name of the service that checks it).
    /// `false` = refused (for example an identity with a NUL byte).
    fn request_web_api_ticket(&self, op: u64, identity: &str) -> bool;
    /// Cancel the ticket of `op` (pending or delivered). Unknown ops are ignored.
    fn cancel_auth_ticket(&self, op: u64);
}
