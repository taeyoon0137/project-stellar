//! XIX.1.6 검증: 실제 다른 프로그램이 앱 endpoint에 연결하면 데이터 교환 없이 거부합니다.
//!
//! 프로세스를 띄우는 테스트는 socket을 여는 unit test와 같은 프로세스에서 동시에 돌면, macOS에서
//! socket 생성과 close-on-exec 설정 사이에 띄운 자식이 다른 테스트의 fd를 물려받을 수 있습니다.
//! 그래서 별도 test 실행 파일로 둡니다.

#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::time::Duration;

use stellar_local_ipc::auth::{AuthError, PeerPolicy};
use stellar_local_ipc::code_identity::CodeIdentityError;
use stellar_local_ipc::listener::{AppEndpoint, AppListener, ListenerEvent};
use stellar_local_ipc::os;
use stellar_local_ipc::registry::{InstanceId, Registry};

#[test]
fn fixture_foreign_program_is_rejected_by_current_process_policy() {
    // 실제 다른 프로그램(Apple platform binary `/usr/bin/nc`)이 endpoint에 연결합니다. 같은
    // 사용자이지만 현재 프로세스 기준(자기 서명에서 정한 규칙)을 만족하지 않아 데이터 교환 없이
    // 닫힙니다.
    let dir = std::env::temp_dir().join(format!("slipc-fp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let registry = Registry::at(&dir, os::current_uid());
    let endpoint = AppEndpoint::bind(&registry, InstanceId::from_bytes([1; 8])).unwrap();
    let app = AppListener::spawn(endpoint, PeerPolicy::for_current_process().unwrap()).unwrap();
    let mut nc = Command::new("/usr/bin/nc")
        .arg("-U")
        .arg(app.socket_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let event = app.events().recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(event.thread(), app.io_thread(), "IPC I/O 스레드에서 인증");
    assert_ne!(event.thread(), std::thread::current().id());
    assert!(
        matches!(
            event,
            ListenerEvent::Rejected {
                error: AuthError::Code(CodeIdentityError::RequirementFailed),
                ..
            }
        ),
        "{event:?}"
    );
    drop(nc.stdin.take());
    let _ = nc.kill();
    let output = nc.wait_with_output().unwrap();
    assert!(output.stdout.is_empty(), "nc가 받은 byte 0");
    assert!(app
        .events()
        .recv_timeout(Duration::from_millis(100))
        .is_err());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
