//! conflict 세 stage의 virtual project snapshot materialize (XIV.11.2).
//!
//! 병합되지 않은 index에서 base(stage 1)·ours(stage 2)·theirs(stage 3) 프로젝트를 각각 임시 디렉터리로
//! 만듭니다(XIV.2.6 materializer, `.git` 없음).
//!
//! - **conflict 없는 경로**: 현재 index의 stage 0 항목(Git이 이미 병합해 stage한 결과)을 세 snapshot에
//!   모두 넣습니다.
//! - **conflict 경로 중 Stellar 프로젝트 파일**([`classify_project_path`]): 그 side의 stage blob이 있으면
//!   넣고, 없으면(그 side에서 삭제·미추가) 넣지 않습니다.
//! - **conflict 경로 중 non-Stellar 파일**: semantic snapshot에 넣지 않고 [`ConflictSnapshots::raw_conflicts`]로
//!   돌려줍니다(raw Git conflict, XIV.11.4).
//!
//! 작업 트리와 index는 읽기만 합니다.

use std::path::Path;

use crate::classify::{classify_project_path, ProjectFileClass};
use crate::conflict::{read_unmerged_paths, UnmergedPath};
use crate::decode::GitPath;
use crate::ops::GitOpError;
use crate::snapshot::index::read_index_snapshot;
use crate::snapshot::materialize::{materialize_snapshot, MaterializeError, MaterializedSnapshot};
use crate::snapshot::{IndexSnapshot, SnapshotEntry};
use crate::trust::GatedGitRunner;

/// conflict side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictSide {
    /// stage 1(공통 조상).
    Base,
    /// stage 2(현재 branch).
    Ours,
    /// stage 3(병합 대상).
    Theirs,
}

impl ConflictSide {
    /// 세 side(고정 순서).
    pub const ALL: [ConflictSide; 3] =
        [ConflictSide::Base, ConflictSide::Ours, ConflictSide::Theirs];

    /// index stage 번호.
    pub fn stage(self) -> u8 {
        match self {
            ConflictSide::Base => 1,
            ConflictSide::Ours => 2,
            ConflictSide::Theirs => 3,
        }
    }
}

/// 세 side snapshot과 raw conflict.
#[derive(Debug)]
pub struct ConflictSnapshots {
    /// base 프로젝트.
    pub base: MaterializedSnapshot,
    /// ours 프로젝트.
    pub ours: MaterializedSnapshot,
    /// theirs 프로젝트.
    pub theirs: MaterializedSnapshot,
    /// semantic 대상 Stellar conflict 경로(Git index 순서).
    pub stellar_conflicts: Vec<GitPath>,
    /// non-Stellar conflict(raw Git conflict로 표시).
    pub raw_conflicts: Vec<UnmergedPath>,
}

impl ConflictSnapshots {
    /// side의 snapshot.
    pub fn side(&self, side: ConflictSide) -> &MaterializedSnapshot {
        match side {
            ConflictSide::Base => &self.base,
            ConflictSide::Ours => &self.ours,
            ConflictSide::Theirs => &self.theirs,
        }
    }
}

/// materialize 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictSnapshotError {
    /// index·unmerged 읽기 실패.
    Op(GitOpError),
    /// side materialize 실패.
    Materialize {
        /// 실패한 side.
        side: ConflictSide,
        /// 원인.
        error: MaterializeError,
    },
}

impl std::fmt::Display for ConflictSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictSnapshotError::Op(e) => write!(f, "{e}"),
            ConflictSnapshotError::Materialize { side, error } => {
                write!(f, "{side:?} snapshot materialize 실패: {error}")
            }
        }
    }
}

impl std::error::Error for ConflictSnapshotError {}

impl From<GitOpError> for ConflictSnapshotError {
    fn from(e: GitOpError) -> ConflictSnapshotError {
        ConflictSnapshotError::Op(e)
    }
}

/// side 하나의 snapshot 항목을 만듭니다(stage 0 + Stellar conflict 경로의 그 side stage).
pub fn side_entries(
    index: &IndexSnapshot,
    unmerged: &[UnmergedPath],
    side: ConflictSide,
) -> Vec<SnapshotEntry> {
    let mut entries = index.entries.clone();
    for path in unmerged {
        if classify_project_path(path.path.as_bytes()) == ProjectFileClass::NonStellar {
            continue;
        }
        if let Some(blob) = path.stage(side.stage()) {
            entries.push(SnapshotEntry {
                path: path.path.clone(),
                mode: blob.mode,
                oid: blob.oid.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// 현재 conflict 상태에서 base·ours·theirs 프로젝트를 `parent` 아래에 materialize합니다.
pub fn materialize_conflict_snapshots(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
) -> Result<ConflictSnapshots, ConflictSnapshotError> {
    let index = read_index_snapshot(git, worktree)?;
    let unmerged = read_unmerged_paths(git, worktree)?;
    let (stellar, raw): (Vec<&UnmergedPath>, Vec<&UnmergedPath>) = unmerged
        .iter()
        .partition(|p| classify_project_path(p.path.as_bytes()) != ProjectFileClass::NonStellar);
    let make = |side: ConflictSide| {
        materialize_snapshot(
            git,
            worktree,
            &side_entries(&index, &unmerged, side),
            parent,
        )
        .map_err(|error| ConflictSnapshotError::Materialize { side, error })
    };
    Ok(ConflictSnapshots {
        base: make(ConflictSide::Base)?,
        ours: make(ConflictSide::Ours)?,
        theirs: make(ConflictSide::Theirs)?,
        stellar_conflicts: stellar.iter().map(|p| p.path.clone()).collect(),
        raw_conflicts: raw.into_iter().cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::snapshot::working_tree::DiskProjectLoader;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;
        use std::path::PathBuf;

        /// fixture 프로젝트 loader: 매니페스트와 chunk 두 개를 디스크에서 읽어 엽니다.
        struct FixtureProjectLoader;

        #[derive(Debug, PartialEq, Eq)]
        struct OpenedProject {
            manifest: Vec<u8>,
            pointer: Vec<u8>,
            tokens: Option<Vec<u8>>,
            readme: Option<Vec<u8>>,
        }

        impl DiskProjectLoader for FixtureProjectLoader {
            type Project = OpenedProject;
            type Error = std::io::ErrorKind;

            fn load_saved_project(&self, root: &Path) -> Result<OpenedProject, std::io::ErrorKind> {
                let read = |p: &str| std::fs::read(root.join(p)).map_err(|e| e.kind());
                if root.join(".git").exists() {
                    return Err(std::io::ErrorKind::InvalidData);
                }
                Ok(OpenedProject {
                    manifest: read("stellar.yaml")?,
                    pointer: read("design/current.stellarref")?,
                    tokens: read("design/objects/tokens.starchunk").ok(),
                    readme: read("README.md").ok(),
                })
            }
        }

        #[test]
        fn base_ours_theirs_projects_open() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("conflict-snap");
            let repo = dir.path().join("repo");
            let out = dir.path().join("out");
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::create_dir_all(&out).unwrap();
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, &repo);
            for (p, b) in [
                ("stellar.yaml", &b"name: p\n"[..]),
                ("design/current.stellarref", b"gen-base\n"),
                ("design/objects/tokens.starchunk", b"tokens-base\n"),
                ("design/objects/shared.starchunk", b"shared\n"),
                ("README.md", b"readme base\n"),
            ] {
                commit_file(&runner, &repo, p, b);
            }
            git_ok(&runner, &repo, ["switch", "-q", "-c", "other"]);
            commit_file(&runner, &repo, "design/current.stellarref", b"gen-theirs\n");
            git_ok(
                &runner,
                &repo,
                ["rm", "-q", "design/objects/tokens.starchunk"],
            );
            git_ok(&runner, &repo, ["commit", "-q", "-m", "rm tokens"]);
            commit_file(&runner, &repo, "README.md", b"readme theirs\n");
            commit_file(
                &runner,
                &repo,
                "design/objects/merged.starchunk",
                b"clean add\n",
            );
            git_ok(&runner, &repo, ["switch", "-q", "main"]);
            commit_file(&runner, &repo, "design/current.stellarref", b"gen-ours\n");
            commit_file(
                &runner,
                &repo,
                "design/objects/tokens.starchunk",
                b"tokens-ours\n",
            );
            commit_file(&runner, &repo, "README.md", b"readme ours\n");
            assert!(!runner
                .run(&crate::runner::argv(["merge", "-q", "other"]), &repo)
                .unwrap()
                .success());

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let worktree_before = std::fs::read(repo.join("design/current.stellarref")).unwrap();
            let index_before = git_ok(&runner, &repo, ["ls-files", "--stage", "-z"]).stdout;
            let snaps = materialize_conflict_snapshots(&gate, &repo, &out).unwrap();

            let stellar: Vec<&[u8]> = snaps
                .stellar_conflicts
                .iter()
                .map(|p| p.as_bytes())
                .collect();
            assert_eq!(
                stellar,
                vec![
                    &b"design/current.stellarref"[..],
                    b"design/objects/tokens.starchunk"
                ]
            );
            let raw: Vec<&[u8]> = snaps
                .raw_conflicts
                .iter()
                .map(|p| p.path.as_bytes())
                .collect();
            assert_eq!(raw, vec![&b"README.md"[..]]);

            let open = |side| FixtureProjectLoader.load_saved_project(snaps.side(side).root());
            let base = open(ConflictSide::Base).unwrap();
            let ours = open(ConflictSide::Ours).unwrap();
            let theirs = open(ConflictSide::Theirs).unwrap();
            assert_eq!(base.pointer, b"gen-base\n");
            assert_eq!(ours.pointer, b"gen-ours\n");
            assert_eq!(theirs.pointer, b"gen-theirs\n");
            assert_eq!(base.tokens.as_deref(), Some(&b"tokens-base\n"[..]));
            assert_eq!(ours.tokens.as_deref(), Some(&b"tokens-ours\n"[..]));
            assert_eq!(theirs.tokens, None, "theirs에서 삭제된 chunk");
            for p in [&base, &ours, &theirs] {
                assert_eq!(p.manifest, b"name: p\n");
                // non-Stellar conflict는 semantic snapshot에 넣지 않음.
                assert_eq!(p.readme, None);
            }
            // conflict 없는 경로(공통·깨끗이 병합된 추가)는 세 snapshot 모두 index stage 0 내용.
            for side in ConflictSide::ALL {
                let root: PathBuf = snaps.side(side).root().to_path_buf();
                assert_eq!(
                    std::fs::read(root.join("design/objects/shared.starchunk")).unwrap(),
                    b"shared\n"
                );
                assert_eq!(
                    std::fs::read(root.join("design/objects/merged.starchunk")).unwrap(),
                    b"clean add\n"
                );
            }
            // 작업 트리·index 읽기 전용.
            assert_eq!(
                std::fs::read(repo.join("design/current.stellarref")).unwrap(),
                worktree_before
            );
            assert_eq!(
                git_ok(&runner, &repo, ["ls-files", "--stage", "-z"]).stdout,
                index_before
            );
        }
    }
}
