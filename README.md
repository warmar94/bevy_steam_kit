# bevy_steam_kit

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![CI](https://github.com/warmar94/bevy_steam_kit/actions/workflows/ci.yml/badge.svg)](https://github.com/warmar94/bevy_steam_kit/actions/workflows/ci.yml)
[![Bevy 0.19.0](https://img.shields.io/badge/Bevy-0.19.0-informational)](https://bevyengine.org)
[![steamworks 0.12.2](https://img.shields.io/badge/steamworks-0.12.2-informational)](https://crates.io/crates/steamworks)
[![crates.io](https://img.shields.io/crates/v/bevy_steam_kit.svg)](https://crates.io/crates/bevy_steam_kit)
[![docs.rs](https://docs.rs/bevy_steam_kit/badge.svg)](https://docs.rs/bevy_steam_kit)

**Steam for [Bevy](https://bevyengine.org), as opt-in features.** Enable the parts of Steam your
game uses; they all plug into one plugin that owns the one Steam callback pump of the process.

Six Steam features:

- **`lobby`**: create and join lobbies, friends-only or otherwise, rich presence so friends see
  **"Join Game"** in their Steam friends list, game invites, and every way a join request can
  reach your game (an accepted invite, "Join Game" in the friends list, or a cold launch with
  `+connect_lobby <id>`) turned into one Bevy message.
- **`stats`**: the player's Steam stats and achievements: set and add to stats, unlock
  achievements, show progress popups, with batched stores at the cadence Valve asks for.
- **`leaderboards`**: find leaderboards, upload scores (queued and rate-limited as Valve asks),
  download the top, the entries around the player, or the player's friends.
- **`auth`**: Steam Web API tickets, so your game server learns who the player is (request, hex,
  cancel; the ticket never appears in logs or `Debug` output).
- **`friends`**: the friends list with online status, the game each friend plays and the
  rich-presence `connect` of friends in your game, game invites carrying any connect string, every
  join request as the raw connect string, user info for non-friends, avatars as RGBA bytes.
- **`overlay`**: open the Steam overlay (a dialog, a profile, a web page, a store page, the invite
  dialog) and know when it opens and closes.

It is a **service**, not a framework: your game sends requests (`CreateLobby`, `JoinLobby`,
`StatsRequest::UnlockAchievement`, ...) and reacts to facts (`LobbyCreated`, `JoinRequested`,
`AchievementUnlocked`, ...).
The kit never decides anything about your game and never touches your networking: what a join
request *means* (open a menu, pick a character, connect) is your code, and connecting to the host
is done with whatever transport you already use. An in-memory fake Steam makes all of it testable
without a Steam client.

## Contents

- [Features](#features)
- [What the kit covers](#what-the-kit-covers)
- [Highlights](#highlights)
- [Quick start](#quick-start)
  - [Lobby quick start](#lobby-quick-start)
  - [Stats quick start](#stats-quick-start)
  - [Leaderboards quick start](#leaderboards-quick-start)
  - [Auth quick start](#auth-quick-start)
  - [Friends quick start](#friends-quick-start)
  - [Overlay quick start](#overlay-quick-start)
- [The one-pump rule](#the-one-pump-rule)
- [Lobbies: how to use them](#lobbies-how-to-use-them)
  - [1. Add the plugin](#1-add-the-plugin)
  - [2. Turn Steam on](#2-turn-steam-on)
  - [3. Host a lobby](#3-host-a-lobby)
  - [4. React to join requests](#4-react-to-join-requests)
  - [5. Enter the lobby and connect](#5-enter-the-lobby-and-connect)
  - [6. Leave](#6-leave)
  - [7. Invite a friend](#7-invite-a-friend)
  - [8. Rich presence](#8-rich-presence)
  - [9. Errors](#9-errors)
  - [10. Wiring a transport (bevy_replicon + renet_steam)](#10-wiring-a-transport-bevy_replicon--renet_steam)
  - [11. System order](#11-system-order)
  - [12. Testing your game with the fake backend](#12-testing-your-game-with-the-fake-backend)
- [Stats and achievements](#stats-and-achievements)
  - [Set up](#set-up)
  - [Write stats and unlock achievements](#write-stats-and-unlock-achievements)
  - [Read current values](#read-current-values)
  - [Readiness: there is no "request stats" step](#readiness-there-is-no-request-stats-step)
  - [Store cadence](#store-cadence)
  - [Guarded traps](#guarded-traps)
  - [Testing stats with the fake backend](#testing-stats-with-the-fake-backend)
  - [Testing stats with real Steam (app 480)](#testing-stats-with-real-steam-app-480)
- [Leaderboards](#leaderboards)
  - [Requests and answers](#requests-and-answers)
  - [Limits and queueing](#limits-and-queueing)
  - [Guards and errors](#guards-and-errors)
  - [Testing leaderboards](#testing-leaderboards)
- [Web API tickets (auth)](#web-api-tickets-auth)
- [Friends](#friends)
  - [The list](#the-list)
  - [Invites and joins](#invites-and-joins)
  - [Avatars](#avatars)
- [The Steam overlay](#the-steam-overlay)
- [How it works](#how-it-works)
- [API reference](#api-reference)
  - [Core (always compiled)](#core-always-compiled)
  - [Lobby: settings, state, backend (feature `lobby`)](#lobby-settings-state-backend-feature-lobby)
  - [Lobby: request messages (you write)](#lobby-request-messages-you-write)
  - [Lobby: fact messages (you read)](#lobby-fact-messages-you-read)
  - [Lobby: enums and helpers](#lobby-enums-and-helpers)
  - [Stats (feature `stats`)](#stats-feature-stats)
  - [Leaderboards (feature `leaderboards`)](#leaderboards-feature-leaderboards)
  - [Auth (feature `auth`)](#auth-feature-auth)
  - [Friends (feature `friends`)](#friends-feature-friends)
  - [Overlay (feature `overlay`)](#overlay-feature-overlay)
- [Compatibility](#compatibility)
- [Examples](#examples)
- [Testing with real Steam](#testing-with-real-steam)
- [FAQ](#faq)
- [License](#license)
- [Contributing](#contributing)

## Features

```toml
[dependencies]
bevy = "0.19.0"
bevy_steam_kit = { version = "0.2.0", features = ["lobby", "stats", "leaderboards", "auth", "friends", "overlay", "steam"] }
```

or from the repository, pinned to a release tag:

```toml
[dependencies]
bevy = "0.19.0"
bevy_steam_kit = { git = "https://github.com/warmar94/bevy_steam_kit", tag = "v0.2.0", features = ["lobby", "stats", "leaderboards", "auth", "friends", "overlay", "steam"] }
```

| feature | default | what it adds |
|---|---|---|
| *(none)* | always | the core: `SteamKitPlugin`, the one callback pump (`SteamKitSystems::Pump`), the backend seam (`SteamBackend`, `SteamBackendRes`), the in-memory `FakeSteamBackend`, `is_individual_steam_id64` |
| `lobby` | no | lobbies, lobby data, rich presence, game invites and friend-join requests as Bevy messages (the `lobby` module, also re-exported at the crate root) |
| `stats` | no | the player's stats and achievements as Bevy messages, with batched stores (the `stats` module, also re-exported at the crate root; adds `bevy_time` for its clock) |
| `leaderboards` | no | find leaderboards, upload scores, download entries (the `leaderboards` module, also re-exported at the crate root; adds `bevy_time`). Independent of `stats` |
| `auth` | no | Steam Web API tickets for logging in to a game server (the `auth` module, also re-exported at the crate root; adds `bevy_time`) |
| `friends` | no | the friends list, invites with any connect string, raw join requests, user info, avatars (the `friends` module, also re-exported at the crate root; adds `bevy_time`) |
| `overlay` | no | open the Steam overlay, know when it opens and closes (the `overlay` module, also re-exported at the crate root) |
| `steam` | no | `RealSteamBackend` over `steamworks` 0.12.2 (links the Steam API library; `steamworks-sys` ships the redistributable for Windows, Linux and macOS) |

Features are additive: a feature you do not enable is not compiled at all. Without `steam` the
crate depends on `bevy_app`, `bevy_ecs` (both without default features), `tracing`, and
`bevy_time` with `stats`, `leaderboards`, `auth` or `friends`, and builds on machines without the
Steam SDK runtime: the game compiles and runs, only without a real backend (requests are answered
with `NoBackend` errors, or driven by the fake backend). A common setup is the Steam features you
use always on and `steam` behind a feature of your own game, so development builds and tests run
without Steam.

## What the kit covers

The kit wraps the parts of Steam that need the one callback pump (callbacks and call results),
or where `steamworks` 0.12.2 has traps (inputs that panic, states it does not map, results it
discards):

| feature | covers |
|---|---|
| `lobby` | lobbies, lobby data, rich presence, invites, every friend-join path |
| `stats` | the player's stats and achievements, batched stores |
| `leaderboards` | find, upload, download, with Valve's limits |
| `auth` | Web API tickets for a game server |
| `friends` | the friends list, invites with any connect string, raw joins, user info, avatars |
| `overlay` | overlay dialogs, store and web pages, invite dialogs, open / closed |

Other Steam calls (DLC ownership, the game's language, Steam Deck detection, the app's build or
beta branch, ...) are single instant calls: make them with your own `steamworks::Client` clone,
next to the kit, and never pump it. Steam Cloud saves need no code at all with Steam Auto-Cloud
(configured on the partner site).

```rust,no_run
use bevy::prelude::*;
use bevy_steam_kit::*;

/// Your own handle on Steam, next to the kit's backend.
#[derive(Resource, Clone)]
struct Steam(steamworks::Client);

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()));
    if let Ok(client) = steamworks::Client::init_app(480) {
        app.insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client.clone()))));
        app.insert_resource(Steam(client));
    }
    app.add_systems(Startup, read_facts).run();
}

fn read_facts(steam: Option<Res<Steam>>) {
    let Some(steam) = steam else { return };
    let language = steam.0.apps().current_game_language();
    let owns_dlc = steam.0.apps().is_dlc_installed(steamworks::AppId(480_001));
    let on_deck = steam.0.utils().is_steam_running_on_steam_deck();
    info!("language {language}, DLC {owns_dlc}, Steam Deck {on_deck}");
}
```

## Highlights

- **One plugin, one pump**: `SteamKitPlugin` is added once and pumps Steam exactly once per
  frame; every feature receives its share of that pump in the same frame. Enabling another
  feature never adds a second pump.
- **ECS-shaped lobby API**: six request messages in, six fact messages out, one read-only
  resource (`SteamLobby`) and public `SystemSet`s to order your systems against.
- **Every join path, one message**: `JoinRequested { lobby, from, source }` for an accepted lobby
  invite, a rich-presence "Join Game" and a cold launch with `+connect_lobby <id>` (checked in the
  process arguments and in Steam's launch command line).
- **The game decides**: the kit never joins a lobby by itself; it reports, you answer with
  `JoinLobby` (or not).
- **Friends list integration**: a lobby you create sets the rich-presence `connect` key for you,
  so friends get "Join Game"; leaving (or quitting) clears what the kit set.
- **Transport-agnostic**: lobby data carries whatever you need (a host SteamID64, a build version)
  and you connect with your own networking: the companion crate
  [`bevy_net_session`](https://crates.io/crates/bevy_net_session) does it in a few lines, or wire
  `bevy_replicon` over `renet_steam` yourself with the documented recipe.
- **Fail closed, never panic**: Steam not running is a `LobbyError`, not a crash; inputs that
  would make `steamworks` panic never reach it (strings with an interior NUL byte are refused,
  a member cap above 250 is clamped); late or abandoned lobby results are left, never adopted.
  Steam quitting while the game runs is one `SteamLost` message, not a crash.
- **Stats and achievements without the pitfalls**: no "request stats" step to forget, writes made
  before Steam is ready are held (never dropped silently), stores are batched to Valve's cadence and
  run once more on exit, and names that would crash `steamworks` are refused first.
- **Leaderboards by name, with Valve's limits built in**: one upload at a time, at most 10 per
  10 minutes, every call answered exactly once by request id (found, uploaded, downloaded, or an
  error), inputs that would crash or corrupt memory in `steamworks` refused first.
- **Logging in to your server**: one request gives a Web API ticket as lowercase hex for Steam's
  `AuthenticateUserTicket`; the kit cancels live tickets on exit and never logs the ticket.
- **Friends without the traps**: the friends list stays current (persona callbacks plus a 5 s
  re-read for rich presence, which sends no callback), persona states are read without the
  `steamworks` call that panics on "invisible", invites take any connect string, and joins arrive
  raw for games that do not use the kit's lobbies.
- **The overlay as messages**: one `OpenOverlay` message per dialog, store page, web page or
  invite dialog, and `OverlayToggled` to pause the game while the overlay is open.
- **Tested without Steam**: everything goes through the `SteamBackend` trait; the
  `FakeSteamBackend` drives lobbies, join requests, invites, stats, achievements, stores,
  leaderboards, tickets, friends, the overlay and every failure deterministically.

## Quick start

Every quick start runs anywhere: the fake backend stands in for Steam (with real Steam, insert
`RealSteamBackend` instead; see [section 2](#2-turn-steam-on)).

### Lobby quick start

A game that hosts a lobby and accepts every join request (feature `lobby`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        // With real Steam: SteamBackendRes(Box::new(RealSteamBackend::new(client)))
        .insert_resource(SteamBackendRes(Box::new(FakeSteamBackend::new())))
        .add_systems(Startup, host)
        .add_systems(Update, (on_created, accept_joins).before(SteamKitSystems::Requests))
        .add_systems(Update, quit_after_a_few_frames)
        .run();
}

fn host(mut create: MessageWriter<CreateLobby>) {
    create.write(CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: vec![("version".into(), "1".into())] });
}

fn on_created(mut created: MessageReader<LobbyCreated>) {
    for ev in created.read() {
        info!("lobby {} is open; friends now see \"Join Game\"", ev.lobby);
    }
}

/// What a join request means is your decision. Here: accept it.
fn accept_joins(mut requests: MessageReader<JoinRequested>, mut join: MessageWriter<JoinLobby>) {
    for req in requests.read() {
        join.write(JoinLobby { lobby: req.lobby });
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 5 {
        exit.write(AppExit::Success);
    }
}
```

`cargo run --example quick_start --features lobby` runs a longer version of this that also
simulates a friend's invite and prints every message.

### Stats quick start

Count a win, unlock an achievement, and hear back when Steam confirmed it (feature `stats`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    let fake = FakeSteamBackend::new();
    fake.define_stat("NumGames", StatValue::I32(0));
    fake.define_achievement("ACH_WIN_ONE_GAME", false);
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake)))
        .add_systems(Startup, win)
        .add_systems(Update, (on_unlocked, quit_after_a_few_frames))
        .run();
}

fn win(mut stats: MessageWriter<StatsRequest>) {
    stats.write(StatsRequest::add_stat("NumGames", StatValue::I32(1)));
    stats.write(StatsRequest::unlock_achievement("ACH_WIN_ONE_GAME"));
    stats.write(StatsRequest::StoreStats); // otherwise stored at the normal cadence
}

fn on_unlocked(mut unlocked: MessageReader<AchievementUnlocked>) {
    for ev in unlocked.read() {
        info!("{} unlocked", ev.name);
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 5 {
        exit.write(AppExit::Success);
    }
}
```

### Leaderboards quick start

Post a time and show the three best (feature `leaderboards`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    let fake = FakeSteamBackend::new();
    fake.add_leaderboard("Quickest Win", LeaderboardSort::Ascending, LeaderboardDisplay::TimeSeconds);
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake)))
        .add_systems(Update, (post_once, show_top).chain().before(SteamKitSystems::Requests))
        .add_systems(Update, quit_after_a_few_frames)
        .run();
}

fn post_once(mut done: Local<bool>, mut ids: ResMut<SteamLeaderboards>, mut boards: MessageWriter<LeaderboardRequest>) {
    if !std::mem::replace(&mut *done, true) {
        boards.write(LeaderboardRequest::upload(ids.next_id(), "Quickest Win", 95, UploadMethod::KeepBest));
    }
}

fn show_top(
    mut uploaded: MessageReader<ScoreUploaded>,
    mut downloaded: MessageReader<ScoresDownloaded>,
    mut ids: ResMut<SteamLeaderboards>,
    mut boards: MessageWriter<LeaderboardRequest>,
) {
    for up in uploaded.read() {
        boards.write(LeaderboardRequest::download(ids.next_id(), up.board.clone(), ScoreRange::Global { first: 1, last: 3 }));
    }
    for d in downloaded.read() {
        for e in &d.entries {
            info!("#{} score {}", e.rank, e.score);
        }
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 8 {
        exit.write(AppExit::Success);
    }
}
```

### Auth quick start

Get a Web API ticket for your game server, send it, cancel it (feature `auth`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(FakeSteamBackend::new())))
        .add_systems(Update, (log_in, on_ticket).chain().before(SteamKitSystems::Requests))
        .add_systems(Update, quit_after_a_few_frames)
        .run();
}

fn log_in(mut done: Local<bool>, mut auth: ResMut<SteamAuth>, mut requests: MessageWriter<AuthRequest>) {
    if !std::mem::replace(&mut *done, true) {
        // The identity is agreed with the service that checks the ticket.
        requests.write(AuthRequest::web_api_ticket(auth.next_id(), "my-game-server"));
    }
}

fn on_ticket(mut ready: MessageReader<WebApiTicketReady>, mut requests: MessageWriter<AuthRequest>) {
    for r in ready.read() {
        let hex = r.ticket.to_hex(); // send this to your server, never to a log
        info!("ticket ready ({} bytes)", r.ticket.len());
        let _ = hex;
        // Once the server answered:
        requests.write(AuthRequest::cancel(r.id));
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 5 {
        exit.write(AppExit::Success);
    }
}
```

### Friends quick start

List friends playing this game and invite one to your server (feature `friends`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    let fake = FakeSteamBackend::new();
    fake.set_friend_name(76_561_197_960_265_730, "A friend");
    fake.add_friend(76_561_197_960_265_730, PersonaState::Online);
    fake.set_friend_game(76_561_197_960_265_730, Some((480, 0)));
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake)))
        .add_systems(Update, (show, on_join).before(SteamKitSystems::Requests))
        .add_systems(Update, quit_after_a_few_frames)
        .run();
}

fn show(
    friends: Res<SteamFriends>,
    mut changed: MessageReader<FriendsChanged>,
    mut invited: Local<std::collections::HashSet<u64>>,
    mut invite: MessageWriter<InviteToGame>,
) {
    if changed.read().count() == 0 {
        return;
    }
    for f in friends.playing_this_game() {
        // Invite each friend once, not on every change of the list.
        if invited.insert(f.steam_id) {
            info!("{} is playing", f.display_name());
            invite.write(InviteToGame { steam_id: f.steam_id, connect: "+connect 203.0.113.7:7777".into() });
        }
    }
}

/// An accepted invite (or "Join Game") on the other side: the raw connect string.
fn on_join(mut joins: MessageReader<ConnectRequested>) {
    for j in joins.read() {
        info!("connect to {:?}", j.connect);
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 5 {
        exit.write(AppExit::Success);
    }
}
```

### Overlay quick start

Open the store page from a menu and pause while the overlay is open (feature `overlay`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

#[derive(Resource, Default)]
struct Paused(bool);

fn main() {
    App::new()
        .add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(FakeSteamBackend::new())))
        .init_resource::<Paused>()
        .add_systems(Startup, |mut open: MessageWriter<OpenOverlay>| {
            open.write(OpenOverlay::store(480));
        })
        .add_systems(Update, (pause_on_overlay, quit_after_a_few_frames))
        .run();
}

fn pause_on_overlay(mut toggled: MessageReader<OverlayToggled>, mut paused: ResMut<Paused>) {
    for t in toggled.read() {
        paused.0 = t.active;
    }
}

fn quit_after_a_few_frames(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames == 5 {
        exit.write(AppExit::Success);
    }
}
```

## The one-pump rule

Steam delivers everything asynchronous through one pump: `steamworks::Client::process_callbacks`
(or `run_callbacks`). A single call dispatches the results of async calls (such as a lobby
create), every callback registered with `Client::register_callback` (a networking transport's, for
example), and the callbacks the caller asked to see. Steam hands each one out **once**, so a
second pump anywhere in the process steals results from the first.

**The kit owns that pump.** `SteamKitPlugin` calls `process_callbacks` exactly once per frame, in
`First` (`SteamKitSystems::Pump`), and every feature (`lobby`, `stats`, `leaderboards`, `auth`,
`friends`, `overlay`) gets its events from that same call. So:

- **Your game must not call `run_callbacks` or `process_callbacks`** anywhere, on any clone of the
  `Client`. Other crates that pump Steam on their own need to be configured not to (or not be used
  next to the kit).
- **Keep your own `steamworks::Client` clone** for any Steam API the kit does not wrap (DLC checks,
  a networking transport, ...). Use it freely; just never pump it. Callbacks you
  register with `register_callback` on your clone still run: they are dispatched by the kit's
  pump. `steamworks` keeps one closure per callback type for the whole process (a second
  `register_callback` of the same type replaces the first). The kit registers exactly three,
  its guards against panics inside `steamworks` 0.12.2: `SteamServersDisconnected` and
  `SteamServerConnectFailure` (Steam quitting, [when Steam quits](#when-steam-quits)) and
  `GameRichPresenceJoinRequested` (a connect string that is not UTF-8). They stay registered for
  the rest of the process. Do not register those three types yourself: a registration made after
  the kit's replaces the guard; one made before is replaced by the guard without notice, and
  dropping its handle later removes the guard. A typed closure for them panics in `steamworks`
  0.12.2 in those cases, with or without the kit.
- Transports that register their own Steam callbacks (such as `renet_steam`) are served by the
  kit's pump too; they need no pump of their own.

```rust,no_run
use bevy::prelude::*;
use bevy_steam_kit::*;

/// Your own handle on Steam, for everything the kit does not wrap.
#[derive(Resource, Clone)]
struct Steam(steamworks::Client);

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()));
    if let Ok(client) = steamworks::Client::init_app(480) {
        app.insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client.clone()))));
        app.insert_resource(Steam(client));
    }
    app.add_systems(Update, check_dlc).run();
}

/// Any other Steam API through your own clone. No pumping here.
fn check_dlc(steam: Option<Res<Steam>>) {
    if let Some(steam) = steam {
        let _owned = steam.0.apps().is_dlc_installed(steamworks::AppId(480_001));
    }
}
```

## Lobbies: how to use them

Sections 1, 2 and 11 apply to every feature; the rest is the `lobby` feature. Everything below uses
`use bevy::prelude::*; use bevy_steam_kit::*;` and the `lobby` feature (sections that insert
`RealSteamBackend` also need `steam`).

### 1. Add the plugin

`SteamKitPlugin` installs the pump and, with `lobby`, registers every lobby message, the
`SteamLobby` resource and three lobby systems. It adds no other plugin and works under
`MinimalPlugins` or `DefaultPlugins`. Add it **once**. Until a backend exists (see
[section 2](#2-turn-steam-on)) it is inert and answers lobby requests with a `NoBackend` error.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

let mut app = App::new();
app.add_plugins((
    MinimalPlugins,
    SteamKitPlugin::default().with_lobby(LobbySettings { connect_prefix: "+connect_lobby".into(), ..Default::default() }),
));
```

Settings go through one builder method per feature (`with_lobby`, `with_stats`,
`with_leaderboards`, `with_auth`, `with_friends`; `overlay` has no settings); the plugin has no
public fields, so your code keeps compiling when another crate in the build enables more
features. Without a `with_*` call a feature uses its defaults.

`LobbySettings`:

| field | default | meaning |
|---|---|---|
| `connect_prefix` | `"+connect_lobby"` | the connect string is `"<prefix> <lobby id>"`: written to rich presence and invites, parsed from "Join Game" and from the launch command line. `+connect_lobby` is what Steam itself uses for lobby launches, so keep it unless you have a reason not to |
| `set_connect_presence` | `true` | when a lobby this process created is ready, set rich presence `connect` so friends get "Join Game" |
| `check_launch_args` | `true` | on the first frame with a lobby-capable backend, look for `<prefix> <id>` (or `<prefix>=<id>`) in the process arguments and in Steam's launch command line, and report it as a `JoinRequested` with `JoinSource::LaunchArgs` |

The plugin also inserts the `LobbySettings` as a resource; treat it as read-only.

### 2. Turn Steam on

Enable the `steam` feature and add `steamworks` itself (the exact version the kit uses):

```toml
[dependencies]
bevy_steam_kit = { version = "0.2.0", features = ["lobby", "steam"] }
steamworks = "=0.12.2"
```

Then initialise Steam **yourself**, before the app runs, and hand the client to the kit. The kit
never initialises Steam: your game owns the app id and decides what to do when Steam is not
running (usually: keep going without the Steam features).

```rust,no_run
use bevy::prelude::*;
use bevy_steam_kit::*;

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()));

    // 480 is Valve's public test app ("Spacewar"); use your own app id in a shipped game.
    match steamworks::Client::init_app(480) {
        Ok(client) => {
            // Optional: warm up the relay network now if you connect over Steam P2P.
            client.networking_utils().init_relay_network_access();
            // The backend keeps a clone of the client alive for the app's lifetime.
            app.insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))));
        }
        Err(e) => warn!("Steam is not available: {e}"),
    }
    app.run();
}
```

- **Development:** the usual setup is a file named `steam_appid.txt` containing your app id (e.g.
  `480`) in the working directory of the executable (for `cargo run`, the directory you run it
  from). `Client::init_app(id)` passes the id itself, so the file is strictly needed only with
  `Client::init()`; it does no harm with `init_app`. Do not ship it (a game launched by Steam gets
  its id from Steam).
- **Keep the `Client` alive.** Steam shuts down when the last clone of the client is dropped.
  The backend holds one; keep your own clone in a resource if you use Steam elsewhere
  ([the one-pump rule](#the-one-pump-rule)).
- **Steam must be running and logged in** before the game starts; `init_app` fails otherwise.
- **Steam's library next to the binary.** `cargo run` finds Steam's API library by itself
  (`steamworks-sys` copies it into its build output directory, `target/<profile>/build/steamworks-sys-*/out/`).
  A built binary started any other way, and a shipped game, needs it next to the executable:
  `steam_api64.dll` on Windows, `libsteam_api.dylib` on macOS; on Linux `libsteam_api.so` goes on
  the library search path (for example `LD_LIBRARY_PATH`, or next to the binary with an
  `$ORIGIN` rpath).
- The backend can be inserted (or removed) at any time, except in `First` between
  `SteamKitSystems::Pump` and `SteamKitSystems::Callbacks` (the events pumped in that frame would
  be dropped, with a warning). A game that starts Steam after launch, or only in some modes, just
  inserts it then. The pump runs only while a backend exists.

### 3. Host a lobby

Send `CreateLobby`. The `data` are lobby metadata keys **you** choose; every member can read them
(typically the host's SteamID64 and a build version, so a joiner can check it can connect).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn host(mut create: MessageWriter<CreateLobby>, backend: Res<SteamBackendRes>) {
    let me = backend.0.local_id();
    create.write(CreateLobby {
        kind: LobbyKind::FriendsOnly,
        max_members: 4, // including you
        data: vec![("host".into(), me.to_string()), ("version".into(), "1".into())],
    });
}

fn on_created(mut created: MessageReader<LobbyCreated>, mut presence: MessageWriter<SetRichPresence>) {
    for ev in created.read() {
        info!("hosting lobby {}", ev.lobby);
        // `connect` is already set; add your own keys on top.
        presence.write(SetRichPresence { key: "status".into(), value: Some("Hosting".into()) });
    }
}
```

When Steam answers, the kit sets your `data`, makes the lobby joinable, sets rich presence
`connect = "+connect_lobby <id>"` (unless `set_connect_presence` is off), records it in
`SteamLobby::current` and writes `LobbyCreated { lobby }`. Friends now see "Join Game" on you.

- `max_members` is clamped to `1..=250` (Steam's hard limit).
- `LobbyKind`: `FriendsOnly` (default; friends of members and invitees can join), `Private`
  (invitation only), `Public` (listed in lobby searches), `Invisible` (joinable by id, not listed,
  not shown to friends).
- A `CreateLobby` while a lobby is current or a create/join is in flight is refused with
  `LobbyErrorKind::AlreadyInLobby`. Send `LeaveLobby` first (in the same frame is fine: leaves are
  handled before creates).
- A failure is a `LobbyError` with `LobbyErrorKind::CreateFailed`.

### 4. React to join requests

`JoinRequested { lobby, from, source }` arrives whenever someone asks this process to join a
lobby:

| `source` | when | `from` |
|---|---|---|
| `JoinSource::LobbyInvite` | the player accepted a lobby invite, or used "Join Game" on a friend who is in a lobby, while the game was running | the friend's SteamID64 |
| `JoinSource::RichPresence` | a game invite carrying the `connect` string was accepted, or "Join Game" on a friend whose rich-presence `connect` is set, while the game was running (for a friend in a Steam lobby, such as a kit host, Steam reports "Join Game" as `LobbyInvite`) | the friend's SteamID64, or `0` when not from a friend |
| `JoinSource::LaunchArgs` | the game was **started** by Steam to join (a cold launch with `+connect_lobby <id>`) | `0` |

**The kit never joins by itself.** Your game decides what happens: join at once, ask the player,
open a character select first, refuse while already in a match. When it wants to join, it sends
`JoinLobby`:

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

#[derive(Resource, Default)]
struct PendingInvite(Option<u64>);

fn on_join_requested(
    mut requests: MessageReader<JoinRequested>,
    backend: Res<SteamBackendRes>,
    mut pending: ResMut<PendingInvite>,
) {
    for req in requests.read() {
        let who = if req.from == 0 { "someone".to_string() } else { backend.0.friend_name(req.from) };
        info!("{who} wants you in lobby {} ({:?})", req.lobby, req.source);
        // Your UI shows "Join {who}?" and calls `accept` on yes.
        pending.0 = Some(req.lobby);
    }
}

fn accept(mut pending: ResMut<PendingInvite>, mut join: MessageWriter<JoinLobby>) {
    if let Some(lobby) = pending.0.take() {
        join.write(JoinLobby { lobby });
    }
}
```

`JoinLobby` leaves the current lobby first (and abandons any create/join in flight). Joining the
lobby you are already in just repeats `LobbyEntered`.

### 5. Enter the lobby and connect

`LobbyEntered { lobby }` means Steam put you in the lobby. Read the data the host wrote and
connect with your own networking. The lobby half of the backend (`backend.0.lobby()`, `Some` for
both the real and the fake backend) has query methods that are safe to call from any system:

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn on_entered(
    mut entered: MessageReader<LobbyEntered>,
    backend: Res<SteamBackendRes>,
    mut leave: MessageWriter<LeaveLobby>,
) {
    let Some(b) = backend.0.lobby() else { return };
    for ev in entered.read() {
        let host = b.lobby_data(ev.lobby, "host").and_then(|h| h.parse::<u64>().ok());
        let version_ok = b.lobby_data(ev.lobby, "version").as_deref() == Some("1");
        match host {
            Some(host) if version_ok && is_individual_steam_id64(host) => {
                info!("lobby {} ({} members): connecting to host {host}", ev.lobby, b.lobby_member_count(ev.lobby));
                // Start your client transport here (section 10: bevy_net_session or bevy_replicon).
            }
            _ => {
                warn!("lobby {} is not joinable by this build", ev.lobby);
                leave.write(LeaveLobby);
            }
        }
    }
}
```

A failed join is a `LobbyError` with `LobbyErrorKind::JoinFailed` (the lobby is gone, full, or
you may not join it).

### 6. Leave

`LeaveLobby` leaves the current lobby (writing `LobbyLeft { lobby }`), abandons any create or join
still in flight, and clears the rich presence the kit set. It is safe to send at any time, also
with no lobby and no backend (then it does nothing), so a game can send it unconditionally when a
session ends.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn on_session_end(mut leave: MessageWriter<LeaveLobby>) {
    leave.write(LeaveLobby);
}
```

On `AppExit` the kit leaves by itself (in `Last`), so a friend's list does not keep showing
"Join Game" on a closed game.

### 7. Invite a friend

`InviteFriend { steam_id }` sends a Steam game invite carrying the connect string of the current
lobby. The friend sees it in Steam; accepting it arrives in their game as a `JoinRequested`.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn invite(mut invites: MessageWriter<InviteFriend>) {
    invites.write(InviteFriend { steam_id: 76_561_197_960_265_730 });
}

fn on_invite_sent(mut sent: MessageReader<InviteSent>) {
    for ev in sent.read() {
        // `ok` = the call was made with valid input. There is no delivery receipt.
        info!("invite to {} for lobby {}: {}", ev.steam_id, ev.lobby, if ev.ok { "sent" } else { "refused" });
    }
}
```

- No current lobby: `LobbyErrorKind::NoLobby`, nothing is sent.
- Not an individual SteamID64 (a group, a lobby, a typo): `LobbyErrorKind::InvalidSteamId`.
- The Steam overlay's own "Invite to game" works too; it arrives the same way on the other side.

### 8. Rich presence

Rich presence is what friends see about you. `connect` is managed by the kit for lobbies you
create; any other key is yours:

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn presence(mut set: MessageWriter<SetRichPresence>, mut clear: MessageWriter<ClearRichPresence>) {
    set.write(SetRichPresence { key: "status".into(), value: Some("In the menu".into()) });
    set.write(SetRichPresence { key: "status".into(), value: None }); // remove one key
    clear.write(ClearRichPresence); // remove every key
}
```

A refused update (Steam said no, or a key/value containing a NUL byte) is a `LobbyError` with
`LobbyErrorKind::PresenceFailed`. Steam's own limits apply (short keys and values, a small number
of keys). What the friends list *displays* is controlled by Steam's `steam_display` key and the
rich-presence localization file configured for your app in Steamworks; `connect` is what powers
"Join Game".

### 9. Errors

Every failed request is a `LobbyError { kind, message }`. The `message` is an ASCII detail for
logs; show your own text to players. `LobbyErrorKind` is `#[non_exhaustive]`, so keep a `_` arm.

| kind | cause | show the player? |
|---|---|---|
| `CreateFailed` | Steam could not create the lobby | yes: "friends cannot join right now" |
| `JoinFailed` | Steam could not put you in the lobby (gone, full, not allowed) | yes: "could not join" |
| `NoBackend` | no `SteamBackendRes` (Steam not running or not compiled in), or a backend without lobby support | yes, if the player asked for a Steam action |
| `NoLobby` | an invite with no current lobby | yes, for an invite button |
| `InvalidSteamId` | an invite to something that is not a user's SteamID64 | yes, for typed ids |
| `AlreadyInLobby` | a create while in (or entering) a lobby | usually a bug in the calling code |
| `InvalidRequest` | a request field was unusable (lobby id 0) | no, log it |
| `PresenceFailed` | Steam refused a rich-presence update | no, log it |

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn on_error(mut errors: MessageReader<LobbyError>) {
    for err in errors.read() {
        match err.kind {
            LobbyErrorKind::JoinFailed => info!("toast: Could not join - the host may have left"),
            LobbyErrorKind::CreateFailed => info!("toast: Friends cannot join via Steam right now"),
            _ => warn!("steam lobby: {:?}: {}", err.kind, err.message),
        }
    }
}
```

### 10. Wiring a transport (bevy_replicon + renet_steam)

The kit stops at "you are in the lobby, here is its data". Connecting is yours.

**The short way: [`bevy_net_session`](https://crates.io/crates/bevy_net_session).** A companion
crate that hosts, joins and leaves `bevy_replicon` sessions over Steam P2P (or plain UDP for LAN
tests) with the same code, adds a version-checked join handshake, a join validator, timeouts and a
clean teardown, and manages the `renet_steam` transport for you. It uses the same `steamworks`
0.12.2 and never pumps Steam, so the kit's pump serves both. Add both crates:

```text
cargo add bevy_steam_kit --features lobby,steam
cargo add bevy_net_session --features steam
```

```rust,ignore
use bevy::prelude::*;
use bevy_net_session::*;
use bevy_steam_kit::*;

// Setup (once Steam is initialised): both crates share the one client; only the kit pumps.
// app.add_plugins((SteamKitPlugin::default().with_lobby(LobbySettings::default()), NetSessionPlugin::default()))
//    .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client.clone()))))
//    .insert_resource(SteamNetClient(client));

/// Host: once the session is up, open a lobby that names this host.
fn open_lobby(mut started: MessageReader<SessionStarted>, mut lobby: MessageWriter<CreateLobby>) {
    for ev in started.read() {
        let Some(me) = ev.steam_id else { continue };
        lobby.write(CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: vec![("host".into(), me.to_string())] });
    }
}

/// Joiner: in the lobby, connect to the host it names.
fn connect(mut entered: MessageReader<LobbyEntered>, backend: Res<SteamBackendRes>, mut join: MessageWriter<JoinSession>) {
    for ev in entered.read() {
        let host = backend.0.lobby().and_then(|l| l.lobby_data(ev.lobby, "host")).and_then(|h| h.parse::<u64>().ok());
        if let Some(host) = host {
            join.write(JoinSession::steam(host));
        }
    }
}
```

The host sends `HostSession::steam(4)` first; a joiner answers `JoinRequested` with `JoinLobby`
as in section 4. Do not add `bevy_net_session`'s example pump: the kit already pumps. More:
the `bevy_net_session` README, section "Combining it with Steam lobbies".

**The manual way.** Wiring `renet_steam` yourself works too. This recipe uses `bevy_replicon`
0.44, `bevy_replicon_renet` 0.20 and its `renet_steam` 3.0.0 transport, which uses the same
`steamworks` 0.12.2 (one copy in the build):

```toml
[dependencies]
bevy_steam_kit = { version = "0.2.0", features = ["lobby", "steam"] }
steamworks = "=0.12.2"
bevy_replicon = "0.44"
bevy_replicon_renet = { version = "0.20", features = ["renet_steam"] }
```

`RepliconRenetPlugins` adds the Steam transport plugins by itself when `renet_steam` is on. The
host listens on Steam P2P and puts its SteamID64 into the lobby; a joiner reads it and connects.
The transport gets its Steam callbacks from the kit's pump; do not pump for it.

```rust,ignore
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::steam::{AccessPermission, SteamClientTransport, SteamServerConfig, SteamServerTransport};
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RenetServer};
use bevy_steam_kit::*;

/// Your own handle on the Steam client (insert it next to the kit's backend at startup).
#[derive(Resource, Clone)]
struct Steam(steamworks::Client);

/// The lobby data key the host writes its SteamID64 into (your choice of name).
const KEY_HOST: &str = "host";

fn connection_config(channels: &RepliconChannels) -> ConnectionConfig {
    ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    }
}

/// Host (an exclusive system, e.g. on entering your "hosting" state): listen on Steam P2P,
/// then open the lobby that tells friends where to connect.
fn start_hosting(world: &mut World) {
    let Some(steam) = world.get_resource::<Steam>().cloned() else { return };
    let server = RenetServer::new(connection_config(world.resource::<RepliconChannels>()));
    let config = SteamServerConfig { max_clients: 3, access_permission: AccessPermission::FriendsOnly };
    let transport = match SteamServerTransport::new(steam.0.clone(), config) {
        Ok(transport) => transport,
        Err(e) => {
            error!("could not listen on Steam: {e:?}");
            return;
        }
    };
    world.insert_resource(server);
    // NON-SEND: bevy_renet's Steam server systems read `NonSendMut<SteamServerTransport>`.
    // `insert_resource` also compiles, but then the transport is never pumped (silently).
    world.insert_non_send(transport);

    let host = steam.0.user().steam_id().raw();
    world.write_message(CreateLobby {
        kind: LobbyKind::FriendsOnly,
        max_members: 4, // the host + max_clients
        data: vec![(KEY_HOST.to_string(), host.to_string()), ("version".to_string(), "1".to_string())],
    });
}

/// Joiner: in the lobby -> read the host's SteamID64 and connect to it.
fn connect_on_entered(
    mut entered: MessageReader<LobbyEntered>,
    backend: Res<SteamBackendRes>,
    steam: Res<Steam>,
    channels: Res<RepliconChannels>,
    mut commands: Commands,
    mut leave: MessageWriter<LeaveLobby>,
) {
    for ev in entered.read() {
        let host = backend.0.lobby().and_then(|l| l.lobby_data(ev.lobby, KEY_HOST)).and_then(|h| h.parse::<u64>().ok());
        let Some(host) = host.filter(|&h| is_individual_steam_id64(h)) else {
            warn!("lobby {} has no usable host", ev.lobby);
            leave.write(LeaveLobby);
            continue;
        };
        match SteamClientTransport::new(steam.0.clone(), &steamworks::SteamId::from_raw(host)) {
            Ok(transport) => {
                commands.insert_resource(RenetClient::new(connection_config(&channels)));
                // The CLIENT transport is a normal resource.
                commands.insert_resource(transport);
            }
            Err(e) => error!("could not connect to {host}: {e:?}"),
        }
    }
}
```

Things that bite:

- **The server transport is non-send data** (`world.insert_non_send`, removed with
  `world.remove_non_send::<SteamServerTransport>()`); the `RenetServer` next to it and the
  **client** transport are ordinary resources.
- `AccessPermission::FriendsOnly` lets any Steam friend of the host connect.
  `AccessPermission::InLobby(lobby_id)` admits lobby members only; start the server after
  `LobbyCreated` then (`steamworks::LobbyId::from_raw(ev.lobby)`).
- Over Steam, renet's client id of a joiner is its SteamID64.
- Put a build version into the lobby data and check it before connecting: the Steam transport
  has no protocol id of its own.
- The transport is not this crate's code and is not a dependency of it; other stacks work the
  same way (read the host id from the lobby, connect with it).

### 11. System order

| set | schedule | what runs |
|---|---|---|
| `SteamKitSystems::Pump` | `First`, before Bevy's `MessageUpdateSystems` | pump Steam once (the only `process_callbacks` of the process) and buffer the events |
| `SteamKitSystems::Callbacks` | `First`, after `Pump`, before `MessageUpdateSystems` | every feature applies its events. Lobby: completions, the launch-args check; writes `LobbyCreated`, `LobbyEntered`, `JoinRequested`, create/join `LobbyError`s |
| `SteamKitSystems::Callbacks` | (same) | stats: `StatsReady` (Steam reloaded the stats), `StatsStored`, `AchievementUnlocked`, `AchievementProgress`, store `StatsError`s. Leaderboards: `LeaderboardFound`, `ScoreUploaded`, `ScoresDownloaded`, Steam-side `LeaderboardError`s |
| `SteamKitSystems::Callbacks` | (same) | auth: `WebApiTicketReady`, Steam-side `AuthError`s. Friends: `ConnectRequested`, which friends to re-read. Overlay: `OverlayToggled` |
| `SteamKitSystems::Requests` | `Update` | lobby: handle `CreateLobby`, `JoinLobby`, `LeaveLobby`, `InviteFriend`, `SetRichPresence`, `ClearRichPresence`; write `InviteSent`, `LobbyLeft` and request `LobbyError`s. Stats: readiness, the request stream, the store policy. Leaderboards: requests (a cached `Find` is answered here), timeouts, the upload queue. Auth: requests, cancels, timeouts. Friends: re-reads (`SteamFriends`, `FriendsChanged`), avatars, `UserInfoReady`, invites (`GameInviteSent`), `FriendsError`s. Overlay: `OpenOverlay` (`OverlayError`s) |
| `SteamKitSystems::Requests` | `Last` | on `AppExit`: lobby leaves and clears presence; stats applies late requests, reports held writes, stores a last time; leaderboards answers everything waiting with `Exiting`; auth answers waiting requests with `Exiting` and cancels live tickets |

| your system | order it |
|---|---|
| writes a request message (`CreateLobby`, `StatsRequest`, `LeaderboardRequest`, ...) and wants it handled this frame | `.before(SteamKitSystems::Requests)` in `Update` (otherwise it is handled next frame, which is also fine) |
| calls `SteamLeaderboards::next_id()` / `SteamAuth::next_id()` (takes `ResMut<..>`) | `.before(SteamKitSystems::Requests)` in `Update` (strict ambiguity checks need an order against the kit's systems) |
| reads `SteamFriends` or `SteamOverlay` in `Update` | order it against `SteamKitSystems::Requests` (`.after` to see this frame's re-read) |
| reads facts from Steam (`LobbyCreated`, `LobbyEntered`, `JoinRequested`, `StatsStored`, `AchievementUnlocked`, `ScoreUploaded`, ...) | anywhere from `PreUpdate` on: they are written in `First`, in the frame Steam delivered them |
| reads what the request handler writes (`InviteSent`, `LobbyLeft`, request errors) | `.after(SteamKitSystems::Requests)` to see them this frame, otherwise next frame |
| runs in `First` | order it against `SteamKitSystems::Pump` / `Callbacks` and against Bevy's `MessageUpdateSystems` (strict ambiguity checks) |
| reads `SteamLobby` | anywhere; it changes in `First`, `Update` and `Last` (`generation` bumps on every change) |
| writes `AppExit` | anywhere before `Last`: every feature's exit handling (leave the lobby, the final stats store, answering waiting leaderboard and auth requests, cancelling live tickets) runs in `SteamKitSystems::Requests` in `Last` |

### 12. Testing your game with the fake backend

`FakeSteamBackend` is an in-memory Steam. Clones share state, so the test keeps one clone to
drive and inspect while the kit owns another. Creates complete on the next pump with ids
1000, 1001, ...; joins succeed on the next pump; both can be switched to manual or failing.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

let fake = FakeSteamBackend::new();
let mut app = App::new();
app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
    .insert_resource(SteamBackendRes(Box::new(fake.clone())));

// A friend's invite arrives from "Steam".
fake.put_lobby_data(42, "host", "76561197960265730");
fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 42, from: 76_561_197_960_265_730 });
app.update();

// Your game's reaction would send JoinLobby; here the test does.
app.world_mut().write_message(JoinLobby { lobby: 42 });
app.update();
app.update();

assert_eq!(app.world().resource::<SteamLobby>().current, Some(42));
assert!(fake.calls().contains(&FakeCall::JoinLobby(42)));
assert_eq!(fake.pump_count(), 3); // one pump per frame
```

Other knobs: `set_auto_complete_create(false)` + `complete_create(id)` / `fail_create(msg)`,
`set_join_succeeds(false)`, `set_member_count`, `set_friend_name`, `set_launch_command_line` (a
cold launch), `set_local_id`, `rich_presence(key)`, `pump_count()` and the full `calls()` log.
Build test apps with `MinimalPlugins`; no Steam client or SDK runtime is involved.

## Stats and achievements

Feature `stats`: the local user's Steam **stats** (`INT` and `FLOAT`) and **achievements**, as
Bevy messages, with the store cadence Valve asks for handled by the kit. Everything below uses
`use bevy::prelude::*; use bevy_steam_kit::*;` and the `stats` feature.

### Set up

```rust
use std::time::Duration;
use bevy::prelude::*;
use bevy_steam_kit::*;

let mut app = App::new();
app.add_plugins((
    MinimalPlugins,
    SteamKitPlugin::default().with_stats(StatsSettings {
        // A stat or achievement your app defines, used to detect that Steam has loaded the stats.
        probe: Some("NumGames".into()),
        stats_store_interval: Duration::from_secs(120),
        ..Default::default()
    }),
));
```

The same `SteamBackendRes` as for lobbies turns it on ([section 2](#2-turn-steam-on));
`RealSteamBackend` and `FakeSteamBackend` both support stats.

| `StatsSettings` field | default | meaning |
|---|---|---|
| `probe` | `None` | a stat or achievement API name used to detect that stats are loaded. `None`: ready once Steam reports at least one achievement for the app, or once a held write succeeds. **Set it if your app has stats but no achievements** |
| `probe_interval` | 1 s | how often readiness is probed while not ready |
| `stats_store_interval` | 60 s | stat changes are stored this long after the first unsaved change |
| `achievement_store_delay` | 1 s | achievement changes are stored this long after the first one (a burst of unlocks is one store; the unlock popup stays prompt) |
| `min_store_gap` | 10 s | never two stores closer than this |
| `store_timeout` | 30 s | a store whose outcome never arrives is given up (`StoreTimedOut`) and the changes are stored again |
| `store_on_exit` | `true` | store unsaved changes on `AppExit` |
| `max_queued` | 256 | writes held while stats are not ready |

The cadence uses Bevy's `Time<Real>` (from `TimePlugin`, part of `MinimalPlugins` and
`DefaultPlugins`). Without it readiness is probed every frame and only `StoreStats` and the exit
store run (no batching, no minimum gap, no store timeout).

### Write stats and unlock achievements

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn on_match_won(mut stats: MessageWriter<StatsRequest>) {
    stats.write(StatsRequest::add_stat("NumGames", StatValue::I32(1))); // an INT stat
    stats.write(StatsRequest::set_stat("MaxFeetTraveled", StatValue::F32(512.5))); // a FLOAT stat
    stats.write(StatsRequest::unlock_achievement("ACH_WIN_ONE_GAME"));
}

fn on_unlocked(mut unlocked: MessageReader<AchievementUnlocked>) {
    for ev in unlocked.read() {
        // Confirmed by Steam; the overlay showed its popup.
        info!("achievement {} unlocked", ev.name);
    }
}
```

- Every request is a `StatsRequest` (one message type), so requests are applied **in the order
  they were written**, whatever their kind: `AddStat` then `SetStat` ends with the set value,
  `UnlockAchievement` then `ResetAllStats` ends reset. Write the variants directly
  (`StatsRequest::SetStat { name, value }`) or with the helpers `set_stat`, `add_stat`,
  `unlock_achievement`, `clear_achievement`, `indicate_achievement_progress`.
- Write requests before `SteamKitSystems::Requests` in `Update` to have them applied the same
  frame. A request written after that set in the frame the game exits is still applied before
  the exit store.
- The value's variant must match the stat's type on the partner site (`INT` = `StatValue::I32`,
  `FLOAT` = `StatValue::F32`); a mismatch, an unknown name, or a value Steam does not take is a
  `StatsError` with `StatsErrorKind::Refused`.
- `AddStat` reads, adds and writes (an `I32` sum saturates).
- `UnlockAchievement` on an unlocked achievement does nothing. `ClearAchievement` locks one again
  (for development).
- `IndicateAchievementProgress { name, current, max }` shows Steam's "current / max" progress
  popup; it sets and unlocks nothing (not even at `current == max`: bind the achievement to a stat
  on the partner site for automatic unlocks). `current` is clamped to `max`; `max == 0` is refused.
- `ResetAllStats { achievements_too }` resets the local user's stats (and achievements) for this
  app. For development only.

### Read current values

Reads are synchronous and cheap; ask the backend's stats half from any system:

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn show_progress(backend: Res<SteamBackendRes>, stats: Res<SteamStats>) {
    if !stats.is_ready() {
        return;
    }
    let Some(s) = backend.0.stats() else { return };
    let games = s.get_stat("NumGames", StatKind::I32);
    let won = s.achievement("ACH_WIN_ONE_GAME");
    info!("games {games:?}, first win {won:?}, unsaved changes: {}", stats.has_unsaved());
}
```

`SteamStats` (read-only): `is_ready()`, `has_unsaved()`, `store_in_flight()`, `queued()`,
`stores_started()` (store attempts, counting one Steam refused locally). `StatsReady` is written
when stats become ready, and again whenever Steam
reloads the local user's stats (read your values again then).

### Readiness: there is no "request stats" step

In this Steam SDK (1.62, bundled with `steamworks` 0.12.2) `RequestCurrentStats` no longer exists:
the Steam client loads the stats before the game starts. So the kit does not wait for a callback;
it **probes** readiness (with `probe`, about once a second) and becomes ready on the first
success, or when Steam reports the local user's stats. Writes sent before that are **held** in a
bounded queue (`max_queued`) and applied in order once ready. A held write is never dropped
silently: when the queue is full the oldest is dropped with `StatsErrorKind::NotReady`, and writes
still held at `AppExit` are reported the same way.

### Store cadence

A write changes Steam's in-memory copy only. Sending it (and showing an unlock popup) takes a
store, which Valve rate-limits and asks to call "on the order of minutes, rather than seconds". The
kit batches:

- achievement changes: one store `achievement_store_delay` (1 s) after the first one;
- stat changes: one store `stats_store_interval` (60 s) after the first one;
- `StatsRequest::StoreStats`: as soon as allowed;
- never two stores within `min_store_gap` (10 s), and one store in flight at a time;
- on `AppExit` (in `Last`): a final store if anything is unsaved (Steam also stores on a clean
  exit).

Outcomes: `StatsStored` on success; `AchievementUnlocked` / `AchievementProgress` per achievement
Steam confirmed; `StatsError` with `StoreRejected` when a stat broke a constraint set on the
partner site (Steam restores its values: read them again), `StoreFailed` / `StoreTimedOut` (the
changes are stored again), `StoreRefused` when Steam refused the store locally.

### Guarded traps

`steamworks` 0.12.2 panics on some inputs, and a panic ends the game. The kit checks them first:

| trap | guard |
|---|---|
| a name with a NUL byte (`CString::new(..).unwrap()` in every stat and achievement call) | refused with `StatsErrorKind::InvalidName` before Steam is called (also empty names and names over 127 bytes, `MAX_API_NAME_BYTES`; see `is_valid_api_name`) |
| `get_achievement_names()` panics for an app with no achievements | never called |
| NaN / infinite floats (Steam's behaviour is undocumented) | refused with `StatsErrorKind::NotFinite` |
| progress with `max == 0` | refused with `StatsErrorKind::InvalidRequest` |

Achievement API names are ASCII, as the partner site defines them: `steamworks` 0.12.2 converts
the names it receives inside the callback pump as UTF-8.

### Testing stats with the fake backend

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

let fake = FakeSteamBackend::new();
fake.define_stat("NumGames", StatValue::I32(0));
fake.define_achievement("ACH_WIN_ONE_GAME", false);
let mut app = App::new();
app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
    .insert_resource(SteamBackendRes(Box::new(fake.clone())));

app.world_mut().write_message(StatsRequest::add_stat("NumGames", StatValue::I32(1)));
app.world_mut().write_message(StatsRequest::StoreStats);
app.update();
app.update();

assert_eq!(fake.stat("NumGames"), Some(StatValue::I32(1)));
assert!(fake.calls().contains(&FakeCall::StoreStats));
```

Other knobs: `set_stats_ready(false)` (as if Steam had not loaded the stats), `fail_next_store(..)`
with `FakeStoreFailure::{Refused, Rejected, Failed, NoAnswer}`, `achieved(name)`. Drive time with
Bevy's `TimeUpdateStrategy::ManualDuration` to test the store cadence.

### Testing stats with real Steam (app 480)

Valve's test app 480 ("Spacewar") defines the stats `NumGames`, `NumWins`, `NumLosses` (INT),
`FeetTraveled`, `MaxFeetTraveled` (FLOAT), `AverageSpeed` (AVGRATE, readable as a float but not
settable here) and the achievements `ACH_WIN_ONE_GAME`, `ACH_WIN_100_GAMES`,
`ACH_TRAVEL_FAR_ACCUM`, `ACH_TRAVEL_FAR_SINGLE`. `cargo run --example stats_480 --features
stats,steam` reads and prints them; `-- --play` changes some and stores; `-- --progress` shows a
progress popup. Everything changed shows on **your own** Spacewar profile. Spacewar's stats only go
up: Steam refuses a `SetStat` that lowers one, so `-- --reset-stats` (every stat back to 0) is the
way to undo `--play`; `ClearAchievement` undoes an unlock.

## Leaderboards

Feature `leaderboards`: find (or create) Steam leaderboards, upload the player's scores, and
download entries, as Bevy messages. It does not need the `stats` feature. Everything below uses
`use bevy::prelude::*; use bevy_steam_kit::*;` and the `leaderboards` feature.

### Requests and answers

Every `LeaderboardRequest` carries a `LeaderboardRequestId`; take a fresh one from
`SteamLeaderboards::next_id()` (the recommended way: never an id that is pending). Every accepted
request gets exactly one answer with its id: `LeaderboardFound`, `ScoreUploaded`,
`ScoresDownloaded` or `LeaderboardError`. The one exception: a request sent with an id that is
still pending is rejected with `DuplicateId` (carrying that id) while the pending request still
gets its own answer, so that id then sees two messages; `next_id` never causes this. If you pick
ids by hand, keep them unique while pending, and when mixing with `next_id` use ids at or above
`LeaderboardRequestId::FIRST_MANUAL` (the kit never issues those). Boards are named; the kit finds
each one once and caches its handle, so an upload or download to a board it has not found before
finds it first (never creating it).

Requests still waiting are always answered: with `NoBackend` when the backend is removed, and with
`Exiting` on `AppExit` (write `AppExit` before `Last`).

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

/// Order your request writers `.before(SteamKitSystems::Requests)`.
fn on_run_finished(mut ids: ResMut<SteamLeaderboards>, mut boards: MessageWriter<LeaderboardRequest>) {
    // Time in milliseconds; details are up to 64 extra values stored with the score.
    boards.write(LeaderboardRequest::UploadScore {
        id: ids.next_id(),
        board: "Quickest Win".into(),
        score: 83_250,
        details: vec![3, 1],
        method: UploadMethod::KeepBest,
    });
}

fn on_uploaded(mut uploaded: MessageReader<ScoreUploaded>, mut ids: ResMut<SteamLeaderboards>, mut boards: MessageWriter<LeaderboardRequest>) {
    for up in uploaded.read() {
        info!("rank {} (was {}), new best: {}", up.rank_new, up.rank_previous, up.changed);
        boards.write(LeaderboardRequest::download(ids.next_id(), up.board.clone(), ScoreRange::AroundUser { before: 2, after: 2 }));
    }
}

fn on_downloaded(mut downloaded: MessageReader<ScoresDownloaded>) {
    for d in downloaded.read() {
        for e in &d.entries {
            info!("#{} {} {}", e.rank, e.steam_id, e.score);
        }
    }
}
```

| request | answer | notes |
|---|---|---|
| `Find { id, name }` | `LeaderboardFound { id, info }` or `NotFound` | `info`: name, handle, sort, display, entry count; also cached in `SteamLeaderboards` |
| `FindOrCreate { id, name, sort, display }` | `LeaderboardFound` | creates a missing board; a board made this way is not shown on the Steam Community site until it is configured on the partner site |
| `UploadScore { id, board, score, details, method }` | `ScoreUploaded { id, board, score, changed, rank_new, rank_previous }` | `KeepBest` keeps the better score by the board's sort; `ForceUpdate` always replaces; `rank_previous` 0 = no entry before |
| `DownloadScores { id, board, range, max_details }` | `ScoresDownloaded { id, board, range, entries, entry_count }` | entries best first: `rank`, `steam_id`, `score`, `details` (cut to `max_details`, at most 64) |

`ScoreRange`: `Global { first, last }` (ranks, from 1), `AroundUser { before, after }` (the
player's entry and its neighbours; empty when the player has no entry), `Friends` (the player and
their Steam friends). Helpers: `LeaderboardRequest::find`, `find_or_create`, `upload`, `download`.

### Limits and queueing

| `LeaderboardSettings` field | default | meaning |
|---|---|---|
| `timeout` | 30 s | a Steam call without an answer is given up (`TimedOut`); a late answer is dropped |
| `uploads_per_window` / `upload_window` | 10 / 600 s | Valve's rate limit: 10 uploads per 10 minutes |
| `max_queued_uploads` | 64 | uploads waiting their turn; more are refused with `QueueFull` |
| `max_download_rows` | 500 | the most entries one download may ask for (capped at `i32::MAX`) |

Uploads run **one at a time** (Valve: one outstanding call) and at most `uploads_per_window` per
`upload_window`; the rest wait in order. Downloads run at once, several at a time. Without Bevy's
`TimePlugin` there are no timeouts and no upload window (still one upload at a time); calls
started before a clock appears are timed from the moment it does.

### Guards and errors

Checked before Steam is called: a board name that is empty, longer than 127 bytes or contains a
NUL byte (`InvalidName`; `steamworks` would panic on the NUL; the kit keeps to 127 bytes, inside
the SDK's limit of 128); more than 64 details (`TooManyDetails`); an unusable
range (`InvalidRange`: `Global` from rank 0, reversed or past `i32::MAX`, or more rows than
`max_download_rows`); an id still pending (`DuplicateId`, answering only the rejected
duplicate). Downloads always leave room
for 64 details per entry (a smaller buffer is unsound in `steamworks` 0.12.2), and the
around-user window's negative start is passed the way `steamworks` needs it.

Errors say what Steam said, nothing more: `NotFound`, `IoFailure` (the only failure `steamworks`
0.12.2 reports), `UploadRejected` (Steam did not accept the upload and gives no reason, for
example a "trusted" board that only takes scores from a server), `TimedOut`, and the kit's own
`NoBackend`, `Exiting`, `QueueFull`.

### Testing leaderboards

With the fake backend: `add_leaderboard(name, sort, display)`, `add_leaderboard_entry(board,
steam_id, score, details)`, `set_friends(ids)`, `fail_next_leaderboard_call(..)` with
`FakeLeaderboardFailure::{IoFailure, Rejected, NoAnswer}`, `leaderboard_entries(board)`. In the
fake, ties rank by earlier upload and an around-user window is clipped at the board's edges.

With real Steam, app 480 ("Spacewar") has the board "Feet Traveled" (descending, numeric, shared
by everyone testing with 480). `cargo run --example leaderboard_480 --features
leaderboards,steam` finds it and downloads the top 10, your friends and the entries around you
(read only); `-- --upload 1` uploads a score of 1 with `KeepBest` (public, and a game cannot
delete it) and shows the neighbourhood. The example never creates a board.

## Web API tickets (auth)

Feature `auth`: a ticket that proves to **your game server** who the player is. The game asks
Steam for a ticket for an identity (a name of the service, agreed with it), sends the ticket as
hex to the server, and the server checks it with Steam's Web API
(`ISteamUserAuth/AuthenticateUserTicket`, with the same identity) and gets the player's SteamID.
Everything below uses `use bevy::prelude::*; use bevy_steam_kit::*;` and the `auth` feature.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

/// The id of the login ticket, to cancel it once the server answered.
#[derive(Resource, Default)]
struct Login(Option<AuthRequestId>);

/// Order request writers `.before(SteamKitSystems::Requests)`.
fn start_login(mut auth: ResMut<SteamAuth>, mut login: ResMut<Login>, mut requests: MessageWriter<AuthRequest>) {
    let id = auth.next_id();
    login.0 = Some(id);
    requests.write(AuthRequest::web_api_ticket(id, "my-game-server"));
}

fn on_ticket(mut ready: MessageReader<WebApiTicketReady>, mut errors: MessageReader<AuthError>) {
    for r in ready.read() {
        let _body = format!("ticket={}", r.ticket.to_hex()); // POST it to your server
    }
    for e in errors.read() {
        warn!("no ticket: {:?} ({})", e.kind, e.message);
    }
}

/// When the server answered (success or not).
fn server_answered(mut login: ResMut<Login>, mut requests: MessageWriter<AuthRequest>) {
    if let Some(id) = login.0.take() {
        requests.write(AuthRequest::cancel(id));
    }
}
```

| request (`AuthRequest`, one ordered stream) | answer |
|---|---|
| `WebApiTicket { id, identity }` (helper `web_api_ticket(id, identity)`) | `WebApiTicketReady { id, identity, ticket }` or `AuthError { id, kind, message }` |
| `Cancel { id }` (helper `cancel(id)`) | none for a delivered ticket (it is cancelled at Steam); a request still waiting is cancelled and answered `AuthError` with `Cancelled`; an unknown id is ignored |

- Every accepted `WebApiTicket` gets exactly one answer. Ids work like the leaderboards' ones:
  `SteamAuth::next_id()` never returns an id that is pending or live; hand-picked ids stay at or
  above `AuthRequestId::FIRST_MANUAL` when mixing. An id still in use is rejected with
  `DuplicateId` (answering only the rejected request).
- `WebApiTicket`: `bytes()`, `to_hex()` (lowercase, two characters per byte), `len()`,
  `is_empty()`. It is a credential: its `Debug` prints `WebApiTicket(<n> bytes)` only, there is no
  `Display`, the kit's logs name the identity and the length only, and the `WebApiTicket` value
  overwrites its bytes with zeros when it is dropped; copies you make (such as the hex string)
  are yours to handle. Send it to your server and nowhere else.
- A ticket stays valid at Steam until it is cancelled or the session ends; cancel it once the
  server answered. On `AppExit` the kit cancels every live ticket (`cancel_on_exit`) and answers
  waiting requests with `Exiting`.
- `AuthErrorKind`: `NoBackend`, `InvalidIdentity` (a NUL byte; `steamworks` would panic on it),
  `DuplicateId`, `Failed` (Steam's failure text in `message`, or Steam refused the request at
  once, for example when not logged on),
  `TimedOut` (no answer within `timeout`; the request is cancelled), `Cancelled`, `Exiting`.
- Only tickets the kit requested are matched: a ticket your game requests on its own `Client`
  clone is left to your code.

| `AuthSettings` field | default | meaning |
|---|---|---|
| `timeout` | 30 s | a request without an answer is given up and cancelled (`TimedOut`; needs Bevy's `TimePlugin`) |
| `cancel_on_exit` | `true` | cancel every live ticket on `AppExit` |

`SteamAuth` (read): `is_pending(id)`, `is_live(id)`, `live_tickets()`, and `next_id()`.

Testing: the fake backend answers on the next pump with the bytes
`FakeSteamBackend::fake_web_api_ticket(op)` (`FAKE-TICKET-<op>`, the kit's ops count 1, 2, 3, ...);
`fail_next_auth_ticket(FakeAuthFailure::{Refused, Failed, NoAnswer})`, `live_auth_tickets()` (the
identities not cancelled). With real Steam: `cargo run --example auth_480 --features auth,steam`
requests a ticket and prints only its length and timing.

## Friends

Feature `friends`: the local user's friends as a resource, invites with any connect string, and
join requests as raw connect strings. Everything below uses `use bevy::prelude::*; use
bevy_steam_kit::*;` and the `friends` feature.

### The list

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

/// Order it `.after(SteamKitSystems::Requests)` in `Update` to see this frame's re-read.
fn friends_panel(friends: Res<SteamFriends>, mut changed: MessageReader<FriendsChanged>) {
    for c in changed.read() {
        info!("friends: +{} -{} ~{}", c.added.len(), c.removed.len(), c.changed.len());
    }
    for f in friends.online() {
        let playing = if f.plays(friends.app_id()) { " (in this game)" } else { "" };
        info!("{} {:?}{playing}", f.display_name(), f.state);
    }
}
```

The kit reads the list when a friends-capable backend appears and keeps `SteamFriends` current:
Steam's `PersonaStateChange` callback for a friend re-reads that friend in the same frame's
`Update`, and the whole list is re-read every `refresh_interval` (5 s; a friend's rich presence
changes send no callback) and on `RefreshFriends`. Each difference is one `FriendsChanged { added,
removed, changed }` (at most one per frame; the first read lists everyone as `added`; the local
user's id is in `changed` when `me()` changed). Removing the backend empties the list (one
`FriendsChanged` with everyone `removed`). `me()` holds the local user's state as friends see it
(`Online`, `Away`, `Invisible`, ...), re-read on the local user's persona change and on every
full re-read. Each full re-read reads every friend from Steam's local cache (no network
traffic).

- `SteamFriends`: `is_loaded()`, `list()` (sorted by SteamID64), `get(id)`, `online()`,
  `playing_this_game()`, `me()`, `app_id()`, `generation()`.
- `FriendInfo`: `steam_id`, `name`, `nickname`, `state: PersonaState`, `game: Option<FriendGame>`
  (`app_id`, `lobby` (0 = none), `server`), `connect` (the friend's rich-presence `connect`, read
  only while they play this game); `display_name()`, `is_online()`, `plays(app_id)`.
- `PersonaState`: `Offline`, `Online`, `Busy`, `Away`, `Snooze`, `LookingToTrade`,
  `LookingToPlay`, `Invisible`, `Unknown`; `is_online()`. Read without `steamworks`' own
  conversion, which panics on states it does not know (such as `Invisible`).
- Names are personal data: the kit logs counts, never names.

### Invites and joins

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn invite(mut invites: MessageWriter<InviteToGame>) {
    invites.write(InviteToGame { steam_id: 76_561_197_960_265_730, connect: "+connect 203.0.113.7:7777".into() });
}

fn on_join(mut joins: MessageReader<ConnectRequested>) {
    for j in joins.read() {
        // The string the inviter sent (or the friend's rich-presence `connect`), as it arrived.
        info!("join via {:?}: {:?}", j.source, j.connect);
    }
}
```

- `InviteToGame { steam_id, connect }` sends a Steam game invite with any connect string (1 to
  255 bytes, no NUL; a user's SteamID64). `GameInviteSent { steam_id, connect, ok }` follows;
  `ok` is Steam's own answer to the call, not a delivery receipt (Steam answers `true` also for
  a friend who is invisible and for an id that is not a friend). The lobby feature's `InviteFriend` stays the way to invite to the kit's
  own lobby.
- `ConnectRequested { connect, from, source }` is every join that reaches the running game:
  an accepted invite, or "Join Game" on a friend whose rich presence has `connect`
  (`ConnectSource::RichPresence`; `from` = the friend, `0` when unknown). "Join Game" on a friend
  who is in a Steam lobby (such as a kit host) arrives as a lobby join instead: the `lobby`
  feature's `JoinRequested` with `LobbyInvite`, and no `ConnectRequested`. With
  `launch_connect_prefix` set, a game Steam **started** with the string (a cold launch) reports it
  once on the first frame (`ConnectSource::LaunchArgs`; the text from the prefix to the end of the
  process arguments, or of Steam's launch command line).
- With the `lobby` feature also enabled, a `+connect_lobby <id>` join is reported by both: a
  `JoinRequested` and a `ConnectRequested`.
- `RequestUserInfo { steam_id, name_only }` loads a non-friend's persona (for example a
  leaderboard row). Every request gets exactly one answer: `UserInfoReady { steam_id, name }`
  once Steam has it (at once for a friend or a user Steam already knows), or a `FriendsError`
  with `TimedOut` (after `user_info_timeout`), `NoBackend`, `InvalidSteamId`, or `Exiting` (the
  app exited first: answered in `Last` on `AppExit`). Two requests for the same user get two
  answers.
- `FriendsError { request, steam_id, kind, message }`: `request` is `FriendsRequestKind::Invite`
  or `UserInfo`, `steam_id` the user the request named; `kind`: `NoBackend`, `InvalidSteamId`,
  `InvalidConnect`, `TimedOut`, `Exiting`.

### Avatars

With `avatars: Some(AvatarSize::Small | Medium | Large)` (32, 64 or 184 pixels), the kit writes
`FriendAvatar { steam_id, width, height, rgba }` once for every friend and the local user, and
again when Steam reports a new avatar (at most 16 per frame; an avatar Steam has none of, for
example a large one it is still loading, is read again on the next full re-read). An image whose
byte count is not `width * height * 4` is skipped. The bytes are RGBA, row by row; for a Bevy
`Image`:

```rust,ignore
let image = Image::new(
    Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 },
    TextureDimension::D2,
    a.rgba.clone(),
    TextureFormat::Rgba8UnormSrgb,
    RenderAssetUsages::RENDER_WORLD,
);
```

| `FriendsSettings` field | default | meaning |
|---|---|---|
| `refresh_interval` | 5 s | re-read the whole list this often (needs Bevy's `TimePlugin`; without it only callbacks and `RefreshFriends` re-read; `Duration::ZERO` = every frame) |
| `read_connect` | `true` | read the rich-presence `connect` of friends playing this game |
| `launch_connect_prefix` | `None` | on the first frame, look for this token in the launch arguments and report a `ConnectRequested` with `LaunchArgs` |
| `avatars` | `None` | read avatars of this size |
| `user_info_timeout` | 10 s | a `RequestUserInfo` Steam has not answered after this long gets a `FriendsError` with `TimedOut` (needs Bevy's `TimePlugin`) |

Testing: `add_friend(id, state)`, `remove_friend(id)`, `set_friend_state(id, state)`,
`set_friend_game(id, Some((app_id, lobby)))`, `set_friend_rich_presence(id, key, value)` (no
event, like Steam: the next refresh finds it), `set_friend_nickname(id, nickname)`,
`set_local_persona(name, state)`, `set_friend_avatar(id, width, height, rgba)`,
`set_friend_avatar_loading(id, loading)`, `set_user_info_loaded(id)`, `silence_next_user_info()`,
`refuse_next_game_invite()`, `set_app_id(id)` and `push_rich_presence_join(from, connect)` (what
Steam delivers for a join, for every compiled feature). Names come from `set_friend_name`. The
leaderboards fake's `set_friends` (who counts for `ScoreRange::Friends`) is separate. With real
Steam: `cargo run --example friends_480 --features friends,steam` prints counts only.

## The Steam overlay

Feature `overlay`: open the overlay and know when it is open. Everything below uses `use
bevy::prelude::*; use bevy_steam_kit::*;` and the `overlay` feature.

```rust
use bevy::prelude::*;
use bevy_steam_kit::*;

fn buy_button(mut open: MessageWriter<OpenOverlay>) {
    open.write(OpenOverlay::store(480)); // your app id, or a DLC's
}

fn invite_button(mut open: MessageWriter<OpenOverlay>) {
    open.write(OpenOverlay::invite_dialog_connect("+connect 203.0.113.7:7777"));
}

fn pause(overlay: Res<SteamOverlay>, mut toggled: MessageReader<OverlayToggled>) {
    for t in toggled.read() {
        info!("overlay open: {} (now {})", t.active, overlay.is_active());
    }
}
```

| `OpenOverlay` | opens |
|---|---|
| `Dialog { dialog }` (`dialog(name)`) | a Steam dialog: `"friends"`, `"community"`, `"players"`, `"settings"`, `"officialgamegroup"`, `"stats"`, `"achievements"` |
| `User { dialog, steam_id }` (`user(name, id)`) | a dialog about a user: `"steamid"` (profile), `"chat"`, `"jointrade"`, `"stats"`, `"achievements"`, `"friendadd"`, `"friendremove"`, `"friendrequestaccept"`, `"friendrequestignore"` |
| `WebPage { url }` (`web_page(url)`) | a web page in the overlay's browser |
| `Store { app_id, flag }` (`store(app_id)`) | an app's store page; `StoreFlag::{None, AddToCart, AddToCartAndShow}` |
| `InviteDialog { lobby }` (`invite_dialog(lobby)`) | the invite dialog for a Steam lobby |
| `InviteDialogConnect { connect }` (`invite_dialog_connect(s)`) | the invite dialog sending a connect string (arrives as `ConnectRequested` with the `friends` feature) |

- An unusable request (an empty name, URL or connect string, a NUL byte, a connect string over
  255 bytes, a SteamID64 that is not a user's, lobby id 0) is an `OverlayError` with
  `InvalidRequest` and never reaches `steamworks` (which would panic on the NUL). Steam gives no
  answer for an accepted request; a call the backend refuses is an `OverlayError` with `Refused`.
- `OverlayToggled { active }` is written in `First` whenever the overlay opens or closes;
  `SteamOverlay` has `is_active()`, `is_enabled()` (Steam reports the overlay as available to this
  process, shortly after start) and `toggles()`.
- The overlay needs Steam's "Enable the Steam Overlay while in-game" setting on and a game window
  Steam draws into; `is_enabled()` tells whether Steam made it available.
- Without a backend the overlay counts as closed: removing the backend while it is open writes one
  `OverlayToggled { active: false }`.

Testing: `set_overlay_enabled(bool)`, `toggle_overlay(active)`, `refuse_next_overlay()`; every request the kit passes on is
recorded as `FakeCall::ActivateOverlay(request)`. With real Steam: `cargo run --example
overlay_480 --features overlay,steam`.

## How it works

**The pump.** One core system in `First` (`SteamKitSystems::Pump`, before Bevy swaps the message
buffers) calls the backend's `pump()`. The real backend calls `Client::process_callbacks` exactly
once: callbacks the features care about (for `lobby`: `GameLobbyJoinRequested`,
`GameRichPresenceJoinRequested`) are mapped to events, and the completion closures of async calls
(`create_lobby` / `join_lobby`), which run inside that same call, push their results into a small
queue that the pump drains before returning. The core keeps the frame's events in a buffer
(replaced on every pump, cleared without a backend, so nothing is ever delivered twice).

**The features.** Right after the pump, in `SteamKitSystems::Callbacks` (still before the buffer
swap, so everything they write is readable in the same frame's `PreUpdate` / `Update`), each
compiled feature reads its own events from the buffer and turns them into state changes and fact
messages. No feature ever pumps Steam. The only callbacks the kit registers are its three
guards (below), registered for the rest of the process.

<a id="when-steam-quits"></a>**When Steam quits.** When the Steam client quits it sends
`SteamServersDisconnected` with the result "OK", and from then on the kit makes no call into
Steam's library. `steamworks` 0.12.2 panics on that "OK" while converting the callback, so
`RealSteamBackend::new` registers its own callback for `SteamServersDisconnected` and
`SteamServerConnectFailure`; `steamworks` runs it on the raw callback data before its own
conversion. When it sees "OK" it turns it into "no connection" (so the conversion never panics)
and marks Steam as shutting down. Only the result "OK" does that; any other result code changes
nothing. Call results, every other callback and your own registrations are untouched. From that
moment:

- the kit makes no Steam call of any kind (only the pump that delivered the callback finishes
  `steamworks`' dispatch: the callbacks and call results already handed to the game, including
  the game's own registered closures): it never pumps again, no feature reads or writes Steam
  (also not on `AppExit`), and the backend's `local_id()` and `launch_command_line()` answer from
  values read at its creation (`friend_name()` returns `""`);
- the other events of that pump are dropped, and the kit writes `SteamLost { reason:
  SteamLostReason::SteamExited }` once, in the same frame (in `First`, readable in that frame's
  `Update`). `SteamBackendRes` stays (a system taking `Res<SteamBackendRes>` keeps working), with
  an inert backend that supports no feature;
- every request is answered with its feature's `NoBackend` error, including the ones still
  waiting for Steam (tickets, leaderboard calls, user info); the friends list is emptied (one
  `FriendsChanged` with everyone `removed`); an open overlay is reported closed; the lobby feature
  clears `SteamLobby` (the lobby this process was in is reported with one `LobbyLeft`) and answers
  a create or join still in flight with one `LobbyError` with `NoBackend`;
- the kit does not shut Steam's library down and keeps its own `steamworks::Client` clone alive
  until the process ends (dropping the last clone calls into Steam);
- the game keeps running and decides what to do. **After `SteamLost`, make no further calls
  through your own `steamworks::Client` either.** The kit does not reconnect when Steam is started
  again, also not through a `RealSteamBackend` created later in the same process; the game's next
  start uses the new Steam client.

**When the Steam client process ends** without that callback (killed or crashed), the real
backend sees it through the operating system, never through Steam: before every pump and before
every feature's access to Steam it checks whether the Steam client process still runs (a check
that never blocks). When the process is gone, the same happens as above, with
`SteamLostReason::SteamProcessEnded`. How the process is found:

- **Windows:** the pid Steam writes to `HKCU\Software\Valve\Steam\ActiveProcess\pid`; the
  process must be `steam.exe`. The kit keeps a handle to that process, so a reused pid is never
  mistaken for Steam.
- **Linux:** the pid in `~/.steam/steam.pid` (or `~/.steam/steam/steam.pid`); the process must be
  `steam`, and its start time must stay the same (a reused pid counts as gone).
- Where no such pid file names the Steam client (macOS, a Flatpak or Snap Steam; logged once at
  start), the kit uses Steam's own process check instead, at most once a second, before the pump.

A Steam client that is restarted while the game runs is a new process: the kit counts Steam as
gone.

**Connect strings that are not UTF-8.** `steamworks` 0.12.2 converts the connect string of a
"Join Game" or an accepted invite (`GameRichPresenceJoinRequested`) with `expect`, so a string
that is not UTF-8 (or has no terminating NUL) would panic in the pump, and any friend can send
one. The kit's third guard fixes the raw string in place before that conversion: every byte that
is not part of valid UTF-8 becomes `?` (and a string without NUL is cut to 255 bytes). The join
then arrives with that string (`ConnectRequested`; the lobby feature ignores it unless it still
parses as `+connect_lobby <id>`).

**Panics inside the pump.** The guards prevent `steamworks`' panics before they happen, so they
work in every build, `panic = "abort"` included. Any other panic inside the pump is caught by the
kit in a build that unwinds on panic (Rust's default): that frame's Steam events are dropped and
the same happens as above, with `SteamLostReason::PumpPanicked`; no `RealSteamBackend` of that
process pumps again. The kit then makes no further Steam call, so the lobby feature's `LobbyLeft`
does not leave the Steam lobby or clear the rich presence at Steam. With `panic = "abort"` such a
panic ends the process.

**Lobby requests** are handled in `Update`, in a fixed order: leave, join, create, invite, rich
presence, clear. That makes "leave + create" or "leave + join" in one frame behave as expected.
**Stats requests** are one message type, applied in the order written. **Leaderboard requests**
are answered exactly once each, by request id.

**Auth, friends and the overlay over Steam.** Their callbacks (`GetTicketForWebApiResponse`,
`PersonaStateChange`, `GameRichPresenceJoinRequested`, `GameOverlayActivated`) are mapped inside
the same pump. A ticket answer is matched to its request by Steam's ticket handle; a ticket the
kit did not request is ignored. Persona states and friends' rich presence are read through
`steamworks`' raw bindings (the `friends` feature enables `raw-bindings`), because the wrapped
persona-state call panics on states it does not know and friends' rich presence is not wrapped.

**Stats and leaderboards over Steam.** Stats callbacks (`UserStatsReceived`, `UserStatsStored`,
`UserAchievementStored`, for the running app only) are mapped inside the same pump; leaderboard
calls are call results whose closures only queue events (a Steam call from inside such a closure
would deadlock `steamworks`). The progress popup uses the raw `IndicateAchievementProgress` (not
wrapped by `steamworks` 0.12.2; the `stats` feature enables `steamworks`' `raw-bindings` for it).

**Ghost lobbies.** A create or join can complete after the game gave up on it (a `LeaveLobby` or a
new `JoinLobby` while it was in flight). The kit counts abandoned creates and leaves such a lobby
the moment it arrives; it never becomes `current` and no data is written to it. A lobby created or
entered without a matching request is left the same way.

**Launch arguments.** On the first frame with a lobby-capable backend, the process arguments and
Steam's launch command line are searched for `<prefix> <id>` (also `<prefix>=<id>`, quoted or not);
a match is one `JoinRequested` with `from: 0` and `JoinSource::LaunchArgs`.

**Guards.** `steamworks` 0.12.2 panics on a few inputs, and a panic inside a Bevy system ends the
game. The real backend refuses keys, values, identities, dialog names, URLs and connect strings
with an interior NUL byte
(`CString::new(..).unwrap()` inside steamworks) and reports them as failures; `max_members` is
clamped to 250 (steamworks asserts); a join request with lobby id 0 or an unparsable connect
string is ignored with a warning; a non-user `from` id becomes `0`. The lobby chat-update callback
is never touched (its conversion is not total in this steamworks version).

**Presence bookkeeping.** The kit remembers whether this process set rich presence (the automatic
`connect` or any `SetRichPresence` with a value) and clears it on leave and on exit.

**Logging.** Every state change of every feature is logged through `tracing` with a `>>> STEAM:`
prefix (visible with Bevy's `LogPlugin`), e.g. `>>> STEAM: lobby 109775241000000000 open`,
`>>> STEAM: stats stored`, `>>> STEAM: leaderboard "Feet Traveled" found (1916472 entries)`.

## API reference

The full documentation is generated with `cargo doc --open --all-features`. Everything is
available at the crate root (`use bevy_steam_kit::*;`); the feature items also live in the
`bevy_steam_kit::lobby`, `bevy_steam_kit::stats`, `bevy_steam_kit::leaderboards`,
`bevy_steam_kit::auth`, `bevy_steam_kit::friends` and `bevy_steam_kit::overlay` modules.

### Core (always compiled)

| item | kind | what / example |
|---|---|---|
| `SteamKitPlugin` | plugin | `app.add_plugins(SteamKitPlugin::default())`; feature settings through builders: `.with_lobby(LobbySettings { .. })` (feature `lobby`), `.with_stats(StatsSettings { .. })` (feature `stats`), `.with_leaderboards(LeaderboardSettings { .. })` (feature `leaderboards`), `.with_auth(AuthSettings { .. })` (feature `auth`), `.with_friends(FriendsSettings { .. })` (feature `friends`) |
| `SteamKitSystems::{Pump, Callbacks, Requests}` (`#[non_exhaustive]`) | system sets | `my_system.before(SteamKitSystems::Requests)` ([order](#11-system-order)) |
| `SteamBackendRes(pub Box<dyn SteamBackend>)` | resource | the active backend; its presence makes the kit live. `SteamBackendRes(Box::new(FakeSteamBackend::new()))`. After `SteamLost` it holds an inert backend ([when Steam quits](#when-steam-quits)) |
| `SteamLost { reason: SteamLostReason }` (`#[non_exhaustive]`) | fact message | Steam is gone for this process; written once, in `First` ([when Steam quits](#when-steam-quits)) |
| `SteamLostReason` (`#[non_exhaustive]`) | enum | `SteamExited` (the Steam client quit and said so), `SteamProcessEnded` (the client process ended without saying so: killed or crashed), `PumpPanicked` (a panic inside the pump was caught; only in a build that unwinds) |
| `SteamBackend` trait | trait | `local_id() -> u64`, `friend_name(id) -> String` (for a user Steam knows nothing about, real Steam returns `"[unknown]"` and the fake backend `""`), `launch_command_line() -> String`, `pump() -> Vec<BackendEvent>` (the kit calls it; never call it yourself), one accessor per feature, each defaulting to `None`: `lobby() -> Option<&dyn LobbyBackend>` (feature `lobby`), `stats() -> Option<&dyn StatsBackend>` (feature `stats`), `leaderboards() -> Option<&dyn LeaderboardBackend>` (feature `leaderboards`), `auth() -> Option<&dyn AuthBackend>` (feature `auth`), `friends() -> Option<&dyn FriendsBackend>` (feature `friends`), `overlay() -> Option<&dyn OverlayBackend>` (feature `overlay`). `Send + Sync + 'static`; must never panic |
| `BackendEvent` (`#[non_exhaustive]`) | enum | what a backend's `pump` returns. Always: `SteamLost { reason }` (the kit takes it out and writes `SteamLost`). With `lobby`: `LobbyCreated { lobby }`, `LobbyCreateFailed { message }`, `LobbyEntered { lobby }`, `LobbyJoinFailed { lobby }`, `LobbyJoinRequested { lobby, from }`, `RichPresenceJoinRequested { from, connect }`. With `stats`: `StatsReceived { user, ok }`, `StatsStored`, `StatsStoreRejected`, `StatsStoreFailed { message }`, `AchievementStored { name, current, max }`. With `leaderboards`: `LeaderboardFound { op, board }`, `LeaderboardNotFound { op }`, `LeaderboardScoreUploaded { op, score, changed, rank_new, rank_previous }`, `LeaderboardUploadRejected { op }`, `LeaderboardScoresDownloaded { op, entries }`, `LeaderboardIoFailure { op }`. With `auth`: `WebApiTicket { op, result: Result<WebApiTicket, String> }`. With `friends`: `PersonaChanged { steam_id, flags }` (Steam's raw `EPersonaChange` bits), `ConnectRequested { from, connect }`. With `overlay`: `OverlayActivated { active }` |
| `RealSteamBackend` (feature `steam`) | backend | `RealSteamBackend::new(client: steamworks::Client)`: the backend over steamworks 0.12.2 (supports every compiled feature); registers the three callbacks of its guards (Steam quitting, non-UTF-8 connect strings) for the rest of the process; after `SteamLost` a backend created later in the same process stays inert |
| `FakeSteamBackend` | backend | in-memory Steam for tests: `new()`, `calls()`, `pump_count()`, `set_local_id(id)`, `push_event(BackendEvent)`, `set_friend_name(id, name)`, `set_launch_command_line(text)`, `set_app_id(id)` (default 480), `simulate_steam_shutdown()` (what Steam's shutdown callback does: no feature from then on, `SteamLost` on the next pump), `simulate_steam_process_ended()` (the same for a killed client), `simulate_steam_exit()` (the next pump reports `SteamLost`), `panic_in_next_pump()`; with `lobby` or `friends` also `push_rich_presence_join(from, connect)`; with `auth` also `FakeSteamBackend::fake_web_api_ticket(op)`, `fail_next_auth_ticket(FakeAuthFailure)`, `live_auth_tickets()`; with `friends` also `add_friend(id, state)`, `remove_friend(id)`, `set_friend_state(id, state)`, `set_friend_game(id, game)`, `set_friend_rich_presence(id, key, value)`, `set_friend_nickname(id, nickname)`, `set_local_persona(name, state)`, `set_friend_avatar(id, w, h, rgba)`, `set_friend_avatar_loading(id, loading)`, `set_user_info_loaded(id)`, `silence_next_user_info()`, `refuse_next_game_invite()`; with `overlay` also `set_overlay_enabled(bool)`, `toggle_overlay(active)`, `refuse_next_overlay()`; with `lobby` also `set_auto_complete_create(bool)`, `complete_create(lobby)`, `fail_create(msg)`, `set_join_succeeds(bool)`, `set_member_count(lobby, n)`, `put_lobby_data(lobby, key, value)`, `rich_presence(key)`; with `stats` also `define_stat(name, StatValue)`, `define_achievement(name, unlocked)`, `set_stats_ready(bool)`, `fail_next_store(FakeStoreFailure)`, `stat(name)`, `achieved(name)`; with `leaderboards` also `add_leaderboard(name, sort, display)`, `add_leaderboard_entry(board, steam_id, score, details)`, `set_friends(ids)`, `fail_next_leaderboard_call(FakeLeaderboardFailure)`, `leaderboard_entries(board)` |
| `FakeCall` (`#[non_exhaustive]`) | enum | one recorded call. With `lobby` or `friends`: `InviteToGame { friend, connect }`. With `lobby`: `CreateLobby { kind, max_members }`, `JoinLobby(lobby)`, `LeaveLobby(lobby)`, `SetLobbyData { lobby, key, value }`, `SetLobbyJoinable { lobby, joinable }`, `SetRichPresence { key, value }`, `ClearRichPresence`. With `auth`: `RequestWebApiTicket { identity }`, `CancelAuthTicket { op }`. With `friends`: `RequestUserInformation { id, name_only }`. With `overlay`: `ActivateOverlay(OpenOverlay)`. With `stats`: `SetStat { name, value }`, `UnlockAchievement(name)`, `ClearAchievement(name)`, `IndicateAchievementProgress { name, current, max }`, `StoreStats`, `ResetAllStats { achievements_too }`. With `leaderboards`: `FindLeaderboard { name, create }`, `UploadScore { board, method, score, details }`, `DownloadScores { board, range }` |
| `is_individual_steam_id64(id) -> bool` | function | a user account in the public universe: `is_individual_steam_id64(76561197960265730) == true`, `is_individual_steam_id64(12345) == false` |

Writing your own backend: implement `SteamBackend` (and, per feature, `LobbyBackend`,
`StatsBackend`, `LeaderboardBackend`, `AuthBackend`, `FriendsBackend` or `OverlayBackend`,
returning `Some(self)` from the matching accessor). A backend that leaves an accessor at its
default keeps compiling when that feature is enabled elsewhere in the build; the kit then treats
that feature as unavailable (`NoBackend`). **Stability promise:** the methods of `SteamBackend`
and of the feature traits stay required as they are, and every method added to them (including
the accessor of a new feature) comes with a default implementation, so your implementation keeps
compiling.

### Lobby: settings, state, backend (feature `lobby`)

| item | kind | what / example |
|---|---|---|
| `LobbySettings { connect_prefix, set_connect_presence, check_launch_args }` | settings + resource (read) | `SteamKitPlugin::default().with_lobby(LobbySettings { .. })` ([fields](#1-add-the-plugin)) |
| `SteamLobby { current, pending_create, pending_join, generation }` | resource (read) | `lobby.current == Some(id)`; never write it |
| `LobbyBackend` trait | trait | `create_lobby(kind, max)`, `join_lobby(lobby)`, `leave_lobby(lobby)`, `set_lobby_data(lobby, key, value) -> bool`, `lobby_data(lobby, key) -> Option<String>`, `lobby_member_count(lobby) -> usize`, `set_lobby_joinable(lobby, bool) -> bool`, `set_rich_presence(key, Option<&str>) -> bool`, `clear_rich_presence()`, `invite_to_game(friend, connect) -> bool`. Reached with `backend.0.lobby()` |
| `MAX_LOBBY_MEMBERS` | const `u32` = 250 | Steam's member cap; `CreateLobby::max_members` is clamped to it |

`SteamLobby` fields: `current: Option<u64>` (the lobby this process is in), `pending_create: bool`
(a create is in flight), `pending_join: Option<u64>` (a join is in flight for this lobby),
`generation: u32` (bumped, wrapping, on every change).

The `LobbyBackend` query methods (`lobby_data`, `lobby_member_count`) are meant for your systems:
`backend.0.lobby().and_then(|l| l.lobby_data(lobby, "host"))`. Leave the mutating ones to the kit
(it keeps `SteamLobby` in sync).

### Lobby: request messages (you write)

| message | fields | example |
|---|---|---|
| `CreateLobby` | `kind: LobbyKind`, `max_members: u32` (clamped `1..=250`), `data: Vec<(String, String)>` | `CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 4, data: vec![] }` |
| `JoinLobby` | `lobby: u64` | `JoinLobby { lobby: req.lobby }` |
| `LeaveLobby` | none (`Default`) | `leave.write(LeaveLobby)` |
| `InviteFriend` | `steam_id: u64` (an individual SteamID64) | `InviteFriend { steam_id: friend }` |
| `SetRichPresence` | `key: String`, `value: Option<String>` (`None` removes the key) | `SetRichPresence { key: "status".into(), value: Some("Hosting".into()) }` |
| `ClearRichPresence` | none (`Default`) | `clear.write(ClearRichPresence)` |

### Lobby: fact messages (you read)

| message | fields | written when |
|---|---|---|
| `LobbyCreated` | `lobby: u64` | a lobby this process created is ready (data, joinable and `connect` already set) |
| `LobbyEntered` | `lobby: u64` | this process joined a lobby after `JoinLobby` (or re-sent `JoinLobby` for the current lobby) |
| `JoinRequested` | `lobby: u64`, `from: u64` (`0` = unknown), `source: JoinSource` | someone asked this process to join a lobby; nothing is joined |
| `LobbyLeft` | `lobby: u64` | this process left a lobby (`LeaveLobby`, a new `JoinLobby`, `AppExit`) |
| `InviteSent` | `steam_id: u64`, `lobby: u64`, `ok: bool` | an `InviteFriend` was passed to Steam (`ok` = Steam accepted the call) |
| `LobbyError` | `kind: LobbyErrorKind`, `message: String` | a request failed ([kinds](#9-errors)) |

### Lobby: enums and helpers

| item | what / example |
|---|---|
| `LobbyKind` (`#[non_exhaustive]`) | `Private`, `FriendsOnly` (default), `Public`, `Invisible` |
| `JoinSource` (`#[non_exhaustive]`) | `LobbyInvite`, `RichPresence`, `LaunchArgs` ([when](#4-react-to-join-requests)) |
| `LobbyErrorKind` (`#[non_exhaustive]`) | `CreateFailed`, `AlreadyInLobby`, `JoinFailed`, `NoLobby`, `NoBackend`, `InvalidSteamId`, `InvalidRequest`, `PresenceFailed` |
| `connect_string(prefix, lobby) -> String` | `connect_string("+connect_lobby", 9) == "+connect_lobby 9"` |
| `parse_connect_lobby(text, prefix) -> Option<u64>` | finds `<prefix> <id>` or `<prefix>=<id>` anywhere in `text`: `parse_connect_lobby("game.exe +connect_lobby 31", "+connect_lobby") == Some(31)`; `None` when missing, malformed or `0` |

### Stats (feature `stats`)

| item | kind | what / example |
|---|---|---|
| `StatsSettings { probe, probe_interval, stats_store_interval, achievement_store_delay, min_store_gap, store_timeout, store_on_exit, max_queued }` | settings + resource (read) | `SteamKitPlugin::default().with_stats(StatsSettings { .. })` ([fields](#set-up)) |
| `SteamStats` | resource (read) | `is_ready()`, `has_unsaved()`, `store_in_flight()`, `queued()`, `stores_started()` |
| `StatsRequest` (`#[non_exhaustive]`) | request message (one ordered stream) | `SetStat { name, value }`, `AddStat { name, delta }`, `UnlockAchievement { name }`, `ClearAchievement { name }`, `IndicateAchievementProgress { name, current, max }` (the popup; sets nothing), `StoreStats`, `ResetAllStats { achievements_too }`; helpers `StatsRequest::add_stat("NumGames", StatValue::I32(1))` etc.; `name()` |
| `StatsReady`, `StatsStored` (read-only, `#[non_exhaustive]` like every stats answer) | fact messages | stats loaded (again); a store succeeded |
| `AchievementUnlocked { name }`, `AchievementProgress { name, current, max }` | fact messages | confirmed by Steam |
| `StatsError { kind: StatsErrorKind, name: Option<String>, message }` | fact message | a request or store failed |
| `StatsErrorKind` (`#[non_exhaustive]`) | enum | `NoBackend`, `InvalidName`, `NotFinite`, `InvalidRequest`, `Refused`, `NotReady`, `StoreRefused`, `StoreRejected`, `StoreFailed`, `StoreTimedOut` |
| `StatValue` (`#[non_exhaustive]`, `PartialEq`) / `StatKind` (`#[non_exhaustive]`) | enums | `StatValue::I32(i32)`, `StatValue::F32(f32)`; `StatKind::{I32, F32}`; `value.kind()` |
| `StatsBackend` trait | trait | `is_ready(probe)`, `get_stat(name, kind) -> Option<StatValue>`, `set_stat(name, value) -> bool`, `achievement(name) -> Option<bool>`, `unlock_achievement`, `clear_achievement`, `indicate_achievement_progress`, `store_stats() -> bool`, `reset_all_stats`. Reached with `backend.0.stats()`; same stability promise as `LobbyBackend` |
| `FakeStoreFailure` (`#[non_exhaustive]`) | enum | `Refused`, `Rejected`, `Failed`, `NoAnswer`, for `FakeSteamBackend::fail_next_store` |
| `is_valid_api_name(name)`, `MAX_API_NAME_BYTES` = 127 | function, const | the name check applied before every Steam call |

The stats systems run in the same sets as the lobby's: pumped events in `SteamKitSystems::Callbacks`
(`First`), requests, readiness and stores in `SteamKitSystems::Requests` (`Update`), the exit store
in `SteamKitSystems::Requests` (`Last`).

### Leaderboards (feature `leaderboards`)

| item | kind | what / example |
|---|---|---|
| `LeaderboardSettings { timeout, uploads_per_window, upload_window, max_queued_uploads, max_download_rows }` | settings + resource (read) | `SteamKitPlugin::default().with_leaderboards(LeaderboardSettings { .. })` ([limits](#limits-and-queueing)) |
| `SteamLeaderboards` | resource | `next_id() -> LeaderboardRequestId` (`&mut self`: the recommended ids), `board(name) -> Option<&LeaderboardInfo>`, `is_pending(id)`, `uploads_queued()`, `upload_in_flight()` |
| `LeaderboardRequest` (`#[non_exhaustive]`) | request message | `Find { id, name }`, `FindOrCreate { id, name, sort, display }`, `UploadScore { id, board, score, details, method }`, `DownloadScores { id, board, range, max_details }`; helpers `find`, `find_or_create`, `upload`, `download`; `id()`, `board()` |
| `LeaderboardRequestId(pub u64)`, `LeaderboardRequestId::FIRST_MANUAL` (`2^63`) | id | from `next_id()` or picked by hand (at or above `FIRST_MANUAL` when mixing); every answer carries it |
| `LeaderboardFound { id, info: LeaderboardInfo }`, `ScoreUploaded { .. }`, `ScoresDownloaded { .. }` | answer messages | [fields](#requests-and-answers) |
| `LeaderboardError { id, board, kind: LeaderboardErrorKind, message }` | answer message | a request failed |
| `LeaderboardErrorKind` (`#[non_exhaustive]`) | enum | `NoBackend`, `InvalidName`, `TooManyDetails`, `InvalidRange`, `DuplicateId`, `NotFound`, `UploadRejected`, `IoFailure`, `TimedOut`, `QueueFull`, `Refused`, `Exiting` |
| `LeaderboardInfo { name, handle, sort, display, entry_count }` (`#[non_exhaustive]`), `LeaderboardEntry { rank, steam_id, score, details }` (constructible, for custom backends) | data | the answer and info structs are read-only (`#[non_exhaustive]`) |
| `LeaderboardSort`, `LeaderboardDisplay`, `UploadMethod`, `ScoreRange` (all `#[non_exhaustive]`) | enums | `Ascending`/`Descending`; `Numeric`/`TimeSeconds`/`TimeMilliseconds`; `KeepBest`/`ForceUpdate`; `Global { first, last }`/`AroundUser { before, after }`/`Friends` |
| `LeaderboardBackend` trait | trait | `find_leaderboard(op, name, create)`, `upload_score(op, board, method, score, details)`, `download_scores(op, board, range)`, `leaderboard_sort/_display/_entry_count(board)`. Reached with `backend.0.leaderboards()`; same stability promise as the other backend traits |
| `FakeLeaderboardFailure` (`#[non_exhaustive]`) | enum | `IoFailure`, `Rejected`, `NoAnswer` |
| `is_valid_leaderboard_name(name)`, `MAX_LEADERBOARD_NAME_BYTES` = 127, `MAX_LEADERBOARD_DETAILS` = 64 | function, consts | the checks applied before every Steam call |

### Auth (feature `auth`)

| item | kind | what / example |
|---|---|---|
| `AuthSettings { timeout, cancel_on_exit }` | settings + resource (read) | `SteamKitPlugin::default().with_auth(AuthSettings { .. })` ([fields](#web-api-tickets-auth)) |
| `SteamAuth` | resource | `next_id() -> AuthRequestId` (`&mut self`), `is_pending(id)`, `is_live(id)`, `live_tickets()` |
| `AuthRequest` (`#[non_exhaustive]`) | request message (one ordered stream) | `WebApiTicket { id, identity }`, `Cancel { id }`; helpers `web_api_ticket(id, identity)`, `cancel(id)`; `id()` |
| `AuthRequestId(pub u64)`, `AuthRequestId::FIRST_MANUAL` (`2^63`) | id | from `next_id()` or picked by hand; every answer carries it |
| `WebApiTicketReady { id, identity, ticket }` (`#[non_exhaustive]`) | answer message | the ticket arrived |
| `AuthError { id, kind: AuthErrorKind, message }` (`#[non_exhaustive]`) | answer message | a request failed |
| `AuthErrorKind` (`#[non_exhaustive]`) | enum | `NoBackend`, `InvalidIdentity`, `DuplicateId`, `Failed`, `TimedOut`, `Cancelled`, `Exiting` |
| `WebApiTicket` | data | `new(bytes)`, `bytes()`, `to_hex()`, `len()`, `is_empty()`; `Clone`, `PartialEq`; `Debug` shows the length only; zeroed on drop |
| `AuthBackend` trait | trait | `request_web_api_ticket(op, identity) -> bool`, `cancel_auth_ticket(op)`. Reached with `backend.0.auth()`; same stability promise as the other backend traits |
| `FakeAuthFailure` (`#[non_exhaustive]`) | enum | `Refused`, `Failed`, `NoAnswer` |

### Friends (feature `friends`)

| item | kind | what / example |
|---|---|---|
| `FriendsSettings { refresh_interval, read_connect, launch_connect_prefix, avatars, user_info_timeout }` | settings + resource (read) | `SteamKitPlugin::default().with_friends(FriendsSettings { .. })` ([fields](#avatars)) |
| `SteamFriends` | resource (read) | `is_loaded()`, `list()`, `get(id)`, `online()`, `playing_this_game()`, `me()`, `app_id()`, `generation()` |
| `FriendInfo { steam_id, name, nickname, state, game, connect }` (`#[non_exhaustive]`) | data | `display_name()`, `is_online()`, `plays(app_id)` |
| `FriendGame { app_id, lobby, server }` (`#[non_exhaustive]`) | data | `FriendGame::new(app_id, lobby, server)` for custom backends |
| `PersonaState` (`#[non_exhaustive]`) | enum | `Offline`, `Online`, `Busy`, `Away`, `Snooze`, `LookingToTrade`, `LookingToPlay`, `Invisible`, `Unknown`; `from_raw(i32)`, `is_online()` |
| `AvatarSize` (`#[non_exhaustive]`) | enum | `Small` (32), `Medium` (64), `Large` (184); `pixels()` |
| `RefreshFriends`, `InviteToGame { steam_id, connect }`, `RequestUserInfo { steam_id, name_only }` | request messages | re-read now; invite with a connect string; load a user's persona |
| `FriendsChanged { added, removed, changed }`, `GameInviteSent { steam_id, connect, ok }`, `ConnectRequested { connect, from, source }`, `UserInfoReady { steam_id, name }`, `FriendAvatar { steam_id, width, height, rgba }` (all `#[non_exhaustive]`) | fact messages | [when](#invites-and-joins) |
| `ConnectSource` (`#[non_exhaustive]`) | enum | `RichPresence`, `LaunchArgs` |
| `FriendsError { request: FriendsRequestKind, steam_id, kind: FriendsErrorKind, message }`, `FriendsRequestKind`, `FriendsErrorKind` (all `#[non_exhaustive]`) | fact message, enums | `Invite`, `UserInfo`; `NoBackend`, `InvalidSteamId`, `InvalidConnect`, `TimedOut`, `Exiting` |
| `FriendsBackend` trait | trait | `current_app_id()`, `friend_ids()`, `persona_name(id)`, `persona_nickname(id)`, `persona_state(id)`, `game_played(id)`, `friend_rich_presence(id, key)`, `local_persona()`, `invite_user_to_game(id, connect) -> bool`, `request_user_information(id, name_only) -> bool`, `friend_avatar(id, size)`. Reached with `backend.0.friends()`; same stability promise |
| `MAX_CONNECT_BYTES` | const `usize` = 255 | Steam's limit on a connect string |

### Overlay (feature `overlay`)

| item | kind | what / example |
|---|---|---|
| `SteamOverlay` | resource (read) | `is_active()`, `is_enabled()`, `toggles()` |
| `OpenOverlay` (`#[non_exhaustive]`) | request message | `Dialog { dialog }`, `User { dialog, steam_id }`, `WebPage { url }`, `Store { app_id, flag }`, `InviteDialog { lobby }`, `InviteDialogConnect { connect }`; helpers `dialog`, `user`, `web_page`, `store`, `invite_dialog`, `invite_dialog_connect` |
| `StoreFlag` (`#[non_exhaustive]`) | enum | `None` (default), `AddToCart`, `AddToCartAndShow` |
| `OverlayToggled { active }` (`#[non_exhaustive]`) | fact message | the overlay opened or closed |
| `OverlayError { request, kind: OverlayErrorKind, message }`, `OverlayErrorKind` (both `#[non_exhaustive]`) | fact message, enum | `NoBackend`, `InvalidRequest`, `Refused` |
| `OverlayBackend` trait | trait | `overlay_enabled() -> bool`, `open_overlay(&OpenOverlay) -> bool`. Reached with `backend.0.overlay()`; same stability promise |

## Compatibility

| bevy_steam_kit | Bevy | steamworks | transport recipe (optional, your dependency) | Rust |
|---|---|---|---|---|
| 0.2 | 0.19.0 | 0.12.2 | bevy_replicon 0.44 / bevy_replicon_renet 0.20 / renet_steam 3.0.0 | 1.95+ |
| 0.1 | 0.19.0 | 0.12.2 | bevy_replicon 0.44 / bevy_replicon_renet 0.20 / renet_steam 3.0.0 | 1.95+ |

**Why steamworks 0.12.2 and not a newer one?** The widely used Steam transport for renet
(`renet_steam` 3.0.0, used by `bevy_replicon_renet` 0.20) requires `steamworks` ^0.12.2. Two
versions of `steamworks` link the same native library and Cargo refuses to build that, so a game
using that transport must stay on 0.12.2, and so does this crate. A change of this pin is a minor
version of this crate.

Platforms: Windows, Linux and macOS, 64-bit (the platforms of `steamworks` 0.12.2).

## Examples

| example | what it shows | needs |
|---|---|---|
| `cargo run --example quick_start --features lobby` | host a lobby, a simulated friend's invite, accept it, enter the friend's lobby and read its data, all printed | nothing (fake backend) |
| `cargo run --example host_lobby --features lobby,steam [-- <friend SteamID64>]` | host a real friends-only lobby, set rich presence, optionally invite a friend | Steam running, `steam_appid.txt` |
| `cargo run --example join_lobby --features lobby,steam [-- <lobby id>]` | join a lobby by id, or on "Join Game" / an invite; print the host's lobby data | Steam running, `steam_appid.txt` |
| `cargo run --example leaderboard_480 --features leaderboards,steam [-- --upload <score> \| --board <name> \| --show-ids]` | find Spacewar's "Feet Traveled" board, download the top 10, your friends and your neighbourhood; optionally upload a score ([details](#testing-leaderboards)) | Steam running |
| `cargo run --example stats_480 --features stats,steam [-- --play \| --round-trip \| --progress \| --reset-stats \| --reset]` | read Spacewar's stats and achievements; change, store, undo ([details](#testing-stats-with-real-steam-app-480)) | Steam running |
| `cargo run --example auth_480 --features auth,steam` | request a Web API ticket, print its length and timing (never the ticket), cancel; request and cancel before the answer | Steam running |
| `cargo run --example friends_480 --features friends,steam [-- --avatars \| --avatars-large \| --invite <SteamID64> \| --user-info <SteamID64> \| +bevy_steam_kit_test 1]` | friends counts by state, players of the app, rich-presence connects, changes, avatars and the slowest frame of the kit's work for 60 s (counts only, no names or ids); optionally an invite with a custom connect string, a user-info request, or a simulated cold launch (put it last) | Steam running (two accounts for the invite) |
| `cargo run --example overlay_480 --features overlay,steam [-- --friends \| --invite-dialog \| --invite-dialog-lobby <lobby id>]` | whether Steam reports the overlay, one overlay request after 3 s, every open / closed event for 60 s | Steam running |
| `cargo run --example steam_exit_480 --features steam,auth,friends` | quit Steam while it runs: it keeps running, prints `SteamLost`, sends two requests and prints their `NoBackend` answers, and exits with code 0 ten seconds later | Steam running, then quit |

The Steam examples read the app id from the `STEAM_APP_ID` environment variable (default 480).
An example exits with a non-zero code when Steam does not start, and the ones that end by
themselves also when a request was answered with an error.

## Testing with real Steam

1. Use **two Steam accounts on two machines** (Steam allows one logged-in account per machine),
   and make the accounts friends.
2. On both: Steam running and logged in, the game initialised with app id `480` (and, as usual
   for development, a `steam_appid.txt` containing `480` in the working directory), then run the
   game (or the examples).
3. **App id 480 is Valve's "Spacewar"** test app: friends see "Spacewar" as the game, invites say
   "Spacewar", and the "Join Game" menu entry is there because rich presence `connect` is set.
4. **With 480, the joining friend must already have the game running.** A "Join Game" or an
   accepted invite then arrives as `JoinRequested`. "Join Game" on a friend who hosts a kit lobby
   arrives with `source: LobbyInvite` (Steam uses the lobby, not the rich-presence `connect`; no
   `ConnectRequested` is written for it); an accepted invite with a connect string arrives with
   `RichPresence` (and as `ConnectRequested` with the `friends` feature). If the game
   is not running, Steam tries to start Spacewar itself instead of your build. Cold launches
   (`JoinSource::LaunchArgs`) can only be tested with your own app id and an installed build.
5. What to look for, in order: the host logs `>>> STEAM: lobby <id> open`; the friend sees you as
   in-game and "Join Game" on you; using it logs `join requested` on their side; after your game
   sends `JoinLobby`, `joined lobby <id>` and `LobbyEntered` with the data you set; `InviteSent`
   with `ok = true` for an invite (the friend gets a Steam notification); quitting the host
   removes "Join Game" within a few seconds.

For `friends`: an invite with a custom connect string (`InviteToGame`) accepted on the second
machine arrives there as `ConnectRequested` with `RichPresence`; a cold launch from an invite
starts Spacewar for 480, so the launch path (`launch_connect_prefix`) is checked by passing the
string as arguments by hand.

Quitting Steam while a game runs: `steam_exit_480` keeps running and prints `SteamLost` and the
`NoBackend` answers ([when Steam quits](#when-steam-quits)).

The crate's own tests never talk to Steam: they drive the kit through `FakeSteamBackend`. CI
checks 23 feature sets: none, `steam`, each feature alone (`lobby`, `stats`, `leaderboards`,
`auth`, `friends`, `overlay`), each feature with `steam`, the pairs `lobby,stats`,
`lobby,leaderboards`, `stats,leaderboards`, `lobby,friends`, `lobby,auth`, `stats,friends`,
`friends,leaderboards`, `friends,overlay`, and all features.

## FAQ

**Does it work with other Steam plugins?** Yes, as long as only the kit pumps Steam callbacks
([the one-pump rule](#the-one-pump-rule)) and everything shares the same `steamworks` version.

**What happens when the host leaves?** Steam hands lobby ownership to another member. Your game
session is hosted by your game's own process, so what happens then is your game's decision.

**Is "invite sent" a delivery receipt?** No, for neither feature. The lobby's
`InviteSent { ok: true }` means the call was made with valid input (`steamworks` 0.12.2 returns
nothing from its invite call). The friends feature's `GameInviteSent { ok }` is Steam's own
result of the call (it calls Steam's flat API directly); `true` still only means Steam accepted
the call: Steam answers `true` also for an invisible friend and for an id that is not a friend.
In both cases the friend can be offline, ignore the invite, or not own the game.

**How do I follow who is in the lobby?** `lobby_member_count(lobby)` on the lobby backend gives
the count; your transport tracks the connections.

**What happens when Steam quits while the game runs?** The game keeps running: the kit writes
`SteamLost` once, stops pumping Steam and answers every request (also the ones still waiting)
with its feature's `NoBackend` error ([details](#when-steam-quits)). From Steam's shutdown
callback on, the kit makes no Steam call at all; stop calling Steam through your own
`steamworks::Client` then too. It does not reconnect to a restarted Steam client.

**Does that need `panic = "unwind"`?** No. The panics `steamworks` 0.12.2 has on Steam's
shutdown callback and on a connect string that is not UTF-8 are prevented before they happen, so
a `panic = "abort"` build keeps running too. Only a panic inside the pump that the kit does not
prevent needs unwinding to be caught; with `panic = "abort"` it ends the process.

**A connect string that is not UTF-8?** It arrives with every invalid byte replaced by `?`:
`steamworks` 0.12.2 would panic on it, so the kit fixes the raw string before `steamworks`
reads it ([details](#when-steam-quits)). The strings this crate writes are always ASCII
(`+connect_lobby` plus a decimal id); keep any custom `connect_prefix` ASCII too.

**Friends: why poll?** Steam sends no callback to this `steamworks` version when a friend's
rich presence changes, so the kit re-reads the list every `refresh_interval` (local Steam cache
reads, no network traffic).
Persona changes (name, status, game, avatar, friendship) arrive by callback and are applied in
the same frame.

**Checking a Web API ticket on the server?** That is your server's code: Steam's
`ISteamUserAuth/AuthenticateUserTicket` with your publisher Web API key (never in the game), the
app id, the ticket hex and the same identity.

**Dedicated servers?** A dedicated server has no Steam user and no friends list; lobbies,
presence, invites, tickets and the overlay are user features. Do not add the backend there.

**Is anything saved or networked by the crate?** No. Lobby state, stats, achievements,
leaderboards and the friends list live on Steam; the crate only mirrors in memory which lobby
this process is in, the friends list it read, the tickets it holds, and what it is still waiting
for.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

## Contributing

Issues and pull requests are welcome. Before opening a pull request, please run (at least for
`--no-default-features`, each single feature, and `--all-features`; CI covers 23 sets):

```text
cargo fmt --all --check
cargo clippy --all-targets <features> -- -D warnings
cargo test <features>
cargo doc --no-deps --all-features
```

Tests must not need a Steam client: drive the kit through `FakeSteamBackend` (see `tests/`).
Keep the README examples compiling (they are checked by `cargo test --all-features`) and add a
line to `CHANGELOG.md`.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
