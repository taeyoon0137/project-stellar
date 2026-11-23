//! 텍스트 원본(Git 원본)을 CLI 편집이 쓰고 다시 읽는 시나리오(ADR-0113).
//!
//! `stellar text-source adopt`로 chunk 프로젝트를 텍스트 원본으로 옮긴 뒤, `stellar apply`는 원본과 로컬 cache를 함께
//! 쓰고, 원본만 바뀐 상태(Git pull)는 `stellar query`가 원본에서 읽습니다.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

const STELLAR: &str = env!("CARGO_BIN_EXE_stellar");

fn stellar(work: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(STELLAR)
        .args(args)
        .current_dir(work)
        .env_clear()
        .env("HOME", home)
        .output()
        .unwrap()
}

fn result(output: &Output, exit: i32) -> JsonValue {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse(
        std::str::from_utf8(&output.stdout).unwrap().trim(),
        JsonLimits::default(),
    )
    .unwrap()
}

fn string<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}

struct Repo {
    _fixture: TempDir,
    root: PathBuf,
    home: PathBuf,
}

impl Repo {
    /// 텍스트 원본으로 옮긴 프로젝트를 담은 Git 저장소(main에 commit).
    fn new(label: &str) -> Repo {
        let fixture = TempDir::new(label);
        let root = fixture.path.join("project");
        let home = fixture.path.join("home");
        fs::create_dir(&home).unwrap();
        fs::write(home.join(".gitconfig"), "").unwrap();
        scaffold_project(&root);
        save_documents(&root, &[&large_document(4)]);
        let repo = Repo {
            _fixture: fixture,
            root,
            home,
        };
        let before = repo.revision();
        let adopted = result(&repo.stellar(&["text-source", "adopt", "."]), 0);
        let adopted = adopted.get("result").unwrap();
        assert_eq!(string(adopted, "revision"), before, "generation은 그대로");
        assert!(repo.root.join("design/document.stellar.yaml").is_file());
        // 두 번째 adopt는 이미 원본이 있어 거부합니다.
        let again = result(&repo.stellar(&["text-source", "adopt", "."]), 1);
        assert_eq!(
            string(again.get("error").unwrap(), "code"),
            "text-source-present"
        );
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "base"]);
        repo
    }

    fn stellar(&self, args: &[&str]) -> Output {
        stellar(&self.root, &self.home, args)
    }

    fn git_output(&self, args: &[&str]) -> Output {
        Command::new("git")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("HOME", &self.home)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = self.git_output(args);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn revision(&self) -> String {
        let project = result(&self.stellar(&["query", ".", "project"]), 0);
        string(
            project.get("result").unwrap().get("revision").unwrap(),
            "token",
        )
        .to_owned()
    }

    fn apply(&self, label: &str, command: &str) {
        self.apply_with(label, command, &[]);
    }

    fn apply_with(&self, label: &str, command: &str, flags: &[&str]) -> JsonValue {
        let document = large_document(4).document_id().to_string();
        let token = self.revision();
        let input = format!(
            r#"{{"format":"stellar-apply","format_version":1,"base_revision":"{token}","document":"{document}","commands":[{command}]}}"#
        );
        let file = self.home.join(format!("{label}.json"));
        fs::write(&file, input).unwrap();
        let mut args = vec!["apply", ".", file.to_str().unwrap()];
        args.extend_from_slice(flags);
        result(&self.stellar(&args), 0)
    }

    fn node(&self, node: &str) -> JsonValue {
        let document = large_document(4).document_id().to_string();
        let found = result(&self.stellar(&["query", ".", "node", &document, node]), 0);
        found.get("result").unwrap().get("node").unwrap().clone()
    }

    fn view_file(&self) -> PathBuf {
        let pages = self.root.join("design/pages");
        let page = fs::read_dir(&pages)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::read_dir(&page)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_file())
            .unwrap()
    }

}

fn kids() -> Vec<String> {
    large_document(4)
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id.to_string())
        .collect()
}

#[test]
fn cli_edits_write_the_text_source_and_a_pulled_source_is_read_from_text() {
    let repo = Repo::new("text-cli");
    let kid = &kids()[0];
    let view = repo.view_file();
    let rename = format!(r#"{{"kind":"rename","node":"{kid}","name":"from-cli"}}"#);
    // dry-run은 semantic diff만 내고 원본·cache를 쓰지 않습니다.
    let before = repo.git(&["status", "--porcelain"]);
    let dry = repo.apply_with("dry", &rename, &["--dry-run"]);
    assert_eq!(string(dry.get("result").unwrap(), "mode"), "dry-run");
    assert_eq!(repo.git(&["status", "--porcelain"]), before);
    repo.apply("rename", &rename);
    assert!(fs::read_to_string(&view).unwrap().contains("from-cli"));
    assert_eq!(string(&repo.node(kid), "name"), "from-cli");
    // 원본 변경이 commit할 파일로 보이고, Git diff는 바뀐 이름 줄 하나입니다.
    let status = repo.git(&["status", "--porcelain", "--", "design/pages"]);
    assert!(status.contains(".stellar.yaml"), "{status}");
    let diff = repo.git(&["diff", "-U0", "--", "design/pages"]);
    let changed: Vec<&str> = diff
        .lines()
        .filter(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
        })
        .collect();
    assert_eq!(changed.len(), 2, "{diff}");
    assert!(changed[1].contains("display_name: from-cli"), "{diff}");
    // 다시 열어도(새 프로세스) 같은 값입니다.
    assert_eq!(string(&repo.node(kid), "name"), "from-cli");

    // Git pull처럼 원본만 바뀌면 조회는 원본을 읽고 revision이 바뀝니다.
    let before = repo.revision();
    let text = fs::read_to_string(&view).unwrap();
    fs::write(&view, text.replace("from-cli", "pulled")).unwrap();
    assert_eq!(string(&repo.node(kid), "name"), "pulled");
    assert_ne!(repo.revision(), before);
}
