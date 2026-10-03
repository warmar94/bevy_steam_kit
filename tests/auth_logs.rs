//! Feature `auth`: the kit's log lines never contain the ticket. Its own test binary with ONE
//! test, because it installs a process-wide `tracing` subscriber (parallel tests in one binary
//! would race over tracing's per-callsite cache).

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_steam_kit::*;

/// Collects every log line's fields as text (a minimal `tracing` subscriber).
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<String>>);

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Text<'a>(&'a mut String);
        impl tracing::field::Visit for Text<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let _ = writeln!(self.0, "{field}={value:?}");
            }
        }
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        event.record(&mut Text(&mut s));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn the_ticket_never_shows_in_the_logs() {
    let capture = Capture::default();
    tracing::subscriber::set_global_default(capture.clone()).expect("the only subscriber of this test binary");

    let fake = FakeSteamBackend::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SteamKitPlugin::default())).insert_resource(SteamBackendRes(Box::new(fake.clone())));
    app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(1), "svc"));
    app.update();
    app.update();
    app.world_mut().write_message(AuthRequest::cancel(AuthRequestId(1)));
    app.update();
    fake.fail_next_auth_ticket(FakeAuthFailure::Failed);
    app.world_mut().write_message(AuthRequest::web_api_ticket(AuthRequestId(2), "svc"));
    app.update();
    app.update();
    app.world_mut().write_message(AppExit::Success);
    app.update();

    let logs = capture.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert!(logs.contains("web API ticket for identity \"svc\" ready (13 bytes)"), "the capture works: {logs}");
    let hex = WebApiTicket::new(FakeSteamBackend::fake_web_api_ticket(1)).to_hex();
    assert!(!logs.contains(&hex), "{logs}");
    assert!(!logs.contains("FAKE-TICKET"), "{logs}");
    assert!(!logs.contains("70, 65, 75"), "no byte list: {logs}");
}
