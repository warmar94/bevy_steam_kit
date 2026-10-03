//! The friends half of the backend seam: [`FriendsBackend`], reached through
//! [`SteamBackend::friends`](crate::SteamBackend::friends), and the types it uses.

use std::net::Ipv4Addr;

/// Steam's limit on a rich-presence value, and so on a connect string: 256 bytes including the
/// terminating NUL (`k_cchMaxRichPresenceValueLength`), so at most 255 bytes of text.
pub const MAX_CONNECT_BYTES: usize = 255;

/// A user's online status as Steam shows it. `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PersonaState {
    /// Offline (or the state is not known to this client).
    #[default]
    Offline,
    /// Online.
    Online,
    /// Busy (do not disturb).
    Busy,
    /// Away.
    Away,
    /// Away for a long time.
    Snooze,
    /// Online, looking to trade.
    LookingToTrade,
    /// Online, looking to play.
    LookingToPlay,
    /// Online but shown as offline to friends (Steam reports this only for the local user).
    Invisible,
    /// A value this version of the kit does not know.
    Unknown,
}

impl PersonaState {
    /// Steam's raw `EPersonaState` value (`0` offline ... `7` invisible); anything else is
    /// [`PersonaState::Unknown`]. Never panics.
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => Self::Offline,
            1 => Self::Online,
            2 => Self::Busy,
            3 => Self::Away,
            4 => Self::Snooze,
            5 => Self::LookingToTrade,
            6 => Self::LookingToPlay,
            7 => Self::Invisible,
            _ => Self::Unknown,
        }
    }

    /// Reachable: everything except `Offline`, `Invisible` and `Unknown`.
    pub fn is_online(self) -> bool {
        !matches!(self, Self::Offline | Self::Invisible | Self::Unknown)
    }
}

/// The game a user is playing. `#[non_exhaustive]`: a custom backend builds it with
/// [`FriendGame::new`] (or `Default` and then the fields).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FriendGame {
    /// The game's Steam app id.
    pub app_id: u32,
    /// The Steam lobby the user is in (`0` = none).
    pub lobby: u64,
    /// The game server the user is on, if Steam knows one (IPv4 address, game port).
    pub server: Option<(Ipv4Addr, u16)>,
}

impl FriendGame {
    /// A game entry.
    pub fn new(app_id: u32, lobby: u64, server: Option<(Ipv4Addr, u16)>) -> Self {
        Self { app_id, lobby, server }
    }
}

/// The avatar size to read (Steam keeps three). `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AvatarSize {
    /// 32 x 32.
    #[default]
    Small,
    /// 64 x 64.
    Medium,
    /// 184 x 184 (Steam loads it on demand: the first reads can come back empty).
    Large,
}

impl AvatarSize {
    /// The edge length in pixels.
    pub fn pixels(self) -> u32 {
        match self {
            Self::Small => 32,
            Self::Medium => 64,
            Self::Large => 184,
        }
    }
}

/// Everything the friends feature needs from Steam. Implementations must never panic. Every
/// method is a read of Steam's local cache or a fire-and-forget call; none waits.
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added to this trait comes with a default implementation.
pub trait FriendsBackend {
    /// The running app's id.
    fn current_app_id(&self) -> u32;
    /// The SteamID64s of the local user's friends (Steam's "immediate" friends).
    fn friend_ids(&self) -> Vec<u64>;
    /// A user's persona name.
    fn persona_name(&self, id: u64) -> String;
    /// The nickname the local user gave this user, if any.
    fn persona_nickname(&self, id: u64) -> Option<String>;
    /// A friend's online status.
    fn persona_state(&self, id: u64) -> PersonaState;
    /// The game a friend is playing, if any.
    fn game_played(&self, id: u64) -> Option<FriendGame>;
    /// One rich-presence value of a friend (`None` when unset or empty). Steam shares the rich
    /// presence of friends playing the same game; others' is generally not available.
    fn friend_rich_presence(&self, id: u64, key: &str) -> Option<String>;
    /// The local user's own persona name and state, the state as friends see it (`Away`,
    /// `Invisible`, ...). The real backend reads it like a friend's state, with the local user's
    /// own id.
    fn local_persona(&self) -> (String, PersonaState);
    /// Send a Steam game invite carrying `connect` (at most [`MAX_CONNECT_BYTES`], no NUL).
    /// `true` = the call was made (Steam gives no delivery receipt).
    fn invite_user_to_game(&self, id: u64, connect: &str) -> bool;
    /// Ask Steam to load a user's persona (name, and the avatar unless `name_only`). `true` = it
    /// is being loaded (a `PersonaChanged` event follows); `false` = it is already available.
    fn request_user_information(&self, id: u64, name_only: bool) -> bool;
    /// A user's avatar as `(width, height, RGBA bytes)`, or `None` when Steam has none (or has not loaded it).
    fn friend_avatar(&self, id: u64, size: AvatarSize) -> Option<(u32, u32, Vec<u8>)>;
}
