//! 터미널은 복원하지 않는다는 검사(XVIII.4.6, 기술 아키텍처 22.3·32, ADR-0023 결정 10).
//!
//! - 창 종료(registry drop)는 실행 중인 터미널 프로세스를 끝내고, 같은 창의 새 registry는 빈 상태로 시작합니다.
//! - 앱 비정상 종료(crash): 앱 역할 helper 프로세스가 PTY 세션을 연 채 `SIGKILL`로 죽으면 PTY master가 닫혀
//!   세션 프로세스가 끝납니다. 다시 시작한 앱의 registry는 비어 있고, 이전 프로세스에 다시 붙는 경로가 없습니다
//!   (child를 만드는 입력은 신뢰 gate를 지난 `TerminalRegistry::start`뿐이고 pid로 붙는 API가 없음).
//! - recovery snapshot에 터미널 출력·프로세스가 들어가지 않는다는 검사는 payload 정의 쪽에 있습니다
//!   (`stellar_project::recovery_payload` 시험, 이 crate는 저장 crate에 의존하지 않음: `lib.rs` 시험).
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_terminal::capability::{TerminalCapability, TerminalTrust};
use stellar_terminal::cwd::CwdContext;
use stellar_terminal::env::TerminalIdentity;
use stellar_terminal::profile::{ArgvCommand, ProfileName, TerminalProfile};
use stellar_terminal::pty::PtyLauncher;
use stellar_terminal::registry::{LaunchEnvironment, TerminalRegistry};
use stellar_terminal::shell::{ShellSettings, SystemShellSource};
use stellar_terminal::size::TerminalSize;

/// helper 모드: 이 값이 있으면 helper 시험이 앱 역할을 합니다(값 = 기록 폴더).
const HELPER_ENV: &str = "STELLAR_TERMINAL_CRASH_HELPER_DIR";

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

fn alive(pid: i32) -> bool {
    // SAFETY: 신호 0은 존재 확인만 합니다.
    unsafe { kill(pid, 0) == 0 }
}

fn window() -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::from_bytes([46; 16]))
}

/// `pid_file`에 자기 pid를 쓰고 오래 자는 세션을 시작합니다. 세션 leader pid를 돌려줍니다.
fn start_sleeper(registry: &mut TerminalRegistry, cwd: &Path, pid_file: &Path) -> u32 {
    let context = CwdContext::new(cwd, &[]).unwrap();
    let shell = ShellSettings::default();
    let identity = TerminalIdentity::stellar("0.0.0-test").unwrap();
    let environment = LaunchEnvironment {
        cwd: &context,
        shell: &shell,
        shell_source: &SystemShellSource,
        identity: &identity,
        size: TerminalSize::DEFAULT,
    };
    let script = format!(
        "echo $$ > '{}.tmp' && mv '{0}.tmp' '{0}' && exec /bin/sleep 300",
        pid_file.display()
    );
    let profile = TerminalProfile::command(
        ProfileName::new("sleeper").unwrap(),
        ArgvCommand::new("/bin/sh", ["-c", script.as_str()]).unwrap(),
    );
    let id = registry.create(profile).unwrap();
    let capability = TerminalCapability::grant(window(), TerminalTrust::Trusted).unwrap();
    let mut launcher = PtyLauncher::with_grace(Duration::from_millis(300));
    registry
        .start(id, capability, &environment, &mut launcher)
        .unwrap()
}

fn wait_until(what: &str, mut until: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !until() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path =
            std::env::temp_dir().join(format!("stellar-no-restore-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 앱 역할 helper. 부모 시험이 [`HELPER_ENV`]를 주고 실행할 때만 동작하며, 일반 실행에서는 바로 끝납니다.
#[test]
fn crash_helper_holds_a_terminal_session() {
    let Some(dir) = std::env::var_os(HELPER_ENV).map(PathBuf::from) else {
        return;
    };
    let mut registry = TerminalRegistry::new(window());
    start_sleeper(&mut registry, &dir, &dir.join("session.pid"));
    // 부모가 SIGKILL로 끝낼 때까지 세션을 쥔 채 기다립니다(정상 정리 경로를 타지 않음).
    std::thread::sleep(Duration::from_secs(60));
    drop(registry);
}

#[test]
fn window_close_ends_terminal_processes_and_reopen_starts_empty() {
    let dir = TempDir::new("close");
    let pid_file = dir.0.join("session.pid");
    let mut registry = TerminalRegistry::new(window());
    start_sleeper(&mut registry, &dir.0, &pid_file);
    wait_until("세션 pid 기록", || read_pid(&pid_file).is_some());
    let pid = read_pid(&pid_file).unwrap();
    assert!(alive(pid));
    drop(registry); // 창 종료
    wait_until("창 종료 뒤 터미널 프로세스 종료", || {
        !alive(pid)
    });
    let reopened = TerminalRegistry::new(window());
    assert!(
        reopened.is_empty(),
        "같은 창을 다시 열어도 터미널을 복원하지 않습니다"
    );
}

#[test]
fn crash_and_relaunch_leave_no_terminal_process_to_reconnect() {
    let dir = TempDir::new("crash");
    let pid_file = dir.0.join("session.pid");
    let mut app = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_helper_holds_a_terminal_session",
            "--test-threads=1",
        ])
        .env(HELPER_ENV, &dir.0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_until("helper 세션 pid 기록", || read_pid(&pid_file).is_some());
    let pid = read_pid(&pid_file).unwrap();
    assert!(alive(pid), "crash 전 터미널 프로세스 실행 중");
    // 앱 crash: 정리 코드를 타지 않고 끝납니다.
    app.kill().unwrap();
    app.wait().unwrap();
    wait_until(
        "crash 뒤 PTY hangup으로 터미널 프로세스 종료",
        || !alive(pid),
    );
    // relaunch: 새 앱의 registry는 비어 있고 이전 프로세스는 남아 있지 않습니다.
    let relaunched = TerminalRegistry::new(window());
    assert!(relaunched.is_empty());
    assert!(!alive(pid));
}
