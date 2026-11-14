//! XVIII.5.3: 일반 Git worktree 생성·사용은 시스템 Git에 남기고, terminal은 cwd preset만 제공합니다.
//!
//! - workspace(모든 Rust workspace member, macOS 앱 Swift)에 agent 전용 작업 사본 타입
//!   (`ShadowWorktree`, `AgentCheckout`, `ShadowWorkspace` 등)이 없습니다.
//! - terminal crate는 Git을 실행하지 않고 worktree 타입을 정의하지 않습니다. worktree root는 호출자가
//!   시스템 Git으로 얻어 [`CwdContext`]에 공급하고, terminal은 그 목록을 cwd preset으로만 씁니다.
//! - 시스템 Git(`git worktree add`)으로 만든 linked worktree가 terminal cwd preset이 되고 그 cwd에서
//!   세션이 시작됩니다(macOS).

use std::path::{Path, PathBuf};

use stellar_terminal::cwd::{CwdContext, CwdPreset};

/// worktree와 같은 작업 사본 개념을 가리키는 이름 조각(소문자·`_` 제거 비교).
const WORKTREE_NOUNS: &[&str] = &[
    "worktree",
    "checkout",
    "workspace",
    "workdir",
    "workingcopy",
];

/// 작업 사본 타입 이름에 붙으면 agent 전용으로 보는 표시(소문자·`_` 제거 비교).
const AGENT_ONLY_MARKERS: &[&str] = &["shadow", "agent", "sandbox", "ephemeral"];

/// Rust 타입을 찾는 workspace 디렉터리. 모든 workspace member가 이 아래에 있어야 합니다
/// (`scanned_rust_roots_cover_every_workspace_member`).
const RUST_ROOTS: &[&str] = &["crates", "tools", "benchmarks", "tests"];

const RUST_TYPE_KEYWORDS: &[&str] = &["struct", "enum", "trait", "type", "union"];
const SWIFT_TYPE_KEYWORDS: &[&str] = &["class", "struct", "enum", "protocol", "typealias", "actor"];

fn workspace_root() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

/// `dir` 아래 `extension` 파일(빌드 산출물·숨김 디렉터리 제외).
fn collect(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name.starts_with('.') || matches!(name, "target" | "build" | "DerivedData") {
                continue;
            }
            collect(&path, extension, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

/// 선언된 타입 이름(키워드 다음 식별자).
fn declared_types(text: &str, keywords: &[&str]) -> Vec<String> {
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
        .collect();
    tokens
        .windows(2)
        .filter(|pair| keywords.contains(&pair[0]))
        .map(|pair| pair[1].to_owned())
        .collect()
}

/// 비교용 이름: 소문자, `_` 제거(`Shadow_Worktree`·`shadow_worktree`도 같은 이름).
fn normalized(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// 이름이 작업 사본(worktree·checkout·workspace 등) 개념인지.
fn is_worktree_type(name: &str) -> bool {
    let name = normalized(name);
    WORKTREE_NOUNS.iter().any(|noun| name.contains(noun))
}

fn is_agent_only_worktree_type(name: &str) -> bool {
    is_worktree_type(name)
        && AGENT_ONLY_MARKERS
            .iter()
            .any(|marker| normalized(name).contains(marker))
}

/// workspace의 (파일, 타입 이름) 중 worktree 타입.
fn worktree_types(root: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for (dirs, extension, keywords) in [
        (RUST_ROOTS, "rs", RUST_TYPE_KEYWORDS),
        (&["apps"][..], "swift", SWIFT_TYPE_KEYWORDS),
    ] {
        let mut files = Vec::new();
        for dir in dirs {
            collect(&root.join(dir), extension, &mut files);
        }
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for name in declared_types(&text, keywords) {
                if is_worktree_type(&name) {
                    found.push((file.clone(), name));
                }
            }
        }
    }
    found
}

#[test]
fn detector_flags_agent_only_worktree_types() {
    // 위반 fixture. 이름은 실행 중에 만들어 이 파일 자체가 workspace 검사에 걸리지 않게 합니다.
    let shadow = ["Shadow", "Worktree"].concat();
    let lease = ["Agent", "Worktree", "Lease"].concat();
    let controller = ["Sandbox", "Worktree", "Controller"].concat();
    let violation = format!("pub struct {shadow} {{ root: PathBuf }}\nenum {lease} {{}}");
    let names: Vec<String> = declared_types(&violation, RUST_TYPE_KEYWORDS)
        .into_iter()
        .filter(|name| is_agent_only_worktree_type(name))
        .collect();
    assert_eq!(names, [shadow, lease]);
    let swift = format!("final class {controller}: NSObject {{}}");
    assert_eq!(
        declared_types(&swift, SWIFT_TYPE_KEYWORDS),
        std::slice::from_ref(&controller)
    );
    assert!(is_agent_only_worktree_type(&controller));
    // worktree와 같은 작업 사본 개념의 다른 이름도 검출합니다.
    for name in [
        ["Agent", "Checkout"].concat(),
        ["Shadow", "Workspace"].concat(),
        ["Ephemeral", "Work", "Dir"].concat(),
        ["sandbox", "_working", "_copy"].concat(),
    ] {
        assert!(is_agent_only_worktree_type(&name), "{name}");
    }
    // 일반 Git worktree 타입과 enum variant, 표시(drop shadow) 타입은 해당하지 않습니다.
    for name in [
        "WorktreeEntry",
        "SparseCheckoutState",
        "WorkspaceGraph",
        "ShadowFx",
    ] {
        assert!(!is_agent_only_worktree_type(name), "{name}");
    }
    assert!(declared_types(
        "enum CwdPreset { WorktreeRoot { path } }",
        RUST_TYPE_KEYWORDS
    )
    .iter()
    .all(|name| !is_worktree_type(name)));
}

/// 검사 대상 Rust 디렉터리가 workspace member를 모두 덮습니다(member가 늘어도 빠지지 않게).
#[test]
fn scanned_rust_roots_cover_every_workspace_member() {
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let members = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("members"))
        .and_then(|rest| rest.split_once('[').map(|(_, list)| list))
        .and_then(|list| list.split_once(']').map(|(list, _)| list))
        .expect("루트 Cargo.toml에 한 줄 `members = [...]`가 없습니다");
    let members: Vec<&str> = members
        .split(',')
        .map(|member| member.trim().trim_matches('"'))
        .filter(|member| !member.is_empty())
        .collect();
    assert!(members.len() >= 10, "{members:?}");
    let uncovered: Vec<&str> = members
        .iter()
        .copied()
        .filter(|member| {
            !RUST_ROOTS
                .iter()
                .any(|dir| Path::new(member).starts_with(dir))
        })
        .collect();
    assert!(uncovered.is_empty(), "검사하지 않는 member: {uncovered:?}");
}

#[test]
fn workspace_has_no_agent_only_worktree_types() {
    let root = workspace_root();
    let all = worktree_types(&root);
    // 검사가 실제로 workspace를 읽었는지(일반 Git worktree 타입은 있음).
    assert!(
        !all.is_empty(),
        "worktree 타입을 하나도 찾지 못했습니다: {root:?}"
    );
    let agent_only: Vec<String> = all
        .iter()
        .filter(|(_, name)| is_agent_only_worktree_type(name))
        .map(|(file, name)| format!("{}: {name}", file.display()))
        .collect();
    assert!(
        agent_only.is_empty(),
        "agent 전용 worktree 타입: {agent_only:?}"
    );
}

#[test]
fn terminal_crate_does_not_run_git_or_define_worktree_types() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&crate_root.join("src"), "rs", &mut files);
    assert!(files.len() >= 20, "{files:?}");
    let mut problems = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for name in declared_types(&text, RUST_TYPE_KEYWORDS) {
            if is_worktree_type(&name) {
                problems.push(format!("{}: 타입 {name}", file.display()));
            }
        }
        for needle in [
            "\"git\"",
            "worktree add",
            "worktree remove",
            "git-porcelain",
        ] {
            if text.contains(needle) {
                problems.push(format!("{}: {needle}", file.display()));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:?}");
}

/// 테스트가 끝나면 지우는 canonical 임시 디렉터리.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "stellar-terminal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(std::fs::canonicalize(path).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 사용자 Git 설정의 영향을 받지 않는 시스템 Git 실행.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("시스템 git을 실행하지 못했습니다");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn system_git_worktrees_become_cwd_presets() {
    let dir = TempDir::new("worktree-presets");
    let repo = dir.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=stellar-test",
            "-c",
            "user.email=stellar-test@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    // worktree 생성은 시스템 Git이 합니다(사용자가 외부 터미널이나 Git UI에서 하는 것과 같음).
    let linked = dir.0.join("repo-feature");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );
    // worktree root 목록도 시스템 Git이 보고한 값입니다.
    let roots: Vec<PathBuf> = git(&repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect();
    assert_eq!(roots.len(), 2, "{roots:?}");
    let context = CwdContext::new(&repo, &roots).unwrap();
    let linked = std::fs::canonicalize(linked).unwrap();
    assert_eq!(
        context.presets(),
        [
            CwdPreset::ProjectRoot { path: repo.clone() },
            CwdPreset::WorktreeRoot {
                path: linked.clone()
            },
        ]
    );

    #[cfg(target_os = "macos")]
    {
        let pwd = start_pwd_in(&context, &context.presets()[1]);
        assert_eq!(pwd, linked.to_str().unwrap());
        // terminal은 worktree 목록을 바꾸지 않습니다.
        let after = git(&repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count();
        assert_eq!(after, 2);
    }
}

/// preset cwd에서 `/bin/pwd`를 일반 profile로 실행해 출력 첫 줄을 돌려줍니다.
#[cfg(target_os = "macos")]
fn start_pwd_in(context: &CwdContext, preset: &CwdPreset) -> String {
    use std::ffi::OsString;
    use std::io::Read;
    use std::time::{Duration, Instant};

    use stellar_core_types::ids::{StableId, WindowSessionId};
    use stellar_terminal::capability::{TerminalCapability, TerminalTrust};
    use stellar_terminal::profile::{ArgvCommand, ProfileName, TerminalProfile};
    use stellar_terminal::pty::PtyLauncher;
    use stellar_terminal::registry::{LaunchEnvironment, TerminalRegistry};
    use stellar_terminal::shell::{ShellSettings, SystemShellSource};
    use stellar_terminal::size::TerminalSize;

    fn env_lookup(name: &str) -> Option<OsString> {
        (name == "PATH").then(|| OsString::from("/usr/bin:/bin"))
    }

    let window = WindowSessionId::from_stable_id(StableId::from_bytes([53; 16]));
    let mut registry = TerminalRegistry::new(window);
    let mut profile = TerminalProfile::command(
        ProfileName::new("pwd").unwrap(),
        ArgvCommand::new("/bin/pwd", ["-P"]).unwrap(),
    );
    profile.cwd = preset.selection();
    let id = registry.create(profile).unwrap();
    let shell = ShellSettings::default();
    let environment = LaunchEnvironment {
        cwd: context,
        shell: &shell,
        shell_source: &SystemShellSource,
        env_lookup,
        size: TerminalSize::DEFAULT,
    };
    let capability = TerminalCapability::grant(window, TerminalTrust::Trusted).unwrap();
    let mut launcher = PtyLauncher::with_grace(Duration::from_millis(300));
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !registry.poll().contains(&id) {
        assert!(Instant::now() < deadline, "pwd가 끝나지 않았습니다");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = String::from_utf8(collector.join().unwrap()).unwrap();
    registry.close(id).unwrap();
    output.lines().next().unwrap_or("").trim_end().to_owned()
}
