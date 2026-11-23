use super::*;

use super::super::file_merge::tests::{card, resized, AR};
use super::super::session::tests::MemoryGit;
use crate::test_support::TempDir;
use stellar_semantic_merge::candidate::Resolution;

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
        apply_session(&git, &DiskFiles, &session),
        Err(ApplyError::Plan(PlanError::Blocked { .. }))
    ));
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let report = apply_session(&git, &DiskFiles, &session).unwrap();
    assert_eq!(report.written, vec![relative.clone()]);
    assert_eq!(report.added, vec![relative.clone()]);
    assert!(git.entries.borrow().is_empty(), "add 뒤 unmerged 없음");
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
        apply_session(&git, &DiskFiles, &session),
        Err(ApplyError::Stale(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"changed outside");
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
}
