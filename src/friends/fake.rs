//! The friends half of [`FakeSteamBackend`] (feature `friends`). Names come from the core's
//! [`FakeSteamBackend::set_friend_name`]. The leaderboards fake's `set_friends` (who counts for
//! `ScoreRange::Friends`) is separate from this list.

use std::collections::{BTreeMap, HashMap};

use super::backend::{AvatarSize, FriendGame, FriendsBackend, PersonaState};
use crate::backend::BackendEvent;
use crate::fake::{FakeCall, FakeSteamBackend};

const NAME: u32 = 0x0001;
const STATUS: u32 = 0x0002;
const GAME_PLAYED: u32 = 0x0010;
const AVATAR: u32 = 0x0040;
const RELATIONSHIP: u32 = 0x0200;
const NICKNAME: u32 = 0x1000;

#[derive(Debug, Default)]
struct FakeFriend {
    state: PersonaState,
    nickname: Option<String>,
    game: Option<FriendGame>,
    rich_presence: HashMap<String, String>,
}

/// The fake's friends state, stored inside the shared fake state.
#[derive(Debug)]
pub(crate) struct FakeFriendsState {
    friends: BTreeMap<u64, FakeFriend>,
    local_name: String,
    local_state: PersonaState,
    avatars: HashMap<u64, (u32, u32, Vec<u8>)>,
    /// Users whose avatar is "loading" (`friend_avatar` returns `None`).
    pub(crate) avatars_loading: Vec<u64>,
    /// Users whose info is "already loaded" (`request_user_information` returns `false`).
    known_users: Vec<u64>,
    /// The next `request_user_information` is accepted but Steam never answers.
    silent_user_info: bool,
    /// The next `invite_user_to_game` returns `false`.
    refuse_invite: bool,
}

impl Default for FakeFriendsState {
    fn default() -> Self {
        Self {
            friends: BTreeMap::new(),
            local_name: "Local Player".into(),
            local_state: PersonaState::Online,
            avatars: HashMap::new(),
            avatars_loading: Vec::new(),
            known_users: Vec::new(),
            silent_user_info: false,
            refuse_invite: false,
        }
    }
}

/// Friends controls (feature `friends`). Each change queues the `PersonaChanged` event real Steam
/// would send (delivered on the next pump).
impl FakeSteamBackend {
    /// Add a friend with this state (name: [`set_friend_name`](Self::set_friend_name)).
    pub fn add_friend(&self, id: u64, state: PersonaState) {
        let mut s = self.lock();
        s.friends.friends.insert(id, FakeFriend { state, ..Default::default() });
        s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: RELATIONSHIP | STATUS | NAME });
    }

    /// Remove a friend.
    pub fn remove_friend(&self, id: u64) {
        let mut s = self.lock();
        if s.friends.friends.remove(&id).is_some() {
            s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: RELATIONSHIP });
        }
    }

    /// Change a friend's online status.
    pub fn set_friend_state(&self, id: u64, state: PersonaState) {
        let mut s = self.lock();
        if let Some(f) = s.friends.friends.get_mut(&id) {
            f.state = state;
            s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: STATUS });
        }
    }

    /// Set (`Some((app_id, lobby))`) or clear the game a friend plays.
    pub fn set_friend_game(&self, id: u64, game: Option<(u32, u64)>) {
        let mut s = self.lock();
        if let Some(f) = s.friends.friends.get_mut(&id) {
            f.game = game.map(|(app_id, lobby)| FriendGame::new(app_id, lobby, None));
            s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: GAME_PLAYED });
        }
    }

    /// Set (`Some`) or remove (`None`) one rich-presence key of a friend. Like real Steam, this
    /// sends NO event: the kit sees it on its next refresh.
    pub fn set_friend_rich_presence(&self, id: u64, key: &str, value: Option<&str>) {
        let mut s = self.lock();
        if let Some(f) = s.friends.friends.get_mut(&id) {
            match value {
                Some(v) => f.rich_presence.insert(key.to_string(), v.to_string()),
                None => f.rich_presence.remove(key),
            };
        }
    }

    /// Set (`Some`) or clear the nickname the local user gave a friend.
    pub fn set_friend_nickname(&self, id: u64, nickname: Option<&str>) {
        let mut s = self.lock();
        if let Some(f) = s.friends.friends.get_mut(&id) {
            f.nickname = nickname.map(str::to_string);
            s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: NICKNAME });
        }
    }

    /// Set the local user's own persona name and state.
    pub fn set_local_persona(&self, name: &str, state: PersonaState) {
        let mut s = self.lock();
        s.friends.local_name = name.to_string();
        s.friends.local_state = state;
        let me = s.local_id;
        s.queued.push(BackendEvent::PersonaChanged { steam_id: me, flags: NAME | STATUS });
    }

    /// Give a user an avatar (returned for every size; `rgba` should be `width * height * 4`
    /// bytes).
    pub fn set_friend_avatar(&self, id: u64, width: u32, height: u32, rgba: Vec<u8>) {
        let mut s = self.lock();
        s.friends.avatars.insert(id, (width, height, rgba));
        s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: AVATAR });
    }

    /// `true`: the user's avatar is "loading" (`None` for every size, as Steam answers while it
    /// loads a large one); `false`: available again, with a `PersonaChanged` like Steam.
    pub fn set_friend_avatar_loading(&self, id: u64, loading: bool) {
        let mut s = self.lock();
        s.friends.avatars_loading.retain(|&x| x != id);
        if loading {
            s.friends.avatars_loading.push(id);
        } else {
            s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: AVATAR });
        }
    }

    /// Mark a user's info as already loaded: `request_user_information` then returns `false`
    /// (as it does for friends; otherwise it returns `true` and queues a `PersonaChanged` for the
    /// user).
    pub fn set_user_info_loaded(&self, id: u64) {
        self.lock().friends.known_users.push(id);
    }

    /// The next `request_user_information` is accepted, but Steam never delivers the persona.
    pub fn silence_next_user_info(&self) {
        self.lock().friends.silent_user_info = true;
    }

    /// The next game invite is refused by Steam (`GameInviteSent { ok: false }`).
    pub fn refuse_next_game_invite(&self) {
        self.lock().friends.refuse_invite = true;
    }
}

impl FriendsBackend for FakeSteamBackend {
    fn current_app_id(&self) -> u32 {
        self.lock().app_id
    }

    fn friend_ids(&self) -> Vec<u64> {
        self.lock().friends.friends.keys().copied().collect()
    }

    fn persona_name(&self, id: u64) -> String {
        self.lock().friend_names.get(&id).cloned().unwrap_or_default()
    }

    fn persona_nickname(&self, id: u64) -> Option<String> {
        self.lock().friends.friends.get(&id).and_then(|f| f.nickname.clone())
    }

    fn persona_state(&self, id: u64) -> PersonaState {
        self.lock().friends.friends.get(&id).map_or(PersonaState::Offline, |f| f.state)
    }

    fn game_played(&self, id: u64) -> Option<FriendGame> {
        self.lock().friends.friends.get(&id).and_then(|f| f.game.clone())
    }

    fn friend_rich_presence(&self, id: u64, key: &str) -> Option<String> {
        self.lock().friends.friends.get(&id).and_then(|f| f.rich_presence.get(key).cloned()).filter(|v| !v.is_empty())
    }

    fn local_persona(&self) -> (String, PersonaState) {
        let s = self.lock();
        (s.friends.local_name.clone(), s.friends.local_state)
    }

    fn invite_user_to_game(&self, id: u64, connect: &str) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::InviteToGame { friend: id, connect: connect.to_string() });
        !std::mem::take(&mut s.friends.refuse_invite) && !connect.contains('\0')
    }

    fn request_user_information(&self, id: u64, name_only: bool) -> bool {
        let mut s = self.lock();
        s.calls.push(FakeCall::RequestUserInformation { id, name_only });
        // Friends and users loaded before are known (Steam: `false`, nothing to load).
        if s.friends.known_users.contains(&id) || s.friends.friends.contains_key(&id) {
            return false;
        }
        if std::mem::take(&mut s.friends.silent_user_info) {
            return true;
        }
        s.friends.known_users.push(id);
        s.queued.push(BackendEvent::PersonaChanged { steam_id: id, flags: NAME });
        true
    }

    fn friend_avatar(&self, id: u64, _size: AvatarSize) -> Option<(u32, u32, Vec<u8>)> {
        let s = self.lock();
        if s.friends.avatars_loading.contains(&id) {
            return None;
        }
        s.friends.avatars.get(&id).cloned()
    }
}
