//! 프로젝트 열기 probe (IV.5.1).
//!
//! 경로 선택 직후 **가벼운 검사만** 수행합니다: root 검증(IV.2.2, `stellar.yaml`+`.git` 동일
//! 경로), `stellar.yaml` YAML parse(IV.1.2), 매니페스트 스키마 검증(IV.1.1). **문서 청크
//! (`design/` 등)는 읽지 않습니다.** 신뢰 상태·문서 로드는 후속(IV.5.2~)에서 다룹니다.
//!
//! `stellar.yaml`은 신뢰되지 않은 입력이므로 loader 상한
//! ([`MAX_INPUT_BYTES`](crate::yaml::loader::MAX_INPUT_BYTES))보다 많이 읽지
//! 않습니다. 열린 파일의 크기가 상한을 넘으면 내용을 읽기 전에 거부하고, 크기 확인 뒤 파일이
//! 자라더라도 상한 + 1 바이트에서 읽기를 멈춥니다.
//!
//! # 열린 handle 기준 판정
//!
//! root 검증과 symlink 사전 검사는 경로 기준이라 검사와 열기 사이에 `stellar.yaml`이 바뀔 수
//! 있습니다. 그래서 판정은 연 handle로 다시 합니다.
//!
//! - macOS에서는 `O_NONBLOCK`으로 엽니다. 검사 뒤 FIFO·장치로 바뀌어도 열기가 writer·장치를
//!   기다리며 멈추지 않습니다(정규 파일 읽기에는 영향 없음). 연 handle의 `fstat`이 정규 파일이
//!   아니면 [`ProbeError::ManifestRead`](`InvalidInput`)로 거부합니다.
//! - 비신뢰 probe([`safe_probe`])는 macOS에서 `O_NOFOLLOW`를 함께 줍니다. 검사 뒤 symlink로
//!   바뀌면 열기가 `ELOOP`로 실패하므로 [`SafeProbeError::UntrustedSymlinkRefused`]입니다. 경로
//!   사전 검사(lstat)는 일찍 거부하는 데만 쓰고 통과 판정에는 쓰지 않습니다.
//! - 다른 플랫폼은 flag 값이 플랫폼·아키텍처마다 달라 주지 않습니다. 비신뢰 probe는 lstat한 항목과
//!   연 handle의 identity(device + inode)가 다르면 symlink 거부로 처리합니다. FIFO로 바뀌면 열기가
//!   멈출 수 있는 한계가 남습니다(macOS 외 플랫폼은 ADR-0006으로 보류).

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestError};
use crate::root::{validate_project_root, GitLink, RootError, MANIFEST_FILE};
use crate::trust::TrustStatus;
use crate::yaml::loader::MAX_INPUT_BYTES;
use crate::yaml::{load_restricted, LoaderError, LoaderErrorKind};

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
    /// `stellar.yaml` 읽기 실패. 연 handle이 정규 파일이 아니면(FIFO·장치·디렉터리) `InvalidInput`
    /// 입니다.
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
///
/// `stellar.yaml` symlink는 따라갑니다(신뢰된 프로젝트용). 비신뢰 프로젝트는 [`safe_probe`]를
/// 씁니다.
pub fn probe_project(path: &Path) -> Result<ProjectProbe, ProbeError> {
    probe_with(path, ManifestLinks::Follow).map_err(|failure| match failure {
        ManifestFailure::Probe(error) => error,
        // Follow 정책은 symlink를 거부하지 않으므로 나오지 않습니다(나오더라도 읽기 실패로 보고).
        ManifestFailure::Symlink => ProbeError::ManifestRead {
            kind: io::ErrorKind::InvalidInput,
        },
    })
}

/// `stellar.yaml` symlink 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestLinks {
    /// 따라갑니다(신뢰된 프로젝트).
    Follow,
    /// 따르지 않고 거부합니다(비신뢰 프로젝트, IV.5.3).
    Refuse,
}

/// probe 내부 실패: probe 오류이거나 [`ManifestLinks::Refuse`] 정책의 symlink 거부.
#[derive(Debug)]
enum ManifestFailure {
    Probe(ProbeError),
    Symlink,
}

impl From<ProbeError> for ManifestFailure {
    fn from(error: ProbeError) -> ManifestFailure {
        ManifestFailure::Probe(error)
    }
}

/// root 검증 → 매니페스트 읽기(`links` 정책) → parse → 스키마 검증.
fn probe_with(path: &Path, links: ManifestLinks) -> Result<ProjectProbe, ManifestFailure> {
    // 1. root 검증(stellar.yaml + .git 동일 경로). 경로 기준 사전 검사이며, 매니페스트 판정은
    //    2단계의 연 handle로 다시 합니다.
    let git_link = validate_project_root(path).map_err(ProbeError::Root)?;

    // 2. stellar.yaml 읽기(연 handle 기준 정규 파일 확인, loader 상한까지만).
    let text = read_manifest_bounded(&path.join(MANIFEST_FILE), links)?;

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

/// `stellar.yaml`을 loader 상한([`MAX_INPUT_BYTES`])까지만 읽어 UTF-8 문자열로 돌려줍니다.
///
/// 판정은 연 handle로 합니다(모듈 문서 "열린 handle 기준 판정"): `O_NONBLOCK`(macOS)으로 열어
/// FIFO·장치에서 멈추지 않고, handle이 정규 파일이 아니면 거부합니다. `links`가
/// [`ManifestLinks::Refuse`]면 macOS에서 `O_NOFOLLOW`로 열어 symlink를 따르지 않습니다.
///
/// 열린 handle의 크기가 상한을 넘으면 내용을 읽지 않고 `input-too-large`로 거부합니다(sparse
/// 파일처럼 실제 바이트가 없어도 크기만으로 거부). 크기 확인 뒤 파일이 자라도 [`read_at_most`]가
/// 상한 + 1 바이트에서 멈추므로 메모리·읽기 시간은 상한에 묶입니다.
fn read_manifest_bounded(path: &Path, links: ManifestLinks) -> Result<String, ManifestFailure> {
    let read_error =
        |e: io::Error| ManifestFailure::Probe(ProbeError::ManifestRead { kind: e.kind() });
    // O_NOFOLLOW가 없는 플랫폼: lstat한 항목과 연 handle이 같은 파일인지 비교합니다.
    let checked = if links == ManifestLinks::Refuse && !cfg!(target_os = "macos") {
        let meta = std::fs::symlink_metadata(path).map_err(read_error)?;
        if meta.file_type().is_symlink() {
            return Err(ManifestFailure::Symlink);
        }
        Some(meta)
    } else {
        None
    };
    let file = match open_manifest(path, links) {
        Ok(file) => file,
        Err(e) if links == ManifestLinks::Refuse && is_symlink_refusal(&e) => {
            return Err(ManifestFailure::Symlink)
        }
        Err(e) => return Err(read_error(e)),
    };
    let opened = file.metadata().map_err(read_error)?;
    if let Some(checked) = checked {
        if !same_file(&checked, &opened) {
            // 검사 뒤 바뀜: symlink를 따라 연 것일 수 있으므로 거부합니다.
            return Err(ManifestFailure::Symlink);
        }
    }
    if !opened.is_file() {
        return Err(ManifestFailure::Probe(ProbeError::ManifestRead {
            kind: io::ErrorKind::InvalidInput,
        }));
    }
    let declared = opened.len();
    let limit = u64::try_from(MAX_INPUT_BYTES).unwrap_or(u64::MAX);
    if declared > limit {
        return Err(input_too_large(Some(declared)).into());
    }
    let bytes = read_at_most(file, MAX_INPUT_BYTES)
        .map_err(read_error)?
        .ok_or_else(|| input_too_large(None))?;
    String::from_utf8(bytes).map_err(|_| {
        ManifestFailure::Probe(ProbeError::ManifestRead {
            kind: io::ErrorKind::InvalidData,
        })
    })
}

/// macOS `<sys/fcntl.h>`·`<sys/errno.h>` 값(SDK header로 확인).
#[cfg(target_os = "macos")]
mod open_flags {
    pub const O_NONBLOCK: i32 = 0x0000_0004;
    pub const O_NOFOLLOW: i32 = 0x0000_0100;
    pub const ELOOP: i32 = 62;
}

/// `stellar.yaml`을 읽기 전용으로 엽니다(macOS: `O_NONBLOCK`, `Refuse`면 `O_NOFOLLOW`도).
fn open_manifest(path: &Path, links: ManifestLinks) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = match links {
            ManifestLinks::Follow => 0,
            ManifestLinks::Refuse => open_flags::O_NOFOLLOW,
        };
        options.custom_flags(open_flags::O_NONBLOCK | nofollow);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = links;
    options.open(path)
}

/// [`open_manifest`]가 마지막 구성 요소 symlink 때문에 거부한 오류인지(macOS `ELOOP`).
fn is_symlink_refusal(e: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        e.raw_os_error() == Some(open_flags::ELOOP)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = e;
        false
    }
}

/// 검사한 metadata와 연 handle의 metadata가 같은 파일인지(unix: device + inode).
#[cfg(unix)]
fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    checked.dev() == opened.dev() && checked.ino() == opened.ino()
}

/// 비-unix(Windows, ADR-0006 보류)는 파일 식별 비교를 하지 않습니다.
#[cfg(not(unix))]
fn same_file(_checked: &std::fs::Metadata, _opened: &std::fs::Metadata) -> bool {
    true
}

/// `reader`에서 최대 `limit + 1` 바이트만 읽습니다. `limit`을 넘으면 `Ok(None)`입니다.
///
/// 끝없는 입력(장치 파일, 계속 자라는 파일)에서도 읽기량과 할당량이 `limit + 1`에 묶입니다.
pub(crate) fn read_at_most(reader: impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader.take(cap).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= limit).then_some(bytes))
}

/// loader와 같은 `input-too-large` 오류를 만듭니다. 크기를 알면 메시지에 넣습니다.
fn input_too_large(declared: Option<u64>) -> ProbeError {
    let message = match declared {
        Some(bytes) => format!("입력이 {bytes} bytes로 상한 {MAX_INPUT_BYTES} bytes를 넘습니다"),
        None => format!("입력이 상한 {MAX_INPUT_BYTES} bytes를 넘습니다"),
    };
    ProbeError::ManifestSyntax(LoaderError {
        kind: LoaderErrorKind::InputTooLarge,
        line: 0,
        col: 0,
        message,
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
///   방지). 검사 뒤 symlink로 바뀌어도 연 handle 기준으로 거부합니다(모듈 문서 "열린 handle 기준
///   판정").
/// - **필터 미실행**: clean/smudge 등 어떤 필터도 실행하지 않습니다(git 미실행이므로 구조적으로
///   불가능).
pub fn safe_probe(path: &Path, trust: TrustStatus) -> Result<ProjectProbe, SafeProbeError> {
    let manifest_path = path.join(MANIFEST_FILE);
    let links = if trust == TrustStatus::NotTrusted {
        // 이미 symlink인 매니페스트는 root 검증 전에 거부합니다(사전 검사는 거부 쪽으로만 씀).
        if is_symlink(&manifest_path) {
            return Err(SafeProbeError::UntrustedSymlinkRefused {
                path: manifest_path,
            });
        }
        ManifestLinks::Refuse
    } else {
        ManifestLinks::Follow
    };
    // 메타데이터 전용 probe(git 실행·필터 없음).
    probe_with(path, links).map_err(|failure| match failure {
        ManifestFailure::Probe(error) => SafeProbeError::Probe(error),
        ManifestFailure::Symlink => SafeProbeError::UntrustedSymlinkRefused {
            path: manifest_path,
        },
    })
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

    /// 상한의 `multiple`배 크기 sparse 매니페스트를 만들고 마지막 바이트를 UTF-8이 아닌 값으로
    /// 둡니다. 파일 전체를 읽으면 UTF-8 오류(`ManifestRead`)가 나므로 읽은 범위를 구분할 수
    /// 있습니다.
    fn write_sparse_manifest(dir: &Path, multiple: u64) {
        use std::io::{Seek, SeekFrom, Write};
        let size = u64::try_from(MAX_INPUT_BYTES).unwrap() * multiple;
        let mut file = std::fs::File::create(dir.join(MANIFEST_FILE)).unwrap();
        file.set_len(size).unwrap();
        file.seek(SeekFrom::Start(size - 1)).unwrap();
        file.write_all(&[0xFF]).unwrap();
    }

    #[test]
    fn oversized_manifest_is_rejected_before_reading_its_body() {
        let dir = TempDir::new();
        write_sparse_manifest(&dir.path, 64);
        make_git_dir(&dir.path);
        match probe_project(&dir.path) {
            Err(ProbeError::ManifestSyntax(error)) => {
                assert_eq!(error.kind, LoaderErrorKind::InputTooLarge, "{error}");
            }
            other => panic!("상한 초과 매니페스트가 input-too-large가 아님: {other:?}"),
        }
        // 비신뢰 probe도 같은 상한을 씁니다.
        assert!(matches!(
            safe_probe(&dir.path, TrustStatus::NotTrusted),
            Err(SafeProbeError::Probe(ProbeError::ManifestSyntax(
                LoaderError {
                    kind: LoaderErrorKind::InputTooLarge,
                    ..
                }
            )))
        ));
    }

    #[test]
    fn bounded_read_stops_on_an_endless_source() {
        // 크기 정보가 없는 끝없는 입력도 상한 + 1 바이트에서 멈춥니다.
        assert_eq!(
            read_at_most(io::repeat(b'a'), MAX_INPUT_BYTES).unwrap(),
            None
        );
        let exact = read_at_most(io::repeat(b'a').take(16), 16).unwrap();
        assert_eq!(exact.as_deref(), Some(&[b'a'; 16][..]));
        assert_eq!(read_at_most(io::repeat(b'a').take(17), 16).unwrap(), None);
    }

    #[test]
    fn manifest_at_exact_limit_is_not_rejected_as_too_large() {
        // 경계값: 정확히 상한 크기는 크기 검사를 통과하고 loader 단계로 넘어갑니다.
        let dir = TempDir::new();
        write_manifest(&dir.path, &"#".repeat(MAX_INPUT_BYTES));
        make_git_dir(&dir.path);
        match probe_project(&dir.path) {
            Err(ProbeError::ManifestSyntax(error)) => {
                assert_ne!(error.kind, LoaderErrorKind::InputTooLarge, "{error}");
            }
            Err(ProbeError::ManifestSchema(_)) => {}
            other => panic!("예상하지 않은 결과: {other:?}"),
        }
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

    /// `name`을 project.name으로 가진 매니페스트 텍스트.
    fn manifest_named(name: &str) -> String {
        let mut manifest = sample_manifest();
        manifest.project.name = name.to_string();
        write_canonical(&manifest.to_yaml())
    }

    /// `stellar.yaml`을 staging 항목으로 계속 바꿔 넣는 thread.
    ///
    /// `stage(i)`가 i번째 교체에 쓸 staging 경로를 만들면 thread가 rename으로 `stellar.yaml` 자리에
    /// 넣습니다. 교체 횟수는 `swaps`에 셉니다.
    #[cfg(unix)]
    struct Swapper {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        swaps: std::sync::Arc<std::sync::atomic::AtomicU64>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    #[cfg(unix)]
    impl Swapper {
        fn spawn(dir: &Path, stage: impl Fn(u64) -> PathBuf + Send + 'static) -> Swapper {
            use std::sync::atomic::{AtomicBool, AtomicU64};
            use std::sync::Arc;
            let stop = Arc::new(AtomicBool::new(false));
            let swaps = Arc::new(AtomicU64::new(0));
            let manifest = dir.join(MANIFEST_FILE);
            let (thread_stop, thread_swaps) = (Arc::clone(&stop), Arc::clone(&swaps));
            let thread = std::thread::spawn(move || {
                let mut i = 0u64;
                while !thread_stop.load(Ordering::Relaxed) {
                    let staged = stage(i);
                    std::fs::rename(&staged, &manifest).unwrap();
                    i += 1;
                    thread_swaps.store(i, Ordering::Relaxed);
                }
            });
            Swapper {
                stop,
                swaps,
                thread: Some(thread),
            }
        }

        /// 교체가 `min_swaps`번 이상 일어났거나 10초가 지났으면 참입니다.
        fn enough(&self, started: std::time::Instant, min_swaps: u64) -> bool {
            self.swaps.load(Ordering::Relaxed) >= min_swaps
                || started.elapsed() > std::time::Duration::from_secs(10)
        }

        /// 교체를 멈추고 교체 횟수를 돌려줍니다.
        fn finish(mut self) -> u64 {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
            self.swaps.load(Ordering::Relaxed)
        }
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_probe_never_reads_a_manifest_swapped_to_an_external_symlink() {
        // 검사와 열기 사이에 `stellar.yaml`이 외부 파일 symlink로 바뀌어도 비신뢰 probe는 외부
        // 파일을 매니페스트로 읽지 않습니다(경로 기준 TOCTOU, 보안 리뷰 FR-2).
        let dir = TempDir::new();
        let outside = TempDir::new();
        make_git_dir(&dir.path);
        let secret = outside.path.join("outside-secret.yaml");
        std::fs::write(&secret, manifest_named("OUTSIDE")).unwrap();
        write_manifest(&dir.path, &manifest_named("INSIDE"));

        let staging = dir.path.clone();
        let swapper = Swapper::spawn(&dir.path, move |i| {
            let staged = staging.join(format!("staged-{i}"));
            if i.is_multiple_of(2) {
                std::os::unix::fs::symlink(&secret, &staged).unwrap();
            } else {
                std::fs::write(&staged, manifest_named("INSIDE")).unwrap();
            }
            staged
        });
        let (mut inside, mut refused, mut other) = (0u32, 0u32, 0u32);
        let mut leaked = Vec::new();
        let started = std::time::Instant::now();
        while !swapper.enough(started, 4000) {
            match safe_probe(&dir.path, TrustStatus::NotTrusted) {
                Ok(probe) if probe.manifest.project.name == "INSIDE" => inside += 1,
                Ok(probe) => leaked.push(probe.manifest.project.name),
                Err(SafeProbeError::UntrustedSymlinkRefused { .. }) => refused += 1,
                Err(_) => other += 1,
            }
        }
        let swaps = swapper.finish();
        assert!(
            leaked.is_empty(),
            "외부 파일을 매니페스트로 {}번 읽음: {:?} (inside={inside} refused={refused} \
             other={other} swaps={swaps})",
            leaked.len(),
            leaked.first()
        );
        assert!(inside > 0 && refused > 0, "교체 경합이 일어나지 않음");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn opened_handle_decides_fifo_and_symlink_manifests() {
        // 경로 사전 검사를 거치지 않고 읽기 함수만 불러도(검사 뒤 교체와 같은 상태) 연 handle로
        // 판정합니다. FIFO는 writer가 없어도 바로 InvalidInput으로 거부합니다.
        let dir = TempDir::new();
        let fifo = dir.path.join("manifest.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo 실패");
        for links in [ManifestLinks::Follow, ManifestLinks::Refuse] {
            assert!(matches!(
                read_manifest_bounded(&fifo, links),
                Err(ManifestFailure::Probe(ProbeError::ManifestRead {
                    kind: io::ErrorKind::InvalidInput
                }))
            ));
        }
        // symlink: Refuse는 따르지 않고, Follow는 대상을 읽습니다.
        let target = dir.path.join("target.yaml");
        std::fs::write(&target, "a: 1\n").unwrap();
        let link = dir.path.join("link.yaml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(
            read_manifest_bounded(&link, ManifestLinks::Refuse),
            Err(ManifestFailure::Symlink)
        ));
        assert_eq!(
            read_manifest_bounded(&link, ManifestLinks::Follow).unwrap(),
            "a: 1\n"
        );
        // FIFO를 가리키는 symlink도 Follow에서 멈추지 않고 거부합니다.
        let fifo_link = dir.path.join("fifo-link.yaml");
        std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
        assert!(matches!(
            read_manifest_bounded(&fifo_link, ManifestLinks::Follow),
            Err(ManifestFailure::Probe(ProbeError::ManifestRead {
                kind: io::ErrorKind::InvalidInput
            }))
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn probe_does_not_block_on_a_manifest_swapped_to_a_fifo() {
        // 검사 뒤 `stellar.yaml`이 FIFO로 바뀌어도 열기가 writer를 기다리며 멈추지 않습니다
        // (보안 리뷰 FR-1). FIFO는 hard link로 보관해 교체마다 같은 FIFO를 다시 넣습니다.
        use std::sync::atomic::AtomicBool;
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};

        let dir = TempDir::new();
        make_git_dir(&dir.path);
        write_manifest(&dir.path, &manifest_named("INSIDE"));
        let fifo = dir.path.join("keep.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo 실패");

        let staging = dir.path.clone();
        let keep = fifo.clone();
        let swapper = Swapper::spawn(&dir.path, move |i| {
            let staged = staging.join(format!("staged-{i}"));
            if i.is_multiple_of(2) {
                std::fs::hard_link(&keep, &staged).unwrap();
            } else {
                std::fs::write(&staged, manifest_named("INSIDE")).unwrap();
            }
            staged
        });
        // probe는 별도 thread에서 돌리고, 결과마다 신호를 받아 멈춤을 감지합니다.
        let (done, results) = mpsc::channel();
        let root = dir.path.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let prober_finished = Arc::clone(&finished);
        let prober = std::thread::spawn(move || {
            let mut round = 0u64;
            while !prober_finished.load(Ordering::Relaxed) {
                // 비신뢰·신뢰 probe를 번갈아 씁니다(둘 다 같은 매니페스트 열기 경로).
                let trust = if round.is_multiple_of(2) {
                    TrustStatus::NotTrusted
                } else {
                    TrustStatus::Trusted
                };
                round += 1;
                let opened = safe_probe(&root, trust).is_ok();
                if done.send(opened).is_err() {
                    return;
                }
            }
        });
        let mut hung = false;
        let (mut opened, mut failed) = (0u32, 0u32);
        let started = Instant::now();
        while !swapper.enough(started, 4000) {
            match results.recv_timeout(Duration::from_secs(5)) {
                Ok(true) => opened += 1,
                Ok(false) => failed += 1,
                Err(_) => {
                    hung = true;
                    break;
                }
            }
        }
        finished.store(true, Ordering::Relaxed);
        let swaps = swapper.finish();
        drop(results);
        if hung {
            // 멈춘 open()을 풀어 줍니다: 같은 FIFO에 writer를 붙였다가 바로 닫아 reader가 EOF를
            // 받게 합니다(reader가 open에서 기다리는 중이므로 writer open은 바로 끝납니다).
            drop(std::fs::OpenOptions::new().write(true).open(&fifo));
            let _ = prober.join();
            panic!(
                "FIFO로 바뀐 매니페스트 열기가 멈춤(opened={opened} failed={failed} swaps={swaps})"
            );
        }
        prober.join().unwrap();
        assert!(opened > 0 && failed > 0, "교체 경합이 일어나지 않음");
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
