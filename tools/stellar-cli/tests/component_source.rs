//! 실제 CLI와 Git으로 컴포넌트별 원본의 병합, 재조립, 실패 보존을 확인합니다.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::TempDir;

const A: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";
const AR: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";
const B: &str = "0195c530-3acd-7bc1-8414-c213f61cb967";
const BR: &str = "0195c530-3acd-7bc1-8414-c213f61cb968";

fn cli(work: &Path, home: &Path, args: &[&str], exit: i32) -> JsonValue {
    let out = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .arg("component-source")
        .args(args)
        .current_dir(work)
        .env_clear()
        .env("HOME", home)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = parse(
        std::str::from_utf8(&out.stdout).unwrap().trim(),
        JsonLimits::default(),
    )
    .unwrap();
    envelope.get("result").unwrap().clone()
}
fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn value<'a>(v: &'a JsonValue, key: &str) -> &'a str {
    v.get(key).unwrap().as_str().unwrap()
}

#[test]
fn independent_component_files_merge_and_open_without_cache() {
    let fixture = TempDir::new("component-source-merge");
    let repo = fixture.path.join("repo");
    let folder = "design/components";
    fs::create_dir_all(repo.join(folder)).unwrap();
    let home = fixture.path.join("home");
    fs::create_dir(&home).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let a = cli(&repo, &home, &["create", folder, "버튼", A, AR], 0);
    let b = cli(&repo, &home, &["create", folder, "버튼", B, BR], 0);
    let ap = value(&a, "file");
    let bp = value(&b, "file");
    assert_eq!(ap, "design/components/버튼-8c5619572809.stellar.yaml");
    assert_eq!(bp, "design/components/버튼-bc6dfb3d3aea.stellar.yaml");
    git(&repo, &["add", folder]);
    git(&repo, &["commit", "-qm", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let edit = |branch: &str, file: &str, hash: &str, w: &str, h: &str| {
        git(&repo, &["checkout", "-qb", branch, &base]);
        let changed = cli(&repo, &home, &["resize", file, hash, w, h], 0);
        assert_ne!(hash, value(&changed, "source_hash"));
        assert_eq!(
            git(
                &repo,
                &["-c", "core.quotePath=false", "diff", "--name-only"]
            ),
            file
        );
        git(&repo, &["add", "--", file]);
        git(&repo, &["commit", "-qm", branch]);
        changed
    };
    let left = edit("edit-a", ap, value(&a, "source_hash"), "240.5", "64");
    let right = edit("edit-b", bp, value(&b, "source_hash"), "32", "55");
    git(&repo, &["merge", "--no-ff", "--no-edit", "edit-a"]);
    let fresh = fixture.path.join("fresh");
    git(
        &repo,
        &[
            "clone",
            "--quiet",
            "--no-local",
            ".",
            fresh.to_str().unwrap(),
        ],
    );
    assert!(!fresh.join(".stellar/cache").exists());
    let check = cli(&fresh, &home, &["check", folder], 0);
    let entries = check.get("components").unwrap().as_array().unwrap();
    assert_eq!(entries, &[left, right]);
    assert_eq!(value(&entries[0], "root"), AR);
    assert_eq!(value(&entries[1], "root"), BR);
    assert_eq!(value(&entries[0], "width"), "240.5");
    assert_eq!(value(&entries[1], "height"), "55");
    assert!(git(&fresh, &["status", "--porcelain"]).is_empty());
    assert_eq!(git(&fresh, &["ls-files"]).lines().count(), 2);
}

#[test]
fn duplicate_ids_stale_writes_and_unsupported_sources_preserve_files() {
    let fixture = TempDir::new("component-source-errors");
    let folder = fixture.path.join("components");
    fs::create_dir(&folder).unwrap();
    let home = fixture.path.join("home");
    fs::create_dir(&home).unwrap();
    let run = |args: &[&str], code| cli(&folder, &home, args, code);
    let a = run(&["create", ".", "Card", A, AR], 0);
    let filename = PathBuf::from(value(&a, "file"));
    let path = folder.join(&filename);
    let file = filename.to_str().unwrap();
    let hash = value(&a, "source_hash");
    run(&["create", ".", "Alias", A, BR], 65);
    run(&["create", ".", "Other", B, AR], 65);
    run(&["create", ".", "Card", A, AR], 73);
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    let good = fs::read(&path).unwrap();
    let text = String::from_utf8(good.clone()).unwrap();
    fs::write(&path, format!("# review\n{text}")).unwrap();
    run(&["resize", file, hash, "10", "20"], 75);
    let unchanged = fs::read(&path).unwrap();
    let info = run(&["check", "."], 0);
    let current = value(
        &info.get("components").unwrap().as_array().unwrap()[0],
        "source_hash",
    );
    let zero = value(&a, "width");
    let height = value(&a, "height");
    run(&["resize", file, current, zero, height], 0);
    assert_eq!(fs::read(&path).unwrap(), unchanged);
    fs::write(&path, format!("{text}slots: []\n")).unwrap();
    let unsupported = fs::read(&path).unwrap();
    run(&["check", "."], 69);
    run(&["resize", file, current, "10", "20"], 69);
    assert_eq!(fs::read(&path).unwrap(), unsupported);
    fs::write(&path, &good).unwrap();
    let wrong = folder.join("Renamed.stellar.yaml");
    fs::rename(&path, &wrong).unwrap();
    run(&["check", "."], 65);
    assert_eq!(fs::read(&wrong).unwrap(), good);
    let renamed_text = text.replace("  name: Card\n", "  name: Card Wide\n");
    assert_ne!(renamed_text, text);
    let new_name = path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .replacen("Card-", "Card Wide-", 1);
    fs::write(&wrong, renamed_text).unwrap();
    fs::rename(&wrong, folder.join(new_name)).unwrap();
    let opened = run(&["check", "."], 0);
    let component = &opened.get("components").unwrap().as_array().unwrap()[0];
    assert_eq!(value(component, "name"), "Card Wide");
    assert_eq!(value(component, "id"), A);
    assert_eq!(value(component, "root"), AR);
}
