//! The overlay half of [`RealSteamBackend`] (features `steam` + `overlay`), over `steamworks`
//! 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `Friends::activate_game_overlay`, `_to_web_page`, `_to_user` and
//!   `activate_invite_dialog_connect_string` (friends.rs:123-196) build `CString::new(..).unwrap()`:
//!   a NUL byte would PANIC. The kit refuses those requests first and this backend checks again.
//!   `activate_game_overlay_to_store(AppId, OverlayToStoreFlag)` and `activate_invite_dialog(LobbyId)`
//!   take no strings. None of them returns anything.
//! - `GameOverlayActivated { active }` (friends.rs:255-265) is in `CallbackResult::from_raw`
//!   (callback.rs:70): the kit's one pump sees it.
//! - `Utils::is_overlay_enabled` (utils.rs:146) = `IsOverlayEnabled`.

use steamworks::{AppId, CallbackResult, LobbyId, OverlayToStoreFlag, SteamId};

use super::backend::{OverlayBackend, StoreFlag};
use crate::backend::BackendEvent;
use crate::real::{has_nul, RealSteamBackend};
use crate::OpenOverlay;

/// The overlay callbacks of one `process_callbacks` run, as backend events. Converts only.
pub(crate) fn map_callback(cb: &CallbackResult) -> Option<BackendEvent> {
    match cb {
        CallbackResult::GameOverlayActivated(r) => Some(BackendEvent::OverlayActivated { active: r.active }),
        _ => None,
    }
}

impl OverlayBackend for RealSteamBackend {
    fn overlay_enabled(&self) -> bool {
        self.client.utils().is_overlay_enabled()
    }

    fn open_overlay(&self, request: &OpenOverlay) -> bool {
        let friends = self.client.friends();
        match request {
            OpenOverlay::Dialog { dialog } if !has_nul(dialog) => friends.activate_game_overlay(dialog),
            OpenOverlay::User { dialog, steam_id } if !has_nul(dialog) => friends.activate_game_overlay_to_user(dialog, SteamId::from_raw(*steam_id)),
            OpenOverlay::WebPage { url } if !has_nul(url) => friends.activate_game_overlay_to_web_page(url),
            OpenOverlay::Store { app_id, flag } => {
                let flag = match flag {
                    StoreFlag::None => OverlayToStoreFlag::None,
                    StoreFlag::AddToCart => OverlayToStoreFlag::AddToCart,
                    StoreFlag::AddToCartAndShow => OverlayToStoreFlag::AddToCartAndShow,
                };
                friends.activate_game_overlay_to_store(AppId(*app_id), flag);
            }
            OpenOverlay::InviteDialog { lobby } => friends.activate_invite_dialog(LobbyId::from_raw(*lobby)),
            OpenOverlay::InviteDialogConnect { connect } if !has_nul(connect) => friends.activate_invite_dialog_connect_string(connect),
            _ => return false,
        }
        true
    }
}
