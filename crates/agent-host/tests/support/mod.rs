//! 통합 테스트 공용 도구: 테스트 대역 설치, 대역 전용 profile, 대역 서명 확인 수단, 디렉터리 hash.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use stellar_agent_host::decoder::DecoderKind;
use stellar_agent_host::output::SessionOutput;
use stellar_agent_host::preflight::{preflight, PreflightInput, PreflightPass, PreflightRefusal};
use stellar_agent_host::process::McpServerLaunch;
use stellar_agent_host::profile::LaunchProfile;
use stellar_agent_host::raw::DEFAULT_RAW_CAP_BYTES;
use stellar_agent_host::target::{confirm_target, CodeVerifier, ConfirmedTarget, SignatureError};
use stellar_local_ipc::binding::BindingToken;
use stellar_local_ipc::code_identity::CodeRule;

/// 테스트 대역 실행 파일.
pub const DOUBLE: &str = env!("CARGO_BIN_EXE_stellar-agent-host-test-cli");

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 테스트별 임시 디렉터리(정규 경로). drop하면 지웁니다.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let dir = base.join(format!(
            "stellar-agent-test-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 대역을 `dir/bin/<name>`에 복사하고 scenario를 둡니다. 실행 파일 경로를 돌려줍니다.
pub fn install_double(dir: &Path, name: &str, scenario: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(name);
    std::fs::copy(DOUBLE, &exe).unwrap();
    std::fs::write(bin.join(format!("{name}.scenario")), scenario).unwrap();
    exe
}

/// 대역 전용 profile fixture.
pub fn double_profile(kind: &str) -> LaunchProfile {
    let path = format!(
        "{}/tests/fixtures/profiles/test-double-{kind}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    LaunchProfile::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// 대역 서명 확인 수단. 대역은 공급자 서명을 가질 수 없으므로 결과를 정해 두고 호출 수를 셉니다.
pub struct FixtureVerifier {
    pub result: Result<(), SignatureError>,
    pub calls: AtomicUsize,
}

impl FixtureVerifier {
    pub fn accepting() -> FixtureVerifier {
        FixtureVerifier {
            result: Ok(()),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn rejecting(error: SignatureError) -> FixtureVerifier {
        FixtureVerifier {
            result: Err(error),
            calls: AtomicUsize::new(0),
        }
    }
}

impl CodeVerifier for FixtureVerifier {
    fn verify(&self, path: &Path, rule: &CodeRule) -> Result<(), SignatureError> {
        assert!(path.is_absolute());
        assert!(rule.requirement_text().contains("TESTDBL001"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result
    }
}

/// 가짜 앱 환경: 허용 목록 이름 일부 + 목록 밖 비밀 변수.
pub fn fake_app_env(home: &Path) -> Vec<(OsString, OsString)> {
    vec![
        ("HOME".into(), home.as_os_str().to_owned()),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "ko_KR.UTF-8".into()),
        ("TMPDIR".into(), std::env::temp_dir().into_os_string()),
        ("STELLAR_TEST_SECRET".into(), "must-not-pass".into()),
        ("ANTHROPIC_API_KEY".into(), "must-not-pass".into()),
        ("OPENAI_API_KEY".into(), "must-not-pass".into()),
        ("DYLD_INSERT_LIBRARIES".into(), "/nonexistent.dylib".into()),
    ]
}

/// 실행 전 확인을 대역으로 실행합니다.
pub fn run_preflight(
    profile: &LaunchProfile,
    target: &ConfirmedTarget,
    verifier: &dyn CodeVerifier,
    home: &Path,
    scratch: &Path,
) -> Result<PreflightPass, PreflightRefusal> {
    preflight(PreflightInput {
        profile,
        target,
        verifier,
        app_env: fake_app_env(home),
        scratch_base: scratch.to_path_buf(),
    })
}

/// 대역 세션 준비 결과.
pub struct DoubleSession {
    pub dir: TempDir,
    pub exe: PathBuf,
    pub home: PathBuf,
    pub project: PathBuf,
    pub scratch: PathBuf,
    pub profile: LaunchProfile,
    pub target: ConfirmedTarget,
}

/// 표준 대역 배치: bin/, home/(사용자 CLI 설정 흉내), project/(working tree 흉내), scratch/.
pub fn setup(tag: &str, kind: &str, scenario: &str) -> DoubleSession {
    let dir = TempDir::new(tag);
    let name = if kind == "claude" { "claude" } else { "codex" };
    let exe = install_double(dir.path(), name, scenario);
    let home = dir.join("home");
    let project = dir.join("project");
    let scratch = dir.join("scratch");
    for d in [&home, &project, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(home.join("cli-config")).unwrap();
    std::fs::write(home.join("cli-config/settings.json"), "{\"user\":true}").unwrap();
    std::fs::create_dir_all(project.join("design")).unwrap();
    std::fs::write(project.join("design/page.txt"), "frame\n").unwrap();
    let profile = double_profile(kind);
    let target = confirm_target(&exe, &profile.cli).unwrap();
    DoubleSession {
        dir,
        exe,
        home,
        project,
        scratch,
        profile,
        target,
    }
}

/// 대역 MCP 서버 정보(결합 token 포함).
pub fn mcp() -> McpServerLaunch {
    McpServerLaunch {
        command: PathBuf::from("/Applications/Stellar.app/Contents/Helpers/stellar-mcp"),
        args: vec!["attach".into()],
        binding: BindingToken::from_bytes([7u8; 32]),
    }
}

/// 새 출력 수신처.
pub fn output_for(profile: &LaunchProfile) -> Arc<SessionOutput> {
    Arc::new(SessionOutput::new(
        DecoderKind::resolve(&profile.decoder).unwrap(),
        DEFAULT_RAW_CAP_BYTES,
    ))
}

/// 디렉터리 아래 모든 파일의 (상대 경로 → 내용) 지도. 내용 비교로 hash를 대신합니다.
pub fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.insert(path.strip_prefix(root).unwrap().join(""), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// 조건이 참이 될 때까지 기다립니다.
pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}

/// 표준 성공 version/status 절.
pub const OK_PROBES: &str = "[version]\nout 2.1.300 (Claude Code)\\n\n[status]\nexit 0\n";
/// codex 성공 probe.
pub const OK_PROBES_CODEX: &str = "[version]\nout codex-cli 0.98.0\\n\n[status]\nexit 0\n";
