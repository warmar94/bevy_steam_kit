//! Overlay feature: headless tests on a tiny made-up app (MinimalPlugins + the kit + the FAKE
//! backend), strict ambiguity detection on every main schedule. No real Steam call is made here.

use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel};
use bevy::prelude::*;

use crate::*;

const A: u64 = 76_561_197_960_265_730;

#[derive(Resource)]
struct Seen<T: Message + Clone>(Vec<T>);

fn collect<T: Message + Clone>(mut r: MessageReader<T>, mut seen: ResMut<Seen<T>>) {
    seen.0.extend(r.read().cloned());
}

fn watch<T: Message + Clone>(app: &mut App) {
    app.insert_resource(Seen::<T>(Vec::new())).add_systems(Last, collect::<T>);
}

fn seen<T: Message + Clone>(app: &App) -> Vec<T> {
    app.world().resource::<Seen<T>>().0.clone()
}

fn app_with(backend: Option<&FakeSteamBackend>) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()));
    if let Some(b) = backend {
        app.insert_resource(SteamBackendRes(Box::new(b.clone())));
    }
    watch::<OverlayToggled>(&mut app);
    watch::<OverlayError>(&mut app);
    for label in [First.intern(), PreUpdate.intern(), Update.intern(), PostUpdate.intern(), Last.intern()] {
        app.edit_schedule(label, |s| {
            s.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default() });
        });
    }
    app
}

fn overlay_calls(fake: &FakeSteamBackend) -> Vec<OpenOverlay> {
    fake.calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::ActivateOverlay(r) => Some(r),
            // Other features' calls.
            #[allow(unreachable_patterns)]
            _ => None,
        })
        .collect()
}

#[test]
fn valid_requests_reach_steam_in_order() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    let requests = vec![
        OpenOverlay::dialog("friends"),
        OpenOverlay::user("steamid", A),
        OpenOverlay::web_page("https://example.com/news"),
        OpenOverlay::store(480),
        OpenOverlay::Store { app_id: 480, flag: StoreFlag::AddToCartAndShow },
        OpenOverlay::invite_dialog(109_775_241_000_000_001),
        OpenOverlay::invite_dialog_connect("+connect 10.0.0.1:7777"),
    ];
    for r in &requests {
        app.world_mut().write_message(r.clone());
    }
    app.update();
    assert_eq!(overlay_calls(&fake), requests);
    assert!(seen::<OverlayError>(&app).is_empty());
    assert!(app.world().resource::<SteamOverlay>().is_enabled());
}

#[test]
fn unusable_requests_never_reach_steam() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    let bad = vec![
        OpenOverlay::dialog(""),
        OpenOverlay::dialog("fri\0ends"),
        OpenOverlay::user("steamid", 12345),
        OpenOverlay::user("", A),
        OpenOverlay::web_page(""),
        OpenOverlay::web_page("https://x\0"),
        OpenOverlay::invite_dialog(0),
        OpenOverlay::invite_dialog_connect(""),
        OpenOverlay::invite_dialog_connect("x".repeat(256)),
        OpenOverlay::invite_dialog_connect("a\0b"),
    ];
    for r in &bad {
        app.world_mut().write_message(r.clone());
    }
    app.update();
    let errors = seen::<OverlayError>(&app);
    assert_eq!(errors.iter().map(|e| e.request.clone()).collect::<Vec<_>>(), bad);
    assert!(errors.iter().all(|e| e.kind == OverlayErrorKind::InvalidRequest));
    assert!(overlay_calls(&fake).is_empty());
    // 255 bytes is fine.
    app.world_mut().write_message(OpenOverlay::invite_dialog_connect("y".repeat(255)));
    app.update();
    assert_eq!(overlay_calls(&fake).len(), 1);
}

#[test]
fn open_and_closed_are_facts_in_the_frame_they_arrive() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    app.update();
    fake.toggle_overlay(true);
    app.update();
    assert_eq!(seen::<OverlayToggled>(&app), vec![OverlayToggled { active: true }]);
    assert!(app.world().resource::<SteamOverlay>().is_active());
    fake.toggle_overlay(false);
    app.update();
    assert!(!app.world().resource::<SteamOverlay>().is_active());
    assert_eq!(app.world().resource::<SteamOverlay>().toggles(), 2);
}

#[test]
fn a_disabled_overlay_is_reported_and_requests_still_go_through() {
    let fake = FakeSteamBackend::new();
    fake.set_overlay_enabled(false);
    let mut app = app_with(Some(&fake));
    app.world_mut().write_message(OpenOverlay::store(480));
    app.update();
    assert!(!app.world().resource::<SteamOverlay>().is_enabled());
    assert_eq!(overlay_calls(&fake).len(), 1);
}

#[test]
fn no_backend_answers_no_backend() {
    let mut app = app_with(None);
    app.world_mut().write_message(OpenOverlay::dialog("friends"));
    app.update();
    let errors = seen::<OverlayError>(&app);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, OverlayErrorKind::NoBackend);
    assert!(!app.world().resource::<SteamOverlay>().is_enabled());
}

#[test]
fn a_refused_call_is_a_refused_error() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    fake.refuse_next_overlay();
    app.world_mut().write_message(OpenOverlay::store(480));
    app.world_mut().write_message(OpenOverlay::dialog("friends"));
    app.update();
    let errors = seen::<OverlayError>(&app);
    assert_eq!(errors.len(), 1);
    assert_eq!((errors[0].request.clone(), errors[0].kind), (OpenOverlay::store(480), OverlayErrorKind::Refused));
    assert_eq!(overlay_calls(&fake).len(), 2, "both reached the backend, the second was accepted");
}

#[test]
fn removing_the_backend_closes_an_open_overlay_once() {
    let fake = FakeSteamBackend::new();
    let mut app = app_with(Some(&fake));
    fake.toggle_overlay(true);
    app.update();
    assert!(app.world().resource::<SteamOverlay>().is_active());
    app.world_mut().remove_resource::<SteamBackendRes>();
    app.update();
    app.update();
    assert!(!app.world().resource::<SteamOverlay>().is_active());
    assert_eq!(seen::<OverlayToggled>(&app), vec![OverlayToggled { active: true }, OverlayToggled { active: false }]);
}
