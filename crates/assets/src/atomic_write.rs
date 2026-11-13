//! 에셋 streaming atomic write (XIII.5.4).
//!
//! storage V.2.5 [`write_file_atomic`](stellar_storage::atomic_file::write_file_atomic)와
//! **같은 단계·순서·관찰자**([`WriteStage`]·[`WriteObserver`])를 씁니다.
//!
//! 1. 대상과 같은 디렉터리에 temp 생성(`create_new`, 같은 파일시스템 → rename이 atomic).
//!    이름이 이미 있으면 새 이름으로 다시 만들고, 기존 항목은 열거나 지우지 않습니다.
//! 2. body가 temp에 bytes를 streaming으로 기록(+검증).
//! 3. flush → 4. `sync_all`(macOS는 std가 `F_FULLFSYNC`) → 5. temp → 대상 rename
//! 6. 부모 디렉터리 sync(POSIX 전용).
//!
//! storage 함수는 `&[u8]` 전체를 받아 대형 에셋을 메모리에 올려야 하므로, 2단계만 caller
//! body(고정 buffer streaming)로 바꾼 버전을 여기 둡니다. rename이 마지막 변경 단계라
//! 어느 단계에서 중단되어도 대상 경로에 부분 파일이 생기지 않습니다. 우아한 실패에서는
//! temp를 지우고, 강제 종료로 남은 temp는 이름이 `.`로 시작해 content 파일로 해석되지
//! 않습니다([`AssetObjectPath::parse`](crate::path::AssetObjectPath::parse)).

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

pub use stellar_storage::atomic_file::{NoopObserver, WriteObserver, WriteStage};

use crate::store::temp_path_for;

/// streaming atomic write 실패.
#[derive(Debug)]
pub enum AtomicWriteError<E> {
    /// 쓰기 단계 실패(관찰자 주입 중단 포함).
    Stage {
        /// 실패 단계.
        stage: WriteStage,
        /// 원인.
        error: std::io::Error,
    },
    /// body(내용 기록·검증)가 실패했습니다.
    Body(E),
}

impl<E: fmt::Display> fmt::Display for AtomicWriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtomicWriteError::Stage { stage, error } => {
                write!(f, "atomic write {stage} 단계 실패: {error}")
            }
            AtomicWriteError::Body(e) => write!(f, "atomic write 내용 기록 실패: {e}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for AtomicWriteError<E> {}

/// 부모 디렉터리를 sync합니다(POSIX 전용).
#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

/// 부모 디렉터리를 sync합니다(Windows는 디렉터리 handle 정책이 달라 no-op).
#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

/// temp 이름 충돌 시 새 이름으로 다시 시도하는 최대 횟수.
pub(crate) const TEMP_NAME_ATTEMPTS: usize = 16;

/// `names`를 차례로 `create_new`(`O_CREAT|O_EXCL`)로 열어 처음 성공한 temp 경로와 파일을
/// 돌려줍니다.
///
/// 이미 있는 이름(다른 process의 temp, 미리 심어 둔 파일·symlink 등)은 **열거나 지우지 않고**
/// 다음 이름으로 넘어갑니다(`O_EXCL`은 마지막 구성 요소 symlink도 따라가지 않음). 그래서 호출자가
/// 정리할 temp는 항상 이 호출이 만든 항목뿐입니다. 모든 이름이 이미 있으면 `AlreadyExists`이고,
/// 다른 오류는 그대로 돌려줍니다. `options`에는 `write(true).create_new(true)`가 있어야 합니다.
pub(crate) fn create_new_temp(
    options: &OpenOptions,
    names: impl IntoIterator<Item = PathBuf>,
) -> std::io::Result<(PathBuf, File)> {
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

/// temp에 `body`로 streaming 기록한 뒤 flush → sync → atomic rename → 부모 sync합니다.
///
/// `body`가 `Err`이면 rename하지 않습니다. rename 전 실패 시 **이 호출이 만든** temp만 최선
/// 노력으로 제거합니다(temp 이름이 이미 있으면 다른 이름으로 만들고 기존 항목은 건드리지 않음).
pub fn write_atomic_streamed<T, E>(
    target: &Path,
    observer: &mut dyn WriteObserver,
    body: impl FnOnce(&mut File) -> Result<T, E>,
) -> Result<T, AtomicWriteError<E>> {
    observer
        .before(WriteStage::CreateTemp)
        .map_err(stage(WriteStage::CreateTemp))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let names = (0..TEMP_NAME_ATTEMPTS).map(|_| temp_path_for(target));
    let (temp, file) = create_new_temp(&options, names).map_err(stage(WriteStage::CreateTemp))?;
    let staged = write_and_rename(target, &temp, file, observer, body);
    if staged.is_err() {
        // rename 전 실패: 이 호출이 만든 temp를 치웁니다(실패해도 무시 — 대상 불변식과 무관).
        // rename 뒤에는 temp 이름에 아무 것도 없으므로 지우지 않습니다.
        let _ = std::fs::remove_file(&temp);
    }
    let value = staged?;

    observer
        .before(WriteStage::SyncParentDir)
        .map_err(stage(WriteStage::SyncParentDir))?;
    if let Some(parent) = target.parent() {
        sync_parent_dir(parent).map_err(stage(WriteStage::SyncParentDir))?;
    }
    Ok(value)
}

fn stage<E>(stage: WriteStage) -> impl FnOnce(std::io::Error) -> AtomicWriteError<E> {
    move |error| AtomicWriteError::Stage { stage, error }
}

/// body 기록 → flush → sync → rename(이 호출이 만든 `temp`).
fn write_and_rename<T, E>(
    target: &Path,
    temp: &Path,
    mut file: File,
    observer: &mut dyn WriteObserver,
    body: impl FnOnce(&mut File) -> Result<T, E>,
) -> Result<T, AtomicWriteError<E>> {
    observer
        .before(WriteStage::WriteBytes)
        .map_err(stage(WriteStage::WriteBytes))?;
    let value = body(&mut file).map_err(AtomicWriteError::Body)?;

    observer
        .before(WriteStage::Flush)
        .map_err(stage(WriteStage::Flush))?;
    file.flush().map_err(stage(WriteStage::Flush))?;

    observer
        .before(WriteStage::SyncFile)
        .map_err(stage(WriteStage::SyncFile))?;
    file.sync_all().map_err(stage(WriteStage::SyncFile))?;
    drop(file);

    observer
        .before(WriteStage::Rename)
        .map_err(stage(WriteStage::Rename))?;
    std::fs::rename(temp, target).map_err(stage(WriteStage::Rename))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{files_under, TempDir};

    /// 지정 단계 직전에 중단을 주입하는 관찰자.
    struct FailAt(WriteStage);

    impl WriteObserver for FailAt {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if stage == self.0 {
                return Err(std::io::Error::other(format!("주입된 중단: {stage}")));
            }
            Ok(())
        }
    }

    /// 호출된 단계를 기록하는 관찰자.
    #[derive(Default)]
    struct Recorder(Vec<WriteStage>);

    impl WriteObserver for Recorder {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            self.0.push(stage);
            Ok(())
        }
    }

    fn write_halves(file: &mut File) -> Result<(), String> {
        file.write_all(b"first half;").map_err(|e| e.to_string())?;
        file.write_all(b"second half").map_err(|e| e.to_string())
    }

    #[test]
    fn stages_run_in_storage_order() {
        let dir = TempDir::new("atomic-order");
        let target = dir.path.join("a.bin");
        let mut recorder = Recorder::default();
        write_atomic_streamed(&target, &mut recorder, write_halves).unwrap();
        assert_eq!(recorder.0, WriteStage::ALL.to_vec());
        assert_eq!(std::fs::read(&target).unwrap(), b"first half;second half");
        assert_eq!(files_under(&dir.path), vec![target], "temp 없음");
    }

    #[test]
    fn interruption_before_rename_leaves_no_final_file() {
        for fail in WriteStage::ALL {
            let dir = TempDir::new("atomic-fail");
            let target = dir.path.join("a.bin");
            let err = write_atomic_streamed(&target, &mut FailAt(fail), write_halves).unwrap_err();
            assert!(
                matches!(err, AtomicWriteError::Stage { stage, .. } if stage == fail),
                "{fail}: {err}"
            );
            if fail == WriteStage::SyncParentDir {
                // rename 이후: 대상은 완전한 내용입니다(부분 파일 아님).
                assert_eq!(std::fs::read(&target).unwrap(), b"first half;second half");
            } else {
                assert!(!target.exists(), "{fail} 중단 후 최종 파일 없음");
            }
            assert!(
                files_under(&dir.path).iter().all(|p| *p == target),
                "{fail}: temp 정리"
            );
        }
    }

    #[test]
    fn interruption_keeps_previous_target_content() {
        let dir = TempDir::new("atomic-prev");
        let target = dir.path.join("a.bin");
        std::fs::write(&target, b"previous").unwrap();
        for fail in [
            WriteStage::CreateTemp,
            WriteStage::WriteBytes,
            WriteStage::Flush,
            WriteStage::SyncFile,
            WriteStage::Rename,
        ] {
            write_atomic_streamed(&target, &mut FailAt(fail), write_halves).unwrap_err();
            assert_eq!(std::fs::read(&target).unwrap(), b"previous", "{fail}");
        }
    }

    #[test]
    fn body_failure_mid_stream_leaves_no_final_file() {
        let dir = TempDir::new("atomic-body");
        let target = dir.path.join("a.bin");
        let err = write_atomic_streamed(&target, &mut NoopObserver, |file| {
            file.write_all(b"partial").map_err(|e| e.to_string())?;
            Err::<(), String>("원본 읽기 실패".to_string())
        })
        .unwrap_err();
        assert!(matches!(err, AtomicWriteError::Body(ref m) if m == "원본 읽기 실패"));
        assert!(!target.exists());
        assert!(files_under(&dir.path).is_empty(), "temp 정리");
    }

    #[test]
    fn forced_abort_mid_stream_leaves_only_hidden_temp() {
        // 강제 종료(정리 코드 미실행)를 panic으로 모사합니다.
        let dir = TempDir::new("atomic-abort");
        let target = dir.path.join("a.bin");
        let target_for_panic = target.clone();
        let outcome = std::panic::catch_unwind(move || {
            let _ = write_atomic_streamed(
                &target_for_panic,
                &mut NoopObserver,
                |file| -> Result<(), String> {
                    file.write_all(b"partial").unwrap();
                    panic!("강제 중단")
                },
            );
        });
        assert!(outcome.is_err());
        assert!(!target.exists(), "부분 내용이 최종 이름으로 보이지 않음");
        let left = files_under(&dir.path);
        assert_eq!(left.len(), 1);
        let name = left[0].file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".a.bin.tmp-"), "{name}");
    }

    /// 미리 있는 temp 이름(다른 process temp·심어 둔 파일·symlink)은 건너뛰고 지우지 않습니다.
    #[test]
    fn existing_temp_entries_are_skipped_and_never_removed() {
        let dir = TempDir::new("atomic-squat");
        let victim = dir.path.join("victim");
        std::fs::write(&victim, b"victim").unwrap();
        let taken_file = dir.path.join(".a.bin.tmp-taken-0");
        std::fs::write(&taken_file, b"other process").unwrap();
        let taken_dir = dir.path.join(".a.bin.tmp-taken-1");
        std::fs::create_dir(&taken_dir).unwrap();
        let mut names = vec![taken_file.clone(), taken_dir.clone()];
        #[cfg(unix)]
        {
            let taken_link = dir.path.join(".a.bin.tmp-taken-2");
            std::os::unix::fs::symlink(&victim, &taken_link).unwrap();
            names.push(taken_link);
        }
        let fresh = dir.path.join(".a.bin.tmp-fresh");
        names.push(fresh.clone());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);

        let (path, _file) = create_new_temp(&options, names.clone()).unwrap();
        assert_eq!(path, fresh, "처음 비어 있는 이름");
        // 모든 이름이 차 있으면 AlreadyExists이며 아무 것도 지우거나 쓰지 않습니다.
        let err = create_new_temp(&options, names.clone()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&taken_file).unwrap(), b"other process");
        assert!(taken_dir.is_dir());
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"victim",
            "symlink를 통해 쓰지 않음"
        );
        #[cfg(unix)]
        assert!(names[2]
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());

        // body 실패 시 정리는 이 호출이 만든 temp만 지웁니다(기존 항목 보존).
        let target = dir.path.join("a.bin");
        write_atomic_streamed(&target, &mut NoopObserver, |_| Err::<(), _>("실패")).unwrap_err();
        assert_eq!(std::fs::read(&taken_file).unwrap(), b"other process");
        assert!(taken_dir.is_dir());
    }

    #[test]
    fn existing_temp_name_is_never_reused() {
        let dir = TempDir::new("atomic-unique");
        let target = dir.path.join("a.bin");
        for _ in 0..3 {
            write_atomic_streamed(&target, &mut NoopObserver, write_halves).unwrap();
        }
        assert_eq!(files_under(&dir.path), vec![target]);
    }
}
