//! 프로젝트 root 검증기 (IV.2.2).
//!
//! 유효한 Stellar 프로젝트 root는 **같은 경로**에 `stellar.yaml`과 Git 링크(`.git` 디렉터리
//! 또는 gitfile)가 모두 있어야 합니다. `.git`이 상위 경로에만 있는 하위 폴더는 root가 아닙니다
//! (이 검증기는 상위로 거슬러 올라가지 않습니다).
//!
//! 본 검증기는 **존재 여부와 동일 경로**만 확인합니다. `stellar.yaml` 내용 파싱은 IV.5가
//! 담당합니다. 일반 Git repo지만 매니페스트가 없는 경우의 사용자 안내 거부는
//! [`classify_project_open`](IV.2.4)이 담당합니다.

use std::path::Path;

/// root에 있는 Git 링크 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitLink {
    /// `.git` 디렉터리(일반 repo).
    Directory,
    /// `.git` gitfile(linked worktree·submodule 등).
    GitFile,
}

/// root 검증 실패 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootError {
    /// 같은 경로에 `stellar.yaml`이 없음.
    MissingManifest,
    /// 같은 경로에 `.git`(디렉터리·gitfile)이 없음.
    MissingGit,
}

impl std::fmt::Display for RootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootError::MissingManifest => f.write_str("같은 경로에 stellar.yaml이 없습니다"),
            RootError::MissingGit => {
                f.write_str("같은 경로에 .git(디렉터리 또는 gitfile)이 없습니다")
            }
        }
    }
}

impl std::error::Error for RootError {}

/// 매니페스트 파일 이름.
pub const MANIFEST_FILE: &str = "stellar.yaml";
/// Git 링크 이름.
pub const GIT_LINK: &str = ".git";

/// 주어진 경로가 유효한 Stellar 프로젝트 root인지 검증합니다.
///
/// `stellar.yaml`이 없으면 `MissingManifest`, 같은 경로에 `.git`이 없으면 `MissingGit`을
/// 반환합니다. 둘 다 있으면 Git 링크 종류를 반환합니다.
pub fn validate_project_root(path: &Path) -> Result<GitLink, RootError> {
    if !path.join(MANIFEST_FILE).is_file() {
        return Err(RootError::MissingManifest);
    }
    let git = path.join(GIT_LINK);
    if git.is_dir() {
        Ok(GitLink::Directory)
    } else if git.is_file() {
        Ok(GitLink::GitFile)
    } else {
        Err(RootError::MissingGit)
    }
}

/// 프로젝트 열기 분류 거부 사유 (IV.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// `.git`은 있지만 `stellar.yaml`이 없음 → Stellar 프로젝트가 아님(자동 변환 안 함).
    NotAStellarProject,
    /// `.git`이 없어 Git 저장소가 아님.
    NotARepository,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotAStellarProject => f.write_str(
                "Git 저장소이지만 stellar.yaml이 없어 Stellar 프로젝트가 아닙니다. \
                 자동 변환은 지원하지 않습니다.",
            ),
            OpenError::NotARepository => f.write_str(".git이 없어 Git 저장소가 아닙니다."),
        }
    }
}

impl std::error::Error for OpenError {}

/// 경로를 Stellar 프로젝트로 열 수 있는지 분류합니다(IV.2.4).
///
/// 유효하면 [`GitLink`]를, 일반 Git repo지만 매니페스트가 없으면 [`OpenError::NotAStellarProject`],
/// Git repo가 아니면 [`OpenError::NotARepository`]를 반환합니다. **자동 변환을 하지 않습니다**
/// (파일을 만들거나 고치지 않음).
pub fn classify_project_open(path: &Path) -> Result<GitLink, OpenError> {
    let manifest = path.join(MANIFEST_FILE).is_file();
    let git = path.join(GIT_LINK);
    let git_kind = if git.is_dir() {
        Some(GitLink::Directory)
    } else if git.is_file() {
        Some(GitLink::GitFile)
    } else {
        None
    };
    match (manifest, git_kind) {
        (true, Some(kind)) => Ok(kind),
        (false, Some(_)) => Err(OpenError::NotAStellarProject),
        (_, None) => Err(OpenError::NotARepository),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 테스트용 임시 디렉터리(드롭 시 정리).
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-root-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
            TempDir { path }
        }

        fn child(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(path: PathBuf, contents: &str) {
        std::fs::write(path, contents).expect("파일 쓰기");
    }

    #[test]
    fn normal_repo_with_git_directory() {
        let dir = TempDir::new();
        write_file(dir.child(MANIFEST_FILE), "schema_version: 1\n");
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        assert_eq!(validate_project_root(&dir.path), Ok(GitLink::Directory));
    }

    #[test]
    fn linked_worktree_with_gitfile() {
        let dir = TempDir::new();
        write_file(dir.child(MANIFEST_FILE), "schema_version: 1\n");
        // gitfile: `.git`이 디렉터리가 아니라 gitdir를 가리키는 파일.
        write_file(dir.child(GIT_LINK), "gitdir: /repo/.git/worktrees/wt\n");
        assert_eq!(validate_project_root(&dir.path), Ok(GitLink::GitFile));
    }

    #[test]
    fn subfolder_with_only_parent_git_is_not_root() {
        let dir = TempDir::new();
        // 부모에만 .git, 하위 폴더에 stellar.yaml.
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        let sub = dir.child("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_file(sub.join(MANIFEST_FILE), "schema_version: 1\n");
        // 하위 폴더에는 .git이 없으므로 root가 아님.
        assert_eq!(validate_project_root(&sub), Err(RootError::MissingGit));
    }

    #[test]
    fn missing_manifest_is_rejected() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        assert_eq!(
            validate_project_root(&dir.path),
            Err(RootError::MissingManifest)
        );
    }

    #[test]
    fn missing_git_is_rejected() {
        let dir = TempDir::new();
        write_file(dir.child(MANIFEST_FILE), "schema_version: 1\n");
        assert_eq!(validate_project_root(&dir.path), Err(RootError::MissingGit));
    }

    #[test]
    fn empty_dir_reports_missing_manifest_first() {
        let dir = TempDir::new();
        assert_eq!(
            validate_project_root(&dir.path),
            Err(RootError::MissingManifest)
        );
    }

    #[test]
    fn manifest_as_directory_is_not_a_manifest() {
        // stellar.yaml이 디렉터리면 매니페스트 파일이 아님.
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.child(MANIFEST_FILE)).unwrap();
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        assert_eq!(
            validate_project_root(&dir.path),
            Err(RootError::MissingManifest)
        );
    }

    // --- IV.2.4: 열기 분류 ---

    #[test]
    fn git_repo_without_manifest_is_not_a_stellar_project() {
        // 일반 Git repo(.git 디렉터리)이지만 stellar.yaml 없음.
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        assert_eq!(
            classify_project_open(&dir.path),
            Err(OpenError::NotAStellarProject)
        );
        // gitfile(worktree)인 경우에도 동일.
        let dir2 = TempDir::new();
        write_file(dir2.child(GIT_LINK), "gitdir: /repo/.git\n");
        assert_eq!(
            classify_project_open(&dir2.path),
            Err(OpenError::NotAStellarProject)
        );
    }

    #[test]
    fn valid_stellar_project_classifies_ok() {
        let dir = TempDir::new();
        write_file(dir.child(MANIFEST_FILE), "schema_version: 1\n");
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        assert_eq!(classify_project_open(&dir.path), Ok(GitLink::Directory));
    }

    #[test]
    fn non_repository_is_classified_as_not_a_repository() {
        // .git 없음(매니페스트 유무 무관).
        let empty = TempDir::new();
        assert_eq!(
            classify_project_open(&empty.path),
            Err(OpenError::NotARepository)
        );
        let manifest_only = TempDir::new();
        write_file(manifest_only.child(MANIFEST_FILE), "schema_version: 1\n");
        assert_eq!(
            classify_project_open(&manifest_only.path),
            Err(OpenError::NotARepository)
        );
    }

    #[test]
    fn classify_does_not_create_manifest() {
        // 자동 변환 없음: 분류 후에도 stellar.yaml이 생기지 않음.
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.child(GIT_LINK)).unwrap();
        let _ = classify_project_open(&dir.path);
        assert!(!dir.child(MANIFEST_FILE).exists());
    }
}
