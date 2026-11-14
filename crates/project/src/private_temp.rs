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
//!   process id가 아직 살아 있으면(또는 이 process 자신이면) 건너뜁니다. pid 검사는 이 기계에서만 의미가
//!   있으므로 표식에 만든 host 이름과 기계 식별자(`host=<이름>`, `machine=<식별자>`)를 기록하고, 둘 다 이
//!   기계와 같은 표식만 지웁니다(공유·네트워크 volume에서 다른 기계의 살아 있는 process가 쓰는 디렉터리를
//!   지우지 않음). host 이름은 기본 이름 등으로 기계끼리 겹칠 수 있으므로 기계 식별자(macOS
//!   `gethostuuid`, 그 밖의 unix `/etc/machine-id`)가 없으면 표식을 비워 두고 잔여 정리도 하지 않습니다.
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

/// 표식 파일을 `0600`으로 새로 만들고 만든 host를 기록합니다([`marker_contents`]).
fn write_marker(dir: &Path) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(MARKER_FILE_MODE);
    }
    let mut file = options.open(dir.join(PRIVATE_TEMP_MARKER))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(MARKER_FILE_MODE))?;
    }
    if let Some(contents) = marker_contents() {
        file.write_all(contents.as_bytes())?;
    }
    drop(file);
    Ok(())
}

/// 표식 파일 첫 줄 접두사. 뒤에 만든 host 이름과 LF가 옵니다.
const MARKER_HOST_PREFIX: &str = "host=";

/// 표식 파일 둘째 줄 접두사. 뒤에 만든 기계 식별자와 LF가 옵니다.
const MARKER_MACHINE_PREFIX: &str = "machine=";

/// 표식 파일 최대 크기. 넘으면 다른 host가 만든 것으로 봅니다(지우지 않음).
const MAX_MARKER_BYTES: u64 = 1024;

/// 이 기계의 표식 파일 내용(`host=<이름>\nmachine=<식별자>\n`). host 이름이나 기계 식별자를 얻지
/// 못하면 `None`(빈 표식, 잔여 정리 안 함).
pub(crate) fn marker_contents() -> Option<String> {
    let host = current_host_name()?;
    let machine = current_machine_id()?;
    Some(format!(
        "{MARKER_HOST_PREFIX}{host}\n{MARKER_MACHINE_PREFIX}{machine}\n"
    ))
}

/// 표식이 이 기계에서 만든 것인지. 내용이 이 기계의 [`marker_contents`]와 정확히 같을 때만 `true`입니다.
///
/// 공유·네트워크 volume에서 같은 사용자의 다른 기계 process가 쓰는 디렉터리를 이 기계의 pid 검사만으로
/// 지우지 않게 합니다. host 이름이 같아도 기계 식별자가 다르거나 없는 표식(host 이름만 있는 이전 형식,
/// 빈 표식 포함)은 지우지 않습니다(남겨도 디렉터리가 남을 뿐 데이터를 잃지 않음).
fn marker_is_from_this_host(dir: &Path) -> bool {
    use std::io::Read;
    let Some(expected) = marker_contents() else {
        return false;
    };
    let Ok(file) = std::fs::File::open(dir.join(PRIVATE_TEMP_MARKER)) else {
        return false;
    };
    let mut contents = Vec::new();
    if file
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut contents)
        .is_err()
    {
        return false;
    }
    contents == expected.as_bytes()
}

/// 이 host 이름(unix `gethostname`). 비었거나 UTF-8이 아니거나 제어 문자가 있으면 `None`.
#[cfg(unix)]
fn current_host_name() -> Option<String> {
    let name = sys::host_name()?;
    let valid = !name.is_empty() && !name.chars().any(char::is_control);
    valid.then_some(name)
}

/// 비-unix(Windows, ADR-0006 보류)는 host 이름을 기록하지 않습니다(잔여 정리도 하지 않음).
#[cfg(not(unix))]
fn current_host_name() -> Option<String> {
    None
}

/// 이 기계의 식별자(macOS hardware UUID, `gethostuuid`). 얻지 못하면 `None`.
#[cfg(target_os = "macos")]
fn current_machine_id() -> Option<String> {
    sys::machine_uuid()
}

/// macOS 밖 unix: `/etc/machine-id`(소문자 hex 32자). 없거나 형식이 다르면 `None`.
#[cfg(all(unix, not(target_os = "macos")))]
fn current_machine_id() -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open("/etc/machine-id")
        .ok()?
        .take(64)
        .read_to_string(&mut text)
        .ok()?;
    let id = text.trim_end_matches('\n');
    let valid = id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit());
    valid.then(|| id.to_ascii_lowercase())
}

/// 비-unix(Windows, ADR-0006 보류)는 기계 식별자를 기록하지 않습니다(잔여 정리도 하지 않음).
#[cfg(not(unix))]
fn current_machine_id() -> Option<String> {
    None
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
    /// 표식이 이 기계에서 만든 것이 아니라(다른 기계·같은 host 이름의 다른 기계·host 이름 변경·이전
    /// 형식) 남긴 디렉터리.
    pub other_host: Vec<PathBuf>,
}

/// `parent` 아래에서 `prefix`로 만든 잔여 비공개 임시 디렉터리를 지웁니다.
///
/// 다음을 모두 만족하는 항목만 지웁니다: 이름이 `<prefix>-<pid>-…`, symlink가 아닌 실제 디렉터리,
/// 이 사용자 소유, 권한 `0700`(unix), 표식 파일이 일반 파일로 있음, 표식이 이 기계에서 만든 것(host
/// 이름과 기계 식별자 모두 같음), `pid` process가 없음. pid 생존 검사는 이 기계에서만 의미가 있으므로,
/// 공유·네트워크 volume에서 다른 기계(같은 host 이름 포함)의 살아 있는 process가 쓰는 디렉터리는 기계
/// 표식이 달라 지우지 않습니다. 그 밖의 항목은 보고하지 않고 그대로 둡니다.
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
        if !marker_is_from_this_host(&path) {
            report.other_host.push(path);
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

/// 최소 libc 선언(macOS `<unistd.h>`·`<signal.h>`·`<gethostuuid.h>`). libc crate 없이 안전한 wrapper로만
/// 씁니다.
#[cfg(unix)]
mod sys {
    extern "C" {
        /// `uid_t geteuid(void)`.
        fn geteuid() -> u32;
        /// `int kill(pid_t pid, int sig)`.
        fn kill(pid: i32, sig: i32) -> i32;
        /// `int gethostname(char *name, size_t namelen)`.
        fn gethostname(name: *mut u8, namelen: usize) -> i32;
    }

    /// `struct timespec`(macOS `<sys/_types/_timespec.h>`: `__darwin_time_t`(= `long`), `long`).
    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct Timespec {
        tv_sec: std::ffi::c_long,
        tv_nsec: std::ffi::c_long,
    }

    #[cfg(target_os = "macos")]
    extern "C" {
        /// `int gethostuuid(uuid_t id, const struct timespec *wait)`(`uuid_t` = `unsigned char[16]`).
        fn gethostuuid(id: *mut u8, wait: *const Timespec) -> i32;
    }

    /// 이 기계의 hardware UUID(소문자 hex, 8-4-4-4-12). 실패하거나 모두 0이면 `None`.
    #[cfg(target_os = "macos")]
    pub(super) fn machine_uuid() -> Option<String> {
        let mut id = [0u8; 16];
        // 0은 무한 대기이므로 상한을 둡니다.
        let wait = Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        };
        // SAFETY: id는 uuid_t 크기(16 bytes)의 쓰기 가능한 지역 배열이고, wait는 유효한 timespec을
        // 가리킵니다. gethostuuid는 id 16 bytes에만 쓰고 wait는 읽기만 합니다.
        let rc = unsafe { gethostuuid(id.as_mut_ptr(), &wait) };
        if rc != 0 || id == [0u8; 16] {
            return None;
        }
        let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
        Some(format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        ))
    }

    /// `ESRCH`(macOS·Linux 공통 값): 그런 process가 없음.
    const ESRCH: i32 = 3;

    /// host 이름 buffer 크기(macOS `MAXHOSTNAMELEN` 256, Linux `HOST_NAME_MAX` 64 + NUL 이상).
    const HOST_NAME_BUFFER: usize = 256;

    /// 이 host 이름. 실패하거나 NUL 종료가 없거나 UTF-8이 아니면 `None`.
    pub(super) fn host_name() -> Option<String> {
        let mut buffer = [0u8; HOST_NAME_BUFFER];
        // SAFETY: buffer는 HOST_NAME_BUFFER bytes의 쓰기 가능한 지역 배열이고 같은 길이를 넘깁니다.
        // gethostname은 그 범위 안에만 씁니다.
        let rc = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len()) };
        if rc != 0 {
            return None;
        }
        // 잘린 이름은 NUL 종료가 보장되지 않으므로 NUL이 없으면 쓰지 않습니다.
        let end = buffer.iter().position(|&b| b == 0)?;
        String::from_utf8(buffer[..end].to_vec()).ok()
    }

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
            let contents = marker_contents().expect("테스트 host 이름");
            std::fs::write(path.join(PRIVATE_TEMP_MARKER), contents).unwrap();
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
        assert!(report.other_host.is_empty(), "{:?}", report.other_host);
    }

    #[test]
    fn marker_records_this_host() {
        let work = Work::new();
        let own = PrivateTempDir::create_in(&work.path, PREFIX).unwrap();
        let contents = std::fs::read_to_string(own.path().join(PRIVATE_TEMP_MARKER)).unwrap();
        let lines: Vec<&str> = contents.split_terminator('\n').collect();
        assert_eq!(lines.len(), 2, "{contents:?}");
        let host = lines[0].strip_prefix(MARKER_HOST_PREFIX).unwrap();
        let machine = lines[1].strip_prefix(MARKER_MACHINE_PREFIX).unwrap();
        assert!(!host.is_empty());
        assert_eq!(machine.len(), 36, "{machine:?}");
        assert!(contents.ends_with('\n'));
        assert_eq!(Some(machine.to_string()), current_machine_id());
        assert!(marker_is_from_this_host(own.path()));
    }

    #[test]
    fn sweep_keeps_dirs_of_another_machine_with_the_same_host_name() {
        let work = Work::new();
        let parent = &work.path;
        let host = current_host_name().expect("테스트 host 이름");
        // 공유 volume에서 같은 host 이름(기본 이름 등)·같은 uid인 다른 기계가 만든 staging(리뷰 재현).
        // host 이름만 적힌 표식은 이 기계 것과 구분되지 않으므로 지우면 안 됩니다.
        let same_name = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-0-0"), 0o700, true);
        std::fs::write(
            same_name.join(PRIVATE_TEMP_MARKER),
            format!("{MARKER_HOST_PREFIX}{host}\n"),
        )
        .unwrap();
        // 같은 host 이름이지만 기계 식별자가 다른 표식.
        let other_machine = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-1-0"), 0o700, true);
        std::fs::write(
            other_machine.join(PRIVATE_TEMP_MARKER),
            format!(
                "{MARKER_HOST_PREFIX}{host}\n{MARKER_MACHINE_PREFIX}00000000-0000-0000-0000-000000000001\n"
            ),
        )
        .unwrap();

        let report = sweep_stale_private_temps(parent, PREFIX).unwrap();
        assert!(report.removed.is_empty(), "{:?}", report.removed);
        assert_eq!(
            report.other_host,
            vec![same_name.clone(), other_machine.clone()]
        );
        for kept in [&same_name, &other_machine] {
            assert!(kept.join("checkout/sub/data").exists(), "{kept:?}");
        }
        // 이 기계가 만든 표식은 host 이름과 함께 기계 식별자를 기록합니다.
        let contents = marker_contents().unwrap();
        assert!(
            contents
                .lines()
                .any(|line| line.starts_with(MARKER_MACHINE_PREFIX)),
            "{contents:?}"
        );
    }

    #[test]
    fn sweep_keeps_dirs_marked_by_another_host_or_legacy_markers() {
        let work = Work::new();
        let parent = &work.path;
        // 공유 volume에서 다른 기계(같은 uid)가 만든 staging: 이 기계에서 pid가 죽었어도 지우지 않음.
        let foreign = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-0-0"), 0o700, true);
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            foreign.join(PRIVATE_TEMP_MARKER),
            format!("{MARKER_HOST_PREFIX}another-machine.invalid\n"),
        )
        .unwrap();
        // 이전 형식(빈 표식)과 지나치게 큰 표식도 남김.
        let legacy = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-1-0"), 0o700, true);
        std::fs::write(legacy.join(PRIVATE_TEMP_MARKER), b"").unwrap();
        let huge = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-2-0"), 0o700, true);
        let mut big = marker_contents().unwrap().into_bytes();
        big.resize(usize::try_from(MAX_MARKER_BYTES).unwrap() + 2, b'x');
        std::fs::write(huge.join(PRIVATE_TEMP_MARKER), big).unwrap();
        // 이 host 표식의 잔여는 지움.
        let stale = leftover(parent, &format!("{PREFIX}-{DEAD_PID}-3-0"), 0o700, true);

        let report = sweep_stale_private_temps(parent, PREFIX).unwrap();
        assert_eq!(report.removed, vec![stale.clone()]);
        assert_eq!(
            report.other_host,
            vec![foreign.clone(), legacy.clone(), huge.clone()]
        );
        assert!(report.live.is_empty());
        for kept in [&foreign, &legacy, &huge] {
            assert!(kept.join("checkout/sub/data").exists(), "{kept:?}");
        }
        assert!(!stale.exists());
    }
}
