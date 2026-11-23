//! rate/batch fixture (XXI.4.3): 비동기 응답, batch, 고빈도 호출 거부, 처리 중 상한.

mod support;

use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stellar_ops::test_support::select_trusted_project;
use stellar_plugin_host::host::{Dispatcher, HostConfig, NetworkPort};
use stellar_plugin_host::json::JsonValue;
use stellar_plugin_host::permission::Permission;
use stellar_plugin_host::rpc::RpcLimits;
use stellar_plugin_host::session::{Clock, HostSession, Submitted};
use support::{json, Fixture};

const WAIT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct ManualClock(Mutex<Duration>);

impl ManualClock {
    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

fn session(fx: &Fixture, clock: Arc<ManualClock>) -> HostSession {
    HostSession::spawn(fx.dispatcher(&Permission::ALL), clock)
}

fn recv(session: &HostSession) -> JsonValue {
    json(&session.recv_timeout(WAIT).expect("응답"))
}

fn info(id: u64) -> String {
    format!(r#"{{"stellar_rpc":1,"id":{id},"method":"host.info"}}"#)
}

#[test]
fn requests_return_immediately_and_replies_match_ids_in_order() {
    let fx = Fixture::new("plugin-rpc-async", 3);
    let clock = Arc::new(ManualClock::default());
    let session = session(&fx, clock);
    for id in 1..=5 {
        assert_eq!(session.submit(&info(id)), Submitted::Queued);
    }
    assert_eq!(
        session.submit(r#"{"stellar_rpc":1,"id":"read","method":"document.read_snapshot"}"#),
        Submitted::Queued
    );
    for id in 1..=5 {
        let reply = recv(&session);
        assert_eq!(reply.get("id").and_then(JsonValue::as_i64), Some(id));
        assert!(reply.get("result").is_some());
    }
    let read = recv(&session);
    assert_eq!(read.get("id").and_then(JsonValue::as_str), Some("read"));
    assert!(read.get("result").and_then(|r| r.get("revision")).is_some());
    assert_eq!(session.in_flight(), 0);
}

#[test]
fn batch_runs_items_in_order_and_keeps_going_after_failures() {
    let fx = Fixture::new("plugin-rpc-batch", 3);
    let session = session(&fx, Arc::new(ManualClock::default()));
    let message = r#"{"stellar_rpc":1,"id":7,"batch":[
        {"method":"file.write","params":{"path":"a.txt","text":"one"}},
        {"method":"nope.method"},
        {"method":"file.read","params":{"path":"a.txt"}},
        {"method":"file.read","params":{"path":"../x"}}
    ]}"#;
    assert_eq!(session.submit(message), Submitted::Queued);
    let reply = recv(&session);
    assert_eq!(reply.get("id").and_then(JsonValue::as_i64), Some(7));
    let items = reply.get("batch").and_then(JsonValue::as_array).unwrap();
    assert_eq!(items.len(), 4);
    assert!(items[0].get("result").is_some());
    assert_eq!(
        items[1]
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(JsonValue::as_str),
        Some("unknown-method")
    );
    assert_eq!(
        items[2]
            .get("result")
            .and_then(|r| r.get("text"))
            .and_then(JsonValue::as_str),
        Some("one")
    );
    assert_eq!(
        items[3]
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(JsonValue::as_str),
        Some("invalid-params")
    );

    let too_big: Vec<String> = (0..65)
        .map(|_| r#"{"method":"host.info"}"#.to_owned())
        .collect();
    let message = format!(
        r#"{{"stellar_rpc":1,"id":8,"batch":[{}]}}"#,
        too_big.join(",")
    );
    assert_eq!(
        session.submit(&message),
        Submitted::Rejected("batch-too-large")
    );
    let reply = recv(&session);
    assert_eq!(reply.get("id").and_then(JsonValue::as_i64), Some(8));
    assert_eq!(
        session.submit(r#"{"stellar_rpc":1,"id":9,"batch":[]}"#),
        Submitted::Rejected("invalid-request")
    );
    recv(&session);
}

#[test]
fn render_loop_rate_is_rejected_and_batch_counts_once() {
    let fx = Fixture::new("plugin-rpc-rate", 1);
    let clock = Arc::new(ManualClock::default());
    let session = session(&fx, Arc::clone(&clock));
    // 60Hz loop: 매 frame(16ms) host.info 한 번. client는 응답을 받은 뒤 다음 frame으로 갑니다.
    let mut accepted = 0;
    let mut limited = 0;
    let mut saw_retry = false;
    for frame in 0..60u64 {
        match session.submit(&info(frame)) {
            Submitted::Queued => accepted += 1,
            Submitted::Rejected("rate-limited") => limited += 1,
            other => panic!("{other:?}"),
        }
        let reply = recv(&session);
        if let Some(error) = reply.get("error") {
            assert_eq!(
                error.get("code").and_then(JsonValue::as_str),
                Some("rate-limited")
            );
            let retry = error
                .get("data")
                .and_then(|d| d.get("retry_after_ms"))
                .and_then(JsonValue::as_i64)
                .unwrap();
            assert!(retry > 0 && retry <= 1000);
            saw_retry = true;
        }
        clock.advance(Duration::from_millis(16));
    }
    // 0~944ms(60 frame) 중 첫 1초 창 20개, 나머지는 거부.
    assert_eq!(accepted, 20, "첫 1초 창에서 20개만 받음");
    assert_eq!(limited, 40);
    assert!(saw_retry);

    // 창이 지나면 다시 받고, 64항목 batch는 메시지 1개로 셉니다.
    clock.advance(Duration::from_secs(2));
    let items: Vec<&str> = (0..64).map(|_| r#"{"method":"host.info"}"#).collect();
    let mut results = 0;
    for id in 0..20u64 {
        let message = format!(
            r#"{{"stellar_rpc":1,"id":{id},"batch":[{}]}}"#,
            items.join(",")
        );
        assert_eq!(session.submit(&message), Submitted::Queued);
        let reply = recv(&session);
        results += reply
            .get("batch")
            .and_then(JsonValue::as_array)
            .unwrap()
            .len();
    }
    assert_eq!(
        session.submit(&info(99)),
        Submitted::Rejected("rate-limited")
    );
    recv(&session);
    assert_eq!(results, 20 * 64);
}

/// 해제할 때까지 막히는 network port(처리 중 상한 fixture).
struct BlockingNetwork(Mutex<Receiver<()>>);

impl NetworkPort for BlockingNetwork {
    fn get(&self, _url: &str) -> Result<(u16, String), String> {
        self.0.lock().unwrap().recv().map_err(|e| e.to_string())?;
        Ok((204, String::new()))
    }
}

#[test]
fn in_flight_limit_rejects_until_worker_catches_up() {
    let fx = Fixture::new("plugin-rpc-inflight", 1);
    let (release, wait): (Sender<()>, Receiver<()>) = channel();
    let config = HostConfig {
        lock: fx.config().lock,
        network: Some(Box::new(BlockingNetwork(Mutex::new(wait)))),
    };
    let granted: BTreeSet<Permission> = Permission::ALL.into_iter().collect();
    let dispatcher = Dispatcher::new(
        select_trusted_project(&fx.project, fx.data.clone()).unwrap(),
        fx.plugin(),
        &granted,
        config,
    );
    let limits = RpcLimits {
        max_in_flight: 2,
        ..RpcLimits::default()
    };
    let session = HostSession::spawn_with(dispatcher, Arc::new(ManualClock::default()), limits);
    let fetch = |id: u64| {
        format!(
            r#"{{"stellar_rpc":1,"id":{id},"method":"network.fetch","params":{{"url":"https://example.com/"}}}}"#
        )
    };
    assert_eq!(session.submit(&fetch(1)), Submitted::Queued);
    assert_eq!(session.submit(&fetch(2)), Submitted::Queued);
    assert_eq!(
        session.submit(&fetch(3)),
        Submitted::Rejected("too-many-in-flight")
    );
    let rejected = recv(&session);
    assert_eq!(rejected.get("id").and_then(JsonValue::as_i64), Some(3));
    release.send(()).unwrap();
    release.send(()).unwrap();
    for id in 1..=2 {
        let reply = recv(&session);
        assert_eq!(reply.get("id").and_then(JsonValue::as_i64), Some(id));
    }
    assert_eq!(session.in_flight(), 0);
    assert_eq!(session.submit(&fetch(4)), Submitted::Queued);
    release.send(()).unwrap();
    recv(&session);
}

#[test]
fn malformed_envelopes_are_rejected_with_fixed_codes() {
    let fx = Fixture::new("plugin-rpc-envelope", 1);
    let session = session(&fx, Arc::new(ManualClock::default()));
    let cases = [
        ("not json", "invalid-request"),
        (
            r#"{"stellar_rpc":1,"method":"host.info"}"#,
            "invalid-request",
        ),
        (
            r#"{"stellar_rpc":1,"id":-1,"method":"host.info"}"#,
            "invalid-request",
        ),
        (
            r#"{"stellar_rpc":2,"id":1,"method":"host.info"}"#,
            "unsupported-rpc-version",
        ),
        (
            r#"{"stellar_rpc":1,"id":1,"method":"host.info","extra":1}"#,
            "invalid-request",
        ),
        (r#"{"stellar_rpc":1,"id":1}"#, "invalid-request"),
        (r#"{"stellar_rpc":1,"id":1,"batch":{}}"#, "invalid-request"),
    ];
    for (message, code) in cases {
        assert_eq!(
            session.submit(message),
            Submitted::Rejected(code),
            "{message}"
        );
        let reply = recv(&session);
        assert_eq!(
            reply
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(JsonValue::as_str),
            Some(code)
        );
    }
    let huge = format!(
        r#"{{"stellar_rpc":1,"id":1,"method":"host.info","params":{{"x":"{}"}}}}"#,
        "a".repeat(1024 * 1024)
    );
    assert_eq!(session.submit(&huge), Submitted::Rejected("too-large"));
}

#[test]
fn permission_changes_apply_after_queued_requests() {
    let fx = Fixture::new("plugin-rpc-revoke", 1);
    let session = session(&fx, Arc::new(ManualClock::default()));
    let read =
        |id: u64| format!(r#"{{"stellar_rpc":1,"id":{id},"method":"document.read_snapshot"}}"#);
    assert_eq!(session.submit(&read(1)), Submitted::Queued);
    session.set_permissions(BTreeSet::new());
    assert_eq!(session.submit(&read(2)), Submitted::Queued);
    assert!(recv(&session).get("result").is_some());
    let denied = recv(&session);
    assert_eq!(
        denied
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(JsonValue::as_str),
        Some("permission-denied")
    );
}
