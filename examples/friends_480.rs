//! The friends list on real Steam with Valve's public test app 480 ("Spacewar").
//!
//! Needs the `steam` and `friends` features and a running, logged-in Steam client. It prints
//! COUNTS only (friends by state, how many play this app, how many have a rich-presence
//! `connect`, how many changes and avatars arrived, the slowest frame of the kit's `Update`
//! work), never a name or a SteamID.
//!
//! ```text
//! cargo run --example friends_480 --features steam,friends                          # watch for 60 s
//! cargo run --example friends_480 --features steam,friends -- --avatars             # also read 64x64 avatars
//! cargo run --example friends_480 --features steam,friends -- --avatars-large       # also read 184x184 avatars
//! cargo run --example friends_480 --features steam,friends -- --invite <SteamID64>   # invite with a custom connect string
//! cargo run --example friends_480 --features steam,friends -- --user-info <SteamID64> # load a user's persona
//! cargo run --example friends_480 --features steam,friends -- +bevy_steam_kit_test 1  # pretend a cold launch
//! ```
//!
//! Two PCs: run it with `--invite <the other account's SteamID64>` on one, plain on the other;
//! accepting the invite there prints `connect requested` with the string
//! `+bevy_steam_kit_test 1` (source `RichPresence` while it runs). A cold launch from the invite
//! starts Spacewar for app 480, so the launch path is checked by passing the string by hand
//! (source `LaunchArgs`). The connect string is everything from `+bevy_steam_kit_test` to the end
//! of the arguments, so put it LAST (after `--invite <id>`, it would contain the id).
//!
//! The example runs for 60 seconds.

use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_steam_kit::*;

const RUN_FOR: Duration = Duration::from_secs(60);
const PREFIX: &str = "+bevy_steam_kit_test";

#[derive(Resource, Default)]
struct Run {
    invite: Option<u64>,
    user_info: Option<u64>,
    user_info_at: f32,
    changes: usize,
    avatars: usize,
    printed_at: f32,
    /// Start of this frame's kit `Update` work, and the slowest one so far.
    kit_started: Option<Instant>,
    slowest: Duration,
    /// An error arrived: exit with an error code.
    failed: bool,
}

fn main() -> AppExit {
    let app_id: u32 = std::env::var("STEAM_APP_ID").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(480);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let invite = id_after("--invite");
    let user_info = id_after("--user-info");
    let avatars =
        if args.iter().any(|a| a == "--avatars-large") { Some(AvatarSize::Large) } else { args.iter().any(|a| a == "--avatars").then_some(AvatarSize::Medium) };

    let client = match steamworks::Client::init_app(app_id) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Steam could not start (is the Steam client running and logged in?): {e}");
            return AppExit::error();
        }
    };
    println!("Steam is up: app {app_id}; invite: {}; user info: {}; avatars: {avatars:?}", invite.is_some(), user_info.is_some());

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 30.0))),
            LogPlugin::default(),
            SteamKitPlugin::default().with_friends(FriendsSettings { launch_connect_prefix: Some(PREFIX.into()), avatars, ..Default::default() }),
        ))
        .insert_resource(SteamBackendRes(Box::new(RealSteamBackend::new(client))))
        .insert_resource(Run { invite, user_info, ..Default::default() })
        .add_systems(Update, kit_start.before(SteamKitSystems::Requests))
        .add_systems(Update, (kit_end, watch, send_requests).chain().after(SteamKitSystems::Requests))
        .add_systems(Update, time_limit)
        .run()
}

fn kit_start(mut run: ResMut<Run>) {
    run.kit_started = Some(Instant::now());
}

fn kit_end(mut run: ResMut<Run>) {
    if let Some(start) = run.kit_started.take() {
        run.slowest = run.slowest.max(start.elapsed());
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct Facts<'w, 's> {
    changed: MessageReader<'w, 's, FriendsChanged>,
    connects: MessageReader<'w, 's, ConnectRequested>,
    sent: MessageReader<'w, 's, GameInviteSent>,
    avatars: MessageReader<'w, 's, FriendAvatar>,
    user_info: MessageReader<'w, 's, UserInfoReady>,
    errors: MessageReader<'w, 's, FriendsError>,
}

fn watch(friends: Res<SteamFriends>, mut run: ResMut<Run>, mut facts: Facts, time: Res<Time<Real>>) {
    let t = time.elapsed().as_secs_f32();
    for c in facts.changed.read() {
        run.changes += 1;
        println!("[{t:6.2}s] friends changed: +{} -{} ~{}", c.added.len(), c.removed.len(), c.changed.len());
    }
    for c in facts.connects.read() {
        println!("[{t:6.2}s] connect requested: {:?} ({:?}, from a friend: {})", c.connect, c.source, c.from != 0);
    }
    for s in facts.sent.read() {
        println!("[{t:6.2}s] invite sent: ok {} ({:?})", s.ok, s.connect);
    }
    for a in facts.avatars.read() {
        run.avatars += 1;
        if run.avatars == 1 {
            println!("[{t:6.2}s] first avatar: {}x{}, {} bytes", a.width, a.height, a.rgba.len());
        }
    }
    for u in facts.user_info.read() {
        // The length only: names are personal data.
        println!("[{t:6.2}s] user info ready: {}-byte name, after {:.2}s", u.name.len(), t - run.user_info_at);
    }
    for e in facts.errors.read() {
        run.failed = true;
        println!("[{t:6.2}s] error {:?} {:?}: {}", e.request, e.kind, e.message);
    }
    if friends.is_loaded() && t - run.printed_at >= 10.0 {
        run.printed_at = t;
        let count = |s: PersonaState| friends.list().iter().filter(|f| f.state == s).count();
        println!(
            "[{t:6.2}s] {} friends: online {}, away {}, busy {}, snooze {}, offline {}, invisible {}, unknown {}; playing app {}: {}; with connect: {}; changes so far {}, avatars {}; slowest kit Update {:.2} ms",
            friends.list().len(),
            count(PersonaState::Online),
            count(PersonaState::Away),
            count(PersonaState::Busy),
            count(PersonaState::Snooze),
            count(PersonaState::Offline),
            count(PersonaState::Invisible),
            count(PersonaState::Unknown),
            friends.app_id(),
            friends.playing_this_game().count(),
            friends.list().iter().filter(|f| f.connect.is_some()).count(),
            run.changes,
            run.avatars,
            run.slowest.as_secs_f64() * 1000.0,
        );
        if let Some(me) = friends.me() {
            println!("           you: {:?}", me.state);
        }
    }
}

fn send_requests(
    mut run: ResMut<Run>,
    friends: Res<SteamFriends>,
    time: Res<Time<Real>>,
    mut invites: MessageWriter<InviteToGame>,
    mut user_info: MessageWriter<RequestUserInfo>,
) {
    if !friends.is_loaded() {
        return;
    }
    if let Some(steam_id) = run.invite.take() {
        println!("sending the invite (connect {PREFIX:?} 1)");
        invites.write(InviteToGame { steam_id, connect: format!("{PREFIX} 1") });
    }
    if let Some(steam_id) = run.user_info.take() {
        println!("requesting the user's persona");
        run.user_info_at = time.elapsed().as_secs_f32();
        user_info.write(RequestUserInfo { steam_id, name_only: true });
    }
}

fn time_limit(run: Res<Run>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>, mut said: Local<bool>) {
    if time.elapsed() > RUN_FOR && !*said {
        *said = true;
        println!("{}s are up - quitting", RUN_FOR.as_secs());
        exit.write(if run.failed { AppExit::error() } else { AppExit::Success });
    }
}
