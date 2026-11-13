//! 경로 기반 검사와 사용 사이 경합(TOCTOU)을 줄이는 파일 열기·identity 비교 (보안 리뷰 S4).
//!
//! std에는 `openat`·`unlinkat`·`renameat` 같은 디렉터리 fd 기준 연산이 없습니다. 그래서 외부
//! crate 없이 다음 두 가지로 검사와 사용 사이의 창을 줄입니다.
//!
//! - **symlink를 따라가지 않는 열기**([`open_read`]): symlink를 따라가면 안 되는 읽기는 macOS에서
//!   `O_NOFOLLOW`(마지막 구성 요소) 또는 `O_NOFOLLOW_ANY`(경로의 모든 구성 요소, macOS 11+)로
//!   엽니다. 열린 뒤에는 fd 기준이므로 경로가 바뀌어도 같은 파일을 읽습니다. FIFO 등에서 막히지
//!   않도록 `O_NONBLOCK`을 함께 줍니다(일반 파일 읽기에는 영향 없음). 다른 Unix에서는 flag 없이
//!   열고 identity 비교만 합니다(flag 값이 플랫폼·아키텍처마다 달라 확인한 macOS 값만 씀).
//! - **identity 비교**([`FileIdentity`]): 검사 때 `lstat`한 `(dev, ino)`와 연 fd의 `fstat`, 또는
//!   사용 직전 다시 `lstat`한 값을 비교해, 검사 뒤 항목이 바뀌었으면 사용하지 않습니다.
//!
//! # 남는 한계(정확히)
//!
//! - 디렉터리를 다시 `lstat`한 직후와 실제 경로 기반 syscall(`rename`·`remove_file`·temp 생성)
//!   사이의 짧은 창은 남습니다(디렉터리 fd 기준 연산 없음). 그 창에 디렉터리를 교체하는 동시
//!   공격자는 쓰기·삭제를 다른 위치로 돌릴 수 있습니다. commit된(정적) symlink는 모두 차단됩니다.
//! - macOS 밖 Unix에는 `O_NOFOLLOW_ANY`·`O_NOFOLLOW`를 쓰지 않으므로 symlink 교체는 identity
//!   비교로만 검출합니다(같은 inode로 바꾸는 경우는 검출하지 않지만 내용도 같음).
//! - 비Unix(Windows 지원 보류)에는 identity가 없어 비교가 항상 같다고 봅니다.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

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

/// symlink를 따라가지 않을 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoFollow {
    /// 마지막 구성 요소만(`O_NOFOLLOW`).
    Final,
    /// 경로의 모든 구성 요소(macOS `O_NOFOLLOW_ANY`) — canonical 경로를 열 때 씁니다.
    AnyComponent,
}

/// macOS `<sys/fcntl.h>`·`<sys/errno.h>` 값(SDK header로 확인).
#[cfg(target_os = "macos")]
mod flags {
    pub const O_NONBLOCK: i32 = 0x0000_0004;
    pub const O_NOFOLLOW: i32 = 0x0000_0100;
    pub const O_NOFOLLOW_ANY: i32 = 0x2000_0000;
    pub const ELOOP: i32 = 62;
}

/// 읽기 전용으로 엽니다. macOS에서는 `follow` 범위의 symlink를 따라가지 않습니다(`ELOOP` →
/// [`is_symlink_refusal`]).
pub(crate) fn open_read(path: &Path, follow: NoFollow) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // `O_NOFOLLOW_ANY`는 마지막 구성 요소도 포함하며, `O_NOFOLLOW`와 함께 주면 macOS가
        // `EINVAL`로 거부합니다(테스트로 확인).
        let nofollow = match follow {
            NoFollow::Final => flags::O_NOFOLLOW,
            NoFollow::AnyComponent => flags::O_NOFOLLOW_ANY,
        };
        options.custom_flags(nofollow | flags::O_NONBLOCK);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = follow;
    options.open(path)
}

/// [`open_read`]가 symlink 때문에 거부한 오류인지(`ELOOP`).
pub(crate) fn is_symlink_refusal(e: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        e.raw_os_error() == Some(flags::ELOOP)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = e;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::TempDir;
    use std::io::Read;

    #[test]
    fn identity_tracks_the_entry_not_the_path() {
        let dir = TempDir::new("fs-guard-identity");
        let a = dir.path.join("a");
        let b = dir.path.join("b");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let (_, before) = FileIdentity::lstat(&a).unwrap();
        let opened = open_read(&a, NoFollow::Final).unwrap();
        assert_eq!(FileIdentity::of(&opened.metadata().unwrap()), before);
        // 같은 경로에 다른 파일을 rename으로 넣으면 identity가 달라집니다.
        std::fs::rename(&b, &a).unwrap();
        let (_, after) = FileIdentity::lstat(&a).unwrap();
        #[cfg(unix)]
        assert_ne!(before, after);
        let _ = after;
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn symlinks_are_refused_at_final_or_any_component() {
        let dir = TempDir::new("fs-guard-nofollow");
        let real_dir = dir.path.join("real");
        std::fs::create_dir(&real_dir).unwrap();
        let file = real_dir.join("f");
        std::fs::write(&file, b"data").unwrap();
        std::os::unix::fs::symlink(&file, dir.path.join("link-file")).unwrap();
        std::os::unix::fs::symlink(&real_dir, dir.path.join("link-dir")).unwrap();

        // 실제 경로는 두 범위 모두 열립니다.
        for follow in [NoFollow::Final, NoFollow::AnyComponent] {
            let mut text = String::new();
            open_read(&file, follow)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            assert_eq!(text, "data");
        }
        // 마지막 구성 요소 symlink는 두 범위 모두 거부합니다.
        for follow in [NoFollow::Final, NoFollow::AnyComponent] {
            let err = open_read(&dir.path.join("link-file"), follow).unwrap_err();
            assert!(is_symlink_refusal(&err), "{follow:?}: {err}");
        }
        // 중간 구성 요소 symlink는 AnyComponent만 거부합니다.
        let via_dir = dir.path.join("link-dir").join("f");
        assert!(open_read(&via_dir, NoFollow::Final).is_ok());
        let err = open_read(&via_dir, NoFollow::AnyComponent).unwrap_err();
        assert!(is_symlink_refusal(&err), "{err}");
        assert!(!is_symlink_refusal(&io::Error::from(
            io::ErrorKind::NotFound
        )));
    }

    /// FIFO는 쓰는 쪽이 없어도 열 때 막히지 않습니다(`O_NONBLOCK`).
    #[cfg(target_os = "macos")]
    #[test]
    fn fifo_open_does_not_block() {
        let dir = TempDir::new("fs-guard-fifo");
        let fifo = dir.path.join("fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let file = open_read(&fifo, NoFollow::Final).unwrap();
        assert!(!file.metadata().unwrap().is_file());
    }
}
