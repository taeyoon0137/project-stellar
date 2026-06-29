//! 기본 프로젝트 scaffold 생성 (IV.4.3).
//!
//! staging 디렉터리에 새 프로젝트의 기본 파일·폴더를 생성합니다:
//! - `stellar.yaml`: 기본 매니페스트(IV.1.5 canonical writer).
//! - `stellar.lock.yaml`: 빈 잠금 파일(IV.1.7 lock writer).
//! - `.gitignore`, `.gitattributes`: 고정 기본 내용.
//! - `design/`, `assets/`: 빈 디렉터리.
//!
//! 같은 입력이면 byte-identical 결과를 만듭니다(결정론적).

use std::io;
use std::path::Path;

use crate::lock::{Lockfile, CURRENT_LOCK_SCHEMA_VERSION};
use crate::manifest::{Manifest, ProjectMeta, CURRENT_SCHEMA_VERSION};
use crate::yaml::write_canonical;

/// 기본 `.gitignore` 내용(고정).
pub const DEFAULT_GITIGNORE: &str = "\
.DS_Store
Thumbs.db
";

/// 기본 `.gitattributes` 내용(고정). 텍스트는 LF로 정규화하고 흔한 에셋은 binary로 표시.
pub const DEFAULT_GITATTRIBUTES: &str = "\
* text=auto eol=lf
*.png binary
*.jpg binary
*.jpeg binary
*.gif binary
*.webp binary
";

/// 기본 매니페스트(빈 target_profiles·dependencies).
pub fn default_manifest(project_name: &str, project_id: &str) -> Manifest {
    Manifest {
        schema_version: CURRENT_SCHEMA_VERSION,
        project: ProjectMeta {
            id: project_id.to_string(),
            name: project_name.to_string(),
        },
        target_profiles: Vec::new(),
        dependencies: Vec::new(),
    }
}

/// 빈 잠금 파일.
pub fn empty_lockfile() -> Lockfile {
    Lockfile {
        schema_version: CURRENT_LOCK_SCHEMA_VERSION,
        resolved: Vec::new(),
    }
}

/// staging 디렉터리에 기본 scaffold를 생성합니다(staging은 이미 존재해야 함).
pub fn write_default_scaffold(
    staging: &Path,
    project_name: &str,
    project_id: &str,
) -> io::Result<()> {
    let manifest = default_manifest(project_name, project_id);
    std::fs::write(
        staging.join("stellar.yaml"),
        write_canonical(&manifest.to_yaml()),
    )?;
    std::fs::write(staging.join("stellar.lock.yaml"), empty_lockfile().write())?;
    std::fs::write(staging.join(".gitignore"), DEFAULT_GITIGNORE)?;
    std::fs::write(staging.join(".gitattributes"), DEFAULT_GITATTRIBUTES)?;
    std::fs::create_dir(staging.join("design"))?;
    std::fs::create_dir(staging.join("assets"))?;
    Ok(())
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
                .join(format!("stellar-scaffold-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const NAME: &str = "My Project";
    const ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    #[test]
    fn scaffold_matches_golden_bytes() {
        let dir = TempDir::new();
        write_default_scaffold(&dir.path, NAME, ID).unwrap();

        // stellar.yaml = canonical(manifest).
        let expected_manifest = write_canonical(&default_manifest(NAME, ID).to_yaml());
        assert_eq!(
            std::fs::read(dir.path.join("stellar.yaml")).unwrap(),
            expected_manifest.into_bytes()
        );
        // stellar.lock.yaml = 빈 잠금 파일 canonical.
        assert_eq!(
            std::fs::read(dir.path.join("stellar.lock.yaml")).unwrap(),
            empty_lockfile().write().into_bytes()
        );
        // .gitignore / .gitattributes 고정 내용.
        assert_eq!(
            std::fs::read(dir.path.join(".gitignore")).unwrap(),
            DEFAULT_GITIGNORE.as_bytes()
        );
        assert_eq!(
            std::fs::read(dir.path.join(".gitattributes")).unwrap(),
            DEFAULT_GITATTRIBUTES.as_bytes()
        );
        // 디렉터리.
        assert!(dir.path.join("design").is_dir());
        assert!(dir.path.join("assets").is_dir());
    }

    #[test]
    fn scaffold_is_deterministic() {
        let a = TempDir::new();
        let b = TempDir::new();
        write_default_scaffold(&a.path, NAME, ID).unwrap();
        write_default_scaffold(&b.path, NAME, ID).unwrap();
        for file in [
            "stellar.yaml",
            "stellar.lock.yaml",
            ".gitignore",
            ".gitattributes",
        ] {
            assert_eq!(
                std::fs::read(a.path.join(file)).unwrap(),
                std::fs::read(b.path.join(file)).unwrap(),
                "{file} 결정론 위반"
            );
        }
    }

    #[test]
    fn manifest_and_lock_round_trip_from_scaffold() {
        // 기록한 scaffold가 loader로 다시 읽혀 동일 모델로 복원되는지.
        let dir = TempDir::new();
        write_default_scaffold(&dir.path, NAME, ID).unwrap();

        let manifest_text = std::fs::read_to_string(dir.path.join("stellar.yaml")).unwrap();
        let value = crate::yaml::load_restricted(&manifest_text).unwrap();
        let manifest = Manifest::from_yaml(&value).unwrap();
        assert_eq!(manifest, default_manifest(NAME, ID));

        let lock_text = std::fs::read_to_string(dir.path.join("stellar.lock.yaml")).unwrap();
        let lock_value = crate::yaml::load_restricted(&lock_text).unwrap();
        let lock = Lockfile::from_yaml(&lock_value).unwrap();
        assert_eq!(lock, empty_lockfile());
    }
}
