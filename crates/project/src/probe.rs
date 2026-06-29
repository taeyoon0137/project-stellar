//! 프로젝트 열기 probe (IV.5.1).
//!
//! 경로 선택 직후 **가벼운 검사만** 수행합니다: root 검증(IV.2.2, `stellar.yaml`+`.git` 동일
//! 경로), `stellar.yaml` YAML parse(IV.1.2), 매니페스트 스키마 검증(IV.1.1). **문서 청크
//! (`design/` 등)는 읽지 않습니다.** 신뢰 상태·문서 로드는 후속(IV.5.2~)에서 다룹니다.

use std::io;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestError};
use crate::root::{validate_project_root, GitLink, RootError, MANIFEST_FILE};
use crate::trust::TrustStatus;
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

/// 신뢰 인식 probe 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeProbeError {
    /// 기본 probe 오류.
    Probe(ProbeError),
    /// 비신뢰 프로젝트에서 따르지 않는 symlink(외부 대상 가능).
    UntrustedSymlinkRefused {
        /// 거부한 경로.
        path: PathBuf,
    },
}

impl std::fmt::Display for SafeProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafeProbeError::Probe(e) => write!(f, "{e}"),
            SafeProbeError::UntrustedSymlinkRefused { path } => {
                write!(f, "비신뢰 프로젝트의 symlink를 따르지 않습니다: {path:?}")
            }
        }
    }
}

impl std::error::Error for SafeProbeError {}

/// 신뢰 상태를 고려한 안전 probe (IV.5.3).
///
/// **신뢰되지 않은** 프로젝트는 **문서 메타데이터(`stellar.yaml`)만** 읽습니다. 다음을 하지
/// 않습니다:
/// - **Git 실행 안 함**: `.git`은 존재만 확인하고 git 프로세스를 실행하지 않습니다(외부 프로세스
///   미실행).
/// - **외부 symlink 미추종**: `stellar.yaml`이 symlink면 따르지 않고 거부합니다(외부 파일 노출
///   방지).
/// - **필터 미실행**: clean/smudge 등 어떤 필터도 실행하지 않습니다(git 미실행이므로 구조적으로
///   불가능).
pub fn safe_probe(path: &Path, trust: TrustStatus) -> Result<ProjectProbe, SafeProbeError> {
    if trust == TrustStatus::NotTrusted {
        // 외부 symlink 거부: 매니페스트가 symlink면 따르지 않음.
        let manifest_path = path.join(MANIFEST_FILE);
        if is_symlink(&manifest_path) {
            return Err(SafeProbeError::UntrustedSymlinkRefused {
                path: manifest_path,
            });
        }
    }
    // 메타데이터 전용 probe(git 실행·필터 없음).
    probe_project(path).map_err(SafeProbeError::Probe)
}

/// 경로가 symlink인지(대상을 따르지 않고) 확인합니다.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
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

    // --- IV.5.3: 비신뢰 probe 안전 제약 ---

    #[test]
    fn untrusted_normal_project_reads_metadata_only() {
        let dir = TempDir::new();
        write_manifest(&dir.path, &write_canonical(&sample_manifest().to_yaml()));
        make_git_dir(&dir.path);
        // 비신뢰여도 정상 매니페스트는 메타데이터를 읽음.
        let probe = safe_probe(&dir.path, TrustStatus::NotTrusted).unwrap();
        assert_eq!(probe.manifest, sample_manifest());
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_refuses_symlinked_manifest() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        make_git_dir(&dir.path);
        // 외부 파일을 가리키는 stellar.yaml symlink(악성).
        let external = dir.path.join("external-secret.yaml");
        std::fs::write(&external, "schema_version: 1\n").unwrap();
        symlink(&external, dir.path.join(MANIFEST_FILE)).unwrap();

        // 비신뢰 → symlink를 따르지 않고 거부.
        assert!(matches!(
            safe_probe(&dir.path, TrustStatus::NotTrusted),
            Err(SafeProbeError::UntrustedSymlinkRefused { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn malicious_fixture_does_not_run_external_process() {
        // .git/config에 fsmonitor로 marker 생성을 심어, git이 실행되면 marker가 생김.
        // safe_probe(비신뢰)는 git을 실행하지 않으므로 marker가 생기지 않아야 함.
        let dir = TempDir::new();
        write_manifest(&dir.path, &write_canonical(&sample_manifest().to_yaml()));
        let git = dir.path.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let marker = dir.path.join("PWNED_BY_GIT");
        std::fs::write(
            git.join("config"),
            format!("[core]\n\tfsmonitor = touch {}\n", marker.to_string_lossy()),
        )
        .unwrap();

        let probe = safe_probe(&dir.path, TrustStatus::NotTrusted).unwrap();
        assert_eq!(probe.manifest, sample_manifest());
        // 외부 프로세스(fsmonitor)가 실행되지 않아 marker가 없음.
        assert!(
            !marker.exists(),
            "비신뢰 probe가 git/외부 프로세스를 실행함(보안 결함)"
        );
    }
}
