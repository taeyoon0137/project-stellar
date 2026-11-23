//! 시스템 Git과 `stellar merge-driver`로 컴포넌트 원본을 의미 단위로 병합합니다(XXIV.2, ADR-0081).
//!
//! 두 branch가 같은 컴포넌트에 서로 다른 자식 Frame을 추가하면 Git 텍스트 merge는 같은 자리 삽입으로 충돌하지만,
//! driver를 등록한 저장소에서는 `git merge`가 의미 단위로 끝납니다.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

const A: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";
const AR: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";
const BADGE: &str = "0195c530-3acd-7bc1-8414-c213f61cb970";
const TAG: &str = "0195c530-3acd-7bc1-8414-c213f61cb971";
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
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse(
        std::str::from_utf8(&output.stdout).unwrap().trim(),
        JsonLimits::default(),
    )
    .unwrap()
}

fn git_output(repo: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("HOME", home)
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
        .current_dir(repo)
        .output()
        .unwrap()
}

fn git(repo: &Path, home: &Path, args: &[&str]) -> String {
    let out = git_output(repo, home, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn string<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}

/// Card 컴포넌트(90x60)를 담은 Git 저장소. (fixture, 저장소, HOME, 원본 상대 경로)
struct Repo {
    _fixture: TempDir,
    root: PathBuf,
    home: PathBuf,
    file: String,
}

impl Repo {
    fn new(label: &str) -> Repo {
        let fixture = TempDir::new(label);
        let root = fixture.path.join("project");
        let home = fixture.path.join("home");
        fs::create_dir(&home).unwrap();
        fs::write(home.join(".gitconfig"), "").unwrap();
        scaffold_project(&root);
        save_documents(&root, &[&large_document(4)]);
        fs::create_dir_all(root.join("design/components")).unwrap();
        let created = result(
            &stellar(
                &root,
                &home,
                &[
                    "component-source",
                    "create",
                    "design/components",
                    "Card",
                    A,
                    AR,
                ],
            ),
            0,
        );
        let file = string(created.get("result").unwrap(), "file").to_owned();
        let repo = Repo {
            _fixture: fixture,
            root,
            home,
            file,
        };
        repo.cli(&["resize", &repo.file.clone(), &repo.hash(), "90", "60"]);
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "Card"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        git(&self.root, &self.home, args)
    }

    fn git_output(&self, args: &[&str]) -> Output {
        git_output(&self.root, &self.home, args)
    }

    fn hash(&self) -> String {
        let check = result(
            &stellar(
                &self.root,
                &self.home,
                &["component-source", "check", "design/components"],
            ),
            0,
        );
        let entries = check.get("result").unwrap().get("components").unwrap();
        string(&entries.as_array().unwrap()[0], "source_hash").to_owned()
    }

    fn cli(&self, args: &[&str]) -> JsonValue {
        let mut full = vec!["component-source"];
        full.extend_from_slice(args);
        result(&stellar(&self.root, &self.home, &full), 0)
    }

    fn read(&self) -> Vec<u8> {
        fs::read(self.root.join(&self.file)).unwrap()
    }

    /// branch를 main에서 만들고 편집을 commit합니다.
    fn branch(&self, name: &str, edit: impl FnOnce(&Repo)) {
        self.git(&["switch", "-q", "main"]);
        self.git(&["switch", "-qc", name]);
        edit(self);
        self.git(&["add", "--", &self.file]);
        self.git(&["commit", "-qm", name]);
    }

    fn add_frame(&self, id: &str, name: &str) {
        let file = self.file.clone();
        self.cli(&["add-frame", &file, &self.hash(), AR, id, name, "10", "10"]);
    }

    #[allow(dead_code)]
    fn resize(&self, width: &str, height: &str) {
        let file = self.file.clone();
        self.cli(&["resize", &file, &self.hash(), width, height]);
    }

    /// driver를 저장소 로컬 config와 `.gitattributes`에 직접 등록합니다(install 명령 없이).
    fn register_manually(&self) {
        self.git(&[
            "config",
            "--local",
            "merge.stellar.driver",
            &format!("'{STELLAR}' merge-driver %O %A %B %P"),
        ]);
        let mut attributes = fs::read_to_string(self.root.join(".gitattributes")).unwrap();
        attributes.push_str("design/components/*.stellar.yaml merge=stellar\n");
        fs::write(self.root.join(".gitattributes"), attributes).unwrap();
        self.git(&["add", ".gitattributes"]);
        self.git(&["commit", "-qm", "driver"]);
    }
}

#[test]
fn independent_child_insertions_merge_through_the_driver() {
    let repo = Repo::new("driver-clean");
    repo.register_manually();
    repo.branch("badge", |r| r.add_frame(BADGE, "Badge"));
    repo.branch("tag", |r| r.add_frame(TAG, "Tag"));

    // driver 없이는 같은 자리 삽입이라 Git 텍스트 충돌입니다(대조군).
    repo.git(&["switch", "-q", "badge"]);
    repo.git(&["switch", "-qc", "plain"]);
    let plain = repo.git_output(&[
        "-c",
        "merge.stellar.driver=false",
        "merge",
        "--no-edit",
        "tag",
    ]);
    assert_eq!(plain.status.code(), Some(1));
    repo.git(&["merge", "--abort"]);

    repo.git(&["switch", "-q", "badge"]);
    let merged = repo.git_output(&["merge", "--no-edit", "tag"]);
    assert!(
        merged.status.success(),
        "{}{}",
        String::from_utf8_lossy(&merged.stdout),
        String::from_utf8_lossy(&merged.stderr)
    );
    assert!(repo.git(&["status", "--porcelain"]).is_empty());
    let text = String::from_utf8(repo.read()).unwrap();
    assert!(text.contains("Badge") && text.contains("Tag"), "{text}");
    // 결과는 codec이 그대로 읽는 원본이고 두 자식이 모두 정의 tree에 있습니다.
    let detail = result(
        &stellar(&repo.root, &repo.home, &["query", ".", "component", A]),
        0,
    );
    let nodes = detail
        .get("result")
        .unwrap()
        .get("nodes")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(nodes.len(), 3);
}

#[test]
fn driver_rejects_wrong_arguments_and_unsupported_files() {
    let repo = Repo::new("driver-args");
    let output = stellar(&repo.root, &repo.home, &["merge-driver", "a", "b"]);
    assert_eq!(output.status.code(), Some(64));
    let readme = repo.root.join("README.tmp");
    fs::write(&readme, "x").unwrap();
    let path = readme.to_str().unwrap();
    let output = stellar(
        &repo.root,
        &repo.home,
        &["merge-driver", path, path, path, "README.md"],
    );
    let envelope = result(&output, 69);
    assert_eq!(
        string(envelope.get("error").unwrap(), "code"),
        "unsupported-file"
    );
    assert_eq!(fs::read(&readme).unwrap(), b"x", "%A 미변경");
}
