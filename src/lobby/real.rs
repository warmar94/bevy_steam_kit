//! The lobby half of [`RealSteamBackend`] (features `steam` + `lobby`), over `steamworks` 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `GameLobbyJoinRequested` / `GameRichPresenceJoinRequested` are mapped by
//!   `CallbackResult::from_raw` (callback.rs:67, :76), so the kit's one `process_callbacks` pump
//!   sees them without a `register_callback` handle. `LobbyChatUpdate` is deliberately never
//!   touched (its conversion `unreachable!()`s on unknown states in 0.12.2).
//! - `create_lobby` `assert!(max_members <= 250)` (matchmaking.rs:90) -> we clamp to `1..=250`.
//! - `lobby_data` / `set_lobby_data` / `set_rich_presence` / `invite_user_to_game` build
//!   `CString::new(..).unwrap()` -> a string with an interior NUL would PANIC inside steamworks;
//!   every such input is rejected here first.
//! - `invite_user_to_game` returns `()` (friends.rs:470): our `bool` is "the call was made".

use steamworks::{CallbackResult, LobbyId, LobbyType, SteamId};

use super::backend::{LobbyBackend, LobbyKind};
use crate::backend::BackendEvent;
use crate::real::{has_nul, push, RealSteamBackend};

/// The lobby callbacks of one `process_callbacks` run, as backend events.
pub(crate) fn map_callback(cb: &CallbackResult) -> Option<BackendEvent> {
    match cb {
        CallbackResult::GameLobbyJoinRequested(r) => Some(BackendEvent::LobbyJoinRequested { lobby: r.lobby_steam_id.raw(), from: r.friend_steam_id.raw() }),
        CallbackResult::GameRichPresenceJoinRequested(r) => {
            Some(BackendEvent::RichPresenceJoinRequested { from: r.friend_steam_id.raw(), connect: r.connect.clone() })
        }
        _ => None,
    }
}

impl LobbyBackend for RealSteamBackend {
    fn create_lobby(&self, kind: LobbyKind, max_members: u32) {
        let ty = match kind {
            LobbyKind::Private => LobbyType::Private,
            LobbyKind::FriendsOnly => LobbyType::FriendsOnly,
            LobbyKind::Public => LobbyType::Public,
            LobbyKind::Invisible => LobbyType::Invisible,
        };
        let queue = self.queue.clone();
        self.client.matchmaking().create_lobby(ty, max_members.clamp(1, super::MAX_LOBBY_MEMBERS), move |res| {
            let ev = match res {
                Ok(id) => BackendEvent::LobbyCreated { lobby: id.raw() },
                Err(e) => BackendEvent::LobbyCreateFailed { message: e.to_string() },
            };
            push(&queue, ev);
        });
    }

    fn join_lobby(&self, lobby: u64) {
        let queue = self.queue.clone();
        self.client.matchmaking().join_lobby(LobbyId::from_raw(lobby), move |res| {
            let ev = match res {
                Ok(id) => BackendEvent::LobbyEntered { lobby: id.raw() },
                Err(()) => BackendEvent::LobbyJoinFailed { lobby },
            };
            push(&queue, ev);
        });
    }

    fn leave_lobby(&self, lobby: u64) {
        self.client.matchmaking().leave_lobby(LobbyId::from_raw(lobby));
    }

    fn set_lobby_data(&self, lobby: u64, key: &str, value: &str) -> bool {
        if has_nul(key) || has_nul(value) {
            return false;
        }
        self.client.matchmaking().set_lobby_data(LobbyId::from_raw(lobby), key, value)
    }

    fn lobby_data(&self, lobby: u64, key: &str) -> Option<String> {
        if has_nul(key) {
            return None;
        }
        self.client.matchmaking().lobby_data(LobbyId::from_raw(lobby), key).map(|s| s.to_string())
    }

    fn lobby_member_count(&self, lobby: u64) -> usize {
        self.client.matchmaking().lobby_member_count(LobbyId::from_raw(lobby))
    }

    fn set_lobby_joinable(&self, lobby: u64, joinable: bool) -> bool {
        self.client.matchmaking().set_lobby_joinable(LobbyId::from_raw(lobby), joinable)
    }

    fn set_rich_presence(&self, key: &str, value: Option<&str>) -> bool {
        if has_nul(key) || value.is_some_and(has_nul) {
            return false;
        }
        self.client.friends().set_rich_presence(key, value)
    }

    fn clear_rich_presence(&self) {
        self.client.friends().clear_rich_presence();
    }

    fn invite_to_game(&self, friend: u64, connect: &str) -> bool {
        if has_nul(connect) {
            return false;
        }
        self.client.friends().get_friend(SteamId::from_raw(friend)).invite_user_to_game(connect);
        true
    }
}
