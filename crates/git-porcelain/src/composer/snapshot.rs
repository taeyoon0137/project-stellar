//! Source A·B·Base snapshot materialize (XXIII.1.3).
//!
//! 해석한 source([`ResolvedSource`])를 SnapshotProvider(XIV.2.4~XIV.2.7)로 읽어 **새 임시 디렉터리**에
//! materialize합니다. 원본 worktree·index·ref·object database를 바꾸지 않습니다.
//!
//! | snapshot 원천 | 읽기 | 내용 지문(`entries`) |
//! |---|---|---|
//! | commit·tree | `ls-tree` + `cat-file --batch`(XIV.2.4·XIV.2.6) | Git tree의 path·mode·blob ID |
//! | index | `ls-files --stage`(XIV.2.5) + `cat-file --batch` | index stage 0의 path·mode·blob ID |
//! | 작업 트리 | `ls-files` 목록 + 디스크 읽기(XIV.2.7, symlink 비추적) | `hash-object --no-filters`로 계산한 blob ID |
//!
//! - 쓰기 전용 명령을 쓰지 않습니다. `git status`(index stat 갱신)·`hash-object -w`(object 쓰기)를 실행하지
//!   않으며, 작업 트리 hash는 `-w` 없이 계산합니다.
//! - 출력 디렉터리(`parent`)가 source worktree 안이면 그 worktree를 바꾸게 되므로 거부합니다.
//! - unmerged index는 materialize하지 않습니다(Semantic Conflict 경로 사용).
//! - Base는 [`ComparisonMode::ThreeWay`]일 때만 만듭니다(criss-cross·2-way는 `None`).
//! - `entries`는 materialize한 내용의 지문이며 이후 source 변경 검사(XXIII.3.4)의 기준입니다.

use std::path::{Path, PathBuf};

use super::base::{comparison_mode, ComparisonMode};
use super::source::{ResolvedSource, SourceRef};
use crate::decode::GitPath;
use crate::object::{FileMode, FileModeKind, ObjectId};
use crate::ops::{run_ok_with_input, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::snapshot::materialize::{create_unique_root, materialize_with};
use crate::snapshot::working_tree::{
    list_working_tree_files, read_working_tree_file, WorkingTreeFile, WorkingTreeFileKind,
};
use crate::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, MaterializeError,
    MaterializedSnapshot, SnapshotEntry, SnapshotError, SnapshotSource,
};
use crate::trust::GatedGitRunner;

/// materialize한 source 하나.
#[derive(Debug)]
pub struct SourceSnapshot {
    /// 해석된 source.
    pub resolved: ResolvedSource,
    /// 내용 지문(path·mode·blob ID).
    pub entries: Vec<SnapshotEntry>,
    /// materialize하지 않은 항목(submodule gitlink, 작업 트리의 디렉터리·특수 파일).
    pub skipped: Vec<GitPath>,
    /// 임시 프로젝트 디렉터리(drop 시 삭제).
    pub materialized: MaterializedSnapshot,
}

/// Source A·B와 Base snapshot.
#[derive(Debug)]
pub struct ComposerSnapshots {
    /// 비교 방식(XXIII.1.2).
    pub mode: ComparisonMode,
    /// Source A.
    pub a: SourceSnapshot,
    /// Source B.
    pub b: SourceSnapshot,
    /// Base(3-way일 때만).
    pub base: Option<SourceSnapshot>,
}

/// snapshot materialize 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerSnapshotError {
    /// 출력 디렉터리가 source worktree 안(원본 변경 방지, 아무것도 만들지 않음).
    ParentInsideWorktree {
        /// 출력 디렉터리.
        parent: PathBuf,
        /// 겹치는 worktree.
        worktree: PathBuf,
    },
    /// 출력 디렉터리를 확인할 수 없음.
    InvalidParent(std::io::ErrorKind),
    /// index에 unmerged 항목이 있음.
    UnmergedIndex {
        /// worktree root.
        worktree: PathBuf,
    },
    /// 작업 트리 파일 읽기 실패.
    Read {
        /// 경로.
        path: GitPath,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// snapshot provider 실패.
    Snapshot(SnapshotError),
    /// materialize 실패.
    Materialize(MaterializeError),
    /// Git 실행·해석 실패.
    Op(GitOpError),
}

impl std::fmt::Display for ComposerSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposerSnapshotError::ParentInsideWorktree { parent, worktree } => write!(
                f,
                "snapshot 출력 위치 {}가 worktree {} 안에 있습니다",
                parent.display(),
                worktree.display()
            ),
            ComposerSnapshotError::InvalidParent(kind) => {
                write!(f, "snapshot 출력 위치를 확인할 수 없음: {kind:?}")
            }
            ComposerSnapshotError::UnmergedIndex { worktree } => {
                write!(
                    f,
                    "unmerged index는 snapshot으로 읽지 않습니다: {}",
                    worktree.display()
                )
            }
            ComposerSnapshotError::Read { path, kind } => {
                write!(
                    f,
                    "작업 트리 파일 읽기 실패 {:?}: {kind:?}",
                    path.text().display()
                )
            }
            ComposerSnapshotError::Snapshot(e) => write!(f, "{e}"),
            ComposerSnapshotError::Materialize(e) => write!(f, "{e}"),
            ComposerSnapshotError::Op(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ComposerSnapshotError {}

impl From<SnapshotError> for ComposerSnapshotError {
    fn from(e: SnapshotError) -> ComposerSnapshotError {
        ComposerSnapshotError::Snapshot(e)
    }
}

impl From<MaterializeError> for ComposerSnapshotError {
    fn from(e: MaterializeError) -> ComposerSnapshotError {
        ComposerSnapshotError::Materialize(e)
    }
}

impl From<GitOpError> for ComposerSnapshotError {
    fn from(e: GitOpError) -> ComposerSnapshotError {
        ComposerSnapshotError::Op(e)
    }
}

/// `parent`가 어떤 worktree 안에도 있지 않은지 확인합니다(실경로 비교).
fn check_parent(parent: &Path, worktrees: &[&Path]) -> Result<(), ComposerSnapshotError> {
    // hash 입력은 줄 단위 경로이므로 개행이 든 출력 위치는 받지 않습니다.
    if parent.as_os_str().as_encoded_bytes().contains(&b'\n') {
        return Err(ComposerSnapshotError::InvalidParent(
            std::io::ErrorKind::InvalidInput,
        ));
    }
    let parent_real = std::fs::canonicalize(parent)
        .map_err(|e| ComposerSnapshotError::InvalidParent(e.kind()))?;
    for worktree in worktrees {
        let real = std::fs::canonicalize(worktree)
            .map_err(|e| ComposerSnapshotError::InvalidParent(e.kind()))?;
        if parent_real.starts_with(&real) {
            return Err(ComposerSnapshotError::ParentInsideWorktree {
                parent: parent.to_path_buf(),
                worktree: worktree.to_path_buf(),
            });
        }
    }
    Ok(())
}

/// 임시 디렉터리(drop 시 삭제).
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mode_of(kind: WorkingTreeFileKind) -> (FileMode, FileModeKind) {
    match kind {
        WorkingTreeFileKind::Regular => (FileMode::REGULAR, FileModeKind::Regular),
        WorkingTreeFileKind::Executable => (FileMode::EXECUTABLE, FileModeKind::Executable),
        WorkingTreeFileKind::Symlink => (FileMode::SYMLINK, FileModeKind::Symlink),
    }
}

/// `git hash-object --no-filters --stdin-paths` 출력(줄마다 ID)을 해석합니다.
fn parse_hashes(stdout: &[u8], expected: usize) -> Result<Vec<ObjectId>, ParseFailure> {
    const COMMAND: &str = "hash-object --stdin-paths";
    let mut out = Vec::with_capacity(expected);
    if expected > 0 {
        let body = stdout
            .strip_suffix(b"\n")
            .ok_or_else(|| ParseFailure::new(COMMAND, "LF로 끝나지 않음", stdout))?;
        for line in body.split(|b| *b == b'\n') {
            out.push(
                ObjectId::from_hex_bytes(line)
                    .ok_or_else(|| ParseFailure::new(COMMAND, "object ID 아님", line))?,
            );
        }
    }
    if out.len() != expected {
        return Err(ParseFailure::new(COMMAND, "ID 수가 요청과 다름", stdout));
    }
    Ok(out)
}

/// 작업 트리 source를 materialize하고 blob ID를 계산합니다(object database 쓰기 없음).
fn materialize_working_tree(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
) -> Result<(Vec<SnapshotEntry>, Vec<GitPath>, MaterializedSnapshot), ComposerSnapshotError> {
    let listing = list_working_tree_files(git, worktree)?;
    let kinds: Vec<(GitPath, FileModeKind)> = listing
        .files
        .iter()
        .map(|f| (f.path.clone(), mode_of(f.kind).1))
        .collect();
    // 한 번 읽은 바이트를 materialize 사본과 hash용 번호 파일에 같이 씁니다(경로 이름과 무관한 hash 입력).
    let scratch = ScratchDir(create_unique_root(parent)?);
    let by_index: &[WorkingTreeFile] = &listing.files;
    let mut read_failure: Option<(GitPath, std::io::ErrorKind)> = None;
    let materialized = materialize_with(&kinds, parent, |index, path, _| {
        let file = by_index.get(index).ok_or(MaterializeError::Io {
            path: Some(path.clone()),
            kind: std::io::ErrorKind::InvalidInput,
        })?;
        let bytes = read_working_tree_file(worktree, file).map_err(|kind| {
            read_failure = Some((path.clone(), kind));
            MaterializeError::Io {
                path: Some(path.clone()),
                kind,
            }
        })?;
        std::fs::write(scratch.0.join(index.to_string()), &bytes).map_err(|e| {
            MaterializeError::Io {
                path: Some(path.clone()),
                kind: e.kind(),
            }
        })?;
        Ok(bytes)
    });
    let materialized = match (materialized, read_failure) {
        (Ok(m), _) => m,
        (Err(_), Some((path, kind))) => return Err(ComposerSnapshotError::Read { path, kind }),
        (Err(e), None) => return Err(ComposerSnapshotError::Materialize(e)),
    };

    let mut entries = Vec::with_capacity(listing.files.len());
    if !listing.files.is_empty() {
        let mut input = Vec::new();
        for index in 0..listing.files.len() {
            input.extend_from_slice(
                scratch
                    .0
                    .join(index.to_string())
                    .as_os_str()
                    .as_encoded_bytes(),
            );
            input.push(b'\n');
        }
        let output = run_ok_with_input(
            git,
            worktree,
            &argv(["hash-object", "--no-filters", "--stdin-paths"]),
            &input,
        )?;
        let oids = parse_hashes(&output.stdout, listing.files.len()).map_err(GitOpError::Parse)?;
        for (file, oid) in listing.files.iter().zip(oids) {
            entries.push(SnapshotEntry {
                path: file.path.clone(),
                mode: mode_of(file.kind).0,
                oid,
            });
        }
    }
    Ok((entries, listing.skipped, materialized))
}

/// source 하나를 `parent` 아래 새 임시 디렉터리에 materialize합니다.
pub fn materialize_source(
    git: &GatedGitRunner,
    source: &ResolvedSource,
    parent: &Path,
) -> Result<SourceSnapshot, ComposerSnapshotError> {
    check_parent(parent, &[&source.worktree])?;
    let worktree = &source.worktree;
    let (entries, skipped, materialized) = match &source.snapshot {
        SnapshotSource::WorkingTree => materialize_working_tree(git, worktree, parent)?,
        SnapshotSource::Index => {
            let index = read_index_snapshot(git, worktree)?;
            if index.has_conflicts() {
                return Err(ComposerSnapshotError::UnmergedIndex {
                    worktree: worktree.clone(),
                });
            }
            let materialized = materialize_snapshot(git, worktree, &index.entries, parent)?;
            let skipped = materialized.skipped_gitlinks().to_vec();
            (index.entries, skipped, materialized)
        }
        tree_source => {
            let tree = read_tree_snapshot(git, worktree, tree_source)?;
            let materialized = materialize_snapshot(git, worktree, &tree.entries, parent)?;
            let skipped = materialized.skipped_gitlinks().to_vec();
            (tree.entries, skipped, materialized)
        }
    };
    Ok(SourceSnapshot {
        resolved: source.clone(),
        entries,
        skipped,
        materialized,
    })
}

/// Source A·B와 (3-way면) Base를 materialize합니다. `current_worktree`는 Merge Composer를 연 worktree
/// root이며 merge-base 실행과 Base 읽기에 씁니다.
pub fn materialize_sources(
    git: &GatedGitRunner,
    current_worktree: &Path,
    a: &ResolvedSource,
    b: &ResolvedSource,
    parent: &Path,
) -> Result<ComposerSnapshots, ComposerSnapshotError> {
    check_parent(parent, &[current_worktree, &a.worktree, &b.worktree])?;
    let mode = comparison_mode(git, current_worktree, a, b)?;
    let base_source = mode.base().map(|oid| ResolvedSource {
        source: SourceRef::Commit(oid.clone()),
        worktree: current_worktree.to_path_buf(),
        snapshot: SnapshotSource::Commit(oid.clone()),
        commit: Some(oid.clone()),
    });
    let a = materialize_source(git, a, parent)?;
    let b = materialize_source(git, b, parent)?;
    let base = match &base_source {
        Some(source) => Some(materialize_source(git, source, parent)?),
        None => None,
    };
    Ok(ComposerSnapshots { mode, a, b, base })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::composer::base::TwoWayReason;
    use crate::composer::source::{resolve_source, WorktreeState};
    use crate::fixture::{build_standard_scenario, GitFixture};
    use std::collections::BTreeMap;

    /// 디렉터리 아래 모든 항목(경로 → 파일 바이트 또는 `link:` + 대상), `.git` 제외.
    fn tree_state(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if rel == ".git" {
                    continue;
                }
                let meta = std::fs::symlink_metadata(&path).unwrap();
                if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&path).unwrap();
                    out.insert(rel, format!("link:{}", target.display()).into_bytes());
                } else if meta.is_dir() {
                    stack.push(path);
                } else {
                    use std::os::unix::fs::PermissionsExt;
                    let mut bytes = std::fs::read(&path).unwrap();
                    bytes.extend_from_slice(
                        format!(" mode:{:o}", meta.permissions().mode() & 0o777).as_bytes(),
                    );
                    out.insert(rel, bytes);
                }
            }
        }
        out
    }

    /// 원본 저장소 상태(HEAD·ref·index 파일·object 수·작업 트리 파일).
    #[derive(Debug, PartialEq, Eq)]
    struct RepoState {
        head: Vec<u8>,
        refs: Vec<u8>,
        index: Vec<u8>,
        objects: Vec<u8>,
        files: BTreeMap<String, Vec<u8>>,
    }

    fn repo_state(fx: &GitFixture, worktree: &Path) -> RepoState {
        let git_dir = fx
            .git_in(
                worktree,
                ["rev-parse", "--path-format=absolute", "--git-dir"],
            )
            .unwrap()
            .stdout;
        let git_dir = PathBuf::from(String::from_utf8(git_dir).unwrap().trim());
        RepoState {
            head: fx.git_in(worktree, ["rev-parse", "HEAD"]).unwrap().stdout,
            refs: fx.git_in(worktree, ["for-each-ref"]).unwrap().stdout,
            index: std::fs::read(git_dir.join("index")).unwrap(),
            objects: fx.git_in(worktree, ["count-objects", "-v"]).unwrap().stdout,
            files: tree_state(worktree),
        }
    }

    /// materialize한 디렉터리를 Git hash로 다시 계산해 지문과 비교합니다.
    fn assert_matches_entries(fx: &GitFixture, snap: &SourceSnapshot) {
        let root = snap.materialized.root();
        assert!(!root.join(".git").exists());
        for entry in &snap.entries {
            let local = root.join(String::from_utf8(entry.path.as_bytes().to_vec()).unwrap());
            let meta = std::fs::symlink_metadata(&local).unwrap();
            let oid = if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&local).unwrap();
                let out = fx
                    .runner()
                    .run_with_input(
                        &argv(["hash-object", "--stdin"]),
                        fx.work(),
                        target.as_os_str().as_encoded_bytes(),
                    )
                    .unwrap();
                out.stdout
            } else {
                fx.git(["hash-object", "--no-filters", "--", local.to_str().unwrap()])
                    .unwrap()
                    .stdout
            };
            assert_eq!(
                String::from_utf8(oid).unwrap().trim(),
                entry.oid.as_str(),
                "{:?}",
                entry.path
            );
        }
    }

    /// source hash 보존: A·B·Base materialize 후 원본 상태 불변, 사본 내용 = 원본 지문.
    #[test]
    fn source_hashes_are_preserved_and_originals_untouched() {
        let Some(fx) = GitFixture::with_system_git("composer-snap").unwrap() else {
            return; // git 미설치 환경 건너뜀.
        };
        let s = build_standard_scenario(&fx).unwrap();
        let wt = fx.add_worktree("linked", "refs/heads/feature").unwrap();
        // 현재 worktree: staged 변경, 그 위 unstaged 변경, untracked, 삭제, symlink, 실행 파일.
        fx.write("design/pages/home.yaml", b"id: home\ntitle: Staged\n")
            .unwrap();
        fx.git(["add", "--", "design/pages/home.yaml"]).unwrap();
        fx.write("design/pages/home.yaml", b"id: home\ntitle: On disk\n")
            .unwrap();
        fx.write("notes/untracked.txt", b"untracked\n").unwrap();
        std::fs::remove_file(fx.work().join("stellar.yaml")).unwrap();
        std::os::unix::fs::symlink("design/pages/home.yaml", fx.work().join("home-link")).unwrap();
        fx.write("tools/run.sh", b"#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                fx.work().join("tools/run.sh"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let out_dir = fx.root().join("snapshots");
        std::fs::create_dir_all(&out_dir).unwrap();
        let gate = fx.gated();
        let work = fx.work();
        let before = (repo_state(&fx, work), repo_state(&fx, &wt));

        let a = resolve_source(&gate, work, &SourceRef::Branch("feature".into())).unwrap();
        let b = resolve_source(&gate, work, &SourceRef::WorkingTree).unwrap();
        let snaps = materialize_sources(&gate, work, &a, &b, &out_dir).unwrap();
        let index_source = resolve_source(&gate, work, &SourceRef::Index).unwrap();
        let index = materialize_source(&gate, &index_source, &out_dir).unwrap();
        let linked = resolve_source(
            &gate,
            work,
            &SourceRef::Worktree {
                path: wt.clone(),
                state: WorktreeState::WorkingTree,
            },
        )
        .unwrap();
        let linked = materialize_source(&gate, &linked, &out_dir).unwrap();

        // 원본 불변: HEAD·ref·index 파일·object 수·작업 트리 파일(두 worktree).
        assert_eq!((repo_state(&fx, work), repo_state(&fx, &wt)), before);

        // 3-way: base = 분기점 initial.
        assert_eq!(
            snaps.mode,
            ComparisonMode::ThreeWay {
                base: s.initial.clone()
            }
        );
        let base = snaps.base.as_ref().unwrap();
        for snap in [&snaps.a, &snaps.b, base, &index, &linked] {
            assert_matches_entries(&fx, snap);
        }
        // 작업 트리 지문 = 원본 파일의 Git hash(필터 없음).
        let wt_paths: Vec<&[u8]> = snaps.b.entries.iter().map(|e| e.path.as_bytes()).collect();
        assert_eq!(
            wt_paths,
            vec![
                &b"design/pages/home.yaml"[..],
                b"home-link",
                b"notes/untracked.txt",
                b"tools/run.sh"
            ]
        );
        for entry in &snaps.b.entries {
            let path = String::from_utf8(entry.path.as_bytes().to_vec()).unwrap();
            if entry.mode == FileMode::SYMLINK {
                assert_eq!(
                    std::fs::read_link(snaps.b.materialized.root().join(&path)).unwrap(),
                    PathBuf::from("design/pages/home.yaml")
                );
                continue;
            }
            let original = fx
                .git(["hash-object", "--no-filters", "--", &path])
                .unwrap()
                .stdout;
            assert_eq!(
                String::from_utf8(original).unwrap().trim(),
                entry.oid.as_str()
            );
        }
        assert_eq!(
            std::fs::read(snaps.b.materialized.root().join("design/pages/home.yaml")).unwrap(),
            b"id: home\ntitle: On disk\n"
        );
        assert_eq!(snaps.b.entries[3].mode, FileMode::EXECUTABLE);
        // index snapshot은 staged 내용, 삭제는 index에 아직 남음.
        assert_eq!(
            std::fs::read(index.materialized.root().join("design/pages/home.yaml")).unwrap(),
            b"id: home\ntitle: Staged\n"
        );
        assert!(index.materialized.root().join("stellar.yaml").is_file());
        // A = feature commit, Base = initial commit.
        assert!(snaps
            .a
            .materialized
            .root()
            .join("design/pages/about.yaml")
            .is_file());
        assert_eq!(
            std::fs::read(base.materialized.root().join("design/pages/home.yaml")).unwrap(),
            b"id: home\n"
        );
        assert!(!base
            .materialized
            .root()
            .join("design/pages/about.yaml")
            .exists());
        // linked worktree 작업 트리 = feature 내용.
        assert!(linked
            .materialized
            .root()
            .join("design/pages/about.yaml")
            .is_file());

        // drop 시 사본 삭제, 원본은 그대로.
        let roots: Vec<PathBuf> = [&snaps.a, &snaps.b]
            .iter()
            .map(|s| s.materialized.root().to_path_buf())
            .collect();
        drop(snaps);
        assert!(roots.iter().all(|r| !r.exists()));
        assert_eq!(repo_state(&fx, work), before.0);
        // hash용 임시 디렉터리도 남지 않음(linked·index 사본만 남음).
        assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 2);
    }

    #[test]
    fn parent_inside_worktree_and_unmerged_index_are_rejected() {
        let Some(fx) = GitFixture::with_system_git("composer-snap-guard").unwrap() else {
            return;
        };
        let s = build_standard_scenario(&fx).unwrap();
        let gate = fx.gated();
        let work = fx.work();
        let inside = work.join("design");
        let a = resolve_source(&gate, work, &SourceRef::Head).unwrap();
        let b = resolve_source(&gate, work, &SourceRef::Branch("feature".into())).unwrap();
        let before = tree_state(work);
        assert!(matches!(
            materialize_sources(&gate, work, &a, &b, &inside),
            Err(ComposerSnapshotError::ParentInsideWorktree { .. })
        ));
        assert!(matches!(
            materialize_source(&gate, &a, work),
            Err(ComposerSnapshotError::ParentInsideWorktree { .. })
        ));
        assert_eq!(tree_state(work), before);

        // 충돌 중인 index.
        fx.git(["switch", "-q", "-c", "conflict", s.initial.as_str()])
            .unwrap();
        fx.write("stellar.yaml", b"format: 1\nname: conflict\n")
            .unwrap();
        fx.commit_all("conflict side").unwrap();
        fx.git(["switch", "-q", "main"]).unwrap();
        fx.write("stellar.yaml", b"format: 1\nname: main side\n")
            .unwrap();
        fx.commit_all("main side").unwrap();
        assert!(fx.git(["merge", "-q", "conflict"]).is_err());
        let out_dir = fx.root().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let index = resolve_source(&gate, work, &SourceRef::Index).unwrap();
        assert!(matches!(
            materialize_source(&gate, &index, &out_dir),
            Err(ComposerSnapshotError::UnmergedIndex { .. })
        ));
        assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 0);
    }

    #[test]
    fn two_way_sources_have_no_base() {
        let Some(fx) = GitFixture::with_system_git("composer-snap-2way").unwrap() else {
            return;
        };
        build_standard_scenario(&fx).unwrap();
        fx.git(["switch", "-q", "--orphan", "unrelated"]).unwrap();
        fx.write("other.txt", b"other\n").unwrap();
        fx.commit_all("orphan").unwrap();
        fx.git(["switch", "-q", "-f", "main"]).unwrap();
        let gate = fx.gated();
        let work = fx.work();
        let a = resolve_source(&gate, work, &SourceRef::Branch("unrelated".into())).unwrap();
        let b = resolve_source(&gate, work, &SourceRef::Head).unwrap();
        let out_dir = fx.root().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let snaps = materialize_sources(&gate, work, &a, &b, &out_dir).unwrap();
        assert_eq!(
            snaps.mode,
            ComparisonMode::TwoWay {
                reason: TwoWayReason::NoCommonHistory
            }
        );
        assert!(snaps.base.is_none());
        assert_eq!(snaps.a.entries.len(), 1);
        assert!(snaps.a.materialized.root().join("other.txt").is_file());
    }

    #[test]
    fn hash_output_parser_checks_count() {
        let a = "a".repeat(40);
        assert_eq!(parse_hashes(b"", 0).unwrap(), Vec::new());
        assert_eq!(
            parse_hashes(format!("{a}\n").as_bytes(), 1).unwrap().len(),
            1
        );
        for (bad, n) in [
            (format!("{a}\n"), 2),
            (format!("{a}\n{a}\n"), 1),
            (a.clone(), 1),
            ("zz\n".to_string(), 1),
        ] {
            assert!(parse_hashes(bad.as_bytes(), n).is_err(), "{bad:?}");
        }
    }
}
