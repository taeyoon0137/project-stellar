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

/// 컴포넌트 폭 conflict와 텍스트 원본(서로 다른 node 이름 변경) 단위가 함께 있는 작업 트리. chunk cache는 Git 밖이라
/// (ADR-0122) 텍스트 원본 파일만 conflict입니다. 돌려주는 값은 Git 대역, 프로젝트 root, conflict인 원본 경로입니다.
fn two_unit_session(dir: &TempDir) -> (MemoryGit, PathBuf) {
    use crate::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
    use crate::lock::LockPolicy;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{random_valid_document, save_text_document, scaffold_project};
    let model = random_valid_document(7);
    let kids: Vec<_> = model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id)
        .collect();
    let mut sides_source = Vec::new();
    let sides = dir.path.join("sides");
    for (index, name) in ["base", "ours", "theirs"].into_iter().enumerate() {
        let side = sides.join(name);
        scaffold_project(&side);
        save_text_document(&side, &model);
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
        sides_source.push(
            stellar_project::document_io::session::read_source(&side)
                .unwrap()
                .unwrap()
                .files,
        );
    }
    // 작업 트리는 ours 원본(병합 중 Git이 놓는 ours 쪽)입니다. cache는 없습니다(Git checkout처럼).
    let root = dir.path.join("project");
    scaffold_project(&root);
    copy_tree(&sides.join("ours").join("design"), &root.join("design"));
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
    let mut conflicted = 0;
    for (relative, ours) in &sides_source[1] {
        let base = sides_source[0].get(relative).cloned().unwrap_or_default();
        let theirs = sides_source[2].get(relative).cloned().unwrap_or_default();
        if &base != ours || &theirs != ours {
            git.conflict(
                &format!("design/{relative}"),
                [base.as_bytes(), ours.as_bytes(), theirs.as_bytes()],
            );
            conflicted += 1;
        }
    }
    assert!(conflicted > 0, "두 이름 변경이 텍스트 원본 conflict여야 함");
    assert!(component.starts_with("design/components"));
    (git, root)
}

/// 이름 변경 두 개가 모두 반영됐는지(원본에서 읽음).
fn merged_names(root: &Path) -> Vec<String> {
    let snapshot = crate::snapshot::open_snapshot(root).unwrap();
    let id = snapshot.documents()[0].id;
    let mut names: Vec<String> = snapshot
        .document_records(id)
        .unwrap()
        .nodes
        .into_iter()
        .map(|n| n.display_name)
        .filter(|n| n == "ours" || n == "theirs")
        .collect();
    names.sort();
    names
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
    assert!(writes >= 2, "컴포넌트 원본 1 + 텍스트 원본: {writes}");
    // 계획에는 Git 원본만 있고 chunk cache 경로는 없습니다(ADR-0122). cache는 다음 열기가 원본에서 다시 만듭니다.
    assert!(
        plan.writes
            .iter()
            .all(|w| w.path.starts_with("design/") && !w.path.starts_with(".stellar")),
        "{:?}",
        plan.writes
    );
    let report = apply_session(&git, &session).unwrap();
    assert_eq!(report.written.len(), writes);
    assert!(git.entries.borrow().is_empty());
    assert_eq!(merged_names(&root), ["ours", "theirs"]);
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
    // 텍스트 원본 반영 단계에서 실패합니다(컴포넌트 원본은 경로 순서로 이미 반영된 뒤).
    let text_path = entries
        .iter()
        .map(|e| e.path.clone())
        .find(|p| !p.starts_with("design/components"))
        .unwrap();
    let generations = root.join(text_path.parent().unwrap());
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
    assert_eq!(merged_names(&root), ["ours", "theirs"]);
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
            assert!(unrestored.is_empty(), "{error} {unrestored:?}");
            assert!(error.guidance().contains("commit하지 않았습니다"));
        }
        other => panic!("add 실패 기대: {other:?}"),
    }
    assert_eq!(tree(&root), before);
    assert_eq!(*failing.entries.borrow(), entries);
    assert!(recover_interrupted(&root).unwrap().transactions.is_empty());
}
