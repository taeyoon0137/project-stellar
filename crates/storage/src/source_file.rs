//! ADR-0059의 단일 의미 단위 파일 I/O. 여러 파일 transaction은 이 API의 범위 밖입니다.

use std::io;
use std::path::Path;

use crate::atomic_file::{
    write_file_atomic_observed, write_file_atomic_observed_with_permissions, NoopObserver,
    WriteObserver, WriteStage,
};
use crate::fs_guard::{ensure_real_dirs, read_regular_file, DirGuard, FileIdentity, Link, Refusal};
use crate::hash::ContentHash;

/// 사용자 지정 symlink를 따르며 열린 handle의 종류·크기를 검사합니다(아키텍처 19.3).
pub fn read(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    read_regular_file(path, limit, Link::Follow)
}

struct Expected<'a> {
    path: &'a Path,
    hash: Option<ContentHash>,
    limit: u64,
    observer: &'a mut dyn WriteObserver,
}

impl WriteObserver for Expected<'_> {
    fn before(&mut self, stage: WriteStage) -> io::Result<()> {
        self.observer.before(stage)?;
        if !matches!(stage, WriteStage::CreateTemp | WriteStage::Rename) {
            return Ok(());
        }
        check_expected(self.path, self.hash, self.limit)
    }
}

fn check_expected(path: &Path, expected: Option<ContentHash>, limit: u64) -> io::Result<()> {
    if expected.is_none() {
        return match std::fs::symlink_metadata(path) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "원본 파일이 이미 존재합니다",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let found = match read_regular_file(path, limit, Link::NoFollow) {
        Ok(bytes) => Some(ContentHash::compute(&bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if found != expected {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "원본이 외부에서 변경되었습니다",
        ));
    }
    Ok(())
}

/// 기대 bytes hash가 같을 때만 temp/fsync/rename으로 저장합니다.
/// `None`은 새 파일이며 hard link로 게시해 마지막 순간에 생긴 기존 항목도 교체하지 않습니다.
///
/// 사용자 지정 부모 디렉터리는 canonicalize하고 쓰기 전·rename 직전에 identity를 확인합니다.
/// 기존 파일 교체에는 마지막 검사와 rename 사이의 비협조적 외부 writer race가 남습니다.
/// 여러 writer의 동시 저장을 직렬화하는 프로젝트 잠금은 호출자 책임입니다.
/// 부모 fsync 또는 link 후 temp 제거 오류는 대상 게시 후 반환될 수 있습니다.
/// 오류 후에는 실제 원본을 다시 읽어야 합니다. hard link 미지원 파일 시스템은 오류로 거부합니다.
/// 기존 파일의 권한은 temp handle에 복사하고 새 파일은 기본 생성 권한을 사용합니다.
pub fn write_checked(
    path: &Path,
    bytes: &[u8],
    expected: Option<ContentHash>,
    limit: u64,
) -> io::Result<()> {
    write_checked_observed(path, bytes, expected, limit, &mut NoopObserver)
}

fn write_checked_observed(
    path: &Path,
    bytes: &[u8],
    expected: Option<ContentHash>,
    limit: u64,
    observer: &mut dyn WriteObserver,
) -> io::Result<()> {
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "텍스트 원본 크기 상한 초과",
        ));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize()?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "파일 이름 없음"))?;
    let target = parent.join(name);
    let (metadata, identity) = FileIdentity::lstat(&parent)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "원본 부모가 디렉터리가 아닙니다",
        ));
    }
    let permissions = if expected.is_some() {
        check_expected(&target, expected, limit)?;
        let (metadata, _) = FileIdentity::lstat(&target)?;
        if !metadata.is_file() {
            return Err(Refusal::NotRegularFile.into_io());
        }
        Some(metadata.permissions())
    } else {
        None
    };
    let mut observer = Expected {
        path: &target,
        hash: expected,
        limit,
        observer,
    };
    let mut guard = DirGuard::new(&mut observer, vec![(parent, identity)]);
    write_file_atomic_observed_with_permissions(
        &target,
        bytes,
        &mut guard,
        permissions,
        expected.is_some(),
    )
}

/// 원본 hash에 속한 파생 cache chunk 하나를 검증하고 없거나 손상됐으면 다시 씁니다.
/// 원본 파일과 달리 cache만 교체할 수 있습니다. 반환값은 기존 bytes 재사용 여부입니다.
/// cache 전용 `.gitignore`를 먼저 보장하며 다른 설정은 덮어쓰지 않습니다.
pub fn refresh_cache_chunk(
    project_root: &Path,
    source_hash: ContentHash,
    chunk: crate::chunk::id::ChunkId,
    bytes: &[u8],
) -> io::Result<bool> {
    let key = source_hash.digest_hex();
    let dirs = ensure_real_dirs(project_root, &[".stellar", "cache", "text-source-v1", &key])
        .map_err(|(_, error)| error)?;
    let cache_root = project_root.join(".stellar/cache/text-source-v1");
    let ignore_path = cache_root.join(".gitignore");
    match read_regular_file(&ignore_path, 2, Link::NoFollow) {
        Ok(bytes) if bytes == b"*\n" => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache Git 제외 설정이 다릅니다",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut observer = NoopObserver;
            let mut expected = Expected {
                path: &ignore_path,
                hash: None,
                limit: 2,
                observer: &mut observer,
            };
            let mut guard = DirGuard::new(&mut expected, dirs.clone());
            write_file_atomic_observed_with_permissions(
                &ignore_path,
                b"*\n",
                &mut guard,
                None,
                false,
            )?;
        }
        Err(error) => return Err(error),
    }
    let path = cache_root.join(key).join(format!("{chunk}.starchunk"));
    let previous = match read_regular_file(&path, bytes.len() as u64, Link::NoFollow) {
        Ok(previous) => Some(previous),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::FileTooLarge
            ) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    if previous.as_deref() == Some(bytes) {
        return Ok(true);
    }
    let mut observer = NoopObserver;
    let mut guard = DirGuard::new(&mut observer, dirs);
    write_file_atomic_observed(&path, bytes, &mut guard)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_guard::test_support::TempDir;

    #[test]
    fn cache_creates_its_ignore_rule_and_preserves_different_rules() {
        let dir = TempDir::new("text-source-cache-ignore");
        let hash = ContentHash::compute(b"source");
        let chunk = crate::chunk::id::ChunkId::from_bytes([1; 16]);
        assert!(!refresh_cache_chunk(&dir.path, hash, chunk, b"cached").unwrap());
        let cache_root = dir.path.join(".stellar/cache/text-source-v1");
        let ignore = cache_root.join(".gitignore");
        let cached = cache_root
            .join(hash.digest_hex())
            .join(format!("{chunk}.starchunk"));
        assert_eq!(std::fs::read(&ignore).unwrap(), b"*\n");
        for other in [b"!\n".as_slice(), b"custom rule\n".as_slice()] {
            std::fs::write(&ignore, other).unwrap();
            assert!(refresh_cache_chunk(&dir.path, hash, chunk, b"replacement").is_err());
            assert_eq!(std::fs::read(&ignore).unwrap(), other);
            assert_eq!(std::fs::read(&cached).unwrap(), b"cached");
        }
    }

    #[test]
    fn creates_new_source_and_refuses_existing_or_changed_content() {
        let dir = TempDir::new("text-source-expected");
        let path = dir.path.join("view.yaml");
        write_checked(&path, b"original", None, 64).unwrap();
        assert_eq!(read(&path, 64).unwrap(), b"original");
        let error = write_checked(&path, b"replacement", None, 64).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        let error = write_checked(
            &path,
            b"replacement",
            Some(ContentHash::compute(b"stale")),
            64,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(read(&path, 64).unwrap(), b"original");
        write_checked(
            &path,
            b"replacement",
            Some(ContentHash::compute(b"original")),
            64,
        )
        .unwrap();
        assert_eq!(read(&path, 64).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 1);
    }

    struct FailAt(WriteStage);

    impl WriteObserver for FailAt {
        fn before(&mut self, stage: WriteStage) -> io::Result<()> {
            if stage == self.0 {
                Err(io::Error::other("injected write failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn write_failures_keep_complete_content_and_clean_temporary_files() {
        for stage in WriteStage::ALL {
            let dir = TempDir::new("text-source-fault");
            let path = dir.path.join("view.yaml");
            std::fs::write(&path, b"original").unwrap();
            let error = write_checked_observed(
                &path,
                b"replacement",
                Some(ContentHash::compute(b"original")),
                64,
                &mut FailAt(stage),
            )
            .unwrap_err();
            assert_eq!(error.to_string(), "injected write failure");
            let expected: &[u8] = if stage == WriteStage::SyncParentDir {
                b"replacement"
            } else {
                b"original"
            };
            assert_eq!(read(&path, 64).unwrap(), expected, "{stage}");
            assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 1, "{stage}");
        }
    }

    struct ExternalEdit<'a>(&'a Path);

    impl WriteObserver for ExternalEdit<'_> {
        fn before(&mut self, stage: WriteStage) -> io::Result<()> {
            if stage == WriteStage::Rename {
                std::fs::write(self.0, b"external edit")?;
            }
            Ok(())
        }
    }

    #[test]
    fn rechecks_external_edits_and_new_files_before_rename() {
        for existing in [false, true] {
            let dir = TempDir::new("text-source-external-edit");
            let path = dir.path.join("view.yaml");
            let expected = if existing {
                std::fs::write(&path, b"original").unwrap();
                Some(ContentHash::compute(b"original"))
            } else {
                None
            };
            let error = write_checked_observed(
                &path,
                b"replacement",
                expected,
                64,
                &mut ExternalEdit(&path),
            )
            .unwrap_err();
            let kind = if existing {
                io::ErrorKind::WouldBlock
            } else {
                io::ErrorKind::AlreadyExists
            };
            assert_eq!(error.kind(), kind);
            assert_eq!(read(&path, 64).unwrap(), b"external edit");
            assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_existing_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("text-source-permissions");
        let path = dir.path.join("view.yaml");
        for mode in [0o600, 0o751, 0o444] {
            write_checked(&path, b"original", None, 64).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            write_checked(
                &path,
                b"replacement",
                Some(ContentHash::compute(b"original")),
                64,
            )
            .unwrap();
            assert_eq!(read(&path, 64).unwrap(), b"replacement");
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
            std::fs::remove_file(&path).unwrap();
        }
    }
}
