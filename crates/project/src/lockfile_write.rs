//! resolve 결과 `stellar.lock.yaml` 기록 transaction (XV.4.5).
//!
//! resolve가 성공하면 resolver 결과([`LockDocument`])만으로 canonical lock 텍스트를 만들어 사용
//! 프로젝트 root의 `stellar.lock.yaml`에 기록합니다. 이 기록은 문서 자동 저장(XIII.1, `design/`
//! generation)과 **별도의** project file transaction이며 다음을 지킵니다.
//!
//! - **입력은 resolver 결과뿐**: [`LockDocument`] → [`Lockfile::write_checked`]. 사용자 편집 필드를
//!   받는 경로가 없고, 다시 읽히지 않을 모델·텍스트는 파일을 건드리기 전에 거부합니다(IV.1.7).
//! - **원자적 교체**: 같은 디렉터리 임시 파일 → `fsync` → `rename` → 부모 디렉터리 `fsync`. 실패하면
//!   임시 파일을 지우고 기존 lockfile을 그대로 둡니다.
//! - **변화 없으면 쓰지 않음**: 기존 bytes가 같으면 파일을 다시 쓰지 않습니다(mtime·Git status 불변).
//! - **기존 권한 유지**: 기존 lockfile의 권한 bit를 새 파일에 옮겨 Git mode 변경을 만들지 않습니다.
//! - **symlink는 교체하지 않음**: lockfile이 symlink면 rename이 링크를 일반 파일로 바꿔 Git 항목
//!   종류가 달라지므로 거부합니다.
//! - **자동 commit 없음**: Git을 실행하지 않습니다. 변경은 사용자가 검토·commit합니다.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_dependencies::lock::LockDocument;

use crate::lock::{LockWriteError, Lockfile};

/// 잠금 파일 이름(사용 프로젝트 root 기준).
pub const LOCK_FILE: &str = "stellar.lock.yaml";

/// 기록 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfileWriteOutcome {
    /// 기존 lockfile과 bytes가 같아 쓰지 않음.
    Unchanged,
    /// 새 lockfile을 원자적으로 기록함.
    Written,
}

/// 기록 실패. 경로 원문은 담지 않습니다(XV.1.4 경로 민감값).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockfileWriteError {
    /// lock 모델·텍스트 생성 실패(파일은 건드리지 않음).
    Encode(LockWriteError),
    /// 기존 lockfile이 symlink라 교체하지 않음.
    SymlinkLockfile,
    /// 파일 시스템 작업 실패(기존 lockfile 유지).
    Io {
        /// 실패한 단계.
        stage: &'static str,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for LockfileWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockfileWriteError::Encode(e) => write!(f, "{e}"),
            LockfileWriteError::SymlinkLockfile => {
                f.write_str("stellar.lock.yaml이 symlink라 교체하지 않습니다")
            }
            LockfileWriteError::Io { stage, kind } => {
                write!(f, "stellar.lock.yaml 기록 실패({stage}): {kind:?}")
            }
        }
    }
}

impl std::error::Error for LockfileWriteError {}

fn io_error(stage: &'static str) -> impl Fn(io::Error) -> LockfileWriteError {
    move |e| LockfileWriteError::Io {
        stage,
        kind: e.kind(),
    }
}

/// resolve 결과를 `project_root/stellar.lock.yaml`에 canonical 텍스트로 기록합니다.
pub fn write_resolved_lockfile(
    project_root: &Path,
    document: &LockDocument,
) -> Result<LockfileWriteOutcome, LockfileWriteError> {
    let text = Lockfile::from_document(document.clone())
        .write_checked()
        .map_err(LockfileWriteError::Encode)?;
    let target = project_root.join(LOCK_FILE);

    let existing = match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(LockfileWriteError::SymlinkLockfile)
        }
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_error("stat")(e)),
    };
    if existing.is_some() {
        let current = std::fs::read(&target).map_err(io_error("read"))?;
        if current == text.as_bytes() {
            return Ok(LockfileWriteOutcome::Unchanged);
        }
    }

    let tmp = temp_path(project_root);
    let result = write_and_replace(&tmp, &target, text.as_bytes(), existing.as_ref());
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sync_dir(project_root).map_err(io_error("sync-dir"))?;
    Ok(LockfileWriteOutcome::Written)
}

/// 같은 디렉터리의 고유 임시 경로(`.stellar.lock.yaml.tmp-<pid>-<n>`).
fn temp_path(project_root: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    project_root.join(format!(
        ".{LOCK_FILE}.tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_and_replace(
    tmp: &Path,
    target: &Path,
    bytes: &[u8],
    existing: Option<&std::fs::Metadata>,
) -> Result<(), LockfileWriteError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
        .map_err(io_error("create-temp"))?;
    file.write_all(bytes).map_err(io_error("write-temp"))?;
    if let Some(meta) = existing {
        file.set_permissions(meta.permissions())
            .map_err(io_error("permissions"))?;
    }
    file.sync_all().map_err(io_error("sync-temp"))?;
    drop(file);
    std::fs::rename(tmp, target).map_err(io_error("rename"))
}

/// 디렉터리 항목을 디스크에 반영합니다(Unix fsync, Windows는 ADR-0006 보류로 생략).
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::fixtures::entry;
    use crate::manifest_source::test_git::{TempDir, TestGit};
    use crate::yaml::load_restricted;

    fn document(entries: Vec<crate::lock::ResolvedDependency>) -> LockDocument {
        Lockfile {
            schema_version: crate::lock::CURRENT_LOCK_SCHEMA_VERSION,
            resolved: entries,
        }
        .to_document()
        .expect("canonical document")
    }

    fn resolved() -> LockDocument {
        document(vec![
            entry(1, 0xa1, &["a"], &[(3, 0xc1)]),
            entry(3, 0xc1, &["a", "c"], &[]),
        ])
    }

    /// 기본 scaffold를 Git 저장소로 commit한 사용 프로젝트.
    fn committed_project(tmp: &TempDir, git: &TestGit) -> PathBuf {
        let root = tmp.path().join("app");
        std::fs::create_dir(&root).unwrap();
        crate::scaffold::write_default_scaffold(
            &root,
            "App",
            "0195c530-3acd-7bc1-8414-0000000000ff",
        )
        .unwrap();
        std::fs::write(root.join("design/keep.txt"), "design\n").unwrap();
        git.init_repo(&root);
        git.ok(&root, ["add", "-A"]);
        git.ok(&root, ["commit", "-q", "-m", "init"]);
        root
    }

    fn status(git: &TestGit, root: &Path) -> String {
        git.ok(
            root,
            [
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignored",
            ],
        )
    }

    #[test]
    fn resolved_lockfile_write_changes_only_the_lockfile_in_git_status() {
        let tmp = TempDir::new("lockfile-write");
        let git = TestGit::new(tmp.path());
        let root = committed_project(&tmp, &git);
        let head = git.ok(&root, ["rev-parse", "HEAD"]);
        assert_eq!(status(&git, &root), "");
        let manifest_before = std::fs::read(root.join("stellar.yaml")).unwrap();

        assert_eq!(
            write_resolved_lockfile(&root, &resolved()),
            Ok(LockfileWriteOutcome::Written)
        );
        // Git status에는 lockfile 수정 하나만(임시 파일·다른 파일 변화 없음), 자동 commit 없음.
        assert_eq!(status(&git, &root), " M stellar.lock.yaml\n");
        assert_eq!(git.ok(&root, ["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read(root.join("stellar.yaml")).unwrap(),
            manifest_before
        );
        // 기록된 텍스트는 canonical이며 실제 parser로 같은 resolver 결과가 됨.
        let text = std::fs::read_to_string(root.join(LOCK_FILE)).unwrap();
        assert_eq!(
            text,
            Lockfile::from_document(resolved()).write_checked().unwrap()
        );
        let reloaded = Lockfile::from_yaml(&load_restricted(&text).unwrap()).unwrap();
        assert_eq!(reloaded.to_document().unwrap(), resolved());

        // 같은 결과를 다시 쓰면 파일을 건드리지 않음(mtime 유지).
        let mtime = std::fs::metadata(root.join(LOCK_FILE))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            write_resolved_lockfile(&root, &resolved()),
            Ok(LockfileWriteOutcome::Unchanged)
        );
        assert_eq!(
            std::fs::metadata(root.join(LOCK_FILE))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
        assert_eq!(status(&git, &root), " M stellar.lock.yaml\n");

        // 빈 결과로 되돌리면 scaffold의 빈 lockfile과 byte 동일 → Git status 깨끗.
        assert_eq!(
            write_resolved_lockfile(&root, &document(Vec::new())),
            Ok(LockfileWriteOutcome::Written)
        );
        assert_eq!(status(&git, &root), "");
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_kept_and_symlinked_lockfile_is_not_replaced() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new("lockfile-perm");
        let git = TestGit::new(tmp.path());
        let root = committed_project(&tmp, &git);
        let lock = root.join(LOCK_FILE);
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_resolved_lockfile(&root, &resolved()).unwrap();
        assert_eq!(
            std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // symlink lockfile은 교체하지 않고(링크 대상도 그대로) 오류.
        let target = tmp.path().join("elsewhere.yaml");
        std::fs::write(&target, "original\n").unwrap();
        std::fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink(&target, &lock).unwrap();
        assert_eq!(
            write_resolved_lockfile(&root, &resolved()),
            Err(LockfileWriteError::SymlinkLockfile)
        );
        assert!(std::fs::symlink_metadata(&lock)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original\n");
    }

    #[cfg(unix)]
    #[test]
    fn failed_write_keeps_existing_lockfile_and_leaves_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new("lockfile-fail");
        let git = TestGit::new(tmp.path());
        let root = committed_project(&tmp, &git);
        let before = std::fs::read(root.join(LOCK_FILE)).unwrap();
        // 프로젝트 root를 쓰기 금지로 만들어 임시 파일 생성 단계를 실패시킴.
        let original = std::fs::metadata(&root).unwrap().permissions();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = write_resolved_lockfile(&root, &resolved());
        std::fs::set_permissions(&root, original).unwrap();

        let err = result.unwrap_err();
        assert_eq!(
            err,
            LockfileWriteError::Io {
                stage: "create-temp",
                kind: io::ErrorKind::PermissionDenied
            }
        );
        assert!(!err.to_string().contains(tmp.path().to_str().unwrap()));
        // 기존 lockfile 유지, 임시 파일 없음, Git status 깨끗.
        assert_eq!(std::fs::read(root.join(LOCK_FILE)).unwrap(), before);
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        assert_eq!(status(&git, &root), "");
    }
}
