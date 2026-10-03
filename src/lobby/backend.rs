//! The lobby half of the backend seam: [`LobbyBackend`], reached through
//! [`SteamBackend::lobby`](crate::SteamBackend::lobby), and the lobby enums it uses.

/// Steam lobby visibility (mirrors `steamworks::LobbyType`). `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LobbyKind {
    /// Only joinable by invitation.
    Private,
    /// Joinable by friends of members (and by invitation). The usual co-op choice.
    #[default]
    FriendsOnly,
    /// Listed publicly.
    Public,
    /// Joinable by anyone with the id, not listed; friends do not see it.
    Invisible,
}

/// How a join request reached this process. `#[non_exhaustive]`: keep a `_` arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum JoinSource {
    /// Steam `GameLobbyJoinRequested`: a friend clicked "Join Game" on a player in a lobby, or the
    /// user accepted a lobby invite, while this game was already running.
    LobbyInvite,
    /// Steam `GameRichPresenceJoinRequested`: the rich-presence `connect` string was used while the
    /// game was running.
    RichPresence,
    /// Cold launch: Steam started the process with the connect string on its command line.
    LaunchArgs,
}

/// Everything the lobby feature needs from Steam. Implementations must never panic; a failure is
/// a `false` / `None` / an error event.
///
/// Asynchronous calls (`create_lobby`, `join_lobby`) return nothing: their outcome is queued and
/// returned as a [`BackendEvent`](crate::BackendEvent) by a following
/// [`SteamBackend::pump`](crate::SteamBackend::pump).
///
/// You may implement it for your own backend. Stability promise: the methods below stay required
/// as they are, and every method added to this trait comes with a default implementation, so
/// an existing implementation keeps compiling.
pub trait LobbyBackend {
    /// Start creating a lobby. Outcome: `BackendEvent::LobbyCreated` or
    /// `BackendEvent::LobbyCreateFailed` from a following pump.
    fn create_lobby(&self, kind: LobbyKind, max_members: u32);
    /// Start joining a lobby. Outcome: `BackendEvent::LobbyEntered` or
    /// `BackendEvent::LobbyJoinFailed` from a following pump.
    fn join_lobby(&self, lobby: u64);
    /// Leave a lobby (no-op if not a member).
    fn leave_lobby(&self, lobby: u64);
    /// Set a lobby metadata key (owner only). `false` on failure.
    fn set_lobby_data(&self, lobby: u64, key: &str, value: &str) -> bool;
    /// Read a lobby metadata key. `None` when missing.
    fn lobby_data(&self, lobby: u64, key: &str) -> Option<String>;
    /// Number of members currently in the lobby.
    fn lobby_member_count(&self, lobby: u64) -> usize;
    /// Allow or forbid joining the lobby. `false` on failure.
    fn set_lobby_joinable(&self, lobby: u64, joinable: bool) -> bool;
    /// Set (`Some`) or remove (`None`) one rich-presence key. `false` on failure.
    fn set_rich_presence(&self, key: &str, value: Option<&str>) -> bool;
    /// Remove every rich-presence key this process set.
    fn clear_rich_presence(&self);
    /// Send a Steam game invite carrying `connect` to `friend`. `false` if it could not be sent.
    fn invite_to_game(&self, friend: u64, connect: &str) -> bool;
}
