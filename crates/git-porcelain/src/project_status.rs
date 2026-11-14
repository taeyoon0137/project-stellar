//! 프로젝트 status: raw Git status와 semantic status의 실패 분리 (XIV.4.5).
//!
//! Stellar 프로젝트의 status는 두 층입니다.
//!
//! 1. **raw Git status**: `git status --porcelain=v2 -z --branch` 결과([`RepositoryStatus`], non-Stellar
//!    raw 항목, Stellar 파일 단위 상태). Git이 디스크 파일을 읽어 계산하므로 프로젝트 내용이 손상돼도
//!    계산됩니다.
//! 2. **semantic status**: HEAD·Index·WorkingTree 프로젝트를 읽어 의미 변경을 계산합니다(semantic-diff
//!    `status`). 프로젝트 loader가 필요하므로 손상된 chunk·manifest가 있으면 실패할 수 있습니다.
//!
//! [`read_project_status`]는 raw status를 **먼저** 읽고, 그 다음 호출자가 공급한
//! [`SemanticStatusProvider`]를 실행합니다. provider 실패는 [`ProjectStatus::semantic`]에 `Err`로 그대로
//! 담기며 raw status는 계속 제공됩니다. 이 함수와 provider 계약은 **어떤 파일도 쓰지 않습니다** — 손상을
//! 자동으로 고치지 않고(작업 트리·index 불변), 복구는 사용자가 진단을 보고 결정합니다. raw status 자체가
//! 실패하면(Git 실행·해석 실패) provider는 실행하지 않습니다.
//!
//! git-porcelain은 crate 그래프상 프로젝트 loader·semantic-diff에 의존할 수 없으므로 semantic 계산은
//! trait로 받습니다(XIV.2.7 [`DiskProjectLoader`](crate::snapshot::DiskProjectLoader)와 같은 방식).

use std::path::Path;

use crate::classify::{partition_status, RawFileStatus};
use crate::file_status::{stellar_file_statuses, StellarFileStatus};
use crate::ops::GitOpError;
use crate::repo_status::{read_in_progress_operations, RepositoryStatus};
use crate::snapshot::SnapshotBasis;
use crate::status::{read_status, StatusOptions};
use crate::trust::GatedGitRunner;

/// semantic status 계산기(호출자가 프로젝트 loader·semantic-diff로 구현).
pub trait SemanticStatusProvider {
    /// semantic status 표현.
    type Status;
    /// 실패(손상된 프로젝트 등).
    type Error;

    /// raw status를 읽은 뒤 호출됩니다. `stellar_files`는 이번 status의 Stellar 파일 단위 상태입니다.
    ///
    /// 구현은 저장소·작업 트리를 **수정하지 않아야** 합니다(HEAD·Index는 임시 디렉터리에 materialize).
    fn semantic_status(
        &self,
        git: &GatedGitRunner,
        worktree: &Path,
        stellar_files: &[StellarFileStatus],
    ) -> Result<Self::Status, Self::Error>;
}

/// raw status와 semantic status 결과.
#[derive(Debug)]
pub struct ProjectStatus<S, E> {
    /// 기준(디스크에 저장된 파일, XIV.2.7).
    pub basis: SnapshotBasis,
    /// 저장소 요약(HEAD·upstream·변경 수·conflict).
    pub repository: RepositoryStatus,
    /// non-Stellar 파일 raw status(XIV.4.4).
    pub raw: Vec<RawFileStatus>,
    /// 경계를 넘는 rename/copy(raw 표시).
    pub cross_boundary: Vec<RawFileStatus>,
    /// Stellar 파일 단위 상태(XIV.4.3).
    pub stellar_files: Vec<StellarFileStatus>,
    /// semantic status. 실패해도 위 raw 정보는 유효합니다.
    pub semantic: Result<S, E>,
}

impl<S, E> ProjectStatus<S, E> {
    /// semantic status가 실패했는지.
    pub fn semantic_failed(&self) -> bool {
        self.semantic.is_err()
    }
}

/// raw status를 읽고 semantic status를 계산합니다(trust gate 적용, 읽기 전용).
///
/// raw status 실패만 `Err`입니다. semantic 실패는 [`ProjectStatus::semantic`]에 담깁니다.
pub fn read_project_status<P: SemanticStatusProvider>(
    git: &GatedGitRunner,
    worktree: &Path,
    provider: &P,
) -> Result<ProjectStatus<P::Status, P::Error>, GitOpError> {
    let report = read_status(git, worktree, StatusOptions::default())?;
    let operations = read_in_progress_operations(git, worktree)?;
    let repository = RepositoryStatus::from_report(&report, operations)?;
    let partition = partition_status(&report);
    let stellar_files = stellar_file_statuses(&partition.stellar);
    let semantic = provider.semantic_status(git, worktree, &stellar_files);
    Ok(ProjectStatus {
        basis: repository.basis(),
        repository,
        raw: partition.raw,
        cross_boundary: partition.cross_boundary,
        stellar_files,
        semantic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::GitRunner;
    use crate::trust::{GatedRunError, GitTrust};
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::time::Duration;

    /// 호출 여부를 세는 provider.
    struct Counting(Cell<u32>);

    impl SemanticStatusProvider for Counting {
        type Status = ();
        type Error = ();

        fn semantic_status(
            &self,
            _git: &GatedGitRunner,
            _worktree: &Path,
            _files: &[StellarFileStatus],
        ) -> Result<(), ()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn untrusted_project_status_spawns_nothing_and_skips_provider() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        let provider = Counting(Cell::new(0));
        let result = read_project_status(&gate, &std::env::temp_dir(), &provider);
        assert!(matches!(
            result,
            Err(GitOpError::Gate(GatedRunError::ProjectNotTrusted))
        ));
        assert_eq!(runner.spawn_count(), 0);
        assert_eq!(provider.0.get(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::file_status::FileChange;
        use crate::snapshot::read_index_snapshot;
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };

        /// fixture chunk 형식: `CHUNK` + 본문 + 본문 바이트 합(mod 256) 1바이트.
        fn chunk(body: &[u8]) -> Vec<u8> {
            let sum = body.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
            let mut out = b"CHUNK".to_vec();
            out.extend_from_slice(body);
            out.push(sum);
            out
        }

        #[derive(Debug, PartialEq, Eq)]
        enum FixtureError {
            CorruptChunk(Vec<u8>),
            Io(std::io::ErrorKind),
        }

        /// WorkingTree의 design chunk를 검증하는 fixture provider(손상 시 실패, 쓰기 없음).
        struct ChunkCheckingProvider;

        impl SemanticStatusProvider for ChunkCheckingProvider {
            type Status = usize;
            type Error = FixtureError;

            fn semantic_status(
                &self,
                _git: &GatedGitRunner,
                worktree: &Path,
                files: &[StellarFileStatus],
            ) -> Result<usize, FixtureError> {
                let mut checked = 0;
                for file in files {
                    let path = file.path.as_bytes();
                    if !path.ends_with(b".starchunk") || file.unstaged == Some(FileChange::Deleted)
                    {
                        continue;
                    }
                    let local = worktree.join(file.path.to_path_buf().unwrap());
                    let bytes = std::fs::read(local).map_err(|e| FixtureError::Io(e.kind()))?;
                    let valid = bytes.len() > 5 && bytes.starts_with(b"CHUNK") && {
                        let (body, sum) = bytes[5..].split_at(bytes.len() - 6);
                        body.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)) == sum[0]
                    };
                    if !valid {
                        return Err(FixtureError::CorruptChunk(path.to_vec()));
                    }
                    checked += 1;
                }
                Ok(checked)
            }
        }

        fn tree_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
            let mut out = Vec::new();
            let mut stack = vec![root.to_path_buf()];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    if path.file_name() == Some(std::ffi::OsStr::new(".git")) {
                        continue;
                    }
                    if entry.file_type().unwrap().is_dir() {
                        stack.push(path);
                    } else {
                        out.push((path.clone(), std::fs::read(&path).unwrap()));
                    }
                }
            }
            out.sort();
            out
        }

        #[test]
        fn corrupt_chunk_fails_semantic_status_but_raw_status_remains() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("project-status-corrupt");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            let files: [(&str, Vec<u8>); 4] = [
                ("stellar.yaml", b"schema_version: 1\n".to_vec()),
                ("design/current.stellarref", b"gen-1\n".to_vec()),
                ("design/objects/aa/one.starchunk", chunk(b"nodes-1")),
                ("README.md", b"# P\n".to_vec()),
            ];
            for (path, body) in &files {
                let full = repo.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, body).unwrap();
            }
            git_ok(&runner, repo, ["add", "-A"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "base"]);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);

            // 정상 상태: semantic 성공.
            let clean = read_project_status(&gate, repo, &ChunkCheckingProvider).unwrap();
            assert!(!clean.semantic_failed());
            assert_eq!(clean.repository.counts.unstaged, 0);

            // 작업 트리 chunk 손상 + README 수정.
            let chunk_path = repo.join("design/objects/aa/one.starchunk");
            std::fs::write(&chunk_path, b"CHUNK\x00garbage").unwrap();
            std::fs::write(repo.join("README.md"), b"# P\n\nmore\n").unwrap();
            let before = tree_bytes(repo);
            let index_before = read_index_snapshot(&gate, repo).unwrap();

            let status = read_project_status(&gate, repo, &ChunkCheckingProvider).unwrap();
            // semantic은 실패를 그대로 보존합니다.
            assert_eq!(
                status.semantic,
                Err(FixtureError::CorruptChunk(
                    b"design/objects/aa/one.starchunk".to_vec()
                ))
            );
            assert!(status.semantic_failed());
            // raw status는 계속 제공됩니다.
            assert_eq!(status.basis, SnapshotBasis::SavedFilesOnDisk);
            assert_eq!(status.repository.counts.unstaged, 2);
            assert_eq!(
                status
                    .raw
                    .iter()
                    .map(|r| (r.path.as_bytes(), r.xy.as_str()))
                    .collect::<Vec<_>>(),
                vec![(&b"README.md"[..], ".M")]
            );
            assert_eq!(status.stellar_files.len(), 1);
            assert_eq!(
                status.stellar_files[0].path.as_bytes(),
                b"design/objects/aa/one.starchunk"
            );
            assert_eq!(status.stellar_files[0].unstaged, Some(FileChange::Modified));
            // 자동 수정 없음: 작업 트리 bytes·index 그대로.
            assert_eq!(tree_bytes(repo), before);
            assert_eq!(read_index_snapshot(&gate, repo).unwrap(), index_before);
            assert_eq!(std::fs::read(&chunk_path).unwrap(), b"CHUNK\x00garbage");
        }
    }
}
