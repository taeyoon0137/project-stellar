//! 외부 process 플러그인 IPC 계약 (XXI.5.1): 요청·응답·heartbeat, process crash isolation.

mod support;

use std::time::Duration;

use stellar_plugin_host::permission::Permission;
use stellar_plugin_host::process::{KillReason, ProcessState};
use support::process::{fast, gone, launch, pump_until};
use support::Fixture;

#[test]
fn well_behaved_process_gets_replies_and_heartbeats() {
    let fx = Fixture::new("plugin-process-ok", 1);
    let out = fx.temp.path.join("process.log");
    let mut plugin = launch(&fx, "well-behaved", &out, fast());
    pump_until(&mut plugin, Duration::from_secs(10), |_| {
        std::fs::read_to_string(&out)
            .map(|t| t.contains("\"result\"") && t.contains("heartbeat 3"))
            .unwrap_or(false)
    });
    assert_eq!(plugin.state(), ProcessState::Running);
    let log = std::fs::read_to_string(&out).unwrap();
    assert!(log.contains(support::PLUGIN_ID), "host.info 응답을 받음");
    assert!(log.contains("heartbeat 3"));
    let pid = plugin.pid();
    assert_eq!(plugin.revoke(), ProcessState::Killed(KillReason::Revoked));
    assert!(gone(pid));
}

#[test]
fn crashing_process_does_not_take_the_host_down() {
    let fx = Fixture::new("plugin-process-crash", 1);
    let out = fx.temp.path.join("process.log");
    let mut plugin = launch(&fx, "crash", &out, fast());
    pump_until(&mut plugin, Duration::from_secs(10), |s| {
        *s != ProcessState::Running
    });
    assert_eq!(plugin.state(), ProcessState::Exited(Some(3)));
    // Host(이 test process)는 계속 일하고, 같은 프로젝트로 새 session을 열 수 있습니다.
    assert!(fx
        .dispatcher(&[Permission::DocumentRead])
        .call("document.read_snapshot", &support::json("{}"))
        .is_ok());
    // 끝난 뒤 pump는 상태만 돌려줍니다.
    assert_eq!(
        plugin.pump(Duration::from_millis(10)),
        ProcessState::Exited(Some(3))
    );
}
