//! `stellar git status → stage → commit` 실제 실행 파일 통합 테스트(ADR-0080). 로컬 저장소만 씁니다.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::test_support::{
    large_document, project_id, save_documents, scaffold_project, web_frame, FileTrustStore,
    FixtureDocument, ProjectIdentity, TempDir,
};

struct Repo {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/xyz.taeyoon.stellar")
    } else {
        home.join(".local/share/xyz.taeyoon.stellar")
    }
}

impl Repo {
    fn new(tag: &str, baseline: &FixtureDocument, trusted: bool) -> Repo {
        let temp = TempDir::new(tag);
        let home = temp.path.join("home");
        let work = temp.path.join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "CLI Fixture"),
            ("user.email", "cli@stellar.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            git(&work, &["config", k, v]);
        }
        scaffold_project(&work);
        save_documents(&work, &[baseline]);
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "baseline"]);
        if trusted {
            let identity = ProjectIdentity::new(project_id(), &work).unwrap();
            FileTrustStore::open(&data_dir(&home))
                .unwrap()
                .grant(&identity)
                .unwrap();
        }
        Repo {
            _temp: temp,
            home,
            work,
        }
    }

    fn stellar(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stellar"))
            .arg("git")
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
        self.work.to_str().unwrap()
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
    assert_eq!(text.matches('\n').count(), 1, "{text}");
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn result(value: &JsonValue) -> &JsonValue {
    value.get("result").unwrap()
}

fn changes<'a>(status: &'a JsonValue, area: &str) -> Vec<&'a str> {
    result(status)
        .get("semantic")
        .unwrap()
        .get("changes")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("area").unwrap().as_str() == Some(area))
        .map(|c| c.get("id").unwrap().as_str().unwrap())
        .collect()
}

fn edited(model: &FixtureDocument) -> FixtureDocument {
    let mut records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter_mut()
        .find(|n| n.parent.is_none())
        .unwrap();
    root.display_name = "cli name".into();
    root.locked = true;
    FixtureDocument::from_records(records).unwrap().0
}

#[test]
fn status_stage_commit_then_git_log_shows_commit() {
    let model = large_document(4);
    let repo = Repo::new("cli-git-flow", &model, true);
    save_documents(&repo.work, &[&edited(&model)]);

    let status = envelope(&repo.stellar(&["status", repo.path()]), 0);
    let r = result(&status);
    assert_eq!(r.get("worktree").unwrap().as_str(), Some("main"));
    assert_eq!(
        r.get("head").unwrap().get("branch").unwrap().as_str(),
        Some("main")
    );
    assert_eq!(
        r.get("identity")
            .unwrap()
            .get("configured")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    let unstaged = changes(&status, "unstaged");
    assert_eq!(unstaged.len(), 2);
    let chosen = unstaged[0].to_string();

    let staged = envelope(&repo.stellar(&["stage", repo.path(), &chosen]), 0);
    assert_eq!(
        result(&staged).get("applied_changes").unwrap().as_i64(),
        Some(1)
    );
    let after = envelope(&repo.stellar(&["status", repo.path()]), 0);
    assert_eq!(changes(&after, "staged"), vec![chosen.as_str()]);
    assert_eq!(changes(&after, "unstaged").len(), 1);

    let message = "CLI 의미 단위 커밋";
    let committed = envelope(&repo.stellar(&["commit", repo.path(), "-m", message]), 0);
    let commit = result(&committed).get("commit").unwrap().as_str().unwrap();
    assert_eq!(
        result(&committed).get("staged_remaining").unwrap().as_i64(),
        Some(0)
    );
    let log = git(&repo.work, &["log", "-1", "--format=%H %s"]);
    assert_eq!(log.trim(), format!("{commit} {message}"));
    assert!(git(&repo.work, &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(
        git(&repo.work, &["rev-list", "--count", "HEAD"]).trim(),
        "2"
    );
    // 남은 변경은 그대로 unstaged입니다.
    let last = envelope(&repo.stellar(&["status", repo.path()]), 0);
    assert!(changes(&last, "staged").is_empty());
    assert_eq!(changes(&last, "unstaged").len(), 1);
}

#[test]
fn untrusted_project_is_refused_without_changes() {
    let model = large_document(4);
    let repo = Repo::new("cli-git-untrusted", &model, false);
    save_documents(&repo.work, &[&edited(&model)]);
    let index = std::fs::read(repo.work.join(".git/index")).unwrap();
    for args in [
        vec!["status", repo.path()],
        vec!["stage", repo.path(), "generation"],
        vec!["commit", repo.path(), "-m", "x"],
    ] {
        let value = envelope(&repo.stellar(&args), 77);
        assert_eq!(
            value.get("error").unwrap().get("code").unwrap().as_str(),
            Some("not-trusted"),
            "{args:?}"
        );
    }
    assert_eq!(std::fs::read(repo.work.join(".git/index")).unwrap(), index);
    assert_eq!(
        git(&repo.work, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[test]
fn companion_component_is_listed_and_included_only_with_approval() {
    use stellar_ops::git::{SourceUnit, COMPONENTS_DIR};
    let repo = Repo::new("cli-git-companion", &web_frame::document(), true);
    let dir = repo.work.join(COMPONENTS_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    stellar_ops::component_source::create(
        &dir,
        "Card".into(),
        web_frame::id(70),
        web_frame::id(7_001),
    )
    .unwrap();
    save_documents(&repo.work, &[&web_frame::placed_components("")]);

    let refused = envelope(&repo.stellar(&["stage", repo.path(), "generation"]), 1);
    assert_eq!(
        refused.get("error").unwrap().get("code").unwrap().as_str(),
        Some("companion-required")
    );
    let required: Vec<String> = result(&refused)
        .get("required")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(required.len(), 1);
    assert!(required[0].starts_with(&format!("file:{COMPONENTS_DIR}")));
    assert!(git(&repo.work, &["diff", "--cached", "--name-only"]).is_empty());

    envelope(
        &repo.stellar(&["stage", repo.path(), "generation", "--with", &required[0]]),
        0,
    );
    let status = envelope(&repo.stellar(&["status", repo.path()]), 0);
    let units = result(&status).get("units").unwrap().as_array().unwrap();
    for id in [SourceUnit::Generation.to_string(), required[0].clone()] {
        let unit = units
            .iter()
            .find(|u| u.get("id").unwrap().as_str() == Some(id.as_str()))
            .unwrap();
        assert_eq!(unit.get("staged").unwrap().as_bool(), Some(true), "{id}");
        assert_eq!(unit.get("unstaged").unwrap().as_bool(), Some(false), "{id}");
    }
    envelope(&repo.stellar(&["commit", repo.path(), "-m", "배치"]), 0);
    assert!(git(&repo.work, &["status", "--porcelain"]).is_empty());
}

#[test]
fn git_usage_errors_write_nothing_to_stdout() {
    let repo = Repo::new("cli-git-usage", &large_document(2), true);
    for args in [
        vec![],
        vec!["push", repo.path()],
        vec!["status"],
        vec!["status", repo.path(), "--limit", "0"],
        vec!["status", repo.path(), "--bogus", "1"],
        vec!["stage", repo.path()],
        vec!["stage", repo.path(), "not-an-item"],
        vec!["stage", repo.path(), "generation", "--with"],
        vec!["commit", repo.path()],
        vec!["commit", repo.path(), "--amend"],
    ] {
        let output = repo.stellar(&args);
        assert_eq!(output.status.code(), Some(64), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}
