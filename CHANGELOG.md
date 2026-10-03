# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (before 1.0, a breaking
change or a Bevy / steamworks bump raises the minor version).

## [0.2.0] - 2026-10-03

### Added

- **Feature `auth`** (module `auth`): Steam Web API tickets for logging in to a game server.
  `AuthRequest::{WebApiTicket { id, identity }, Cancel { id }}` (one ordered stream) answered once
  per request with `WebApiTicketReady { id, identity, ticket }` or `AuthError { id, kind, message }`
  (`AuthErrorKind`: `NoBackend`, `InvalidIdentity`, `DuplicateId`, `Failed`, `TimedOut`,
  `Cancelled`, `Exiting`); `SteamAuth` (`next_id()`, `is_pending`, `is_live`, `live_tickets`);
  `AuthSettings { timeout, cancel_on_exit }` + `SteamKitPlugin::with_auth`. `WebApiTicket`
  (`bytes()`, lowercase `to_hex()`, `len()`) holds only the first `ticket_len` bytes of Steam's
  buffer, prints its length only in `Debug`, has no `Display`, is zeroed on drop and is never
  logged. Live tickets are cancelled on `AppExit`. A request Steam refuses at once (an invalid
  ticket handle) is answered `Failed` immediately. `AuthBackend`, `SteamBackend::auth()`,
  `BackendEvent::WebApiTicket`, `FakeAuthFailure`, fake knobs.
- **Feature `friends`** (module `friends`): the `SteamFriends` resource (list sorted by
  SteamID64, `me()`, `online()`, `playing_this_game()`, `generation()`), kept current by
  `PersonaStateChange` through the one pump and a re-read every `refresh_interval` (5 s);
  `FriendsChanged { added, removed, changed }`; `FriendInfo` with name, nickname, `PersonaState`
  (read through raw bindings: no panic on `Invisible` or unknown states), game and the
  rich-presence `connect` of friends in this game; `InviteToGame { steam_id, connect }` with any
  connect string (`GameInviteSent`, whose `ok` is Steam's own result); `ConnectRequested {
  connect, from, source }` for every raw join (`RichPresence`) and a cold launch (`LaunchArgs`,
  with `launch_connect_prefix`); `RequestUserInfo` answered once per request (`UserInfoReady`,
  or a `FriendsError` with `TimedOut` after `user_info_timeout` or `NoBackend`); avatars as RGBA
  bytes (`FriendAvatar`, opt-in with `avatars`, retried at the next re-read while Steam has
  none); `FriendsError { request: FriendsRequestKind, steam_id, kind, message }`;
  `FriendsSettings` + `SteamKitPlugin::with_friends`;
  `FriendsBackend`, `SteamBackend::friends()`, `BackendEvent::{PersonaChanged, ConnectRequested}`,
  fake knobs and `FakeCall::RequestUserInformation`.
- **Feature `overlay`** (module `overlay`): `OpenOverlay` (a dialog, a user dialog, a web page, a
  store page with `StoreFlag`, the lobby invite dialog, the invite dialog with a connect string),
  validated before Steam (`OverlayError`); `OverlayToggled { active }` and `SteamOverlay`
  (`is_active()`, `is_enabled()`, `toggles()`; removing the backend closes an open overlay with
  one `OverlayToggled { active: false }`); `OverlayBackend`, `SteamBackend::overlay()`,
  `BackendEvent::OverlayActivated`, `FakeCall::ActivateOverlay`, fake knobs.
- Core: `FakeSteamBackend::set_app_id` and, with `lobby` or `friends`,
  `push_rich_presence_join(from, connect)` (what Steam delivers for a join, for every compiled
  feature). `FakeCall::InviteToGame` is compiled with `lobby` or `friends`.
- Examples `auth_480`, `friends_480`, `overlay_480` (real Steam, app 480; they print no names,
  SteamIDs or ticket bytes).
- README: "What the kit covers", and how to make other Steam calls with your own `steamworks`
  client next to the kit.
- Core: `SteamLost { reason: SteamLostReason }` (`SteamExited`, `SteamProcessEnded`, `PumpPanicked`), written once
  when Steam is gone; `BackendEvent::SteamLost { reason }` for backends; fake knobs
  `simulate_steam_shutdown()` (Steam's shutdown callback: no backend call afterwards),
  `simulate_steam_process_ended()`,
  `simulate_steam_exit()` and `panic_in_next_pump()`. Example `steam_exit_480`.
- README: the overlay needs Steam's in-game overlay setting and a game window; "Join Game" on a
  friend in a lobby arrives as `JoinRequested` with `LobbyInvite`; `GameInviteSent::ok` is Steam's
  answer to the call, not a delivery receipt; where Steam's API library goes next to a built
  binary (`steam_api64.dll`, `libsteam_api.dylib`, `libsteam_api.so`).
- Examples exit with a non-zero code when Steam does not start or a request is answered with an
  error.

### Fixed

- **Steam quitting no longer crashes the app.** `steamworks` 0.12.2 panics on the callback Steam
  sends when its client quits (`SteamServersDisconnected` with result "OK"), inside the kit's
  callback pump, so every app with the real backend ended with a panic (also with 0.1.x). The real
  backend now registers a guard for that callback (and for `SteamServerConnectFailure`) that keeps
  the panic from happening, also with `panic = "abort"`, and that marks Steam as shutting down: from
  then on the kit makes no Steam call of any kind, apart from the end of the pump that delivered
  the callback, and does not shut Steam's library down at exit. When the Steam client quits, or its
  process ends without that callback (killed or crashed: the real backend asks the operating
  system, never Steam, before every pump and every feature access;
  `SteamLostReason::SteamProcessEnded`), the kit writes `SteamLost` once, stops pumping Steam and
  answers every request, including the ones still waiting, with `NoBackend`; the game keeps
  running and decides what to do. Any other panic inside the pump is caught in a build that
  unwinds and handled the same way. With `lobby`, `SteamLost` clears `SteamLobby` (one `LobbyLeft`
  for the current lobby) and answers a create or join in flight with one `LobbyError` with
  `NoBackend`.
- **A "Join Game" or invite connect string that is not UTF-8 no longer crashes the app.**
  `steamworks` 0.12.2 panicked on it inside the callback pump (any friend could send one). The
  real backend fixes the raw string before `steamworks` converts it: invalid bytes become `?`, a
  string without NUL is cut to 255 bytes.
- `friends`: `SteamFriends::me().state` is the local user's real state (`Away`, `Invisible`, ...);
  it was always `Online` (Steam's `GetPersonaState` returns Online whatever the status is; the
  kit reads `GetFriendPersonaState` with the local user's own id).
- `friends`: a `RequestUserInfo` still waiting when the app exits is answered with the new
  `FriendsErrorKind::Exiting` (also one written after the `Update` set in the exit frame); it
  got no answer before.
- `lobby`: the launch-args check skips process arguments that are not valid Unicode (it used
  `std::env::args`, which panics on them). Nothing else changes.

### Changed

- Documentation: `SteamBackend::friend_name` states that real Steam returns `"[unknown]"` for a
  user it knows nothing about (the fake returns `""`); `SteamStats::stores_started` counts a store
  Steam refused locally; `parse_connect_lobby` uses only the first `<prefix>` token; the README
  notes that `steamworks` keeps one `register_callback` closure per callback type; the README and
  the rustdoc describe only what exists.
- `FakeSteamBackend::set_friend_name` also queues the `PersonaChanged` event Steam sends when the
  `friends` feature is compiled.
- CI: 23 feature sets.
- `RealSteamBackend::new` registers three `steamworks` callbacks for the rest of the process, the kit's guards
  (`SteamServersDisconnected`, `SteamServerConnectFailure`, `GameRichPresenceJoinRequested`); the `steam` feature enables `steamworks`'
  `raw-bindings`.

## [0.1.1] - 2026-09-30

Documentation only, no code changes.

### Changed

- README: section 10 recommends the companion crate `bevy_net_session` for connecting players
  (host / join / leave over Steam or UDP), before the manual `renet_steam` recipe; install lines
  name the full version.

## [0.1.0] - 2026-09-29

First release, for Bevy 0.19.0 and steamworks 0.12.2: a core that owns the one Steam callback
pump, and three opt-in features on top of it (`lobby`, `stats`, `leaderboards`).

### Added

- **Core** (always compiled):
  - `SteamKitPlugin` (add once; no public fields; feature settings through one builder per
    feature: `with_lobby`, `with_stats`, `with_leaderboards`);
  - the public sets `SteamKitSystems::{Pump, Callbacks, Requests}` (`#[non_exhaustive]`): the one
    callback pump in `First` before `MessageUpdateSystems`, whose events every feature applies in
    `Callbacks` of the same frame; requests in `Update`; exit handling in `Last`;
  - the backend seam `SteamBackend` (`local_id`, `friend_name`, `launch_command_line`, `pump`, and
    one accessor per feature defaulting to `None`) + `SteamBackendRes` + `BackendEvent`
    (`#[non_exhaustive]`, `PartialEq` without `Eq`);
  - `RealSteamBackend` over steamworks 0.12.2 (feature `steam`) and the in-memory
    `FakeSteamBackend` + `FakeCall` (`#[non_exhaustive]`, `PartialEq` without `Eq`), with
    `pump_count()`;
  - a stability promise for backend implementors: every method added to the traits has a default;
  - the helper `is_individual_steam_id64`.
- **Feature `lobby`** (module `lobby`):
  - settings `LobbySettings` (`connect_prefix`, `set_connect_presence`, `check_launch_args`);
  - requests `CreateLobby` (member cap clamped to 250), `JoinLobby`, `LeaveLobby`, `InviteFriend`,
    `SetRichPresence`, `ClearRichPresence`;
  - facts `LobbyCreated`, `LobbyEntered`, `JoinRequested` (`JoinSource::LobbyInvite`,
    `RichPresence`, `LaunchArgs`), `LobbyLeft`, `InviteSent`, `LobbyError` with `LobbyErrorKind`;
  - the `SteamLobby` resource; rich presence `connect` set for a created lobby; cold-launch
    detection of `+connect_lobby <id>`; abandoned lobbies left on arrival; leave + clear presence
    on `AppExit`;
  - `LobbyBackend`, `LobbyKind`, `JoinSource` (both `#[non_exhaustive]`), `connect_string`,
    `parse_connect_lobby`, `MAX_LOBBY_MEMBERS`.
- **Feature `stats`** (module `stats`): the local user's stats and achievements.
  - settings `StatsSettings` (`probe` None, `probe_interval` 1 s, `stats_store_interval` 60 s,
    `achievement_store_delay` 1 s, `min_store_gap` 10 s, `store_timeout` 30 s, `store_on_exit`
    true, `max_queued` 256);
  - ONE ordered request message `StatsRequest` (`#[non_exhaustive]`: `SetStat`, `AddStat`,
    `UnlockAchievement`, `ClearAchievement`, `IndicateAchievementProgress`, `StoreStats`,
    `ResetAllStats`, with helper constructors), applied in the order written;
  - facts `StatsReady`, `StatsStored`, `AchievementUnlocked`, `AchievementProgress`, `StatsError`
    with `StatsErrorKind` (all `#[non_exhaustive]`);
  - the `SteamStats` resource (`is_ready`, `has_unsaved`, `store_in_flight`, `queued`,
    `stores_started`);
  - readiness by probe (SDK 1.62 has no `RequestCurrentStats`); writes before readiness held in a
    bounded queue and applied in order, never dropped silently (queue overflow and exit reported
    as `NotReady`, also without a backend);
  - batched stores (achievements after a short delay, stats after an interval, a minimum gap, one
    store in flight, a timeout, a final store on `AppExit` that also applies requests written late
    in the exit frame); a failed store keeps achievement changes on the short delay; a locally
    refused store is reported once; stats callbacks of another app id are ignored;
  - `StatsBackend`, `StatValue`, `StatKind`, `FakeStoreFailure`, `is_valid_api_name`,
    `MAX_API_NAME_BYTES` (127); names that would panic steamworks and non-finite floats are
    refused before Steam is called; `get_achievement_names` is never called; the progress popup
    through the raw `IndicateAchievementProgress` (the feature enables `steamworks/raw-bindings`).
- **Feature `leaderboards`** (module `leaderboards`; independent of `stats`):
  - settings `LeaderboardSettings` (`timeout` 30 s, `uploads_per_window` 10, `upload_window`
    600 s, `max_queued_uploads` 64, `max_download_rows` 500, capped at `i32::MAX`);
  - the request message `LeaderboardRequest` (`#[non_exhaustive]`: `Find`, `FindOrCreate`,
    `UploadScore`, `DownloadScores`, with helpers) carrying a `LeaderboardRequestId`;
  - exactly one answer per accepted request: `LeaderboardFound`, `ScoreUploaded`,
    `ScoresDownloaded` or `LeaderboardError` with `LeaderboardErrorKind` (all `#[non_exhaustive]`;
    `DuplicateId` answers only the rejected duplicate); requests still waiting are answered
    `NoBackend` when the backend is removed and `Exiting` on `AppExit`;
  - the `SteamLeaderboards` resource: a handle cache by name, and `next_id()` (kit-issued ids,
    increasing, never a pending id, below `LeaderboardRequestId::FIRST_MANUAL` so hand-picked ids
    from there up never collide);
  - one upload in flight, a sliding-window upload limiter, a bounded upload queue, a timeout per
    call (started when a clock appears for calls made without one);
  - `LeaderboardInfo`, `LeaderboardEntry`, `LeaderboardSort`, `LeaderboardDisplay`,
    `UploadMethod`, `ScoreRange` (`Global`, `AroundUser`, `Friends`), `LeaderboardBackend`,
    `FakeLeaderboardFailure`, `is_valid_leaderboard_name`, `MAX_LEADERBOARD_NAME_BYTES` (127),
    `MAX_LEADERBOARD_DETAILS` (64); names, detail counts and ranges checked before Steam;
    downloads always sized for 64 details; not supported (not wrapped by steamworks 0.12.2):
    downloading chosen users' entries, attaching UGC.
- Examples `quick_start` (fake backend, `lobby`), `host_lobby` and `join_lobby` (real Steam,
  `lobby` + `steam`), `stats_480` (`stats` + `steam`) and `leaderboard_480` (`leaderboards` +
  `steam`) for Valve's test app 480; a README recipe for `bevy_replicon` 0.44 over `renet_steam`
  3.0.0.
