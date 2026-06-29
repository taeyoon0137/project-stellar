//! 프로젝트 신뢰 상태 (IV.5.2).
//!
//! 신뢰 상태는 [`ProjectIdentity`]의 **canonical target**(symlink를 모두 해석한 실경로)을 키로
//! 저장합니다. 따라서:
//! - **symlink alias**: 같은 target을 가리키는 서로 다른 symlink 경로는 신뢰를 **공유**합니다.
//! - **root rename/move**: 폴더를 옮기면 canonical target이 달라져 신뢰가 **이어지지 않습니다**
//!   (재신뢰 필요 — 옮겨지거나 복제된 폴더를 자동 신뢰하지 않는 안전 정책).
//!
//! 본 모듈은 **인터페이스**([`TrustStore`])와 in-memory 구현을 정의합니다. OS 사용자 데이터
//! 영역에 영속하는 실제 구현과 UI 연결은 후속에서 합니다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::identity::ProjectIdentity;

/// 신뢰 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustStatus {
    /// 신뢰됨.
    Trusted,
    /// 신뢰되지 않음(기본값).
    NotTrusted,
}

/// 신뢰 저장 키: identity의 canonical target(실경로).
pub fn trust_key(identity: &ProjectIdentity) -> &Path {
    identity.canonical_target()
}

/// 프로젝트 신뢰 상태 저장 인터페이스.
pub trait TrustStore {
    /// identity의 신뢰 상태(모르면 `NotTrusted`).
    fn status(&self, identity: &ProjectIdentity) -> TrustStatus;
    /// identity의 신뢰 상태를 설정합니다.
    fn set(&mut self, identity: &ProjectIdentity, status: TrustStatus);
    /// identity의 신뢰 기록을 제거합니다(이후 `NotTrusted`).
    fn forget(&mut self, identity: &ProjectIdentity);
}

/// in-memory 신뢰 저장소(신뢰된 canonical target 집합). 테스트·임시용.
#[derive(Debug, Default)]
pub struct InMemoryTrustStore {
    trusted: HashSet<PathBuf>,
}

impl InMemoryTrustStore {
    /// 빈 저장소.
    pub fn new() -> InMemoryTrustStore {
        InMemoryTrustStore {
            trusted: HashSet::new(),
        }
    }
}

impl TrustStore for InMemoryTrustStore {
    fn status(&self, identity: &ProjectIdentity) -> TrustStatus {
        if self.trusted.contains(trust_key(identity)) {
            TrustStatus::Trusted
        } else {
            TrustStatus::NotTrusted
        }
    }

    fn set(&mut self, identity: &ProjectIdentity, status: TrustStatus) {
        let key = trust_key(identity);
        match status {
            TrustStatus::Trusted => {
                self.trusted.insert(key.to_path_buf());
            }
            TrustStatus::NotTrusted => {
                self.trusted.remove(key);
            }
        }
    }

    fn forget(&mut self, identity: &ProjectIdentity) {
        self.trusted.remove(trust_key(identity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-trust-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn id() -> StableId {
        StableId::new_v7(1, 0, 1)
    }

    #[test]
    fn unknown_is_not_trusted_by_default() {
        let dir = TempDir::new();
        let identity = ProjectIdentity::new(id(), &dir.path).unwrap();
        let store = InMemoryTrustStore::new();
        assert_eq!(store.status(&identity), TrustStatus::NotTrusted);
    }

    #[test]
    fn set_and_forget_trust() {
        let dir = TempDir::new();
        let identity = ProjectIdentity::new(id(), &dir.path).unwrap();
        let mut store = InMemoryTrustStore::new();
        store.set(&identity, TrustStatus::Trusted);
        assert_eq!(store.status(&identity), TrustStatus::Trusted);
        store.forget(&identity);
        assert_eq!(store.status(&identity), TrustStatus::NotTrusted);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_shares_trust() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let real = dir.path.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path.join("alias");
        symlink(&real, &link).unwrap();

        let via_real = ProjectIdentity::new(id(), &real).unwrap();
        let via_alias = ProjectIdentity::new(id(), &link).unwrap();
        // 열린 경로는 다르지만 canonical target은 같음.
        assert_ne!(via_real.root_path(), via_alias.root_path());
        assert_eq!(via_real.canonical_target(), via_alias.canonical_target());

        let mut store = InMemoryTrustStore::new();
        store.set(&via_real, TrustStatus::Trusted);
        // alias 경로로 열어도 신뢰 공유.
        assert_eq!(store.status(&via_alias), TrustStatus::Trusted);
    }

    #[cfg(unix)]
    #[test]
    fn root_rename_does_not_carry_trust() {
        let dir = TempDir::new();
        let a = dir.path.join("a");
        std::fs::create_dir_all(&a).unwrap();
        let id_a = ProjectIdentity::new(id(), &a).unwrap();

        let mut store = InMemoryTrustStore::new();
        store.set(&id_a, TrustStatus::Trusted);
        assert_eq!(store.status(&id_a), TrustStatus::Trusted);

        // 폴더를 b로 rename/move.
        let b = dir.path.join("b");
        std::fs::rename(&a, &b).unwrap();
        let id_b = ProjectIdentity::new(id(), &b).unwrap();

        // 옮긴 폴더는 신뢰가 이어지지 않음(재신뢰 필요).
        assert_eq!(store.status(&id_b), TrustStatus::NotTrusted);
    }
}
