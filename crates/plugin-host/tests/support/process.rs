//! 외부 process 플러그인 fixture 공용 함수.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use stellar_plugin_host::permission::Permission;
use stellar_plugin_host::process::{ProcessConfig, ProcessPlugin, ProcessState};
use stellar_plugin_host::session::{HostSession, SystemClock};

use super::Fixture;

pub fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process/plugin.sh")
}

pub fn launch(fx: &Fixture, mode: &str, out: &Path, config: ProcessConfig) -> ProcessPlugin {
    let session = HostSession::spawn(
        fx.dispatcher(&[Permission::DocumentRead]),
        Arc::new(SystemClock::default()),
    );
    let mut command = Command::new("/bin/sh");
    command.arg(script()).arg(mode).arg(out);
    ProcessPlugin::launch(command, session, config).unwrap()
}

pub fn fast() -> ProcessConfig {
    ProcessConfig {
        heartbeat_interval: Duration::from_millis(50),
        heartbeat_timeout: Duration::from_millis(400),
        ..ProcessConfig::default()
    }
}

pub fn pump_until(
    plugin: &mut ProcessPlugin,
    limit: Duration,
    done: impl Fn(&ProcessState) -> bool,
) {
    let started = Instant::now();
    while started.elapsed() < limit {
        let state = plugin.pump(Duration::from_millis(50));
        if done(&state) {
            return;
        }
    }
}

/// pid가 더는 살아 있지 않은지(`kill -0` 실패).
pub fn gone(pid: u32) -> bool {
    !Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
