//! XXIV.1.1 검증: headless 환경에서 도움말 실행, core crate만 의존.
//!
//! 빌드된 `stellar-cli` 실행 파일을 환경 변수를 모두 지운 상태(HOME·PATH·DISPLAY·TMPDIR 없음),
//! 빈 작업 디렉터리, 닫힌 stdin으로 실행합니다. 앱·GUI·네트워크·프로젝트 없이 도움말이 성공해야
//! 합니다.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-cli-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 환경 변수를 지우고 빈 디렉터리에서 실행하는 headless 명령.
fn headless(dir: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_stellar-cli"))
        .args(args)
        .env_clear()
        .current_dir(&dir.path)
        .stdin(Stdio::null())
        .output()
        .expect("stellar-cli 실행")
}

#[test]
fn help_runs_without_app_gui_or_environment() {
    let dir = TempDir::new("headless");
    for flag in ["--help", "-h", "help"] {
        let output = headless(&dir, &[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 도움말");
        assert!(stdout.contains("사용법:"), "{flag}: {stdout}");
        assert!(stdout.contains("stellar-cli <명령>"), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}: stderr는 비어야 함");
    }
    // 실행이 작업 디렉터리에 아무것도 만들지 않습니다(설정·캐시 파일 없음).
    assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 0);
}

#[test]
fn usage_errors_exit_64_with_hint_on_stderr() {
    let dir = TempDir::new("usage");
    for args in [&[][..], &["no-such-command"][..]] {
        let output = headless(&dir, args);
        assert_eq!(output.status.code(), Some(64), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("--help"), "{args:?}: {stderr}");
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_is_usage_error_without_panic() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = TempDir::new("non-utf8");
    let output = Command::new(env!("CARGO_BIN_EXE_stellar-cli"))
        .arg(OsStr::from_bytes(&[0x66, 0xff, 0x6f]))
        .env_clear()
        .current_dir(&dir.path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("UTF-8"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// stellar-cli가 직접 의존할 수 있는 core crate.
const ALLOWED_CORE_DEPS: [&str; 5] = [
    "stellar-core-types",
    "stellar-project",
    "stellar-document",
    "stellar-storage",
    "stellar-diagnostics",
];

/// `[section]` 본문의 `key = ...` 줄에서 key를 모읍니다.
fn section_keys(manifest: &str, section: &str) -> Vec<(String, String)> {
    let header = format!("[{section}]");
    let mut keys = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            keys.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    keys
}

#[test]
fn manifest_depends_only_on_core_crates() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    for (key, value) in section_keys(&manifest, "dependencies") {
        assert!(
            ALLOWED_CORE_DEPS.contains(&key.as_str()),
            "core crate 외 의존성 금지: {key}"
        );
        // 외부(crates.io/git) 의존이 아니라 workspace 내부 path 의존이어야 합니다.
        assert!(
            value.contains("path = \"../../crates/"),
            "{key}는 crates/ 아래 path 의존이어야 함: {value}"
        );
    }
    // build script·빌드 의존성도 두지 않습니다.
    assert!(section_keys(&manifest, "build-dependencies").is_empty());
    assert!(!manifest.contains("build ="), "build script 금지");
    assert!(
        !std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs")).exists(),
        "build.rs 금지"
    );
}
