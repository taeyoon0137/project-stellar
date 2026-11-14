//! Stage candidate 임시 project tree writer (XIV.5.5).
//!
//! 선택 staging은 작업 트리를 stage 목적으로 수정하지 않습니다(아키텍처 §26.3). 대신 다음 Index 상태가 될
//! **candidate project tree**를 소유자 전용 임시 디렉터리에 씁니다. 이후 단계(index tree와의 file-level diff,
//! blob 기록, index 갱신)가 이 tree를 입력으로 씁니다.
//!
//! | 경로 | candidate 내용 |
//! |---|---|
//! | non-Stellar 파일(XIV.4.4 분류) | 항상 현재 **index**의 blob·mode 그대로(작업 트리 수정분은 넣지 않음) |
//! | Stellar 파일 | 호출자(serializer)가 준 [`CandidateFile`] 목록이 전부입니다 — index 유지·작업 트리 파일·새 bytes |
//! | 목록에 없는 index의 Stellar 파일 | candidate에 없음(삭제) |
//!
//! - **작업 트리 불변**: 작업 트리는 [`CandidateFile::FromWorkingTree`]의 읽기(symlink를 따르지 않는 안전 읽기,
//!   XIV.2.7)만 하고 쓰지 않습니다. 쓰기는 모두 `parent` 아래 새 `0700` 디렉터리에서 일어나며 `.git`을 만들지
//!   않습니다(XIV.2.6 materialize 규칙: 먼저 모든 경로 검증, `create_new`, 상위 symlink 거부, 실패 시 전체 삭제).
//! - **index 불변**: index는 `ls-files --stage`·`cat-file --batch`로 읽기만 합니다. unmerged 항목이 있으면 conflict
//!   흐름(XIV.11) 대상이므로 거부합니다.
//! - submodule(gitlink)은 내용이 다른 저장소라 쓰지 않고 [`CandidateTree::preserved_gitlinks`]로 돌려줍니다.
//! - 결과는 경로 bytes 순으로 결정론적입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::classify::{classify_project_path, ProjectFileClass};
use crate::decode::GitPath;
use crate::object::{FileModeKind, ObjectId};
use crate::ops::GitOpError;
use crate::snapshot::blob::{read_blobs, BlobError};
use crate::snapshot::materialize::{materialize_with, MaterializeError, MaterializedSnapshot};
use crate::snapshot::read_index_snapshot;
use crate::snapshot::working_tree::{
    list_working_tree_files, read_working_tree_file, WorkingTreeFileKind,
};
use crate::trust::GatedGitRunner;

/// candidate Stellar 파일 하나(serializer 출력).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateFile {
    /// 현재 index의 같은 경로 항목(blob·mode)을 유지합니다.
    KeepIndex {
        /// 경로.
        path: GitPath,
    },
    /// 작업 트리에 저장된 파일을 그대로 씁니다(종류는 디스크 관찰값, 예: 새 asset).
    FromWorkingTree {
        /// 경로.
        path: GitPath,
    },
    /// serializer가 만든 bytes(예: 새 chunk·manifest·current pointer).
    Bytes {
        /// 경로.
        path: GitPath,
        /// 종류(일반·실행·symlink).
        kind: FileModeKind,
        /// 내용(symlink면 대상 경로 bytes).
        bytes: Vec<u8>,
    },
}

impl CandidateFile {
    /// 경로.
    pub fn path(&self) -> &GitPath {
        match self {
            CandidateFile::KeepIndex { path }
            | CandidateFile::FromWorkingTree { path }
            | CandidateFile::Bytes { path, .. } => path,
        }
    }
}

/// candidate 파일 내용의 출처.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateOrigin {
    /// 현재 index blob.
    Index {
        /// blob ID.
        oid: ObjectId,
    },
    /// 작업 트리 파일.
    WorkingTree,
    /// serializer bytes.
    Bytes,
}

/// candidate tree의 파일 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateTreeFile {
    /// 경로.
    pub path: GitPath,
    /// 종류.
    pub kind: FileModeKind,
    /// 출처.
    pub origin: CandidateOrigin,
}

/// candidate tree writer 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateTreeError {
    /// index 읽기 실패.
    Index(GitOpError),
    /// index에 unmerged 항목이 있음(conflict 흐름 대상).
    Conflicted,
    /// candidate 목록에 non-Stellar 경로가 있음(non-Stellar 파일은 항상 index 그대로).
    NotStellar {
        /// 경로.
        path: GitPath,
    },
    /// 같은 경로가 두 번 있음.
    DuplicatePath {
        /// 경로.
        path: GitPath,
    },
    /// 내용 출처에 그 경로가 없음(index 유지인데 index에 없음, 작업 트리 파일인데 디스크에 없음 등).
    MissingSource {
        /// 경로.
        path: GitPath,
    },
    /// 쓸 수 없는 종류(tree·gitlink·없음).
    UnsupportedKind {
        /// 경로.
        path: GitPath,
    },
    /// 작업 트리 파일 읽기 실패.
    WorkingTreeRead {
        /// 경로.
        path: GitPath,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// index blob 읽기 실패.
    Blob(BlobError),
    /// 임시 tree 쓰기 실패(경로 검증·충돌·io).
    Materialize(MaterializeError),
}

impl std::fmt::Display for CandidateTreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |p: &GitPath| p.text().display().to_string();
        match self {
            CandidateTreeError::Index(e) => write!(f, "{e}"),
            CandidateTreeError::Conflicted => {
                f.write_str("병합되지 않은 index 항목이 있어 stage candidate를 만들 수 없습니다")
            }
            CandidateTreeError::NotStellar { path } => {
                write!(
                    f,
                    "Stellar 파일이 아닌 경로는 candidate에 넣을 수 없습니다: {:?}",
                    show(path)
                )
            }
            CandidateTreeError::DuplicatePath { path } => {
                write!(f, "candidate 경로 중복: {:?}", show(path))
            }
            CandidateTreeError::MissingSource { path } => {
                write!(f, "candidate 내용 출처에 경로가 없습니다: {:?}", show(path))
            }
            CandidateTreeError::UnsupportedKind { path } => {
                write!(f, "candidate에 쓸 수 없는 종류: {:?}", show(path))
            }
            CandidateTreeError::WorkingTreeRead { path, kind } => {
                write!(f, "작업 트리 파일 읽기 실패 {:?}: {kind:?}", show(path))
            }
            CandidateTreeError::Blob(e) => write!(f, "{e}"),
            CandidateTreeError::Materialize(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CandidateTreeError {}

/// 임시 디렉터리에 쓴 candidate project tree(drop 시 삭제).
#[derive(Debug)]
pub struct CandidateTree {
    snapshot: MaterializedSnapshot,
    files: Vec<CandidateTreeFile>,
    preserved_gitlinks: Vec<GitPath>,
}

impl CandidateTree {
    /// candidate tree root(`.git` 없음).
    pub fn root(&self) -> &Path {
        self.snapshot.root()
    }

    /// candidate 파일(경로 bytes 순).
    pub fn files(&self) -> &[CandidateTreeFile] {
        &self.files
    }

    /// index에 있으나 쓰지 않은 submodule 경로(index 그대로 유지 대상).
    pub fn preserved_gitlinks(&self) -> &[GitPath] {
        &self.preserved_gitlinks
    }
}

fn working_kind(kind: WorkingTreeFileKind) -> FileModeKind {
    match kind {
        WorkingTreeFileKind::Regular => FileModeKind::Regular,
        WorkingTreeFileKind::Executable => FileModeKind::Executable,
        WorkingTreeFileKind::Symlink => FileModeKind::Symlink,
    }
}

/// 현재 index와 serializer의 Stellar 파일 목록으로 candidate project tree를 `parent` 아래 새 임시 디렉터리에
/// 씁니다(작업 트리·index 불변, trust gate 적용).
pub fn write_candidate_tree(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    stellar_files: &[CandidateFile],
) -> Result<CandidateTree, CandidateTreeError> {
    // 1) 목록 검증(Git 실행·파일 생성 전).
    let mut seen = BTreeSet::new();
    for file in stellar_files {
        let path = file.path();
        if classify_project_path(path.as_bytes()) == ProjectFileClass::NonStellar {
            return Err(CandidateTreeError::NotStellar { path: path.clone() });
        }
        if !seen.insert(path.clone()) {
            return Err(CandidateTreeError::DuplicatePath { path: path.clone() });
        }
        if let CandidateFile::Bytes { kind, .. } = file {
            if !matches!(
                kind,
                FileModeKind::Regular | FileModeKind::Executable | FileModeKind::Symlink
            ) {
                return Err(CandidateTreeError::UnsupportedKind { path: path.clone() });
            }
        }
    }

    // 2) index 읽기.
    let index = read_index_snapshot(git, worktree).map_err(CandidateTreeError::Index)?;
    if index.has_conflicts() {
        return Err(CandidateTreeError::Conflicted);
    }
    let index_entries: BTreeMap<&GitPath, _> = index.entries.iter().map(|e| (&e.path, e)).collect();

    // 3) 최종 파일 목록: non-Stellar index 항목 + candidate Stellar 목록.
    let mut files: BTreeMap<GitPath, CandidateTreeFile> = BTreeMap::new();
    let mut preserved_gitlinks = Vec::new();
    let mut bytes_of: BTreeMap<GitPath, Vec<u8>> = BTreeMap::new();
    for entry in &index.entries {
        if classify_project_path(entry.path.as_bytes()) != ProjectFileClass::NonStellar {
            continue;
        }
        if entry.is_gitlink() {
            preserved_gitlinks.push(entry.path.clone());
            continue;
        }
        let kind = entry
            .kind()
            .ok_or_else(|| CandidateTreeError::UnsupportedKind {
                path: entry.path.clone(),
            })?;
        files.insert(
            entry.path.clone(),
            CandidateTreeFile {
                path: entry.path.clone(),
                kind,
                origin: CandidateOrigin::Index {
                    oid: entry.oid.clone(),
                },
            },
        );
    }
    let needs_working_tree = stellar_files
        .iter()
        .any(|f| matches!(f, CandidateFile::FromWorkingTree { .. }));
    let listing = if needs_working_tree {
        Some(list_working_tree_files(git, worktree).map_err(CandidateTreeError::Index)?)
    } else {
        None
    };
    for file in stellar_files {
        let path = file.path().clone();
        let missing = || CandidateTreeError::MissingSource { path: path.clone() };
        let (kind, origin) = match file {
            CandidateFile::KeepIndex { .. } => {
                let entry = index_entries.get(&path).ok_or_else(missing)?;
                if entry.is_gitlink() {
                    return Err(CandidateTreeError::UnsupportedKind { path });
                }
                let kind = entry.kind().ok_or_else(missing)?;
                (
                    kind,
                    CandidateOrigin::Index {
                        oid: entry.oid.clone(),
                    },
                )
            }
            CandidateFile::FromWorkingTree { .. } => {
                let observed = listing
                    .as_ref()
                    .and_then(|l| l.files.iter().find(|f| f.path == path))
                    .ok_or_else(missing)?;
                let bytes = read_working_tree_file(worktree, observed).map_err(|kind| {
                    CandidateTreeError::WorkingTreeRead {
                        path: path.clone(),
                        kind,
                    }
                })?;
                bytes_of.insert(path.clone(), bytes);
                (working_kind(observed.kind), CandidateOrigin::WorkingTree)
            }
            CandidateFile::Bytes { kind, bytes, .. } => {
                bytes_of.insert(path.clone(), bytes.clone());
                (*kind, CandidateOrigin::Bytes)
            }
        };
        files.insert(path.clone(), CandidateTreeFile { path, kind, origin });
    }

    // 4) index blob 일괄 읽기.
    let oids: Vec<ObjectId> = files
        .values()
        .filter_map(|f| match &f.origin {
            CandidateOrigin::Index { oid } => Some(oid.clone()),
            _ => None,
        })
        .collect();
    let blobs = read_blobs(git, worktree, &oids).map_err(CandidateTreeError::Blob)?;

    // 5) 임시 tree 쓰기(검증 먼저, 실패 시 전체 삭제).
    let files: Vec<CandidateTreeFile> = files.into_values().collect();
    let entries: Vec<(GitPath, FileModeKind)> =
        files.iter().map(|f| (f.path.clone(), f.kind)).collect();
    let snapshot = materialize_with(&entries, parent, |index, path, _| {
        let file = &files[index];
        match &file.origin {
            CandidateOrigin::Index { oid } => blobs
                .get(oid)
                .cloned()
                .ok_or_else(|| MaterializeError::Blob(BlobError::Missing(oid.clone()))),
            _ => bytes_of.get(path).cloned().ok_or(MaterializeError::Io {
                path: Some(path.clone()),
                kind: std::io::ErrorKind::NotFound,
            }),
        }
    })
    .map_err(CandidateTreeError::Materialize)?;
    Ok(CandidateTree {
        snapshot,
        files,
        preserved_gitlinks,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::GitFixture;
    use std::path::PathBuf;

    fn files_under(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for item in std::fs::read_dir(&dir).unwrap() {
                let item = item.unwrap();
                let path = item.path();
                if path.file_name() == Some(std::ffi::OsStr::new(".git")) {
                    continue;
                }
                if item.file_type().unwrap().is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    out.push((relative, std::fs::read(&path).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    fn p(path: &str) -> GitPath {
        GitPath::from_bytes(path.as_bytes())
    }

    /// commit된 프로젝트 + 작업 트리 미stage 변경.
    fn project() -> Option<GitFixture> {
        let fx = GitFixture::with_system_git("candidate-tree").unwrap()?;
        for (path, body) in [
            ("README.md", &b"# P\n"[..]),
            (".gitignore", b"/.stellar/\n"),
            ("stellar.yaml", b"schema_version: 1\n"),
            ("design/current.stellarref", b"gen-1\n"),
            ("design/objects/aa/one.starchunk", b"CHUNK:1"),
            ("assets/sha256/aa/logo.png", b"\x89PNG-1"),
        ] {
            fx.write(path, body).unwrap();
        }
        fx.commit_all("base").unwrap();
        for (path, body) in [
            ("README.md", &b"# P\n\nwork in progress\n"[..]),
            ("design/current.stellarref", b"gen-2\n"),
            ("design/objects/bb/two.starchunk", b"CHUNK:2"),
            ("assets/sha256/bb/photo.png", b"\x89PNG-2"),
        ] {
            fx.write(path, body).unwrap();
        }
        Some(fx)
    }

    #[test]
    fn candidate_tree_is_written_without_touching_the_working_tree_or_index() {
        let Some(fx) = project() else {
            return; // git 미설치 환경 건너뜀.
        };
        let gate = fx.gated();
        let parent = fx.root().join("candidates");
        std::fs::create_dir_all(&parent).unwrap();
        let worktree_before = files_under(fx.work());
        let index_before = read_index_snapshot(&gate, fx.work()).unwrap();

        // serializer 출력 모사: 매니페스트·chunk 1·logo 유지, pointer·chunk 2는 새 bytes, photo는 작업 트리.
        let candidate = [
            CandidateFile::KeepIndex {
                path: p("stellar.yaml"),
            },
            CandidateFile::Bytes {
                path: p("design/current.stellarref"),
                kind: FileModeKind::Regular,
                bytes: b"gen-2\n".to_vec(),
            },
            CandidateFile::KeepIndex {
                path: p("design/objects/aa/one.starchunk"),
            },
            CandidateFile::Bytes {
                path: p("design/objects/bb/two.starchunk"),
                kind: FileModeKind::Regular,
                bytes: b"CHUNK:2".to_vec(),
            },
            CandidateFile::FromWorkingTree {
                path: p("assets/sha256/bb/photo.png"),
            },
        ];
        let tree = write_candidate_tree(&gate, fx.work(), &parent, &candidate).unwrap();
        let written = files_under(tree.root());
        let expected: Vec<(PathBuf, Vec<u8>)> = vec![
            (PathBuf::from(".gitignore"), b"/.stellar/\n".to_vec()),
            // non-Stellar 파일은 작업 트리 수정분이 아니라 index 내용입니다.
            (PathBuf::from("README.md"), b"# P\n".to_vec()),
            (
                PathBuf::from("assets/sha256/bb/photo.png"),
                b"\x89PNG-2".to_vec(),
            ),
            (
                PathBuf::from("design/current.stellarref"),
                b"gen-2\n".to_vec(),
            ),
            (
                PathBuf::from("design/objects/aa/one.starchunk"),
                b"CHUNK:1".to_vec(),
            ),
            (
                PathBuf::from("design/objects/bb/two.starchunk"),
                b"CHUNK:2".to_vec(),
            ),
            (
                PathBuf::from("stellar.yaml"),
                b"schema_version: 1\n".to_vec(),
            ),
        ];
        assert_eq!(written, expected);
        // 목록에 없는 index Stellar 파일(logo)은 candidate에서 빠집니다.
        assert!(!tree.root().join("assets/sha256/aa/logo.png").exists());
        assert!(!tree.root().join(".git").exists());
        let origins: Vec<(&[u8], &CandidateOrigin)> = tree
            .files()
            .iter()
            .map(|f| (f.path.as_bytes(), &f.origin))
            .collect();
        assert!(matches!(
            origins[0],
            (b".gitignore", CandidateOrigin::Index { .. })
        ));
        assert_eq!(
            origins[2],
            (
                &b"assets/sha256/bb/photo.png"[..],
                &CandidateOrigin::WorkingTree
            )
        );
        assert_eq!(
            origins[3],
            (&b"design/current.stellarref"[..], &CandidateOrigin::Bytes)
        );
        // 작업 트리 bytes·index는 write 전후 동일합니다.
        assert_eq!(files_under(fx.work()), worktree_before);
        assert_eq!(read_index_snapshot(&gate, fx.work()).unwrap(), index_before);
        let root = tree.root().to_path_buf();
        drop(tree);
        assert!(!root.exists(), "drop 시 candidate tree 삭제");
    }

    #[test]
    fn invalid_candidate_lists_are_rejected_before_writing() {
        let Some(fx) = project() else {
            return;
        };
        let gate = fx.gated();
        let parent = fx.root().join("candidates");
        std::fs::create_dir_all(&parent).unwrap();
        let run = |files: &[CandidateFile]| write_candidate_tree(&gate, fx.work(), &parent, files);
        assert_eq!(
            run(&[CandidateFile::FromWorkingTree {
                path: p("README.md")
            }])
            .unwrap_err(),
            CandidateTreeError::NotStellar {
                path: p("README.md")
            }
        );
        let twice = CandidateFile::KeepIndex {
            path: p("stellar.yaml"),
        };
        assert_eq!(
            run(&[twice.clone(), twice]).unwrap_err(),
            CandidateTreeError::DuplicatePath {
                path: p("stellar.yaml")
            }
        );
        assert_eq!(
            run(&[CandidateFile::KeepIndex {
                path: p("design/objects/bb/two.starchunk")
            }])
            .unwrap_err(),
            CandidateTreeError::MissingSource {
                path: p("design/objects/bb/two.starchunk")
            }
        );
        assert_eq!(
            run(&[CandidateFile::FromWorkingTree {
                path: p("assets/none.png")
            }])
            .unwrap_err(),
            CandidateTreeError::MissingSource {
                path: p("assets/none.png")
            }
        );
        assert_eq!(
            run(&[CandidateFile::Bytes {
                path: p("design/x"),
                kind: FileModeKind::Tree,
                bytes: Vec::new(),
            }])
            .unwrap_err(),
            CandidateTreeError::UnsupportedKind {
                path: p("design/x")
            }
        );
        assert!(matches!(
            run(&[CandidateFile::Bytes {
                path: p("design/.git/config"),
                kind: FileModeKind::Regular,
                bytes: Vec::new(),
            }])
            .unwrap_err(),
            CandidateTreeError::Materialize(MaterializeError::UnsafePath { .. })
        ));
        // 거부된 시도는 임시 디렉터리를 남기지 않습니다.
        assert!(std::fs::read_dir(&parent).unwrap().next().is_none());
    }

    #[test]
    fn untrusted_candidate_tree_spawns_nothing() {
        use crate::runner::GitRunner;
        use crate::trust::{GatedRunError, GitTrust};
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        let result = write_candidate_tree(&gate, &std::env::temp_dir(), &std::env::temp_dir(), &[]);
        assert!(matches!(
            result,
            Err(CandidateTreeError::Index(GitOpError::Gate(
                GatedRunError::ProjectNotTrusted
            )))
        ));
        assert_eq!(runner.spawn_count(), 0);
    }
}
