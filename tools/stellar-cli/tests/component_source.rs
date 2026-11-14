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
    run(work, home, "component-source", args, exit)
}
fn run(work: &Path, home: &Path, command: &str, args: &[&str], exit: i32) -> JsonValue {
    let out = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .arg(command)
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
    let before_query = file_digest(&fresh);
    let summary = run(&fresh, &home, "query", &[".", "components"], 0);
    assert_eq!(summary.get("chunk_count").unwrap().as_i64(), Some(0));
    let queried = summary.get("entries").unwrap().as_array().unwrap();
    assert_eq!(queried.len(), entries.len());
    for (result, file) in queried.iter().zip(entries) {
        for field in [
            "id",
            "root",
            "name",
            "width",
            "height",
            "node_count",
            "source_hash",
        ] {
            assert_eq!(result.get(field), file.get(field), "{field}");
        }
        assert!(result.get("file").is_none());
    }
    assert_eq!(file_digest(&fresh), before_query);
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

#[test]
fn semantic_diff_compares_project_components_across_git_worktrees() {
    let fixture = TempDir::new("component-worktree-diff");
    let repo = fixture.path.join("repo");
    let base = fixture.path.join("base");
    let home = fixture.path.join("home");
    fs::create_dir(&home).unwrap();
    scaffold_project(&repo);
    save_documents(&repo, &[&large_document(4)]);
    git(&repo, &["init", "-q", "-b", "main"]);
    let folder = "design/components";
    fs::create_dir_all(repo.join(folder)).unwrap();
    let a = cli(&repo, &home, &["create", folder, "Card", A, AR], 0);
    let b = cli(&repo, &home, &["create", folder, "Badge", B, BR], 0);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            base.to_str().unwrap(),
            "HEAD",
        ],
    );
    let path = value(&a, "file");
    let resized = cli(
        &repo,
        &home,
        &["resize", path, value(&a, "source_hash"), "160", "44"],
        0,
    );
    cli(
        &repo,
        &home,
        &[
            "add-frame",
            path,
            value(&resized, "source_hash"),
            AR,
            FIRST,
            "Label",
            "40",
            "14",
        ],
        0,
    );
    let new_path = path.replace("Card-", "Tile-");
    git(&repo, &["mv", path, &new_path]);
    let yaml = fs::read_to_string(repo.join(&new_path)).unwrap();
    fs::write(
        repo.join(&new_path),
        yaml.replace("  name: Card\n", "  name: Tile\n"),
    )
    .unwrap();
    fs::remove_file(repo.join(value(&b, "file"))).unwrap();

    let args = [base.to_str().unwrap(), "."];
    let observe = || run(&repo, &home, "semantic-diff", &args, 0);
    let staged = git(&repo, &["write-tree"]);
    let before = file_digest(&repo);
    let result = observe();
    assert_eq!(result.get("base_revision"), result.get("target_revision"));
    let changes = result
        .get("changes")
        .unwrap()
        .get("entries")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(changes.len(), 5, "{changes:?}");
    for (id, kind, owner) in [
        (A, "set-property", A),
        (AR, "set-property", A),
        (FIRST, "add-object", A),
        (B, "remove-object", B),
        (BR, "remove-object", B),
    ] {
        let change = changes.iter().find(|e| value(e, "object") == id).unwrap();
        assert_eq!(value(change, "kind"), kind);
        assert_eq!(change.get("document"), Some(&JsonValue::Null));
        assert_eq!(value(change, "component"), owner);
    }
    assert_eq!(file_digest(&repo), before);
    assert_eq!(git(&repo, &["write-tree"]), staged);
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(repo.join(&new_path))
        .unwrap();
    writeln!(file, "# formatting only").unwrap();
    assert_eq!(observe(), result);

    let summary = run(&repo, &home, "diff-summary", &args, 0);
    assert_eq!(summary.get("total").and_then(JsonValue::as_i64), Some(5));
    let entries = summary.get("changes").unwrap().as_array().unwrap();
    assert_eq!(entries[0].get("component"), changes[0].get("component"));

    // 같은 Frame에 서로 다른 크기를 준 두 작업 트리는 정확히 그 속성에서 충돌합니다.
    let theirs = fixture.path.join("theirs");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            theirs.to_str().unwrap(),
            "HEAD",
        ],
    );
    cli(
        &theirs,
        &home,
        &["resize", path, value(&a, "source_hash"), "200", "60"],
        0,
    );
    let merge = run(
        &repo,
        &home,
        "semantic-merge",
        &[args[0], ".", theirs.to_str().unwrap()],
        1,
    );
    let conflicts = merge.get("conflicts").unwrap().as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(value(&conflicts[0], "object"), AR);
    assert_eq!(value(&conflicts[0], "component"), A);
    assert_eq!(value(&conflicts[0], "kind"), "property-value");
    assert_eq!(observe(), result);

    // 손상된 텍스트 파일을 변경 없음으로 취급하거나 기록하지 않습니다.
    fs::write(repo.join(&new_path), "invalid").unwrap();
    let invalid = file_digest(&repo);
    run(&repo, &home, "semantic-diff", &args, 65);
    assert_eq!(file_digest(&repo), invalid);
}

#[test]
fn rename_frame_persists_names_and_preserves_source_on_rejected_edits() {
    let fixture = TempDir::new("component-frame-rename");
    let home = fixture.path.join("home");
    let folder = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&folder).unwrap();
    let folder = folder.to_str().unwrap();
    let created = cli(&fixture.path, &home, &["create", folder, "Card", A, AR], 0);
    let file = value(&created, "file");
    let added = cli(
        &fixture.path,
        &home,
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            BR,
            "Child",
            "32",
            "16",
        ],
        0,
    );
    let name = "새 이름 / : # \"버튼\"\n다음 줄";
    let renamed = cli(
        &fixture.path,
        &home,
        &["rename-frame", file, value(&added, "source_hash"), BR, name],
        0,
    );
    let hash = value(&renamed, "source_hash");
    assert_ne!(hash, value(&added, "source_hash"));
    let text = fs::read_to_string(file).unwrap();
    assert!(text.contains("새 이름 / : #"));
    let checked = stellar_ops::component_source::check(Path::new(folder)).unwrap();
    assert_eq!(checked[0].name, "Card");
    assert_eq!(checked[0].node_count, 2);
    let before = fs::read(file).unwrap();
    let noop = cli(
        &fixture.path,
        &home,
        &["rename-frame", file, hash, BR, name],
        0,
    );
    assert_eq!(value(&noop, "source_hash"), hash);
    for (expected, node, exit) in [(value(&added, "source_hash"), BR, 75), (hash, B, 65)] {
        cli(
            &fixture.path,
            &home,
            &["rename-frame", file, expected, node, "Rejected"],
            exit,
        );
        assert_eq!(fs::read(file).unwrap(), before);
    }
    cli(
        &fixture.path,
        &home,
        &["rename-frame", file, hash, AR, ""],
        0,
    );
    assert_ne!(fs::read(file).unwrap(), before);
}

#[test]
fn resize_frame_changes_only_nested_geometry_and_rejects_invalid_edits() {
    let fixture = TempDir::new("component-frame-resize");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    let parent = run(
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            FIRST,
            "Parent",
            "20",
            "30",
        ],
        0,
    );
    let child = run(
        &[
            "add-frame",
            file,
            value(&parent, "source_hash"),
            FIRST,
            NESTED,
            "Child",
            "12",
            "8",
        ],
        0,
    );
    let path = dir.join(file);
    let before = fs::read_to_string(&path).unwrap();
    let changed = run(
        &[
            "resize-frame",
            file,
            value(&child, "source_hash"),
            NESTED,
            "19.25",
            "0",
        ],
        0,
    );
    let hash = value(&changed, "source_hash");
    assert_ne!(hash, value(&child, "source_hash"));
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, before.replace("size: \"12 8\"", "size: \"19.25 0\""));
    let reopened = stellar_ops::component_source::check(&dir).unwrap();
    assert_eq!(reopened[0].hash.to_string(), hash);
    let noop = run(&["resize-frame", file, hash, NESTED, "19.25", "0"], 0);
    assert_eq!(value(&noop, "source_hash"), hash);
    for (expected, node, width, height, code) in [
        (value(&child, "source_hash"), NESTED, "10", "10", 75),
        (hash, B, "10", "10", 65),
        (hash, NESTED, "NaN", "10", 65),
        (hash, NESTED, "10", "-1", 65),
    ] {
        run(&["resize-frame", file, expected, node, width, height], code);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
    let root = run(&["resize-frame", file, hash, AR, "100", "50"], 0);
    assert_eq!(value(&root, "width"), "100");
    assert_eq!(value(&root, "height"), "50");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        after.replacen("size: \"0 0\"", "size: \"100 50\"", 1)
    );
}

#[test]
fn remove_frame_deletes_only_subtree_and_reopens_root_only_source() {
    let fixture = TempDir::new("component-frame-remove");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    run(&["create", ".", "Badge", B, BR], 0);
    let mut info = created.clone();
    for (parent, id) in [(AR, FIRST), (FIRST, NESTED), (AR, SECOND)] {
        info = run(
            &[
                "add-frame",
                file,
                value(&info, "source_hash"),
                parent,
                id,
                "Child",
                "12",
                "8",
            ],
            0,
        );
    }
    let before = file_digest(&dir);
    let hash = value(&info, "source_hash");
    for (expected, id, code) in [
        (value(&created, "source_hash"), FIRST, 75),
        (hash, AR, 65),
        (hash, BR, 65),
        (hash, A, 65),
    ] {
        run(&["remove-frame", file, expected, id], code);
        assert_eq!(file_digest(&dir), before);
    }
    let original = fs::read_to_string(dir.join(file)).unwrap();
    let (root, children) = original.split_once("children:\n").unwrap();
    let sibling = children
        .split("  -\n")
        .find(|record| record.contains(SECOND))
        .unwrap();
    let removed = run(&["remove-frame", file, hash, FIRST], 0);
    assert_eq!(removed.get("node_count").unwrap().as_i64(), Some(2));
    let text = fs::read_to_string(dir.join(file)).unwrap();
    assert!(!text.contains(FIRST));
    assert!(!text.contains(NESTED));
    assert_eq!(text, format!("{root}children:\n  -\n{sibling}"));
    let after = file_digest(&dir);
    for (path, bytes) in &before {
        if Path::new(file).file_name().unwrap().to_str().unwrap() != path {
            assert_eq!(after.get(path), Some(bytes));
        }
    }
    run(
        &["remove-frame", file, value(&removed, "source_hash"), FIRST],
        65,
    );
    assert_eq!(file_digest(&dir), after);
    let empty = run(
        &["remove-frame", file, value(&removed, "source_hash"), SECOND],
        0,
    );
    assert_eq!(empty.get("node_count").unwrap().as_i64(), Some(1));
    assert_eq!(value(&empty, "source_hash"), value(&created, "source_hash"));
    let checked = run(&["check", "."], 0);
    assert_eq!(
        checked.get("components").unwrap().as_array().unwrap().len(),
        2
    );
}

#[test]
fn set_visible_preserves_children_and_rejects_invalid_edits() {
    let fixture = TempDir::new("component-frame-visible");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    let added = run(
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            FIRST,
            "Child",
            "20",
            "30",
        ],
        0,
    );
    let path = dir.join(file);
    let before = fs::read_to_string(&path).unwrap();
    let hidden = run(
        &[
            "set-visible",
            file,
            value(&added, "source_hash"),
            AR,
            "false",
        ],
        0,
    );
    let hash = value(&hidden, "source_hash");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before.replacen("visible: \"true\"", "visible: \"false\"", 1)
    );
    let checked = stellar_ops::component_source::check(&dir).unwrap();
    assert_eq!(checked[0].hash.to_string(), hash);
    assert_eq!(checked[0].node_count, 2);
    // A no-op must retain comments as well as the semantic value.
    fs::write(&path, format!("# retained comment\n{after}")).unwrap();
    let checked = stellar_ops::component_source::check(&dir).unwrap();
    let current = checked[0].hash.to_string();
    let unchanged = fs::read(&path).unwrap();
    let noop = run(&["set-visible", file, &current, AR, "false"], 0);
    assert_eq!(value(&noop, "source_hash"), current);
    for (expected, node, visible, code) in [
        (hash, AR, "true", 75),
        (&current, B, "false", 65),
        (&current, FIRST, "False", 65),
        (&current, FIRST, "1", 65),
    ] {
        run(&["set-visible", file, expected, node, visible], code);
        assert_eq!(fs::read(&path).unwrap(), unchanged);
    }
    let child = run(&["set-visible", file, &current, FIRST, "false"], 0);
    let child_text = fs::read_to_string(&path).unwrap();
    assert_eq!(child_text.matches("visible: \"false\"").count(), 2);
    let shown = run(
        &[
            "set-visible",
            file,
            value(&child, "source_hash"),
            AR,
            "true",
        ],
        0,
    );
    assert_ne!(value(&shown, "source_hash"), value(&child, "source_hash"));
    assert_eq!(
        fs::read_to_string(&path)
            .unwrap()
            .matches("visible: \"false\"")
            .count(),
        1
    );
}

#[test]
fn set_locked_preserves_children_and_rejects_invalid_edits() {
    let fixture = TempDir::new("component-frame-locked");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    let added = run(
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            FIRST,
            "Child",
            "20",
            "30",
        ],
        0,
    );
    let path = dir.join(file);
    let before = fs::read_to_string(&path).unwrap();
    let locked = run(
        &["set-locked", file, value(&added, "source_hash"), AR, "true"],
        0,
    );
    let hash = value(&locked, "source_hash");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before.replacen("locked: \"false\"", "locked: \"true\"", 1)
    );
    let checked = stellar_ops::component_source::check(&dir).unwrap();
    assert_eq!(checked[0].hash.to_string(), hash);
    assert_eq!(checked[0].node_count, 2);
    // A no-op must retain comments as well as the semantic value.
    fs::write(&path, format!("# retained comment\n{after}")).unwrap();
    let checked = stellar_ops::component_source::check(&dir).unwrap();
    let current = checked[0].hash.to_string();
    let unchanged = fs::read(&path).unwrap();
    let noop = run(&["set-locked", file, &current, AR, "true"], 0);
    assert_eq!(value(&noop, "source_hash"), current);
    for (expected, node, locked, code) in [
        (hash, AR, "false", 75),
        (&current, B, "true", 65),
        (&current, FIRST, "False", 65),
        (&current, FIRST, "1", 65),
    ] {
        run(&["set-locked", file, expected, node, locked], code);
        assert_eq!(fs::read(&path).unwrap(), unchanged);
    }
    let child = run(&["set-locked", file, &current, FIRST, "true"], 0);
    let child_text = fs::read_to_string(&path).unwrap();
    assert_eq!(child_text.matches("locked: \"true\"").count(), 2);
    let unlocked = run(
        &[
            "set-locked",
            file,
            value(&child, "source_hash"),
            AR,
            "false",
        ],
        0,
    );
    assert_ne!(
        value(&unlocked, "source_hash"),
        value(&child, "source_hash")
    );
    assert_eq!(
        fs::read_to_string(&path)
            .unwrap()
            .matches("locked: \"true\"")
            .count(),
        1
    );
}

#[test]
fn set_position_preserves_affine_children_and_rejects_invalid_edits() {
    let fixture = TempDir::new("component-position");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    let parent = run(
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            FIRST,
            "Parent",
            "20",
            "30",
        ],
        0,
    );
    run(
        &[
            "add-frame",
            file,
            value(&parent, "source_hash"),
            FIRST,
            NESTED,
            "Child",
            "12",
            "8",
        ],
        0,
    );
    let path = dir.join(file);
    let original = fs::read_to_string(&path).unwrap();
    assert!(original.contains("transform: \"1 0 0 1 0 0\""));
    let before = original.replace("transform: \"1 0 0 1 0 0\"", "transform: \"0 2 -3 0 4 5\"");
    fs::write(&path, &before).unwrap();
    let hash = stellar_ops::component_source::check(&dir).unwrap()[0]
        .hash
        .to_string();
    let changed = run(&["set-position", file, &hash, FIRST, "-19.25", "0"], 0);
    let next = value(&changed, "source_hash");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after.matches("transform: \"0 2 -3 0 -19.25 0\"").count(), 1);
    assert_eq!(
        after.replace(
            "transform: \"0 2 -3 0 -19.25 0\"",
            "transform: \"0 2 -3 0 4 5\""
        ),
        before
    );
    for (expected, node, x, y, code) in [
        (&hash[..], FIRST, "1", "2", 75),
        (next, B, "1", "2", 65),
        (next, FIRST, "NaN", "2", 65),
        (next, FIRST, "1", "inf", 65),
        (next, FIRST, "bad", "2", 65),
    ] {
        run(&["set-position", file, expected, node, x, y], code);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
    // A no-op must preserve comments as well as geometry.
    fs::write(&path, format!("# preserved\n{after}")).unwrap();
    let commented = fs::read(&path).unwrap();
    let hash = stellar_ops::component_source::check(&dir).unwrap()[0]
        .hash
        .to_string();
    let noop = run(&["set-position", file, &hash, FIRST, "-19.25", "0"], 0);
    assert_eq!(value(&noop, "source_hash"), hash);
    assert_eq!(fs::read(&path).unwrap(), commented);
    run(&["set-position", file, &hash, AR, "10", "-20"], 0);
    let reopened = fs::read_to_string(&path).unwrap();
    assert_eq!(
        reopened,
        after.replacen(
            "transform: \"0 2 -3 0 4 5\"",
            "transform: \"0 2 -3 0 10 -20\"",
            1
        )
    );
    stellar_ops::component_source::check(&dir).unwrap();
}

#[test]
fn geometry_edits_respect_self_and_ancestor_locks_without_writing() {
    let fixture = TempDir::new("component-geometry-lock");
    let home = fixture.path.join("home");
    let dir = fixture.path.join("components");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&dir).unwrap();
    let run = |args: &[&str], code| cli(&dir, &home, args, code);
    let created = run(&["create", ".", "Card", A, AR], 0);
    let file = value(&created, "file");
    let parent = run(
        &[
            "add-frame",
            file,
            value(&created, "source_hash"),
            AR,
            FIRST,
            "Parent",
            "20",
            "30",
        ],
        0,
    );
    let child = run(
        &[
            "add-frame",
            file,
            value(&parent, "source_hash"),
            FIRST,
            NESTED,
            "Child",
            "12",
            "8",
        ],
        0,
    );
    let locked_child = run(
        &[
            "set-locked",
            file,
            value(&child, "source_hash"),
            NESTED,
            "true",
        ],
        0,
    );
    let locked_root = run(
        &[
            "set-locked",
            file,
            value(&locked_child, "source_hash"),
            AR,
            "true",
        ],
        0,
    );
    let path = dir.join(file);
    // Preserve comments and exact bytes on both no-op and changing requests.
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("# locked geometry\n{original}")).unwrap();
    let before = fs::read(&path).unwrap();
    let hash = stellar_ops::component_source::check(&dir).unwrap()[0]
        .hash
        .to_string();
    for node in [AR, FIRST, NESTED] {
        for command in ["set-position", "resize-frame"] {
            for (x, y) in [("0", "0"), ("42", "24")] {
                run(&[command, file, &hash, node, x, y], 65);
                assert_eq!(fs::read(&path).unwrap(), before);
            }
        }
    }
    run(&["resize", file, &hash, "42", "24"], 65);
    run(
        &[
            "set-position",
            file,
            value(&locked_root, "source_hash"),
            FIRST,
            "1",
            "2",
        ],
        75,
    );
    assert_eq!(fs::read(&path).unwrap(), before);

    let unlocked = run(&["set-locked", file, &hash, AR, "false"], 0);
    let hash = value(&unlocked, "source_hash");
    let before = fs::read(&path).unwrap();
    // Unlocking an ancestor leaves the child's own lock intact.
    run(&["set-position", file, hash, NESTED, "1", "2"], 65);
    run(&["resize-frame", file, hash, NESTED, "42", "24"], 65);
    assert_eq!(fs::read(&path).unwrap(), before);
    let moved = run(&["set-position", file, hash, FIRST, "1", "2"], 0);
    let resized = run(
        &["resize", file, value(&moved, "source_hash"), "42", "24"],
        0,
    );
    let unlocked = run(
        &[
            "set-locked",
            file,
            value(&resized, "source_hash"),
            NESTED,
            "false",
        ],
        0,
    );
    let moved = run(
        &[
            "set-position",
            file,
            value(&unlocked, "source_hash"),
            NESTED,
            "1",
            "2",
        ],
        0,
    );
    run(
        &[
            "resize-frame",
            file,
            value(&moved, "source_hash"),
            NESTED,
            "42",
            "24",
        ],
        0,
    );
    stellar_ops::component_source::check(&dir).unwrap();
}
