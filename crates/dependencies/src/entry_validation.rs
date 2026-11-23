//! library cache entry 검증 (XV.3.4).
//!
//! materialize된 cache entry(또는 local override root)는 사용 전에 두 단계를 통과해야 합니다.
//!
//! 1. **ProjectProbe**(IV.5.1): root 구조·`stellar.yaml` 파싱·스키마. 일반 Git 저장소(`stellar.yaml`
//!    없음)와 손상 매니페스트를 거부합니다. probe의 `project.id`가 선언한 [`LibraryId`]와 같아야
//!    합니다.
//! 2. **storage validator**(V.4.4): current pointer → generation manifest → chunk 순 검증. 텍스트 원본이 Git
//!    원본인 library(ADR-0088)는 원본 해석도 이 단계입니다(ADR-0119 결정 1).
//!
//! 두 검증기는 project·storage crate에 있으므로 [`LibraryProjectValidator`] trait으로 받습니다.
//! 비싼 storage 검증은 probe와 id 확인을 통과한 뒤에만 실행합니다. 검증을 통과한 결과는
//! [`ValidatedLibraryEntry`]로만 표현되며, 이 타입은 [`validate_library_entry`] 밖에서 만들 수
//! 없습니다(검증되지 않은 entry 사용 방지).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::declaration::LibraryId;

/// ProjectProbe 성공 요약.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedLibrary {
    /// `stellar.yaml`의 `project.id`.
    pub project_id: LibraryId,
    /// 매니페스트 schema version.
    pub schema_version: u32,
}

/// ProjectProbe 실패 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryProbeFailure {
    /// `stellar.yaml`이 없는 일반 디렉터리·Git 저장소.
    NotAStellarProject,
    /// `stellar.yaml` 문법·스키마 오류.
    CorruptManifest,
    /// 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

/// storage validator 실패 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageValidationFailure {
    /// current pointer 누락·손상.
    CurrentPointer,
    /// generation manifest 누락·손상.
    Manifest,
    /// chunk 누락·hash 불일치.
    Chunk,
    /// 저장소 내부 project id가 매니페스트와 다름.
    ProjectIdMismatch,
    /// 텍스트 원본(`design/**/*.stellar.yaml`, ADR-0088)을 해석할 수 없음.
    TextSource,
}

/// project·storage 검증기(project crate가 ProjectProbe·generation loader로 구현).
pub trait LibraryProjectValidator {
    /// ProjectProbe를 실행합니다(문서 청크는 읽지 않음).
    fn probe(&mut self, root: &Path) -> Result<ProbedLibrary, LibraryProbeFailure>;

    /// storage validator를 실행합니다.
    fn validate_storage(
        &mut self,
        root: &Path,
        project_id: LibraryId,
    ) -> Result<(), StorageValidationFailure>;
}

/// entry 거부 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryEntryRejection {
    /// 일반 Git 저장소 또는 Stellar 프로젝트가 아님.
    NotAStellarProject,
    /// 매니페스트 손상.
    CorruptManifest,
    /// 선언한 library id와 entry의 project id가 다름.
    LibraryIdMismatch {
        /// 선언한 id.
        expected: LibraryId,
        /// entry의 id.
        found: LibraryId,
    },
    /// 저장 데이터 손상.
    CorruptStorage(StorageValidationFailure),
    /// 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for LibraryEntryRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryEntryRejection::NotAStellarProject => {
                f.write_str("Stellar library 프로젝트가 아닙니다(일반 Git 저장소)")
            }
            LibraryEntryRejection::CorruptManifest => {
                f.write_str("library 매니페스트가 손상되었습니다")
            }
            LibraryEntryRejection::LibraryIdMismatch { expected, found } => {
                write!(f, "library id가 다릅니다: 선언 {expected}, 실제 {found}")
            }
            LibraryEntryRejection::CorruptStorage(stage) => {
                write!(f, "library 저장 데이터가 손상되었습니다: {stage:?}")
            }
            LibraryEntryRejection::Io { kind } => write!(f, "library 읽기 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for LibraryEntryRejection {}

/// 검증을 통과한 library entry. [`validate_library_entry`]만 만들 수 있습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedLibraryEntry {
    root: PathBuf,
    library_id: LibraryId,
    schema_version: u32,
}

impl ValidatedLibraryEntry {
    /// entry root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 검증된 library id.
    pub fn library_id(&self) -> LibraryId {
        self.library_id
    }

    /// library 매니페스트 schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

/// entry를 ProjectProbe → library id 확인 → storage validator 순으로 검증합니다.
pub fn validate_library_entry(
    root: &Path,
    expected: LibraryId,
    validator: &mut dyn LibraryProjectValidator,
) -> Result<ValidatedLibraryEntry, LibraryEntryRejection> {
    let probed = validator.probe(root).map_err(|failure| match failure {
        LibraryProbeFailure::NotAStellarProject => LibraryEntryRejection::NotAStellarProject,
        LibraryProbeFailure::CorruptManifest => LibraryEntryRejection::CorruptManifest,
        LibraryProbeFailure::Io { kind } => LibraryEntryRejection::Io { kind },
    })?;
    if probed.project_id != expected {
        return Err(LibraryEntryRejection::LibraryIdMismatch {
            expected,
            found: probed.project_id,
        });
    }
    validator
        .validate_storage(root, probed.project_id)
        .map_err(LibraryEntryRejection::CorruptStorage)?;
    Ok(ValidatedLibraryEntry {
        root: root.to_path_buf(),
        library_id: probed.project_id,
        schema_version: probed.schema_version,
    })
}

#[cfg(test)]
pub(crate) mod fake {
    //! 테스트용 검증기: root 경로별 probe·storage 결과 표.

    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    pub(crate) struct TableValidator {
        pub(crate) probes: BTreeMap<PathBuf, Result<ProbedLibrary, LibraryProbeFailure>>,
        pub(crate) storage: BTreeMap<PathBuf, StorageValidationFailure>,
        pub(crate) storage_calls: Vec<PathBuf>,
    }

    impl LibraryProjectValidator for TableValidator {
        fn probe(&mut self, root: &Path) -> Result<ProbedLibrary, LibraryProbeFailure> {
            self.probes
                .get(root)
                .copied()
                .unwrap_or(Err(LibraryProbeFailure::NotAStellarProject))
        }

        fn validate_storage(
            &mut self,
            root: &Path,
            _: LibraryId,
        ) -> Result<(), StorageValidationFailure> {
            self.storage_calls.push(root.to_path_buf());
            match self.storage.get(root) {
                Some(failure) => Err(*failure),
                None => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::TableValidator;
    use super::*;
    use crate::declaration::fixtures::library;

    fn fixture() -> TableValidator {
        let ok = ProbedLibrary {
            project_id: library(1),
            schema_version: 3,
        };
        let mut v = TableValidator::default();
        v.probes.insert("/cache/good".into(), Ok(ok));
        v.probes.insert(
            "/cache/plain-git".into(),
            Err(LibraryProbeFailure::NotAStellarProject),
        );
        v.probes.insert(
            "/cache/bad-yaml".into(),
            Err(LibraryProbeFailure::CorruptManifest),
        );
        v.probes.insert(
            "/cache/other-lib".into(),
            Ok(ProbedLibrary {
                project_id: library(2),
                schema_version: 3,
            }),
        );
        v.probes.insert("/cache/missing-chunk".into(), Ok(ok));
        v.storage.insert(
            "/cache/missing-chunk".into(),
            StorageValidationFailure::Chunk,
        );
        v
    }

    #[test]
    fn invalid_library_fixtures_are_rejected() {
        let mut v = fixture();
        let check = |v: &mut TableValidator, root: &str| {
            validate_library_entry(Path::new(root), library(1), v)
        };
        assert_eq!(
            check(&mut v, "/cache/plain-git"),
            Err(LibraryEntryRejection::NotAStellarProject)
        );
        assert_eq!(
            check(&mut v, "/cache/bad-yaml"),
            Err(LibraryEntryRejection::CorruptManifest)
        );
        assert_eq!(
            check(&mut v, "/cache/other-lib"),
            Err(LibraryEntryRejection::LibraryIdMismatch {
                expected: library(1),
                found: library(2)
            })
        );
        // probe·id 단계에서 거부되면 storage 검증을 실행하지 않음.
        assert!(v.storage_calls.is_empty());
        assert_eq!(
            check(&mut v, "/cache/missing-chunk"),
            Err(LibraryEntryRejection::CorruptStorage(
                StorageValidationFailure::Chunk
            ))
        );
        assert_eq!(v.storage_calls.len(), 1);
    }

    #[test]
    fn valid_entry_carries_schema_version() {
        let mut v = fixture();
        let entry = validate_library_entry(Path::new("/cache/good"), library(1), &mut v).unwrap();
        assert_eq!(entry.root(), Path::new("/cache/good"));
        assert_eq!(entry.library_id(), library(1));
        assert_eq!(entry.schema_version(), 3);
    }
}
