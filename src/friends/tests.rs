//! Friends feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend), strict ambiguity detection on every main schedule, real time driven by hand
//! (100 ms per frame). No real Steam call is ever made here.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use super::find_launch_connect;
use crate::*;

const ME: u64 = 76_561_197_960_265_729;
const A: u64 = 76_561_197_960_265_730;
const B: u64 = 76_561_197_960_265_731;

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

fn take<T: Message + Clone>(app: &mut App) -> Vec<T> {
    std::mem::take(&mut app.world_mut().resource_mut::<Seen<T>>().0)
}

fn strict(app: &mut App) {
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
}

fn app_with(backend: Option<&FakeSteamBackend>, settings: FriendsSettings, clock: bool) -> App {
    let mut app = App::new();
    if clock {
        app.add_plugins(MinimalPlugins).insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
    } else {
        app.add_plugins(bevy::app::TaskPoolPlugin::default());
    }
    app.add_plugins(SteamKitPlugin::default().with_friends(settings));
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<FriendsChanged>(&mut app);
    watch::<GameInviteSent>(&mut app);
    watch::<ConnectRequested>(&mut app);
    watch::<UserInfoReady>(&mut app);
    watch::<FriendAvatar>(&mut app);
    watch::<FriendsError>(&mut app);
    strict(&mut app);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn friends(app: &App) -> &SteamFriends {
    app.world().resource::<SteamFriends>()
}

fn change(added: &[u64], removed: &[u64], changed: &[u64]) -> FriendsChanged {
    FriendsChanged { added: added.to_vec(), removed: removed.to_vec(), changed: changed.to_vec() }
}

fn two_friends() -> FakeSteamBackend {
    let fake = FakeSteamBackend::new();
    fake.set_friend_name(A, "Friend A");
    fake.set_friend_name(B, "Friend B");
    fake.add_friend(B, PersonaState::Online);
    fake.add_friend(A, PersonaState::Away);
    fake
}

#[test]
fn the_list_loads_on_the_first_frame_with_everyone_added() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    assert!(friends(&app).is_loaded());
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[A, B], &[], &[])]);
    let f = friends(&app);
    assert_eq!(f.list().iter().map(|f| f.steam_id).collect::<Vec<_>>(), vec![A, B]);
    assert_eq!(f.get(A).map(|f| (f.name.as_str(), f.state)), Some(("Friend A", PersonaState::Away)));
    assert_eq!(f.online().count(), 2);
    assert_eq!(f.app_id(), 480);
    assert_eq!(f.me().map(|m| (m.steam_id, m.name.as_str(), m.state)), Some((ME, "Local Player", PersonaState::Online)));
    // The events queued by the knobs arrive afterwards and change nothing.
    frames(&mut app, 2);
    assert!(take::<FriendsChanged>(&mut app).is_empty());
}

#[test]
fn a_persona_change_rereads_that_friend_in_the_same_frame() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    take::<FriendsChanged>(&mut app);
    let generation = friends(&app).generation();
    fake.set_friend_state(A, PersonaState::Busy);
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[A])]);
    assert_eq!(friends(&app).get(A).unwrap().state, PersonaState::Busy);
    assert_ne!(friends(&app).generation(), generation);

    fake.set_friend_nickname(B, Some("Bee"));
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[B])]);
    assert_eq!(friends(&app).get(B).unwrap().display_name(), "Bee");
}

#[test]
fn friends_added_and_removed() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    take::<FriendsChanged>(&mut app);
    let c = 76_561_197_960_265_740;
    fake.add_friend(c, PersonaState::Online);
    fake.remove_friend(A);
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[c], &[A], &[])]);
    assert_eq!(friends(&app).list().iter().map(|f| f.steam_id).collect::<Vec<_>>(), vec![B, c]);
}

#[test]
fn invisible_and_unknown_states_map_without_panicking() {
    let fake = two_friends();
    fake.set_friend_state(A, PersonaState::Invisible);
    fake.set_friend_state(B, PersonaState::Unknown);
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    assert_eq!(friends(&app).get(A).unwrap().state, PersonaState::Invisible);
    assert!(!friends(&app).get(A).unwrap().is_online());
    assert_eq!(friends(&app).online().count(), 0);
}

#[test]
fn connect_is_read_only_for_friends_playing_this_game() {
    let fake = two_friends();
    fake.set_friend_game(A, Some((480, 0)));
    fake.set_friend_rich_presence(A, "connect", Some("+connect 10.0.0.1:7777"));
    fake.set_friend_game(B, Some((570, 0)));
    fake.set_friend_rich_presence(B, "connect", Some("+other"));
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    let f = friends(&app);
    assert_eq!(f.get(A).unwrap().connect.as_deref(), Some("+connect 10.0.0.1:7777"));
    assert!(f.get(A).unwrap().plays(480));
    assert_eq!(f.get(B).unwrap().connect, None, "another app's rich presence is not read");
    assert_eq!(f.playing_this_game().map(|f| f.steam_id).collect::<Vec<_>>(), vec![A]);

    // read_connect off: never read.
    let mut app = app_with(Some(&fake), FriendsSettings { read_connect: false, ..Default::default() }, true);
    frames(&mut app, 1);
    assert_eq!(friends(&app).get(A).unwrap().connect, None);
}

#[test]
fn the_poll_finds_a_rich_presence_change_that_sent_no_callback() {
    let fake = two_friends();
    fake.set_friend_game(A, Some((480, 0)));
    let mut app = app_with(Some(&fake), FriendsSettings { refresh_interval: Duration::from_secs(1), ..Default::default() }, true);
    frames(&mut app, 3);
    take::<FriendsChanged>(&mut app);
    fake.set_friend_rich_presence(A, "connect", Some("+join 5"));
    frames(&mut app, 3);
    assert!(take::<FriendsChanged>(&mut app).is_empty(), "no callback, not due yet");
    frames(&mut app, 10);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[A])]);
    assert_eq!(friends(&app).get(A).unwrap().connect.as_deref(), Some("+join 5"));
}

#[test]
fn without_a_clock_only_events_and_refresh_friends_reread() {
    let fake = two_friends();
    fake.set_friend_game(A, Some((480, 0)));
    let mut app = app_with(Some(&fake), FriendsSettings { refresh_interval: Duration::ZERO, ..Default::default() }, false);
    frames(&mut app, 2);
    take::<FriendsChanged>(&mut app);
    fake.set_friend_rich_presence(A, "connect", Some("+join 5"));
    frames(&mut app, 5);
    assert!(take::<FriendsChanged>(&mut app).is_empty());
    app.world_mut().write_message(RefreshFriends);
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[A])]);
}

#[test]
fn invite_to_game_is_validated_before_steam() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    for (steam_id, connect) in [(12345, "+x"), (A, ""), (A, &*"x".repeat(MAX_CONNECT_BYTES + 1)), (A, "a\0b")] {
        app.world_mut().write_message(InviteToGame { steam_id, connect: connect.to_string() });
    }
    frames(&mut app, 1);
    let kinds: Vec<FriendsErrorKind> = seen::<FriendsError>(&app).iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![FriendsErrorKind::InvalidSteamId, FriendsErrorKind::InvalidConnect, FriendsErrorKind::InvalidConnect, FriendsErrorKind::InvalidConnect]
    );
    assert!(!fake.calls().iter().any(|c| matches!(c, FakeCall::InviteToGame { .. })));

    let longest = "y".repeat(MAX_CONNECT_BYTES);
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: longest.clone() });
    frames(&mut app, 1);
    assert_eq!(seen::<GameInviteSent>(&app), vec![GameInviteSent { steam_id: A, connect: longest.clone(), ok: true }]);
    assert!(fake.calls().contains(&FakeCall::InviteToGame { friend: A, connect: longest }));
}

#[test]
fn raw_join_requests_arrive_as_connect_requested_whatever_the_string() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    fake.push_event(BackendEvent::ConnectRequested { from: A, connect: "server=10.0.0.1:7777;pw=x".into() });
    fake.push_event(BackendEvent::ConnectRequested { from: 1, connect: "".into() });
    frames(&mut app, 1);
    assert_eq!(
        seen::<ConnectRequested>(&app),
        vec![
            ConnectRequested { connect: "server=10.0.0.1:7777;pw=x".into(), from: A, source: ConnectSource::RichPresence },
            ConnectRequested { connect: "".into(), from: 0, source: ConnectSource::RichPresence },
        ]
    );
}

#[test]
fn a_cold_launch_with_the_prefix_is_one_connect_requested() {
    let fake = two_friends();
    fake.set_launch_command_line("-novid +connect 10.0.0.1:7777");
    let settings = FriendsSettings { launch_connect_prefix: Some("+connect".into()), ..Default::default() };
    let mut app = app_with(Some(&fake), settings, true);
    frames(&mut app, 3);
    assert_eq!(seen::<ConnectRequested>(&app), vec![ConnectRequested { connect: "+connect 10.0.0.1:7777".into(), from: 0, source: ConnectSource::LaunchArgs }]);

    // No prefix configured: no check.
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    assert!(seen::<ConnectRequested>(&app).is_empty());
}

#[test]
fn launch_connect_parsing() {
    assert_eq!(find_launch_connect("game.exe +connect 1.2.3.4:5 -x", "+connect").as_deref(), Some("+connect 1.2.3.4:5 -x"));
    assert_eq!(find_launch_connect("a \"+connect=7\"", "+connect").as_deref(), Some("+connect=7"));
    assert_eq!(find_launch_connect("+connected 1", "+connect"), None);
    assert_eq!(find_launch_connect("x +connect_lobby 5 +connect 9", "+connect").as_deref(), Some("+connect 9"));
    assert_eq!(find_launch_connect("nothing here", "+connect"), None);
    assert_eq!(find_launch_connect("+connect 1", " "), None);
}

#[test]
fn user_info_for_a_non_friend() {
    let fake = two_friends();
    let stranger = 76_561_197_960_265_750;
    fake.set_friend_name(stranger, "Stranger");
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    frames(&mut app, 1);
    assert!(seen::<UserInfoReady>(&app).is_empty(), "loading");
    frames(&mut app, 1);
    assert_eq!(seen::<UserInfoReady>(&app), vec![UserInfoReady { steam_id: stranger, name: "Stranger".into() }]);
    assert!(fake.calls().contains(&FakeCall::RequestUserInformation { id: stranger, name_only: true }));
    assert!(take::<FriendsChanged>(&mut app).len() == 1, "a non-friend's persona change does not touch the list");

    // Already loaded: answered at once.
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: false });
    frames(&mut app, 1);
    assert_eq!(seen::<UserInfoReady>(&app).len(), 2);
    app.world_mut().write_message(RequestUserInfo { steam_id: 5, name_only: false });
    frames(&mut app, 1);
    assert_eq!(seen::<FriendsError>(&app)[0].kind, FriendsErrorKind::InvalidSteamId);
}

#[test]
fn avatars_are_sent_once_and_again_on_change() {
    let fake = two_friends();
    fake.set_friend_avatar(A, 2, 2, vec![7; 16]);
    fake.set_friend_avatar(ME, 1, 1, vec![1; 4]);
    let mut app = app_with(Some(&fake), FriendsSettings { avatars: Some(AvatarSize::Small), ..Default::default() }, true);
    frames(&mut app, 3);
    let mut got: Vec<(u64, u32)> = take::<FriendAvatar>(&mut app).iter().map(|a| (a.steam_id, a.width)).collect();
    got.sort();
    assert_eq!(got, vec![(ME, 1), (A, 2)], "B has none");
    fake.set_friend_avatar(A, 2, 2, vec![9; 16]);
    frames(&mut app, 1);
    let again = take::<FriendAvatar>(&mut app);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].rgba, vec![9; 16]);
    assert!(format!("{:?}", again[0]).contains("16 bytes"));

    // Avatars off: none.
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    assert!(seen::<FriendAvatar>(&app).is_empty());
}

#[test]
fn own_persona_changes_show_in_me_and_changed() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    take::<FriendsChanged>(&mut app);
    fake.set_local_persona("Renamed", PersonaState::Away);
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[ME])]);
    assert_eq!(friends(&app).me().map(|m| m.name.as_str()), Some("Renamed"));
}

#[test]
fn own_state_follows_away_invisible_online() {
    // The live check (Online -> Away -> Invisible -> Online): `me().state` is the local user's
    // own state each time, never stuck on Online.
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 2);
    assert_eq!(friends(&app).me().map(|m| m.state), Some(PersonaState::Online));
    for state in [PersonaState::Away, PersonaState::Invisible, PersonaState::Online] {
        take::<FriendsChanged>(&mut app);
        fake.set_local_persona("Local Player", state);
        frames(&mut app, 1);
        assert_eq!(friends(&app).me().map(|m| m.state), Some(state));
        assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[], &[ME])], "{state:?}");
    }
}

#[test]
fn no_backend_answers_requests_and_removal_clears_the_list() {
    let mut app = app_with(None, FriendsSettings::default(), true);
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: "+x".into() });
    app.world_mut().write_message(RequestUserInfo { steam_id: A, name_only: true });
    app.world_mut().write_message(RefreshFriends);
    frames(&mut app, 1);
    assert_eq!(seen::<FriendsError>(&app).iter().map(|e| e.kind).collect::<Vec<_>>(), vec![FriendsErrorKind::NoBackend; 2]);
    assert!(!friends(&app).is_loaded());

    let fake = two_friends();
    app.insert_resource(SteamBackendRes(Box::new(fake.clone())));
    frames(&mut app, 1);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[A, B], &[], &[])]);
    app.world_mut().remove_resource::<SteamBackendRes>();
    frames(&mut app, 2);
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[A, B], &[])]);
    assert!(!friends(&app).is_loaded());
    assert!(friends(&app).list().is_empty());
}

#[test]
fn a_backend_without_friends_counts_as_none() {
    struct CoreOnly;
    impl SteamBackend for CoreOnly {
        fn local_id(&self) -> u64 {
            ME
        }
        fn friend_name(&self, _: u64) -> String {
            String::new()
        }
        fn launch_command_line(&self) -> String {
            String::new()
        }
        fn pump(&self) -> Vec<BackendEvent> {
            vec![BackendEvent::ConnectRequested { from: A, connect: "x".into() }]
        }
    }
    let mut app = app_with(None, FriendsSettings::default(), true);
    app.insert_resource(SteamBackendRes(Box::new(CoreOnly)));
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: "+x".into() });
    frames(&mut app, 2);
    assert_eq!(seen::<FriendsError>(&app)[0].kind, FriendsErrorKind::NoBackend);
    assert!(seen::<ConnectRequested>(&app).is_empty());
    assert!(!friends(&app).is_loaded());
}

#[test]
fn settings_builder_and_resource() {
    let app = app_with(None, FriendsSettings { refresh_interval: Duration::from_secs(9), avatars: Some(AvatarSize::Large), ..Default::default() }, true);
    let s = app.world().resource::<FriendsSettings>();
    assert_eq!((s.refresh_interval, s.avatars, s.read_connect), (Duration::from_secs(9), Some(AvatarSize::Large), true));
    assert_eq!(AvatarSize::Large.pixels(), 184);
}

fn errors(app: &App) -> Vec<(FriendsRequestKind, u64, FriendsErrorKind)> {
    seen::<FriendsError>(app).iter().map(|e| (e.request, e.steam_id, e.kind)).collect()
}

#[test]
fn errors_name_the_request_and_the_steam_id() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: String::new() });
    app.world_mut().write_message(InviteToGame { steam_id: B, connect: "a\0".into() });
    app.world_mut().write_message(RequestUserInfo { steam_id: 5, name_only: true });
    frames(&mut app, 1);
    assert_eq!(
        errors(&app),
        vec![
            (FriendsRequestKind::UserInfo, 5, FriendsErrorKind::InvalidSteamId),
            (FriendsRequestKind::Invite, A, FriendsErrorKind::InvalidConnect),
            (FriendsRequestKind::Invite, B, FriendsErrorKind::InvalidConnect),
        ]
    );
}

#[test]
fn a_refused_invite_reports_steams_false() {
    let fake = two_friends();
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    fake.refuse_next_game_invite();
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: "+x".into() });
    app.world_mut().write_message(InviteToGame { steam_id: B, connect: "+x".into() });
    frames(&mut app, 1);
    let sent: Vec<(u64, bool)> = seen::<GameInviteSent>(&app).iter().map(|s| (s.steam_id, s.ok)).collect();
    assert_eq!(sent, vec![(A, false), (B, true)]);
}

#[test]
fn user_info_is_answered_once_per_request_even_for_duplicates() {
    let fake = two_friends();
    let stranger = 76_561_197_960_265_750;
    fake.set_friend_name(stranger, "Stranger");
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    // A friend is known at once.
    app.world_mut().write_message(RequestUserInfo { steam_id: A, name_only: true });
    frames(&mut app, 3);
    let got: Vec<u64> = seen::<UserInfoReady>(&app).iter().map(|u| u.steam_id).collect();
    assert_eq!(got, vec![A, stranger, stranger]);
    assert!(app.world().resource::<super::FriendsInternals>().user_info_waiting.is_empty());
}

#[test]
fn user_info_steam_never_delivers_times_out() {
    let fake = two_friends();
    let stranger = 76_561_197_960_265_750;
    let mut app = app_with(Some(&fake), FriendsSettings { user_info_timeout: Duration::from_secs(1), ..Default::default() }, true);
    frames(&mut app, 1);
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    frames(&mut app, 5);
    assert!(errors(&app).is_empty());
    frames(&mut app, 12);
    assert_eq!(errors(&app), vec![(FriendsRequestKind::UserInfo, stranger, FriendsErrorKind::TimedOut)]);
    assert!(seen::<UserInfoReady>(&app).is_empty());
    frames(&mut app, 20);
    assert_eq!(errors(&app).len(), 1, "answered once");
}

#[test]
fn user_info_waiting_when_the_backend_is_removed_is_answered_no_backend() {
    let fake = two_friends();
    let stranger = 76_561_197_960_265_750;
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    frames(&mut app, 1);
    app.world_mut().remove_resource::<SteamBackendRes>();
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(FriendsRequestKind::UserInfo, stranger, FriendsErrorKind::NoBackend)]);
}

#[test]
fn user_info_waiting_at_app_exit_is_answered_exiting_once() {
    let fake = two_friends();
    let (s1, s2, late) = (76_561_197_960_265_750, 76_561_197_960_265_751, 76_561_197_960_265_752);
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    // A request written after the `Update` set in the exit frame.
    app.add_systems(PostUpdate, |mut exit: MessageReader<AppExit>, mut w: MessageWriter<RequestUserInfo>| {
        if exit.read().count() > 0 {
            w.write(RequestUserInfo { steam_id: 76_561_197_960_265_752, name_only: true });
        }
    });
    frames(&mut app, 1);
    // Two requests Steam never answers (one of them twice), and one it answers.
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: s1, name_only: true });
    frames(&mut app, 1);
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: s2, name_only: false });
    frames(&mut app, 1);
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: s2, name_only: false });
    frames(&mut app, 1);
    assert!(errors(&app).is_empty());

    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(
        errors(&app),
        vec![
            (FriendsRequestKind::UserInfo, s1, FriendsErrorKind::Exiting),
            (FriendsRequestKind::UserInfo, s2, FriendsErrorKind::Exiting),
            (FriendsRequestKind::UserInfo, s2, FriendsErrorKind::Exiting),
            (FriendsRequestKind::UserInfo, late, FriendsErrorKind::Exiting),
        ]
    );
    let internals = app.world().resource::<super::FriendsInternals>();
    assert!(internals.user_info_waiting.is_empty() && internals.user_info_due.is_empty());
    // Nothing is answered twice afterwards.
    frames(&mut app, 3);
    assert_eq!(errors(&app).len(), 4);
    assert!(seen::<UserInfoReady>(&app).is_empty());
}

#[test]
fn steam_lost_answers_waiting_user_info_no_backend_and_clears_the_list() {
    let fake = two_friends();
    let stranger = 76_561_197_960_265_750;
    let mut app = app_with(Some(&fake), FriendsSettings::default(), true);
    frames(&mut app, 1);
    fake.silence_next_user_info();
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    frames(&mut app, 1);
    take::<FriendsChanged>(&mut app);
    fake.simulate_steam_exit();
    frames(&mut app, 1);
    assert!(app.world().resource::<SteamBackendRes>().0.friends().is_none(), "inert backend");
    assert_eq!(errors(&app), vec![(FriendsRequestKind::UserInfo, stranger, FriendsErrorKind::NoBackend)]);
    assert!(!friends(&app).is_loaded());
    assert_eq!(take::<FriendsChanged>(&mut app), vec![change(&[], &[A, B], &[])]);
    // New requests after the loss are answered too.
    app.world_mut().write_message(RequestUserInfo { steam_id: stranger, name_only: true });
    app.world_mut().write_message(InviteToGame { steam_id: A, connect: "+x".into() });
    frames(&mut app, 1);
    assert_eq!(errors(&app).len(), 3);
    assert!(errors(&app)[1..].iter().all(|e| e.2 == FriendsErrorKind::NoBackend));
}

#[test]
fn a_loading_avatar_arrives_on_the_next_full_refresh() {
    let fake = two_friends();
    fake.set_friend_avatar(A, 1, 1, vec![5; 4]);
    fake.set_friend_avatar_loading(A, true);
    let settings = FriendsSettings { avatars: Some(AvatarSize::Large), refresh_interval: Duration::from_secs(1), ..Default::default() };
    let mut app = app_with(Some(&fake), settings, true);
    frames(&mut app, 3);
    assert!(take::<FriendAvatar>(&mut app).is_empty(), "still loading");
    // Loaded without an avatar event (Steam sends none to this steamworks version): the retry finds it.
    fake.lock().friends.avatars_loading.clear();
    frames(&mut app, 12);
    let got = take::<FriendAvatar>(&mut app);
    assert_eq!(got.iter().map(|a| a.steam_id).collect::<Vec<_>>(), vec![A]);
}

#[test]
fn at_most_sixteen_avatars_per_frame_and_wrong_sizes_are_skipped() {
    let fake = FakeSteamBackend::new();
    for i in 0..20u64 {
        let id = 76_561_197_960_265_800 + i;
        fake.add_friend(id, PersonaState::Online);
        fake.set_friend_avatar(id, 1, 1, vec![1; 4]);
    }
    let bad = 76_561_197_960_265_900;
    fake.add_friend(bad, PersonaState::Online);
    fake.set_friend_avatar(bad, 2, 2, vec![0; 3]);
    fake.set_friend_avatar(ME, 1, 1, vec![1; 4]);
    let mut app = app_with(Some(&fake), FriendsSettings { avatars: Some(AvatarSize::Small), ..Default::default() }, true);
    app.update();
    assert_eq!(take::<FriendAvatar>(&mut app).len(), 16);
    app.update();
    // Sorted ids: the local user + 15 friends first, then 5 friends and the bad one (skipped).
    let rest = take::<FriendAvatar>(&mut app);
    assert_eq!(rest.len(), 5);
    assert!(rest.iter().all(|a| a.steam_id != bad));
    frames(&mut app, 3);
    assert!(take::<FriendAvatar>(&mut app).iter().all(|a| a.steam_id != bad));
}
