//! `stellar library add|import|fetch|update`와 `stellar diagnose` 문서 참조 진단 실제 실행 파일 시험(ADR-0137).
//!
//! 로컬 `file://` library 저장소만 씁니다. 공용 cache는 시험 HOME 아래 OS 사용자 데이터 위치입니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::library_fixture::*;
use stellar_ops::test_support::TempDir;

const SPACE: u64 = 9;

struct Env {
    _temp: TempDir,
    home: PathBuf,
    project: PathBuf,
    libraries: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Option<Env> {
        if !has_git() {
            eprintln!("시스템 Git 없음: 건너뜀");
            return None;
        }
        let temp = TempDir::new(tag);
        let home = temp.path.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let project = using_project(
            &temp.path,
            document(SPACE, &[(sid(SPACE, 100), instance_of(sid(SPACE, 1)))]),
        );
        let libraries = temp.path.join("libraries");
        Some(Env {
            _temp: temp,
            home,
            project,
            libraries,
        })
    }

    fn stellar(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stellar"))
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn path(&self) -> &str {
        self.project.to_str().unwrap()
    }

    fn files(&self) -> (Vec<u8>, Vec<u8>) {
        (
            std::fs::read(self.project.join("stellar.yaml")).unwrap(),
            std::fs::read(self.project.join("stellar.lock.yaml")).unwrap(),
        )
    }
}

fn envelope(output: &Output, exit: i32) -> JsonValue {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::str::from_utf8(&output.stdout).unwrap();
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn result(value: &JsonValue) -> &JsonValue {
    value.get("result").unwrap()
}

fn reuse(lock: &JsonValue) -> Vec<String> {
    lock.get("nodes")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.get("reuse").unwrap().as_str().unwrap().to_string())
        .collect()
}

fn diagnose_codes(env: &Env) -> Vec<String> {
    let page = envelope(&env.stellar(&["diagnose", env.path(), "--limit", "100"]), 0);
    result(&page)
        .get("items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.get("code").unwrap().as_str().unwrap().to_string())
        .filter(|c| c.starts_with("library"))
        .collect()
}

fn release(repo: &LibraryRepo, ids: &[u64], tag: &str) -> String {
    let defs: Vec<(_, _, String)> = ids
        .iter()
        .map(|n| (sid(SPACE, *n), sid(SPACE, 500 + n), format!("C{n}")))
        .collect();
    let borrowed: Vec<_> = defs.iter().map(|(a, b, c)| (*a, *b, c.as_str())).collect();
    repo.release(SPACE, components(&borrowed), tag)
}

fn library_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/xyz.taeyoon.stellar/library-cache")
    } else {
        home.join(".local/share/xyz.taeyoon.stellar/library-cache")
    }
}

#[test]
fn add_import_fetch_update_through_the_cli() {
    let Some(env) = Env::new("cli-library") else {
        return;
    };
    let repo = LibraryRepo::init(&env.libraries, "ds", 51);
    let v1 = release(&repo, &[1, 2], "v1");
    let v2 = release(&repo, &[2], "v2");
    let card = sid(SPACE, 1).to_string();
    let badge = sid(SPACE, 2).to_string();

    // 저장 참조 없이 추가하면 문서 참조 진단이 저장을 알립니다.
    let added = envelope(
        &env.stellar(&["library", "add", env.path(), "ds", &repo.url, "--tag", "v1"]),
        0,
    );
    let lock = result(&added).get("lock").unwrap();
    assert_eq!(reuse(lock), ["created"]);
    assert_eq!(lock.get("lock_written").unwrap().as_bool(), Some(true));
    assert_eq!(
        lock.get("nodes").unwrap().as_array().unwrap()[0]
            .get("commit")
            .unwrap()
            .as_str(),
        Some(v1.as_str())
    );
    assert!(
        library_dir(&env.home).join("repository-key").is_file(),
        "제품 cache key"
    );
    let stdout = String::from_utf8_lossy(&env.stellar(&["library", "fetch", env.path()]).stdout)
        .into_owned();
    assert!(!stdout.contains(&repo.url), "결과에 URL 원문 없음");
    assert_eq!(diagnose_codes(&env), ["library-ref/undeclared"]);

    // import로 참조를 저장하면 진단이 없어집니다. export되지 않는 component는 종료 코드 1입니다.
    let imported = envelope(
        &env.stellar(&["library", "import", env.path(), "ds", &card]),
        0,
    );
    assert_eq!(
        result(&imported).get("manifest_written").unwrap().as_bool(),
        Some(true)
    );
    let unknown = sid(SPACE, 77).to_string();
    let refused = envelope(
        &env.stellar(&["library", "import", env.path(), "ds", &unknown]),
        1,
    );
    assert_eq!(
        refused.get("error").unwrap().get("code").unwrap().as_str(),
        Some("unknown-component")
    );
    assert!(
        diagnose_codes(&env).is_empty(),
        "{:?}",
        diagnose_codes(&env)
    );

    // fetch: cache에 있으면 Git 없이 끝납니다.
    let fetched = envelope(&env.stellar(&["library", "fetch", env.path()]), 0);
    assert_eq!(reuse(result(&fetched)), ["commit-hit"]);

    // dry-run 검토: v2는 Card를 지워 instance가 끊깁니다.
    let before = env.files();
    let review = envelope(
        &env.stellar(&[
            "library",
            "update",
            env.path(),
            "ds",
            "--tag",
            "v2",
            "--dry-run",
        ]),
        0,
    );
    let r = result(&review);
    assert_eq!(r.get("applied").unwrap().as_bool(), Some(false));
    assert_eq!(r.get("previous").unwrap().as_str(), Some(v1.as_str()));
    assert_eq!(r.get("candidate").unwrap().as_str(), Some(v2.as_str()));
    assert_eq!(r.get("orphans").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        r.get("diff").unwrap().get("removed").unwrap().as_i64(),
        Some(1)
    );
    assert_eq!(env.files(), before);

    // 승인 없이 적용하면 종료 코드 1, 파일 불변.
    let blocked = envelope(
        &env.stellar(&["library", "update", env.path(), "ds", "--tag", "v2"]),
        1,
    );
    assert_eq!(
        blocked.get("error").unwrap().get("code").unwrap().as_str(),
        Some("orphans")
    );
    assert_eq!(
        result(&blocked)
            .get("orphans")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(env.files(), before);

    // 승인하면 적용합니다. 저장된 Card 참조는 이제 library에 없어 해석 실패 진단입니다.
    let applied = envelope(
        &env.stellar(&[
            "library",
            "update",
            env.path(),
            "ds",
            "--tag",
            "v2",
            "--allow-orphans",
        ]),
        0,
    );
    assert_eq!(
        result(&applied).get("applied").unwrap().as_bool(),
        Some(true)
    );
    assert_ne!(env.files(), before);
    assert_eq!(diagnose_codes(&env), ["library-ref/unresolved"]);

    // 참조를 지우고(--remove, library를 읽지 않음) Badge를 저장합니다.
    envelope(
        &env.stellar(&["library", "import", env.path(), "ds", &card, "--remove"]),
        0,
    );
    let badge_import = envelope(
        &env.stellar(&["library", "import", env.path(), "ds", &badge]),
        0,
    );
    assert_eq!(
        result(&badge_import)
            .get("components")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // 사용법 오류(stderr만, 종료 코드 64).
    for args in [
        vec!["library", "add", env.path(), "x", repo.url.as_str()],
        vec![
            "library",
            "add",
            env.path(),
            "x",
            repo.url.as_str(),
            "--tag",
            "v1",
            "--branch",
            "main",
        ],
        vec!["library", "import", env.path(), "ds", "NOT-A-UUID"],
        vec!["library", "fetch"],
    ] {
        assert_eq!(env.stellar(&args).status.code(), Some(64), "{args:?}");
    }
}
