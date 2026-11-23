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
        apply_session(&git, &DiskFiles, &session),
        Err(ApplyError::Plan(PlanError::Blocked { .. }))
    ));
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let report = apply_session(&git, &DiskFiles, &session).unwrap();
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
        apply_session(&git, &DiskFiles, &session),
        Err(ApplyError::Stale(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"changed outside");
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
}

/// n번째(0부터) 쓰기·삭제 호출을 실패시키는 파일 시스템.
struct FaultyFiles {
    fail: std::collections::BTreeSet<usize>,
    calls: std::cell::Cell<usize>,
}

impl FaultyFiles {
    fn new(fail: &[usize]) -> FaultyFiles {
        FaultyFiles {
            fail: fail.iter().copied().collect(),
            calls: std::cell::Cell::new(0),
        }
    }

    fn tick(&self) -> io::Result<()> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        if self.fail.contains(&n) {
            Err(io::Error::other(format!("주입한 쓰기 실패 {n}")))
        } else {
            Ok(())
        }
    }
}

impl TreeFiles for FaultyFiles {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        DiskFiles.read(path)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.tick()?;
        DiskFiles.write(path, bytes)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        DiskFiles.remove(path)
    }
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

#[test]
fn fault_matrix_restores_the_working_tree_and_keeps_conflicts_unresolved() {
    let dir = TempDir::new("apply-faults");
    let (git, root) = two_unit_session(&dir);
    let mut session = ConflictSession::open(&git, &root).unwrap();
    assert_eq!(session.units().len(), 2, "{:?}", session.unsupported());
    session.resolve(0, Resolution::ChooseTheirs).unwrap();
    let writes = session.plan().unwrap().writes.len();
    assert!(writes >= 4, "원본 1 + chunk·manifest·current: {writes}");
    let before = crate::test_support::file_digest(&root);
    let entries = git.entries.borrow().clone();
    // 쓰기 i번째 실패: 되돌린 뒤 작업 트리 bytes와 unmerged가 그대로이고 add는 부르지 않습니다.
    for fail in 0..writes {
        let files = FaultyFiles::new(&[fail]);
        match apply_session(&git, &files, &session) {
            Err(error @ ApplyError::Write { .. }) => {
                let ApplyError::Write { unrestored, .. } = &error else {
                    unreachable!()
                };
                assert!(unrestored.is_empty(), "{fail}");
                assert!(error.guidance().contains("unresolved"));
            }
            other => panic!("쓰기 {fail} 실패 기대: {other:?}"),
        }
        assert_eq!(
            crate::test_support::file_digest(&root),
            before,
            "쓰기 {fail}"
        );
        assert_eq!(*git.entries.borrow(), entries);
    }
    assert!(!git.calls.borrow().iter().any(|c| c.starts_with("add")));
    // git add 실패: 모든 쓰기를 되돌리고 unresolved 상태와 안내를 돌려줍니다.
    let failing = MemoryGit {
        entries: std::cell::RefCell::new(entries.clone()),
        blobs: std::cell::RefCell::new(git.blobs.borrow().clone()),
        add_fails: true,
        ..MemoryGit::default()
    };
    match apply_session(&failing, &DiskFiles, &session) {
        Err(error @ ApplyError::Add { .. }) => {
            assert!(error.guidance().contains("commit하지 않았습니다"));
        }
        other => panic!("add 실패 기대: {other:?}"),
    }
    assert_eq!(crate::test_support::file_digest(&root), before);
    assert_eq!(*failing.entries.borrow(), entries);
    // 되돌리기마저 실패하면 남은 경로를 안내합니다.
    let files = FaultyFiles::new(&[1, 2]);
    match apply_session(&git, &files, &session) {
        Err(error @ ApplyError::Write { .. }) => {
            let ApplyError::Write { unrestored, .. } = &error else {
                unreachable!()
            };
            assert_eq!(unrestored.len(), 1);
            assert!(error.guidance().contains("git checkout --merge"));
        }
        other => panic!("{other:?}"),
    }
    // 정상 적용은 두 단위를 모두 쓰고 add합니다.
    assert_eq!(crate::test_support::file_digest(&root), before);
    let mut fresh = ConflictSession::open(&git, &root).unwrap();
    fresh.resolve(0, Resolution::ChooseTheirs).unwrap();
    let report = apply_session(&git, &DiskFiles, &fresh).unwrap();
    assert_eq!(report.written.len(), writes);
    assert!(git.entries.borrow().is_empty());
    let opened = crate::snapshot::open_snapshot(&root).unwrap();
    assert_eq!(opened.revision().generation(), 3);
}
