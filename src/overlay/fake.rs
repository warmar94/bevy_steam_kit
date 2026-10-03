//! The overlay half of [`FakeSteamBackend`] (feature `overlay`).

use super::backend::OverlayBackend;
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};
use crate::OpenOverlay;

/// The fake's overlay state, stored inside the shared fake state.
#[derive(Debug)]
pub(crate) struct FakeOverlayState {
    enabled: bool,
    refuse_next: bool,
}

impl Default for FakeOverlayState {
    fn default() -> Self {
        Self { enabled: true, refuse_next: false }
    }
}

/// Overlay controls (feature `overlay`). Every request the kit passes on is recorded as
/// [`FakeCall::ActivateOverlay`].
impl FakeSteamBackend {
    /// Whether Steam reports the overlay as available (default `true`).
    pub fn set_overlay_enabled(&self, enabled: bool) {
        self.lock().overlay.enabled = enabled;
    }

    /// Queue an "overlay opened" (`true`) or "closed" (`false`) event, as Steam sends when the
    /// player presses Shift+Tab.
    pub fn toggle_overlay(&self, active: bool) {
        self.lock().queued.push(BackendEvent::OverlayActivated { active });
    }

    /// The backend refuses the next overlay call (`OverlayErrorKind::Refused`).
    pub fn refuse_next_overlay(&self) {
        self.lock().overlay.refuse_next = true;
    }
}

impl OverlayBackend for FakeSteamBackend {
    fn overlay_enabled(&self) -> bool {
        self.lock().overlay.enabled
    }

    fn open_overlay(&self, request: &OpenOverlay) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::ActivateOverlay(request.clone()));
        !std::mem::take(&mut s.overlay.refuse_next)
    }
}
