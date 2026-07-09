//! current generation pointer 파일 (V.4.3).
//!
//! `design/current` 파일에는 **manifest content hash 하나만** 기록합니다
//! (ADR-0004 문자열 + LF, 총 72바이트). 전환은 temp write 후 atomic replace
//! (V.2.5)로만 수행하므로 어떤 시점에 읽어도 이전 hash 또는 새 hash 중
//! 하나만 보입니다 — partial 상태가 노출되지 않습니다.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::atomic_file::{write_file_atomic_observed, NoopObserver, WriteObserver};
use crate::hash::ContentHash;

/// current pointer 파일의 프로젝트 root 기준 상대 경로 구성 요소.
const CURRENT_POINTER_COMPONENTS: [&str; 2] = ["design", "current"];

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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CurrentPointerError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let content = format!("{hash}\n");
    write_file_atomic_observed(&path, content.as_bytes(), observer).map_err(|source| {
        CurrentPointerError::Io {
            path: path.clone(),
            source,
        }
    })
}

/// current pointer를 읽습니다. 내용은 정확히 `hash 문자열 + LF`여야 합니다.
pub fn read_current_pointer(root: &Path) -> Result<ContentHash, CurrentPointerError> {
    let path = current_pointer_path(root);
    if !path.exists() {
        return Err(CurrentPointerError::Missing { path });
    }
    let content = std::fs::read_to_string(&path).map_err(|source| CurrentPointerError::Io {
        path: path.clone(),
        source,
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
}
