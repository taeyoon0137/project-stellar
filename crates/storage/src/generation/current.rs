//! current generation pointer 파일 (V.4.3).
//!
//! `design/current` 파일에는 **manifest content hash 하나만** 기록합니다
//! (ADR-0004 문자열 + LF, 총 72바이트). 전환은 temp write 후 atomic replace
//! (V.2.5)로만 수행하므로 어떤 시점에 읽어도 이전 hash 또는 새 hash 중
//! 하나만 보입니다 — partial 상태가 노출되지 않습니다.
//!
//! # 비신뢰 프로젝트 폴더 (보안 감사 STOR-1·4)
//!
//! - 읽기는 symlink를 따르되(기술 아키텍처 19.3) 연 handle이 **정규 파일**이고 72 bytes 이하일 때만
//!   읽습니다. `/dev/zero`·FIFO·거대한 파일을 가리키는 commit된 `design/current`가 메모리를 끝없이
//!   쓰거나 열기를 멈추게 하지 못합니다(crate 내부 `fs_guard`).
//! - 쓰기는 `design`이 symlink면 따라가지 않고 거부합니다(프로젝트 밖에 쓰지 않음). 없으면 한 단계만
//!   만들고, temp 생성·rename 직전에 같은 디렉터리인지 다시 확인합니다.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::atomic_file::{write_file_atomic_observed, NoopObserver, WriteObserver};
use crate::fs_guard::{ensure_real_dirs, read_regular_file, DirGuard, Link, Refusal};
use crate::hash::ContentHash;

/// current pointer 파일의 프로젝트 root 기준 상대 경로 구성 요소.
const CURRENT_POINTER_COMPONENTS: [&str; 2] = ["design", "current"];

/// current pointer 파일 크기(bytes): ADR-0004 hash 문자열(`sha256:` + hex 64자 = 71자) + LF.
///
/// 읽기 상한으로도 씁니다. 새 hash 알고리즘은 새 ADR로만 추가하므로(ADR-0004) 그때 함께 바꿉니다.
const CURRENT_POINTER_FILE_BYTES: u64 = 72;

/// current pointer 파일 절대 경로.
pub fn current_pointer_path(root: &Path) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in CURRENT_POINTER_COMPONENTS {
        path.push(component);
    }
    path
}

/// current pointer 오류.
#[derive(Debug)]
pub enum CurrentPointerError {
    /// current pointer 파일이 없습니다(아직 저장된 세대 없음).
    Missing {
        /// 없는 경로.
        path: PathBuf,
    },
    /// 파일 내용이 `hash 문자열 + LF` 형식이 아닙니다.
    InvalidFormat {
        /// 원인 설명.
        message: String,
    },
    /// 파일시스템 I/O 실패.
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// 원인.
        source: std::io::Error,
    },
}

impl fmt::Display for CurrentPointerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurrentPointerError::Missing { path } => {
                write!(f, "current pointer 파일이 없습니다: {}", path.display())
            }
            CurrentPointerError::InvalidFormat { message } => {
                write!(f, "current pointer 형식이 잘못되었습니다: {message}")
            }
            CurrentPointerError::Io { path, source } => {
                write!(f, "I/O 실패 ({}): {source}", path.display())
            }
        }
    }
}

impl std::error::Error for CurrentPointerError {}

/// current pointer를 atomic replace로 기록합니다.
pub fn write_current_pointer(root: &Path, hash: &ContentHash) -> Result<(), CurrentPointerError> {
    write_current_pointer_observed(root, hash, &mut NoopObserver)
}

/// [`write_current_pointer`]에 쓰기 단계 관찰자를 주입한 버전(fault injection 테스트용).
pub fn write_current_pointer_observed(
    root: &Path,
    hash: &ContentHash,
    observer: &mut dyn WriteObserver,
) -> Result<(), CurrentPointerError> {
    let path = current_pointer_path(root);
    // `design`을 symlink를 따르지 않고 확인·생성합니다(`create_dir_all`은 symlink를 통과함).
    let dirs = ensure_real_dirs(root, &CURRENT_POINTER_COMPONENTS[..1])
        .map_err(|(path, source)| CurrentPointerError::Io { path, source })?;
    let content = format!("{hash}\n");
    let mut guard = DirGuard::new(observer, dirs);
    write_file_atomic_observed(&path, content.as_bytes(), &mut guard).map_err(|source| {
        CurrentPointerError::Io {
            path: path.clone(),
            source,
        }
    })
}

/// current pointer를 읽습니다. 내용은 정확히 `hash 문자열 + LF`여야 합니다.
///
/// 정규 파일이 아니면(FIFO·장치·디렉터리) 읽지 않고 [`CurrentPointerError::Io`], 72 bytes를 넘으면
/// 읽지 않고 [`CurrentPointerError::InvalidFormat`]입니다.
pub fn read_current_pointer(root: &Path) -> Result<ContentHash, CurrentPointerError> {
    let path = current_pointer_path(root);
    let bytes = match read_regular_file(&path, CURRENT_POINTER_FILE_BYTES, Link::Follow) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(CurrentPointerError::Missing { path })
        }
        Err(e) if matches!(Refusal::of(&e), Some(Refusal::TooLarge { .. })) => {
            return Err(CurrentPointerError::InvalidFormat {
                message: format!("{CURRENT_POINTER_FILE_BYTES} bytes(hash 문자열 + LF)를 넘습니다"),
            })
        }
        Err(source) => return Err(CurrentPointerError::Io { path, source }),
    };
    // UTF-8이 아니면 이전(`read_to_string`)과 같이 InvalidData I/O 오류입니다.
    let content = String::from_utf8(bytes).map_err(|_| CurrentPointerError::Io {
        path: path.clone(),
        source: std::io::Error::new(ErrorKind::InvalidData, "UTF-8이 아닙니다"),
    })?;
    let Some(line) = content.strip_suffix('\n') else {
        return Err(CurrentPointerError::InvalidFormat {
            message: "마지막 LF가 없습니다".to_string(),
        });
    };
    if line.contains('\n') {
        return Err(CurrentPointerError::InvalidFormat {
            message: "hash 하나만 있어야 합니다(여러 줄 발견)".to_string(),
        });
    }
    line.parse()
        .map_err(
            |e: crate::hash::ParseContentHashError| CurrentPointerError::InvalidFormat {
                message: e.to_string(),
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic_file::WriteStage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-current-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    struct FailAt {
        stage: WriteStage,
    }
    impl WriteObserver for FailAt {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if stage == self.stage {
                return Err(std::io::Error::other(format!("주입된 중단: {stage}")));
            }
            Ok(())
        }
    }

    #[test]
    fn round_trip() {
        let dir = TempDir::new();
        let hash = ContentHash::compute(b"generation-1 manifest");
        write_current_pointer(&dir.path, &hash).unwrap();
        assert_eq!(read_current_pointer(&dir.path).unwrap(), hash);
        // 파일 내용은 hash 문자열 + LF 하나뿐(72바이트).
        let raw = std::fs::read(current_pointer_path(&dir.path)).unwrap();
        assert_eq!(raw.len(), 72);
        assert_eq!(raw.last(), Some(&b'\n'));
    }

    #[test]
    fn missing_pointer_is_reported() {
        let dir = TempDir::new();
        assert!(matches!(
            read_current_pointer(&dir.path).unwrap_err(),
            CurrentPointerError::Missing { .. }
        ));
    }

    #[test]
    fn replace_is_atomic_under_fault_injection() {
        // 검증 기준(V.4.3): current pointer가 partial 상태를 노출하지 않는 fault test.
        let dir = TempDir::new();
        let gen1 = ContentHash::compute(b"gen-1");
        let gen2 = ContentHash::compute(b"gen-2");
        write_current_pointer(&dir.path, &gen1).unwrap();

        for stage in WriteStage::ALL {
            if stage == WriteStage::SyncParentDir {
                continue; // rename 이후 — 아래 별도 확인.
            }
            let result = write_current_pointer_observed(&dir.path, &gen2, &mut FailAt { stage });
            assert!(result.is_err(), "stage {stage}에서 중단되어야 함");
            // 어떤 단계에서 중단돼도 이전 hash가 온전히 읽힙니다.
            assert_eq!(
                read_current_pointer(&dir.path).unwrap(),
                gen1,
                "stage {stage} 중단 후 이전 pointer가 보존되어야 함"
            );
        }

        // rename 이후 중단: 이미 새 hash로 전환 완료.
        let _ = write_current_pointer_observed(
            &dir.path,
            &gen2,
            &mut FailAt {
                stage: WriteStage::SyncParentDir,
            },
        );
        assert_eq!(read_current_pointer(&dir.path).unwrap(), gen2);
    }

    #[test]
    fn malformed_pointer_contents_are_rejected() {
        let dir = TempDir::new();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let hash = ContentHash::compute(b"x").to_string();
        let cases: Vec<(&str, String)> = vec![
            ("LF 없음", hash.clone()),
            ("여러 줄", format!("{hash}\n{hash}\n")),
            ("hash 아님", "not-a-hash\n".to_string()),
            ("대문자 hex", format!("{}\n", hash.to_uppercase())),
        ];
        for (name, content) in cases {
            std::fs::write(&path, content).unwrap();
            assert!(
                matches!(
                    read_current_pointer(&dir.path).unwrap_err(),
                    CurrentPointerError::InvalidFormat { .. }
                ),
                "{name} 형식이 거부되어야 함"
            );
        }
    }

    /// 거부 이유(guard)가 담긴 I/O 오류인지.
    fn io_refusal(err: CurrentPointerError) -> Refusal {
        match err {
            CurrentPointerError::Io { source, .. } => Refusal::of(&source)
                .unwrap_or_else(|| panic!("guard 거부가 아닌 I/O 오류: {source:?}")),
            other => panic!("Io 오류가 아님: {other:?}"),
        }
    }

    /// 보안 감사 STOR-1: 장치·FIFO를 가리키는 `design/current`는 열기·읽기에서 멈추거나 메모리를
    /// 끝없이 쓰지 않고 거부됩니다.
    #[cfg(unix)]
    #[test]
    fn device_and_fifo_pointers_are_refused_without_reading() {
        use crate::fs_guard::test_support::{bounded, mkfifo};

        let dir = TempDir::new();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fifo = dir.path.join("fifo");
        mkfifo(&fifo);
        for target in [Path::new("/dev/zero"), fifo.as_path()] {
            let _ = std::fs::remove_file(&path);
            std::os::unix::fs::symlink(target, &path).unwrap();
            let root = dir.path.clone();
            let err = bounded("current pointer 읽기", move || {
                read_current_pointer(&root).unwrap_err()
            });
            assert_eq!(io_refusal(err), Refusal::NotRegularFile, "{target:?}");
        }
        // 디렉터리도 정규 파일이 아닙니다.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            io_refusal(read_current_pointer(&dir.path).unwrap_err()),
            Refusal::NotRegularFile
        );
    }

    /// 보안 감사 STOR-1: 72 bytes를 넘는 파일(거대한 sparse 파일 포함)은 읽지 않고 형식 오류입니다.
    #[test]
    fn oversized_pointer_is_rejected_without_reading() {
        let dir = TempDir::new();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let hash = ContentHash::compute(b"x");
        for len in [73, 512 << 20] {
            let file = std::fs::File::create(&path).unwrap();
            file.set_len(len).unwrap();
            let CurrentPointerError::InvalidFormat { message } =
                read_current_pointer(&dir.path).unwrap_err()
            else {
                panic!("{len} bytes는 형식 오류여야 함");
            };
            assert!(message.contains("72 bytes"), "{message}");
        }
        // 정상 hash 줄 뒤에 내용이 더 있어도 크기로 거부합니다.
        std::fs::write(&path, format!("{hash}\n\n")).unwrap();
        assert!(matches!(
            read_current_pointer(&dir.path).unwrap_err(),
            CurrentPointerError::InvalidFormat { .. }
        ));
    }

    #[test]
    fn non_utf8_pointer_is_invalid_data_io() {
        let dir = TempDir::new();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0xff_u8; 72]).unwrap();
        let CurrentPointerError::Io { source, .. } = read_current_pointer(&dir.path).unwrap_err()
        else {
            panic!("Io 오류여야 함");
        };
        assert_eq!(source.kind(), ErrorKind::InvalidData);
    }

    /// 사용자 작업 트리의 symlink는 따릅니다(기술 아키텍처 19.3). 대상이 정규 파일이면 읽습니다.
    #[cfg(unix)]
    #[test]
    fn symlink_to_regular_pointer_file_is_followed() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let hash = ContentHash::compute(b"linked");
        let real = outside.path.join("current");
        std::fs::write(&real, format!("{hash}\n")).unwrap();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert_eq!(read_current_pointer(&dir.path).unwrap(), hash);
    }

    /// 보안 감사 STOR-4: `design`이 symlink면 따라가지 않고 쓰기를 거부합니다(프로젝트 밖에 쓰지 않음).
    #[cfg(unix)]
    #[test]
    fn write_refuses_symlinked_design_directory() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        std::os::unix::fs::symlink(&outside.path, dir.path.join("design")).unwrap();
        let err = write_current_pointer(&dir.path, &ContentHash::compute(b"x")).unwrap_err();
        assert_eq!(io_refusal(err), Refusal::SymlinkDirectory);
        assert_eq!(
            std::fs::read_dir(&outside.path).unwrap().count(),
            0,
            "symlink 대상에 아무것도 쓰지 않음"
        );
    }

    /// `design/current` 자리의 symlink는 따라 쓰지 않고 rename으로 정규 파일로 바뀝니다(대상 보존).
    #[cfg(unix)]
    #[test]
    fn write_replaces_pointer_symlink_without_touching_its_target() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let victim = outside.path.join("victim");
        std::fs::write(&victim, b"USER DATA").unwrap();
        let path = current_pointer_path(&dir.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        let hash = ContentHash::compute(b"new");
        write_current_pointer(&dir.path, &hash).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"USER DATA");
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_file());
        assert_eq!(read_current_pointer(&dir.path).unwrap(), hash);
    }
}
