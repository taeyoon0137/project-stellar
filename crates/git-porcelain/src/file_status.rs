//! Stellar 프로젝트 파일 단위 status (XIV.4.3).
//!
//! Semantic Status(semantic-diff `status`)는 객체·속성 단위이고, 이 모듈은 그 입력이 되는 **Stellar
//! 프로젝트 파일**(XIV.4.4 분류)의 파일 단위 변경을 영역별로 정리합니다.
//!
//! | 영역 | Git 비교 | 값 |
//! |---|---|---|
//! | staged | HEAD → Index(`XY`의 X) | [`FileChange`] |
//! | unstaged | Index → WorkingTree(`XY`의 Y, untracked 포함) | [`FileChange`] |
//!
//! - **untracked Stellar 파일**(새 chunk·asset 등)은 WorkingTree snapshot에는 포함되고(XIV.2.7
//!   `--others --exclude-standard`) index snapshot에는 없습니다. 그래서 unstaged
//!   [`FileChange::Untracked`]이고 `in_index == false`입니다. 그 안의 객체는 semantic 쪽에서
//!   WorkingTree에만 있는 AddObject가 됩니다.
//! - ignored 항목은 WorkingTree snapshot에 들어가지 않으므로 목록에서 뺍니다.
//! - unmerged 항목은 conflict 흐름(XIV.11)으로 넘기므로 영역 변경 없이 `conflicted`로만 표시합니다.
//! - non-Stellar 파일과 경계를 넘는 rename/copy는 raw status(XIV.4.4)로만 표시하고 여기 넣지
//!   않습니다.

use std::path::Path;

use crate::classify::{classify_project_path, partition_status, ProjectFileClass, StellarFileKind};
use crate::decode::GitPath;
use crate::ops::GitOpError;
use crate::status::{read_status, ChangedEntry, StatusCode, StatusEntry, StatusOptions};
use crate::trust::GatedGitRunner;

/// 한 영역에서 파일 하나의 변경.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    /// 추가(staged 쪽 `A`).
    Added,
    /// 내용 수정(`M`).
    Modified,
    /// 종류 변경(파일↔symlink 등, `T`).
    TypeChanged,
    /// 삭제(`D`).
    Deleted,
    /// 이름 변경(`R`).
    Renamed {
        /// 원래 경로.
        from: GitPath,
    },
    /// 복사(`C`).
    Copied {
        /// 원본 경로.
        from: GitPath,
    },
    /// Git이 추적하지 않는 새 파일(WorkingTree에만 있음).
    Untracked,
}

/// Stellar 프로젝트 파일 하나의 영역별 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StellarFileStatus {
    /// 경로(저장소 root 기준).
    pub path: GitPath,
    /// Stellar 파일 종류.
    pub kind: StellarFileKind,
    /// HEAD → Index 변경.
    pub staged: Option<FileChange>,
    /// Index → WorkingTree 변경.
    pub unstaged: Option<FileChange>,
    /// index(stage 0)에 이 경로가 있는지.
    pub in_index: bool,
    /// 병합되지 않은 경로인지(conflict 흐름 대상).
    pub conflicted: bool,
}

impl StellarFileStatus {
    /// untracked(WorkingTree에만 있고 index에는 없음)인지.
    pub fn is_untracked(&self) -> bool {
        self.unstaged == Some(FileChange::Untracked)
    }
}

fn code_change(code: StatusCode, from: Option<&GitPath>) -> Option<FileChange> {
    match code {
        StatusCode::Unmodified | StatusCode::Unmerged => None,
        StatusCode::Added => Some(FileChange::Added),
        StatusCode::Modified => Some(FileChange::Modified),
        StatusCode::TypeChanged => Some(FileChange::TypeChanged),
        StatusCode::Deleted => Some(FileChange::Deleted),
        StatusCode::Renamed => from.map(|from| FileChange::Renamed { from: from.clone() }),
        StatusCode::Copied => from.map(|from| FileChange::Copied { from: from.clone() }),
    }
}

fn changed(
    entry: &ChangedEntry,
    from: Option<&GitPath>,
) -> (Option<FileChange>, Option<FileChange>) {
    (
        code_change(entry.index, from),
        code_change(entry.worktree, None),
    )
}

/// status 항목 하나를 Stellar 파일 상태로 바꿉니다. Stellar 파일이 아니거나 ignored면 `None`.
pub fn stellar_file_status(entry: &StatusEntry) -> Option<StellarFileStatus> {
    let ProjectFileClass::Stellar(kind) = classify_project_path(entry.path().as_bytes()) else {
        return None;
    };
    let (staged, unstaged, in_index, conflicted) = match entry {
        StatusEntry::Ordinary(e) => {
            let (staged, unstaged) = changed(e, None);
            // index 쪽이 삭제면 index에 없습니다(`D.`). WorkingTree 삭제(`.D`)는 index에 남습니다.
            (staged, unstaged, e.index != StatusCode::Deleted, false)
        }
        StatusEntry::Renamed(e) => {
            let (staged, unstaged) = changed(&e.entry, Some(&e.original_path));
            (staged, unstaged, true, false)
        }
        StatusEntry::Unmerged(_) => (None, None, false, true),
        StatusEntry::Untracked(_) => (None, Some(FileChange::Untracked), false, false),
        StatusEntry::Ignored(_) => return None,
    };
    Some(StellarFileStatus {
        path: entry.path().clone(),
        kind,
        staged,
        unstaged,
        in_index,
        conflicted,
    })
}

/// status 항목 목록에서 Stellar 파일 상태를 만듭니다(Git 출력 순서, 경계 넘는 rename/copy 제외).
pub fn stellar_file_statuses(entries: &[StatusEntry]) -> Vec<StellarFileStatus> {
    entries.iter().filter_map(stellar_file_status).collect()
}

/// worktree의 status를 읽어 Stellar 파일 상태를 만듭니다(trust gate 적용, 읽기 전용).
pub fn read_stellar_file_statuses(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<Vec<StellarFileStatus>, GitOpError> {
    let report = read_status(git, worktree, StatusOptions::default())?;
    let partition = partition_status(&report);
    Ok(stellar_file_statuses(&partition.stellar))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::parse_status_v2;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn status_codes_map_to_file_changes_per_area() {
        let z = "0".repeat(40);
        let bytes = format!(
            "# branch.oid {E1}\0# branch.head main\0\
             1 MM N... 100644 100644 100644 {E1} {E1} design/current.stellarref\0\
             1 A. N... 000000 100644 100644 {z} {E1} assets/sha256/aa/a.png\0\
             1 D. N... 100644 000000 000000 {E1} {z} design/old.starchunk\0\
             1 .D N... 100644 100644 000000 {E1} {E1} design/gone.starchunk\0\
             1 .T N... 100644 100644 120000 {E1} {E1} stellar.lock.yaml\0\
             2 R. N... 100644 100644 100644 {E1} {E1} R100 assets/new.png\0assets/old.png\0\
             u UU N... 100644 100644 100644 100644 {E1} {E1} {E1} stellar.yaml\0\
             1 .M N... 100644 100644 100644 {E1} {E1} README.md\0\
             ? design/objects/new.starchunk\0? notes.txt\0! design/cache.tmp\0"
        );
        let report = parse_status_v2(bytes.as_bytes()).unwrap();
        let statuses = stellar_file_statuses(&report.entries);
        type Row<'a> = (&'a [u8], Option<FileChange>, Option<FileChange>, bool, bool);
        let summary: Vec<Row<'_>> = statuses
            .iter()
            .map(|s| {
                (
                    s.path.as_bytes(),
                    s.staged.clone(),
                    s.unstaged.clone(),
                    s.in_index,
                    s.conflicted,
                )
            })
            .collect();
        use FileChange::*;
        assert_eq!(
            summary,
            vec![
                (
                    &b"design/current.stellarref"[..],
                    Some(Modified),
                    Some(Modified),
                    true,
                    false
                ),
                (b"assets/sha256/aa/a.png", Some(Added), None, true, false),
                (b"design/old.starchunk", Some(Deleted), None, false, false),
                (b"design/gone.starchunk", None, Some(Deleted), true, false),
                (b"stellar.lock.yaml", None, Some(TypeChanged), true, false),
                (
                    b"assets/new.png",
                    Some(Renamed {
                        from: GitPath::from_bytes(b"assets/old.png")
                    }),
                    None,
                    true,
                    false
                ),
                (b"stellar.yaml", None, None, false, true),
                (
                    b"design/objects/new.starchunk",
                    None,
                    Some(Untracked),
                    false,
                    false
                ),
            ]
        );
        assert_eq!(statuses[2].kind, StellarFileKind::Design);
        assert_eq!(statuses[1].kind, StellarFileKind::Asset);
        let untracked: Vec<&[u8]> = statuses
            .iter()
            .filter(|s| s.is_untracked())
            .map(|s| s.path.as_bytes())
            .collect();
        assert_eq!(untracked, vec![&b"design/objects/new.starchunk"[..]]);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::snapshot::read_index_snapshot;
        use crate::snapshot::working_tree::list_working_tree_files;
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn untracked_chunk_and_asset_are_working_tree_only_file_changes() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("file-status-untracked");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            for (path, body) in [
                ("stellar.yaml", &b"schema_version: 1\n"[..]),
                ("design/current.stellarref", b"gen-1\n"),
                ("design/objects/aa/one.starchunk", b"chunk-1"),
                ("assets/sha256/aa/logo.png", b"\x89PNG-1"),
                ("README.md", b"# P\n"),
            ] {
                let full = repo.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, body).unwrap();
            }
            git_ok(&runner, repo, ["add", "-A"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "base"]);
            // 저장 파이프라인이 새 chunk·asset을 썼지만 아직 stage하지 않은 상태.
            for (path, body) in [
                ("design/objects/bb/two.starchunk", &b"chunk-2"[..]),
                ("assets/sha256/bb/photo.png", b"\x89PNG-2"),
                ("notes.txt", b"memo\n"),
            ] {
                let full = repo.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, body).unwrap();
            }
            std::fs::write(repo.join("design/current.stellarref"), b"gen-2\n").unwrap();

            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let statuses = read_stellar_file_statuses(&gate, repo).unwrap();
            let untracked: Vec<(&[u8], StellarFileKind)> = statuses
                .iter()
                .filter(|s| s.is_untracked())
                .map(|s| (s.path.as_bytes(), s.kind))
                .collect();
            assert_eq!(
                untracked,
                vec![
                    (&b"assets/sha256/bb/photo.png"[..], StellarFileKind::Asset),
                    (b"design/objects/bb/two.starchunk", StellarFileKind::Design),
                ]
            );
            assert!(statuses
                .iter()
                .filter(|s| s.is_untracked())
                .all(|s| !s.in_index && s.staged.is_none() && !s.conflicted));
            let pointer = statuses
                .iter()
                .find(|s| s.path.as_bytes() == b"design/current.stellarref")
                .unwrap();
            assert_eq!(pointer.unstaged, Some(FileChange::Modified));
            assert!(pointer.in_index);
            // non-Stellar untracked(notes.txt)는 여기 없습니다(raw status 전용).
            assert!(statuses.iter().all(|s| s.path.as_bytes() != b"notes.txt"));

            // index snapshot에는 없고 WorkingTree 목록에는 있습니다.
            let index = read_index_snapshot(&gate, repo).unwrap();
            let listing = list_working_tree_files(&gate, repo).unwrap();
            for (path, _) in &untracked {
                assert!(!index.entries.iter().any(|e| e.path.as_bytes() == *path));
                assert!(listing.files.iter().any(|f| f.path.as_bytes() == *path));
            }
        }
    }
}
