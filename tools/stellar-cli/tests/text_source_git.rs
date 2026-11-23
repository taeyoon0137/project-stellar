//! 텍스트 원본(Git 원본)을 CLI 편집·merge driver·conflict 세션이 함께 쓰는 시나리오(ADR-0113).
//!
//! `stellar text-source adopt`로 chunk 프로젝트를 텍스트 원본으로 옮긴 뒤, `stellar apply`는 원본과 로컬 cache를 함께
//! 쓰고, 원본만 바뀐 상태(Git pull)는 `stellar query`가 원본에서 읽습니다. 두 branch가 같은 View 파일의 이웃한 줄을
//! 바꾸면 Git 텍스트 merge는 충돌하지만 driver는 의미 단위로 병합하고, 남은 cache pointer conflict는
//! `merge-driver resolve --apply`가 원본 결과로 다시 써서 unmerged가 0이 됩니다.
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

    fn trust_and_install(&self) {
        use stellar_ops::test_support::{project_id, FileTrustStore, ProjectIdentity};
        let data = self
            .home
            .join("Library/Application Support/xyz.taeyoon.stellar");
        FileTrustStore::open(&data)
            .unwrap()
            .grant(&ProjectIdentity::new(project_id(), &self.root.canonicalize().unwrap()).unwrap())
            .unwrap();
        result(&self.stellar(&["merge-driver", "install", "."]), 0);
        let attributes = fs::read_to_string(self.root.join(".gitattributes")).unwrap();
        assert!(
            attributes.contains("design/pages/**/*.stellar.yaml merge=stellar"),
            "{attributes}"
        );
        self.git(&["add", ".gitattributes"]);
        self.git(&["commit", "-qm", "driver"]);
    }

    fn branch(&self, name: &str, label: &str, command: &str) {
        self.git(&["switch", "-q", "main"]);
        self.git(&["switch", "-qc", name]);
        self.apply(label, command);
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", name]);
    }

    fn unmerged(&self) -> Vec<String> {
        let text = self.git(&["diff", "--name-only", "--diff-filter=U"]);
        text.lines().map(str::to_owned).collect()
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

#[test]
fn adjacent_line_edits_in_one_view_file_merge_through_the_driver_and_resolve_clears_the_cache_conflict(
) {
    let repo = Repo::new("text-driver");
    repo.trust_and_install();
    let kid = kids()[0].clone();
    repo.branch(
        "renamed",
        "rename",
        &format!(r#"{{"kind":"rename","node":"{kid}","name":"renamed"}}"#),
    );
    repo.branch(
        "locked",
        "lock",
        &format!(r#"{{"kind":"set-locked","node":"{kid}","locked":true}}"#),
    );
    // 대조군: driver 없는 Git 텍스트 merge는 이웃한 두 줄(locked·display_name) 변경을 충돌로 남깁니다.
    let view_path = repo
        .view_file()
        .strip_prefix(&repo.root)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let sides = repo.home.join("sides");
    fs::create_dir_all(&sides).unwrap();
    for (name, rev) in [("base", "main"), ("ours", "renamed"), ("theirs", "locked")] {
        let bytes = repo
            .git_output(&["show", &format!("{rev}:{view_path}")])
            .stdout;
        fs::write(sides.join(name), bytes).unwrap();
    }
    let textual = repo.git_output(&[
        "merge-file",
        "-p",
        sides.join("ours").to_str().unwrap(),
        sides.join("base").to_str().unwrap(),
        sides.join("theirs").to_str().unwrap(),
    ]);
    assert_ne!(textual.status.code(), Some(0), "텍스트 merge는 충돌");

    repo.git(&["switch", "-q", "renamed"]);
    let merged = repo.git_output(&["merge", "--no-edit", "locked"]);
    // View 파일은 driver가 의미 단위로 병합하고, chunk cache는 Git 밖이라(ADR-0122) conflict 후보가 없습니다.
    assert_eq!(
        merged.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&merged.stderr)
    );
    assert!(repo.unmerged().is_empty());
    let view = fs::read_to_string(repo.view_file()).unwrap();
    assert!(!view.contains("<<<<<<<"), "{view}");
    assert!(view.contains("renamed"));
    // merge commit의 tree에는 cache 경로가 없습니다.
    let tracked = repo.git(&["ls-files"]);
    assert!(
        !tracked
            .lines()
            .any(|l| l.starts_with(".stellar/") || l == "design/current"),
        "{tracked}"
    );
    // 병합 뒤 로컬 cache는 원본과 짝이 아니고, 열기는 원본에서 읽습니다.
    let node = repo.node(&kid);
    assert_eq!(string(&node, "name"), "renamed");
    assert_eq!(node.get("locked"), Some(&JsonValue::Bool(true)));
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[test]
fn a_real_text_source_conflict_is_listed_and_resolved_to_theirs() {
    let repo = Repo::new("text-conflict");
    repo.trust_and_install();
    let kid = kids()[0].clone();
    repo.branch(
        "ours",
        "ours",
        &format!(r#"{{"kind":"rename","node":"{kid}","name":"ours-name"}}"#),
    );
    repo.branch(
        "theirs",
        "theirs",
        &format!(r#"{{"kind":"rename","node":"{kid}","name":"theirs-name"}}"#),
    );
    repo.git(&["switch", "-q", "ours"]);
    let merged = repo.git_output(&["merge", "--no-edit", "theirs"]);
    assert_eq!(merged.status.code(), Some(1));
    let unmerged = repo.unmerged();
    assert!(
        unmerged.iter().any(|p| p.starts_with("design/pages/")),
        "{unmerged:?}"
    );
    let listed = result(&repo.stellar(&["merge-driver", "resolve", "."]), 0);
    let unit = &listed
        .get("result")
        .unwrap()
        .get("units")
        .unwrap()
        .as_array()
        .unwrap()[0];
    assert_eq!(string(unit, "kind"), "text-source");
    assert_eq!(unit.get("conflicts").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(unit.get("writable"), Some(&JsonValue::Bool(false)));

    result(
        &repo.stellar(&[
            "merge-driver",
            "resolve",
            ".",
            "--resolve",
            "0=theirs",
            "--apply",
        ]),
        0,
    );
    assert!(repo.unmerged().is_empty());
    assert_eq!(string(&repo.node(&kid), "name"), "theirs-name");
    assert_eq!(repo.git(&["diff", "--name-only"]), "");
}
