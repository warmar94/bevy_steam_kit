//! Feature `overlay` through the public API only: a made-up game pauses while the overlay is open
//! and opens the store page from its menu (fake backend, no Steam).

use bevy::prelude::*;
use bevy_steam_kit::*;

#[derive(Resource, Default)]
struct Paused(bool);

fn pause_on_overlay(mut toggled: MessageReader<OverlayToggled>, mut paused: ResMut<Paused>) {
    for t in toggled.read() {
        paused.0 = t.active;
    }
}

#[test]
fn pause_while_the_overlay_is_open_and_open_the_store() {
    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<Paused>()
        .add_systems(PreUpdate, pause_on_overlay);
    app.world_mut().write_message(OpenOverlay::store(480));
    app.update();
    assert_eq!(fake.calls(), vec![FakeCall::ActivateOverlay(OpenOverlay::store(480))]);

    fake.toggle_overlay(true);
    app.update();
    assert!(app.world().resource::<Paused>().0);
    fake.toggle_overlay(false);
    app.update();
    assert!(!app.world().resource::<Paused>().0);
}
