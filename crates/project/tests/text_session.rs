//! V.10.6 텍스트 원본 session: 저장(journal)·재열기·Git diff, clean·dirty 외부 변경, 깨진 원본 진단(UX.4.4),
//! 열 때 journal 복구.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use common::golden::golden_document;
use common::TempDir;
use stellar_commands::command::Command as EditCommand;
use stellar_commands::id::{CommandId, CommandOrigin, WindowSessionId};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::document_io::source_v4::project::{encode, write_new, TextProject};
use stellar_project::external_change::source::{
    TextExternal, TextSave, TextSessionError, TextSourceSession, DESIGN_DIR,
};

fn window(n: u64) -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::new_v7(80_000 + n, 0x0a0a, n))
}

fn command(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(81_000 + n, 0x0b0b, n))
}

/// 첫 node 하나의 이름을 바꾸는 transaction.
fn rename(session: &TextSourceSession, n: u64, node: StableId, name: &str) -> CommandTransaction {
    CommandTransaction::new(command(n), CommandOrigin::User).with_command(
        session.document().revision(),
        EditCommand::rename_node(node, name.to_string()),
    )
}

fn json(model: &DocumentModel) -> String {
    model_snapshot_json(model).unwrap()
}

/// 텍스트 원본 프로젝트 하나와 이름을 바꿀 node 둘.
fn project(tag: &str) -> (TempDir, StableId, StableId) {
    let dir = TempDir::new(tag);
    let document = golden_document(0);
    let records = document.to_records().unwrap();
    let nodes: Vec<StableId> = records.nodes.iter().map(|node| node.id).collect();
    let text = TextProject {
        document,
        components: ComponentLibrary::new(),
        token_sets: Vec::new(),
        images: BTreeMap::new(),
    };
    write_new(&dir.path.join(DESIGN_DIR), &encode(&text).unwrap()).unwrap();
    (dir, nodes[0], nodes[nodes.len() - 1])
}

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(relative, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&root.join(DESIGN_DIR), root, &mut out);
    out
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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
        .ok()?;
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).unwrap())
}

#[test]
fn save_writes_only_changed_files_through_the_journal_and_reopens_identically() {
    let (dir, node, _) = project("text-session-save");
    let has_git = git(&dir.path, &["init", "-q", "-b", "main"]).is_some();
    if has_git {
        git(&dir.path, &["add", "."]);
        git(&dir.path, &["commit", "-qm", "base"]);
    }
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    assert_eq!(session.save().unwrap(), TextSave::Clean);
    let before = files(&dir.path);
    session
        .apply(&rename(&session, 1, node, "저장한 이름"))
        .unwrap();
    assert!(session.is_dirty());
    let TextSave::Saved { written, deleted } = session.save().unwrap() else {
        panic!("저장해야 함");
    };
    assert_eq!((written, deleted), (1, 0), "바뀐 파일 하나만 씀");
    assert!(!session.is_dirty());
    let after = files(&dir.path);
    let changed: Vec<_> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .map(|(path, _)| path.clone())
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
    let leftovers: Vec<String> = std::fs::read_dir(dir.path.join(".stellar/journal"))
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    name.starts_with("tx-")
                        || name.starts_with("trash-")
                        || name.starts_with("quarantine-")
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "journal 거래가 남지 않음: {leftovers:?}"
    );
    // 재열기: 같은 의미.
    let reopened = TextSourceSession::open(&dir.path, window(2)).unwrap();
    assert_eq!(
        json(reopened.document().model()),
        json(session.document().model())
    );
    // Git diff: 바뀐 원본 파일 하나, 바꾼 이름이 보입니다.
    if has_git {
        let names = git(&dir.path, &["diff", "--name-only"]).unwrap();
        assert_eq!(names.lines().collect::<Vec<_>>(), vec![changed[0].as_str()]);
        let diff = git(&dir.path, &["diff"]).unwrap();
        assert!(diff.contains("저장한 이름"), "{diff}");
    }
}

#[test]
fn clean_session_applies_an_external_edit_as_one_undoable_command_and_ignores_its_own_save() {
    let (dir, first, last) = project("text-session-clean");
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    let original = json(session.document().model());
    // 자기 저장은 hash로 무시합니다.
    session
        .apply(&rename(&session, 1, last, "내 저장"))
        .unwrap();
    session.save().unwrap();
    assert!(matches!(
        session.poll_external(command(90)),
        TextExternal::Unchanged
    ));
    let mine = json(session.document().model());
    // 다른 writer(다른 session·편집기·git pull)가 저장합니다.
    let mut other = TextSourceSession::open(&dir.path, window(2)).unwrap();
    other.apply(&rename(&other, 2, first, "외부 편집")).unwrap();
    other.save().unwrap();

    let TextExternal::Applied {
        changes: Some(changes),
        library_changed,
    } = session.poll_external(command(91))
    else {
        panic!("clean session은 반영해야 함");
    };
    assert!(!library_changed);
    assert_eq!(changes.changed_nodes(), vec![first], "cache 무효화 입력");
    assert_eq!(
        json(session.document().model()),
        json(other.document().model())
    );
    assert!(!session.is_dirty());
    assert_eq!(
        session
            .history()
            .history()
            .entries()
            .last()
            .unwrap()
            .action_name(),
        "외부 변경 반영"
    );
    assert!(session.undo().unwrap());
    assert_eq!(json(session.document().model()), mine, "Undo 한 번");
    assert_ne!(mine, original);
}

#[test]
fn dirty_session_keeps_its_edits_and_its_next_save_is_stale() {
    let (dir, first, last) = project("text-session-dirty");
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    session
        .apply(&rename(&session, 1, last, "로컬 편집"))
        .unwrap();
    let local = json(session.document().model());
    let mut other = TextSourceSession::open(&dir.path, window(2)).unwrap();
    other.apply(&rename(&other, 2, first, "외부 편집")).unwrap();
    other.save().unwrap();
    let disk = files(&dir.path);

    let TextExternal::Pending { paths } = session.poll_external(command(91)) else {
        panic!("dirty session은 보존해야 함");
    };
    assert_eq!(paths.len(), 1);
    assert_eq!(json(session.document().model()), local, "dirty 편집 보존");
    assert!(matches!(
        session.save(),
        Err(TextSessionError::Stale { paths: stale }) if stale == paths
    ));
    assert_eq!(files(&dir.path), disk, "stale 저장은 아무것도 쓰지 않음");
    assert!(session.is_dirty());
}

#[test]
fn broken_external_yaml_is_rejected_with_path_and_line_and_applies_nothing() {
    // UX.4.4: 반영 0, 경로·줄·원인.
    let (dir, _, _) = project("text-session-broken");
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    let before = json(session.document().model());
    let document = dir.path.join(DESIGN_DIR).join("document.stellar.yaml");
    let original = std::fs::read_to_string(&document).unwrap();
    let mut lines: Vec<&str> = original.lines().collect();
    lines.insert(2, "  broken: [unclosed");
    std::fs::write(&document, lines.join("\n") + "\n").unwrap();

    let TextExternal::Rejected(diagnostic) = session.poll_external(command(1)) else {
        panic!("깨진 원본은 거부해야 함");
    };
    assert_eq!(diagnostic.code, "yaml-syntax");
    assert_eq!(diagnostic.path.as_deref(), Some("document.stellar.yaml"));
    assert!(
        diagnostic.line.is_some_and(|line| line >= 3),
        "{diagnostic}"
    );
    assert!(diagnostic.to_string().starts_with("document.stellar.yaml:"));
    assert_eq!(json(session.document().model()), before);
    assert!(session.history().history().is_empty());
    // 깨진 원본으로는 열지도 않습니다.
    assert!(matches!(
        TextSourceSession::open(&dir.path, window(2)),
        Err(TextSessionError::Source(d)) if d.code == "yaml-syntax"
    ));
    // 고치면 다시 정상(같은 내용이라 반영할 변경 없음).
    std::fs::write(&document, &original).unwrap();
    assert!(matches!(
        session.poll_external(command(2)),
        TextExternal::Unchanged
    ));
}

#[test]
fn semantic_error_names_the_changed_file() {
    let (dir, _, _) = project("text-session-semantic");
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    let document = dir.path.join(DESIGN_DIR).join("document.stellar.yaml");
    let text = std::fs::read_to_string(&document).unwrap();
    std::fs::write(
        &document,
        text.replacen("schema_version: ", "schema_version: 9", 1),
    )
    .unwrap();
    let TextExternal::Rejected(diagnostic) = session.poll_external(command(1)) else {
        panic!("의미 오류는 거부해야 함");
    };
    assert_ne!(diagnostic.code, "yaml-syntax");
    assert_eq!(diagnostic.path.as_deref(), Some("document.stellar.yaml"));
}

#[test]
fn external_change_waits_for_the_gesture() {
    let (dir, first, _) = project("text-session-gesture");
    let mut session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    let mut other = TextSourceSession::open(&dir.path, window(2)).unwrap();
    other.apply(&rename(&other, 2, first, "외부 편집")).unwrap();
    other.save().unwrap();
    session.begin_pointer_gesture(1).unwrap();
    assert!(matches!(
        session.poll_external(command(1)),
        TextExternal::Deferred
    ));
    session.cancel_preview().unwrap();
    assert!(matches!(
        session.poll_external(command(1)),
        TextExternal::Applied {
            changes: Some(_),
            ..
        }
    ));
}

#[test]
fn open_recovers_a_leftover_save_transaction_first() {
    let (dir, _, _) = project("text-session-recover");
    // manifest 전에 끝난 거래(대상은 건드리지 않음)를 남깁니다.
    std::fs::create_dir_all(dir.path.join(".stellar/journal/tx-0000000000000001")).unwrap();
    let before = files(&dir.path);
    let session = TextSourceSession::open(&dir.path, window(1)).unwrap();
    assert_eq!(session.recovered().transactions.len(), 1);
    assert_eq!(files(&dir.path), before, "복구가 원본을 바꾸지 않음");
    assert!(!dir
        .path
        .join(".stellar/journal/tx-0000000000000001")
        .exists());
}

/// 파일 감시 경로: FSEvents event 가운데 텍스트 원본 변경이 있을 때만 session을 다시 확인합니다.
#[cfg(target_os = "macos")]
#[test]
fn watcher_events_drive_the_external_check() {
    use std::time::{Duration, Instant};
    use stellar_project::watcher::fsevents::FsEventsWatcher;
    use stellar_project::watcher::{FileWatchEvent, ProjectFileWatcher};

    let (dir, first, _) = project("text-session-watch");
    let root = std::fs::canonicalize(&dir.path).unwrap();
    let mut session = TextSourceSession::open(&root, window(1)).unwrap();
    let mut watcher = FsEventsWatcher::start(&root, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut other = TextSourceSession::open(&root, window(2)).unwrap();
    other.apply(&rename(&other, 2, first, "감시 편집")).unwrap();
    other.save().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut applied = false;
    while Instant::now() < deadline && !applied {
        let events = watcher.wait(Duration::from_millis(100));
        if events.iter().any(FileWatchEvent::affects_text_source) {
            applied = matches!(
                session.poll_external(command(5)),
                TextExternal::Applied {
                    changes: Some(_),
                    ..
                }
            );
        }
    }
    assert!(applied, "감시 event로 외부 저장을 반영");
    assert_eq!(
        json(session.document().model()),
        json(other.document().model())
    );
}
