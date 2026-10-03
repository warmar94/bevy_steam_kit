//! The friends half of [`RealSteamBackend`] (features `steam` + `friends`), over `steamworks`
//! 0.12.2.
//!
//! Verified against the locked `steamworks-0.12.2` source:
//! - `Friend::state()` (friends.rs:352-366) PANICS (`unreachable!()`) on any persona state it does
//!   not map, e.g. `Invisible = 7`: NEVER called. The state is read through the raw
//!   `SteamAPI_ISteamFriends_GetFriendPersonaState` instead, for friends AND for the local user (its
//!   own id). `GetPersonaState` is NOT used: it returns
//!   Online whatever the user's status is, while `GetFriendPersonaState(own id)` returns the real
//!   one (Away, Invisible, ...). The bindgen return type `EPersonaState` is a Rust enum
//!   (`#[repr(i32)]` on Windows, `#[repr(u32)]` on Linux / macOS; values 0..=8); a value outside
//!   it received AS that enum would be undefined behaviour. So the kit declares the C function
//!   itself with the C return type `int` (`c_int`; the SDK's `EPersonaState` is a plain C enum)
//!   in [`raw`], links it by name (the
//!   library is linked by `steamworks-sys`), and maps the value with a total function
//!   ([`PersonaState::from_raw`]). No transmute. `catch_unwind` is no guard (a `panic = "abort"`
//!   build aborts).
//! - `Friend::name()` / `nick_name()` (:328-350) are lossy UTF-8 and never panic;
//!   `Friend::game_played()` (:369-384) is safe; `get_friends(IMMEDIATE)` (:71-89) too.
//! - A friend's rich presence is NOT wrapped: raw `GetFriendRichPresence(friends, id, key)`
//!   (bindings, identical on Windows / Linux / macOS) returns `""` when unset. Steam shares the
//!   rich presence of friends in the same game without `RequestFriendRichPresence`.
//!   `FriendRichPresenceUpdate_t` is not in `CallbackResult::from_raw`: the kit polls.
//! - `PersonaStateChange` (:242-253, `from_bits_truncate`, safe) and
//!   `GameRichPresenceJoinRequested` (:281-309) are in `CallbackResult::from_raw` (callback.rs:76,
//!   :88). The latter converts the connect string with `expect`: a non-UTF-8 (or unterminated)
//!   string would panic inside the pump. The core's guard (`crate::real`, `RichPresenceJoinGuard`)
//!   replaces invalid bytes with `?` in place before that conversion, so it never panics.
//! - `Friend::invite_user_to_game` (:470-479) discards Steam's result (and builds
//!   `CString::new(..).unwrap()`): the kit calls the raw `SteamAPI_ISteamFriends_InviteUserToGame
//!   -> bool` (identical on all three bindings) after its NUL / length checks, so
//!   `GameInviteSent.ok` is Steam's own answer. `request_user_information` (:117-121) returns
//!   Steam's bool.
//! - `small/medium/large_avatar()` (:398-460) return `width * height * 4` RGBA bytes of a fixed
//!   size (32 / 64 / 184) or `None`; `AvatarImageLoaded_t` is not delivered, the kit retries.

use std::ffi::{c_char, CStr, CString};
use std::net::Ipv4Addr;

use steamworks::{sys, CallbackResult, FriendFlags, SteamId};

use super::backend::{AvatarSize, FriendGame, FriendsBackend, PersonaState, MAX_CONNECT_BYTES};
use crate::backend::BackendEvent;
use crate::real::{has_nul, RealSteamBackend};

/// The friends callbacks of one `process_callbacks` run, as backend events. Converts only.
pub(crate) fn map_callback(cb: &CallbackResult) -> Option<BackendEvent> {
    match cb {
        CallbackResult::PersonaStateChange(r) => Some(BackendEvent::PersonaChanged { steam_id: r.steam_id.raw(), flags: r.flags.bits() as u32 }),
        CallbackResult::GameRichPresenceJoinRequested(r) => Some(BackendEvent::ConnectRequested { from: r.friend_steam_id.raw(), connect: r.connect.clone() }),
        _ => None,
    }
}

/// The friends interface (null when Steam is not initialised).
fn friends_ptr() -> *mut sys::ISteamFriends {
    // SAFETY: a plain accessor; the kit only calls it while `RealSteamBackend` holds a live
    // `Client` (steamworks' own `friends()` calls the same function).
    unsafe { sys::SteamAPI_SteamFriends_v018() }
}

/// The flat-API function with its C return type `int` (the SDK's `EPersonaState` is a C enum, so
/// an `int` at the C level), declared here instead of through the bindgen enum.
mod raw {
    use std::ffi::c_int;

    use steamworks::sys;

    extern "C" {
        #[link_name = "SteamAPI_ISteamFriends_GetFriendPersonaState"]
        pub(super) fn get_friend_persona_state(friends: *mut sys::ISteamFriends, id: sys::uint64_steamid) -> c_int;
    }
}

/// A user's persona state as the raw value Steam returns (`GetFriendPersonaState`; for the local
/// user too, with its own id).
fn raw_state(id: u64) -> i32 {
    let friends = friends_ptr();
    if friends.is_null() {
        return 0;
    }
    // SAFETY: the declaration matches the SDK 1.62 C signature (`EPersonaState` is a C enum = `int`
    // in the C ABI); the interface pointer is non-null and valid while the client lives. An
    // unknown value is just an `int`, never an out-of-range Rust enum.
    unsafe { raw::get_friend_persona_state(friends, id) }
}

/// The local user's state: read like a friend's, with the local user's own id
/// (`GetPersonaState` returns Online whatever the status is).
fn own_state(local_id: u64, state_of: impl FnOnce(u64) -> i32) -> PersonaState {
    PersonaState::from_raw(state_of(local_id))
}

impl FriendsBackend for RealSteamBackend {
    fn current_app_id(&self) -> u32 {
        self.client.utils().app_id().0
    }

    fn friend_ids(&self) -> Vec<u64> {
        self.client.friends().get_friends(FriendFlags::IMMEDIATE).iter().map(|f| f.id().raw()).collect()
    }

    fn persona_name(&self, id: u64) -> String {
        self.client.friends().get_friend(SteamId::from_raw(id)).name()
    }

    fn persona_nickname(&self, id: u64) -> Option<String> {
        self.client.friends().get_friend(SteamId::from_raw(id)).nick_name()
    }

    fn persona_state(&self, id: u64) -> PersonaState {
        PersonaState::from_raw(raw_state(id))
    }

    fn game_played(&self, id: u64) -> Option<FriendGame> {
        let g = self.client.friends().get_friend(SteamId::from_raw(id)).game_played()?;
        let server = (g.game_address != Ipv4Addr::UNSPECIFIED && g.game_port != 0).then_some((g.game_address, g.game_port));
        Some(FriendGame::new(g.game.app_id().0, g.lobby.raw(), server))
    }

    fn friend_rich_presence(&self, id: u64, key: &str) -> Option<String> {
        if has_nul(key) {
            return None;
        }
        let key = CString::new(key).ok()?;
        let friends = friends_ptr();
        if friends.is_null() {
            return None;
        }
        // SAFETY: non-null interface; `key` outlives the call; Steam returns a pointer to a
        // NUL-terminated string it owns (or null), read at once and copied.
        let value = unsafe {
            let p: *const c_char = sys::SteamAPI_ISteamFriends_GetFriendRichPresence(friends, id, key.as_ptr());
            if p.is_null() {
                return None;
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        (!value.is_empty()).then_some(value)
    }

    fn local_persona(&self) -> (String, PersonaState) {
        (self.client.friends().name(), own_state(self.client.user().steam_id().raw(), raw_state))
    }

    fn invite_user_to_game(&self, id: u64, connect: &str) -> bool {
        if has_nul(connect) || connect.is_empty() || connect.len() > MAX_CONNECT_BYTES {
            return false;
        }
        let Ok(connect) = CString::new(connect) else { return false };
        let friends = friends_ptr();
        if friends.is_null() {
            return false;
        }
        // SAFETY: non-null interface valid while the client lives; `connect` outlives the call;
        // the signature is the bindgen one (identical on Windows / Linux / macOS).
        unsafe { sys::SteamAPI_ISteamFriends_InviteUserToGame(friends, id, connect.as_ptr()) }
    }

    fn request_user_information(&self, id: u64, name_only: bool) -> bool {
        self.client.friends().request_user_information(SteamId::from_raw(id), name_only)
    }

    fn friend_avatar(&self, id: u64, size: AvatarSize) -> Option<(u32, u32, Vec<u8>)> {
        let friend = self.client.friends().get_friend(SteamId::from_raw(id));
        let rgba = match size {
            AvatarSize::Small => friend.small_avatar(),
            AvatarSize::Medium => friend.medium_avatar(),
            AvatarSize::Large => friend.large_avatar(),
        }?;
        let px = size.pixels();
        Some((px, px, rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_raw_persona_state_maps_without_panicking() {
        let all = [
            (0, PersonaState::Offline),
            (1, PersonaState::Online),
            (2, PersonaState::Busy),
            (3, PersonaState::Away),
            (4, PersonaState::Snooze),
            (5, PersonaState::LookingToTrade),
            (6, PersonaState::LookingToPlay),
            (7, PersonaState::Invisible),
            (8, PersonaState::Unknown),
            (-1, PersonaState::Unknown),
            (i32::MAX, PersonaState::Unknown),
        ];
        for (raw, state) in all {
            assert_eq!(PersonaState::from_raw(raw), state, "raw {raw}");
        }
        // The bindgen enum's values are the ones mapped above.
        assert_eq!(sys::EPersonaState::k_EPersonaStateInvisible as i32, 7);
        assert_eq!(sys::EPersonaState::k_EPersonaStateLookingToPlay as i32, 6);
        assert_eq!(sys::EPersonaState::k_EPersonaStateMax as i32, 8);
    }

    #[test]
    fn the_local_users_state_is_read_with_its_own_id() {
        // Steam's answers as seen live for the local user: GetPersonaState said Online every time,
        // GetFriendPersonaState(own id) said Away / Invisible / Online.
        const ME: u64 = 76_561_197_960_265_729;
        for (raw, expected) in [(3, PersonaState::Away), (7, PersonaState::Invisible), (1, PersonaState::Online), (2, PersonaState::Busy)] {
            let mut asked = None;
            let state = own_state(ME, |id| {
                asked = Some(id);
                raw
            });
            assert_eq!(asked, Some(ME), "the own id is the one read");
            assert_eq!(state, expected);
        }
    }

    #[test]
    fn the_callback_mapper_ignores_other_callbacks() {
        let cb = CallbackResult::GameOverlayActivated(steamworks::GameOverlayActivated { active: true });
        assert_eq!(map_callback(&cb), None);
    }
}
