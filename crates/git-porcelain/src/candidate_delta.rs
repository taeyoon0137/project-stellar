//! Stage candidate와 기준 Git index의 file-level diff(XIV.5.6).
//!
//! bytes와 mode가 같은 경로는 결과에서 제외하고, 추가·수정에는 원시 blob bytes, 삭제에는 경로만
//! 돌려줍니다. symlink bytes는 링크 대상이며 gitlink는 항상 index 그대로 유지합니다. Git object 기록,
//! index 갱신, 작업 트리 쓰기는 하지 않습니다. 비교 전후 실제 index가 candidate 기준과 다르면 실패합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::candidate_tree::{CandidateOrigin, CandidateTree, CandidateTreeFile};
use crate::classify::{classify_project_path, ProjectFileClass};
use crate::decode::GitPath;
use crate::object::{FileMode, FileModeKind};
use crate::ops::GitOpError;
use crate::snapshot::blob::{BlobError, BlobLimits, OrderedBlobReader};
use crate::snapshot::materialize::{validate_entries, validate_snapshot_path, MaterializeError};
use crate::snapshot::working_tree::{
    checked_local_path, read_regular_no_follow, read_working_tree_file, WorkingTreeFile,
    WorkingTreeFileKind,
};
use crate::snapshot::{read_index_snapshot, IndexSnapshot};
use crate::trust::GatedGitRunner;

/// 실제 index에 반영할 파일 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDelta {
    /// 새 파일 또는 bytes·mode 변경. symlink의 bytes는 대상 경로입니다.
    Upsert {
        /// 저장소 root 기준 경로.
        path: GitPath,
        /// Git file mode.
        mode: FileMode,
        /// 기록할 원시 blob bytes(filter·textconv 적용 없음).
        bytes: Vec<u8>,
    },
    /// 기준 index에만 있는 파일.
    Remove {
        /// 저장소 root 기준 경로.
        path: GitPath,
    },
}

impl FileDelta {
    /// 변경 경로.
    pub fn path(&self) -> &GitPath {
        match self {
            Self::Upsert { path, .. } | Self::Remove { path } => path,
        }
    }
}

/// 비교 기준과 그 기준에 적용할 변경(경로 bytes 순).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDelta {
    base: IndexSnapshot,
    changes: Vec<FileDelta>,
}

impl CandidateDelta {
    /// 후속 index 설치 단계에서 다시 확인할 기준 index.
    pub fn base_index(&self) -> &IndexSnapshot {
        &self.base
    }

    /// 추가·수정·삭제만 포함한 변경 목록(unchanged 경로 없음).
    pub fn changes(&self) -> &[FileDelta] {
        &self.changes
    }
}

/// file-level diff 실패. 실패 시 부분 delta를 반환하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateDeltaError {
    /// 실제 index 읽기 실패(trust gate 포함).
    Index(GitOpError),
    /// candidate 생성 이후 실제 index가 바뀜(unmerged 상태 포함).
    StaleBase,
    /// index blob 읽기 실패 또는 크기 상한 초과.
    Blob(BlobError),
    /// 경로·종류 검증 실패.
    Materialize(MaterializeError),
    /// candidate 파일 없음, 종류 변경, symlink 상위 경로 또는 읽기 오류.
    CandidateRead {
        /// 경로.
        path: GitPath,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// candidate 파일 하나가 읽기 상한을 넘음.
    CandidateTooLarge {
        /// 경로.
        path: GitPath,
        /// 상한(byte).
        max_bytes: u64,
    },
    /// candidate에서 읽는 bytes 합이 상한을 넘음.
    CandidateTotalTooLarge {
        /// 상한(byte).
        max_bytes: u64,
    },
    /// 항상 보존해야 하는 non-Stellar 파일 또는 gitlink 경로 변경.
    PreservedPathChanged {
        /// 경로.
        path: GitPath,
    },
}

impl std::fmt::Display for CandidateDeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(error) => write!(f, "{error}"),
            Self::StaleBase => {
                f.write_str("candidate 기준과 실제 index가 달라 다시 준비해야 합니다")
            }
            Self::Blob(error) => write!(f, "{error}"),
            Self::Materialize(error) => write!(f, "{error}"),
            Self::CandidateRead { path, kind } => {
                write!(
                    f,
                    "candidate 파일 읽기 실패 {:?}: {kind:?}",
                    path.text().display()
                )
            }
            Self::CandidateTooLarge { path, max_bytes } => write!(
                f,
                "candidate 파일이 상한 {max_bytes} bytes를 넘습니다: {:?}",
                path.text().display()
            ),
            Self::CandidateTotalTooLarge { max_bytes } => {
                write!(f, "candidate 전체 크기가 상한 {max_bytes} bytes를 넘습니다")
            }
            Self::PreservedPathChanged { path } => {
                write!(
                    f,
                    "보존해야 하는 index 경로 변경: {:?}",
                    path.text().display()
                )
            }
        }
    }
}

impl std::error::Error for CandidateDeltaError {}

/// 기본 메모리 상한으로 candidate와 실제 index의 변경을 계산합니다(읽기 전용, trust gate 적용).
pub fn compute_candidate_delta(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &CandidateTree,
) -> Result<CandidateDelta, CandidateDeltaError> {
    compute_candidate_delta_with_limits(git, worktree, candidate, &BlobLimits::IN_MEMORY)
}

/// 명시적 읽기 상한으로 변경을 계산합니다.
///
/// 기준 index의 같은 경로·같은 blob·같은 종류에서 만든 파일(`CandidateOrigin::Index`)은 내용이 같으므로
/// 양쪽 모두 읽지 않고 unchanged로 둡니다. 나머지 index blob은 기존 batch reader로 한 batch만 유지하고,
/// candidate는 한 파일씩 안전하게 읽어 변경된 bytes만 결과에 보관합니다. `max_total_bytes`는 비교 대상인
/// index bytes 합과 candidate bytes 합 각각에 적용합니다. 반환 후 동시 index 변경의 원자적 검증은 설치
/// 단계의 책임입니다.
pub fn compute_candidate_delta_with_limits(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &CandidateTree,
    limits: &BlobLimits,
) -> Result<CandidateDelta, CandidateDeltaError> {
    let base = candidate.base_index();
    if read_index_snapshot(git, worktree).map_err(CandidateDeltaError::Index)? != *base {
        return Err(CandidateDeltaError::StaleBase);
    }
    let entries: Vec<_> = candidate
        .files()
        .iter()
        .map(|file| (file.path.clone(), file.kind))
        .collect();
    validate_entries(&entries).map_err(CandidateDeltaError::Materialize)?;
    for entry in &base.entries {
        validate_snapshot_path(entry.path.as_bytes()).map_err(|reason| {
            CandidateDeltaError::Materialize(MaterializeError::UnsafePath {
                path: entry.path.clone(),
                reason,
            })
        })?;
    }
    let indexed: BTreeMap<_, _> = base.entries.iter().map(|e| (&e.path, e)).collect();
    let paths: BTreeSet<_> = candidate
        .files()
        .iter()
        .map(|file| file.path.clone())
        .collect();
    // gitlink 자신뿐 아니라 그 아래 파일을 추가하거나 상위 파일로 덮는 것도 보존 계약 위반입니다.
    for entry in base.entries.iter().filter(|entry| entry.is_gitlink()) {
        let raw = entry.path.as_bytes();
        let mut prefix = raw.to_vec();
        prefix.push(b'/');
        if paths.contains(&entry.path)
            || raw
                .iter()
                .enumerate()
                .any(|(at, byte)| *byte == b'/' && paths.contains(&GitPath::from_bytes(&raw[..at])))
            || paths
                .range(GitPath::from_bytes(&prefix)..)
                .next()
                .is_some_and(|path| path.as_bytes().starts_with(&prefix))
        {
            return Err(CandidateDeltaError::PreservedPathChanged {
                path: entry.path.clone(),
            });
        }
    }
    // 추적 파일 전체를 읽지 않도록 같은 index blob에서 만든 파일은 비교·상한 계산에서 뺍니다.
    let compared: Vec<&CandidateTreeFile> = candidate
        .files()
        .iter()
        .filter(|file| match &file.origin {
            CandidateOrigin::Index { oid } => !indexed
                .get(&file.path)
                .is_some_and(|entry| entry.oid == *oid && entry.kind() == Some(file.kind)),
            CandidateOrigin::WorkingTree | CandidateOrigin::Bytes => true,
        })
        .collect();
    let oids = compared
        .iter()
        .filter_map(|file| indexed.get(&file.path).map(|entry| entry.oid.clone()))
        .collect();
    let mut blobs =
        OrderedBlobReader::new(git, worktree, oids, limits).map_err(CandidateDeltaError::Blob)?;
    let mut changes = Vec::new();
    let mut total = 0u64;
    for file in compared {
        let bytes = read_candidate_file(candidate.root(), file, limits, total)?;
        total += bytes.len() as u64;
        let mode = indexed
            .get(&file.path)
            .filter(|entry| entry.kind() == Some(file.kind))
            .map(|entry| entry.mode)
            .unwrap_or(match file.kind {
                FileModeKind::Executable => FileMode::EXECUTABLE,
                FileModeKind::Symlink => FileMode::SYMLINK,
                _ => FileMode::REGULAR,
            });
        if let Some(entry) = indexed.get(&file.path) {
            let original = blobs
                .next_content(&entry.oid)
                .map_err(CandidateDeltaError::Blob)?;
            // 같은 종류는 Git의 구버전 regular mode(100664)도 그대로 유지합니다.
            if entry.kind() == Some(file.kind) && original == bytes {
                continue;
            }
        }
        if classify_project_path(file.path.as_bytes()) == ProjectFileClass::NonStellar {
            return Err(CandidateDeltaError::PreservedPathChanged {
                path: file.path.clone(),
            });
        }
        changes.push(FileDelta::Upsert {
            path: file.path.clone(),
            mode,
            bytes,
        });
    }
    for entry in &base.entries {
        if !entry.is_gitlink() && !paths.contains(&entry.path) {
            changes.push(FileDelta::Remove {
                path: entry.path.clone(),
            });
        }
    }
    changes.sort_by(|a, b| a.path().cmp(b.path()));
    if read_index_snapshot(git, worktree).map_err(CandidateDeltaError::Index)? != *base {
        return Err(CandidateDeltaError::StaleBase);
    }
    Ok(CandidateDelta {
        base: base.clone(),
        changes,
    })
}

fn read_candidate_file(
    root: &Path,
    file: &CandidateTreeFile,
    limits: &BlobLimits,
    total: u64,
) -> Result<Vec<u8>, CandidateDeltaError> {
    let io = |kind| CandidateDeltaError::CandidateRead {
        path: file.path.clone(),
        kind,
    };
    let check_size = |size| {
        if size > limits.max_blob_bytes {
            Err(CandidateDeltaError::CandidateTooLarge {
                path: file.path.clone(),
                max_bytes: limits.max_blob_bytes,
            })
        } else if size > limits.max_total_bytes.saturating_sub(total) {
            Err(CandidateDeltaError::CandidateTotalTooLarge {
                max_bytes: limits.max_total_bytes,
            })
        } else {
            Ok(())
        }
    };
    if !std::fs::symlink_metadata(root)
        .map_err(|e| io(e.kind()))?
        .is_dir()
    {
        return Err(io(std::io::ErrorKind::InvalidInput));
    }
    let local = checked_local_path(root, &file.path).map_err(io)?;
    let meta = std::fs::symlink_metadata(&local).map_err(|e| io(e.kind()))?;
    check_size(meta.len())?;
    let bytes = if file.kind == FileModeKind::Symlink {
        read_working_tree_file(
            root,
            &WorkingTreeFile {
                path: file.path.clone(),
                kind: WorkingTreeFileKind::Symlink,
                len: meta.len(),
            },
        )
        .map_err(io)?
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if (meta.permissions().mode() & 0o111 != 0) != (file.kind == FileModeKind::Executable) {
                return Err(io(std::io::ErrorKind::InvalidInput));
            }
        }
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let limit = limits.max_blob_bytes.min(remaining).saturating_add(1);
        read_regular_no_follow(&local, limit).map_err(io)?
    };
    check_size(bytes.len() as u64)?;
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests;
