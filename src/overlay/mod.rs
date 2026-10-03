//! Feature `overlay`: open the Steam overlay (a dialog, a user's page, a web page, the store page,
//! the invite dialog) and learn when it opens and closes.
//!
//! The game sends [`OpenOverlay`]; the kit checks it (no empty or NUL strings, a valid SteamID64,
//! a lobby id, a connect string within Steam's limit) and passes it to Steam. An unusable request
//! is an [`OverlayError`]; Steam gives no answer for an accepted one. Every time the overlay opens
//! or closes the kit writes [`OverlayToggled`] (Steam's `GameOverlayActivated`, through the one
//! pump) and updates [`SteamOverlay`]; a game typically pauses while it is open.
//!
//! The overlay needs Steam's "Enable the Steam Overlay while in-game" setting on and a game window
//! Steam draws into; [`SteamOverlay::is_enabled`] says whether Steam made it available (reported
//! shortly after start).
//!
//! Inert until a [`SteamBackendRes`] whose backend supports the overlay exists; requests are
//! answered with [`OverlayErrorKind::NoBackend`] until then. Schedules: [`OverlayToggled`] in
//! [`SteamKitSystems::Callbacks`] (`First`); requests in [`SteamKitSystems::Requests`] (`Update`).

mod backend;
pub(crate) mod fake;
#[cfg(feature = "steam")]
pub(crate) mod real;
#[cfg(test)]
mod tests;

pub use backend::{OverlayBackend, StoreFlag};

use bevy_app::{App, First, Update};
use bevy_ecs::prelude::*;
use tracing::{info, warn};

use crate::{is_individual_steam_id64, BackendEvent, PumpedEvents, SteamBackendRes, SteamKitSystems};

/// Steam's limit on an invite dialog's connect string (a rich-presence value: 255 bytes of text).
const MAX_CONNECT_BYTES: usize = 255;

/// The overlay's state. Written only by the kit; read it through its methods.
#[derive(Resource, Debug, Default)]
pub struct SteamOverlay {
    active: bool,
    enabled: bool,
    toggles: u32,
}

impl SteamOverlay {
    /// The overlay is open now.
    pub fn is_active(&self) -> bool {
        self.active
    }
    /// Steam reports the overlay as available to this process (read every frame a backend
    /// exists; `false` without one).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    /// How often the overlay opened or closed so far (wrapping).
    pub fn toggles(&self) -> u32 {
        self.toggles
    }
}

/// Open the Steam overlay. `#[non_exhaustive]`: build the variants (or use the helpers), match
/// with a `_` arm.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenOverlay {
    /// A Steam dialog by name. Valve's names: `"friends"`, `"community"`, `"players"`,
    /// `"settings"`, `"officialgamegroup"`, `"stats"`, `"achievements"`.
    Dialog {
        /// The dialog name.
        dialog: String,
    },
    /// A dialog about one user. Valve's names: `"steamid"` (the profile), `"chat"`,
    /// `"jointrade"`, `"stats"`, `"achievements"`, `"friendadd"`, `"friendremove"`,
    /// `"friendrequestaccept"`, `"friendrequestignore"`.
    User {
        /// The dialog name.
        dialog: String,
        /// The user's SteamID64 (an individual account).
        steam_id: u64,
    },
    /// A web page in the overlay's browser (a full URL, e.g. `https://...`).
    WebPage {
        /// The URL.
        url: String,
    },
    /// An app's store page (your own app id for "buy the full game", a DLC's for an upsell).
    Store {
        /// The app (or DLC) id.
        app_id: u32,
        /// Add it to the cart too.
        flag: StoreFlag,
    },
    /// The invite dialog for a Steam lobby (friends invited there get a lobby invite).
    InviteDialog {
        /// The lobby's raw id (not 0).
        lobby: u64,
    },
    /// The invite dialog sending a connect string (accepted invites arrive as the `friends`
    /// feature's `ConnectRequested`, or on the command line of a game Steam starts).
    InviteDialogConnect {
        /// The connect string: 1..=255 bytes, no NUL.
        connect: String,
    },
}

impl OpenOverlay {
    /// [`OpenOverlay::Dialog`].
    pub fn dialog(dialog: impl Into<String>) -> Self {
        Self::Dialog { dialog: dialog.into() }
    }
    /// [`OpenOverlay::User`].
    pub fn user(dialog: impl Into<String>, steam_id: u64) -> Self {
        Self::User { dialog: dialog.into(), steam_id }
    }
    /// [`OpenOverlay::WebPage`].
    pub fn web_page(url: impl Into<String>) -> Self {
        Self::WebPage { url: url.into() }
    }
    /// [`OpenOverlay::Store`] without adding to the cart.
    pub fn store(app_id: u32) -> Self {
        Self::Store { app_id, flag: StoreFlag::None }
    }
    /// [`OpenOverlay::InviteDialog`].
    pub fn invite_dialog(lobby: u64) -> Self {
        Self::InviteDialog { lobby }
    }
    /// [`OpenOverlay::InviteDialogConnect`].
    pub fn invite_dialog_connect(connect: impl Into<String>) -> Self {
        Self::InviteDialogConnect { connect: connect.into() }
    }
}

/// The overlay opened or closed (Steam's `GameOverlayActivated`). `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayToggled {
    /// The overlay is open now.
    pub active: bool,
}

/// What went wrong in an [`OverlayError`]. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OverlayErrorKind {
    /// No [`SteamBackendRes`], or its backend does not support the overlay.
    NoBackend,
    /// An empty name / URL / connect string, a NUL byte, a connect string over 255 bytes, a
    /// SteamID64 that is not a user, or lobby id 0.
    InvalidRequest,
    /// The backend refused the call.
    Refused,
}

/// An [`OpenOverlay`] could not be passed to Steam. `#[non_exhaustive]`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayError {
    /// The request.
    pub request: OpenOverlay,
    /// Category.
    pub kind: OverlayErrorKind,
    /// Human-readable detail.
    pub message: String,
}

/// Called by [`SteamKitPlugin::build`](crate::SteamKitPlugin).
pub(crate) fn build(app: &mut App) {
    app.init_resource::<SteamOverlay>()
        .add_message::<OpenOverlay>()
        .add_message::<OverlayToggled>()
        .add_message::<OverlayError>()
        .add_systems(First, apply_overlay_events.in_set(SteamKitSystems::Callbacks))
        .add_systems(Update, handle_overlay_requests.in_set(SteamKitSystems::Requests));
}

/// `First` ([`SteamKitSystems::Callbacks`]): overlay open / closed.
fn apply_overlay_events(pumped: Res<PumpedEvents>, mut state: ResMut<SteamOverlay>, mut toggled: MessageWriter<OverlayToggled>) {
    for ev in &pumped.0 {
        // Irrefutable when `overlay` is the only feature with events.
        #[allow(irrefutable_let_patterns)]
        let &BackendEvent::OverlayActivated { active } = ev
        else {
            continue;
        };
        state.active = active;
        state.toggles = state.toggles.wrapping_add(1);
        info!(">>> STEAM: overlay {}", if active { "opened" } else { "closed" });
        toggled.write(OverlayToggled { active });
    }
}

/// Why a request cannot go to Steam (`None` = it can).
fn invalid(req: &OpenOverlay) -> Option<&'static str> {
    let bad = |s: &str| s.is_empty() || s.contains('\0');
    match req {
        OpenOverlay::Dialog { dialog } if bad(dialog) => Some("empty dialog name or a NUL byte"),
        OpenOverlay::User { dialog, .. } if bad(dialog) => Some("empty dialog name or a NUL byte"),
        OpenOverlay::User { steam_id, .. } if !is_individual_steam_id64(*steam_id) => Some("not a SteamID64"),
        OpenOverlay::WebPage { url } if bad(url) => Some("empty URL or a NUL byte"),
        OpenOverlay::InviteDialog { lobby: 0 } => Some("lobby id 0"),
        OpenOverlay::InviteDialogConnect { connect } if bad(connect) || connect.len() > MAX_CONNECT_BYTES => {
            Some("connect string must be 1..=255 bytes without NUL")
        }
        _ => None,
    }
}

/// `Update` ([`SteamKitSystems::Requests`]): refresh "enabled", then pass every request to Steam.
/// Without a backend the overlay counts as closed (one `OverlayToggled { active: false }` if it
/// was open, so a game paused on it resumes).
fn handle_overlay_requests(
    backend: Option<Res<SteamBackendRes>>,
    mut state: ResMut<SteamOverlay>,
    mut requests: MessageReader<OpenOverlay>,
    mut errors: MessageWriter<OverlayError>,
    mut toggled: MessageWriter<OverlayToggled>,
) {
    let Some(api) = backend.as_ref().and_then(|b| b.0.overlay()) else {
        state.enabled = false;
        if state.active {
            state.active = false;
            state.toggles = state.toggles.wrapping_add(1);
            info!(">>> STEAM: overlay closed (backend removed)");
            toggled.write(OverlayToggled { active: false });
        }
        for req in requests.read() {
            warn!(">>> STEAM: overlay request: NoBackend");
            errors.write(OverlayError { request: req.clone(), kind: OverlayErrorKind::NoBackend, message: "overlay: Steam is not available".into() });
        }
        return;
    };
    let enabled = api.overlay_enabled();
    if enabled != state.enabled {
        state.enabled = enabled;
    }
    for req in requests.read() {
        if let Some(why) = invalid(req) {
            warn!(">>> STEAM: overlay request: InvalidRequest ({why})");
            errors.write(OverlayError { request: req.clone(), kind: OverlayErrorKind::InvalidRequest, message: why.into() });
            continue;
        }
        if !enabled {
            warn!(">>> STEAM: overlay requested while Steam reports it unavailable");
        }
        if api.open_overlay(req) {
            info!(">>> STEAM: overlay request sent");
        } else {
            warn!(">>> STEAM: overlay request: Refused");
            errors.write(OverlayError { request: req.clone(), kind: OverlayErrorKind::Refused, message: "the backend refused the overlay call".into() });
        }
    }
}
