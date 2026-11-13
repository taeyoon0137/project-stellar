//! External Font — 관리형 패키지의 고정 hash/face 참조 (XIII.6.4).
//!
//! External Font는 버전 고정 외부 패키지가 제공하는 폰트입니다(아키텍처 11.3). 문서는 패키지
//! 이름·정확한 버전과 face 참조(파일 hash + face index)를 기록합니다([`ExternalFontRef`]).
//!
//! **인터페이스만 제공합니다.** 패키지를 찾고 검증하는 일은 [`ExternalFontPackageProvider`]
//! 구현(관리형 패키지 cache)이 맡고, 이 모듈은 이미 설치된 패키지 내용만 조회합니다. Stellar는
//! 패키지 다운로드 서비스를 제공하지 않으며, provider는 네트워크에 접근하거나 패키지를 내려받지
//! 않아야 합니다. 설치되지 않은 패키지는 [`ExternalFontDiagnosticKind::PackageMissing`] 진단으로
//! 보고하고 다른 폰트로 대체하지 않습니다(X.8.3).
//!
//! - 패키지 이름: 1..=[`MAX_PACKAGE_NAME_BYTES`] bytes, `/`로 나눈 segment마다 `[a-z0-9]`로 시작하고
//!   `[a-z0-9._-]`만 씁니다(`.`·`..` segment 금지).
//! - 버전: 1..=[`MAX_PACKAGE_VERSION_BYTES`] bytes의 **정확한** 값입니다. 영숫자로 시작하고
//!   `[0-9A-Za-z.+-]`만 써서 범위 연산자(`^`, `~`, `>=`, `*`, 공백)를 표현할 수 없습니다.
//! - face 정체성은 hash+index입니다. 같은 패키지 버전이라도 설치된 파일 hash가 기록과 다르면 그
//!   face는 없는 것으로 봅니다([`ExternalFontDiagnosticKind::FaceMissing`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_text::font::{FontFaceRef, FontFileHash, FontSource};
use stellar_text::registry::{FontFaceKey, FontHandle, FontRegistry, FontRegistryError};

use super::sfnt::MAX_FACES_PER_FILE;

/// 패키지 이름 최대 길이(byte).
pub const MAX_PACKAGE_NAME_BYTES: usize = 128;

/// 패키지 버전 최대 길이(byte).
pub const MAX_PACKAGE_VERSION_BYTES: usize = 64;

/// External Font 참조 생성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalFontRefError {
    /// 패키지 이름 규칙 위반.
    InvalidPackageName {
        /// 위반 규칙.
        reason: &'static str,
    },
    /// 패키지 버전 규칙 위반(정확한 버전이 아님).
    InvalidPackageVersion {
        /// 위반 규칙.
        reason: &'static str,
    },
    /// face 참조의 출처가 External이 아닙니다.
    NotExternalSource {
        /// 실제 출처.
        source: FontSource,
    },
}

impl fmt::Display for ExternalFontRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalFontRefError::InvalidPackageName { reason } => {
                write!(f, "외부 폰트 패키지 이름이 잘못되었습니다: {reason}")
            }
            ExternalFontRefError::InvalidPackageVersion { reason } => {
                write!(f, "외부 폰트 패키지 버전이 정확한 값이 아닙니다: {reason}")
            }
            ExternalFontRefError::NotExternalSource { source } => {
                write!(f, "External Font 참조의 출처가 {source}입니다")
            }
        }
    }
}

impl std::error::Error for ExternalFontRefError {}

/// 관리형 폰트 패키지 이름.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalFontPackageName(String);

impl ExternalFontPackageName {
    /// 이름을 검사해 만듭니다.
    pub fn new(name: &str) -> Result<ExternalFontPackageName, ExternalFontRefError> {
        let invalid = |reason| ExternalFontRefError::InvalidPackageName { reason };
        if name.is_empty() {
            return Err(invalid("비었습니다"));
        }
        if name.len() > MAX_PACKAGE_NAME_BYTES {
            return Err(invalid("너무 깁니다"));
        }
        for segment in name.split('/') {
            let Some(first) = segment.bytes().next() else {
                return Err(invalid("빈 segment가 있습니다"));
            };
            if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
                return Err(invalid("segment가 소문자·숫자로 시작하지 않습니다"));
            }
            if !segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
            {
                return Err(invalid("허용하지 않는 문자가 있습니다"));
            }
        }
        Ok(ExternalFontPackageName(name.to_string()))
    }

    /// 이름 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExternalFontPackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 관리형 폰트 패키지의 정확한 버전.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalFontPackageVersion(String);

impl ExternalFontPackageVersion {
    /// 버전을 검사해 만듭니다. 범위 표현은 거부합니다.
    pub fn new(version: &str) -> Result<ExternalFontPackageVersion, ExternalFontRefError> {
        let invalid = |reason| ExternalFontRefError::InvalidPackageVersion { reason };
        let Some(first) = version.bytes().next() else {
            return Err(invalid("비었습니다"));
        };
        if version.len() > MAX_PACKAGE_VERSION_BYTES {
            return Err(invalid("너무 깁니다"));
        }
        if !first.is_ascii_alphanumeric() {
            return Err(invalid("영숫자로 시작하지 않습니다"));
        }
        if !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
        {
            return Err(invalid("범위 연산자 또는 허용하지 않는 문자가 있습니다"));
        }
        Ok(ExternalFontPackageVersion(version.to_string()))
    }

    /// 버전 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExternalFontPackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 버전 고정 폰트 패키지(이름 + 정확한 버전).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalFontPackage {
    /// 패키지 이름.
    pub name: ExternalFontPackageName,
    /// 정확한 버전.
    pub version: ExternalFontPackageVersion,
}

impl ExternalFontPackage {
    /// 이름·버전 문자열을 검사해 만듭니다.
    pub fn parse(name: &str, version: &str) -> Result<ExternalFontPackage, ExternalFontRefError> {
        Ok(ExternalFontPackage {
            name: ExternalFontPackageName::new(name)?,
            version: ExternalFontPackageVersion::new(version)?,
        })
    }
}

impl fmt::Display for ExternalFontPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// 문서가 기록하는 External Font 참조: 패키지 + 고정 face(hash + index + variation).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalFontRef {
    package: ExternalFontPackage,
    face: FontFaceRef,
}

impl ExternalFontRef {
    /// 참조를 만듭니다. face 출처는 `FontSource::External`이어야 합니다.
    pub fn new(
        package: ExternalFontPackage,
        face: FontFaceRef,
    ) -> Result<ExternalFontRef, ExternalFontRefError> {
        if face.source() != FontSource::External {
            return Err(ExternalFontRefError::NotExternalSource {
                source: face.source(),
            });
        }
        Ok(ExternalFontRef { package, face })
    }

    /// 패키지.
    pub fn package(&self) -> &ExternalFontPackage {
        &self.package
    }

    /// face 참조.
    pub fn face(&self) -> &FontFaceRef {
        &self.face
    }

    /// face 정체성(hash+index).
    pub fn key(&self) -> FontFaceKey {
        FontFaceKey::from(&self.face)
    }
}

/// 설치된 패키지 내용 등록 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledPackageError {
    /// face 수가 1..=[`MAX_FACES_PER_FILE`] 밖입니다.
    InvalidFaceCount {
        /// face 수.
        count: u32,
    },
    /// 같은 hash 파일이 다른 face 수로 두 번 등록되었습니다.
    ConflictingFaceCount {
        /// 파일 hash.
        file_hash: FontFileHash,
    },
}

impl fmt::Display for InstalledPackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstalledPackageError::InvalidFaceCount { count } => {
                write!(f, "설치된 폰트 파일의 face 수가 잘못되었습니다: {count}")
            }
            InstalledPackageError::ConflictingFaceCount { file_hash } => {
                write!(
                    f,
                    "같은 폰트 파일이 다른 face 수로 등록되었습니다: {file_hash}"
                )
            }
        }
    }
}

impl std::error::Error for InstalledPackageError {}

/// provider가 알려 준 설치 패키지 내용: 폰트 파일 hash → face 수.
///
/// provider가 설치 시 파일 hash를 검증했다고 가정합니다(이 모듈은 파일을 읽지 않습니다).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledFontPackage {
    files: BTreeMap<FontFileHash, u32>,
}

impl InstalledFontPackage {
    /// 빈 패키지.
    pub fn new() -> InstalledFontPackage {
        InstalledFontPackage::default()
    }

    /// 폰트 파일 하나를 추가합니다.
    pub fn add_file(
        &mut self,
        file_hash: FontFileHash,
        face_count: u32,
    ) -> Result<(), InstalledPackageError> {
        if face_count == 0 || face_count > MAX_FACES_PER_FILE {
            return Err(InstalledPackageError::InvalidFaceCount { count: face_count });
        }
        match self.files.get(&file_hash) {
            Some(existing) if *existing != face_count => {
                Err(InstalledPackageError::ConflictingFaceCount { file_hash })
            }
            _ => {
                self.files.insert(file_hash, face_count);
                Ok(())
            }
        }
    }

    /// 패키지가 이 face(hash+index)를 포함하는지.
    pub fn contains(&self, key: FontFaceKey) -> bool {
        self.files
            .get(&key.file_hash())
            .is_some_and(|count| key.face_index() < *count)
    }

    /// (파일 hash, face 수) 목록(hash 오름차순).
    pub fn files(&self) -> impl Iterator<Item = (FontFileHash, u32)> + '_ {
        self.files.iter().map(|(hash, count)| (*hash, *count))
    }
}

/// 관리형 폰트 패키지 조회 인터페이스(다운로드 없음).
pub trait ExternalFontPackageProvider {
    /// 로컬에 **이미 설치된** 패키지의 내용. 없으면 `None`.
    ///
    /// 구현은 네트워크에 접근하거나 패키지를 내려받지 않아야 합니다.
    fn installed_package(&self, package: &ExternalFontPackage) -> Option<InstalledFontPackage>;
}

/// External Font 진단 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExternalFontDiagnosticKind {
    /// 패키지(이름·버전)가 설치되어 있지 않습니다.
    PackageMissing,
    /// 패키지는 있지만 기록한 hash/face가 없습니다.
    FaceMissing,
}

impl ExternalFontDiagnosticKind {
    /// 안정 진단 코드.
    pub const fn code(self) -> &'static str {
        match self {
            ExternalFontDiagnosticKind::PackageMissing => "missing-external-font-package",
            ExternalFontDiagnosticKind::FaceMissing => "missing-external-font-face",
        }
    }
}

/// External Font 진단(패키지 하나 단위).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFontDiagnostic {
    kind: ExternalFontDiagnosticKind,
    package: ExternalFontPackage,
    faces: Vec<FontFaceRef>,
}

impl ExternalFontDiagnostic {
    /// 진단 종류.
    pub fn kind(&self) -> ExternalFontDiagnosticKind {
        self.kind
    }

    /// 안정 진단 코드.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// 패키지.
    pub fn package(&self) -> &ExternalFontPackage {
        &self.package
    }

    /// 영향받는 face 참조(정렬, 중복 없음).
    pub fn faces(&self) -> &[FontFaceRef] {
        &self.faces
    }
}

impl fmt::Display for ExternalFontDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ExternalFontDiagnosticKind::PackageMissing => write!(
                f,
                "외부 폰트 패키지 {}가 설치되어 있지 않습니다(face {}개). Stellar는 패키지를 내려받지 않습니다",
                self.package,
                self.faces.len()
            ),
            ExternalFontDiagnosticKind::FaceMissing => write!(
                f,
                "외부 폰트 패키지 {}에 기록한 face {}개가 없습니다",
                self.package,
                self.faces.len()
            ),
        }
    }
}

/// External Font 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFontResolution {
    handles: Vec<Option<FontHandle>>,
    diagnostics: Vec<ExternalFontDiagnostic>,
}

impl ExternalFontResolution {
    /// 입력 참조 순서의 handle(없으면 `None`).
    pub fn handles(&self) -> &[Option<FontHandle>] {
        &self.handles
    }

    /// 진단(패키지 오름차순, 같은 패키지는 PackageMissing·FaceMissing 중 하나).
    pub fn diagnostics(&self) -> &[ExternalFontDiagnostic] {
        &self.diagnostics
    }

    /// 모든 참조가 해석되었으면 `true`.
    pub fn is_fully_resolved(&self) -> bool {
        self.handles.iter().all(Option::is_some)
    }
}

/// External Font 참조를 설치 패키지에서 찾아 registry에 `FontSource::External`로 등록합니다.
///
/// provider는 패키지마다 한 번만 조회합니다. 없는 패키지·face는 진단으로 보고하고 대체하지
/// 않습니다. registry 상한 오류는 그대로 반환합니다.
pub fn resolve_external_fonts<P, R>(
    refs: &[ExternalFontRef],
    provider: &P,
    registry: &mut R,
) -> Result<ExternalFontResolution, FontRegistryError>
where
    P: ExternalFontPackageProvider + ?Sized,
    R: FontRegistry + ?Sized,
{
    let mut installed: BTreeMap<&ExternalFontPackage, Option<InstalledFontPackage>> =
        BTreeMap::new();
    let mut missing: BTreeMap<
        (&ExternalFontPackage, ExternalFontDiagnosticKind),
        BTreeSet<&FontFaceRef>,
    > = BTreeMap::new();
    let mut handles = Vec::with_capacity(refs.len());
    for reference in refs {
        let contents = installed
            .entry(&reference.package)
            .or_insert_with(|| provider.installed_package(&reference.package));
        let handle = match contents {
            None => {
                missing
                    .entry((
                        &reference.package,
                        ExternalFontDiagnosticKind::PackageMissing,
                    ))
                    .or_default()
                    .insert(&reference.face);
                None
            }
            Some(contents) if !contents.contains(reference.key()) => {
                missing
                    .entry((&reference.package, ExternalFontDiagnosticKind::FaceMissing))
                    .or_default()
                    .insert(&reference.face);
                None
            }
            Some(_) => Some(registry.register(FontSource::External, reference.key())?),
        };
        handles.push(handle);
    }
    let diagnostics = missing
        .into_iter()
        .map(|((package, kind), faces)| ExternalFontDiagnostic {
            kind,
            package: package.clone(),
            faces: faces.into_iter().cloned().collect(),
        })
        .collect();
    Ok(ExternalFontResolution {
        handles,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use stellar_core_types::value::FiniteF32;
    use stellar_text::font::{FontVariation, OpenTypeTag};
    use stellar_text::registry::InMemoryFontRegistry;

    fn hash(fill: u8) -> FontFileHash {
        FontFileHash::from_sha256_digest([fill; 32])
    }

    fn package(name: &str, version: &str) -> ExternalFontPackage {
        ExternalFontPackage::parse(name, version).unwrap()
    }

    fn external(pkg: &ExternalFontPackage, fill: u8, index: u32) -> ExternalFontRef {
        ExternalFontRef::new(
            pkg.clone(),
            FontFaceRef::new(FontSource::External, hash(fill), index, Vec::new()),
        )
        .unwrap()
    }

    /// 설치 목록 provider. 조회 횟수를 셉니다.
    struct Installed {
        packages: BTreeMap<ExternalFontPackage, InstalledFontPackage>,
        queries: RefCell<Vec<ExternalFontPackage>>,
    }

    impl ExternalFontPackageProvider for Installed {
        fn installed_package(&self, package: &ExternalFontPackage) -> Option<InstalledFontPackage> {
            self.queries.borrow_mut().push(package.clone());
            self.packages.get(package).cloned()
        }
    }

    fn provider() -> Installed {
        let mut sans = InstalledFontPackage::new();
        sans.add_file(hash(1), 2).unwrap();
        sans.add_file(hash(2), 1).unwrap();
        Installed {
            packages: BTreeMap::from([(package("fonts/fixture-sans", "2.1.0"), sans)]),
            queries: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn missing_external_package_is_diagnosed_without_substitution() {
        let sans = package("fonts/fixture-sans", "2.1.0");
        let serif = package("fonts/fixture-serif", "1.0.0");
        let refs = vec![
            external(&sans, 1, 0),
            external(&serif, 7, 0),
            external(&sans, 1, 1),
            external(&serif, 7, 1),
            external(&serif, 7, 0),
        ];
        let provider = provider();
        let mut registry = InMemoryFontRegistry::new();
        let resolution = resolve_external_fonts(&refs, &provider, &mut registry).unwrap();

        // 설치된 패키지의 face만 External 출처로 등록됩니다.
        assert_eq!(registry.len(), 2);
        let handles = resolution.handles();
        assert!(handles[0].is_some() && handles[2].is_some());
        assert_eq!(handles[1], None);
        assert_eq!(handles[3], None);
        assert_eq!(handles[4], None);
        assert!(!resolution.is_fully_resolved());
        for handle in handles.iter().flatten() {
            assert!(registry
                .face(*handle)
                .unwrap()
                .has_source(FontSource::External));
        }

        // 없는 패키지는 진단 하나로 묶고 영향 face를 중복 없이 나열합니다.
        let diagnostics = resolution.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        let missing = &diagnostics[0];
        assert_eq!(missing.kind(), ExternalFontDiagnosticKind::PackageMissing);
        assert_eq!(missing.code(), "missing-external-font-package");
        assert_eq!(missing.package(), &serif);
        assert_eq!(
            missing.faces(),
            &[refs[1].face().clone(), refs[3].face().clone()]
        );
        let message = missing.to_string();
        assert!(message.contains("fonts/fixture-serif@1.0.0"));
        assert!(message.contains("내려받지 않습니다"));

        // provider는 패키지마다 한 번만 조회됩니다(다운로드·재시도 없음).
        assert_eq!(provider.queries.borrow().as_slice(), &[sans, serif]);
    }

    #[test]
    fn installed_package_without_recorded_face_is_diagnosed() {
        let sans = package("fonts/fixture-sans", "2.1.0");
        // face index 2는 2-face 파일 범위 밖, hash 9는 패키지에 없는 파일.
        let refs = vec![
            external(&sans, 1, 2),
            external(&sans, 9, 0),
            external(&sans, 2, 0),
        ];
        let mut registry = InMemoryFontRegistry::new();
        let resolution = resolve_external_fonts(&refs, &provider(), &mut registry).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(resolution.diagnostics().len(), 1);
        let diagnostic = &resolution.diagnostics()[0];
        assert_eq!(diagnostic.kind(), ExternalFontDiagnosticKind::FaceMissing);
        assert_eq!(diagnostic.code(), "missing-external-font-face");
        assert_eq!(diagnostic.faces().len(), 2);
    }

    #[test]
    fn variations_do_not_change_face_identity() {
        let sans = package("fonts/fixture-sans", "2.1.0");
        let wght = FontVariation {
            tag: OpenTypeTag::parse("wght").unwrap(),
            value: FiniteF32::new(700.0).unwrap(),
        };
        let bold = ExternalFontRef::new(
            sans.clone(),
            FontFaceRef::new(FontSource::External, hash(1), 0, vec![wght]),
        )
        .unwrap();
        let refs = vec![external(&sans, 1, 0), bold];
        let mut registry = InMemoryFontRegistry::new();
        let resolution = resolve_external_fonts(&refs, &provider(), &mut registry).unwrap();
        assert!(resolution.is_fully_resolved());
        assert_eq!(resolution.handles()[0], resolution.handles()[1]);
        assert!(resolution.diagnostics().is_empty());
    }

    #[test]
    fn registry_capacity_error_is_returned() {
        let sans = package("fonts/fixture-sans", "2.1.0");
        let refs = vec![external(&sans, 1, 0), external(&sans, 1, 1)];
        let mut registry = InMemoryFontRegistry::with_max_faces(1);
        assert_eq!(
            resolve_external_fonts(&refs, &provider(), &mut registry),
            Err(FontRegistryError::CapacityExceeded { max_faces: 1 })
        );
    }

    #[test]
    fn package_identity_rules() {
        assert!(ExternalFontPackage::parse("fixture-sans", "1.0.0").is_ok());
        assert!(
            ExternalFontPackage::parse("org/fonts/fixture_sans.v2", "2026.11.0-rc.1+b3").is_ok()
        );
        for bad in [
            "", "/fixture", "fixture/", "a//b", "Fixture", "../x", "a/./b", "a b", "a@b",
        ] {
            assert!(
                matches!(
                    ExternalFontPackageName::new(bad),
                    Err(ExternalFontRefError::InvalidPackageName { .. })
                ),
                "{bad:?}"
            );
        }
        assert!(ExternalFontPackageName::new(&"a".repeat(MAX_PACKAGE_NAME_BYTES)).is_ok());
        assert!(ExternalFontPackageName::new(&"a".repeat(MAX_PACKAGE_NAME_BYTES + 1)).is_err());
        for bad in [
            "",
            "^1.0",
            "~1.0",
            ">=1.0",
            "1.*",
            "1.0 - 2.0",
            "latest ",
            ".1",
            "-1",
        ] {
            assert!(
                matches!(
                    ExternalFontPackageVersion::new(bad),
                    Err(ExternalFontRefError::InvalidPackageVersion { .. })
                ),
                "{bad:?}"
            );
        }
        assert!(
            ExternalFontPackageVersion::new(&"1".repeat(MAX_PACKAGE_VERSION_BYTES + 1)).is_err()
        );

        let pkg = package("fixture-sans", "1.0.0");
        for source in [FontSource::Project, FontSource::System] {
            assert_eq!(
                ExternalFontRef::new(pkg.clone(), FontFaceRef::new(source, hash(1), 0, vec![])),
                Err(ExternalFontRefError::NotExternalSource { source })
            );
        }
    }

    #[test]
    fn installed_package_contents_are_validated() {
        let mut contents = InstalledFontPackage::new();
        assert_eq!(
            contents.add_file(hash(1), 0),
            Err(InstalledPackageError::InvalidFaceCount { count: 0 })
        );
        assert_eq!(
            contents.add_file(hash(1), MAX_FACES_PER_FILE + 1),
            Err(InstalledPackageError::InvalidFaceCount {
                count: MAX_FACES_PER_FILE + 1
            })
        );
        contents.add_file(hash(1), 3).unwrap();
        contents.add_file(hash(1), 3).unwrap();
        assert_eq!(
            contents.add_file(hash(1), 2),
            Err(InstalledPackageError::ConflictingFaceCount { file_hash: hash(1) })
        );
        assert!(contents.contains(FontFaceKey::new(hash(1), 2)));
        assert!(!contents.contains(FontFaceKey::new(hash(1), 3)));
        assert_eq!(contents.files().collect::<Vec<_>>(), vec![(hash(1), 3)]);
    }
}
