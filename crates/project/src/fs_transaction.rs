//! 폴더 생성 트랜잭션 (IV.4.2).
//!
//! [`DirTransaction`]은 새 폴더를 생성하고, **`commit` 없이 드롭되면 생성한 폴더(와 그 안에
//! 만든 내용)를 제거**합니다(롤백). 생성 이후 어느 단계에서 실패하더라도 `?`로 일찍 반환하면
//! 가드가 드롭되며 롤백되어, **원래 부모 디렉터리가 오염되지 않습니다**.
//!
//! 폴더는 `create_dir`로 만들므로(이미 존재하면 실패), 트랜잭션이 잡는 폴더는 항상 **이 호출이
//! 새로 만든 것**입니다 — 롤백이 타인의 데이터를 지우지 않습니다.

use std::io;
use std::path::{Path, PathBuf};

/// 새로 만든 폴더의 생명주기를 관리하는 트랜잭션 가드.
#[derive(Debug)]
pub struct DirTransaction {
    path: PathBuf,
    committed: bool,
}

impl DirTransaction {
    /// `target` 폴더를 새로 만듭니다(이미 존재하거나 부모가 없으면 실패). 트랜잭션을 반환합니다.
    pub fn create(target: &Path) -> io::Result<DirTransaction> {
        std::fs::create_dir(target)?;
        Ok(DirTransaction {
            path: target.to_path_buf(),
            committed: false,
        })
    }

    /// 생성한 폴더 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 트랜잭션을 확정합니다(드롭 시 롤백하지 않음).
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for DirTransaction {
    fn drop(&mut self) {
        if !self.committed {
            // 롤백: 우리가 만든 폴더와 그 안에 만든 내용을 모두 제거.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-fstx-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn entry_count(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    /// 폴더를 만들고 scaffold를 쓴 뒤, `fail` 단계에서 일찍 반환(드롭→롤백)하는 흐름을 흉내.
    fn attempt(parent: &Path, fail: bool) -> Result<PathBuf, ()> {
        let tx = DirTransaction::create(&parent.join("proj")).map_err(|_| ())?;
        // scaffold 일부 작성.
        std::fs::write(tx.path().join("stellar.yaml"), b"schema_version: 1\n").map_err(|_| ())?;
        std::fs::create_dir_all(tx.path().join("design")).map_err(|_| ())?;
        if fail {
            // 이후 단계 실패 → tx 드롭 → 롤백.
            return Err(());
        }
        let path = tx.path().to_path_buf();
        tx.commit();
        Ok(path)
    }

    #[test]
    fn commit_keeps_created_folder() {
        let dir = TempDir::new();
        let created = attempt(&dir.path, false).unwrap();
        assert!(created.exists());
        assert!(created.join("stellar.yaml").is_file());
        assert!(created.join("design").is_dir());
    }

    #[test]
    fn failure_rolls_back_and_parent_is_clean() {
        let dir = TempDir::new();
        assert_eq!(entry_count(&dir.path), 0);
        let result = attempt(&dir.path, true);
        assert!(result.is_err());
        // 생성한 폴더가 제거되어 부모가 깨끗함.
        assert!(!dir.path.join("proj").exists());
        assert_eq!(entry_count(&dir.path), 0, "부모가 오염됨");
    }

    #[test]
    fn explicit_drop_without_commit_rolls_back() {
        let dir = TempDir::new();
        let target = dir.path.join("proj");
        {
            let tx = DirTransaction::create(&target).unwrap();
            std::fs::write(tx.path().join("f"), b"x").unwrap();
            assert!(target.exists());
            // commit 없이 스코프 종료 → 롤백.
        }
        assert!(!target.exists());
        assert_eq!(entry_count(&dir.path), 0);
    }

    #[test]
    fn create_fails_when_target_exists_without_touching_it() {
        let dir = TempDir::new();
        let target = dir.path.join("existing");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("precious.txt"), b"keep me").unwrap();

        // 이미 존재 → create_dir 실패. 우리가 만든 게 아니므로 롤백 대상 아님.
        assert!(DirTransaction::create(&target).is_err());
        // 기존 데이터가 보존됨.
        assert!(target.join("precious.txt").is_file());
    }

    #[test]
    fn create_fails_when_parent_missing() {
        let dir = TempDir::new();
        // 부모 경로가 없으면 create_dir 실패(create_dir_all 아님).
        let nested = dir.path.join("missing-parent").join("proj");
        assert!(DirTransaction::create(&nested).is_err());
        assert_eq!(entry_count(&dir.path), 0);
    }
}
