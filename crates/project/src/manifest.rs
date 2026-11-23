//! `stellar.yaml` 매니페스트 데이터 모델 (IV.1.1).
//!
//! 사용자가 관리하는 프로젝트 설정과 의존성 선언을 표현합니다(기술 아키텍처 20장). 본 모듈은
//! 데이터 구조와 [`YamlValue`] 사이의 변환(`to_yaml`/`from_yaml`)을 정의합니다. 텍스트 파싱은
//! IV.1.2 제한 loader, canonical 기록은 IV.1.5 writer가 담당합니다.
//!
//! 필수 최상위 필드: `schema_version`, `project`, `target_profiles`, `dependencies`.
//! 선택 최상위 필드: `style_defaults`(활성 기본값 세트 이름, ADR-0082 결정 4, ADR-0086). 없으면 `css`입니다.
//! 알 수 없는 키는 거부합니다(제한 스키마).
//!
//! `dependencies` 절의 entry 스키마는 의존성 모델(XV.1.1, `stellar-dependencies`)이 정의합니다:
//! logical name(매핑 키), `source`(Git), `library_id`, `repository`, `version`(tag 스칼라 또는
//! `{branch: ...}`·`{commit: ...}`), 선택 `local_override`(XV.6.1). 이 모듈은 [`YamlValue`]를
//! 형식 중립 트리로 바꿔([`crate::manifest_node`]) 그 모델로 검증·기록하므로 `stellar.yaml`
//! 의존성 형식은 하나뿐입니다.
//!
//! ```yaml
//! dependencies:
//!   design-system:
//!     source: git
//!     library_id: 0195c530-3acd-7bc1-8414-c213f61cb965
//!     repository: git@github.com:example/design-system.git
//!     version: v2.4.0
//!   icons:
//!     source: git
//!     library_id: 0195c530-3acd-7bc1-8414-c213f61cb966
//!     repository: https://example.com/icons.git
//!     version:
//!       branch: main
//!     local_override: ../icons
//! ```

use std::path::Path;

use stellar_dependencies::declaration::{dependencies_from_node, dependencies_to_node};
use stellar_dependencies::local_override::{local_overrides, LocalOverrideMap};
use stellar_dependencies::node::{SchemaError, ValueError};
use stellar_dependencies::validation::{validate_declarations, DeclarationReport};

use crate::manifest_node::{node_to_yaml, yaml_to_node, NodeConversionError};
use crate::yaml::YamlValue;

/// 의존성 선언 entry(XV.1.1 모델). `stellar.yaml` `dependencies` 절의 유일한 표현입니다.
pub use stellar_dependencies::declaration::DependencyDeclaration as Dependency;

/// 현재 매니페스트 스키마 버전.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// 매니페스트 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// 필수 필드 누락.
    MissingField {
        /// 누락된 필드의 경로(예: `project.id`).
        path: String,
    },
    /// 타입 불일치.
    WrongType {
        /// 필드 경로.
        path: String,
        /// 기대한 종류(`mapping`/`sequence`/`scalar`).
        expected: &'static str,
    },
    /// `schema_version`가 음이 아닌 정수가 아님.
    InvalidSchemaVersion {
        /// 원본 값.
        value: String,
    },
    /// 알 수 없는 키(제한 스키마).
    UnknownKey {
        /// 키가 나타난 경로.
        path: String,
    },
    /// 의존성 선언 값 규칙 위반(XV.1.1). 입력 값은 담지 않습니다(XV.1.4).
    InvalidValue {
        /// 필드 경로(예: `dependencies.design-system.library_id`).
        path: String,
        /// 위반한 규칙.
        error: ValueError,
    },
    /// 컨테이너 중첩 깊이 초과.
    TooDeep {
        /// 상한을 넘은 위치.
        path: String,
    },
    /// `style_defaults` 값이 세트 이름 형식이 아닙니다.
    InvalidStyleDefaults {
        /// 원본 값.
        value: String,
    },
}

impl From<SchemaError> for ManifestError {
    fn from(error: SchemaError) -> ManifestError {
        match error {
            SchemaError::MissingField { path } => ManifestError::MissingField { path },
            SchemaError::WrongType { path, expected } => {
                ManifestError::WrongType { path, expected }
            }
            SchemaError::UnknownKey { path } => ManifestError::UnknownKey { path },
            SchemaError::InvalidValue { path, error } => {
                ManifestError::InvalidValue { path, error }
            }
        }
    }
}

impl From<NodeConversionError> for ManifestError {
    fn from(error: NodeConversionError) -> ManifestError {
        match error {
            NodeConversionError::NonScalarKey { path } => ManifestError::WrongType {
                path: format!("{path} 키"),
                expected: "scalar",
            },
            NodeConversionError::TooDeep { path } => ManifestError::TooDeep { path },
        }
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::MissingField { path } => write!(f, "필수 필드 누락: {path}"),
            ManifestError::WrongType { path, expected } => {
                write!(f, "타입 불일치: {path}는 {expected}여야 합니다")
            }
            ManifestError::InvalidSchemaVersion { value } => {
                write!(f, "schema_version가 음이 아닌 정수가 아닙니다: {value}")
            }
            ManifestError::UnknownKey { path } => write!(f, "알 수 없는 키: {path}"),
            ManifestError::InvalidValue { path, error } => {
                write!(f, "잘못된 값({path}): {error}")
            }
            ManifestError::TooDeep { path } => write!(f, "중첩 깊이 상한 초과: {path}"),
            ManifestError::InvalidStyleDefaults { value } => {
                write!(f, "style_defaults 값 {value}은 세트 이름 형식이 아닙니다")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// 프로젝트 식별 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMeta {
    /// 프로젝트 ID(UUID 텍스트, StableId 매핑은 IV.2.3).
    pub id: String,
    /// 사람이 읽는 이름.
    pub name: String,
}

/// 대상 프로파일. 플랫폼별 선택 필드를 포함합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetProfile {
    /// 프로파일 ID.
    pub id: String,
    /// 플랫폼(예: `apple`, `web`).
    pub platform: String,
    /// 최소 OS 버전(있으면).
    pub minimum_os: Option<String>,
    /// baseline(있으면).
    pub baseline: Option<String>,
}

/// `stellar.yaml` 매니페스트.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// 스키마 버전.
    pub schema_version: u32,
    /// 프로젝트 정보.
    pub project: ProjectMeta,
    /// 대상 프로파일(저장 순서 보존).
    pub target_profiles: Vec<TargetProfile>,
    /// 의존성 선언(저장 순서 보존, XV.1.1 모델).
    pub dependencies: Vec<Dependency>,
    /// 활성 기본값 세트 이름(`style_defaults`). `None`이면 `css`입니다(ADR-0082 결정 4).
    pub style_defaults: Option<String>,
}

impl Manifest {
    /// 매니페스트를 [`YamlValue`] 트리로 변환합니다(키 순서 고정).
    pub fn to_yaml(&self) -> YamlValue {
        let top: Vec<(YamlValue, YamlValue)> = vec![
            (
                YamlValue::scalar("schema_version"),
                YamlValue::scalar(self.schema_version.to_string()),
            ),
            (
                YamlValue::scalar("project"),
                YamlValue::Mapping(vec![
                    (
                        YamlValue::scalar("id"),
                        YamlValue::scalar(self.project.id.clone()),
                    ),
                    (
                        YamlValue::scalar("name"),
                        YamlValue::scalar(self.project.name.clone()),
                    ),
                ]),
            ),
            (
                YamlValue::scalar("target_profiles"),
                YamlValue::Sequence(self.target_profiles.iter().map(profile_to_yaml).collect()),
            ),
            (
                YamlValue::scalar("dependencies"),
                node_to_yaml(&dependencies_to_node(&self.dependencies)),
            ),
        ];
        let mut top = top;
        if let Some(name) = &self.style_defaults {
            top.push((
                YamlValue::scalar("style_defaults"),
                YamlValue::scalar(name.clone()),
            ));
        }
        YamlValue::Mapping(top)
    }

    /// 의존성 선언 충돌을 검사합니다(XV.1.3: 같은 logical name 중복, 같은 LibraryId 다른 선택자,
    /// 다른 LibraryId 같은 repository). 스키마 검증([`Manifest::from_yaml`])과 별개의 의미 검사라
    /// 프로젝트 열기를 막지 않고 진단으로 보고합니다.
    pub fn validate_dependencies(&self) -> DeclarationReport {
        validate_declarations(&self.dependencies)
    }

    /// `local_override`를 logical name → 로컬 프로젝트 root로 매핑합니다(XV.6.1).
    ///
    /// `project_root`는 이 매니페스트가 있는 사용 프로젝트 root의 절대 경로입니다. 대상이 사용
    /// 프로젝트 자신·내부이거나 같은 이름 override가 두 번이면 경로와 함께 거부합니다.
    pub fn local_overrides(&self, project_root: &Path) -> Result<LocalOverrideMap, ManifestError> {
        Ok(local_overrides(&self.dependencies, project_root)?)
    }

    /// 매니페스트를 loader 제한을 강제하며 canonical 텍스트로 기록합니다.
    ///
    /// 제한 loader가 다시 읽지 못할 출력(같은 이름의 의존성 → 중복 키, `<<` 이름, 크기·선읽기 상한
    /// 초과)은 텍스트를 만들지 않고 [`WriteError`](crate::yaml::WriteError)로 거부합니다.
    pub fn write(&self) -> Result<String, crate::yaml::WriteError> {
        crate::yaml::write_canonical_checked(&self.to_yaml())
    }

    /// [`YamlValue`] 트리에서 매니페스트를 만듭니다. 알 수 없는 키와 타입 불일치를 거부합니다.
    pub fn from_yaml(value: &YamlValue) -> Result<Manifest, ManifestError> {
        let top = expect_mapping(value, "<root>")?;
        reject_unknown_keys(
            top,
            "<root>",
            &[
                "schema_version",
                "project",
                "target_profiles",
                "dependencies",
                "style_defaults",
            ],
        )?;

        let schema_version = parse_schema_version(require(value, "schema_version")?)?;
        let project = parse_project(require(value, "project")?)?;
        let target_profiles = parse_target_profiles(require(value, "target_profiles")?)?;
        let dependencies = parse_dependencies(require(value, "dependencies")?)?;
        let style_defaults = match value.get("style_defaults") {
            None => None,
            Some(node) => {
                let name = expect_scalar(node, "style_defaults")?;
                if !stellar_document::css::style_defaults::is_valid_set_name(name) {
                    return Err(ManifestError::InvalidStyleDefaults {
                        value: name.to_string(),
                    });
                }
                Some(name.to_string())
            }
        };

        Ok(Manifest {
            schema_version,
            project,
            target_profiles,
            dependencies,
            style_defaults,
        })
    }
}

fn profile_to_yaml(profile: &TargetProfile) -> YamlValue {
    let mut pairs: Vec<(YamlValue, YamlValue)> = vec![
        (
            YamlValue::scalar("id"),
            YamlValue::scalar(profile.id.clone()),
        ),
        (
            YamlValue::scalar("platform"),
            YamlValue::scalar(profile.platform.clone()),
        ),
    ];
    if let Some(minimum_os) = &profile.minimum_os {
        pairs.push((
            YamlValue::scalar("minimum_os"),
            YamlValue::scalar(minimum_os.clone()),
        ));
    }
    if let Some(baseline) = &profile.baseline {
        pairs.push((
            YamlValue::scalar("baseline"),
            YamlValue::scalar(baseline.clone()),
        ));
    }
    YamlValue::Mapping(pairs)
}

fn require<'a>(value: &'a YamlValue, key: &str) -> Result<&'a YamlValue, ManifestError> {
    value.get(key).ok_or_else(|| ManifestError::MissingField {
        path: key.to_string(),
    })
}

fn expect_mapping<'a>(
    value: &'a YamlValue,
    path: &str,
) -> Result<&'a [(YamlValue, YamlValue)], ManifestError> {
    value.as_mapping().ok_or_else(|| ManifestError::WrongType {
        path: path.to_string(),
        expected: "mapping",
    })
}

fn expect_scalar<'a>(value: &'a YamlValue, path: &str) -> Result<&'a str, ManifestError> {
    value.as_scalar().ok_or_else(|| ManifestError::WrongType {
        path: path.to_string(),
        expected: "scalar",
    })
}

fn reject_unknown_keys(
    pairs: &[(YamlValue, YamlValue)],
    parent: &str,
    allowed: &[&str],
) -> Result<(), ManifestError> {
    for (key, _) in pairs {
        let Some(name) = key.as_scalar() else {
            return Err(ManifestError::WrongType {
                path: format!("{parent} 키"),
                expected: "scalar",
            });
        };
        if !allowed.contains(&name) {
            return Err(ManifestError::UnknownKey {
                path: join(parent, name),
            });
        }
    }
    Ok(())
}

fn parse_schema_version(value: &YamlValue) -> Result<u32, ManifestError> {
    let text = expect_scalar(value, "schema_version")?;
    // 엄격 정수 해석(IV.1.4): 암묵 타입 추론 없이 canonical base-10만 허용.
    // 부호·소수점·공백·leading-zero(`01`)·진법 접두·`yes` 등은 거부(`0`은 허용).
    crate::yaml::scalar::parse_u32(text).ok_or_else(|| ManifestError::InvalidSchemaVersion {
        value: text.to_string(),
    })
}

fn parse_project(value: &YamlValue) -> Result<ProjectMeta, ManifestError> {
    let pairs = expect_mapping(value, "project")?;
    reject_unknown_keys(pairs, "project", &["id", "name"])?;
    let id = expect_scalar(require(value, "id").map_err(at("project"))?, "project.id")?;
    let name = expect_scalar(
        require(value, "name").map_err(at("project"))?,
        "project.name",
    )?;
    Ok(ProjectMeta {
        id: id.to_string(),
        name: name.to_string(),
    })
}

fn parse_target_profiles(value: &YamlValue) -> Result<Vec<TargetProfile>, ManifestError> {
    let items = value
        .as_sequence()
        .ok_or_else(|| ManifestError::WrongType {
            path: "target_profiles".to_string(),
            expected: "sequence",
        })?;
    let mut profiles = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let path = format!("target_profiles[{index}]");
        let pairs = expect_mapping(item, &path)?;
        reject_unknown_keys(pairs, &path, &["id", "platform", "minimum_os", "baseline"])?;
        let id = expect_scalar(
            item.get("id").ok_or_else(|| ManifestError::MissingField {
                path: join(&path, "id"),
            })?,
            &join(&path, "id"),
        )?;
        let platform = expect_scalar(
            item.get("platform")
                .ok_or_else(|| ManifestError::MissingField {
                    path: join(&path, "platform"),
                })?,
            &join(&path, "platform"),
        )?;
        let minimum_os = optional_scalar(item, "minimum_os", &path)?;
        let baseline = optional_scalar(item, "baseline", &path)?;
        profiles.push(TargetProfile {
            id: id.to_string(),
            platform: platform.to_string(),
            minimum_os,
            baseline,
        });
    }
    Ok(profiles)
}

fn parse_dependencies(value: &YamlValue) -> Result<Vec<Dependency>, ManifestError> {
    // 의존성 entry 스키마는 의존성 모델 하나가 정의합니다(XV.1.1). 같은 logical name 중복은
    // 보존하고 판정은 XV.1.3 validation이 합니다(제한 loader는 중복 키를 이미 거부합니다).
    let node = yaml_to_node(value, "dependencies")?;
    Ok(dependencies_from_node(&node)?)
}

fn optional_scalar(
    item: &YamlValue,
    key: &str,
    parent: &str,
) -> Result<Option<String>, ManifestError> {
    match item.get(key) {
        None => Ok(None),
        Some(v) => Ok(Some(expect_scalar(v, &join(parent, key))?.to_string())),
    }
}

fn join(parent: &str, child: &str) -> String {
    format!("{parent}.{child}")
}

/// `require`가 반환한 누락 오류의 경로 앞에 부모 경로를 붙입니다.
fn at(parent: &'static str) -> impl Fn(ManifestError) -> ManifestError {
    move |err| match err {
        ManifestError::MissingField { path } => ManifestError::MissingField {
            path: join(parent, &path),
        },
        other => other,
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 의존성 선언 fixture(manifest·lock·writer 테스트).

    use stellar_dependencies::declaration::{
        LibraryId, LocalOverridePath, LogicalName, RefName, RepositoryUrl, SourceKind,
        VersionSelector,
    };

    use super::Dependency;

    /// 번호로 결정적 LibraryId(UUID v7 형식)를 만듭니다.
    pub(crate) fn library(n: u8) -> LibraryId {
        format!("0195c530-3acd-7bc1-8414-{:012x}", n)
            .parse()
            .expect("library id fixture")
    }

    /// tag 선택자 선언.
    pub(crate) fn tag_dependency(name: &str, lib: u8, url: &str, tag: &str) -> Dependency {
        Dependency {
            name: LogicalName::new(name).expect("name"),
            library_id: library(lib),
            source: SourceKind::Git,
            repository: RepositoryUrl::new(url).expect("url"),
            version: VersionSelector::Tag(RefName::new(tag).expect("tag")),
            local_override: None,
            components: Default::default(),
        }
    }

    /// branch 선택자 + local override 선언.
    pub(crate) fn branch_dependency(
        name: &str,
        lib: u8,
        url: &str,
        branch: &str,
        local_override: Option<&str>,
    ) -> Dependency {
        let mut dep = tag_dependency(name, lib, url, "unused");
        dep.version = VersionSelector::Branch(RefName::new(branch).expect("branch"));
        dep.local_override = local_override.map(|p| LocalOverridePath::new(p).expect("override"));
        dep
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{branch_dependency, library, tag_dependency};
    use super::*;
    use stellar_dependencies::declaration::{CommitId, LogicalName, VersionSelector};
    use stellar_dependencies::validation::ConflictKind;

    fn minimal() -> Manifest {
        Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Project Stellar".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
            style_defaults: None,
        }
    }

    fn full() -> Manifest {
        Manifest {
            schema_version: 1,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Project Stellar".to_string(),
            },
            target_profiles: vec![
                TargetProfile {
                    id: "apple-desktop".to_string(),
                    platform: "apple".to_string(),
                    minimum_os: Some("26.0".to_string()),
                    baseline: None,
                },
                TargetProfile {
                    id: "web-modern".to_string(),
                    platform: "web".to_string(),
                    minimum_os: None,
                    baseline: Some("modern".to_string()),
                },
            ],
            dependencies: vec![tag_dependency(
                "design-system",
                1,
                "git@github.com:example/design-system.git",
                "v2.4.0",
            )],
            style_defaults: None,
        }
    }

    #[test]
    fn minimal_round_trips() {
        let manifest = minimal();
        let restored = Manifest::from_yaml(&manifest.to_yaml()).unwrap();
        assert_eq!(restored, manifest);
    }

    #[test]
    fn full_round_trips() {
        let manifest = full();
        let restored = Manifest::from_yaml(&manifest.to_yaml()).unwrap();
        assert_eq!(restored, manifest);
        // 선택 필드가 정확히 보존되는지.
        assert_eq!(
            restored.target_profiles[0].minimum_os.as_deref(),
            Some("26.0")
        );
        assert_eq!(restored.target_profiles[0].baseline, None);
        assert_eq!(
            restored.target_profiles[1].baseline.as_deref(),
            Some("modern")
        );
        assert_eq!(restored.dependencies[0].name.as_str(), "design-system");
    }

    #[test]
    fn rejects_missing_required_field() {
        // schema_version 없는 매핑.
        let value = YamlValue::Mapping(vec![(
            YamlValue::scalar("project"),
            YamlValue::Mapping(vec![
                (YamlValue::scalar("id"), YamlValue::scalar("x")),
                (YamlValue::scalar("name"), YamlValue::scalar("y")),
            ]),
        )]);
        assert!(matches!(
            Manifest::from_yaml(&value),
            Err(ManifestError::UnknownKey { .. }) | Err(ManifestError::MissingField { .. })
        ));
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let mut yaml = minimal().to_yaml();
        if let YamlValue::Mapping(pairs) = &mut yaml {
            pairs.push((YamlValue::scalar("extra"), YamlValue::scalar("1")));
        }
        assert_eq!(
            Manifest::from_yaml(&yaml),
            Err(ManifestError::UnknownKey {
                path: "<root>.extra".to_string()
            })
        );
    }

    #[test]
    fn rejects_invalid_schema_version() {
        for bad in ["01", "-1", "1.0", "x", "", " 1"] {
            let value = YamlValue::Mapping(vec![
                (YamlValue::scalar("schema_version"), YamlValue::scalar(bad)),
                (
                    YamlValue::scalar("project"),
                    YamlValue::Mapping(vec![
                        (YamlValue::scalar("id"), YamlValue::scalar("x")),
                        (YamlValue::scalar("name"), YamlValue::scalar("y")),
                    ]),
                ),
                (
                    YamlValue::scalar("target_profiles"),
                    YamlValue::Sequence(vec![]),
                ),
                (
                    YamlValue::scalar("dependencies"),
                    YamlValue::Mapping(vec![]),
                ),
            ]);
            assert!(
                matches!(
                    Manifest::from_yaml(&value),
                    Err(ManifestError::InvalidSchemaVersion { .. })
                ),
                "bad={bad:?}"
            );
        }
    }

    #[test]
    fn yaml_1_1_confusables_preserved_as_strings_end_to_end() {
        // 암시적 타입 추론 없음(IV.1.4): yes/날짜/8진수-혼동/on을 문자열로 보존.
        let text = "schema_version: 1\n\
                    project:\n  id: yes\n  name: 2020-01-01\n\
                    target_profiles:\n  - id: 010\n    platform: on\n\
                    dependencies: {}\n";
        let value = crate::yaml::load_restricted(text).unwrap();
        let manifest = Manifest::from_yaml(&value).unwrap();
        assert_eq!(manifest.project.id, "yes"); // 불리언 아님
        assert_eq!(manifest.project.name, "2020-01-01"); // 날짜 아님
        assert_eq!(manifest.target_profiles[0].id, "010"); // 8진수/정수 아님(원문 보존)
        assert_eq!(manifest.target_profiles[0].platform, "on"); // 불리언 아님
    }

    #[test]
    fn schema_version_rejects_yaml_1_1_confusables_end_to_end() {
        for bad in ["yes", "01", "0x1", "1_0", "true", "1.0"] {
            let text = format!(
                "schema_version: {bad}\n\
                 project:\n  id: x\n  name: y\n\
                 target_profiles: []\n\
                 dependencies: {{}}\n"
            );
            let value = crate::yaml::load_restricted(&text).unwrap();
            assert!(
                matches!(
                    Manifest::from_yaml(&value),
                    Err(ManifestError::InvalidSchemaVersion { .. })
                ),
                "bad={bad:?}"
            );
        }
    }

    #[test]
    fn rejects_wrong_type_for_project() {
        let value = YamlValue::Mapping(vec![
            (YamlValue::scalar("schema_version"), YamlValue::scalar("1")),
            (YamlValue::scalar("project"), YamlValue::scalar("oops")),
            (
                YamlValue::scalar("target_profiles"),
                YamlValue::Sequence(vec![]),
            ),
            (
                YamlValue::scalar("dependencies"),
                YamlValue::Mapping(vec![]),
            ),
        ]);
        assert_eq!(
            Manifest::from_yaml(&value),
            Err(ManifestError::WrongType {
                path: "project".to_string(),
                expected: "mapping"
            })
        );
    }

    #[test]
    fn checked_write_rejects_manifests_that_would_not_reload() {
        let dep = |name: &str| tag_dependency(name, 1, "git@example.com:lib.git", "v1");
        let base = Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Demo".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: vec![dep("design-system")],
            style_defaults: None,
        };
        // 정상 매니페스트는 기록 후 같은 모델로 다시 열림.
        let text = base.write().unwrap();
        let reloaded = Manifest::from_yaml(&crate::yaml::load_restricted(&text).unwrap()).unwrap();
        assert_eq!(reloaded, base);

        // 같은 이름의 의존성 → 중복 매핑 키.
        let mut dup = base.clone();
        dup.dependencies.push(dep("design-system"));
        assert!(matches!(
            dup.write(),
            Err(crate::yaml::WriteError::DuplicateKey { .. })
        ));
        // `<<` 이름(merge key)은 의존성 모델의 logical name 규칙이 먼저 막습니다.
        assert!(LogicalName::new("<<").is_err());
        // 선읽기 상한을 넘는 이름.
        let mut long = base.clone();
        long.project.name = "n".repeat(crate::yaml::loader::MAX_EVENT_LOOKAHEAD_CHARS + 1);
        assert!(matches!(
            long.write(),
            Err(crate::yaml::WriteError::NotReloadable(_))
        ));
    }

    /// 아키텍처 20.3 형식 + XV.1.1 entry 스키마(tag 스칼라, branch 매핑 + local override,
    /// commit 매핑)를 담은 실제 `stellar.yaml` 텍스트(canonical 형식).
    const DEPENDENCY_SCHEMA_TEXT: &str = r#"schema_version: "1"
project:
  id: "0195c530-3acd-7bc1-8414-c213f61cb965"
  name: "Project Stellar"
target_profiles: []
dependencies:
  design-system:
    source: git
    library_id: "0195c530-3acd-7bc1-8414-000000000001"
    repository: "git@github.com:example/design-system.git"
    version: "v2.4.0"
  icons:
    source: git
    library_id: "0195c530-3acd-7bc1-8414-000000000002"
    repository: "https://example.com/icons.git"
    version:
      branch: "feature/next"
    local_override: "../icons"
  tokens:
    source: git
    library_id: "0195c530-3acd-7bc1-8414-000000000003"
    repository: "file:///srv/tokens"
    version:
      commit: abababababababababababababababababababab
"#;

    fn parse_text(text: &str) -> Result<Manifest, ManifestError> {
        Manifest::from_yaml(&crate::yaml::load_restricted(text).expect("제한 YAML"))
    }

    #[test]
    fn dependency_entry_schema_round_trips_through_stellar_yaml_parser() {
        // XV.1.1: 실제 stellar.yaml 파서(제한 loader → Manifest::from_yaml)가 entry 스키마를 받음.
        let manifest = parse_text(DEPENDENCY_SCHEMA_TEXT).expect("새 스키마 파싱");
        let mut pinned = tag_dependency("tokens", 3, "file:///srv/tokens", "unused");
        pinned.version = VersionSelector::Commit(
            "abababababababababababababababababababab"
                .parse::<CommitId>()
                .unwrap(),
        );
        let expected = vec![
            tag_dependency(
                "design-system",
                1,
                "git@github.com:example/design-system.git",
                "v2.4.0",
            ),
            branch_dependency(
                "icons",
                2,
                "https://example.com/icons.git",
                "feature/next",
                Some("../icons"),
            ),
            pinned,
        ];
        assert_eq!(manifest.dependencies, expected);
        assert_eq!(manifest.dependencies[0].library_id, library(1));

        // canonical writer 출력은 입력 텍스트와 byte 단위로 같고, 다시 읽으면 같은 모델.
        let written = manifest.write().expect("checked write");
        assert_eq!(written, DEPENDENCY_SCHEMA_TEXT);
        assert_eq!(parse_text(&written).unwrap(), manifest);

        // `version: {tag: ...}` 입력은 스칼라로 정규화해 기록.
        let tag_mapping = DEPENDENCY_SCHEMA_TEXT.replace(
            "    version: \"v2.4.0\"\n",
            "    version:\n      tag: \"v2.4.0\"\n",
        );
        let normalized = parse_text(&tag_mapping).unwrap();
        assert_eq!(normalized, manifest);
        assert_eq!(normalized.write().unwrap(), DEPENDENCY_SCHEMA_TEXT);
    }

    #[test]
    fn dependency_schema_violations_are_rejected_by_stellar_yaml_parser() {
        let manifest_with = |entry: &str| {
            format!(
                "schema_version: 1\nproject:\n  id: x\n  name: y\ntarget_profiles: []\n\
                 dependencies:\n  lib:\n{entry}"
            )
        };
        let valid = "    source: git\n    library_id: 0195c530-3acd-7bc1-8414-000000000009\n    \
                     repository: https://e/x\n    version: v1\n";
        assert!(parse_text(&manifest_with(valid)).is_ok());

        // IV.1.1 형식(library_id 없음)은 더 이상 받지 않음.
        assert_eq!(
            parse_text(&manifest_with(
                "    source: git\n    repository: https://e/x\n    version: v1\n"
            )),
            Err(ManifestError::MissingField {
                path: "dependencies.lib.library_id".to_string()
            })
        );

        // (바꿀 줄, 바꿀 내용, 기대 오류).
        let cases: Vec<(&str, &str, ManifestError)> = vec![
            (
                "    version: v1\n",
                "    version: v1\n    mirror: x\n",
                ManifestError::UnknownKey {
                    path: "dependencies.lib.mirror".to_string(),
                },
            ),
            (
                "source: git",
                "source: svn",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.source".to_string(),
                    error: ValueError::UnknownSource,
                },
            ),
            (
                "library_id: 0195c530-3acd-7bc1-8414-000000000009",
                "library_id: not-a-uuid",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.library_id".to_string(),
                    error: ValueError::InvalidLibraryId,
                },
            ),
            (
                "repository: https://e/x",
                "repository: \"--upload-pack=touch\"",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.repository".to_string(),
                    error: ValueError::LeadingDash,
                },
            ),
            (
                "version: v1",
                "version:\n      commit: abc1234",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.version.commit".to_string(),
                    error: ValueError::InvalidCommitLength,
                },
            ),
            (
                "version: v1",
                "version:\n      tag: v1\n      branch: main",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.version".to_string(),
                    error: ValueError::InvalidSelector,
                },
            ),
            (
                "    version: v1\n",
                "    version: v1\n    local_override: ./ds\n",
                ManifestError::InvalidValue {
                    path: "dependencies.lib.local_override".to_string(),
                    error: ValueError::InvalidOverridePath,
                },
            ),
        ];
        for (from, to, expected) in cases {
            let entry = valid.replace(from, to);
            let err = parse_text(&manifest_with(&entry)).unwrap_err();
            assert_eq!(err, expected, "entry={entry:?}");
            // 오류 메시지는 URL·경로 원문을 복사하지 않음(XV.1.4).
            let text = err.to_string();
            assert!(
                !text.contains("upload-pack") && !text.contains("./ds"),
                "{text}"
            );
        }
    }

    #[test]
    fn local_override_schema_is_validated_through_stellar_yaml_parser() {
        // XV.6.1: 실제 파서가 읽은 local_override를 logical name → 로컬 root로 매핑.
        let manifest = parse_text(DEPENDENCY_SCHEMA_TEXT).unwrap();
        let project = std::path::Path::new("/work/app");
        let map = manifest.local_overrides(project).unwrap();
        assert_eq!(map.len(), 1);
        let icons = map.get(&LogicalName::new("icons").unwrap()).unwrap();
        assert_eq!(icons.root, std::path::Path::new("/work/icons"));
        assert_eq!(icons.declared.reveal(), "../icons");

        // 사용 프로젝트 안을 가리키는 override는 경로와 함께 거부.
        let inside = DEPENDENCY_SCHEMA_TEXT.replace("\"../icons\"", "\"../app/vendor/icons\"");
        assert_eq!(
            parse_text(&inside).unwrap().local_overrides(project),
            Err(ManifestError::InvalidValue {
                path: "dependencies.icons.local_override".to_string(),
                error: ValueError::OverrideInsideProject,
            })
        );
        // 형식 위반(`\` 구분자, 빈 component, `.` component)은 파싱 단계에서 거부.
        for bad in [r#""..\\icons""#, r#""../a//icons""#, r#""./icons""#] {
            let text = DEPENDENCY_SCHEMA_TEXT.replace("\"../icons\"", bad);
            let err = parse_text(&text).unwrap_err();
            assert!(
                matches!(
                    &err,
                    ManifestError::InvalidValue { path, .. }
                        if path == "dependencies.icons.local_override"
                ),
                "{bad}: {err:?}"
            );
        }
    }

    #[test]
    fn dependency_conflicts_are_reported_from_parsed_manifest() {
        // XV.1.3: 같은 LibraryId 다른 선택자(경고), 다른 LibraryId 같은 URL(오류)을 파싱 결과에서 판정.
        let text = DEPENDENCY_SCHEMA_TEXT
            .replace(
                "0195c530-3acd-7bc1-8414-000000000003",
                "0195c530-3acd-7bc1-8414-000000000001",
            )
            .replace(
                "\"https://example.com/icons.git\"",
                "\"git@github.com:example/design-system.git\"",
            );
        let report = parse_text(&text).unwrap().validate_dependencies();
        let kinds: Vec<ConflictKind> = report.conflicts.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                ConflictKind::DifferentLibrarySameRepository,
                ConflictKind::SameLibraryDifferentSelector
            ]
        );
        assert!(report.has_errors());
        // 충돌 없는 선언은 보고 없음.
        assert!(parse_text(DEPENDENCY_SCHEMA_TEXT)
            .unwrap()
            .validate_dependencies()
            .conflicts
            .is_empty());
    }

    #[test]
    fn parsed_dependency_values_stay_redacted_in_debug_and_diagnostics() {
        // XV.1.4: 파서가 만든 모델의 Debug·진단·telemetry에 URL·override 경로 원문이 없음.
        let manifest = parse_text(DEPENDENCY_SCHEMA_TEXT).unwrap();
        let secrets = ["example.com", "github.com", "/srv/tokens", "../icons"];
        let debug = format!("{manifest:?}");
        for secret in secrets {
            assert!(!debug.contains(secret), "Debug에 {secret} 노출: {debug}");
        }
        for dep in &manifest.dependencies {
            let diagnostic = dep.diagnostic().to_string();
            let telemetry = format!("{:?}", dep.telemetry());
            for secret in secrets {
                assert!(!diagnostic.contains(secret), "{diagnostic}");
                assert!(!telemetry.contains(secret), "{telemetry}");
            }
        }
    }
}
