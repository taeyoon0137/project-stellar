//! 폰트 파일 hash 불일치 재현성 경고 (XIII.6.5).
//!
//! 문서는 face를 파일 hash + face index로 기록합니다(X.8.1). 실행 환경에 같은 family 이름의 폰트가
//! 있어도 파일 hash가 기록과 다르면 **다른 face**입니다. 이름이 같다는 이유로 대체하지 않고,
//! 그 face는 해석되지 않은 채로 두며 재현성 경고([`FontHashMismatchWarning`])를 만듭니다.
//! (대체는 문서가 명시한 fallback chain만 적용합니다 — X.8.3, XIII.6.7.)
//!
//! - [`FaceNameIndex`]: 출처(Project·System·External)별로 발견한 face의 family 이름 → face key
//!   색인입니다. 이름 비교는 앞뒤 공백을 지우고 Unicode 소문자로 바꾼 값으로 합니다.
//! - [`check_font_reproducibility`]: 기록한 face마다 registry에서 hash+index로 찾고, 없으면 같은
//!   family 이름이면서 hash가 다른 face를 색인에서 찾아 경고합니다. 같은 hash의 다른 face index는
//!   hash 불일치가 아니므로 경고 후보에 넣지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_text::font::{FontFaceRef, FontSource};
use stellar_text::registry::{FontFaceKey, FontHandle, FontRegistry};

use super::project_font::ProjectFontRecord;
use super::sfnt::{validate_face_name, FaceNameError, FontFaceNames};
use super::system::SystemFontCatalog;

/// family 이름 비교 key(앞뒤 공백 제거 + Unicode 소문자).
fn family_key(family: &str) -> String {
    family.trim().to_lowercase()
}

/// 문서에 기록된 face와 그 family 이름.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpectedFontFace {
    face: FontFaceRef,
    family: String,
}

impl ExpectedFontFace {
    /// 기록된 face와 family 이름으로 만듭니다.
    pub fn new(
        face: FontFaceRef,
        family: impl Into<String>,
    ) -> Result<ExpectedFontFace, FaceNameError> {
        let family = family.into();
        validate_face_name(&family)?;
        Ok(ExpectedFontFace { face, family })
    }

    /// 프로젝트 폰트 record의 face 중 family 이름이 있는 face.
    pub fn from_project_font(record: &ProjectFontRecord) -> Vec<ExpectedFontFace> {
        record
            .face_refs()
            .zip(record.faces())
            .filter_map(|(face, names)| {
                names.family().map(|family| ExpectedFontFace {
                    face,
                    family: family.to_string(),
                })
            })
            .collect()
    }

    /// 기록된 face 참조.
    pub fn face(&self) -> &FontFaceRef {
        &self.face
    }

    /// 기록된 family 이름.
    pub fn family(&self) -> &str {
        &self.family
    }
}

/// 같은 family 이름으로 발견된 face 하나.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFaceCandidate {
    /// face 정체성(hash+index).
    pub key: FontFaceKey,
    /// 발견된 출처(오름차순).
    pub sources: Vec<FontSource>,
}

/// 출처별로 발견한 face의 family 이름 색인.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FaceNameIndex {
    by_family: BTreeMap<String, BTreeMap<FontFaceKey, BTreeSet<FontSource>>>,
}

impl FaceNameIndex {
    /// 빈 색인.
    pub fn new() -> FaceNameIndex {
        FaceNameIndex::default()
    }

    /// face 하나를 추가합니다. family 이름이 없으면 무시합니다.
    pub fn insert(&mut self, source: FontSource, key: FontFaceKey, names: &FontFaceNames) {
        if let Some(family) = names.family() {
            self.by_family
                .entry(family_key(family))
                .or_default()
                .entry(key)
                .or_default()
                .insert(source);
        }
    }

    /// 시스템 폰트 discovery 결과를 추가합니다(XIII.6.2).
    pub fn add_system_catalog(&mut self, catalog: &SystemFontCatalog) {
        for face in catalog.faces() {
            self.insert(FontSource::System, face.key(), face.names());
        }
    }

    /// 프로젝트 폰트 record를 추가합니다(XIII.6.1).
    pub fn add_project_font(&mut self, record: &ProjectFontRecord) {
        for (index, names) in (0u32..).zip(record.faces()) {
            self.insert(
                FontSource::Project,
                FontFaceKey::new(record.file_hash(), index),
                names,
            );
        }
    }

    /// family 이름이 같은 face(key 오름차순).
    pub fn candidates(&self, family: &str) -> Vec<FontFaceCandidate> {
        self.by_family
            .get(&family_key(family))
            .map(|faces| {
                faces
                    .iter()
                    .map(|(key, sources)| FontFaceCandidate {
                        key: *key,
                        sources: sources.iter().copied().collect(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// 같은 family 이름이지만 파일 hash가 다른 face만 있다는 재현성 경고.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontHashMismatchWarning {
    expected: FontFaceRef,
    family: String,
    candidates: Vec<FontFaceCandidate>,
}

impl FontHashMismatchWarning {
    /// 안정 진단 코드.
    pub const CODE: &'static str = "font-hash-mismatch";

    /// 기록된 face.
    pub fn expected(&self) -> &FontFaceRef {
        &self.expected
    }

    /// 기록된 family 이름.
    pub fn family(&self) -> &str {
        &self.family
    }

    /// 같은 family 이름의 다른 hash face(대체하지 않음, key 오름차순).
    pub fn candidates(&self) -> &[FontFaceCandidate] {
        &self.candidates
    }
}

impl fmt::Display for FontHashMismatchWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "family '{}'의 기록된 폰트 파일({}#{})이 없고 hash가 다른 같은 이름 face {}개만 있습니다. \
             다른 face로 보고 대체하지 않습니다",
            self.family,
            self.expected.file_hash(),
            self.expected.face_index(),
            self.candidates.len()
        )
    }
}

/// 재현성 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproducibilityReport {
    handles: Vec<Option<FontHandle>>,
    warnings: Vec<FontHashMismatchWarning>,
}

impl ReproducibilityReport {
    /// 입력 순서의 handle. hash+index가 정확히 같은 face만 해석됩니다.
    pub fn handles(&self) -> &[Option<FontHandle>] {
        &self.handles
    }

    /// 재현성 경고(기록 face 오름차순, 같은 기록은 한 번).
    pub fn warnings(&self) -> &[FontHashMismatchWarning] {
        &self.warnings
    }
}

/// 기록된 face를 hash+index로만 해석하고, 같은 이름·다른 hash face가 있으면 경고합니다.
pub fn check_font_reproducibility<R: FontRegistry + ?Sized>(
    expected: &[ExpectedFontFace],
    registry: &R,
    names: &FaceNameIndex,
) -> ReproducibilityReport {
    let mut warnings: BTreeMap<(&FontFaceRef, String), FontHashMismatchWarning> = BTreeMap::new();
    let handles = expected
        .iter()
        .map(|item| {
            let handle = registry.resolve(&item.face);
            if handle.is_none() {
                let candidates: Vec<FontFaceCandidate> = names
                    .candidates(&item.family)
                    .into_iter()
                    .filter(|c| c.key.file_hash() != item.face.file_hash())
                    .collect();
                if !candidates.is_empty() {
                    warnings
                        .entry((&item.face, family_key(&item.family)))
                        .or_insert_with(|| FontHashMismatchWarning {
                            expected: item.face.clone(),
                            family: item.family.clone(),
                            candidates,
                        });
                }
            }
            handle
        })
        .collect();
    ReproducibilityReport {
        handles,
        warnings: warnings.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::license::FontLicenseInfo;
    use crate::fonts::project_font::import_project_font;
    use crate::fonts::system::{SystemFontDiscovery, SystemFontLimits};
    use crate::fonts::test_fixtures::{family_faces, sfnt_face, TempDir, SIG_TRUETYPE};
    use std::path::Path;
    use stellar_assets::id::AssetId;
    use stellar_assets::policy::AssetImportPolicy;
    use stellar_assets::store::AssetStore;
    use stellar_text::font::FontFileHash;
    use stellar_text::registry::InMemoryFontRegistry;

    fn shipped_policy() -> AssetImportPolicy {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/asset-import.yaml"
        );
        AssetImportPolicy::from_config_file(Path::new(path)).unwrap()
    }

    /// 같은 이름("Fixture Sans" Regular)이지만 bytes가 다른 두 폰트 파일.
    fn same_name_different_bytes() -> (Vec<u8>, Vec<u8>) {
        let original = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        let mut revised = original.clone();
        revised.extend_from_slice(b"rev2");
        (original, revised)
    }

    #[test]
    fn same_name_with_different_hash_is_a_different_face_with_warning() {
        let (original, revised) = same_name_different_bytes();
        let project = TempDir::new("project");
        let system = TempDir::new("system");
        // 프로젝트가 기록한 face: original.
        let source = project.path.join("FixtureSans-Regular.ttf");
        std::fs::write(&source, &original).unwrap();
        let record = import_project_font(
            &AssetStore::new(&project.path),
            &shipped_policy(),
            AssetId::from_bytes([1; 16]),
            &source,
            FontLicenseInfo::new(),
        )
        .unwrap()
        .font;
        let expected = ExpectedFontFace::from_project_font(&record);
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].family(), "Fixture Sans");

        // 실행 환경(이 머신)에는 같은 이름의 revised만 system font로 있습니다.
        let installed = system.path.join("FixtureSans-Regular.ttf");
        std::fs::write(&installed, &revised).unwrap();
        let mut registry = InMemoryFontRegistry::new();
        let catalog = SystemFontDiscovery::new(SystemFontLimits::default())
            .register_files([installed], &mut registry);
        let system_face = &catalog.faces()[0];
        assert_eq!(system_face.names(), record.face_names(0).unwrap());
        assert_ne!(system_face.key().file_hash(), record.file_hash());

        let mut names = FaceNameIndex::new();
        names.add_system_catalog(&catalog);
        let report = check_font_reproducibility(&expected, &registry, &names);
        // 이름이 같아도 대체하지 않습니다.
        assert_eq!(report.handles(), &[None]);
        assert_eq!(report.warnings().len(), 1);
        let warning = &report.warnings()[0];
        assert_eq!(FontHashMismatchWarning::CODE, "font-hash-mismatch");
        assert_eq!(warning.expected(), expected[0].face());
        assert_eq!(warning.family(), "Fixture Sans");
        assert_eq!(
            warning.candidates(),
            &[FontFaceCandidate {
                key: system_face.key(),
                sources: vec![FontSource::System],
            }]
        );
        assert!(warning.to_string().contains("대체하지 않습니다"));

        // 기록한 파일도 등록되면 그 face로만 해석되고 경고가 없습니다(두 face는 다른 handle).
        let project_handles = record.register_faces(&mut registry).unwrap();
        names.add_project_font(&record);
        let report = check_font_reproducibility(&expected, &registry, &names);
        assert_eq!(report.handles(), &[Some(project_handles[0])]);
        assert_ne!(project_handles[0], system_face.handle());
        assert!(report.warnings().is_empty());
        assert_eq!(names.candidates("fixture sans").len(), 2);
    }

    #[test]
    fn no_warning_without_same_name_candidates_or_for_same_file_other_index() {
        let hash = FontFileHash::from_sha256_digest([3; 32]);
        let expected = vec![ExpectedFontFace::new(
            FontFaceRef::new(FontSource::Project, hash, 1, vec![]),
            "Fixture Serif",
        )
        .unwrap()];
        let registry = InMemoryFontRegistry::new();
        let mut names = FaceNameIndex::new();
        // 다른 family만 있음.
        let other = FontFaceNames::new(Some("Other".into()), None, None).unwrap();
        names.insert(
            FontSource::System,
            FontFaceKey::new(FontFileHash::from_sha256_digest([4; 32]), 0),
            &other,
        );
        // 같은 파일의 다른 face index(hash 불일치 아님).
        let serif = FontFaceNames::new(Some("  FIXTURE serif ".into()), None, None).unwrap();
        names.insert(FontSource::System, FontFaceKey::new(hash, 0), &serif);
        // family 없는 face는 색인되지 않습니다.
        names.insert(
            FontSource::System,
            FontFaceKey::new(FontFileHash::from_sha256_digest([5; 32]), 0),
            &FontFaceNames::default(),
        );
        let report = check_font_reproducibility(&expected, &registry, &names);
        assert_eq!(report.handles(), &[None]);
        assert!(report.warnings().is_empty());
        assert_eq!(names.candidates("Fixture Serif").len(), 1);
    }

    #[test]
    fn warnings_are_deduplicated_and_ordered() {
        let recorded = FontFaceRef::new(
            FontSource::System,
            FontFileHash::from_sha256_digest([1; 32]),
            0,
            vec![],
        );
        let item = ExpectedFontFace::new(recorded.clone(), "Fixture Sans").unwrap();
        let sans = FontFaceNames::new(Some("Fixture Sans".into()), None, None).unwrap();
        let mut names = FaceNameIndex::new();
        let found = FontFaceKey::new(FontFileHash::from_sha256_digest([2; 32]), 0);
        names.insert(FontSource::System, found, &sans);
        names.insert(FontSource::External, found, &sans);
        let registry = InMemoryFontRegistry::new();
        let report = check_font_reproducibility(&[item.clone(), item], &registry, &names);
        assert_eq!(report.handles(), &[None, None]);
        assert_eq!(report.warnings().len(), 1);
        assert_eq!(
            report.warnings()[0].candidates()[0].sources,
            vec![FontSource::System, FontSource::External]
        );
        assert_eq!(
            ExpectedFontFace::new(recorded, ""),
            Err(FaceNameError::Empty)
        );
    }
}
