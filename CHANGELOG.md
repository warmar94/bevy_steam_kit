# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (before 1.0, a breaking
change or a Bevy / steamworks bump raises the minor version).

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
  - a stability promise for backend implementors: methods added later always have a default;
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
