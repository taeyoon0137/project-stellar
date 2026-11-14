//! 후보 원시 bytes를 Git blob으로 기록합니다(XIV.6.1). index·작업 트리는 변경하지 않습니다.

use std::path::Path;

use crate::candidate_delta::{CandidateDelta, FileDelta};
use crate::decode::GitPath;
use crate::object::{FileMode, ObjectId};
use crate::ops::{run_ok_with_input, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::snapshot::IndexSnapshot;
use crate::trust::GatedGitRunner;

/// blob 기록이 끝난 파일 변경. index 반영은 아직 하지 않은 값입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrittenFileDelta {
    /// mode와 기록된 blob OID.
    Upsert {
        /// 저장소 상대 경로.
        path: GitPath,
        /// Git mode.
        mode: FileMode,
        /// 검증된 blob OID.
        oid: ObjectId,
    },
    /// 삭제에는 blob 기록이 필요하지 않습니다.
    Remove {
        /// 저장소 상대 경로.
        path: GitPath,
    },
}

/// 실제 index 설치 전 상태. 기준 index를 후속 경합 검사에 넘깁니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenCandidate {
    base: IndexSnapshot,
    changes: Vec<WrittenFileDelta>,
}

impl WrittenCandidate {
    #[cfg(test)]
    pub(crate) fn for_test(base: IndexSnapshot, changes: Vec<WrittenFileDelta>) -> Self {
        Self { base, changes }
    }

    /// 후보 생성 당시의 index.
    pub fn base_index(&self) -> &IndexSnapshot {
        &self.base
    }

    /// 경로 순 변경 목록.
    pub fn changes(&self) -> &[WrittenFileDelta] {
        &self.changes
    }
}

/// 변경된 blob만 기록합니다. 동일 후보와 삭제는 Git object 기록 0회입니다.
/// Git의 index·작업 트리는 변경하지 않습니다. 반환값은 index 설치 승인이나 성공을 뜻하지 않습니다.
/// 도중 실패하면 부분 결과를 반환하지 않지만 이미 기록한 미참조 object는 남을 수 있습니다.
pub fn write_candidate_blobs(
    git: &GatedGitRunner,
    worktree: &Path,
    delta: &CandidateDelta,
) -> Result<WrittenCandidate, GitOpError> {
    git.runner()?;
    let mut changes = Vec::with_capacity(delta.changes().len());
    for change in delta.changes() {
        changes.push(match change {
            FileDelta::Upsert { path, mode, bytes } => WrittenFileDelta::Upsert {
                path: path.clone(),
                mode: *mode,
                oid: write_blob(git, worktree, bytes)?,
            },
            FileDelta::Remove { path } => WrittenFileDelta::Remove { path: path.clone() },
        });
    }
    Ok(WrittenCandidate {
        base: delta.base_index().clone(),
        changes,
    })
}

/// Git이 출력한 단일 object ID를 검증합니다(null·추가 레코드·공백 거부).
pub(crate) fn parse_written_oid(command: &'static str, raw: &[u8]) -> Result<ObjectId, GitOpError> {
    raw.strip_suffix(b"\n")
        .and_then(ObjectId::from_hex_bytes)
        .filter(|oid| !oid.is_null())
        .ok_or_else(|| {
            ParseFailure::new(
                command,
                "단일 non-null object ID가 아닙니다",
                &raw[..raw.len().min(128)],
            )
            .into()
        })
}

/// `hash-object -w --stdin` 1회로 원시 blob을 기록합니다. 경로·filter·shell은 사용하지 않습니다.
/// 실패 전에 쓴 object는 미참조 상태로 남을 수 있으며 삭제하거나 rollback하지 않습니다.
pub fn write_blob(
    git: &GatedGitRunner,
    worktree: &Path,
    bytes: &[u8],
) -> Result<ObjectId, GitOpError> {
    let output = run_ok_with_input(
        git,
        worktree,
        &argv(["hash-object", "-w", "--stdin"]),
        bytes,
    )?;
    parse_written_oid("hash-object -w --stdin", &output.stdout)
}

#[cfg(all(test, unix))]
mod candidate_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_output_is_one_non_null_sha1_or_sha256_record() {
        for len in [40, 64] {
            assert!(
                parse_written_oid("hash-object", format!("{}\n", "a".repeat(len)).as_bytes())
                    .is_ok()
            );
        }
        for raw in [
            Vec::new(),
            format!("{}\n", "0".repeat(40)).into_bytes(),
            format!("{}\n", "A".repeat(40)).into_bytes(),
            "a".repeat(40).into_bytes(),
            format!("{}\nextra\n", "a".repeat(40)).into_bytes(),
        ] {
            assert!(parse_written_oid("hash-object", &raw).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_git_writes_once_and_preserves_binary_bytes_and_index() {
        use crate::fixture::GitFixture;
        use crate::snapshot::read_index_snapshot;
        let fx = GitFixture::with_system_git("blob-write")
            .unwrap()
            .expect("system Git required");
        fx.write("unrelated", b"staged\n").unwrap();
        fx.git(["add", "unrelated"]).unwrap();
        let gate = fx.gated();
        let before = read_index_snapshot(&gate, fx.work()).unwrap();
        let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
        for bytes in [b"".as_slice(), b"\0\xff\r\n$(literal);\0\n"] {
            let count = gate.spawn_count();
            let oid = write_blob(&gate, fx.work(), bytes).unwrap();
            assert_eq!(gate.spawn_count() - count, 1);
            let read = fx.git(["cat-file", "blob", oid.as_str()]).unwrap();
            assert_eq!(read.stdout, bytes);
        }
        assert_eq!(read_index_snapshot(&gate, fx.work()).unwrap(), before);
        assert_eq!(
            std::fs::read(fx.work().join(".git/index")).unwrap(),
            index_bytes
        );
        assert_eq!(
            std::fs::read(fx.work().join("unrelated")).unwrap(),
            b"staged\n"
        );
    }

    #[test]
    fn untrusted_write_never_spawns() {
        use crate::runner::GitRunner;
        use crate::trust::{GatedRunError, GitTrust};
        let gate = GatedGitRunner::new(
            GitRunner::new(
                "/nonexistent/git".into(),
                Vec::new(),
                std::time::Duration::from_secs(1),
            ),
            GitTrust::NotTrusted,
        );
        assert_eq!(
            write_blob(&gate, Path::new("/"), b"x"),
            Err(GitOpError::Gate(GatedRunError::ProjectNotTrusted))
        );
        assert_eq!(gate.spawn_count(), 0);
    }
}
