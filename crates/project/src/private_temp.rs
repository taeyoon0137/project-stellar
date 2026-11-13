//! 비공개 임시 디렉터리와 정리 (XVI.7.5).
//!
//! clone staging(IV.6.2)과 Git snapshot materialize(XIV.2.6) 같은 작업은 프로젝트 내용(비공개
//! 저장소일 수 있음)을 잠시 디스크에 둡니다. 이 모듈은 그 임시 디렉터리를 같은 규칙으로 만들고 지웁니다.
//!
//! - **권한**: unix에서 `0700`으로 **만들면서** 설정합니다(`DirBuilder::mode`, 만든 뒤 바꾸는 사이의 창
//!   없음). umask가 소유자 비트까지 지우는 환경에 대비해 만든 직후 정확히 `0700`으로 맞추고, 만든 항목이
//!   symlink가 아닌 실제 디렉터리이고 이 프로세스의 effective 사용자 소유이며 group·other 비트가 없음을
//!   다시 확인합니다. 다른 사용자는 목록과 내용 어디에도 접근하지 못합니다(하위 항목의 권한과 무관하게
//!   root 디렉터리에서 막힘).
//! - **표식**: 디렉터리 안에 `0600` 표식 파일([`PRIVATE_TEMP_MARKER`])을 둡니다. 잔여 정리는 표식이 있는
//!   디렉터리만 지웁니다(같은 접두사를 쓰는 다른 항목을 지우지 않음).
//! - **정리**: [`PrivateTempDir`]는 drop 시 지웁니다(오류·panic 경로 포함). 쓰기 권한이 없는 하위
//!   디렉터리(읽기 전용으로 만들어진 checkout 등)가 있어도 symlink를 따르지 않고 소유자 권한을 되돌린 뒤
//!   다시 지웁니다(명시 stack으로 반복 순회, 재귀 없음). [`PrivateTempDir::cleanup`]은 실패를 돌려줍니다.
//! - **잔여 정리**: 비정상 종료로 남은 디렉터리는 [`sweep_stale_private_temps`]가 지웁니다. 이름에 기록한
//!   process id가 아직 살아 있으면(또는 이 process 자신이면) 건너뜁니다.
//!
//! Windows는 ADR-0006에 따라 지원을 미룹니다. 비-unix 빌드는 권한 검사를 하지 않고 잔여 정리도 하지
//! 않습니다(어떤 process도 죽었다고 판정하지 않음).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 비공개 임시 디렉터리 안의 표식 파일 이름.
pub const PRIVATE_TEMP_MARKER: &str = ".stellar-private-temp";

/// 비공개 임시 디렉터리 권한(unix): 소유자만 읽기·쓰기·실행.
pub const PRIVATE_DIR_MODE: u32 = 0o700;

/// 표식 파일 권한(unix): 소유자만 읽기·쓰기.
const MARKER_FILE_MODE: u32 = 0o600;

/// 이름 충돌 시 다시 시도하는 횟수.
const CREATE_ATTEMPTS: usize = 64;

/// 비공개 임시 디렉터리 생성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateTempError {
    /// 접두사가 비었거나 `[A-Za-z0-9._-]` 밖의 문자를 씀.
    InvalidPrefix {
        /// 요청 접두사.
        prefix: String,
    },
    /// 파일 시스템 오류.
    Io {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 만든 디렉터리가 비공개 조건(실제 디렉터리·소유자·`0700`)을 만족하지 않음(지웠음).
    NotPrivate {
        /// 관찰한 권한 비트(unix, 하위 12비트).
        mode: u32,
    },
    /// 고유 이름을 얻지 못함.
    NameExhausted,
}

impl fmt::Display for PrivateTempError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateTempError::InvalidPrefix { prefix } => {
                write!(f, "임시 디렉터리 접두사가 잘못되었습니다: {prefix:?}")
            }
            PrivateTempError::Io { kind } => write!(f, "임시 디렉터리 오류: {kind:?}"),
            PrivateTempError::NotPrivate { mode } => {
                write!(f, "임시 디렉터리가 비공개가 아닙니다(mode {mode:o})")
            }
            PrivateTempError::NameExhausted => {
                f.write_str("고유한 임시 디렉터리 이름을 얻지 못했습니다")
            }
        }
    }
}

impl std::error::Error for PrivateTempError {}

fn io_error(e: &io::Error) -> PrivateTempError {
    PrivateTempError::Io { kind: e.kind() }
}

/// 소유자만 접근할 수 있는 임시 디렉터리(drop 시 삭제, [`PrivateTempDir::keep`]으로 보존).
#[derive(Debug)]
pub struct PrivateTempDir {
    path: PathBuf,
    armed: bool,
}

impl PrivateTempDir {
    /// `parent` 아래에 `<prefix>-<pid>-<n>-<nanos>` 이름의 비공개 임시 디렉터리를 만듭니다.
    pub fn create_in(parent: &Path, prefix: &str) -> Result<PrivateTempDir, PrivateTempError> {
        validate_prefix(prefix)?;
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        for _ in 0..CREATE_ATTEMPTS {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = parent.join(format!("{prefix}-{}-{n}-{nanos:08x}", std::process::id()));
            match create_private_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error(&e)),
            }
            // 여기부터 실패하면 drop이 지웁니다.
            let dir = PrivateTempDir { path, armed: true };
            dir.verify_private()?;
            write_marker(&dir.path).map_err(|e| io_error(&e))?;
            return Ok(dir);
        }
        Err(PrivateTempError::NameExhausted)
    }

    /// 디렉터리 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// drop 시 지우지 않고 경로를 돌려줍니다(호출자가 정리 책임).
    pub fn keep(mut self) -> PathBuf {
        self.armed = false;
        std::mem::take(&mut self.path)
    }

    /// 지금 지우고 실패를 돌려줍니다(실패해도 다시 시도하지 않음).
    pub fn cleanup(mut self) -> io::Result<()> {
        self.armed = false;
        remove_private_tree(&self.path)
    }

    /// 만든 디렉터리가 비공개 조건을 만족하는지 확인합니다.
    fn verify_private(&self) -> Result<(), PrivateTempError> {
        let meta = std::fs::symlink_metadata(&self.path).map_err(|e| io_error(&e))?;
        let mode = permission_bits(&meta);
        if !meta.is_dir() || meta.file_type().is_symlink() || !owned_by_current_user(&meta) {
            return Err(PrivateTempError::NotPrivate { mode });
        }
        #[cfg(unix)]
        if mode != PRIVATE_DIR_MODE {
            return Err(PrivateTempError::NotPrivate { mode });
        }
        Ok(())
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_private_tree(&self.path);
        }
    }
}

/// 접두사 규칙: 비어 있지 않은 `[A-Za-z0-9._-]`이고 `.`·`..`가 아님.
fn validate_prefix(prefix: &str) -> Result<(), PrivateTempError> {
    let valid = !prefix.is_empty()
        && prefix != "."
        && prefix != ".."
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(PrivateTempError::InvalidPrefix {
            prefix: prefix.to_string(),
        })
    }
}

/// `0700` 디렉터리를 만들고(이미 있으면 `AlreadyExists`) umask와 무관하게 정확히 `0700`으로 맞춥니다.
fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(PRIVATE_DIR_MODE);
    }
    builder.create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        {
            let _ = std::fs::remove_dir(path);
            return Err(e);
        }
    }
    Ok(())
}

/// 표식 파일을 `0600`으로 새로 만듭니다.
fn write_marker(dir: &Path) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(MARKER_FILE_MODE);
    }
    let file = options.open(dir.join(PRIVATE_TEMP_MARKER))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(MARKER_FILE_MODE))?;
    }
    drop(file);
    Ok(())
}

#[cfg(unix)]
fn permission_bits(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permission_bits(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn owned_by_current_user(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == sys::current_uid()
}

#[cfg(not(unix))]
fn owned_by_current_user(_meta: &std::fs::Metadata) -> bool {
    true
}

/// 비공개 임시 트리를 지웁니다. 없으면 성공입니다.
///
/// 먼저 그대로 지우고, 실패하면 트리를 symlink를 따르지 않고 반복 순회하며 실제 디렉터리의 권한을
/// 소유자 `rwx`로 되돌린 뒤 한 번 더 지웁니다(다른 사용자의 파일·외부 경로는 건드리지 않음). `path`
/// 자체가 symlink면 링크만 지웁니다.
pub fn remove_private_tree(path: &Path) -> io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return std::fs::remove_file(path);
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    restore_owner_access(path);
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// 트리의 실제 디렉터리(이 사용자 소유)를 소유자 `rwx`로 되돌립니다(반복, symlink 미추종).
#[cfg(unix)]
fn restore_owner_access(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() || !owned_by_current_user(&meta) {
            continue;
        }
        let mode = permission_bits(&meta) | 0o700;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(entry.path());
            }
        }
    }
}

#[cfg(not(unix))]
fn restore_owner_access(_root: &Path) {}

/// 잔여 정리 결과(각 목록은 경로 순).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// 지운 잔여 디렉터리.
    pub removed: Vec<PathBuf>,
    /// 만든 process가 살아 있거나 이 process라 남긴 디렉터리.
    pub live: Vec<PathBuf>,
    /// 지우지 못한 디렉터리와 오류 종류.
    pub failed: Vec<(PathBuf, io::ErrorKind)>,
}

/// `parent` 아래에서 `prefix`로 만든 잔여 비공개 임시 디렉터리를 지웁니다.
///
/// 다음을 모두 만족하는 항목만 지웁니다: 이름이 `<prefix>-<pid>-…`, symlink가 아닌 실제 디렉터리,
/// 이 사용자 소유, 권한 `0700`(unix), 표식 파일이 일반 파일로 있음, `pid` process가 없음. 그 밖의 항목은
/// 보고하지 않고 그대로 둡니다.
pub fn sweep_stale_private_temps(
    parent: &Path,
    prefix: &str,
) -> Result<SweepReport, PrivateTempError> {
    validate_prefix(prefix)?;
    let mut report = SweepReport::default();
    let head = format!("{prefix}-");
    let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(parent)
        .map_err(|e| io_error(&e))?
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .collect();
    names.sort();
    for name in names {
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(pid) = text
            .strip_prefix(&head)
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        let path = parent.join(&name);
        if !is_private_temp(&path) {
            continue;
        }
        if pid == std::process::id() || process_may_be_alive(pid) {
            report.live.push(path);
            continue;
        }
        match remove_private_tree(&path) {
            Ok(()) => report.removed.push(path),
            Err(e) => report.failed.push((path, e.kind())),
        }
    }
    Ok(report)
}

/// 이 모듈이 만든 비공개 임시 디렉터리처럼 보이는지(실제 디렉터리·소유자·`0700`·표식).
fn is_private_temp(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || meta.file_type().is_symlink() || !owned_by_current_user(&meta) {
        return false;
    }
    #[cfg(unix)]
    if permission_bits(&meta) != PRIVATE_DIR_MODE {
        return false;
    }
    std::fs::symlink_metadata(path.join(PRIVATE_TEMP_MARKER))
        .is_ok_and(|marker| marker.is_file() && !marker.file_type().is_symlink())
}

/// `pid` process가 있을 수 있는지. 확실히 없을 때(`ESRCH`)만 `false`입니다.
#[cfg(unix)]
fn process_may_be_alive(pid: u32) -> bool {
    match i32::try_from(pid) {
        Ok(pid) if pid > 0 => sys::process_may_exist(pid),
        _ => true,
    }
}

#[cfg(not(unix))]
fn process_may_be_alive(_pid: u32) -> bool {
    true
}

/// 최소 libc 선언(macOS `<unistd.h>`·`<signal.h>`). libc crate 없이 안전한 wrapper로만 씁니다.
#[cfg(unix)]
mod sys {
    extern "C" {
        /// `uid_t geteuid(void)`.
        fn geteuid() -> u32;
        /// `int kill(pid_t pid, int sig)`.
        fn kill(pid: i32, sig: i32) -> i32;
    }

    /// `ESRCH`(macOS·Linux 공통 값): 그런 process가 없음.
    const ESRCH: i32 = 3;

    /// 현재 프로세스의 effective 사용자 id.
    pub(super) fn current_uid() -> u32 {
        // SAFETY: geteuid는 인자가 없고 항상 성공하며 메모리를 읽거나 쓰지 않습니다.
        unsafe { geteuid() }
    }

    /// `pid`(> 0) process가 있을 수 있는지. signal 0은 아무것도 보내지 않고 존재·권한만 검사합니다.
    pub(super) fn process_may_exist(pid: i32) -> bool {
        // SAFETY: kill은 정수 인자만 받고 메모리를 읽거나 쓰지 않습니다. sig 0은 signal을 보내지
        // 않으며 pid > 0이라 process group 전체를 가리키지 않습니다.
        let rc = unsafe { kill(pid, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    /// 어떤 OS에서도 쓰이지 않는 process id(macOS 최대 99998, Linux `pid_max` 최대 4194304보다 큼).
    const DEAD_PID: u32 = 99_999_999;
    const PREFIX: &str = ".stellar-test-private";

    /// 테스트 작업 디렉터리(일반 권한, drop 시 권한을 되돌려 삭제).
    struct Work {
        path: PathBuf,
    }
    impl Work {
        fn new() -> Work {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-private-temp-test-{}-{n}",
                std::process::id()
            ));
            let _ = remove_private_tree(&path);
            std::fs::create_dir_all(&path).unwrap();
            Work { path }
        }
    }
    impl Drop for Work {
        fn drop(&mut self) {
            let _ = remove_private_tree(&self.path);
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    }

    #[test]
    fn private_dir_is_owner_only_with_marker_and_removed_on_drop() {
        let work = Work::new();
        let dir = PrivateTempDir::create_in(&work.path, PREFIX).unwrap();
        let path = dir.path().to_path_buf();
        assert_eq!(path.parent(), Some(work.path.as_path()));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(&format!("{PREFIX}-{}-", std::process::id())));
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
        assert_eq!(mode(&path), 0o700, "소유자만 rwx");
        assert_eq!(mode(&path) & 0o077, 0, "group·other 접근 없음");
        assert_eq!(meta.uid(), sys::current_uid(), "이 사용자 소유");
        let marker = path.join(PRIVATE_TEMP_MARKER);
        assert!(std::fs::symlink_metadata(&marker).unwrap().is_file());
        assert_eq!(mode(&marker), 0o600);
        // 같은 parent에 여럿 만들어도 이름이 겹치지 않음.
        let other = PrivateTempDir::create_in(&work.path, PREFIX).unwrap();
        assert_ne!(other.path(), dir.path());

        drop(dir);
        assert!(!path.exists(), "drop 시 삭제");
        let kept = other.keep();
        assert!(kept.is_dir(), "keep은 보존");
        remove_private_tree(&kept).unwrap();
        assert!(!kept.exists());
    }

    #[test]
    fn cleanup_restores_owner_access_without_following_links() {
        let work = Work::new();
        // 프로젝트 밖(링크 대상): 권한·내용이 바뀌면 안 됨.
        let outside = work.path.join("outside");
        std::fs::create_dir_all(outside.join("inner")).unwrap();
        std::fs::write(outside.join("inner/keep.txt"), b"keep").unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o555)).unwrap();

        let dir = PrivateTempDir::create_in(&work.path, PREFIX).unwrap();
        let root = dir.path().to_path_buf();
        // 읽기 전용 디렉터리·권한 없는 디렉터리·읽기 전용 파일·외부 링크.
        std::fs::create_dir_all(root.join("checkout/objects/ab")).unwrap();
        std::fs::write(root.join("checkout/objects/ab/blob"), b"x").unwrap();
        std::fs::set_permissions(
            root.join("checkout/objects/ab/blob"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        std::fs::set_permissions(
            root.join("checkout/objects/ab"),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("checkout/locked/deep")).unwrap();
        std::fs::set_permissions(
            root.join("checkout/locked"),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, root.join("checkout/link")).unwrap();
        assert!(
            std::fs::remove_dir_all(&root).is_err(),
            "fixture: 그냥 지우면 실패해야 함"
        );

        dir.cleanup().unwrap();
        assert!(!root.exists(), "강화 정리로 삭제");
        assert_eq!(mode(&outside), 0o555, "링크 대상 권한 미변경");
        assert_eq!(
            std::fs::read(outside.join("inner/keep.txt")).unwrap(),
            b"keep"
        );
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        // 없는 경로 정리는 성공.
        remove_private_tree(&root).unwrap();
    }

    #[test]
    fn drop_after_failure_paths_cleans_up() {
        let work = Work::new();
        let path = {
            let dir = PrivateTempDir::create_in(&work.path, PREFIX).unwrap();
            std::fs::create_dir_all(dir.path().join("partial")).unwrap();
            std::fs::write(dir.path().join("partial/half-written"), b"h").unwrap();
            std::fs::set_permissions(
                dir.path().join("partial"),
                std::fs::Permissions::from_mode(0o500),
            )
            .unwrap();
            std::fs::create_dir_all(dir.path().join("x")).unwrap();
            dir.path().to_path_buf()
        }; // 오류 경로처럼 명시적 정리 없이 drop.
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&work.path).unwrap().count(), 0);
    }

    #[test]
    fn invalid_prefixes_are_rejected() {
        let work = Work::new();
        for prefix in ["", ".", "..", "a/b", "a b", "한글", "a\\b"] {
            assert_eq!(
                PrivateTempDir::create_in(&work.path, prefix).map(|_| ()),
                Err(PrivateTempError::InvalidPrefix {
                    prefix: prefix.to_string()
                }),
                "{prefix:?}"
            );
        }
        assert!(matches!(
            PrivateTempDir::create_in(&work.path.join("missing"), PREFIX),
            Err(PrivateTempError::Io {
                kind: io::ErrorKind::NotFound
            })
        ));
        assert_eq!(std::fs::read_dir(&work.path).unwrap().count(), 0);
    }

    /// 잔여 디렉터리를 흉내 냅니다(`mode`, 표식 여부).
    fn leftover(parent: &Path, name: &str, dir_mode: u32, marker: bool) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(path.join("checkout/sub")).unwrap();
        std::fs::write(path.join("checkout/sub/data"), b"d").unwrap();
        if marker {
            std::fs::write(path.join(PRIVATE_TEMP_MARKER), b"").unwrap();
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(dir_mode)).unwrap();
        path
    }

    #[test]
    fn sweep_removes_only_stale_marked_private_dirs() {
        let work = Work::new();
        let parent = &work.path;
        let stale = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-0-0"), 0o700, true);
        // 읽기 전용 하위 디렉터리가 있는 잔여도 지움.
        std::fs::set_permissions(
            stale.join("checkout/sub"),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();
        let own = PrivateTempDir::create_in(parent, PREFIX).unwrap();
        let init = leftover(parent, &format!("{PREFIX}-1-0-0"), 0o700, true);
        let unmarked = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-1-0"), 0o700, false);
        let public = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-2-0"), 0o755, true);
        let target = leftover(parent, "link-target", 0o700, true);
        let link = parent.join(format!("{PREFIX}-{DEAD_PID}-3-0"));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let other_prefix = leftover(parent, &format!("other-{DEAD_PID}-0-0"), 0o700, true);
        let not_pid = leftover(parent, &format!("{PREFIX}-abc-0-0"), 0o700, true);

        let report = sweep_stale_private_temps(parent, PREFIX).unwrap();
        assert_eq!(report.removed, vec![stale.clone()]);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        let mut live = vec![own.path().to_path_buf(), init.clone()];
        live.sort();
        assert_eq!(report.live, live, "살아 있는 process(이 process·pid 1)");
        assert!(!stale.exists());
        for kept in [&init, &unmarked, &public, &target, &other_prefix, &not_pid] {
            assert!(kept.join("checkout/sub/data").exists(), "{kept:?}");
        }
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(own.path().is_dir());
    }
}
