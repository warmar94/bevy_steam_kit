//! Auth feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend), strict ambiguity detection on every main schedule, real time driven by hand
//! (100 ms per frame). No real Steam call is ever made here.

use std::time::Duration;

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use crate::*;

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

fn errors(app: &App) -> Vec<(u64, AuthErrorKind)> {
    seen::<AuthError>(app).iter().map(|e| (e.id.0, e.kind)).collect()
}

fn ready(app: &App) -> Vec<(u64, String)> {
    seen::<WebApiTicketReady>(app).iter().map(|r| (r.id.0, r.ticket.to_hex())).collect()
}

fn app_with(backend: Option<&FakeSteamBackend>, settings: AuthSettings) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default().with_auth(settings)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<WebApiTicketReady>(&mut app);
    watch::<AuthError>(&mut app);
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn send(app: &mut App, req: AuthRequest) {
    app.world_mut().write_message(req);
}

fn id(n: u64) -> AuthRequestId {
    AuthRequestId(n)
}

fn hex_of(bytes: &[u8]) -> String {
    WebApiTicket::new(bytes.to_vec()).to_hex()
}

fn cancels(fake: &FakeSteamBackend) -> usize {
    fake.calls().iter().filter(|c| matches!(c, FakeCall::CancelAuthTicket { .. })).count()
}

#[test]
fn the_ticket_arrives_on_the_next_frame_as_lowercase_hex() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(7), "my-server"));
    frames(&mut app, 1);
    assert!(app.world().resource::<SteamAuth>().is_pending(id(7)));
    assert!(ready(&app).is_empty());
    frames(&mut app, 1);
    assert_eq!(ready(&app), vec![(7, hex_of(b"FAKE-TICKET-1"))]);
    assert_eq!(hex_of(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    let r = &seen::<WebApiTicketReady>(&app)[0];
    assert_eq!(r.identity, "my-server");
    assert_eq!(r.ticket.bytes(), b"FAKE-TICKET-1");
    let state = app.world().resource::<SteamAuth>();
    assert!(!state.is_pending(id(7)) && state.is_live(id(7)));
    assert_eq!(state.live_tickets(), 1);
    assert_eq!(fake.live_auth_tickets(), vec!["my-server".to_string()]);

    // Cancel after the server answered: cancelled at Steam, no message.
    send(&mut app, AuthRequest::cancel(id(7)));
    frames(&mut app, 1);
    assert_eq!(app.world().resource::<SteamAuth>().live_tickets(), 0);
    assert!(fake.live_auth_tickets().is_empty());
    assert!(errors(&app).is_empty());
    assert!(fake.calls().contains(&FakeCall::CancelAuthTicket { op: 1 }));
}

#[test]
fn two_concurrent_tickets_go_to_the_right_ids() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(20), "a"));
    send(&mut app, AuthRequest::web_api_ticket(id(10), "b"));
    frames(&mut app, 2);
    let mut got: Vec<(u64, String, String)> = seen::<WebApiTicketReady>(&app).iter().map(|r| (r.id.0, r.identity.clone(), r.ticket.to_hex())).collect();
    got.sort();
    assert_eq!(got, vec![(10, "b".into(), hex_of(b"FAKE-TICKET-2")), (20, "a".into(), hex_of(b"FAKE-TICKET-1"))]);
}

#[test]
fn cancel_before_the_answer_gives_exactly_one_cancelled_error() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    send(&mut app, AuthRequest::cancel(id(1)));
    frames(&mut app, 3);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::Cancelled)]);
    assert!(ready(&app).is_empty(), "the late answer is dropped");
    assert_eq!(cancels(&fake), 1);
    assert!(fake.live_auth_tickets().is_empty());
    // An unknown id is ignored.
    send(&mut app, AuthRequest::cancel(id(99)));
    frames(&mut app, 1);
    assert_eq!(errors(&app).len(), 1);
}

#[test]
fn a_nul_identity_never_reaches_the_backend() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "bad\0id"));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::InvalidIdentity)]);
    assert!(fake.calls().is_empty());
}

#[test]
fn a_duplicate_id_is_rejected_and_the_first_request_still_answers() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "a"));
    send(&mut app, AuthRequest::web_api_ticket(id(1), "b"));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::DuplicateId)]);
    assert_eq!(ready(&app).len(), 1);
    // Still live: the id stays taken.
    send(&mut app, AuthRequest::web_api_ticket(id(1), "c"));
    frames(&mut app, 2);
    assert_eq!(errors(&app).len(), 2);
    // next_id never hands out a live id.
    let next = app.world_mut().resource_mut::<SteamAuth>().next_id();
    assert_eq!(next, id(2));
}

#[test]
fn next_id_skips_ids_in_use_and_wraps() {
    let mut state = SteamAuth::default();
    assert_eq!(state.next_id(), id(1));
    state.live.insert(id(2), 5);
    state.pending.insert(id(3), super::Pending { op: 6, identity: String::new(), started: None });
    assert_eq!(state.next_id(), id(4));
    state.last_issued = AuthRequestId::FIRST_MANUAL - 1;
    assert_eq!(state.next_id(), id(1));
}

#[test]
fn steam_failure_and_refusal_are_failed_errors() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    fake.fail_next_auth_ticket(FakeAuthFailure::Failed);
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::Failed)]);
    assert_eq!(cancels(&fake), 1, "the failed handle is released");
    assert!(fake.live_auth_tickets().is_empty());

    fake.fail_next_auth_ticket(FakeAuthFailure::Refused);
    send(&mut app, AuthRequest::web_api_ticket(id(2), "svc"));
    frames(&mut app, 1);
    assert_eq!(errors(&app)[1], (2, AuthErrorKind::Failed));
}

#[test]
fn no_backend_answers_no_backend() {
    let mut app = app_with(None, AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    send(&mut app, AuthRequest::cancel(id(1)));
    frames(&mut app, 2);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::NoBackend)]);
}

#[test]
fn a_backend_without_auth_counts_as_none() {
    struct CoreOnly;
    impl SteamBackend for CoreOnly {
        fn local_id(&self) -> u64 {
            76_561_197_960_265_729
        }
        fn friend_name(&self, _: u64) -> String {
            String::new()
        }
        fn launch_command_line(&self) -> String {
            String::new()
        }
        fn pump(&self) -> Vec<BackendEvent> {
            Vec::new()
        }
    }
    let mut app = app_with(None, AuthSettings::default());
    app.insert_resource(SteamBackendRes(Box::new(CoreOnly)));
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    frames(&mut app, 1);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::NoBackend)]);
}

#[test]
fn backend_removed_while_pending_answers_no_backend_once() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    fake.fail_next_auth_ticket(FakeAuthFailure::NoAnswer);
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    frames(&mut app, 1);
    app.world_mut().remove_resource::<SteamBackendRes>();
    frames(&mut app, 3);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::NoBackend)]);
    assert!(!app.world().resource::<SteamAuth>().is_pending(id(1)));
}

#[test]
fn a_silent_request_times_out_and_is_cancelled() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings { timeout: Duration::from_secs(1), ..Default::default() });
    fake.fail_next_auth_ticket(FakeAuthFailure::NoAnswer);
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    frames(&mut app, 5);
    assert!(errors(&app).is_empty());
    frames(&mut app, 12);
    assert_eq!(errors(&app), vec![(1, AuthErrorKind::TimedOut)]);
    assert_eq!(cancels(&fake), 1);
    assert!(fake.live_auth_tickets().is_empty());
}

#[test]
fn app_exit_answers_pending_with_exiting_and_cancels_live_tickets() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "live"));
    frames(&mut app, 2);
    fake.fail_next_auth_ticket(FakeAuthFailure::NoAnswer);
    send(&mut app, AuthRequest::web_api_ticket(id(2), "pending"));
    frames(&mut app, 1);
    assert_eq!(fake.live_auth_tickets().len(), 2);
    // A request written in the exit frame after the Update set is answered too.
    app.add_systems(PostUpdate, |mut w: MessageWriter<AuthRequest>, mut once: Local<bool>| {
        if !std::mem::replace(&mut *once, true) {
            w.write(AuthRequest::web_api_ticket(AuthRequestId(3), "late"));
        }
    });
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    let mut e = errors(&app);
    e.sort_by_key(|x| x.0);
    assert_eq!(e, vec![(2, AuthErrorKind::Exiting), (3, AuthErrorKind::Exiting)]);
    assert!(fake.live_auth_tickets().is_empty(), "every ticket cancelled");
    assert_eq!(app.world().resource::<SteamAuth>().live_tickets(), 0);
}

#[test]
fn cancel_on_exit_can_be_turned_off() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings { cancel_on_exit: false, ..Default::default() });
    send(&mut app, AuthRequest::web_api_ticket(id(1), "live"));
    frames(&mut app, 2);
    app.world_mut().write_message(AppExit::Success);
    frames(&mut app, 1);
    assert_eq!(fake.live_auth_tickets(), vec!["live".to_string()]);
}

#[test]
fn the_ticket_never_shows_in_debug_output() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    send(&mut app, AuthRequest::web_api_ticket(id(1), "svc"));
    frames(&mut app, 2);
    let r = seen::<WebApiTicketReady>(&app).remove(0);
    let hex = r.ticket.to_hex();
    for text in [format!("{r:?}"), format!("{:?}", r.ticket), format!("{:?}", BackendEvent::WebApiTicket { op: 1, result: Ok(r.ticket.clone()) })] {
        assert!(!text.contains(&hex), "{text}");
        assert!(!text.contains("FAKE-TICKET"), "{text}");
        assert!(!text.contains("70, 65, 75, 69"), "no byte list: {text}");
    }
    assert_eq!(format!("{:?}", r.ticket), "WebApiTicket(13 bytes)");
}

#[test]
fn an_event_for_an_unknown_op_is_ignored() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake), AuthSettings::default());
    fake.push_event(BackendEvent::WebApiTicket { op: 77, result: Ok(WebApiTicket::new(vec![1, 2])) });
    frames(&mut app, 2);
    assert!(ready(&app).is_empty() && errors(&app).is_empty());
}

#[test]
fn settings_builder_and_resource() {
    let app = app_with(None, AuthSettings { timeout: Duration::from_secs(3), cancel_on_exit: false });
    let s = app.world().resource::<AuthSettings>();
    assert_eq!((s.timeout, s.cancel_on_exit), (Duration::from_secs(3), false));
    assert_eq!(AuthSettings::default().timeout, Duration::from_secs(30));
}
