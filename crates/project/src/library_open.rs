//! 프로젝트를 열 때 lock 기준 library 로드와 부분 열기 (XV.5.5, ADR-0119 결정 6).
//!
//! [`open_project_libraries`]는 사용 프로젝트 root의 `stellar.lock.yaml`을 실제 lock parser([`Lockfile::from_yaml`],
//! XV.4.2 validator)로 읽고, 모든 node를 공용 cache에서 제품 검증기([`ProjectLibraryValidator::cache_entry`])와 제품
//! export 공급자([`SourceExports`])로 로드합니다([`load_locked_libraries`]). Git·network를 쓰지 않습니다.
//!
//! **이 함수는 실패하지 않습니다.** library 쪽 문제는 모두 진단으로 남기고 프로젝트 기본 데이터(문서·자기
//! component·token)는 그대로 열 수 있게 합니다.
//!
//! | 상황 | 결과 |
//! |---|---|
//! | lock 없음 | library 없음, 진단 없음(의존성이 없는 프로젝트 또는 아직 받지 않은 의존성) |
//! | lock 읽기·해석 실패, lock 구조 오류 | library 없음 + [`LibraryOpenProblem`] |
//! | cache에 entry 없음(offline·미 다운로드) | 그 node만 [`LibraryUnavailableDiagnostic`](`MissingCacheEntry`) |
//! | entry 손상·일반 Git 저장소·손상 library | 그 node만 진단(다른 commit을 고르지 않음) |
//!
//! 사용 프로젝트 `stellar.yaml`이 저장한 외부 component 참조(의존성 선언 `components`, ADR-0137 결정 1)도 함께 읽어
//! [`ProjectLibraries::instance`]에 붙입니다. lock이 없어도 저장된 참조는 남습니다(받지 않은 library의 참조). 매니페스트를
//! 읽지 못하면 [`LibraryOpenProblem::ManifestInvalid`]이고 저장된 참조는 없습니다.
//!
//! 문서 열기 자체(창 session·CLI·MCP)는 이 함수와 독립입니다. 열기 경로는 문서를 연 뒤 이 결과를 받아 외부
//! 참조를 해석하고, 사용할 수 없는 library의 참조는 [`ResourceResolution::Unavailable`]로 표시합니다.
//!
//! [`LibraryUnavailableDiagnostic`]: stellar_dependencies::library_load::LibraryUnavailableDiagnostic
//! [`ResourceResolution::Unavailable`]: stellar_dependencies::library_load::ResourceResolution::Unavailable

use std::fmt;
use std::io;
use std::path::Path;

use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::component_imports::ComponentImports;
use stellar_dependencies::declaration::DependencyDeclaration;
use stellar_dependencies::library_load::{load_locked_libraries, LoadedLibraries};
use stellar_dependencies::library_snapshot::SnapshotLimits;
use stellar_dependencies::lock::LockDocument;
use stellar_dependencies::mirror::RepositoryKeyer;
use stellar_dependencies::ref_resolution::LibraryGraphInstance;

use crate::library_entry::ProjectLibraryValidator;
use crate::library_exports::SourceExports;
use crate::library_git::StorageSha256;
use crate::lock::Lockfile;
use crate::lockfile_write::LOCK_FILE;
use crate::manifest::Manifest;
use crate::probe::read_at_most;
use crate::root::MANIFEST_FILE;
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// library를 하나도 로드하지 못한 이유(프로젝트는 계속 열림).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryOpenProblem {
    /// lock 파일을 읽지 못함.
    LockUnreadable {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// lock 파일 문법·validator 오류.
    LockInvalid,
    /// lock entry 경로 구조 오류(부모 없음·충돌).
    LockStructure,
    /// `stellar.yaml`을 읽지 못했거나 매니페스트로 해석되지 않음(저장된 외부 참조 없음으로 봄).
    ManifestInvalid,
}

impl fmt::Display for LibraryOpenProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryOpenProblem::LockUnreadable { kind } => {
                write!(f, "stellar.lock.yaml을 읽지 못했습니다: {kind:?}")
            }
            LibraryOpenProblem::LockInvalid => f.write_str("stellar.lock.yaml이 올바르지 않습니다"),
            LibraryOpenProblem::LockStructure => {
                f.write_str("stellar.lock.yaml의 의존성 경로 구조가 올바르지 않습니다")
            }
            LibraryOpenProblem::ManifestInvalid => {
                f.write_str("stellar.yaml을 읽지 못해 저장된 외부 component 참조를 알 수 없습니다")
            }
        }
    }
}

/// 프로젝트 library 열기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLibraries {
    loaded: Option<LoadedLibraries>,
    problem: Option<LibraryOpenProblem>,
    imports: ComponentImports,
}

impl ProjectLibraries {
    /// lock을 읽기 전에 정해진 결과(매니페스트 문제·lock 문제).
    pub fn without_lock(
        problem: Option<LibraryOpenProblem>,
        imports: ComponentImports,
    ) -> ProjectLibraries {
        ProjectLibraries {
            loaded: None,
            problem,
            imports,
        }
    }

    /// 사용 프로젝트 `stellar.yaml`이 저장한 외부 component 참조.
    pub fn imports(&self) -> &ComponentImports {
        &self.imports
    }

    /// lock 기준으로 로드한 library(lock이 없거나 읽지 못했으면 `None`).
    pub fn loaded(&self) -> Option<&LoadedLibraries> {
        self.loaded.as_ref()
    }

    /// 로드한 graph instance와 저장된 외부 component 참조. library가 없으면 snapshot 없는 instance입니다.
    pub fn instance(&self) -> LibraryGraphInstance {
        self.loaded
            .as_ref()
            .map(|l| l.instance().clone())
            .unwrap_or_default()
            .with_imports(self.imports.clone())
    }

    /// lock 자체의 문제.
    pub fn problem(&self) -> Option<&LibraryOpenProblem> {
        self.problem.as_ref()
    }

    /// 모든 library를 사용할 수 있는지(lock 문제·unavailable 진단 없음).
    pub fn is_complete(&self) -> bool {
        self.problem.is_none()
            && self
                .loaded
                .as_ref()
                .is_none_or(LoadedLibraries::is_complete)
    }
}

/// cache를 열지 않고 `stellar.yaml`이 저장한 외부 component 참조만 읽습니다(lock이 없거나 cache를 열지 못한 경우).
pub fn declared_only(project_root: &Path) -> ProjectLibraries {
    match read_declarations(project_root) {
        Ok(declarations) => {
            ProjectLibraries::without_lock(None, ComponentImports::from_declarations(&declarations))
        }
        Err(problem) => ProjectLibraries::without_lock(Some(problem), ComponentImports::default()),
    }
}

/// `project_root`의 lock을 읽어 cache에서 library를 로드합니다(실패하지 않음, 모듈 문서).
pub fn open_project_libraries<K: RepositoryKeyer>(
    project_root: &Path,
    store: &LibraryEntryStore<StorageSha256, K>,
    limits: &SnapshotLimits,
    now_unix_ms: u64,
) -> ProjectLibraries {
    let (imports, manifest_problem) = match read_declarations(project_root) {
        Ok(declarations) => (ComponentImports::from_declarations(&declarations), None),
        Err(problem) => (ComponentImports::default(), Some(problem)),
    };
    let lock = match read_lock(project_root) {
        Ok(Some(lock)) => lock,
        Ok(None) => return ProjectLibraries::without_lock(manifest_problem, imports),
        Err(problem) => return ProjectLibraries::without_lock(Some(problem), imports),
    };
    let mut validator = ProjectLibraryValidator::cache_entry();
    let mut exports = SourceExports;
    match load_locked_libraries(
        &lock,
        store,
        &mut validator,
        &mut exports,
        limits,
        now_unix_ms,
    ) {
        Ok(loaded) => ProjectLibraries {
            loaded: Some(loaded),
            problem: manifest_problem,
            imports,
        },
        Err(_) => ProjectLibraries::without_lock(Some(LibraryOpenProblem::LockStructure), imports),
    }
}

/// 사용 프로젝트 `stellar.yaml`의 의존성 선언을 loader 상한까지 읽어 실제 매니페스트 parser로 해석합니다. 파일이 없으면
/// 빈 목록입니다(선언 없음).
pub fn read_declarations(
    project_root: &Path,
) -> Result<Vec<DependencyDeclaration>, LibraryOpenProblem> {
    read_manifest(project_root).map(|m| m.map(|m| m.dependencies).unwrap_or_default())
}

/// 사용 프로젝트 `stellar.yaml`을 읽습니다. 없으면 `Ok(None)`입니다. symlink는 따르지 않습니다.
pub fn read_manifest(project_root: &Path) -> Result<Option<Manifest>, LibraryOpenProblem> {
    let path = project_root.join(MANIFEST_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(LibraryOpenProblem::ManifestInvalid)
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LibraryOpenProblem::ManifestInvalid),
    }
    let file = std::fs::File::open(&path).map_err(|_| LibraryOpenProblem::ManifestInvalid)?;
    let bytes = read_at_most(file, MAX_INPUT_BYTES)
        .map_err(|_| LibraryOpenProblem::ManifestInvalid)?
        .ok_or(LibraryOpenProblem::ManifestInvalid)?;
    let text = String::from_utf8(bytes).map_err(|_| LibraryOpenProblem::ManifestInvalid)?;
    let value = load_restricted(&text).map_err(|_| LibraryOpenProblem::ManifestInvalid)?;
    Manifest::from_yaml(&value)
        .map(Some)
        .map_err(|_| LibraryOpenProblem::ManifestInvalid)
}

/// lock 파일을 loader 상한까지 읽어 검증합니다. 없으면 `Ok(None)`입니다.
pub fn read_lock(project_root: &Path) -> Result<Option<LockDocument>, LibraryOpenProblem> {
    let file = match std::fs::File::open(project_root.join(LOCK_FILE)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LibraryOpenProblem::LockUnreadable { kind: e.kind() }),
    };
    let bytes = read_at_most(file, MAX_INPUT_BYTES)
        .map_err(|e| LibraryOpenProblem::LockUnreadable { kind: e.kind() })?
        .ok_or(LibraryOpenProblem::LockInvalid)?;
    let text = String::from_utf8(bytes).map_err(|_| LibraryOpenProblem::LockInvalid)?;
    let value = load_restricted(&text).map_err(|_| LibraryOpenProblem::LockInvalid)?;
    let lock = Lockfile::from_yaml(&value).map_err(|_| LibraryOpenProblem::LockInvalid)?;
    lock.to_document()
        .map(Some)
        .map_err(|_| LibraryOpenProblem::LockInvalid)
}
