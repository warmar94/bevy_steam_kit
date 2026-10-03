//! Feature `friends` through the public API only: a made-up friends panel lists friends, invites
//! one with a custom connect string, and receives a raw join (fake backend, no Steam).

use bevy::prelude::*;
use bevy_steam_kit::*;

const A: u64 = 76_561_197_960_265_730;
const B: u64 = 76_561_197_960_265_731;

/// What the made-up panel shows and did.
#[derive(Resource, Default)]
struct Panel {
    rows: Vec<(u64, String, bool)>,
    joins: Vec<String>,
    invited: bool,
}

fn show(friends: Res<SteamFriends>, mut changed: MessageReader<FriendsChanged>, mut panel: ResMut<Panel>) {
    if changed.read().count() > 0 {
        panel.rows = friends.list().iter().map(|f| (f.steam_id, f.display_name().to_string(), f.plays(friends.app_id()))).collect();
    }
}

/// Invite the first friend playing this game to a server of our own.
fn invite(friends: Res<SteamFriends>, mut panel: ResMut<Panel>, mut invites: MessageWriter<InviteToGame>) {
    if panel.invited {
        return;
    }
    if let Some(f) = friends.playing_this_game().next() {
        panel.invited = true;
        invites.write(InviteToGame { steam_id: f.steam_id, connect: "+connect 10.0.0.1:7777".into() });
    }
}

fn on_join(mut joins: MessageReader<ConnectRequested>, mut panel: ResMut<Panel>) {
    for j in joins.read() {
        panel.joins.push(j.connect.clone());
    }
}

#[test]
fn a_friends_panel() {
    let fake = FakeSteamBackend::new();
    fake.set_friend_name(A, "Friend A");
    fake.set_friend_name(B, "Friend B");
    fake.add_friend(A, PersonaState::Online);
    fake.add_friend(B, PersonaState::Offline);
    fake.set_friend_game(A, Some((480, 0)));

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<Panel>()
        .add_systems(Update, (show, invite, on_join).chain().before(SteamKitSystems::Requests));
    for _ in 0..3 {
        app.update();
    }
    let panel = app.world().resource::<Panel>();
    assert_eq!(panel.rows, vec![(A, "Friend A".to_string(), true), (B, "Friend B".to_string(), false)]);
    assert!(fake.calls().contains(&FakeCall::InviteToGame { friend: A, connect: "+connect 10.0.0.1:7777".into() }));

    // The friend accepts on their side; here, our game receives a join from them.
    fake.push_rich_presence_join(A, "+connect 10.0.0.2:7777");
    app.update();
    assert_eq!(app.world().resource::<Panel>().joins, vec!["+connect 10.0.0.2:7777".to_string()]);
}
