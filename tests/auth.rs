//! Feature `auth` through the public API only: a made-up game logs in to its server with a Web API
//! ticket (fake backend, no Steam).

use bevy::prelude::*;
use bevy_steam_kit::*;

/// What the made-up game's "server" received, as hex.
#[derive(Resource, Default)]
struct Server {
    tickets: Vec<String>,
    login: Option<AuthRequestId>,
}

/// The game asks for a ticket once.
fn log_in(mut done: Local<bool>, mut auth: ResMut<SteamAuth>, mut server: ResMut<Server>, mut requests: MessageWriter<AuthRequest>) {
    if !std::mem::replace(&mut *done, true) {
        let id = auth.next_id();
        server.login = Some(id);
        requests.write(AuthRequest::web_api_ticket(id, "made-up-server"));
    }
}

/// Send the hex to the server; the server checks it with Steam and answers; then cancel.
fn on_ticket(mut ready: MessageReader<WebApiTicketReady>, mut server: ResMut<Server>, mut requests: MessageWriter<AuthRequest>) {
    for r in ready.read() {
        server.tickets.push(r.ticket.to_hex());
        requests.write(AuthRequest::cancel(r.id));
    }
}

#[test]
fn log_in_to_a_server_with_a_web_api_ticket() {
    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default()))
        .insert_resource(SteamBackendRes(Box::new(fake.clone())))
        .init_resource::<Server>()
        .add_systems(Update, (log_in, on_ticket).chain().before(SteamKitSystems::Requests));
    for _ in 0..4 {
        app.update();
    }
    let expected: String = FakeSteamBackend::fake_web_api_ticket(1).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(app.world().resource::<Server>().tickets, vec![expected]);
    assert_eq!(app.world().resource::<SteamAuth>().live_tickets(), 0, "cancelled after use");
    assert!(fake.live_auth_tickets().is_empty());
    assert_eq!(fake.calls(), vec![FakeCall::RequestWebApiTicket { identity: "made-up-server".into() }, FakeCall::CancelAuthTicket { op: 1 }]);
}
