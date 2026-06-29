//! 프로젝트 열기 probe (IV.5.1).
//!
//! 경로 선택 직후 **가벼운 검사만** 수행합니다: root 검증(IV.2.2, `stellar.yaml`+`.git` 동일
//! 경로), `stellar.yaml` YAML parse(IV.1.2), 매니페스트 스키마 검증(IV.1.1). **문서 청크
//! (`design/` 등)는 읽지 않습니다.** 신뢰 상태·문서 로드는 후속(IV.5.2~)에서 다룹니다.

use std::io;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestError};
use crate::root::{validate_project_root, GitLink, RootError, MANIFEST_FILE};
use crate::yaml::{load_restricted, LoaderError};

/// probe 성공 결과(문서는 아직 로드하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProbe {
    /// 프로젝트 root 경로.
    pub root: PathBuf,
    /// `.git` 링크 종류.
    pub git_link: GitLink,
    /// 파싱한 매니페스트.
    pub manifest: Manifest,
}

/// probe 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// root 검증 실패(매니페스트·`.git` 누락/불일치).
    Root(RootError),
    /// `stellar.yaml` 읽기 실패.
    ManifestRead {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `stellar.yaml` YAML 문법 오류(손상).
    ManifestSyntax(LoaderError),
    /// 매니페스트 스키마 오류.
    ManifestSchema(ManifestError),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Root(e) => write!(f, "프로젝트 root 오류: {e}"),
            ProbeError::ManifestRead { kind } => write!(f, "stellar.yaml 읽기 실패: {kind:?}"),
            ProbeError::ManifestSyntax(e) => write!(f, "stellar.yaml 손상: {e}"),
            ProbeError::ManifestSchema(e) => write!(f, "매니페스트 스키마 오류: {e}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// 경로를 probe 합니다: root 검증 → 매니페스트 parse → 결과 반환(문서는 읽지 않음).
pub fn probe_project(path: &Path) -> Result<ProjectProbe, ProbeError> {
    // 1. root 검증(stellar.yaml + .git 동일 경로).
    let git_link = validate_project_root(path).map_err(ProbeError::Root)?;

    // 2. stellar.yaml 읽기.
    let text = std::fs::read_to_string(path.join(MANIFEST_FILE))
        .map_err(|e| ProbeError::ManifestRead { kind: e.kind() })?;

    // 3. YAML parse(제한 loader).
    let value = load_restricted(&text).map_err(ProbeError::ManifestSyntax)?;

    // 4. 매니페스트 스키마 검증.
    let manifest = Manifest::from_yaml(&value).map_err(ProbeError::ManifestSchema)?;

    Ok(ProjectProbe {
        root: path.to_path_buf(),
        git_link,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ProjectMeta, CURRENT_SCHEMA_VERSION};
    use crate::yaml::write_canonical;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-probe-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sample_manifest() -> Manifest {
        Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "My Project".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    fn write_manifest(dir: &Path, contents: &str) {
        std::fs::write(dir.join(MANIFEST_FILE), contents).unwrap();
    }

    fn make_git_dir(dir: &Path) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
    }

    #[test]
    fn normal_project_probes_successfully() {
        let dir = TempDir::new();
        write_manifest(&dir.path, &write_canonical(&sample_manifest().to_yaml()));
        make_git_dir(&dir.path);

        let probe = probe_project(&dir.path).unwrap();
        assert_eq!(probe.git_link, GitLink::Directory);
        assert_eq!(probe.manifest, sample_manifest());
        // 문서 청크는 읽지 않음 — design/ 없어도 성공.
        assert!(!dir.path.join("design").exists());
    }

    #[test]
    fn corrupt_yaml_is_manifest_syntax_error() {
        let dir = TempDir::new();
        // anchor가 든 손상 YAML(제한 loader가 거부).
        write_manifest(&dir.path, "schema_version: &a 1\nproject: *a\n");
        make_git_dir(&dir.path);
        assert!(matches!(
            probe_project(&dir.path),
            Err(ProbeError::ManifestSyntax(_))
        ));
    }

    #[test]
    fn bad_schema_is_manifest_schema_error() {
        let dir = TempDir::new();
        // 문법은 맞지만 필수 필드 누락.
        write_manifest(&dir.path, "schema_version: 1\n");
        make_git_dir(&dir.path);
        assert!(matches!(
            probe_project(&dir.path),
            Err(ProbeError::ManifestSchema(_))
        ));
    }

    #[test]
    fn git_mismatch_is_root_error() {
        let dir = TempDir::new();
        // 매니페스트는 있으나 .git 없음 → root 검증 실패(Git 불일치).
        write_manifest(&dir.path, &write_canonical(&sample_manifest().to_yaml()));
        assert_eq!(
            probe_project(&dir.path),
            Err(ProbeError::Root(RootError::MissingGit))
        );
    }

    #[test]
    fn missing_manifest_is_root_error() {
        let dir = TempDir::new();
        make_git_dir(&dir.path);
        assert_eq!(
            probe_project(&dir.path),
            Err(ProbeError::Root(RootError::MissingManifest))
        );
    }
}
