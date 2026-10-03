//! Lobby feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend). No real Steam call is ever made here.

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;

use crate::*;

/// A fabricated individual SteamID64 (the placeholder the fake backend and the README use too).
const FRIEND: u64 = 76_561_197_960_265_730;

#[derive(Resource)]
struct Seen<T: Message + Clone>(Vec<T>);

fn collect<T: Message + Clone>(mut r: MessageReader<T>, mut seen: ResMut<Seen<T>>) {
    seen.0.extend(r.read().cloned());
}

fn watch<T: Message + Clone>(app: &mut App) {
    app.insert_resource(Seen::<T>(Vec::new())).add_systems(Last, collect::<T>.after(SteamKitSystems::Requests));
}

fn seen<T: Message + Clone>(app: &App) -> Vec<T> {
    app.world().resource::<Seen<T>>().0.clone()
}

fn app_with(backend: Option<&FakeSteamBackend>) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(SteamKitPlugin::default());
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<LobbyCreated>(&mut app);
    watch::<LobbyEntered>(&mut app);
    watch::<JoinRequested>(&mut app);
    watch::<LobbyLeft>(&mut app);
    watch::<InviteSent>(&mut app);
    watch::<LobbyError>(&mut app);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn create_msg() -> CreateLobby {
    CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 9, data: vec![("host".into(), "123".into()), ("version".into(), "7".into())] }
}

fn creates(fake: &FakeSteamBackend) -> usize {
    fake.calls().iter().filter(|c| matches!(c, FakeCall::CreateLobby { .. })).count()
}

fn lobby(app: &App) -> SteamLobby {
    app.world().resource::<SteamLobby>().clone()
}

#[test]
fn create_sets_data_joinable_and_connect_presence() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);

    assert_eq!(seen::<LobbyCreated>(&app), vec![LobbyCreated { lobby: 1000 }]);
    assert_eq!(lobby(&app).current, Some(1000));
    assert!(!lobby(&app).pending_create);
    assert_eq!(fake.lobby_data(1000, "host").as_deref(), Some("123"));
    assert_eq!(fake.lobby_data(1000, "version").as_deref(), Some("7"));
    assert!(fake.calls().contains(&FakeCall::CreateLobby { kind: LobbyKind::FriendsOnly, max_members: 9 }));
    assert!(fake.calls().contains(&FakeCall::SetLobbyJoinable { lobby: 1000, joinable: true }));
    assert_eq!(fake.rich_presence("connect").as_deref(), Some("+connect_lobby 1000"));
    assert!(seen::<LobbyError>(&app).is_empty());
}

#[test]
fn a_duplicate_create_is_refused_and_steam_is_called_once() {
    let fake = FakeSteamBackend::new();
    fake.set_auto_complete_create(false);
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);
    // And again while the first is still pending, in a later frame.
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);

    assert_eq!(creates(&fake), 1);
    let errs = seen::<LobbyError>(&app);
    assert_eq!(errs.len(), 2);
    assert!(errs.iter().all(|e| e.kind == LobbyErrorKind::AlreadyInLobby));

    // Once created, a further create is still refused.
    fake.complete_create(555);
    frames(&mut app, 1);
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);
    assert_eq!(creates(&fake), 1);
    assert_eq!(seen::<LobbyError>(&app).len(), 3);
    assert_eq!(lobby(&app).current, Some(555));
}

#[test]
fn leave_clears_everything_and_a_recreate_makes_exactly_one_lobby() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    assert_eq!(lobby(&app).current, Some(1000));

    app.world_mut().write_message(LeaveLobby);
    frames(&mut app, 2);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(1000)));
    assert!(fake.calls().contains(&FakeCall::ClearRichPresence));
    assert_eq!(lobby(&app).current, None);
    assert_eq!(seen::<LobbyLeft>(&app), vec![LobbyLeft { lobby: 1000 }]);
    assert_eq!(fake.rich_presence("connect"), None);

    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    assert_eq!(creates(&fake), 2);
    assert_eq!(seen::<LobbyCreated>(&app), vec![LobbyCreated { lobby: 1000 }, LobbyCreated { lobby: 1001 }]);
    assert_eq!(lobby(&app).current, Some(1001));
}

#[test]
fn a_create_completing_after_leave_is_left_and_never_current() {
    let fake = FakeSteamBackend::new();
    fake.set_auto_complete_create(false);
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);
    assert!(lobby(&app).pending_create);

    app.world_mut().write_message(LeaveLobby);
    frames(&mut app, 2);
    assert!(!lobby(&app).pending_create);

    fake.complete_create(777);
    frames(&mut app, 2);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(777)));
    assert_eq!(lobby(&app).current, None);
    assert!(seen::<LobbyCreated>(&app).is_empty());
    // No data was ever written to the abandoned lobby.
    assert_eq!(fake.lobby_data(777, "host"), None);

    // Re-host after that: exactly one new lobby, and it becomes current.
    fake.set_auto_complete_create(true);
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    assert_eq!(creates(&fake), 2);
    assert_eq!(lobby(&app).current, Some(1000));
}

#[test]
fn backend_join_requests_become_messages_with_their_source() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 42, from: FRIEND });
    fake.push_event(BackendEvent::RichPresenceJoinRequested { from: FRIEND, connect: "+connect_lobby 42".into() });
    fake.push_event(BackendEvent::RichPresenceJoinRequested { from: FRIEND, connect: "+connect_lobby banana".into() });
    fake.push_event(BackendEvent::RichPresenceJoinRequested { from: FRIEND, connect: "garbage".into() });
    frames(&mut app, 2);

    assert_eq!(
        seen::<JoinRequested>(&app),
        vec![
            JoinRequested { lobby: 42, from: FRIEND, source: JoinSource::LobbyInvite },
            JoinRequested { lobby: 42, from: FRIEND, source: JoinSource::RichPresence },
        ]
    );
    // The plugin never joins by itself.
    assert!(!fake.calls().iter().any(|c| matches!(c, FakeCall::JoinLobby(_))));
}

#[test]
fn launch_args_are_reported_once() {
    let fake = FakeSteamBackend::new();
    fake.set_launch_command_line("-foo +connect_lobby 99 -bar");
    let mut app = app_with(Some(&fake));
    frames(&mut app, 4);
    assert_eq!(seen::<JoinRequested>(&app), vec![JoinRequested { lobby: 99, from: 0, source: JoinSource::LaunchArgs }]);
}

#[test]
fn join_lobby_success_and_failure() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(JoinLobby { lobby: 42 });
    frames(&mut app, 3);
    assert_eq!(seen::<LobbyEntered>(&app), vec![LobbyEntered { lobby: 42 }]);
    assert_eq!(lobby(&app).current, Some(42));

    // Joining another lobby leaves the old one first.
    fake.set_join_succeeds(false);
    app.world_mut().write_message(JoinLobby { lobby: 43 });
    frames(&mut app, 3);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(42)));
    assert_eq!(seen::<LobbyLeft>(&app), vec![LobbyLeft { lobby: 42 }]);
    let errs = seen::<LobbyError>(&app);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, LobbyErrorKind::JoinFailed);
    assert_eq!(lobby(&app).current, None);
    assert_eq!(lobby(&app).pending_join, None);
}

#[test]
fn invite_needs_a_lobby_and_uses_the_connect_string() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(InviteFriend { steam_id: FRIEND });
    frames(&mut app, 2);
    let errs = seen::<LobbyError>(&app);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, LobbyErrorKind::NoLobby);
    assert!(!fake.calls().iter().any(|c| matches!(c, FakeCall::InviteToGame { .. })));

    // A bad id is refused without a call either.
    app.world_mut().write_message(InviteFriend { steam_id: 12345 });
    frames(&mut app, 1);
    assert_eq!(seen::<LobbyError>(&app).last().map(|e| e.kind), Some(LobbyErrorKind::InvalidSteamId));

    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    app.world_mut().write_message(InviteFriend { steam_id: FRIEND });
    frames(&mut app, 2);
    assert!(fake.calls().contains(&FakeCall::InviteToGame { friend: FRIEND, connect: "+connect_lobby 1000".into() }));
    assert_eq!(seen::<InviteSent>(&app), vec![InviteSent { steam_id: FRIEND, lobby: 1000, ok: true }]);
}

#[test]
fn without_a_backend_requests_get_no_backend_errors_and_nothing_panics() {
    let mut app = app_with(None);
    app.world_mut().write_message(create_msg());
    app.world_mut().write_message(LeaveLobby);
    app.world_mut().write_message(ClearRichPresence);
    frames(&mut app, 3);
    let errs = seen::<LobbyError>(&app);
    assert_eq!(errs.len(), 1, "leave/clear are silent no-ops without Steam: {errs:?}");
    assert_eq!(errs[0].kind, LobbyErrorKind::NoBackend);
    assert_eq!(lobby(&app), SteamLobby::default());
}

#[test]
fn app_exit_leaves_the_lobby() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(1000)));
    assert!(fake.calls().contains(&FakeCall::ClearRichPresence));
    assert_eq!(lobby(&app).current, None);
}

#[test]
fn no_ambiguous_systems_in_any_main_schedule() {
    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(SteamKitPlugin::default());
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);
    assert_eq!(app.world().resource::<SteamLobby>().current, Some(1000));
}

#[test]
fn parse_connect_lobby_cases() {
    let p = "+connect_lobby";
    assert_eq!(parse_connect_lobby("+connect_lobby 123", p), Some(123));
    assert_eq!(parse_connect_lobby("game.exe -x +connect_lobby 109775241000000000 -y", p), Some(109_775_241_000_000_000));
    assert_eq!(parse_connect_lobby("+connect_lobby=77", p), Some(77));
    assert_eq!(parse_connect_lobby("  +connect_lobby   5 ", p), Some(5));
    assert_eq!(parse_connect_lobby("+connect_lobby", p), None);
    assert_eq!(parse_connect_lobby("+connect_lobby abc", p), None);
    assert_eq!(parse_connect_lobby("+connect_lobby 0", p), None);
    assert_eq!(parse_connect_lobby("+connect_lobby -5", p), None);
    assert_eq!(parse_connect_lobby("+connect_lobbyx 5", p), None);
    assert_eq!(parse_connect_lobby("", p), None);
    assert_eq!(parse_connect_lobby("+connect_lobby 5", ""), None);
    assert_eq!(parse_connect_lobby("+join 8", "+join"), Some(8));
    let args = ["game.exe".to_string(), "+connect_lobby".to_string(), "31".to_string()];
    assert_eq!(parse_connect_lobby(&args.join(" "), p), Some(31));
    assert_eq!(connect_string(p, 9), "+connect_lobby 9");
}

#[test]
fn max_members_is_clamped_to_steams_range() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(CreateLobby { kind: LobbyKind::Private, max_members: 0, data: vec![] });
    frames(&mut app, 2);
    app.world_mut().write_message(LeaveLobby);
    app.world_mut().write_message(CreateLobby { kind: LobbyKind::Public, max_members: 10_000, data: vec![] });
    frames(&mut app, 2);
    let caps: Vec<_> = fake
        .calls()
        .iter()
        .filter_map(|c| match c {
            FakeCall::CreateLobby { max_members, .. } => Some(*max_members),
            _ => None,
        })
        .collect();
    assert_eq!(caps, vec![1, MAX_LOBBY_MEMBERS]);
}

/// Which frame (1-based) each `JoinRequested` was read in, by a game system in `PreUpdate`.
#[derive(Resource, Default)]
struct ReadFrames(Vec<(u32, u64)>);

fn read_in_pre_update(mut frame: Local<u32>, mut requests: MessageReader<JoinRequested>, mut seen: ResMut<ReadFrames>) {
    *frame += 1;
    for req in requests.read() {
        seen.0.push((*frame, req.lobby));
    }
}

#[test]
fn a_pumped_lobby_event_is_a_message_in_the_same_frame() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.init_resource::<ReadFrames>().add_systems(PreUpdate, read_in_pre_update);
    frames(&mut app, 2);
    let pumps_before = fake.pump_count();

    // Queued between frames 2 and 3: pumped in frame 3's `First`, readable in frame 3's `PreUpdate`.
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 64, from: FRIEND });
    frames(&mut app, 1);
    assert_eq!(fake.pump_count(), pumps_before + 1);
    assert_eq!(app.world().resource::<ReadFrames>().0, vec![(3, 64)]);

    // Never delivered twice.
    frames(&mut app, 3);
    assert_eq!(app.world().resource::<ReadFrames>().0, vec![(3, 64)]);
    assert_eq!(seen::<JoinRequested>(&app).len(), 1);
}

#[test]
fn a_join_request_from_a_non_user_id_reports_from_zero() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 5, from: 12345 });
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 0, from: FRIEND });
    frames(&mut app, 1);
    assert_eq!(seen::<JoinRequested>(&app), vec![JoinRequested { lobby: 5, from: 0, source: JoinSource::LobbyInvite }]);
}

/// A backend written without the lobby half: `SteamBackend::lobby` keeps its `None` default.
struct CoreOnly(FakeSteamBackend);

impl SteamBackend for CoreOnly {
    fn local_id(&self) -> u64 {
        self.0.local_id()
    }
    fn friend_name(&self, id: u64) -> String {
        self.0.friend_name(id)
    }
    fn launch_command_line(&self) -> String {
        self.0.launch_command_line()
    }
    fn pump(&self) -> Vec<BackendEvent> {
        self.0.pump()
    }
}

#[test]
fn a_backend_without_lobby_support_is_treated_as_no_steam() {
    let fake = FakeSteamBackend::new();
    fake.set_launch_command_line("+connect_lobby 99");
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default())).insert_resource(SteamBackendRes(Box::new(CoreOnly(fake.clone()))));
    watch::<JoinRequested>(&mut app);
    watch::<LobbyError>(&mut app);
    app.world_mut().write_message(create_msg());
    app.world_mut().write_message(LeaveLobby);
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 7, from: FRIEND });
    frames(&mut app, 3);

    // Still pumped every frame by the core ...
    assert_eq!(fake.pump_count(), 3);
    // ... but the lobby feature does nothing with it: no launch-arg / pumped join request, and
    // requests are refused like without Steam (leave silently).
    assert!(seen::<JoinRequested>(&app).is_empty());
    assert_eq!(seen::<LobbyError>(&app).iter().map(|e| e.kind).collect::<Vec<_>>(), vec![LobbyErrorKind::NoBackend]);
    assert!(fake.calls().is_empty());
    assert_eq!(lobby(&app), SteamLobby::default());
}

#[test]
fn settings_come_from_the_kit_plugin_builder() {
    let fake = FakeSteamBackend::new();
    fake.set_launch_command_line("+join 12");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        SteamKitPlugin::default().with_lobby(LobbySettings { connect_prefix: "+join".into(), set_connect_presence: false, ..Default::default() }),
    ))
    .insert_resource(SteamBackendRes(Box::new(fake.clone())));
    watch::<JoinRequested>(&mut app);
    app.world_mut().write_message(create_msg());
    frames(&mut app, 3);

    assert_eq!(app.world().resource::<LobbySettings>().connect_prefix, "+join");
    assert_eq!(seen::<JoinRequested>(&app), vec![JoinRequested { lobby: 12, from: 0, source: JoinSource::LaunchArgs }]);
    assert_eq!(lobby(&app).current, Some(1000));
    assert_eq!(fake.rich_presence("connect"), None, "set_connect_presence: false");
}

#[test]
fn parse_connect_lobby_handles_quotes() {
    assert_eq!(parse_connect_lobby("\"+connect_lobby\" \"77\"", "+connect_lobby"), Some(77));
}

#[test]
fn an_abandoned_join_that_completes_is_left_and_never_current() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    // Two joins in one frame: the first is abandoned by the second while both are in flight.
    app.world_mut().write_message(JoinLobby { lobby: 42 });
    app.world_mut().write_message(JoinLobby { lobby: 43 });
    frames(&mut app, 1);
    assert_eq!(lobby(&app).pending_join, Some(43));

    // Both complete on the next pump: 42 is left at once, 43 becomes current.
    frames(&mut app, 2);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(42)));
    assert_eq!(lobby(&app).current, Some(43));
    assert_eq!(seen::<LobbyEntered>(&app), vec![LobbyEntered { lobby: 43 }]);
    assert!(seen::<LobbyLeft>(&app).is_empty(), "42 was never current, so no LobbyLeft");

    // A lobby entered with no request at all is left the same way.
    fake.push_event(BackendEvent::LobbyEntered { lobby: 44 });
    frames(&mut app, 1);
    assert!(fake.calls().contains(&FakeCall::LeaveLobby(44)));
    assert_eq!(lobby(&app).current, Some(43));
}

/// A game system that removes the backend between `Pump` and `Callbacks` (the documented misuse).
fn remove_backend_mid_first(world: &mut World) {
    world.remove_resource::<SteamBackendRes>();
}

#[test]
fn removing_the_backend_between_pump_and_callbacks_drops_that_frames_events_without_panicking() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.add_systems(First, remove_backend_mid_first.after(SteamKitSystems::Pump).before(SteamKitSystems::Callbacks));
    fake.push_event(BackendEvent::LobbyJoinRequested { lobby: 8, from: FRIEND });
    app.world_mut().write_message(LeaveLobby);
    frames(&mut app, 3);

    assert_eq!(fake.pump_count(), 1, "pumped once, then the backend was gone");
    assert!(seen::<JoinRequested>(&app).is_empty());
    assert!(seen::<LobbyError>(&app).is_empty());
    assert_eq!(lobby(&app), SteamLobby::default());
}

#[test]
fn launch_args_that_are_not_unicode_are_skipped() {
    use std::ffi::OsString;
    #[cfg(windows)]
    let bad: OsString = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0xD800]) // a lone surrogate
    };
    #[cfg(unix)]
    let bad: OsString = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xff, 0xfe])
    };
    let args = vec![OsString::from("-novid"), bad, OsString::from("+connect_lobby"), OsString::from("42")];
    let text = super::launch_text(args.into_iter(), "");
    assert_eq!(text, "-novid +connect_lobby 42 ");
    assert_eq!(parse_connect_lobby(&text, "+connect_lobby"), Some(42));
    assert_eq!(super::launch_text(std::iter::empty(), "+connect_lobby 7"), " +connect_lobby 7");
}

#[test]
fn steam_lost_clears_the_lobby_and_answers_a_create_in_flight_once() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    // In a lobby: Steam lost -> `current` cleared, one `LobbyLeft`, no error.
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);
    let open = lobby(&app).current.expect("lobby open");
    let generation = lobby(&app).generation;
    fake.simulate_steam_exit();
    frames(&mut app, 1);
    assert_eq!(lobby(&app).current, None);
    assert_ne!(lobby(&app).generation, generation);
    assert_eq!(seen::<LobbyLeft>(&app), vec![LobbyLeft { lobby: open }]);
    assert!(seen::<LobbyError>(&app).is_empty());
    frames(&mut app, 3);
    assert_eq!(seen::<LobbyLeft>(&app).len(), 1);

    // A create in flight: answered NoBackend exactly once, `pending_create` cleared.
    let fake = FakeSteamBackend::new();
    fake.set_auto_complete_create(false);
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(create_msg());
    frames(&mut app, 2);
    assert!(lobby(&app).pending_create);
    fake.simulate_steam_exit();
    frames(&mut app, 5);
    assert_eq!(seen::<LobbyError>(&app).iter().map(|e| e.kind).collect::<Vec<_>>(), vec![LobbyErrorKind::NoBackend]);
    assert_eq!(SteamLobby { generation: 0, ..lobby(&app) }, SteamLobby::default());
    assert!(seen::<LobbyCreated>(&app).is_empty());
}

#[test]
fn steam_lost_answers_a_join_in_flight_once() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    frames(&mut app, 1);
    app.world_mut().write_message(JoinLobby { lobby: 42 });
    frames(&mut app, 1);
    assert_eq!(lobby(&app).pending_join, Some(42));
    // The join's answer is lost with the pump (a caught panic drops that frame's events).
    fake.panic_in_next_pump();
    frames(&mut app, 4);
    assert_eq!(seen::<LobbyError>(&app).iter().map(|e| e.kind).collect::<Vec<_>>(), vec![LobbyErrorKind::NoBackend]);
    assert_eq!(lobby(&app).pending_join, None);
    assert!(seen::<LobbyEntered>(&app).is_empty());
    assert!(seen::<LobbyLeft>(&app).is_empty());
    // New requests afterwards: NoBackend as without a backend.
    app.world_mut().write_message(JoinLobby { lobby: 43 });
    frames(&mut app, 1);
    assert_eq!(seen::<LobbyError>(&app).len(), 2);
}
