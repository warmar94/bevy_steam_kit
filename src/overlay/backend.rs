//! The overlay half of the backend seam: [`OverlayBackend`], reached through
//! [`SteamBackend::overlay`](crate::SteamBackend::overlay).

/// What the store page does with the app besides showing it. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StoreFlag {
    /// Only show the page.
    #[default]
    None,
    /// Add the app to the cart.
    AddToCart,
    /// Add the app to the cart and show the cart.
    AddToCartAndShow,
}

/// Everything the overlay feature needs from Steam. Implementations must never panic; the kit
/// validates every request before calling (see [`OverlayErrorKind::InvalidRequest`](crate::OverlayErrorKind::InvalidRequest)).
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added to this trait comes with a default implementation.
pub trait OverlayBackend {
    /// Steam reports the overlay as available to this process.
    fn overlay_enabled(&self) -> bool;
    /// Open the overlay as requested. `true` = the call was made (Steam gives no answer).
    fn open_overlay(&self, request: &crate::OpenOverlay) -> bool;
}
