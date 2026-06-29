//! 프로젝트 식별 (IV.2.3).
//!
//! [`ProjectIdentity`]는 **project [`StableId`]**와 **canonical root path**(절대·lexical 경로,
//! symlink 보존)의 조합입니다. 추가로, symlink 경로와 실제 경로의 **중복 open**을 판정하기
//! 위해 symlink를 모두 해석한 **canonical target**을 별도로 저장합니다. 서로 다른 symlink
//! 경로라도 같은 실제 target을 가리키면 동일 identity target으로 판정됩니다.
//!
//! 매니페스트의 `project.id`(UUID 텍스트)를 [`StableId`]로 매핑하는 [`parse_project_id`]도
//! 제공합니다.

use std::io;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;

/// 프로젝트 식별 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    /// 프로젝트 안정 식별자.
    project_id: StableId,
    /// canonical root path(절대·lexical, symlink 보존).
    root_path: PathBuf,
    /// symlink를 모두 해석한 실제 target(중복 open 판정용).
    canonical_target: PathBuf,
}

impl ProjectIdentity {
    /// project `StableId`와 열린 경로로 identity를 만듭니다.
    ///
    /// `root_path`는 절대·lexical 경로(symlink 보존)로, `canonical_target`은 symlink를 모두
    /// 해석한 실제 경로로 저장합니다. 경로가 존재하지 않으면 `canonicalize`가 실패합니다.
    pub fn new(project_id: StableId, opened_path: &Path) -> io::Result<ProjectIdentity> {
        let root_path = std::path::absolute(opened_path)?;
        let canonical_target = std::fs::canonicalize(opened_path)?;
        Ok(ProjectIdentity {
            project_id,
            root_path,
            canonical_target,
        })
    }

    /// 프로젝트 안정 식별자.
    pub fn project_id(&self) -> StableId {
        self.project_id
    }

    /// canonical root path(symlink 보존).
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// symlink를 해석한 실제 target.
    pub fn canonical_target(&self) -> &Path {
        &self.canonical_target
    }

    /// 두 identity가 같은 실제 target을 가리키는지(중복 open) 판정합니다.
    pub fn same_target(&self, other: &ProjectIdentity) -> bool {
        self.canonical_target == other.canonical_target
    }
}

/// UUID 텍스트(`8-4-4-4-12` 16진수)를 [`StableId`]로 파싱합니다. 형식이 틀리면 `None`.
pub fn parse_project_id(text: &str) -> Option<StableId> {
    // 하이픈 위치 검증: 8-4-4-4-12.
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return None;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return None;
                }
            }
        }
    }
    // 하이픈을 제거하고 16바이트로 파싱.
    let mut out = [0u8; 16];
    let hex: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'-').collect();
    debug_assert_eq!(hex.len(), 32);
    for (i, chunk) in hex.chunks_exact(2).enumerate() {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(StableId::from_bytes(out))
}

/// 16진수 한 자리를 값으로 변환합니다.
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-identity-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
            TempDir { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn parse_project_id_round_trips_known_uuid() {
        let text = "0195c530-3acd-7bc1-8414-c213f61cb965";
        let id = parse_project_id(text).expect("유효한 UUID");
        // 첫 바이트 0x01, 둘째 0x95 ...
        assert_eq!(id.to_bytes()[0], 0x01);
        assert_eq!(id.to_bytes()[1], 0x95);
        assert_eq!(id.to_bytes()[15], 0x65);
    }

    #[test]
    fn parse_project_id_rejects_malformed() {
        for bad in [
            "",
            "not-a-uuid",
            "0195c530-3acd-7bc1-8414-c213f61cb96",   // 짧음
            "0195c530-3acd-7bc1-8414-c213f61cb9655", // 김
            "0195c5303acd-7bc1-8414-c213f61cb965",   // 하이픈 위치 틀림
            "0195c530-3acd-7bc1-8414-c213f61cb96g",  // 비16진수
        ] {
            assert!(parse_project_id(bad).is_none(), "입력: {bad:?}");
        }
    }

    #[test]
    fn identity_holds_id_and_paths() {
        let dir = TempDir::new();
        let id = StableId::new_v7(1, 0, 1);
        let identity = ProjectIdentity::new(id, &dir.path).unwrap();
        assert_eq!(identity.project_id(), id);
        assert!(identity.root_path().is_absolute());
        assert!(identity.canonical_target().is_absolute());
    }

    #[cfg(unix)]
    #[test]
    fn two_symlinks_to_same_target_have_same_identity_target() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new();
        let real = dir.path.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link1 = dir.path.join("link1");
        let link2 = dir.path.join("link2");
        symlink(&real, &link1).unwrap();
        symlink(&real, &link2).unwrap();

        let id = StableId::new_v7(7, 0, 7);
        let a = ProjectIdentity::new(id, &link1).unwrap();
        let b = ProjectIdentity::new(id, &link2).unwrap();

        // 열린 경로(root_path)는 서로 다름.
        assert_ne!(a.root_path(), b.root_path());
        // 실제 target은 동일 → 중복 open 판정.
        assert_eq!(a.canonical_target(), b.canonical_target());
        assert!(a.same_target(&b));
    }

    #[cfg(unix)]
    #[test]
    fn different_targets_are_not_duplicates() {
        let dir = TempDir::new();
        let one = dir.path.join("one");
        let two = dir.path.join("two");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        let id = StableId::new_v7(1, 0, 1);
        let a = ProjectIdentity::new(id, &one).unwrap();
        let b = ProjectIdentity::new(id, &two).unwrap();
        assert!(!a.same_target(&b));
    }
}
