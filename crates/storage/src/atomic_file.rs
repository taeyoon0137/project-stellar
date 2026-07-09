//! atomic 파일 쓰기 플랫폼 abstraction (V.2.5).
//!
//! 쓰기 단계는 항상 다음 순서입니다:
//!
//! 1. 대상과 **같은 디렉터리**에 temp 파일 생성(같은 파일시스템 → rename이 atomic).
//! 2. bytes 기록.
//! 3. flush(사용자 공간 buffer 비움).
//! 4. sync_file(디스크 내구성 — macOS는 std가 `F_FULLFSYNC`, 그 외 POSIX는 fsync,
//!    Windows는 FlushFileBuffers).
//! 5. temp → 대상 atomic rename(기존 파일 교체 포함 — Windows는 std가
//!    `MOVEFILE_REPLACE_EXISTING`).
//! 6. 부모 디렉터리 sync(POSIX 전용 — rename 항목의 내구성. Windows는 디렉터리
//!    handle 정책이 달라 no-op).
//!
//! rename 전 어느 단계에서 중단되어도 대상 경로는 이전 상태(없음 또는 이전 내용)를
//! 유지합니다 — 부분 파일이 current로 읽히지 않습니다. 각 단계 중단은
//! [`WriteObserver`]로 주입해 테스트합니다.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// atomic 쓰기의 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteStage {
    /// temp 파일 생성.
    CreateTemp,
    /// bytes 기록.
    WriteBytes,
    /// flush.
    Flush,
    /// 파일 내구성 sync.
    SyncFile,
    /// temp → 대상 rename.
    Rename,
    /// 부모 디렉터리 sync.
    SyncParentDir,
}

impl WriteStage {
    /// 모든 단계(순서대로).
    pub const ALL: [WriteStage; 6] = [
        WriteStage::CreateTemp,
        WriteStage::WriteBytes,
        WriteStage::Flush,
        WriteStage::SyncFile,
        WriteStage::Rename,
        WriteStage::SyncParentDir,
    ];
}

impl fmt::Display for WriteStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            WriteStage::CreateTemp => "create-temp",
            WriteStage::WriteBytes => "write-bytes",
            WriteStage::Flush => "flush",
            WriteStage::SyncFile => "sync-file",
            WriteStage::Rename => "rename",
            WriteStage::SyncParentDir => "sync-parent-dir",
        };
        f.write_str(name)
    }
}

/// 각 쓰기 단계 직전에 호출되는 관찰자.
///
/// `Err` 반환은 해당 단계 직전의 강제 중단(전원 차단·프로세스 kill)을 모사합니다.
/// 프로덕션 경로는 [`NoopObserver`]를 사용합니다.
pub trait WriteObserver {
    /// `stage` 실행 직전 호출됩니다.
    fn before(&mut self, stage: WriteStage) -> std::io::Result<()>;
}

/// 아무 것도 주입하지 않는 기본 관찰자.
#[derive(Debug, Default)]
pub struct NoopObserver;

impl WriteObserver for NoopObserver {
    fn before(&mut self, _stage: WriteStage) -> std::io::Result<()> {
        Ok(())
    }
}

/// 대상과 같은 디렉터리에 만들 temp 파일 경로.
fn temp_path_for(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    let temp_name = format!(".{file_name}.tmp-{}-{n}", std::process::id());
    target.with_file_name(temp_name)
}

/// 부모 디렉터리를 sync합니다(POSIX 전용, Windows는 no-op).
#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}

/// 부모 디렉터리를 sync합니다(POSIX 전용, Windows는 no-op).
#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

/// bytes를 temp write → flush → sync → atomic rename으로 대상 경로에 기록합니다.
pub fn write_file_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_file_atomic_observed(target, bytes, &mut NoopObserver)
}

/// [`write_file_atomic`]에 단계 관찰자를 주입한 버전(fault injection 테스트용).
///
/// 관찰자가 `Err`를 반환하면 그 단계를 실행하지 않고 중단합니다. temp 파일은
/// 최선 노력으로 제거하지만(우아한 실패), 강제 종료 시 남을 수 있습니다 — 대상
/// 경로 불변식(이전 상태 유지)은 rename이 마지막 변경 단계라는 데서 나옵니다.
pub fn write_file_atomic_observed(
    target: &Path,
    bytes: &[u8],
    observer: &mut dyn WriteObserver,
) -> std::io::Result<()> {
    let temp = temp_path_for(target);
    let result = write_steps(target, &temp, bytes, observer);
    if result.is_err() {
        // 우아한 실패 경로에서는 temp를 치웁니다(실패해도 무시 — 대상 불변식과 무관).
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn write_steps(
    target: &Path,
    temp: &Path,
    bytes: &[u8],
    observer: &mut dyn WriteObserver,
) -> std::io::Result<()> {
    observer.before(WriteStage::CreateTemp)?;
    let mut file = std::fs::File::create(temp)?;

    observer.before(WriteStage::WriteBytes)?;
    file.write_all(bytes)?;

    observer.before(WriteStage::Flush)?;
    file.flush()?;

    observer.before(WriteStage::SyncFile)?;
    file.sync_all()?;
    drop(file);

    observer.before(WriteStage::Rename)?;
    std::fs::rename(temp, target)?;

    observer.before(WriteStage::SyncParentDir)?;
    if let Some(parent) = target.parent() {
        sync_parent_dir(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-atomicfile-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 지정 단계 직전에 중단을 주입하는 관찰자.
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
    fn successful_write_creates_exact_content() {
        let dir = TempDir::new();
        let target = dir.path.join("current.bin");
        write_file_atomic(&target, b"hello atomic").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello atomic");
        // temp 파일이 남지 않음.
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 1);
    }

    #[test]
    fn replace_existing_is_atomic() {
        let dir = TempDir::new();
        let target = dir.path.join("current.bin");
        write_file_atomic(&target, b"generation-1").unwrap();
        write_file_atomic(&target, b"generation-2").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"generation-2");
    }

    #[test]
    fn fault_before_each_stage_never_exposes_partial_file_fresh_target() {
        // 검증 기준(V.2.5): 각 write 단계 강제 종료에서 부분 파일을 current로 읽지 않음.
        // rename 이전 단계 중단 → 대상 없음. rename 자체 중단 → 대상 없음.
        for stage in WriteStage::ALL {
            if stage == WriteStage::SyncParentDir {
                continue; // rename 이후 — 아래 별도 검증.
            }
            let dir = TempDir::new();
            let target = dir.path.join("current.bin");
            let err = write_file_atomic_observed(&target, b"new bytes", &mut FailAt { stage })
                .unwrap_err();
            assert!(err.to_string().contains("주입된 중단"), "stage {stage}");
            assert!(
                !target.exists(),
                "stage {stage} 중단 후 대상이 존재하면 안 됨"
            );
        }
    }

    #[test]
    fn fault_before_each_stage_keeps_previous_content() {
        // 기존 내용이 있는 대상: rename 이전 어떤 단계에서 중단해도 이전 내용 유지.
        for stage in WriteStage::ALL {
            if stage == WriteStage::SyncParentDir {
                continue;
            }
            let dir = TempDir::new();
            let target = dir.path.join("current.bin");
            write_file_atomic(&target, b"old generation").unwrap();
            let _ = write_file_atomic_observed(&target, b"new generation", &mut FailAt { stage })
                .unwrap_err();
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"old generation",
                "stage {stage} 중단 후 이전 내용이 보존되어야 함"
            );
        }
    }

    #[test]
    fn fault_after_rename_keeps_new_content() {
        // rename 이후(SyncParentDir 직전) 중단: 이미 전환 완료 — 새 내용이 보입니다.
        let dir = TempDir::new();
        let target = dir.path.join("current.bin");
        write_file_atomic(&target, b"old").unwrap();
        let _ = write_file_atomic_observed(
            &target,
            b"new",
            &mut FailAt {
                stage: WriteStage::SyncParentDir,
            },
        )
        .unwrap_err();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn graceful_failure_cleans_temp_file() {
        let dir = TempDir::new();
        let target = dir.path.join("current.bin");
        let _ = write_file_atomic_observed(
            &target,
            b"bytes",
            &mut FailAt {
                stage: WriteStage::Rename,
            },
        )
        .unwrap_err();
        // 우아한 실패 경로에서는 temp가 제거됩니다.
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 0);
    }
}
