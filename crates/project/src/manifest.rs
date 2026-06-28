//! `stellar.yaml` 매니페스트 데이터 모델 (IV.1.1).
//!
//! 사용자가 관리하는 프로젝트 설정과 의존성 선언을 표현합니다(기술 아키텍처 20장). 본 모듈은
//! 데이터 구조와 [`YamlValue`] 사이의 변환(`to_yaml`/`from_yaml`)을 정의합니다. 텍스트 파싱은
//! IV.1.2 제한 loader, canonical 기록은 IV.1.5 writer가 담당합니다.
//!
//! 필수 최상위 필드: `schema_version`, `project`, `target_profiles`, `dependencies`.
//! 알 수 없는 키는 거부합니다(제한 스키마).

use crate::yaml::YamlValue;

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

/// 의존성 선언.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// 의존성 논리 이름(매핑 키).
    pub name: String,
    /// source(예: `git`).
    pub source: String,
    /// repository URL.
    pub repository: String,
    /// 버전 조건.
    pub version: String,
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
    /// 의존성(저장 순서 보존).
    pub dependencies: Vec<Dependency>,
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
                YamlValue::Mapping(self.dependencies.iter().map(dependency_to_yaml).collect()),
            ),
        ];
        YamlValue::Mapping(top)
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
            ],
        )?;

        let schema_version = parse_schema_version(require(value, "schema_version")?)?;
        let project = parse_project(require(value, "project")?)?;
        let target_profiles = parse_target_profiles(require(value, "target_profiles")?)?;
        let dependencies = parse_dependencies(require(value, "dependencies")?)?;

        Ok(Manifest {
            schema_version,
            project,
            target_profiles,
            dependencies,
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

fn dependency_to_yaml(dependency: &Dependency) -> (YamlValue, YamlValue) {
    (
        YamlValue::scalar(dependency.name.clone()),
        YamlValue::Mapping(vec![
            (
                YamlValue::scalar("source"),
                YamlValue::scalar(dependency.source.clone()),
            ),
            (
                YamlValue::scalar("repository"),
                YamlValue::scalar(dependency.repository.clone()),
            ),
            (
                YamlValue::scalar("version"),
                YamlValue::scalar(dependency.version.clone()),
            ),
        ]),
    )
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
    // 음이 아닌 정수만 허용(부호·소수점·공백 거부). leading zero(`01`)도 거부하되 `0`은 허용.
    let all_digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let leading_zero = text.len() > 1 && text.starts_with('0');
    if !all_digits || leading_zero {
        return Err(ManifestError::InvalidSchemaVersion {
            value: text.to_string(),
        });
    }
    text.parse::<u32>()
        .map_err(|_| ManifestError::InvalidSchemaVersion {
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
    let pairs = expect_mapping(value, "dependencies")?;
    let mut deps = Vec::with_capacity(pairs.len());
    for (key, spec) in pairs {
        let name = key.as_scalar().ok_or_else(|| ManifestError::WrongType {
            path: "dependencies 키".to_string(),
            expected: "scalar",
        })?;
        let path = join("dependencies", name);
        let spec_pairs = expect_mapping(spec, &path)?;
        reject_unknown_keys(spec_pairs, &path, &["source", "repository", "version"])?;
        let source = expect_scalar(
            spec.get("source")
                .ok_or_else(|| ManifestError::MissingField {
                    path: join(&path, "source"),
                })?,
            &join(&path, "source"),
        )?;
        let repository = expect_scalar(
            spec.get("repository")
                .ok_or_else(|| ManifestError::MissingField {
                    path: join(&path, "repository"),
                })?,
            &join(&path, "repository"),
        )?;
        let version = expect_scalar(
            spec.get("version")
                .ok_or_else(|| ManifestError::MissingField {
                    path: join(&path, "version"),
                })?,
            &join(&path, "version"),
        )?;
        deps.push(Dependency {
            name: name.to_string(),
            source: source.to_string(),
            repository: repository.to_string(),
            version: version.to_string(),
        });
    }
    Ok(deps)
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
mod tests {
    use super::*;

    fn minimal() -> Manifest {
        Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Project Stellar".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
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
            dependencies: vec![Dependency {
                name: "design-system".to_string(),
                source: "git".to_string(),
                repository: "git@github.com:example/design-system.git".to_string(),
                version: "v2.4.0".to_string(),
            }],
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
        assert_eq!(restored.dependencies[0].name, "design-system");
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
}
