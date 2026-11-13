//! XIX.1.7 검증: 브리지 측 IPC 상대 인증 실패 fixture에서 데이터 교환 0, 종료 코드가 XXIV.1.2 표와
//! 일치.
//!
//! 빌드된 `stellar-mcp --attach --instance <id>`를 실제 프로세스로 실행합니다. 앱 endpoint는 현재
//! 사용자의 기본 registry(`<사용자 임시 디렉터리>/stellar-ipc-<uid>`)에 테스트마다 고유한 instance
//! id로 엽니다. 기본 registry 디렉터리가 없으면 앱 계약대로 0700으로 만들고 남겨 둡니다.
//!
//! macOS에서는 socket·pipe 생성과 close-on-exec 설정 사이에 다른 thread가 띄운 자식이 그 fd를
//! 물려받을 수 있으므로, 이 파일의 테스트는 [`SERIAL`]로 하나씩 실행합니다.

#![cfg(target_os = "macos")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use stellar_mcp::exit;
use stellar_mcp::ipc::auth::PeerPolicy;
use stellar_mcp::ipc::listener::{AppEndpoint, AppListener, ListenerEvent};
use stellar_mcp::ipc::registry::{InstanceId, Registry};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");

/// 프로세스·socket을 만드는 테스트를 직렬화합니다.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// 테스트마다 고유한 instance id(pid + 순번).
fn unique_instance() -> InstanceId {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    InstanceId::parse(&format!(
        "{:08x}{:08x}",
        std::process::id(),
        0xa000_0000 | n
    ))
    .unwrap()
}

fn run_attach(args: &[&str], stdin: Stdio) -> Output {
    Command::new(BIN)
        .args(args)
        .env_clear()
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

/// socket 파일이 생길 때까지 기다립니다.
fn wait_for_socket(path: &Path) {
    use std::os::unix::fs::FileTypeExt;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("socket이 생기지 않음: {}", path.display());
}

/// `nc`가 연 가짜 앱 endpoint. drop하면 nc를 끝내고 socket 파일을 지웁니다.
struct ForeignEndpoint {
    nc: std::process::Child,
    path: PathBuf,
}

impl Drop for ForeignEndpoint {
    fn drop(&mut self) {
        let _ = self.nc.kill();
        let _ = self.nc.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn fixture_foreign_endpoint_is_rejected_without_data_exchange() {
    let _serial = serial();
    // 같은 사용자의 실제 다른 프로그램(Apple platform binary `/usr/bin/nc`)이 앱 endpoint 자리를
    // 차지합니다. 브리지는 HELLO를 보내기 전에 인증해 거부하고 77(noperm)으로 끝나야 합니다.
    let registry = Registry::for_current_user().unwrap();
    // 앱 계약대로 registry를 준비합니다(AppEndpoint::bind와 같은 0700 생성).
    let probe = AppEndpoint::bind(&registry, unique_instance()).unwrap();
    drop(probe);
    let instance = unique_instance();
    let path = registry.socket_path(&instance).unwrap();
    let nc = std::process::Command::new("/usr/bin/nc")
        .arg("-lkU")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut foreign = ForeignEndpoint { nc, path };
    wait_for_socket(&foreign.path);

    let output = run_attach(
        &["--attach", "--instance", instance.as_str()],
        Stdio::null(),
    );
    assert_eq!(output.status.code(), Some(i32::from(exit::NOPERM.code)));
    assert!(output.stdout.is_empty(), "MCP stdout 비어 있음");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("(noperm)"), "{stderr}");
    assert!(stderr.contains("서명 요구 사항"), "{stderr}");

    // nc는 받은 data를 stdout에 씁니다. 브리지가 보낸 byte는 0이어야 합니다.
    let _ = foreign.nc.kill();
    let _ = foreign.nc.wait();
    let mut received = Vec::new();
    foreign
        .nc
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut received)
        .unwrap();
    assert!(
        received.is_empty(),
        "가짜 endpoint가 받은 byte: {received:?}"
    );
}

#[test]
fn fixture_attach_failures_use_xxiv_1_2_exit_codes() {
    let _serial = serial();
    // 지정 instance 없음 → 69(unavailable).
    let missing = unique_instance();
    let output = run_attach(&["--attach", "--instance", missing.as_str()], Stdio::null());
    assert_eq!(
        output.status.code(),
        Some(i32::from(exit::UNAVAILABLE.code))
    );
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("(unavailable)"));

    // 형식이 잘못된 instance·인자 → 64(usage), stdout 비어 있음.
    for args in [
        &["--attach", "--instance", "ABCDEF0123456789"][..],
        &["--attach", "--instance"][..],
        &["--attach", "extra"][..],
        &["--instance", "0123456789abcdef"][..],
    ] {
        let output = run_attach(args, Stdio::null());
        assert_eq!(
            output.status.code(),
            Some(i32::from(exit::USAGE.code)),
            "{args:?}"
        );
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn authenticated_app_attach_serves_mcp_and_ends_on_eof() {
    let _serial = serial();
    let registry = Registry::for_current_user().unwrap();
    let instance = unique_instance();
    let endpoint = AppEndpoint::bind(&registry, instance.clone()).unwrap();
    let app = AppListener::spawn(endpoint, PeerPolicy::for_current_process().unwrap()).unwrap();

    let mut child = Command::new(BIN)
        .args(["--attach", "--instance", instance.as_str()])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n"
    );
    assert!(stderr.contains("인증 후 연결했습니다"), "{stderr}");
    match app.events().recv_timeout(Duration::from_secs(10)).unwrap() {
        ListenerEvent::Connected { connection, .. } => {
            assert_eq!(connection.peer().euid, stellar_mcp::ipc::os::current_uid());
        }
        other => panic!("{other:?}"),
    }
}

/// `tools/stellar-cli/src/exit.rs`에서 `fn <name>(self)` 본문의 `ExitStatus::X => 값` 쌍을 읽습니다.
fn cli_table(source: &str, function: &str) -> Vec<(String, String)> {
    let start = source
        .find(&format!("pub fn {function}(self)"))
        .unwrap_or_else(|| panic!("{function} 없음"));
    let body = &source[start..];
    let body = &body[..body.find("\n    }\n").unwrap()];
    body.lines()
        .filter_map(|line| {
            let (variant, value) = line.trim().strip_prefix("ExitStatus::")?.split_once("=>")?;
            Some((
                variant.trim().to_string(),
                value
                    .trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .to_string(),
            ))
        })
        .collect()
}

#[test]
fn exit_table_matches_xxiv_1_2() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../stellar-cli/src/exit.rs"
    ));
    let codes = cli_table(source, "code");
    let names = cli_table(source, "name");
    assert_eq!(codes.len(), 13, "XXIV.1.2 표 13행");
    for status in exit::ALL {
        let variant = names
            .iter()
            .find(|(_, name)| name == status.name)
            .map(|(variant, _)| variant)
            .unwrap_or_else(|| panic!("XXIV.1.2 표에 없는 이름: {}", status.name));
        let code = codes
            .iter()
            .find(|(v, _)| v == variant)
            .map(|(_, code)| code.parse::<u8>().unwrap())
            .unwrap();
        assert_eq!(code, status.code, "{}", status.name);
    }
}
