//! 외부 process 플러그인 수명 (XXI.5.2): heartbeat timeout, protocol 위반, 철회·drop 정리.

mod support;

use std::time::{Duration, Instant};

use stellar_plugin_host::process::{KillReason, ProcessConfig, ProcessState};
use support::process::{fast, gone, launch, pump_until};
use support::Fixture;

#[test]
fn hanging_process_is_killed_after_heartbeat_timeout() {
    let fx = Fixture::new("plugin-process-hang", 1);
    let out = fx.temp.path.join("process.log");
    let mut plugin = launch(&fx, "hang", &out, fast());
    let pid = plugin.pid();
    let started = Instant::now();
    pump_until(&mut plugin, Duration::from_secs(10), |s| {
        *s != ProcessState::Running
    });
    assert_eq!(
        plugin.state(),
        ProcessState::Killed(KillReason::HeartbeatTimeout)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(gone(pid), "멈춘 process를 정리함");
}

#[test]
fn protocol_violation_kills_and_drop_cleans_up() {
    let fx = Fixture::new("plugin-process-garbage", 1);
    let out = fx.temp.path.join("process.log");
    let mut plugin = launch(&fx, "garbage", &out, fast());
    let pid = plugin.pid();
    pump_until(&mut plugin, Duration::from_secs(10), |s| {
        *s != ProcessState::Running
    });
    assert_eq!(
        plugin.state(),
        ProcessState::Killed(KillReason::ProtocolViolation)
    );
    assert!(gone(pid));

    let other = launch(&fx, "hang", &out, ProcessConfig::default());
    let pid = other.pid();
    drop(other);
    assert!(gone(pid), "Host가 놓으면 process를 끝냄");
}
