//! Git snapshot 원천과 provider (XIV.2.3~).
//!
//! Semantic Diff·Staging·Merge가 비교하는 프로젝트 상태는 [`SnapshotSource`] 중 하나에서 옵니다.
//!
//! | 원천 | 의미 | 변하는가 |
//! |---|---|---|
//! | [`SnapshotSource::Head`] | 읽는 시점의 HEAD commit(의 tree) | HEAD가 움직이면 바뀜 |
//! | [`SnapshotSource::Index`] | 현재 Git index(stage 0) | stage할 때 바뀜 |
//! | [`SnapshotSource::WorkingTree`] | 디스크에 **저장된** 작업 트리 파일 | 저장할 때 바뀜 |
//! | [`SnapshotSource::Commit`] | 지정한 commit(의 tree) | 불변 |
//! | [`SnapshotSource::Tree`] | 지정한 tree | 불변 |
//!
//! 원천은 텍스트 spec(`head`, `index`, `worktree`, `commit:<oid>`, `tree:<oid>`)으로 왕복 변환할 수
//! 있습니다. object ID는 검증된 [`ObjectId`](소문자 16진수 40·64자)만 허용합니다.

use crate::decode::GitPath;
use crate::object::{FileMode, FileModeKind, ObjectId};
use crate::ops::GitOpError;

/// HEAD·commit·tree 파일 목록 provider(XIV.2.4).
pub mod tree;

pub use tree::{read_tree_snapshot, resolve_tree, ResolvedTree, TreeSnapshot};

/// Git index provider(XIV.2.5).
pub mod index;

pub use index::{read_index_snapshot, IndexSnapshot, StagedEntry};

/// blob content reader(`git cat-file --batch`, XIV.2.6).
pub mod blob;

/// 임시 virtual project 디렉터리 materialize(XIV.2.6).
pub mod materialize;

pub use blob::{read_blobs, BlobError};
pub use materialize::{materialize_snapshot, MaterializeError, MaterializedSnapshot};

/// 디스크 저장 파일 기준 WorkingTree provider(XIV.2.7).
pub mod working_tree;

pub use working_tree::{
    read_working_tree_snapshot, DiskProjectLoader, SnapshotBasis, WorkingTreeSnapshot,
};

/// snapshot 한 항목(경로·mode·object ID). gitlink(submodule)는 commit ID를 가집니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotEntry {
    /// 저장소 root 기준 경로(원시 바이트, `/` 구분).
    pub path: GitPath,
    /// file mode.
    pub mode: FileMode,
    /// blob ID(gitlink면 submodule commit ID).
    pub oid: ObjectId,
}

impl SnapshotEntry {
    /// mode 종류.
    pub fn kind(&self) -> Option<FileModeKind> {
        self.mode.kind()
    }

    /// submodule gitlink인지(materialize 대상 아님).
    pub fn is_gitlink(&self) -> bool {
        self.mode == FileMode::GITLINK
    }
}

/// snapshot provider 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// Git 실행·해석 실패.
    Op(GitOpError),
    /// provider가 다루지 않는 원천 종류.
    WrongSourceKind {
        /// 기대한 종류.
        expected: SnapshotSourceKind,
        /// 받은 원천.
        source: SnapshotSource,
    },
    /// 지정한 commit·tree가 없거나 종류가 다름.
    ObjectNotFound {
        /// 요청한 원천.
        source: SnapshotSource,
    },
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Op(e) => write!(f, "{e}"),
            SnapshotError::WrongSourceKind { expected, source } => {
                write!(
                    f,
                    "{source} 원천은 {expected:?} provider로 읽을 수 없습니다"
                )
            }
            SnapshotError::ObjectNotFound { source } => {
                write!(f, "{source} object가 없거나 종류가 다릅니다")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<GitOpError> for SnapshotError {
    fn from(e: GitOpError) -> SnapshotError {
        SnapshotError::Op(e)
    }
}

/// snapshot 원천.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SnapshotSource {
    /// 현재 HEAD가 가리키는 commit(읽는 시점에 commit으로 해석, unborn이면 빈 snapshot).
    Head,
    /// 현재 Git index의 stage 0 항목.
    Index,
    /// 디스크에 저장된 작업 트리 파일(저장 대기 메모리 상태는 포함하지 않음).
    WorkingTree,
    /// 지정한 commit.
    Commit(ObjectId),
    /// 지정한 tree.
    Tree(ObjectId),
}

/// 원천 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotSourceKind {
    /// Git object(tree)로 해석되는 원천(Head·Commit·Tree).
    GitTree,
    /// Git index.
    Index,
    /// 작업 트리 파일.
    WorkingTree,
}

/// spec 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpecError {
    /// 알 수 없는 원천 이름.
    UnknownSource(String),
    /// object ID 형식 오류.
    InvalidObjectId(String),
}

impl std::fmt::Display for SourceSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceSpecError::UnknownSource(s) => write!(f, "알 수 없는 snapshot 원천: {s:?}"),
            SourceSpecError::InvalidObjectId(s) => write!(f, "잘못된 object ID: {s:?}"),
        }
    }
}

impl std::error::Error for SourceSpecError {}

impl SnapshotSource {
    /// 원천 종류.
    pub fn kind(&self) -> SnapshotSourceKind {
        match self {
            SnapshotSource::Head | SnapshotSource::Commit(_) | SnapshotSource::Tree(_) => {
                SnapshotSourceKind::GitTree
            }
            SnapshotSource::Index => SnapshotSourceKind::Index,
            SnapshotSource::WorkingTree => SnapshotSourceKind::WorkingTree,
        }
    }

    /// 내용이 바뀌지 않는 원천(지정 commit·tree)인지.
    pub fn is_immutable(&self) -> bool {
        matches!(self, SnapshotSource::Commit(_) | SnapshotSource::Tree(_))
    }

    /// 텍스트 spec(`head`, `index`, `worktree`, `commit:<oid>`, `tree:<oid>`).
    pub fn to_spec(&self) -> String {
        match self {
            SnapshotSource::Head => "head".to_string(),
            SnapshotSource::Index => "index".to_string(),
            SnapshotSource::WorkingTree => "worktree".to_string(),
            SnapshotSource::Commit(oid) => format!("commit:{oid}"),
            SnapshotSource::Tree(oid) => format!("tree:{oid}"),
        }
    }

    /// 텍스트 spec을 해석합니다(공백·대문자·다른 형식은 거부).
    pub fn parse_spec(spec: &str) -> Result<SnapshotSource, SourceSpecError> {
        let oid = |hex: &str| {
            ObjectId::parse(hex).ok_or_else(|| SourceSpecError::InvalidObjectId(hex.to_string()))
        };
        match spec {
            "head" => Ok(SnapshotSource::Head),
            "index" => Ok(SnapshotSource::Index),
            "worktree" => Ok(SnapshotSource::WorkingTree),
            _ => match spec.split_once(':') {
                Some(("commit", hex)) => Ok(SnapshotSource::Commit(oid(hex)?)),
                Some(("tree", hex)) => Ok(SnapshotSource::Tree(oid(hex)?)),
                _ => Err(SourceSpecError::UnknownSource(spec.to_string())),
            },
        }
    }
}

impl std::fmt::Display for SnapshotSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_spec())
    }
}

impl std::str::FromStr for SnapshotSource {
    type Err = SourceSpecError;

    fn from_str(s: &str) -> Result<SnapshotSource, SourceSpecError> {
        SnapshotSource::parse_spec(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    fn all_sources() -> Vec<SnapshotSource> {
        let sha256 = ObjectId::parse(&"c".repeat(64)).unwrap();
        vec![
            SnapshotSource::Head,
            SnapshotSource::Index,
            SnapshotSource::WorkingTree,
            SnapshotSource::Commit(ObjectId::parse(SHA1).unwrap()),
            SnapshotSource::Tree(ObjectId::parse(SHA1).unwrap()),
            SnapshotSource::Commit(sha256.clone()),
            SnapshotSource::Tree(sha256),
        ]
    }

    #[test]
    fn every_source_round_trips_through_spec() {
        for source in all_sources() {
            let spec = source.to_spec();
            assert_eq!(
                SnapshotSource::parse_spec(&spec),
                Ok(source.clone()),
                "{spec}"
            );
            assert_eq!(spec.parse::<SnapshotSource>(), Ok(source.clone()));
            assert_eq!(source.to_string(), spec);
        }
        assert_eq!(
            SnapshotSource::Commit(ObjectId::parse(SHA1).unwrap()).to_spec(),
            format!("commit:{SHA1}")
        );
    }

    #[test]
    fn kinds_and_immutability() {
        use SnapshotSourceKind::*;
        let kinds: Vec<(SnapshotSourceKind, bool)> = all_sources()
            .iter()
            .map(|s| (s.kind(), s.is_immutable()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (GitTree, false),
                (Index, false),
                (WorkingTree, false),
                (GitTree, true),
                (GitTree, true),
                (GitTree, true),
                (GitTree, true),
            ]
        );
    }

    #[test]
    fn invalid_specs_are_rejected() {
        for bad in [
            "",
            "HEAD",
            " head",
            "head ",
            "work-tree",
            "blob:x",
            "commit",
        ] {
            assert!(
                matches!(
                    SnapshotSource::parse_spec(bad),
                    Err(SourceSpecError::UnknownSource(_))
                ),
                "{bad:?}"
            );
        }
        for bad in [
            "commit:",
            "commit:abc",
            &format!("commit:{}", SHA1.to_uppercase()),
            &format!("tree:{SHA1} "),
            &format!("tree:{SHA1}0"),
        ] {
            assert!(
                matches!(
                    SnapshotSource::parse_spec(bad),
                    Err(SourceSpecError::InvalidObjectId(_))
                ),
                "{bad:?}"
            );
        }
    }
}
