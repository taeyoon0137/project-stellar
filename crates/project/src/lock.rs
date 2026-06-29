//! `stellar.lock.yaml` 잠금 파일 데이터 모델 (IV.1.6).
//!
//! 매니페스트([`crate::manifest`])와 **별도**로, resolver가 확정한 의존성 해석 결과를
//! 표현합니다. 각 resolved dependency는 logical ID, source, exact commit, content hash,
//! schema version, dependency path를 포함합니다. `resolved`는 **시퀀스**라 같은 라이브러리가
//! 서로 다른 경로/버전으로 여러 번 등장할 수 있습니다(다중 버전).
//!
//! 잠금 파일은 전부 resolver 결과에서 생성됩니다(사용자 편집 필드 금지는 IV.1.7).

use crate::yaml::scalar;
use crate::yaml::YamlValue;

/// 현재 잠금 파일 스키마 버전.
pub const CURRENT_LOCK_SCHEMA_VERSION: u32 = 1;

/// 잠금 파일 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// 필수 필드 누락.
    MissingField {
        /// 누락된 필드 경로.
        path: String,
    },
    /// 타입 불일치.
    WrongType {
        /// 필드 경로.
        path: String,
        /// 기대 종류(`mapping`/`sequence`/`scalar`).
        expected: &'static str,
    },
    /// schema version이 음이 아닌 정수가 아님.
    InvalidSchemaVersion {
        /// 필드 경로.
        path: String,
        /// 원본 값.
        value: String,
    },
    /// 알 수 없는 키(제한 스키마).
    UnknownKey {
        /// 키 경로.
        path: String,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::MissingField { path } => write!(f, "필수 필드 누락: {path}"),
            LockError::WrongType { path, expected } => {
                write!(f, "타입 불일치: {path}는 {expected}여야 합니다")
            }
            LockError::InvalidSchemaVersion { path, value } => {
                write!(f, "{path}가 음이 아닌 정수가 아닙니다: {value}")
            }
            LockError::UnknownKey { path } => write!(f, "알 수 없는 키: {path}"),
        }
    }
}

impl std::error::Error for LockError {}

/// 해석된 의존성 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDependency {
    /// 의존성 논리 ID.
    pub logical_id: String,
    /// source(예: `git`).
    pub source: String,
    /// 정확한 commit 해시.
    pub exact_commit: String,
    /// content 해시(예: `sha256:...`).
    pub content_hash: String,
    /// 해석된 라이브러리의 매니페스트 schema version.
    pub schema_version: u32,
    /// 루트에서 이 의존성까지의 logical ID 경로.
    pub dependency_path: Vec<String>,
}

/// `stellar.lock.yaml` 잠금 파일.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    /// 잠금 파일 스키마 버전.
    pub schema_version: u32,
    /// 해석된 의존성(저장 순서 보존, 같은 라이브러리 다중 등장 가능).
    pub resolved: Vec<ResolvedDependency>,
}

impl Lockfile {
    /// 잠금 파일을 [`YamlValue`] 트리로 변환합니다(키 순서 고정).
    pub fn to_yaml(&self) -> YamlValue {
        YamlValue::Mapping(vec![
            (
                YamlValue::scalar("schema_version"),
                YamlValue::scalar(self.schema_version.to_string()),
            ),
            (
                YamlValue::scalar("resolved"),
                YamlValue::Sequence(self.resolved.iter().map(resolved_to_yaml).collect()),
            ),
        ])
    }

    /// [`YamlValue`] 트리에서 잠금 파일을 만듭니다. 알 수 없는 키·타입 불일치를 거부합니다.
    pub fn from_yaml(value: &YamlValue) -> Result<Lockfile, LockError> {
        let top = mapping_of(value, "<root>")?;
        reject_unknown(top, "<root>", &["schema_version", "resolved"])?;

        let schema_version = parse_version(require(value, "schema_version")?, "schema_version")?;
        let resolved_value = require(value, "resolved")?;
        let items = resolved_value
            .as_sequence()
            .ok_or_else(|| LockError::WrongType {
                path: "resolved".to_string(),
                expected: "sequence",
            })?;
        let mut resolved = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            resolved.push(parse_resolved(item, index)?);
        }

        Ok(Lockfile {
            schema_version,
            resolved,
        })
    }
}

fn resolved_to_yaml(dep: &ResolvedDependency) -> YamlValue {
    YamlValue::Mapping(vec![
        (
            YamlValue::scalar("logical_id"),
            YamlValue::scalar(dep.logical_id.clone()),
        ),
        (
            YamlValue::scalar("source"),
            YamlValue::scalar(dep.source.clone()),
        ),
        (
            YamlValue::scalar("exact_commit"),
            YamlValue::scalar(dep.exact_commit.clone()),
        ),
        (
            YamlValue::scalar("content_hash"),
            YamlValue::scalar(dep.content_hash.clone()),
        ),
        (
            YamlValue::scalar("schema_version"),
            YamlValue::scalar(dep.schema_version.to_string()),
        ),
        (
            YamlValue::scalar("dependency_path"),
            YamlValue::Sequence(dep.dependency_path.iter().map(YamlValue::scalar).collect()),
        ),
    ])
}

fn parse_resolved(item: &YamlValue, index: usize) -> Result<ResolvedDependency, LockError> {
    let path = format!("resolved[{index}]");
    let pairs = mapping_of(item, &path)?;
    reject_unknown(
        pairs,
        &path,
        &[
            "logical_id",
            "source",
            "exact_commit",
            "content_hash",
            "schema_version",
            "dependency_path",
        ],
    )?;

    let logical_id = scalar_field(item, "logical_id", &path)?;
    let source = scalar_field(item, "source", &path)?;
    let exact_commit = scalar_field(item, "exact_commit", &path)?;
    let content_hash = scalar_field(item, "content_hash", &path)?;
    let schema_version = parse_version(
        field(item, "schema_version", &path)?,
        &join(&path, "schema_version"),
    )?;

    let dep_path_value = field(item, "dependency_path", &path)?;
    let dep_path_items = dep_path_value
        .as_sequence()
        .ok_or_else(|| LockError::WrongType {
            path: join(&path, "dependency_path"),
            expected: "sequence",
        })?;
    let mut dependency_path = Vec::with_capacity(dep_path_items.len());
    for (i, segment) in dep_path_items.iter().enumerate() {
        let seg_path = format!("{path}.dependency_path[{i}]");
        dependency_path.push(
            segment
                .as_scalar()
                .ok_or(LockError::WrongType {
                    path: seg_path,
                    expected: "scalar",
                })?
                .to_string(),
        );
    }

    Ok(ResolvedDependency {
        logical_id,
        source,
        exact_commit,
        content_hash,
        schema_version,
        dependency_path,
    })
}

// --- 트리 접근 헬퍼(LockError 전용) ---

fn require<'a>(value: &'a YamlValue, key: &str) -> Result<&'a YamlValue, LockError> {
    value.get(key).ok_or_else(|| LockError::MissingField {
        path: key.to_string(),
    })
}

fn field<'a>(item: &'a YamlValue, key: &str, parent: &str) -> Result<&'a YamlValue, LockError> {
    item.get(key).ok_or_else(|| LockError::MissingField {
        path: join(parent, key),
    })
}

fn scalar_field(item: &YamlValue, key: &str, parent: &str) -> Result<String, LockError> {
    let path = join(parent, key);
    Ok(field(item, key, parent)?
        .as_scalar()
        .ok_or(LockError::WrongType {
            path,
            expected: "scalar",
        })?
        .to_string())
}

fn mapping_of<'a>(
    value: &'a YamlValue,
    path: &str,
) -> Result<&'a [(YamlValue, YamlValue)], LockError> {
    value.as_mapping().ok_or_else(|| LockError::WrongType {
        path: path.to_string(),
        expected: "mapping",
    })
}

fn parse_version(value: &YamlValue, path: &str) -> Result<u32, LockError> {
    let text = value.as_scalar().ok_or_else(|| LockError::WrongType {
        path: path.to_string(),
        expected: "scalar",
    })?;
    scalar::parse_u32(text).ok_or_else(|| LockError::InvalidSchemaVersion {
        path: path.to_string(),
        value: text.to_string(),
    })
}

fn reject_unknown(
    pairs: &[(YamlValue, YamlValue)],
    parent: &str,
    allowed: &[&str],
) -> Result<(), LockError> {
    for (key, _) in pairs {
        let Some(name) = key.as_scalar() else {
            return Err(LockError::WrongType {
                path: format!("{parent} 키"),
                expected: "scalar",
            });
        };
        if !allowed.contains(&name) {
            return Err(LockError::UnknownKey {
                path: join(parent, name),
            });
        }
    }
    Ok(())
}

fn join(parent: &str, child: &str) -> String {
    format!("{parent}.{child}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::{load_restricted, write_canonical};

    fn dep(logical_id: &str, commit: &str, path: &[&str]) -> ResolvedDependency {
        ResolvedDependency {
            logical_id: logical_id.to_string(),
            source: "git".to_string(),
            exact_commit: commit.to_string(),
            content_hash: format!("sha256:{commit}"),
            schema_version: 1,
            dependency_path: path.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 같은 라이브러리(design-system)가 서로 다른 경로/commit으로 두 번 등장.
    fn multi_version_lock() -> Lockfile {
        Lockfile {
            schema_version: CURRENT_LOCK_SCHEMA_VERSION,
            resolved: vec![
                dep("design-system", "aaaa1111", &["root", "design-system"]),
                dep(
                    "design-system",
                    "bbbb2222",
                    &["root", "icons", "design-system"],
                ),
                dep("icons", "cccc3333", &["root", "icons"]),
            ],
        }
    }

    #[test]
    fn multi_version_same_library_round_trips() {
        let lock = multi_version_lock();
        let restored = Lockfile::from_yaml(&lock.to_yaml()).unwrap();
        assert_eq!(restored, lock);
        // 같은 logical_id가 두 번, 서로 다른 commit·경로로 보존.
        let ds: Vec<&ResolvedDependency> = restored
            .resolved
            .iter()
            .filter(|d| d.logical_id == "design-system")
            .collect();
        assert_eq!(ds.len(), 2);
        assert_ne!(ds[0].exact_commit, ds[1].exact_commit);
        assert_ne!(ds[0].dependency_path, ds[1].dependency_path);
    }

    #[test]
    fn lock_text_round_trips_through_writer_and_loader() {
        let lock = multi_version_lock();
        let text = write_canonical(&lock.to_yaml());
        let value = load_restricted(&text).unwrap();
        let restored = Lockfile::from_yaml(&value).unwrap();
        assert_eq!(restored, lock);
    }

    #[test]
    fn empty_resolved_round_trips() {
        let lock = Lockfile {
            schema_version: 1,
            resolved: Vec::new(),
        };
        assert_eq!(Lockfile::from_yaml(&lock.to_yaml()).unwrap(), lock);
    }

    #[test]
    fn rejects_unknown_key() {
        let mut yaml = multi_version_lock().to_yaml();
        if let YamlValue::Mapping(pairs) = &mut yaml {
            pairs.push((YamlValue::scalar("extra"), YamlValue::scalar("x")));
        }
        assert_eq!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::UnknownKey {
                path: "<root>.extra".to_string()
            })
        );
    }

    #[test]
    fn rejects_unknown_resolved_entry_key() {
        let yaml = YamlValue::Mapping(vec![
            (YamlValue::scalar("schema_version"), YamlValue::scalar("1")),
            (
                YamlValue::scalar("resolved"),
                YamlValue::Sequence(vec![YamlValue::Mapping(vec![
                    (YamlValue::scalar("logical_id"), YamlValue::scalar("x")),
                    (YamlValue::scalar("oops"), YamlValue::scalar("y")),
                ])]),
            ),
        ]);
        assert!(matches!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::UnknownKey { .. })
        ));
    }

    #[test]
    fn rejects_missing_field_in_resolved() {
        let yaml = YamlValue::Mapping(vec![
            (YamlValue::scalar("schema_version"), YamlValue::scalar("1")),
            (
                YamlValue::scalar("resolved"),
                YamlValue::Sequence(vec![YamlValue::Mapping(vec![(
                    YamlValue::scalar("logical_id"),
                    YamlValue::scalar("x"),
                )])]),
            ),
        ]);
        assert!(matches!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::MissingField { .. })
        ));
    }

    #[test]
    fn rejects_invalid_schema_version() {
        // 잠금 파일 schema_version에 leading-zero.
        let yaml = YamlValue::Mapping(vec![
            (YamlValue::scalar("schema_version"), YamlValue::scalar("01")),
            (YamlValue::scalar("resolved"), YamlValue::Sequence(vec![])),
        ]);
        assert!(matches!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::InvalidSchemaVersion { .. })
        ));
    }

    #[test]
    fn rejects_wrong_type_for_resolved() {
        let yaml = YamlValue::Mapping(vec![
            (YamlValue::scalar("schema_version"), YamlValue::scalar("1")),
            (YamlValue::scalar("resolved"), YamlValue::scalar("oops")),
        ]);
        assert_eq!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::WrongType {
                path: "resolved".to_string(),
                expected: "sequence"
            })
        );
    }
}
