//! 대화형 터미널 child 환경 검사(XVIII.2.5, ADR-0023).
//!
//! - 실제 PTY에서 login 셸이 임시 HOME의 profile 파일로 더한 `PATH` 항목과 marker 변수가 child에 보입니다.
//! - 앱(이 시험 프로세스) 환경에만 있는 변수는 옛 허용 목록 밖이어도 child에 보입니다.
//! - 식별 변수 4개는 spawn 입력 값이고, 기본 셸 profile의 `SHELL`은 실행한 셸, argv profile의 `SHELL`은 상속값입니다.
//! - Stellar가 더한 변수는 식별 변수 4개와 `SHELL`뿐이고, profile 명시 지정이 마지막에 이깁니다.
//! - 세션 전후 rc·profile 파일 bytes가 같고, 대화형 spawn 경로 소스에 `env_clear`·상속 허용 목록 사용이 없습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_terminal::capability::{TerminalCapability, TerminalTrust};
use stellar_terminal::cwd::CwdContext;
use stellar_terminal::env::{TerminalIdentity, STELLAR_SET_VARIABLES};
use stellar_terminal::profile::{ArgvCommand, ProfileName, TerminalProfile};
use stellar_terminal::registry::{LaunchEnvironment, TerminalRegistry};
use stellar_terminal::shell::{ShellChoice, ShellMode, ShellSettings, SystemShellSource};
use stellar_terminal::size::TerminalSize;

/// 대화형 spawn 경로의 소스(주석 줄 제외)에 남으면 안 되는 표현(ADR-0023 결정 3).
const FORBIDDEN: [&str; 5] = [
    "env_clear(",
    "DEFAULT_INHERITED_ENV",
    "with_defaults(",
    "resolve_from_process(",
    ".inherit(",
];

#[test]
fn interactive_spawn_path_has_no_env_clear_or_inherited_allowlist() {
    let sources = [
        ("env.rs", include_str!("../src/env.rs")),
        ("launch.rs", include_str!("../src/launch.rs")),
        ("profile.rs", include_str!("../src/profile.rs")),
        ("registry.rs", include_str!("../src/registry.rs")),
        ("pty/mod.rs", include_str!("../src/pty/mod.rs")),
    ];
    let mut found = Vec::new();
    for (name, text) in sources {
        for (line_no, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for needle in FORBIDDEN {
                if code.contains(needle) {
                    found.push(format!("{name}:{}: {needle}", line_no + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "inventory: {found:?}");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-interactive-env-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn identity() -> TerminalIdentity {
    TerminalIdentity::stellar("9.8.7-fixture").unwrap()
}

/// `env` 출력(줄마다 이름=값)을 표로 만듭니다.
fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim_end_matches('\r').split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// 세션을 시작하고(입력이 있으면 쓴 뒤) 끝날 때까지 기다려 출력 전체를 돌려줍니다.
fn run(
    profile: TerminalProfile,
    shell: &ShellSettings,
    cwd: &Path,
    input: Option<&[u8]>,
) -> String {
    let window = WindowSessionId::from_stable_id(StableId::from_bytes([77; 16]));
    let mut registry = TerminalRegistry::new(window);
    let context = CwdContext::new(cwd, &[]).unwrap();
    let identity = identity();
    let environment = LaunchEnvironment {
        cwd: &context,
        shell,
        shell_source: &SystemShellSource,
        identity: &identity,
        size: TerminalSize::DEFAULT,
    };
    let id = registry.create(profile).unwrap();
    let capability = TerminalCapability::grant(window, TerminalTrust::Trusted).unwrap();
    let mut launcher = stellar_terminal::pty::PtyLauncher::with_grace(Duration::from_millis(300));
    registry
        .start(id, capability, &environment, &mut launcher)
        .unwrap();
    let mut reader = registry
        .session(id)
        .unwrap()
        .process()
        .unwrap()
        .output_reader()
        .unwrap();
    let collector = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });
    if let Some(input) = input {
        registry
            .session_mut(id)
            .unwrap()
            .process_mut()
            .unwrap()
            .write_input(input)
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !registry.poll().contains(&id) {
        assert!(Instant::now() < deadline, "터미널 세션이 끝나지 않았습니다");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = String::from_utf8_lossy(&collector.join().unwrap()).into_owned();
    registry.close(id).unwrap();
    output
}

fn hash_files(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<u8>)> {
    paths
        .iter()
        .map(|p| (p.clone(), std::fs::read(p).unwrap()))
        .collect()
}

#[cfg(target_os = "macos")]
#[test]
fn login_shell_reads_user_profile_and_inherits_app_environment() {
    let home = TempDir::new("home");
    let fixture_bin = home.0.join("fixture-bin");
    std::fs::create_dir_all(&fixture_bin).unwrap();
    let profile_file = home.0.join(".bash_profile");
    let rc_file = home.0.join(".bashrc");
    std::fs::write(
        &profile_file,
        "export PATH=\"$HOME/fixture-bin:$PATH\"\nexport STELLAR_PROFILE_MARKER=from-profile\n. \"$HOME/.bashrc\"\n",
    )
    .unwrap();
    std::fs::write(&rc_file, "export STELLAR_RC_MARKER=from-rc\n").unwrap();
    let before = hash_files(&[profile_file.clone(), rc_file.clone()]);
    let out = home.0.join("child-env.out");

    // 계정 로그인 셸(macOS 기본 zsh)과 다른 셸을 고른 기본 셸 profile. HOME은 profile 명시 지정입니다.
    let shell = ShellSettings {
        choice: ShellChoice::Custom(PathBuf::from("/bin/bash")),
        mode: ShellMode::Login,
    };
    let mut profile = TerminalProfile::default_shell(ProfileName::new("login").unwrap());
    profile
        .environment
        .set("HOME", home.0.as_os_str().to_owned())
        .unwrap();
    profile
        .environment
        .set("BASH_SILENCE_DEPRECATION_WARNING", "1")
        .unwrap();
    let command = format!("/usr/bin/env > '{}'; exit 0\n", out.display());
    run(profile, &shell, &home.0, Some(command.as_bytes()));

    let child = parse_env(&std::fs::read_to_string(&out).unwrap());
    let fixture_bin_text = fixture_bin.display().to_string();
    assert!(
        child["PATH"]
            .split(':')
            .any(|entry| entry == fixture_bin_text),
        "profile이 더한 PATH 항목: {}",
        child["PATH"]
    );
    assert_eq!(
        child.get("STELLAR_PROFILE_MARKER").map(String::as_str),
        Some("from-profile")
    );
    assert_eq!(
        child.get("STELLAR_RC_MARKER").map(String::as_str),
        Some("from-rc")
    );
    // 앱 환경에만 있는 기존 허용 목록 밖 변수(cargo가 시험 프로세스에 넣음).
    assert_eq!(
        child.get("CARGO_MANIFEST_DIR"),
        std::env::var("CARGO_MANIFEST_DIR").ok().as_ref()
    );
    assert!(child.contains_key("CARGO_MANIFEST_DIR"));
    // 식별 변수 4개 = spawn 입력, SHELL = 실행한 셸.
    assert_eq!(
        child.get("TERM").map(String::as_str),
        Some("xterm-256color")
    );
    assert_eq!(
        child.get("COLORTERM").map(String::as_str),
        Some("truecolor")
    );
    assert_eq!(
        child.get("TERM_PROGRAM").map(String::as_str),
        Some("Stellar")
    );
    assert_eq!(
        child.get("TERM_PROGRAM_VERSION").map(String::as_str),
        Some("9.8.7-fixture")
    );
    assert_eq!(child.get("SHELL").map(String::as_str), Some("/bin/bash"));
    // 셸 설정 파일은 세션 전후 bytes가 같습니다(결정 6).
    assert_eq!(hash_files(&[profile_file, rc_file]), before);
}

#[cfg(target_os = "macos")]
#[test]
fn argv_profile_keeps_inherited_shell_and_stellar_adds_only_identity_variables() {
    let dir = TempDir::new("argv");
    let parent: BTreeMap<String, String> = std::env::vars().collect();
    let shell = ShellSettings::default();

    let profile = TerminalProfile::command(
        ProfileName::new("env").unwrap(),
        ArgvCommand::new("/usr/bin/env", Vec::<OsString>::new()).unwrap(),
    );
    let child = parse_env(&run(profile, &shell, &dir.0, None));
    // argv profile: SHELL은 상속값 그대로입니다.
    assert_eq!(child.get("SHELL"), parent.get("SHELL"));
    // Stellar가 더하거나 바꾼 변수는 식별 변수 4개와 SHELL 안에만 있습니다.
    let allowed: BTreeSet<&str> = STELLAR_SET_VARIABLES.into_iter().collect();
    let changed: Vec<&String> = child
        .iter()
        .filter(|(name, value)| parent.get(*name) != Some(*value))
        .map(|(name, _)| name)
        .filter(|name| !allowed.contains(name.as_str()))
        .collect();
    assert!(changed.is_empty(), "Stellar가 더한 변수: {changed:?}");
    // macOS SIP는 보호된 시스템 실행 파일(`/usr/bin/env`)을 exec할 때 `DYLD_*`를 지웁니다(OS 동작).
    let removed: Vec<&String> = parent
        .keys()
        .filter(|name| !name.starts_with("DYLD_"))
        .filter(|name| !child.contains_key(*name) && !allowed.contains(name.as_str()))
        .collect();
    assert!(removed.is_empty(), "child에서 빠진 앱 변수: {removed:?}");

    // profile 명시 지정이 식별 변수보다 이깁니다(결정 5).
    let mut profile = TerminalProfile::command(
        ProfileName::new("env-explicit").unwrap(),
        ArgvCommand::new("/usr/bin/env", Vec::<OsString>::new()).unwrap(),
    );
    profile
        .environment
        .set("TERM_PROGRAM", "profile-wins")
        .unwrap();
    profile.environment.set("SHELL", "/custom/shell").unwrap();
    let child = parse_env(&run(profile, &shell, &dir.0, None));
    assert_eq!(
        child.get("TERM_PROGRAM").map(String::as_str),
        Some("profile-wins")
    );
    assert_eq!(
        child.get("SHELL").map(String::as_str),
        Some("/custom/shell")
    );
    assert_eq!(
        child.get("TERM").map(String::as_str),
        Some("xterm-256color")
    );
}
