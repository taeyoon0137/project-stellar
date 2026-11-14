//! 저장소 파일 접근 guard (보안 감사 STOR-1·2·4).
//!
//! 프로젝트 폴더는 Git으로 받은 비신뢰 입력일 수 있습니다. commit된 symlink·FIFO·장치 파일·거대한
//! sparse 파일이 저장소 경로(`design/...`)에 있으면, 경로 기반 `std::fs::read`는 끝없이 읽거나
//! (`/dev/zero`) 멈추고(FIFO), `create_dir_all`·rename은 symlink 디렉터리를 따라 프로젝트 밖에
//! 쓰며, symlink를 따라가는 GC 탐색은 프로젝트 밖 파일을 지웁니다. 이 모듈은 외부 crate 없이 그
//! 경로를 막는 공통 도구를 둡니다(crate 내부 전용, 공개 API 아님).
//!
//! - **읽기**([`read_regular_file`]): 연 handle을 `fstat`해 **정규 파일**이고 크기가 상한 이하일 때만
//!   읽고, 읽기도 `상한 + 1` bytes에서 멈춥니다(검사 뒤 파일이 자라도 상한 유지). macOS에서는
//!   `O_NONBLOCK`으로 열어 FIFO·장치를 열 때 기다리지 않습니다(정규 파일 읽기에는 영향 없음).
//!   [`Link::Follow`]는 symlink를 따르고(기술 아키텍처 19.3 "심볼릭 링크 전면 허용" — 최종 대상의
//!   종류·크기만 검사), [`Link::NoFollow`]는 마지막 구성 요소 symlink를 거부합니다(macOS
//!   `O_NOFOLLOW`). [`Link::Resolved`]는 이미 해석한 절대 경로를 열 때 macOS의 `O_NOFOLLOW_ANY`를 씁니다.
//! - **쓰기 디렉터리**([`ensure_real_dirs`]): root 아래 구성 요소를 한 단계씩 `lstat`해 symlink면
//!   따라가지 않고 거부하고, 없으면 한 단계씩 `create_dir`로 만듭니다(`create_dir_all`은 symlink
//!   디렉터리를 그대로 통과하므로 쓰지 않음). [`DirGuard`]는 temp 생성과 rename 직전에 같은 디렉터리
//!   identity(`(dev, ino)`)인지 다시 확인합니다.
//! - **identity**([`FileIdentity`]·[`same_dir`]): 검사 때 `lstat`한 항목과 사용(쓰기·삭제) 직전
//!   항목이 같은지 비교합니다.
//!
//! 거부는 [`Refusal`]을 담은 `std::io::Error`로 돌려줍니다. 호출하는 공개 함수의 오류 enum(ADR 보호
//! 공개 API)에 새 variant를 더하지 않고 기존 `Io` variant로 보고하기 위해서입니다.
//!
//! # 남는 한계(정확히)
//!
//! - std에는 `openat`·`renameat` 같은 디렉터리 fd 기준 연산이 없습니다. 디렉터리를 다시 확인한
//!   직후와 실제 경로 기반 syscall(temp 생성·rename·삭제) 사이의 짧은 창은 남습니다. 그 창에
//!   디렉터리를 교체하는 **동시** 공격자는 쓰기·삭제를 다른 위치로 돌릴 수 있습니다. commit된(정적)
//!   symlink는 모두 차단됩니다.
//! - macOS 밖 Unix에는 flag 값이 플랫폼·아키텍처마다 달라 `O_NONBLOCK`·`O_NOFOLLOW`를 주지 않습니다.
//!   열기 전에 `stat`·`lstat`으로 종류를 확인하고 연 handle과 identity를 비교하므로, 그 사이 FIFO로
//!   바꾸는 동시 공격자만 열기를 멈출 수 있습니다(Linux 등은 아직 지원 대상이 아님).
//! - 비Unix(Windows 지원 보류, ADR-0006)에는 identity가 없어 비교가 항상 같다고 봅니다.

use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use crate::atomic_file::{WriteObserver, WriteStage};

/// 저장소 guard가 파일 접근을 거부한 이유. `std::io::Error`의 내부 오류로 담깁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 디렉터리 구성 요소가 symlink라 따라가지 않습니다(쓰기·삭제 경로).
    SymlinkDirectory,
    /// 대상 항목이 symlink라 따라가지 않습니다([`Link::NoFollow`]).
    SymlinkFile,
    /// 디렉터리여야 하는 구성 요소가 디렉터리가 아닙니다.
    NotDirectory,
    /// 정규 파일이 아닙니다(디렉터리·FIFO·장치·socket 등).
    NotRegularFile,
    /// 파일 크기가 상한을 넘습니다.
    TooLarge {
        /// 상한(bytes).
        limit: u64,
    },
    /// 검사한 항목이 사용 직전에 다른 항목으로 바뀌었습니다.
    Changed,
}

impl Refusal {
    /// 이 거부를 담은 I/O 오류.
    pub(crate) fn into_io(self) -> io::Error {
        let kind = match self {
            Refusal::SymlinkDirectory | Refusal::SymlinkFile | Refusal::NotRegularFile => {
                ErrorKind::InvalidInput
            }
            Refusal::NotDirectory => ErrorKind::NotADirectory,
            Refusal::TooLarge { .. } => ErrorKind::FileTooLarge,
            Refusal::Changed => ErrorKind::Other,
        };
        io::Error::new(kind, self)
    }

    /// I/O 오류가 guard 거부라면 그 이유.
    pub(crate) fn of(error: &io::Error) -> Option<Refusal> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Refusal>())
            .copied()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::SymlinkDirectory => f.write_str("symlink 디렉터리를 따라가지 않습니다"),
            Refusal::SymlinkFile => f.write_str("symlink를 따라가지 않습니다"),
            Refusal::NotDirectory => f.write_str("디렉터리가 아닙니다"),
            Refusal::NotRegularFile => f.write_str("정규 파일이 아닙니다"),
            Refusal::TooLarge { limit } => {
                write!(f, "파일 크기가 상한 {limit} bytes를 넘습니다")
            }
            Refusal::Changed => f.write_str("검사 뒤 항목이 바뀌었습니다"),
        }
    }
}

impl std::error::Error for Refusal {}

/// 파일 시스템 항목 identity(`(st_dev, st_ino)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl FileIdentity {
    /// metadata(`lstat`·`fstat` 결과)의 identity.
    pub(crate) fn of(metadata: &Metadata) -> FileIdentity {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            FileIdentity {
                dev: metadata.dev(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            FileIdentity { dev: 0, ino: 0 }
        }
    }

    /// 경로를 `lstat`해(따라가지 않음) metadata와 identity를 돌려줍니다.
    pub(crate) fn lstat(path: &Path) -> io::Result<(Metadata, FileIdentity)> {
        let metadata = std::fs::symlink_metadata(path)?;
        let identity = FileIdentity::of(&metadata);
        Ok((metadata, identity))
    }
}

/// 경로가 지금도 `identity`의 실제 디렉터리인지(`lstat`, 따라가지 않음).
pub(crate) fn same_dir(path: &Path, identity: FileIdentity) -> bool {
    matches!(FileIdentity::lstat(path), Ok((metadata, id)) if metadata.is_dir() && id == identity)
}

/// 읽을 때 마지막 구성 요소 symlink를 따를지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    /// 따릅니다(기술 아키텍처 19.3). 최종 대상의 종류·크기는 검사합니다.
    Follow,
    /// 따르지 않고 [`Refusal::SymlinkFile`]로 거부합니다.
    NoFollow,
    /// 이미 해석한 절대 경로입니다. macOS에서는 여는 순간 모든 경로 구성 요소의 링크를 거부합니다.
    Resolved,
}

/// macOS `<sys/fcntl.h>`·`<sys/errno.h>` 값(SDK header로 확인, assets `fs_guard`와 같은 값).
#[cfg(target_os = "macos")]
mod flags {
    pub const O_NONBLOCK: i32 = 0x0000_0004;
    pub const O_NOFOLLOW: i32 = 0x0000_0100;
    pub const O_NOFOLLOW_ANY: i32 = 0x2000_0000;
    pub const ELOOP: i32 = 62;
}

/// 읽기 전용으로 엽니다. macOS는 `O_NONBLOCK`과(`NoFollow`면) `O_NOFOLLOW`를 줍니다.
fn open_read(path: &Path, link: Link) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = match link {
            Link::Follow => 0,
            Link::NoFollow => flags::O_NOFOLLOW,
            Link::Resolved => flags::O_NOFOLLOW_ANY,
        };
        options.custom_flags(flags::O_NONBLOCK | nofollow);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = link;
    options.open(path)
}

/// `open_read`가 `O_NOFOLLOW` 때문에 거부한 오류인지(macOS `ELOOP`).
fn is_nofollow_refusal(error: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        error.raw_os_error() == Some(flags::ELOOP)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = error;
        false
    }
}

/// 정규 파일 하나를 `limit` bytes까지 읽습니다.
///
/// 연 handle을 `fstat`해 정규 파일이 아니면 [`Refusal::NotRegularFile`], 크기가 `limit`를 넘으면
/// 읽지 않고 [`Refusal::TooLarge`]입니다. 검사 뒤 파일이 자라도 `limit + 1` bytes에서 읽기를 멈춰
/// 같은 오류로 거부합니다. 없으면 `NotFound` I/O 오류 그대로입니다(호출자가 누락 variant로 보고).
pub(crate) fn read_regular_file(path: &Path, limit: u64, link: Link) -> io::Result<Vec<u8>> {
    // flag 없이 여는 플랫폼은 열기 전에 종류를 확인해 FIFO·장치 열기에서 멈추지 않게 합니다.
    #[cfg(not(target_os = "macos"))]
    let checked = {
        let metadata = match link {
            Link::Follow => std::fs::metadata(path)?,
            Link::NoFollow => std::fs::symlink_metadata(path)?,
            Link::Resolved => {
                if std::fs::canonicalize(path)? != path {
                    return Err(Refusal::Changed.into_io());
                }
                std::fs::symlink_metadata(path)?
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(Refusal::SymlinkFile.into_io());
        }
        if !metadata.is_file() {
            return Err(Refusal::NotRegularFile.into_io());
        }
        FileIdentity::of(&metadata)
    };
    let file = match open_read(path, link) {
        Ok(file) => file,
        Err(e) if link != Link::Follow && is_nofollow_refusal(&e) => {
            return Err(Refusal::SymlinkFile.into_io())
        }
        Err(e) => return Err(e),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Refusal::NotRegularFile.into_io());
    }
    #[cfg(not(target_os = "macos"))]
    if FileIdentity::of(&metadata) != checked {
        return Err(Refusal::Changed.into_io());
    }
    if metadata.len() > limit {
        return Err(Refusal::TooLarge { limit }.into_io());
    }
    // 위 검사로 metadata.len() ≤ limit입니다(capacity는 실제 크기까지만).
    let capacity = usize::try_from(metadata.len()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Refusal::TooLarge { limit }.into_io());
    }
    Ok(bytes)
}

/// `root` 아래 `components`를 한 단계씩 실제 디렉터리로 보장하고 각 경로와 identity를 돌려줍니다.
///
/// 없으면 `create_dir`로 만들고(동시에 만들어진 경우 포함) 다시 `lstat`합니다. symlink면 따라가지
/// 않고 [`Refusal::SymlinkDirectory`], 디렉터리가 아니면 [`Refusal::NotDirectory`]입니다. 실패하면
/// 실패한 경로와 오류를 돌려줍니다. `root` 자체는 사용자가 고른 경로라 검사하지 않고(19.3), 없으면
/// 만듭니다.
pub(crate) fn ensure_real_dirs(
    root: &Path,
    components: &[&str],
) -> Result<Vec<(PathBuf, FileIdentity)>, (PathBuf, io::Error)> {
    std::fs::create_dir_all(root).map_err(|e| (root.to_path_buf(), e))?;
    let mut path = root.to_path_buf();
    let mut dirs = Vec::with_capacity(components.len());
    for component in components {
        path.push(component);
        let identity = ensure_real_dir(&path).map_err(|e| (path.clone(), e))?;
        dirs.push((path.clone(), identity));
    }
    Ok(dirs)
}

/// 실제 디렉터리 하나를 보장합니다([`ensure_real_dirs`]).
fn ensure_real_dir(path: &Path) -> io::Result<FileIdentity> {
    let checked = match FileIdentity::lstat(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            match std::fs::create_dir(path) {
                Ok(()) => {}
                // 동시에 다른 저장이 만든 경우: 무엇이 생겼는지 다시 확인합니다.
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            FileIdentity::lstat(path)
        }
        other => other,
    };
    let (metadata, identity) = checked?;
    if metadata.file_type().is_symlink() {
        return Err(Refusal::SymlinkDirectory.into_io());
    }
    if !metadata.is_dir() {
        return Err(Refusal::NotDirectory.into_io());
    }
    Ok(identity)
}

/// 쓰기 관찰자 wrapper: temp 생성과 rename 직전에 확인한 디렉터리가 같은 실제 디렉터리인지 다시
/// 봅니다.
///
/// 디렉터리 확인 → 쓰기 사이의 창을 "다시 확인 → syscall" 직전으로 줄입니다(완전히 없애지는 못함,
/// 모듈 문서의 남는 한계). 바뀌었으면 그 단계를 실행하지 않고 [`Refusal::SymlinkDirectory`] 또는
/// [`Refusal::Changed`]를 담은 I/O 오류입니다.
pub(crate) struct DirGuard<'a> {
    inner: &'a mut dyn WriteObserver,
    dirs: Vec<(PathBuf, FileIdentity)>,
}

impl<'a> DirGuard<'a> {
    /// `dirs`를 확인하는 wrapper.
    pub(crate) fn new(
        inner: &'a mut dyn WriteObserver,
        dirs: Vec<(PathBuf, FileIdentity)>,
    ) -> DirGuard<'a> {
        DirGuard { inner, dirs }
    }
}

impl WriteObserver for DirGuard<'_> {
    fn before(&mut self, stage: WriteStage) -> io::Result<()> {
        self.inner.before(stage)?;
        if !matches!(stage, WriteStage::CreateTemp | WriteStage::Rename) {
            return Ok(());
        }
        for (path, identity) in &self.dirs {
            match FileIdentity::lstat(path) {
                Ok((metadata, id)) if metadata.is_dir() && id == *identity => {}
                Ok((metadata, _)) if metadata.file_type().is_symlink() => {
                    return Err(Refusal::SymlinkDirectory.into_io())
                }
                _ => return Err(Refusal::Changed.into_io()),
            }
        }
        Ok(())
    }
}

/// 테스트 도우미(이 crate 단위 테스트 전용).
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// 테스트 임시 디렉터리(drop 시 삭제).
    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }

    impl TempDir {
        pub(crate) fn new(name: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-storage-guard-{name}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// `f`를 별도 thread에서 실행하고 제한 시간 안에 끝난 결과를 돌려줍니다.
    ///
    /// 회귀하면 무한 읽기(`/dev/zero` → 메모리를 끝없이 할당)나 무한 대기(FIFO)·지수 순회가 되는
    /// 검사용입니다. 제한 시간을 넘기면 남은 thread가 메모리·CPU를 계속 쓰지 않도록 test process를
    /// 즉시 끝냅니다(abort).
    pub(crate) fn bounded<T: Send + 'static>(
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(value) => value,
            Err(_) => {
                eprintln!("{what}: 10초 안에 끝나지 않았습니다(무한 읽기·대기·순회 회귀)");
                std::process::abort();
            }
        }
    }

    /// FIFO를 만듭니다(`mkfifo`).
    #[cfg(unix)]
    pub(crate) fn mkfifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo 실패");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TempDir;
    use super::*;

    fn refusal(result: io::Result<impl fmt::Debug>) -> Refusal {
        let error = result.unwrap_err();
        Refusal::of(&error).unwrap_or_else(|| panic!("guard 거부가 아님: {error:?}"))
    }

    /// [`ensure_real_dirs`] 실패의 경로와 거부 이유.
    fn dir_refusal(
        result: Result<Vec<(PathBuf, FileIdentity)>, (PathBuf, io::Error)>,
    ) -> (PathBuf, Refusal) {
        let (path, error) = result.unwrap_err();
        let refusal = Refusal::of(&error).unwrap_or_else(|| panic!("guard 거부가 아님: {error:?}"));
        (path, refusal)
    }

    #[test]
    fn regular_file_within_limit_is_read() {
        let dir = TempDir::new("read-ok");
        let file = dir.path.join("f");
        std::fs::write(&file, b"0123456789").unwrap();
        for link in [Link::Follow, Link::NoFollow] {
            assert_eq!(read_regular_file(&file, 10, link).unwrap(), b"0123456789");
        }
        assert_eq!(
            refusal(read_regular_file(&file, 9, Link::Follow)),
            Refusal::TooLarge { limit: 9 }
        );
        let missing = read_regular_file(&dir.path.join("none"), 9, Link::Follow).unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);
        assert_eq!(Refusal::of(&missing), None);
    }

    #[test]
    fn directory_is_not_a_regular_file() {
        let dir = TempDir::new("read-dir");
        assert_eq!(
            refusal(read_regular_file(&dir.path, 1 << 20, Link::Follow)),
            Refusal::NotRegularFile
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_followed_or_refused_by_link_rule() {
        let dir = TempDir::new("read-link");
        let file = dir.path.join("f");
        std::fs::write(&file, b"data").unwrap();
        let link = dir.path.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_regular_file(&link, 4, Link::Follow).unwrap(), b"data");
        assert_eq!(
            refusal(read_regular_file(&link, 4, Link::NoFollow)),
            Refusal::SymlinkFile
        );
    }

    #[cfg(unix)]
    #[test]
    fn devices_and_fifos_are_refused_without_blocking() {
        let dir = TempDir::new("read-special");
        let fifo = dir.path.join("fifo");
        super::test_support::mkfifo(&fifo);
        let zero = dir.path.join("zero");
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        let fifo_link = dir.path.join("fifo-link");
        std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
        for path in [zero, fifo, fifo_link] {
            let result = super::test_support::bounded("특수 파일 읽기", move || {
                read_regular_file(&path, 1 << 20, Link::Follow).map(|_| ())
            });
            assert_eq!(refusal(result), Refusal::NotRegularFile);
        }
    }

    #[test]
    fn sparse_file_over_limit_is_refused_before_reading() {
        let dir = TempDir::new("read-sparse");
        let file = dir.path.join("sparse");
        File::create(&file).unwrap().set_len(1 << 30).unwrap();
        assert_eq!(
            refusal(read_regular_file(&file, 1 << 20, Link::Follow)),
            Refusal::TooLarge { limit: 1 << 20 }
        );
    }

    #[test]
    fn ensure_real_dirs_creates_each_level() {
        let dir = TempDir::new("dirs-create");
        let dirs = ensure_real_dirs(&dir.path, &["a", "b", "c"]).unwrap();
        assert_eq!(dirs.len(), 3);
        assert!(dir.path.join("a/b/c").is_dir());
        for (path, identity) in &dirs {
            assert!(same_dir(path, *identity));
        }
        assert!(!same_dir(&dir.path.join("a/b"), dirs[2].1), "다른 디렉터리");
        assert!(!same_dir(&dir.path.join("none"), dirs[0].1), "없는 경로");
        // 다시 불러도 같은 identity.
        let again = ensure_real_dirs(&dir.path, &["a", "b", "c"]).unwrap();
        assert_eq!(dirs, again);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_or_non_directory_components_are_refused() {
        let dir = TempDir::new("dirs-refuse");
        let outside = TempDir::new("dirs-refuse-outside");
        std::fs::create_dir(dir.path.join("a")).unwrap();
        std::os::unix::fs::symlink(&outside.path, dir.path.join("a/link")).unwrap();
        std::fs::write(dir.path.join("a/file"), b"x").unwrap();

        assert_eq!(
            dir_refusal(ensure_real_dirs(&dir.path, &["a", "link", "b"])),
            (dir.path.join("a/link"), Refusal::SymlinkDirectory)
        );
        assert_eq!(
            dir_refusal(ensure_real_dirs(&dir.path, &["a", "file", "b"])),
            (dir.path.join("a/file"), Refusal::NotDirectory)
        );
        assert_eq!(
            std::fs::read_dir(&outside.path).unwrap().count(),
            0,
            "symlink 대상 밖에 아무것도 만들지 않음"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dir_guard_detects_replacement_before_create_and_rename() {
        use crate::atomic_file::NoopObserver;

        let dir = TempDir::new("dir-guard");
        let outside = TempDir::new("dir-guard-outside");
        let dirs = ensure_real_dirs(&dir.path, &["d"]).unwrap();
        let mut noop = NoopObserver;
        let mut guard = DirGuard::new(&mut noop, dirs.clone());
        for stage in WriteStage::ALL {
            guard.before(stage).unwrap();
        }
        // 확인 뒤 디렉터리를 symlink로 교체(원래 디렉터리는 옮겨 두어 inode 재사용을 피함).
        let d = dir.path.join("d");
        std::fs::rename(&d, dir.path.join("d-original")).unwrap();
        std::os::unix::fs::symlink(&outside.path, &d).unwrap();
        let mut guard = DirGuard::new(&mut noop, dirs.clone());
        assert_eq!(
            refusal(guard.before(WriteStage::CreateTemp)),
            Refusal::SymlinkDirectory
        );
        assert_eq!(
            refusal(guard.before(WriteStage::Rename)),
            Refusal::SymlinkDirectory
        );
        // temp 생성·rename 밖 단계는 확인하지 않습니다.
        guard.before(WriteStage::WriteBytes).unwrap();
        // 다른 실제 디렉터리로 교체.
        std::fs::remove_file(&d).unwrap();
        std::fs::create_dir(&d).unwrap();
        let mut guard = DirGuard::new(&mut noop, dirs);
        assert_eq!(refusal(guard.before(WriteStage::Rename)), Refusal::Changed);
    }
}
