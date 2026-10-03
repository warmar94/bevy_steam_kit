//! The lobby half of [`FakeSteamBackend`] (feature `lobby`).

use std::collections::HashMap;

use super::backend::{LobbyBackend, LobbyKind};
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};

/// The fake's lobby state, stored inside the shared fake state.
#[derive(Debug)]
pub(crate) struct FakeLobbyState {
    auto_complete_create: bool,
    join_succeeds: bool,
    next_lobby: u64,
    lobby_data: HashMap<(u64, String), String>,
    member_counts: HashMap<u64, usize>,
    rich_presence: HashMap<String, String>,
}

impl Default for FakeLobbyState {
    fn default() -> Self {
        Self {
            auto_complete_create: true,
            join_succeeds: true,
            next_lobby: 1000,
            lobby_data: HashMap::new(),
            member_counts: HashMap::new(),
            rich_presence: HashMap::new(),
        }
    }
}

/// Lobby controls (feature `lobby`).
impl FakeSteamBackend {
    /// `false`: `create_lobby` stays pending until [`complete_create`](Self::complete_create) /
    /// [`fail_create`](Self::fail_create).
    pub fn set_auto_complete_create(&self, auto: bool) {
        self.lock().lobby.auto_complete_create = auto;
    }

    /// Make the following `join_lobby` calls fail (`false`) or succeed (`true`).
    pub fn set_join_succeeds(&self, ok: bool) {
        self.lock().lobby.join_succeeds = ok;
    }

    /// Complete a pending create with `lobby` (delivered on the next pump).
    pub fn complete_create(&self, lobby: u64) {
        self.lock().queued.push(BackendEvent::LobbyCreated { lobby });
    }

    /// Fail a pending create (delivered on the next pump).
    pub fn fail_create(&self, message: &str) {
        self.lock().queued.push(BackendEvent::LobbyCreateFailed { message: message.to_string() });
    }

    /// Set a lobby's member count as seen by `lobby_member_count`.
    pub fn set_member_count(&self, lobby: u64, count: usize) {
        self.lock().lobby.member_counts.insert(lobby, count);
    }

    /// Pre-set lobby metadata (as if another host had set it).
    pub fn put_lobby_data(&self, lobby: u64, key: &str, value: &str) {
        self.lock().lobby.lobby_data.insert((lobby, key.to_string()), value.to_string());
    }

    /// Current rich presence value for `key`.
    pub fn rich_presence(&self, key: &str) -> Option<String> {
        self.lock().lobby.rich_presence.get(key).cloned()
    }
}

impl LobbyBackend for FakeSteamBackend {
    fn create_lobby(&self, kind: LobbyKind, max_members: u32) {
        let mut guard = self.lock();
        let s = &mut *guard;
        s.calls.push(FakeCall::CreateLobby { kind, max_members });
        if s.lobby.auto_complete_create {
            let lobby = s.lobby.next_lobby;
            s.lobby.next_lobby = s.lobby.next_lobby.wrapping_add(1).max(1);
            s.lobby.member_counts.insert(lobby, 1);
            s.queued.push(BackendEvent::LobbyCreated { lobby });
        }
    }

    fn join_lobby(&self, lobby: u64) {
        let mut s = self.lock();
        s.calls.push(FakeCall::JoinLobby(lobby));
        let ev = if s.lobby.join_succeeds { BackendEvent::LobbyEntered { lobby } } else { BackendEvent::LobbyJoinFailed { lobby } };
        s.queued.push(ev);
    }

    fn leave_lobby(&self, lobby: u64) {
        self.lock().calls.push(FakeCall::LeaveLobby(lobby));
    }

    fn set_lobby_data(&self, lobby: u64, key: &str, value: &str) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::SetLobbyData { lobby, key: key.to_string(), value: value.to_string() });
        s.lobby.lobby_data.insert((lobby, key.to_string()), value.to_string());
        true
    }

    fn lobby_data(&self, lobby: u64, key: &str) -> Option<String> {
        self.lock().lobby.lobby_data.get(&(lobby, key.to_string())).cloned()
    }

    fn lobby_member_count(&self, lobby: u64) -> usize {
        self.lock().lobby.member_counts.get(&lobby).copied().unwrap_or(0)
    }

    fn set_lobby_joinable(&self, lobby: u64, joinable: bool) -> bool {
        self.lock().calls.push(FakeCall::SetLobbyJoinable { lobby, joinable });
        true
    }

    fn set_rich_presence(&self, key: &str, value: Option<&str>) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::SetRichPresence { key: key.to_string(), value: value.map(str::to_string) });
        match value {
            Some(v) => s.lobby.rich_presence.insert(key.to_string(), v.to_string()),
            None => s.lobby.rich_presence.remove(key),
        };
        true
    }

    fn clear_rich_presence(&self) {
        let mut s = self.lock();
        s.calls.push(FakeCall::ClearRichPresence);
        s.lobby.rich_presence.clear();
    }

    fn invite_to_game(&self, friend: u64, connect: &str) -> bool {
        self.lock().calls.push(FakeCall::InviteToGame { friend, connect: connect.to_string() });
        true
    }
}
