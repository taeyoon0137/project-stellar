//! WorkingTree provider — 디스크에 저장된 파일 기준 (XIV.2.7).
//!
//! [`SnapshotSource::WorkingTree`](super::SnapshotSource::WorkingTree)는 **디스크에 저장된 파일**만
//! 뜻합니다. 편집기 메모리에 있는 저장 대기 상태(autosave 전 편집)는 포함하지 않습니다. Git
//! status([`crate::status`], [`crate::repo_status`])도 Git이 디스크 파일을 읽어 계산하므로 같은
//! 기준([`SnapshotBasis::SavedFilesOnDisk`])입니다.
//!
//! - 파일 목록: Git이 아는 작업 트리 파일(tracked + ignore되지 않은 untracked)을
//!   `git ls-files -z --cached --others --exclude-standard --full-name -- :/`로 받고, 각 경로를
//!   `symlink_metadata`로 확인합니다(symlink를 따르지 않음). index에 있으나 디스크에 없는 경로는
//!   `missing_tracked`, 디렉터리(submodule 등)·특수 파일은 `skipped`로 구분합니다.
//! - 프로젝트 내용: 호출자가 공급하는 [`DiskProjectLoader`](프로젝트 열기 loader, V.4.4)로 같은
//!   root를 디스크에서 읽습니다. git-porcelain은 loader 구현에 의존하지 않습니다(crate 경계). loader
//!   실패는 snapshot 실패로 숨기지 않고 `project`에 그대로 담아, 파일 목록·raw status는 계속 쓸 수
//!   있게 합니다.

use std::path::{Path, PathBuf};

use crate::decode::{split_nul_terminated, GitPath};
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

use super::materialize::validate_snapshot_path;

/// snapshot·status가 기준으로 삼는 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotBasis {
    /// 디스크에 저장된 파일(편집기의 저장 대기 메모리 상태는 제외).
    SavedFilesOnDisk,
}

/// 디스크 프로젝트 loader(프로젝트 열기 loader, V.4.4를 호출자가 연결).
pub trait DiskProjectLoader {
    /// 읽은 프로젝트 표현.
    type Project;
    /// loader 실패.
    type Error;

    /// 작업 트리 root에서 **디스크에 저장된** 프로젝트를 읽습니다.
    fn load_saved_project(&self, worktree_root: &Path) -> Result<Self::Project, Self::Error>;
}

/// 작업 트리 파일 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingTreeFileKind {
    /// 일반 파일.
    Regular,
    /// 실행 비트가 있는 파일.
    Executable,
    /// symbolic link.
    Symlink,
}

/// 작업 트리 파일(디스크 관찰값).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeFile {
    /// 경로(root 기준).
    pub path: GitPath,
    /// 종류.
    pub kind: WorkingTreeFileKind,
    /// 바이트 길이(symlink는 대상 경로 길이).
    pub len: u64,
}

/// 작업 트리 파일 목록(디스크 관찰 결과).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeListing {
    /// 디스크에 있는 파일(Git 경로 순).
    pub files: Vec<WorkingTreeFile>,
    /// index에 있으나 디스크에 없는 경로.
    pub missing_tracked: Vec<GitPath>,
    /// 파일이 아닌 항목.
    pub skipped: Vec<GitPath>,
}

/// 작업 트리 snapshot.
#[derive(Debug)]
pub struct WorkingTreeSnapshot<P, E> {
    /// 기준(항상 디스크 저장 파일).
    pub basis: SnapshotBasis,
    /// 작업 트리 root.
    pub root: PathBuf,
    /// 디스크에 있는 파일(Git 경로 순).
    pub files: Vec<WorkingTreeFile>,
    /// index에 있으나 디스크에 없는 경로(작업 트리 삭제).
    pub missing_tracked: Vec<GitPath>,
    /// 파일이 아닌 항목(디렉터리·submodule·특수 파일).
    pub skipped: Vec<GitPath>,
    /// loader가 디스크에서 읽은 프로젝트(실패도 그대로 보존).
    pub project: Result<P, E>,
}

/// 경로를 root 아래 로컬 경로로 바꿉니다(경로 검증 포함).
fn local_path(root: &Path, path: &GitPath) -> Option<PathBuf> {
    validate_snapshot_path(path.as_bytes()).ok()?;
    let relative = path.to_path_buf()?;
    Some(root.join(relative))
}

/// 작업 트리 파일 목록을 읽습니다(디스크 관찰, symlink 미추적).
pub fn list_working_tree_files(
    git: &GatedGitRunner,
    worktree_root: &Path,
) -> Result<WorkingTreeListing, GitOpError> {
    list_working_tree_pathspec(git, worktree_root, ":/")
}

/// 지정한 Git pathspec에 해당하는 파일만 열거하고 해당 항목의 mode를 조회합니다.
pub fn list_working_tree_pathspec(
    git: &GatedGitRunner,
    worktree_root: &Path,
    pathspec: &str,
) -> Result<WorkingTreeListing, GitOpError> {
    let output = run_ok(
        git,
        worktree_root,
        &argv([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--full-name",
            "--",
            pathspec,
        ]),
    )?;
    let records = split_nul_terminated(&output.stdout)
        .map_err(|e| ParseFailure::new("ls-files -z", e.to_string(), &output.stdout))?;
    let mut paths: Vec<GitPath> = records.into_iter().map(GitPath::from_bytes).collect();
    paths.sort();
    paths.dedup();

    let mut files = Vec::new();
    let mut missing = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let Some(local) = local_path(worktree_root, &path) else {
            skipped.push(path);
            continue;
        };
        match std::fs::symlink_metadata(&local) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let len = std::fs::read_link(&local)
                    .map(|t| t.as_os_str().len() as u64)
                    .unwrap_or(0);
                files.push(WorkingTreeFile {
                    path,
                    kind: WorkingTreeFileKind::Symlink,
                    len,
                });
            }
            Ok(meta) if meta.is_file() => {
                let kind = if is_executable(&meta) {
                    WorkingTreeFileKind::Executable
                } else {
                    WorkingTreeFileKind::Regular
                };
                files.push(WorkingTreeFile {
                    path,
                    kind,
                    len: meta.len(),
                });
            }
            Ok(_) => skipped.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(path),
            Err(_) => skipped.push(path),
        }
    }
    Ok(WorkingTreeListing {
        files,
        missing_tracked: missing,
        skipped,
    })
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// 작업 트리 파일 하나를 디스크에서 읽습니다(symlink는 대상 경로 바이트, 상위 symlink 거부).
///
/// 목록을 만든 뒤 파일이 symlink로 바뀌어도 root 밖을 읽지 않습니다. 일반·실행 파일은 마지막
/// component를 `symlink_metadata`로 확인한 뒤 열고, 연 handle의 metadata가 같은 파일(unix: device·
/// inode)인지 다시 확인합니다. 확인과 열기 사이에 symlink로 바뀌면 열린 대상이 달라지므로
/// [`std::io::ErrorKind::InvalidInput`]으로 거부합니다.
pub fn read_working_tree_file(
    worktree_root: &Path,
    file: &WorkingTreeFile,
) -> Result<Vec<u8>, std::io::ErrorKind> {
    let local = checked_local_path(worktree_root, &file.path)?;
    match file.kind {
        WorkingTreeFileKind::Symlink => {
            let target = std::fs::read_link(&local).map_err(|e| e.kind())?;
            Ok(os_bytes(target.as_os_str()))
        }
        _ => read_regular_no_follow(&local, u64::MAX),
    }
}

/// 경로를 검증하고 상위 component가 모두 symlink가 아닌 디렉터리인 root 아래 로컬 경로를 돌려줍니다.
///
/// 상위 component가 symlink면 root 밖을 가리킬 수 있으므로 [`std::io::ErrorKind::InvalidInput`],
/// 상위 디렉터리가 없으면 [`std::io::ErrorKind::NotFound`]입니다. 마지막 component는 확인하지 않습니다.
pub(crate) fn checked_local_path(
    root: &Path,
    path: &GitPath,
) -> Result<PathBuf, std::io::ErrorKind> {
    let components =
        validate_snapshot_path(path.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let mut current = root.to_path_buf();
    for c in &components[..components.len() - 1] {
        let os = GitPath::from_bytes(c)
            .to_path_buf()
            .ok_or(std::io::ErrorKind::InvalidInput)?;
        current.push(os);
        let meta = std::fs::symlink_metadata(&current).map_err(|e| e.kind())?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(std::io::ErrorKind::InvalidInput);
        }
    }
    local_path(root, path).ok_or(std::io::ErrorKind::InvalidInput)
}

/// 마지막 component를 따라가지 않고 일반 파일을 최대 `limit` byte까지 읽습니다(확인 → 열기 → 같은 파일
/// 재확인).
pub(crate) fn read_regular_no_follow(
    local: &Path,
    limit: u64,
) -> Result<Vec<u8>, std::io::ErrorKind> {
    use std::io::Read;
    let before = std::fs::symlink_metadata(local).map_err(|e| e.kind())?;
    if !before.is_file() {
        // symlink·디렉터리·특수 파일로 바뀜.
        return Err(std::io::ErrorKind::InvalidInput);
    }
    let file = std::fs::File::open(local).map_err(|e| e.kind())?;
    let opened = file.metadata().map_err(|e| e.kind())?;
    if !opened.is_file() || !same_file(&before, &opened) {
        return Err(std::io::ErrorKind::InvalidInput);
    }
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| e.kind())?;
    Ok(bytes)
}

/// 두 metadata가 같은 파일을 가리키는지(device·inode).
#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// unix 외 플랫폼은 안정된 file identity API가 없어 종류만 비교합니다(Windows 보류, ADR-0006).
#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.is_file() && b.is_file()
}

#[cfg(unix)]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// 디스크 기준 작업 트리 snapshot을 읽습니다(파일 목록 + loader 결과).
pub fn read_working_tree_snapshot<L: DiskProjectLoader>(
    git: &GatedGitRunner,
    worktree_root: &Path,
    loader: &L,
) -> Result<WorkingTreeSnapshot<L::Project, L::Error>, GitOpError> {
    let WorkingTreeListing {
        files,
        missing_tracked,
        skipped,
    } = list_working_tree_files(git, worktree_root)?;
    let project = loader.load_saved_project(worktree_root);
    Ok(WorkingTreeSnapshot {
        basis: SnapshotBasis::SavedFilesOnDisk,
        root: worktree_root.to_path_buf(),
        files,
        missing_tracked,
        skipped,
        project,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 목록 이후 일반 파일이 root 밖을 가리키는 symlink로 바뀌어도 밖의 내용을 읽지 않습니다(리뷰 G5 회귀).
    #[cfg(unix)]
    #[test]
    fn regular_entry_replaced_by_symlink_is_not_followed() {
        let dir = crate::test_support::TempDir::new("wt-final-symlink");
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let outside = dir.path().join("outside-secret.txt");
        std::fs::write(&outside, b"outside secret\n").unwrap();
        std::fs::write(root.join("sub/a.txt"), b"inside\n").unwrap();
        let listed = WorkingTreeFile {
            path: GitPath::from_bytes(b"sub/a.txt"),
            kind: WorkingTreeFileKind::Regular,
            len: 7,
        };
        assert_eq!(read_working_tree_file(&root, &listed).unwrap(), b"inside\n");

        // 목록 작성 뒤 같은 경로가 root 밖 symlink로 교체됨.
        std::fs::remove_file(root.join("sub/a.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("sub/a.txt")).unwrap();
        assert_eq!(
            read_working_tree_file(&root, &listed),
            Err(std::io::ErrorKind::InvalidInput)
        );
        // 실행 파일 종류로 기록된 항목도 같습니다.
        let executable = WorkingTreeFile {
            kind: WorkingTreeFileKind::Executable,
            ..listed.clone()
        };
        assert_eq!(
            read_working_tree_file(&root, &executable),
            Err(std::io::ErrorKind::InvalidInput)
        );
        // 디렉터리로 바뀐 경우도 거부.
        std::fs::remove_file(root.join("sub/a.txt")).unwrap();
        std::fs::create_dir(root.join("sub/a.txt")).unwrap();
        assert_eq!(
            read_working_tree_file(&root, &listed),
            Err(std::io::ErrorKind::InvalidInput)
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_file_compares_identity_not_content() {
        let dir = crate::test_support::TempDir::new("wt-same-file");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"same").unwrap();
        std::fs::write(&b, b"same").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&b, &link).unwrap();
        let meta = |p: &Path| std::fs::metadata(p).unwrap();
        assert!(same_file(&meta(&a), &meta(&a)));
        assert!(!same_file(&meta(&a), &meta(&b)));
        // 확인 시점에 a였다가 열 때 symlink를 따라 b가 열리는 경우를 구분합니다.
        assert!(!same_file(&meta(&a), &meta(&link)));
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::repo_status::read_repository_status;
        use crate::status::{read_status, StatusEntry, StatusOptions};
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        /// 테스트용 디스크 loader: `stellar.yaml`을 디스크에서 읽습니다.
        struct ManifestLoader;

        impl DiskProjectLoader for ManifestLoader {
            type Project = Vec<u8>;
            type Error = std::io::ErrorKind;

            fn load_saved_project(&self, root: &Path) -> Result<Vec<u8>, std::io::ErrorKind> {
                std::fs::read(root.join("stellar.yaml")).map_err(|e| e.kind())
            }
        }

        /// 저장 대기 메모리 상태를 가진 편집기 문서 모형.
        struct EditorDocument {
            path: PathBuf,
            memory: Vec<u8>,
            dirty: bool,
        }

        impl EditorDocument {
            fn edit(&mut self, text: &[u8]) {
                self.memory = text.to_vec();
                self.dirty = true;
            }

            fn save(&mut self) {
                std::fs::write(&self.path, &self.memory).unwrap();
                self.dirty = false;
            }
        }

        #[test]
        fn unsaved_memory_edit_does_not_appear_in_git_status_or_snapshot() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("wt-unsaved");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "stellar.yaml", b"name: saved\n");
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

            let mut doc = EditorDocument {
                path: repo.join("stellar.yaml"),
                memory: b"name: saved\n".to_vec(),
                dirty: false,
            };
            doc.edit(b"name: unsaved memory edit\n");
            assert!(doc.dirty);

            // 저장 전: Git status는 깨끗하고 snapshot은 디스크 내용.
            let status = read_status(&gate, repo, StatusOptions::default()).unwrap();
            assert!(status.entries.is_empty(), "{:?}", status.entries);
            let summary = read_repository_status(&gate, repo).unwrap();
            assert_eq!(summary.counts.unstaged, 0);
            assert_eq!(summary.basis(), SnapshotBasis::SavedFilesOnDisk);
            let snap = read_working_tree_snapshot(&gate, repo, &ManifestLoader).unwrap();
            assert_eq!(snap.basis, SnapshotBasis::SavedFilesOnDisk);
            assert_eq!(snap.project, Ok(b"name: saved\n".to_vec()));
            let manifest = snap
                .files
                .iter()
                .find(|f| f.path.as_bytes() == b"stellar.yaml")
                .unwrap();
            assert_eq!(manifest.len, b"name: saved\n".len() as u64);
            assert_eq!(
                read_working_tree_file(repo, manifest).unwrap(),
                b"name: saved\n"
            );

            // 저장 후에만 Git status와 snapshot에 나타남.
            doc.save();
            let status = read_status(&gate, repo, StatusOptions::default()).unwrap();
            assert!(matches!(
                &status.entries[..],
                [StatusEntry::Ordinary(e)] if e.path.as_bytes() == b"stellar.yaml"
            ));
            let snap = read_working_tree_snapshot(&gate, repo, &ManifestLoader).unwrap();
            assert_eq!(snap.project, Ok(b"name: unsaved memory edit\n".to_vec()));
        }

        #[test]
        fn file_list_distinguishes_kinds_missing_and_skipped() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("wt-list");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
            std::fs::write(repo.join("a.txt"), "a\n").unwrap();
            std::fs::write(repo.join("gone.txt"), "g\n").unwrap();
            std::fs::write(repo.join("run.sh"), "#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(repo.join("run.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            std::os::unix::fs::symlink("a.txt", repo.join("link")).unwrap();
            git_ok(&runner, repo, ["add", "-A"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "base"]);
            std::fs::remove_file(repo.join("gone.txt")).unwrap();
            std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
            std::fs::write(repo.join("debug.log"), "ignored\n").unwrap();

            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let snap = read_working_tree_snapshot(&gate, repo, &ManifestLoader).unwrap();
            let kinds: Vec<(&[u8], WorkingTreeFileKind)> = snap
                .files
                .iter()
                .map(|f| (f.path.as_bytes(), f.kind))
                .collect();
            use WorkingTreeFileKind::*;
            assert_eq!(
                kinds,
                vec![
                    (&b".gitignore"[..], Regular),
                    (b"a.txt", Regular),
                    (b"link", Symlink),
                    (b"run.sh", Executable),
                    (b"untracked.txt", Regular),
                ]
            );
            assert_eq!(snap.missing_tracked, vec![GitPath::from_bytes(b"gone.txt")]);
            assert!(snap.skipped.is_empty());
            // loader 실패(매니페스트 없음)는 목록과 별개로 보존.
            assert_eq!(snap.project, Err(std::io::ErrorKind::NotFound));
            let link = snap.files.iter().find(|f| f.kind == Symlink).unwrap();
            assert_eq!(read_working_tree_file(repo, link).unwrap(), b"a.txt");
        }
    }
}
