use super::*;

use super::super::file_merge::tests::{card, resized, AR};
use super::super::session::tests::MemoryGit;
use crate::test_support::TempDir;
use stellar_semantic_merge::candidate::Resolution;

use super::super::git_port::GitOperation;

/// 폭 conflict 하나(ours 144, theirs 128)인 컴포넌트 원본 세션.
fn width_session(dir: &TempDir) -> (MemoryGit, PathBuf) {
    let (relative, path) = card(&dir.path);
    let base = std::fs::read(&path).unwrap();
    let ours = resized(&path, 144.0, 60.0);
    let theirs = resized(&path, 128.0, 60.0);
    std::fs::write(&path, &ours).unwrap();
    let git = MemoryGit::default();
    git.conflict(relative.to_str().unwrap(), [&base, &ours, &theirs]);
    (git, relative)
}

#[test]
fn validated_resolution_is_written_once_and_added() {
    let dir = TempDir::new("apply-ok");
    let (git, relative) = width_session(&dir);
    let mut session = ConflictSession::open(&git, &dir.path).unwrap();
    assert!(matches!(
        apply_session(&git, &session),
        Err(ApplyError::Plan(PlanError::Blocked { .. }))
    ));
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let report = apply_session(&git, &session).unwrap();
    assert_eq!(report.written, vec![relative.clone()]);
    assert_eq!(report.added, vec![relative.clone()]);
    assert!(git.entries.borrow().is_empty(), "add 뒤 unmerged 없음");
    // Git 상태만 돌려주고 continue·commit을 부르지 않습니다(XIV.12.5).
    assert_eq!(report.operation, Some(GitOperation::Merge));
    assert_eq!(report.remaining, Some(Vec::new()));
    let calls = git.calls.borrow().clone();
    assert_eq!(
        calls,
        [
            "unmerged",
            "operation",
            "unmerged",
            "add 1",
            "operation",
            "unmerged"
        ]
    );
    assert!(!calls
        .iter()
        .any(|c| c.contains("continue") || c.contains("commit")));
    // 결과 파일은 theirs 폭(128)이고 codec이 그대로 읽습니다.
    let path = dir.path.join(&relative);
    assert_eq!(std::fs::read(&path).unwrap(), resized(&path, 128.0, 60.0));
    let (_, root) = stellar_project::document_io::component_source::open(&path)
        .map(|(source, hash)| (hash, source.root().id))
        .unwrap();
    assert_eq!(root.to_string(), AR);
}

#[test]
fn stale_session_writes_nothing() {
    let dir = TempDir::new("apply-stale");
    let (git, relative) = width_session(&dir);
    let mut session = ConflictSession::open(&git, &dir.path).unwrap();
    session.resolve(0, Resolution::ChooseOurs).unwrap();
    let path = dir.path.join(&relative);
    std::fs::write(&path, b"changed outside").unwrap();
    assert!(matches!(
        apply_session(&git, &session),
        Err(ApplyError::Stale(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"changed outside");
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// 컴포넌트 폭 conflict와 generation(서로 다른 node 이름 변경) 단위가 함께 있는 작업 트리.
fn two_unit_session(dir: &TempDir) -> (MemoryGit, PathBuf) {
    use crate::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
    use crate::lock::LockPolicy;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{random_valid_document, save_documents, scaffold_project};
    let model = random_valid_document(7);
    let kids: Vec<_> = model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id)
        .collect();
    let mut pointers = Vec::new();
    let sides = dir.path.join("sides");
    for (index, name) in ["base", "ours", "theirs"].into_iter().enumerate() {
        let side = sides.join(name);
        scaffold_project(&side);
        save_documents(&side, &[&model]);
        if index > 0 {
            let request = ApplyRequest {
                base_revision: open_snapshot(&side).unwrap().revision(),
                document: model.document_id(),
                commands: vec![CommandInput::Rename {
                    node: kids[index - 1],
                    name: name.into(),
                }],
            };
            apply(&side, &request, ApplyMode::Write, &LockPolicy::None).unwrap();
        }
        pointers.push(std::fs::read(side.join("design/current")).unwrap());
    }
    let root = dir.path.join("project");
    scaffold_project(&root);
    for name in ["base", "ours", "theirs"] {
        copy_tree(&sides.join(name).join("design"), &root.join("design"));
    }
    std::fs::write(root.join("design/current"), &pointers[1]).unwrap();
    let (git, component) = {
        let (relative, path) = card(&root);
        let base = std::fs::read(&path).unwrap();
        let ours = resized(&path, 144.0, 60.0);
        let theirs = resized(&path, 128.0, 60.0);
        std::fs::write(&path, &ours).unwrap();
        let git = MemoryGit::default();
        git.conflict(relative.to_str().unwrap(), [&base, &ours, &theirs]);
        (git, relative)
    };
    git.conflict("design/current", [&pointers[0], &pointers[1], &pointers[2]]);
    assert!(component.starts_with("design/components"));
    (git, root)
}

/// journal 작업 디렉터리(`.stellar/journal/`)를 뺀 작업 트리 digest.
fn tree(root: &Path) -> std::collections::BTreeMap<String, String> {
    let mut digest = crate::test_support::file_digest(root);
    digest.retain(|path, _| !path.starts_with(".stellar/journal"));
    digest
}

/// `dir`을 읽기 전용으로 바꾸고, 실제로 쓰기가 막히는지(root 권한이 아닌지) 돌려줍니다.
#[cfg(unix)]
fn make_read_only(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = dir.join(".probe");
    let blocked = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    blocked
}

#[cfg(unix)]
fn make_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn plan_keeps_generation_write_order_inside_one_journal_transaction() {
    let dir = TempDir::new("apply-order");
    let (git, root) = two_unit_session(&dir);
    let mut session = ConflictSession::open(&git, &root).unwrap();
    assert_eq!(session.units().len(), 2, "{:?}", session.unsupported());
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let plan = session.plan().unwrap();
    let writes = plan.writes.len();
    assert!(writes >= 4, "원본 1 + chunk·manifest·current: {writes}");
    // 세대 저장 순서: chunk가 manifest보다, manifest가 current pointer보다 먼저이고 pointer가 마지막입니다.
    // journal은 입력 순서로 반영하므로 거래 안에서도 같은 순서입니다.
    let order: Vec<&Path> = plan.writes.iter().map(|w| w.path.as_path()).collect();
    assert_eq!(
        order.last(),
        Some(&Path::new("design/current")),
        "{order:?}"
    );
    let first_manifest = order
        .iter()
        .position(|p| p.starts_with("design/generations"))
        .unwrap();
    let last_chunk = order
        .iter()
        .rposition(|p| p.starts_with("design/objects"))
        .unwrap();
    assert!(last_chunk < first_manifest, "{order:?}");
    let report = apply_session(&git, &session).unwrap();
    assert_eq!(report.written.len(), writes);
    assert!(git.entries.borrow().is_empty());
    let opened = crate::snapshot::open_snapshot(&root).unwrap();
    assert_eq!(opened.revision().generation(), 3);
    // 끝난 거래는 journal에 남지 않습니다.
    assert!(recover_interrupted(&root).unwrap().transactions.is_empty());
}

#[cfg(unix)]
#[test]
fn journal_fault_rolls_back_every_target_and_retry_succeeds() {
    let dir = TempDir::new("apply-journal-fault");
    let (git, root) = two_unit_session(&dir);
    let mut session = ConflictSession::open(&git, &root).unwrap();
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let before = tree(&root);
    let entries = git.entries.borrow().clone();
    // manifest 반영 단계에서 실패합니다(chunk·컴포넌트 원본은 이미 반영된 뒤).
    let generations = root.join("design/generations");
    if !make_read_only(&generations) {
        make_writable(&generations);
        eprintln!("권한으로 쓰기를 막을 수 없는 환경(root)이라 건너뜁니다");
        return;
    }
    let outcome = apply_session(&git, &session);
    make_writable(&generations);
    match outcome {
        Err(error @ ApplyError::Write { .. }) => {
            let ApplyError::Write { unrestored, .. } = &error else {
                unreachable!()
            };
            assert!(unrestored.is_empty(), "{error}");
            assert!(error.guidance().contains("unresolved"));
        }
        other => panic!("journal 쓰기 실패 기대: {other:?}"),
    }
    // journal이 모든 대상을 앞 세대로 되돌렸고 add는 부르지 않았습니다.
    assert_eq!(tree(&root), before);
    assert_eq!(*git.entries.borrow(), entries);
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
    // 원인을 없애고 다시 적용하면 두 단위를 모두 씁니다.
    let report = apply_session(&git, &session).unwrap();
    assert!(git.entries.borrow().is_empty());
    assert!(git.entries.borrow().is_empty(), "{:?}", report.added);
    assert_eq!(
        crate::snapshot::open_snapshot(&root)
            .unwrap()
            .revision()
            .generation(),
        3
    );
}

#[test]
fn interrupted_transaction_is_recovered_before_apply() {
    let dir = TempDir::new("apply-journal-crash");
    let (git, root) = two_unit_session(&dir);
    let mut session = ConflictSession::open(&git, &root).unwrap();
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    // 앞 적용이 거래 디렉터리를 만든 직후(manifest 전) 프로세스가 죽은 상태입니다(ADR-0078 복구 규칙: 대상을
    // 건드리기 전이라 버림). 첫 journal 접근이 journal 디렉터리를 만들도록 빈 복구를 먼저 합니다.
    assert!(recover_interrupted(&root).unwrap().transactions.is_empty());
    std::fs::create_dir(root.join(".stellar/journal/tx-crashed")).unwrap();
    let before = tree(&root);
    let report = apply_session(&git, &session).unwrap();
    assert_ne!(tree(&root), before);
    assert!(git.entries.borrow().is_empty(), "{:?}", report.added);
    assert!(!root.join(".stellar/journal/tx-crashed").exists());
    assert!(recover_interrupted(&root).unwrap().transactions.is_empty());
}

#[test]
fn add_failure_restores_the_working_tree_with_an_inverse_transaction() {
    let dir = TempDir::new("apply-add-fault");
    let (git, root) = two_unit_session(&dir);
    let mut session = ConflictSession::open(&git, &root).unwrap();
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let before = tree(&root);
    let entries = git.entries.borrow().clone();
    let failing = MemoryGit {
        entries: std::cell::RefCell::new(entries.clone()),
        blobs: std::cell::RefCell::new(git.blobs.borrow().clone()),
        add_fails: true,
        ..MemoryGit::default()
    };
    match apply_session(&failing, &session) {
        Err(error @ ApplyError::Add { .. }) => {
            let ApplyError::Add { unrestored, .. } = &error else {
                unreachable!()
            };
            assert!(unrestored.is_empty());
            assert!(error.guidance().contains("commit하지 않았습니다"));
        }
        other => panic!("add 실패 기대: {other:?}"),
    }
    assert_eq!(tree(&root), before);
    assert_eq!(*failing.entries.borrow(), entries);
    assert!(recover_interrupted(&root).unwrap().transactions.is_empty());
}
