//! atomic 파일 쓰기 플랫폼 abstraction (V.2.5).
//!
//! 쓰기 단계는 항상 다음 순서입니다:
//!
//! 1. 대상과 **같은 디렉터리**에 temp 파일 생성(같은 파일시스템 → rename이 atomic).
//!    `create_new`(`O_CREAT|O_EXCL`)로만 만들고, 이름이 이미 있으면(다른 process의 temp, 미리 심어
//!    둔 파일·symlink) 열거나 지우지 않고 다음 이름으로 다시 만듭니다(보안 감사 STOR-5).
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
//! [`WriteObserver`]로 주입해 테스트합니다. 우아한 실패에서 지우는 temp는 **이 호출이 만든**
//! 항목뿐입니다.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
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

/// temp 이름의 process 내 일련번호.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// temp 이름 충돌 시 새 이름으로 다시 시도하는 최대 횟수(assets `atomic_write`와 같은 값).
const TEMP_NAME_ATTEMPTS: usize = 16;

/// 대상과 같은 디렉터리에 만들 temp 파일 경로(`.<name>.tmp-<pid>-<n>`).
fn temp_path_for(target: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    let temp_name = format!(".{file_name}.tmp-{}-{n}", std::process::id());
    target.with_file_name(temp_name)
}

/// `names`를 차례로 `create_new`(`O_CREAT|O_EXCL`, 쓰기 전용)로 열어 처음 성공한 temp 경로와
/// 파일을 돌려줍니다.
///
/// 이미 있는 이름(다른 process의 temp, 미리 심어 둔 파일·symlink 등)은 **열거나 지우지 않고** 다음
/// 이름으로 넘어갑니다. `O_EXCL`은 마지막 구성 요소가 symlink면 따라가지 않고 실패하므로, 예측한
/// temp 이름에 심은 symlink로 프로젝트 밖 파일을 truncate·덮어쓰지 않습니다. 모든 이름이 이미 있으면
/// `AlreadyExists`이고 다른 오류는 그대로 돌려줍니다.
fn create_new_temp(names: impl IntoIterator<Item = PathBuf>) -> std::io::Result<(PathBuf, File)> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    for path in names {
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        ErrorKind::AlreadyExists,
        "사용 가능한 temp 이름이 없습니다",
    ))
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
/// 관찰자가 `Err`를 반환하면 그 단계를 실행하지 않고 중단합니다. 이 호출이 만든 temp
/// 파일은 최선 노력으로 제거하지만(우아한 실패), 강제 종료 시 남을 수 있습니다 — 대상
/// 경로 불변식(이전 상태 유지)은 rename이 마지막 변경 단계라는 데서 나옵니다.
pub fn write_file_atomic_observed(
    target: &Path,
    bytes: &[u8],
    observer: &mut dyn WriteObserver,
) -> std::io::Result<()> {
    write_file_atomic_with_names(target, bytes, observer, || temp_path_for(target))
}

/// [`write_file_atomic_observed`] 본체. `next_name`은 temp 후보 이름을 만듭니다(테스트가 이름을
/// 고정하는 지점이며, 실제 경로는 [`temp_path_for`]).
fn write_file_atomic_with_names(
    target: &Path,
    bytes: &[u8],
    observer: &mut dyn WriteObserver,
    mut next_name: impl FnMut() -> PathBuf,
) -> std::io::Result<()> {
    observer.before(WriteStage::CreateTemp)?;
    let (temp, file) = create_new_temp((0..TEMP_NAME_ATTEMPTS).map(|_| next_name()))?;
    let staged = write_and_rename(target, &temp, file, bytes, observer);
    if staged.is_err() {
        // rename 전 실패: 이 호출이 만든 temp만 치웁니다(실패해도 무시 — 대상 불변식과 무관).
        // rename 뒤에는 temp 이름에 아무 것도 없으므로 지우지 않습니다.
        let _ = std::fs::remove_file(&temp);
    }
    staged?;

    observer.before(WriteStage::SyncParentDir)?;
    if let Some(parent) = target.parent() {
        sync_parent_dir(parent)?;
    }
    Ok(())
}

/// bytes 기록 → flush → sync → rename(이 호출이 만든 `temp`).
fn write_and_rename(
    target: &Path,
    temp: &Path,
    mut file: File,
    bytes: &[u8],
    observer: &mut dyn WriteObserver,
) -> std::io::Result<()> {
    observer.before(WriteStage::WriteBytes)?;
    file.write_all(bytes)?;

    observer.before(WriteStage::Flush)?;
    file.flush()?;

    observer.before(WriteStage::SyncFile)?;
    file.sync_all()?;
    drop(file);

    observer.before(WriteStage::Rename)?;
    std::fs::rename(temp, target)
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

    /// 보안 감사 STOR-5: 예측 가능한 temp 이름에 미리 심은 symlink를 따라가 프로젝트 밖 파일을
    /// truncate·덮어쓰지 않고, 대상이 symlink로 바뀌지도 않습니다.
    #[cfg(unix)]
    #[test]
    fn planted_symlinks_at_predicted_temp_names_are_never_followed() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let victim = outside.path.join("victim.txt");
        std::fs::write(&victim, b"USER DATA - must not change").unwrap();
        let target = dir.path.join("current");
        // 다음에 쓸 일련번호부터 8개(TEMP_NAME_ATTEMPTS보다 적게) 심습니다. 병렬 테스트가 그 사이
        // 번호를 써도 검사는 그대로 성립합니다(심은 이름을 건너뛸 뿐).
        let start = TEMP_COUNTER.load(Ordering::Relaxed);
        let planted: Vec<PathBuf> = (start..start + 8)
            .map(|n| {
                dir.path
                    .join(format!(".current.tmp-{}-{n}", std::process::id()))
            })
            .collect();
        for link in &planted {
            std::os::unix::fs::symlink(&victim, link).unwrap();
        }

        write_file_atomic(&target, b"sha256:new\n").unwrap();

        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"USER DATA - must not change"
        );
        let metadata = std::fs::symlink_metadata(&target).unwrap();
        assert!(metadata.file_type().is_file(), "대상은 정규 파일");
        assert_eq!(std::fs::read(&target).unwrap(), b"sha256:new\n");
        for link in &planted {
            let kept = std::fs::symlink_metadata(link).unwrap();
            assert!(kept.file_type().is_symlink(), "심은 항목은 지우지 않음");
        }
    }

    /// 이미 있는 temp 이름(다른 process temp·파일·디렉터리·symlink)은 건너뛰고 열거나 지우지 않습니다.
    /// 모든 이름이 차 있으면 `AlreadyExists`이며 대상도 만들지 않습니다.
    #[test]
    fn existing_temp_entries_are_skipped_and_never_removed() {
        let dir = TempDir::new();
        let target = dir.path.join("a.bin");
        let taken_file = dir.path.join(".a.bin.tmp-taken-0");
        std::fs::write(&taken_file, b"other process").unwrap();
        let taken_dir = dir.path.join(".a.bin.tmp-taken-1");
        std::fs::create_dir(&taken_dir).unwrap();
        let mut taken = vec![taken_file.clone(), taken_dir.clone()];
        #[cfg(unix)]
        {
            let dangling = dir.path.join(".a.bin.tmp-taken-2");
            std::os::unix::fs::symlink(dir.path.join("would-be-created"), &dangling).unwrap();
            taken.push(dangling);
        }

        // 모든 후보가 차 있음 → AlreadyExists, 아무 것도 쓰거나 지우지 않음.
        let mut names = taken.clone().into_iter().cycle();
        let err = write_file_atomic_with_names(&target, b"x", &mut NoopObserver, || {
            names.next().unwrap()
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert!(!target.exists());
        assert!(
            !dir.path.join("would-be-created").exists(),
            "symlink 대상 생성 없음"
        );
        assert_eq!(std::fs::read(&taken_file).unwrap(), b"other process");
        assert!(taken_dir.is_dir());

        // 빈 이름이 하나 있으면 그 이름으로 기록합니다.
        let fresh = dir.path.join(".a.bin.tmp-fresh");
        let mut names = taken.clone().into_iter().chain([fresh.clone()]);
        write_file_atomic_with_names(&target, b"content", &mut NoopObserver, || {
            names.next().unwrap()
        })
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"content");
        assert!(!fresh.exists(), "temp는 대상으로 rename됨");
        assert!(!dir.path.join("would-be-created").exists());
        assert_eq!(std::fs::read(&taken_file).unwrap(), b"other process");

        // 우아한 실패 정리는 이 호출이 만든 temp만 지웁니다(기존 항목 보존).
        let fresh = dir.path.join(".a.bin.tmp-fresh-2");
        let mut names = taken.clone().into_iter().chain([fresh.clone()]);
        let mut fail = FailAt {
            stage: WriteStage::Rename,
        };
        write_file_atomic_with_names(&target, b"new", &mut fail, || names.next().unwrap())
            .unwrap_err();
        assert!(!fresh.exists(), "이 호출이 만든 temp는 정리");
        assert_eq!(std::fs::read(&target).unwrap(), b"content");
        assert_eq!(std::fs::read(&taken_file).unwrap(), b"other process");
        assert!(taken_dir.is_dir());
        for entry in &taken {
            assert!(std::fs::symlink_metadata(entry).is_ok(), "{entry:?} 보존");
        }
    }

    #[test]
    fn create_temp_failure_before_creation_touches_nothing() {
        // CreateTemp 직전 중단: temp를 만들지 않았으므로 지울 것도 없습니다(같은 이름의 기존 항목 보존).
        let dir = TempDir::new();
        let target = dir.path.join("a.bin");
        let existing = dir.path.join(".a.bin.tmp-existing");
        std::fs::write(&existing, b"keep").unwrap();
        let mut fail = FailAt {
            stage: WriteStage::CreateTemp,
        };
        write_file_atomic_with_names(&target, b"x", &mut fail, || existing.clone()).unwrap_err();
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep");
        assert!(!target.exists());
    }
}
