use super::*;
use std::cell::RefCell;

use super::super::file_merge::tests::{card, edited, resized, AR, CHILD};
use super::super::git_port::StageBlob;
use crate::test_support::TempDir;
use stellar_project::document_io::component_source::add_frame;
use stellar_semantic_diff::core_types::value::Size2;

/// 메모리 Git: unmerged 목록과 blob을 돌려주고 호출을 기록합니다. `add_fails`면 add가 실패합니다.
#[derive(Default)]
pub(crate) struct MemoryGit {
    pub(crate) entries: RefCell<Vec<UnmergedEntry>>,
    pub(crate) blobs: RefCell<BTreeMap<String, Vec<u8>>>,
    pub(crate) calls: RefCell<Vec<String>>,
    pub(crate) add_fails: bool,
    pub(crate) operation: Option<GitOperation>,
}

impl MemoryGit {
    /// 경로 하나의 세 stage(빈 bytes = 그 stage 없음).
    pub(crate) fn conflict(&self, path: &str, stages: [&[u8]; 3]) {
        let mut entry = UnmergedEntry {
            path: PathBuf::from(path),
            stages: [None, None, None],
        };
        for (index, bytes) in stages.into_iter().enumerate() {
            if bytes.is_empty() {
                continue;
            }
            let oid = format!("{path}#{index}");
            self.blobs.borrow_mut().insert(oid.clone(), bytes.to_vec());
            entry.stages[index] = Some(StageBlob {
                mode: "100644".into(),
                oid,
            });
        }
        self.entries.borrow_mut().push(entry);
    }
}

impl MergeGit for MemoryGit {
    fn unmerged(&self, _: &Path) -> Result<Vec<UnmergedEntry>, GitPortError> {
        self.calls.borrow_mut().push("unmerged".into());
        Ok(self.entries.borrow().clone())
    }
    fn blob(&self, _: &Path, oid: &str, _: u64) -> Result<Vec<u8>, GitPortError> {
        self.blobs
            .borrow()
            .get(oid)
            .cloned()
            .ok_or_else(|| GitPortError(format!("blob {oid} 없음")))
    }
    fn add(&self, _: &Path, paths: &[PathBuf]) -> Result<(), GitPortError> {
        self.calls.borrow_mut().push(format!("add {}", paths.len()));
        if self.add_fails {
            return Err(GitPortError("index.lock".into()));
        }
        let mut entries = self.entries.borrow_mut();
        entries.retain(|e| !paths.contains(&e.path));
        Ok(())
    }
    fn operation(&self, _: &Path) -> Result<GitOperation, GitPortError> {
        self.calls.borrow_mut().push("operation".into());
        Ok(self.operation.unwrap_or(GitOperation::Merge))
    }
    fn local_config(&self, _: &Path, _: &str) -> Result<Option<String>, GitPortError> {
        Ok(None)
    }
    fn set_local_config(&self, _: &Path, key: &str, _: &str) -> Result<(), GitPortError> {
        self.calls.borrow_mut().push(format!("config {key}"));
        Ok(())
    }
}

/// 컴포넌트 원본 conflict 하나(폭), clean하게 합쳐지는 원본 하나는 없음, raw README, Stellar manifest 경로.
pub(crate) fn fixture(dir: &TempDir) -> (MemoryGit, PathBuf) {
    let (relative, path) = card(&dir.path);
    let base = std::fs::read(&path).unwrap();
    let ours = resized(&path, 144.0, 60.0);
    let theirs = edited(&path, |path, hash| {
        let (_, hash) = add_frame(
            path,
            hash,
            AR.parse().unwrap(),
            CHILD.parse().unwrap(),
            "Badge".into(),
            Size2::new(10.0, 10.0).unwrap(),
        )
        .unwrap();
        stellar_project::document_io::component_source::resize(
            path,
            hash,
            Size2::new(128.0, 60.0).unwrap(),
        )
        .unwrap();
    });
    // 작업 트리에는 driver가 남긴 ours bytes가 있습니다.
    std::fs::write(&path, &ours).unwrap();
    let git = MemoryGit::default();
    git.conflict(relative.to_str().unwrap(), [&base, &ours, &theirs]);
    git.conflict("README.md", [b"a\n", b"b\n", b"c\n"]);
    git.conflict("stellar.yaml", [b"x", b"y", b"z"]);
    (git, relative)
}

#[test]
fn conflict_paths_map_to_semantic_objects() {
    let dir = TempDir::new("session-mapping");
    let (git, component) = fixture(&dir);
    let session = ConflictSession::open(&git, &dir.path).unwrap();
    assert_eq!(session.operation(), GitOperation::Merge);
    assert_eq!(session.raw_conflicts(), &[PathBuf::from("README.md")]);
    assert_eq!(session.unsupported().len(), 1);
    assert_eq!(session.unsupported()[0].0, PathBuf::from("stellar.yaml"));
    assert_eq!(session.units().len(), 1);
    let unit = &session.units()[0];
    assert_eq!(unit.kind(), UnitKind::ComponentSource);
    assert_eq!(unit.path(), component.as_path());
    // raw 경로 → 객체: 정의, root, theirs가 더한 자식(세 snapshot 합집합).
    let a: StableId = super::super::file_merge::tests::A.parse().unwrap();
    let root: StableId = AR.parse().unwrap();
    let child: StableId = CHILD.parse().unwrap();
    let mapping = session.objects_by_path();
    assert_eq!(
        mapping[component.as_path()]
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        {
            let mut ids = vec![a, root, child];
            ids.sort();
            ids
        }
    );
    // semantic conflict → raw 경로: 폭 conflict는 root Frame의 속성이고 그 원본 파일로 돌아갑니다.
    let conflicts = session.conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].index, 0);
    assert_eq!(conflicts[0].conflict.object, root);
    assert_eq!(conflicts[0].unit.path(), component.as_path());
    assert_eq!(session.paths_for(root), vec![component.as_path()]);
    assert_eq!(session.paths_for(child), vec![component.as_path()]);
    assert!(session.paths_for(StableId::NIL).is_empty());
    // 자동 병합된 자식 추가는 C에 있고 출처는 B(theirs)입니다.
    let candidate = unit.candidate();
    assert!(candidate.snapshot().contains(child));
    assert_eq!(
        candidate.provenance(
            child,
            &stellar_semantic_merge::model::ConflictTarget::Object
        ),
        stellar_semantic_merge::candidate::Source::B
    );
    assert_eq!(
        git.calls.borrow().as_slice(),
        ["unmerged", "operation"],
        "읽기만"
    );
}

#[test]
fn unreadable_stage_is_reported_not_merged() {
    let dir = TempDir::new("session-unreadable");
    let (relative, path) = card(&dir.path);
    let base = std::fs::read(&path).unwrap();
    let git = MemoryGit::default();
    let marker = [b"<".repeat(7), b" ours\n".to_vec()].concat();
    git.conflict(relative.to_str().unwrap(), [&base, &marker, &base]);
    let session = ConflictSession::open(&git, &dir.path).unwrap();
    assert!(session.units().is_empty());
    assert_eq!(session.unsupported().len(), 1);
}
