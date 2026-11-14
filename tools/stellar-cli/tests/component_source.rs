//! 실제 CLI와 Git으로 컴포넌트별 원본의 병합, 재조립, 실패 보존을 확인합니다.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::{
    file_digest, large_document, save_documents, scaffold_project, TempDir,
};

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
    scaffold_project(&repo);
    save_documents(&repo, &[&large_document(4)]);
    let project_before = file_digest(&repo);
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
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let edit = |branch: &str, file: &str, hash: &str, w: &str, h: &str| {
        git(&repo, &["checkout", "-qb", branch, &base]);
        let other = repo.join(if file == ap { bp } else { ap });
        let other_before = fs::read(&other).unwrap();
        let changed = cli(&repo, &home, &["resize", file, hash, w, h], 0);
        assert_eq!(fs::read(&other).unwrap(), other_before);
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
    assert_eq!(git(&fresh, &["ls-files", folder]).lines().count(), 2);
    let reopened_project = file_digest(&fresh);
    for (path, hash) in project_before {
        assert_eq!(reopened_project.get(&path), Some(&hash), "{path}");
    }
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
    let created = file_digest(&folder);
    run(&["create", ".", "Alias", A, BR], 65);
    run(&["create", ".", "Other", B, AR], 65);
    run(&["create", ".", "RootAsComponent", AR, BR], 65);
    run(&["create", ".", "ComponentAsRoot", B, A], 65);
    run(&["create", ".", "Card", A, AR], 73);
    assert_eq!(file_digest(&folder), created);
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    let good = fs::read(&path).unwrap();
    let text = String::from_utf8(good.clone()).unwrap();
    fs::write(&path, format!("# review\n{text}")).unwrap();
    let reviewed = file_digest(&folder);
    run(&["resize", file, hash, "10", "20"], 75);
    assert_eq!(file_digest(&folder), reviewed);
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

#[test]
fn nested_unsupported_records_and_invalid_references_are_never_rewritten() {
    let fixture = TempDir::new("component-source-unsupported");
    let folder = fixture.path.join("components");
    let home = fixture.path.join("home");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(&home).unwrap();
    let run = |args: &[&str], code| cli(&folder, &home, args, code);
    let a = run(&["create", ".", "Card", A, AR], 0);
    run(&["create", ".", "Button", B, BR], 0);
    let file = value(&a, "file");
    let path = folder.join(file);
    let original = fs::read_to_string(&path).unwrap();
    let mut unsupported: Vec<_> = ["properties", "slots", "state_machine", "local_tokens"]
        .into_iter()
        .map(|key| {
            (
                original.replace("component:\n", &format!("component:\n  {key}: []\n")),
                69,
            )
        })
        .collect();
    unsupported.extend([
        (
            original.replace("root_node:\n", "root_node:\n  children: []\n"),
            69,
        ),
        (
            original.replace("  payload:\n", "  payload:\n    future_property: 1\n"),
            69,
        ),
        (
            original.replace(&format!("  root: \"{AR}\""), &format!("  root: \"{BR}\"")),
            65,
        ),
        (
            original.replace("  parent: none", &format!("  parent: {BR}")),
            65,
        ),
    ]);
    for (text, code) in unsupported {
        assert_ne!(text, original);
        fs::write(&path, &text).unwrap();
        let before = file_digest(&folder);
        // 현재 hash로 요청해 오래된 쓰기 보호가 아니라 미지원 레코드 거부를 검사합니다.
        let hash = before
            .get(Path::new(file).file_name().unwrap().to_str().unwrap())
            .unwrap();
        run(&["check", "."], code);
        run(&["resize", file, hash, "10", "20"], code);
        run(
            &[
                "create",
                ".",
                "Another",
                "0195c530-3acd-7bc1-8414-c213f61cb969",
                "0195c530-3acd-7bc1-8414-c213f61cb96a",
            ],
            code,
        );
        assert_eq!(file_digest(&folder), before);
    }
}

const FIRST: &str = "0195c530-3acd-7bc1-8414-c213f61cb980";
const SECOND: &str = "0195c530-3acd-7bc1-8414-c213f61cb970";
const NESTED: &str = "0195c530-3acd-7bc1-8414-c213f61cb960";

#[test]
fn nested_frames_survive_a_component_rename_and_fresh_clone() {
    let fixture = TempDir::new("component-frames-clone");
    let repo = fixture.path.join("repo");
    let dir = "design/components";
    fs::create_dir_all(repo.join(dir)).unwrap();
    let home = fixture.path.join("home");
    fs::create_dir(&home).unwrap();
    let run = |args: &[&str]| cli(&repo, &home, args, 0);
    git(&repo, &["init", "-q", "-b", "main"]);
    let a = run(&["create", dir, "Card", A, AR]);
    let b = run(&["create", dir, "Badge", B, BR]);
    let file = value(&a, "file");
    let other = value(&b, "file");
    let original = fs::read_to_string(repo.join(file)).unwrap();
    let other_bytes = fs::read(repo.join(other)).unwrap();
    assert!(original.contains("schema_version: \"1\"\n"));
    assert!(!original.contains("children:"));
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "root frames"]);

    let mut info = a.clone();
    for (parent, id, name, width, height) in [
        (AR, FIRST, "First", "12.5", "8.25"),
        (AR, SECOND, "Second", "21", "13"),
        (FIRST, NESTED, "Nested", "9", "4"),
    ] {
        info = run(&[
            "add-frame",
            file,
            value(&info, "source_hash"),
            parent,
            id,
            name,
            width,
            height,
        ]);
        assert_eq!(fs::read(repo.join(other)).unwrap(), other_bytes);
    }
    assert_eq!(info.get("node_count").and_then(JsonValue::as_i64), Some(4));
    assert_eq!(git(&repo, &["diff", "--name-only"]), file);
    let yaml = fs::read_to_string(repo.join(file)).unwrap();
    assert!(yaml.contains("schema_version: \"2\"\n"));
    let root_lines = original.split_once("root_node:\n").unwrap().1;
    assert!(yaml.contains(&format!("root_node:\n{root_lines}")));
    let child_records: Vec<_> = yaml
        .split_once("children:\n")
        .unwrap()
        .1
        .split("  -\n")
        .skip(1)
        .collect();
    assert_eq!(child_records.len(), 3);
    for (record, (id, parent, order_key)) in child_records.iter().zip([
        (NESTED, FIRST, "\"80\""),
        (SECOND, AR, "c0"),
        (FIRST, AR, "\"80\""),
    ]) {
        assert!(
            record.starts_with(&format!(
                "    id: \"{id}\"\n    parent: \"{parent}\"\n    order_key: {order_key}\n"
            )),
            "{record}"
        );
    }
    run(&["resize", file, value(&info, "source_hash"), "248.5", "44"]);
    let resized_yaml = fs::read_to_string(repo.join(file)).unwrap();
    assert_eq!(
        resized_yaml,
        yaml.replacen("size: \"0 0\"", "size: \"248.5 44\"", 1)
    );
    assert_ne!(resized_yaml, yaml);
    assert_eq!(fs::read(repo.join(other)).unwrap(), other_bytes);
    git(&repo, &["add", "--", file]);
    git(&repo, &["commit", "-qm", "nested frames"]);
    let renamed = file.replace("Card-", "CardLarge-");
    git(&repo, &["mv", "--", file, &renamed]);
    let renamed_yaml = resized_yaml.replace("  name: Card\n", "  name: CardLarge\n");
    assert_ne!(renamed_yaml, resized_yaml);
    fs::write(repo.join(&renamed), &renamed_yaml).unwrap();
    git(&repo, &["add", "--", &renamed]);
    git(&repo, &["commit", "-qm", "rename card"]);
    let change = git(&repo, &["diff", "--name-status", "-M", "HEAD^", "HEAD"]);
    assert!(
        change.starts_with('R') && change.ends_with(&format!("\t{file}\t{renamed}")),
        "{change}"
    );
    let expected = run(&["check", dir]);
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
    assert_eq!(cli(&fresh, &home, &["check", dir], 0), expected);
    let components = expected.get("components").unwrap().as_array().unwrap();
    assert_eq!(value(&components[0], "id"), A);
    assert_eq!(value(&components[0], "root"), AR);
    assert_eq!(value(&components[0], "name"), "CardLarge");
    assert_eq!(
        components[0].get("node_count").and_then(JsonValue::as_i64),
        Some(4)
    );
    assert_eq!(components[1], b);
    assert_eq!(fs::read(fresh.join(other)).unwrap(), other_bytes);
    assert_eq!(
        fs::read_to_string(fresh.join(&renamed)).unwrap(),
        renamed_yaml
    );
    assert!(!fresh.join(".stellar/cache").exists());
    assert!(git(&fresh, &["status", "--porcelain"]).is_empty());
}

#[test]
fn nested_frame_errors_preserve_every_component_file() {
    let fixture = TempDir::new("component-frames-preserve");
    let dir = fixture.path.join("components");
    let home = fixture.path.join("home");
    fs::create_dir(&dir).unwrap();
    fs::create_dir(&home).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let a = run(&["create", ".", "Card", A, AR], 0);
    let b = run(&["create", ".", "Badge", B, BR], 0);
    let ap = value(&a, "file");
    let bp = value(&b, "file");
    let hash = value(&a, "source_hash");
    let changed = run(&["add-frame", ap, hash, AR, FIRST, "First", "20", "30"], 0);
    let current = value(&changed, "source_hash");
    let before = file_digest(&dir);
    run(
        &["add-frame", ap, hash, AR, SECOND, "Stale", "10", "20"],
        75,
    );
    for (file, hash, parent, id) in [
        (ap, current, FIRST, FIRST),
        (ap, current, NESTED, SECOND),
        (bp, value(&b, "source_hash"), BR, FIRST),
    ] {
        run(
            &["add-frame", file, hash, parent, id, "Other", "10", "10"],
            65,
        );
    }
    assert_eq!(file_digest(&dir), before);
    let path = dir.join(ap);
    let valid = fs::read_to_string(&path).unwrap();
    let (head, children) = valid.split_once("children:\n").unwrap();
    assert!(children.contains("kind: frame"));
    fs::write(
        &path,
        format!(
            "{head}children:\n{}",
            children.replacen("kind: frame", "kind: shape", 1)
        ),
    )
    .unwrap();
    let malformed = file_digest(&dir);
    let latest = malformed
        .get(path.file_name().unwrap().to_str().unwrap())
        .unwrap();
    run(&["check", "."], 69);
    run(
        &["add-frame", ap, latest, AR, SECOND, "Other", "0", "0"],
        69,
    );
    assert_eq!(file_digest(&dir), malformed);
}
