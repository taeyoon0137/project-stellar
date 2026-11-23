//! journal 검사와 fault matrix (V.10.5, ADR-0078).
//!
//! fault matrix는 성공 실행의 모든 단계를 기록한 뒤, 각 단계 직전에 프로세스 중단(crash)과 오류(일반·디스크
//! 가득 참·권한)를 하나씩 주입합니다. 중단 뒤에는 [`recover`]로 다시 엽니다. 매번 프로젝트 파일 전체가 앞
//! 세대나 새 세대 중 하나와 bytes 단위로 같은지, 그리고 marker 전 중단은 앞 세대·marker 뒤 중단은 새 세대인지
//! 확인합니다. 복구 자체가 중단되는 경우도 복구 단계마다 주입합니다.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use super::*;
use crate::fs_guard::test_support::TempDir;

const LIMIT: u64 = 1 << 20;

type Tree = BTreeMap<String, Vec<u8>>;

fn h(bytes: &[u8]) -> ContentHash {
    ContentHash::compute(bytes)
}

/// 프로젝트 파일 전체(`.stellar/` 제외, symlink는 `->대상`으로 기록).
fn snapshot(root: &Path) -> Tree {
    fn walk(root: &Path, dir: &Path, out: &mut Tree) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rel == ".stellar" {
                continue;
            }
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                let to = std::fs::read_link(&path).unwrap();
                out.insert(rel, format!("->{}", to.display()).into_bytes());
            } else if kind.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = Tree::new();
    walk(root, root, &mut out);
    out
}

fn tree(files: &[(&str, &str)]) -> Tree {
    files
        .iter()
        .map(|(p, b)| (p.to_string(), b.as_bytes().to_vec()))
        .collect()
}

fn old_tree() -> Tree {
    tree(&[
        ("design/views/home.yaml", "home v1\n"),
        ("design/views/about.yaml", "about v1\n"),
        ("design/components/button.yaml", "button v1\n"),
        ("design/tokens/old.yaml", "old tokens\n"),
        ("README.md", "keep\n"),
    ])
}

fn new_tree() -> Tree {
    tree(&[
        ("design/views/home.yaml", "home v2\n"),
        ("design/views/about.yaml", "about v2 longer\n"),
        ("design/components/confirm.yaml", "confirm v2\n"),
        ("design/pages/new/page.yaml", "page v1\n"),
        ("README.md", "keep\n"),
    ])
}

fn write_tree(root: &Path, files: &Tree) {
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

fn changes() -> Vec<JournalChange> {
    let [delete_button, create_confirm] = JournalChange::rename(
        "design/components/button.yaml",
        "design/components/confirm.yaml",
        b"confirm v2\n".to_vec(),
        h(b"button v1\n"),
    );
    vec![
        JournalChange::write(
            "design/views/home.yaml",
            b"home v2\n".to_vec(),
            Some(h(b"home v1\n")),
        ),
        JournalChange::write(
            "design/views/about.yaml",
            b"about v2 longer\n".to_vec(),
            Some(h(b"about v1\n")),
        ),
        delete_button,
        create_confirm,
        JournalChange::delete("design/tokens/old.yaml", h(b"old tokens\n")),
        JournalChange::write("design/pages/new/page.yaml", b"page v1\n".to_vec(), None),
        // 같은 내용: 거래에서 빠집니다.
        JournalChange::write("README.md", b"keep\n".to_vec(), Some(h(b"keep\n"))),
    ]
}

fn fixture(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    write_tree(&dir.path, &old_tree());
    dir
}

/// journal 디렉터리에 `.gitignore` 말고 아무것도 남지 않았는지.
fn journal_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(".stellar/journal"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != ".gitignore")
        .collect();
    names.sort();
    names
}

/// 성공 실행의 단계를 기록합니다.
struct Recorder(Vec<Step>);

impl Observer for Recorder {
    fn before(&mut self, step: Step) -> Result<(), Interrupt> {
        self.0.push(step);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    Crash,
    Fail(io::ErrorKind),
}

/// `index`번째 단계 직전에 `fault`를 주입합니다.
struct FaultAt {
    index: usize,
    seen: usize,
    fault: Fault,
}

impl FaultAt {
    fn new(index: usize, fault: Fault) -> FaultAt {
        FaultAt {
            index,
            seen: 0,
            fault,
        }
    }
}

impl Observer for FaultAt {
    fn before(&mut self, _step: Step) -> Result<(), Interrupt> {
        let now = self.seen;
        self.seen += 1;
        if now != self.index {
            return Ok(());
        }
        match self.fault {
            Fault::Crash => Err(Interrupt::Crash),
            Fault::Fail(kind) => Err(Interrupt::Fail(io::Error::new(kind, "주입된 오류"))),
        }
    }
}

fn recorded_steps(name: &str) -> Vec<Step> {
    let dir = fixture(name);
    let mut recorder = Recorder(Vec::new());
    let report = commit_observed(&dir.path, &changes(), LIMIT, &mut recorder).unwrap();
    assert_eq!((report.written, report.deleted), (4, 2));
    assert_eq!(snapshot(&dir.path), new_tree());
    recorder.0
}

#[test]
fn commit_writes_every_change_as_one_generation() {
    let dir = fixture("journal-commit");
    let report = commit(&dir.path, &changes(), LIMIT).unwrap();
    assert_eq!(report.written, 4);
    assert_eq!(report.deleted, 2);
    assert!(!report.cleanup_pending);
    assert!(report.recovered.transactions.is_empty());
    assert_eq!(snapshot(&dir.path), new_tree());
    assert!(journal_names(&dir.path).is_empty());
    assert_eq!(
        std::fs::read(dir.path.join(".stellar/journal/.gitignore")).unwrap(),
        b"*\n"
    );
    // 같은 변경을 다시 보내면 기대 hash가 달라 stale입니다.
    let error = commit(&dir.path, &changes(), LIMIT).unwrap_err();
    assert!(matches!(error, JournalError::Stale { .. }), "{error:?}");
    assert_eq!(snapshot(&dir.path), new_tree());
}

#[test]
fn recorded_steps_follow_the_adr_order() {
    let steps = recorded_steps("journal-steps");
    use Step::*;
    // 항목 순서: home, about, button 삭제, confirm 생성, old 삭제, page 생성(README는 같은 내용이라 빠짐).
    assert_eq!(
        steps,
        vec![
            CreateTx,
            WriteManifest,
            WriteBackup(0),
            WriteBackup(1),
            WriteBackup(2),
            WriteBackup(4),
            WriteNew(0),
            WriteNew(1),
            WriteNew(3),
            WriteNew(5),
            SyncStaging,
            Publish(0),
            Publish(1),
            Publish(2),
            Publish(3),
            Publish(4),
            Publish(5),
            SyncTargets,
            WriteMarker,
            Cleanup,
        ]
    );
}

#[test]
fn fault_matrix_recovers_exactly_one_generation() {
    let steps = recorded_steps("journal-matrix-record");
    let faults = [
        Fault::Crash,
        Fault::Fail(io::ErrorKind::Other),
        Fault::Fail(io::ErrorKind::StorageFull),
        Fault::Fail(io::ErrorKind::PermissionDenied),
    ];
    for (index, at) in steps.iter().enumerate() {
        for fault in faults {
            let dir = fixture("journal-matrix");
            let result = commit_observed(
                &dir.path,
                &changes(),
                LIMIT,
                &mut FaultAt::new(index, fault),
            );
            let after_marker = *at == Step::Cleanup;
            match (fault, &result) {
                (Fault::Crash, Err(Failure::Crash)) => {}
                (Fault::Fail(_), Ok(report)) if after_marker => assert!(report.cleanup_pending),
                (Fault::Fail(kind), Err(Failure::Error(JournalError::Io { source, .. }))) => {
                    assert_eq!(source.kind(), kind, "{at:?} {fault:?}");
                }
                other => panic!("{at:?} {fault:?}: 예상 밖 결과 {other:?}"),
            }
            if matches!(fault, Fault::Fail(_)) && !after_marker {
                // 우아한 실패는 그 자리에서 앞 세대로 되돌리고 거래를 남기지 않습니다.
                assert_eq!(snapshot(&dir.path), old_tree(), "{at:?} {fault:?}");
                assert!(journal_names(&dir.path).is_empty(), "{at:?} {fault:?}");
            }
            // 재시작: 복구 뒤 정확히 한 세대.
            let report = recover(&dir.path).unwrap();
            let expected = if after_marker { new_tree() } else { old_tree() };
            assert_eq!(snapshot(&dir.path), expected, "{at:?} {fault:?}");
            assert!(journal_names(&dir.path).is_empty(), "{at:?} {fault:?}");
            for tx in &report.transactions {
                assert!(
                    !matches!(
                        tx.outcome,
                        RecoveryOutcome::Quarantined { .. } | RecoveryOutcome::Unreadable
                    ),
                    "{at:?} {fault:?}: {tx:?}"
                );
            }
            // 복구한 프로젝트에 같은 저장을 다시 하면 새 세대가 됩니다(앞 세대일 때).
            if !after_marker {
                commit(&dir.path, &changes(), LIMIT).unwrap();
                assert_eq!(snapshot(&dir.path), new_tree(), "{at:?} {fault:?}");
            }
        }
    }
}

/// commit 중단 뒤 복구가 다시 중단되어도, 마지막 복구는 정확히 한 세대를 만듭니다.
#[test]
fn interrupted_recovery_is_restartable() {
    let steps = recorded_steps("journal-recovery-record");
    for (index, at) in steps.iter().enumerate() {
        // 이 commit 중단 지점에서 복구가 밟는 단계 수.
        let dir = fixture("journal-recovery-count");
        let _ = commit_observed(
            &dir.path,
            &changes(),
            LIMIT,
            &mut FaultAt::new(index, Fault::Crash),
        );
        let mut recorder = Recorder(Vec::new());
        recover_observed(&dir.path, &mut recorder).unwrap();
        let recovery_steps = recorder.0.len();
        let expected = if *at == Step::Cleanup {
            new_tree()
        } else {
            old_tree()
        };
        assert_eq!(snapshot(&dir.path), expected, "{at:?}");

        for j in 0..recovery_steps {
            for fault in [Fault::Crash, Fault::Fail(io::ErrorKind::PermissionDenied)] {
                let dir = fixture("journal-recovery");
                let _ = commit_observed(
                    &dir.path,
                    &changes(),
                    LIMIT,
                    &mut FaultAt::new(index, Fault::Crash),
                );
                let interrupted = recover_observed(&dir.path, &mut FaultAt::new(j, fault));
                assert!(interrupted.is_err(), "{at:?} 복구 {j} {fault:?}");
                // 복구 도중의 상태는 섞일 수 있습니다. 다음 복구가 같은 hash 판정으로 끝냅니다.
                recover(&dir.path).unwrap();
                assert_eq!(snapshot(&dir.path), expected, "{at:?} 복구 {j} {fault:?}");
                assert!(journal_names(&dir.path).is_empty(), "{at:?} 복구 {j}");
            }
        }
    }
}

/// marker가 있는데 반영이 덜 된 거래(파일 시스템이 순서를 바꾼 경우)는 새 세대로 끝냅니다.
#[test]
fn marker_with_unfinished_publish_rolls_forward() {
    let steps = recorded_steps("journal-forward-record");
    for publish in steps.iter().filter(|s| matches!(s, Step::Publish(_))) {
        let index = steps.iter().position(|s| s == publish).unwrap();
        let dir = fixture("journal-forward");
        let _ = commit_observed(
            &dir.path,
            &changes(),
            LIMIT,
            &mut FaultAt::new(index, Fault::Crash),
        );
        let names = journal_names(&dir.path);
        assert_eq!(names.len(), 1);
        let tx = dir.path.join(".stellar/journal").join(&names[0]);
        std::fs::write(tx.join(MARKER_FILE), MARKER).unwrap();
        let report = recover(&dir.path).unwrap();
        assert_eq!(
            report.transactions[0].outcome,
            RecoveryOutcome::RolledForward,
            "{publish:?}"
        );
        assert_eq!(snapshot(&dir.path), new_tree(), "{publish:?}");
        assert!(journal_names(&dir.path).is_empty());
    }
}

/// 손상된 journal 사본은 쓰지 않고, 그 대상은 건드리지 않은 채 거래를 보존합니다.
#[test]
fn damaged_journal_copy_is_never_installed() {
    let steps = recorded_steps("journal-damaged-record");
    let index = steps.iter().position(|s| *s == Step::Publish(1)).unwrap();
    let dir = fixture("journal-damaged");
    let _ = commit_observed(
        &dir.path,
        &changes(),
        LIMIT,
        &mut FaultAt::new(index, Fault::Crash),
    );
    let names = journal_names(&dir.path);
    let tx = dir.path.join(".stellar/journal").join(&names[0]);
    // 반영한 항목 0(home)의 원본 사본을 손상시킵니다.
    std::fs::write(tx.join(ORIG_DIR).join("0"), b"torn").unwrap();
    let report = recover(&dir.path).unwrap();
    assert_eq!(
        report.transactions[0].outcome,
        RecoveryOutcome::Quarantined {
            conflicts: vec!["design/views/home.yaml".to_string()]
        }
    );
    let mut expected = old_tree();
    expected.insert("design/views/home.yaml".into(), b"home v2\n".to_vec());
    assert_eq!(snapshot(&dir.path), expected);
    let kept = journal_names(&dir.path);
    assert_eq!(kept.len(), 1);
    assert!(kept[0].starts_with(QUARANTINE_PREFIX));
    // 보존한 거래는 다음 저장을 막지 않습니다.
    commit(
        &dir.path,
        &[JournalChange::write(
            "design/views/home.yaml",
            b"home v1\n".to_vec(),
            Some(h(b"home v2\n")),
        )],
        LIMIT,
    )
    .unwrap();
    assert_eq!(snapshot(&dir.path), old_tree());
}

/// 중단 뒤 사용자가 고친 대상은 복구가 덮어쓰지 않습니다.
#[test]
fn recovery_never_overwrites_external_edits() {
    let steps = recorded_steps("journal-external-record");
    let index = steps.iter().position(|s| *s == Step::Publish(2)).unwrap();
    let dir = fixture("journal-external");
    let _ = commit_observed(
        &dir.path,
        &changes(),
        LIMIT,
        &mut FaultAt::new(index, Fault::Crash),
    );
    std::fs::write(dir.path.join("design/views/about.yaml"), b"user edit\n").unwrap();
    let report = recover(&dir.path).unwrap();
    assert_eq!(
        report.transactions[0].outcome,
        RecoveryOutcome::Quarantined {
            conflicts: vec!["design/views/about.yaml".to_string()]
        }
    );
    let mut expected = old_tree();
    expected.insert("design/views/about.yaml".into(), b"user edit\n".to_vec());
    assert_eq!(snapshot(&dir.path), expected);
}

#[test]
fn stale_expected_hash_writes_nothing() {
    let dir = fixture("journal-stale");
    std::fs::write(dir.path.join("design/views/about.yaml"), b"external\n").unwrap();
    std::fs::write(dir.path.join("design/pages-other.yaml"), b"x").unwrap();
    let mut before = snapshot(&dir.path);
    let mut changes = changes();
    changes.push(JournalChange::write(
        "design/pages-other.yaml",
        b"y".to_vec(),
        None,
    ));
    let error = commit(&dir.path, &changes, LIMIT).unwrap_err();
    match error {
        JournalError::Stale { paths } => assert_eq!(
            paths,
            vec![
                "design/views/about.yaml".to_string(),
                "design/pages-other.yaml".to_string()
            ]
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(snapshot(&dir.path), before);
    assert!(journal_names(&dir.path).is_empty());
    assert!(
        !dir.path.join("design/pages").exists(),
        "부모 디렉터리도 만들지 않음"
    );
    // 지울 대상이 이미 없어도 stale입니다.
    std::fs::remove_file(dir.path.join("design/tokens/old.yaml")).unwrap();
    before.remove("design/tokens/old.yaml");
    let error = commit(
        &dir.path,
        &[JournalChange::delete(
            "design/tokens/old.yaml",
            h(b"old tokens\n"),
        )],
        LIMIT,
    )
    .unwrap_err();
    assert!(matches!(error, JournalError::Stale { .. }));
    assert_eq!(snapshot(&dir.path), before);
}

/// 반영 도중 다른 대상이 외부에서 바뀌면 이미 반영한 대상을 되돌리고 stale입니다.
#[test]
fn concurrent_external_edit_during_publish_rolls_back() {
    struct EditAt<'a> {
        step: Step,
        path: &'a Path,
        bytes: &'a [u8],
    }
    impl Observer for EditAt<'_> {
        fn before(&mut self, step: Step) -> Result<(), Interrupt> {
            if step == self.step {
                std::fs::write(self.path, self.bytes).map_err(Interrupt::Fail)?;
            }
            Ok(())
        }
    }
    // 교체 대상, 새 파일 대상(외부 생성), 삭제 대상 각각.
    for (step, path, bytes) in [
        (
            Step::Publish(1),
            "design/views/about.yaml",
            b"external\n".as_slice(),
        ),
        (
            Step::Publish(3),
            "design/components/confirm.yaml",
            b"external\n",
        ),
        (Step::Publish(4), "design/tokens/old.yaml", b"external\n"),
    ] {
        let dir = fixture("journal-concurrent");
        let target = dir.path.join(path);
        let result = commit_observed(
            &dir.path,
            &changes(),
            LIMIT,
            &mut EditAt {
                step,
                path: &target,
                bytes,
            },
        );
        match result {
            Err(Failure::Error(JournalError::Stale { paths })) => assert_eq!(paths, vec![path]),
            other => panic!("{step:?}: {other:?}"),
        }
        let mut expected = old_tree();
        expected.insert(path.to_string(), bytes.to_vec());
        assert_eq!(snapshot(&dir.path), expected, "{step:?}");
        assert!(journal_names(&dir.path).is_empty(), "{step:?}");
    }
}

#[test]
fn second_writer_is_refused_while_journal_is_locked() {
    let dir = fixture("journal-busy");
    // 첫 저장으로 journal 디렉터리를 만듭니다.
    commit(&dir.path, &[], LIMIT).unwrap();
    let held = File::open(dir.path.join(".stellar/journal")).unwrap();
    held.lock().unwrap();
    let error = commit(&dir.path, &changes(), LIMIT).unwrap_err();
    assert!(matches!(error, JournalError::Busy), "{error:?}");
    assert!(matches!(
        recover(&dir.path).unwrap_err(),
        JournalError::Busy
    ));
    assert_eq!(snapshot(&dir.path), old_tree());
    drop(held);
    commit(&dir.path, &changes(), LIMIT).unwrap();
    assert_eq!(snapshot(&dir.path), new_tree());
}

#[test]
fn invalid_paths_and_duplicates_are_refused_before_any_write() {
    let dir = fixture("journal-paths");
    for path in [
        "../outside.yaml",
        "/abs.yaml",
        ".git/hooks/pre-commit",
        ".stellar/journal/x",
        "design//x",
    ] {
        let error = commit(
            &dir.path,
            &[JournalChange::write(path, b"x".to_vec(), None)],
            LIMIT,
        )
        .unwrap_err();
        assert!(matches!(error, JournalError::InvalidPath { .. }), "{path}");
    }
    let error = commit(
        &dir.path,
        &[
            JournalChange::write("design/A.yaml", b"x".to_vec(), None),
            JournalChange::write("design/a.yaml", b"y".to_vec(), None),
        ],
        LIMIT,
    )
    .unwrap_err();
    assert!(matches!(error, JournalError::DuplicatePath { .. }));
    let error = commit(
        &dir.path,
        &[JournalChange::write("design/big.yaml", vec![0; 9], None)],
        8,
    )
    .unwrap_err();
    assert!(matches!(error, JournalError::TooLarge { limit: 8, .. }));
    assert_eq!(snapshot(&dir.path), old_tree());
    assert!(
        !dir.path.join(".stellar").exists(),
        "검사 실패는 journal도 만들지 않음"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_directories_and_targets_are_refused() {
    let dir = fixture("journal-symlink");
    let outside = TempDir::new("journal-symlink-outside");
    std::fs::write(outside.path.join("victim.yaml"), b"outside\n").unwrap();
    std::os::unix::fs::symlink(&outside.path, dir.path.join("design/linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.path.join("victim.yaml"),
        dir.path.join("design/link.yaml"),
    )
    .unwrap();
    let before = snapshot(&dir.path);
    for change in [
        JournalChange::write("design/linked/new.yaml", b"x".to_vec(), None),
        JournalChange::write(
            "design/linked/victim.yaml",
            b"x".to_vec(),
            Some(h(b"outside\n")),
        ),
        JournalChange::write("design/link.yaml", b"x".to_vec(), Some(h(b"outside\n"))),
        JournalChange::delete("design/link.yaml", h(b"outside\n")),
    ] {
        let mut all = changes();
        all.push(change.clone());
        let error = commit(&dir.path, &all, LIMIT).unwrap_err();
        let JournalError::Io { source, .. } = &error else {
            panic!("{change:?}: {error:?}");
        };
        assert!(Refusal::of(source).is_some(), "{change:?}: {error:?}");
        assert_eq!(snapshot(&dir.path), before, "{change:?}");
    }
    assert_eq!(
        std::fs::read(outside.path.join("victim.yaml")).unwrap(),
        b"outside\n"
    );
    assert_eq!(std::fs::read_dir(&outside.path).unwrap().count(), 1);
}

/// 실제 권한 오류: 두 번째 디렉터리에 쓸 수 없으면 첫 디렉터리에 반영한 것도 되돌립니다.
#[cfg(unix)]
#[test]
fn real_permission_error_mid_publish_restores_previous_generation() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fixture("journal-permission");
    let locked = dir.path.join("design/tokens");
    std::fs::set_permissions(&locked, Permissions::from_mode(0o555)).unwrap();
    let result = commit(&dir.path, &changes(), LIMIT);
    std::fs::set_permissions(&locked, Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    let JournalError::Io { source, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied, "{error:?}");
    assert_eq!(snapshot(&dir.path), old_tree());
    assert!(journal_names(&dir.path).is_empty());
}

#[cfg(unix)]
#[test]
fn replacement_and_rollback_preserve_permission_bits() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fixture("journal-mode");
    let home = dir.path.join("design/views/home.yaml");
    std::fs::set_permissions(&home, Permissions::from_mode(0o640)).unwrap();
    // 반영 뒤 실패 → 원본 사본이 권한과 함께 돌아옵니다.
    let steps = recorded_steps("journal-mode-record");
    let index = steps.iter().position(|s| *s == Step::Publish(2)).unwrap();
    let _ = commit_observed(
        &dir.path,
        &changes(),
        LIMIT,
        &mut FaultAt::new(index, Fault::Fail(io::ErrorKind::Other)),
    );
    assert_eq!(snapshot(&dir.path), old_tree());
    assert_eq!(
        std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
        0o640
    );
    commit(&dir.path, &changes(), LIMIT).unwrap();
    assert_eq!(
        std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn tampered_or_partial_journal_entries_are_handled_without_touching_targets() {
    let dir = fixture("journal-tampered");
    commit(&dir.path, &[], LIMIT).unwrap();
    let journal = dir.path.join(".stellar/journal");
    // manifest 전 중단: 버림.
    std::fs::create_dir_all(journal.join("tx-00aa/orig")).unwrap();
    // 끝난 거래 정리 중단: 지움.
    std::fs::create_dir_all(journal.join("trash-00bb/new")).unwrap();
    // 프로젝트 밖을 가리키는 manifest: 대상을 건드리지 않고 보존.
    std::fs::create_dir_all(journal.join("tx-00cc/new")).unwrap();
    let evil = format!(
        "stellar-journal 1\ntx 00cc\nwrite none 0 {} 4 ../outside.yaml\n",
        h(b"evil")
    );
    std::fs::write(journal.join("tx-00cc/manifest"), evil).unwrap();
    std::fs::write(journal.join("tx-00cc/new/0"), b"evil").unwrap();
    std::fs::write(journal.join("tx-00cc/committed"), MARKER).unwrap();
    // 다른 거래 ID를 적은 manifest.
    std::fs::create_dir_all(journal.join("tx-00dd")).unwrap();
    std::fs::write(
        journal.join("tx-00dd/manifest"),
        "stellar-journal 1\ntx 00ee\n",
    )
    .unwrap();
    let report = recover(&dir.path).unwrap();
    let outcomes: Vec<(&str, &RecoveryOutcome)> = report
        .transactions
        .iter()
        .map(|t| (t.id.as_str(), &t.outcome))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("00aa", &RecoveryOutcome::Discarded),
            ("00cc", &RecoveryOutcome::Unreadable),
            ("00dd", &RecoveryOutcome::Unreadable),
        ]
    );
    assert_eq!(snapshot(&dir.path), old_tree());
    assert!(!dir.path.parent().unwrap().join("outside.yaml").exists());
    assert_eq!(
        journal_names(&dir.path),
        vec!["quarantine-00cc", "quarantine-00dd"]
    );
}

#[test]
fn different_ignore_rule_is_not_overwritten() {
    let dir = fixture("journal-ignore");
    std::fs::create_dir_all(dir.path.join(".stellar/journal")).unwrap();
    std::fs::write(dir.path.join(".stellar/journal/.gitignore"), b"!\n").unwrap();
    assert!(commit(&dir.path, &changes(), LIMIT).is_err());
    assert_eq!(
        std::fs::read(dir.path.join(".stellar/journal/.gitignore")).unwrap(),
        b"!\n"
    );
    assert_eq!(snapshot(&dir.path), old_tree());
}
