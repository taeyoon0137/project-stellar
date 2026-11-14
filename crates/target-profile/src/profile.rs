//! Target Profile 식별자, 종류, profile schema version (IX.10.1).
//!
//! - [`TargetProfileId`]: 프로젝트 안에서 profile을 가리키는 사람이 읽는 slug(`stellar.yaml`
//!   `target_profiles[i].id`, 예: `apple-desktop`). ASCII 영숫자로 시작하고 영숫자·`-`·`_`·`.`만, 최대
//!   [`TargetProfileId::MAX_LEN`] byte입니다.
//! - [`TargetProfileKind`]: Web·Apple·Android(기술 아키텍처 17.1). numeric code는 표현식 환경 key `platform`의
//!   variant 번호(VIII.7.1: Web=1, Apple=2, Android=3)와 같으며 저장·지문 계약이라 바꾸거나 재사용하지 않습니다.
//!   `stellar.yaml`에는 이름(`web`·`apple`·`android`)으로 씁니다(기존 `platform` 키).
//! - [`ProfileSchemaVersion`]: profile entry 형식의 버전입니다. 이 build가 읽을 수 있는 최대값은
//!   [`ProfileSchemaVersion::CURRENT`]이고 더 새 값은 조용히 해석하지 않고 거부합니다.
//!
//! | code | 종류 | 이름 |
//! |---|---|---|
//! | 1 | Web | `web` |
//! | 2 | Apple | `apple` |
//! | 3 | Android | `android` |

use std::fmt;

use crate::node::{
    expect_mapping, invalid, join, parse_u32, reject_unknown_keys, required_scalar, ManifestNode,
    SchemaError, ValueError,
};

/// profile entry 키: id.
pub(crate) const KEY_ID: &str = "id";
/// profile entry 키: 종류(기존 `stellar.yaml` 키 이름 유지).
pub(crate) const KEY_PLATFORM: &str = "platform";
/// profile entry 키: profile schema version.
pub(crate) const KEY_SCHEMA_VERSION: &str = "profile_schema_version";

/// 프로젝트 안의 Target Profile 식별자.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetProfileId(String);

impl TargetProfileId {
    /// 최대 byte 길이.
    pub const MAX_LEN: usize = 64;

    /// 규칙을 검사해 만듭니다. 오류는 입력 값을 담지 않습니다.
    pub fn new(text: &str) -> Result<TargetProfileId, ValueError> {
        let bytes = text.as_bytes();
        let Some(first) = bytes.first() else {
            return Err(ValueError::Empty);
        };
        if bytes.len() > TargetProfileId::MAX_LEN {
            return Err(ValueError::TooLong {
                max: TargetProfileId::MAX_LEN,
            });
        }
        let allowed = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.');
        if !first.is_ascii_alphanumeric() || !bytes.iter().all(allowed) {
            return Err(ValueError::InvalidCharacter);
        }
        Ok(TargetProfileId(text.to_string()))
    }

    /// 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Target Profile 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TargetProfileKind {
    /// Web(브라우저 기준선, CSS·접근성 Capability 집합).
    Web = 1,
    /// Apple(대상 플랫폼, 최소 OS 버전).
    Apple = 2,
    /// Android(API Level, UI 시스템 버전).
    Android = 3,
}

impl TargetProfileKind {
    /// 모든 종류(code 오름차순).
    pub const ALL: [TargetProfileKind; 3] = [
        TargetProfileKind::Web,
        TargetProfileKind::Apple,
        TargetProfileKind::Android,
    ];

    /// 안정 numeric code.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// code의 역. 미정의 code는 `None`.
    pub const fn from_code(code: u8) -> Option<TargetProfileKind> {
        match code {
            1 => Some(TargetProfileKind::Web),
            2 => Some(TargetProfileKind::Apple),
            3 => Some(TargetProfileKind::Android),
            _ => None,
        }
    }

    /// `stellar.yaml` 이름.
    pub const fn name(self) -> &'static str {
        match self {
            TargetProfileKind::Web => "web",
            TargetProfileKind::Apple => "apple",
            TargetProfileKind::Android => "android",
        }
    }

    /// 이름의 역(대소문자 구분).
    pub fn from_name(name: &str) -> Option<TargetProfileKind> {
        TargetProfileKind::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

impl fmt::Display for TargetProfileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// profile entry 형식 버전.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileSchemaVersion(u32);

impl ProfileSchemaVersion {
    /// 이 build가 기록·해석하는 버전.
    pub const CURRENT: ProfileSchemaVersion = ProfileSchemaVersion(1);

    /// 지원 범위(1..=CURRENT)를 검사해 만듭니다.
    pub const fn new(value: u32) -> Result<ProfileSchemaVersion, ValueError> {
        if value == 0 || value > ProfileSchemaVersion::CURRENT.0 {
            Err(ValueError::UnsupportedSchemaVersion)
        } else {
            Ok(ProfileSchemaVersion(value))
        }
    }

    /// 숫자 값.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// profile entry의 공통 머리(id·종류·schema version).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileHeader {
    /// 식별자.
    pub id: TargetProfileId,
    /// 종류.
    pub kind: TargetProfileKind,
    /// entry 형식 버전.
    pub schema_version: ProfileSchemaVersion,
}

impl ProfileHeader {
    /// 머리 키 목록(기록 순서).
    pub const KEYS: [&'static str; 3] = [KEY_ID, KEY_PLATFORM, KEY_SCHEMA_VERSION];

    /// 머리 필드를 canonical 순서(`id`, `platform`, `profile_schema_version`)의 매핑 쌍으로 만듭니다.
    pub fn to_pairs(&self) -> Vec<(String, ManifestNode)> {
        vec![
            (KEY_ID.to_string(), ManifestNode::scalar(self.id.as_str())),
            (
                KEY_PLATFORM.to_string(),
                ManifestNode::scalar(self.kind.name()),
            ),
            (
                KEY_SCHEMA_VERSION.to_string(),
                ManifestNode::scalar(self.schema_version.get().to_string()),
            ),
        ]
    }

    /// 머리만 담은 매핑 노드.
    pub fn to_node(&self) -> ManifestNode {
        ManifestNode::Mapping(self.to_pairs())
    }

    /// 매핑에서 머리 필드를 읽습니다. 다른 키는 검사하지 않습니다(종류별 필드는 호출자가 검사).
    pub fn read(node: &ManifestNode, path: &str) -> Result<ProfileHeader, SchemaError> {
        expect_mapping(node, path)?;
        let id_path = join(path, KEY_ID);
        let id = TargetProfileId::new(required_scalar(node, path, KEY_ID)?)
            .map_err(|e| invalid(&id_path, e))?;
        let kind_path = join(path, KEY_PLATFORM);
        let kind = TargetProfileKind::from_name(required_scalar(node, path, KEY_PLATFORM)?)
            .ok_or_else(|| invalid(&kind_path, ValueError::UnknownKind))?;
        let version_path = join(path, KEY_SCHEMA_VERSION);
        let version = parse_u32(required_scalar(node, path, KEY_SCHEMA_VERSION)?)
            .and_then(ProfileSchemaVersion::new)
            .map_err(|e| invalid(&version_path, e))?;
        Ok(ProfileHeader {
            id,
            kind,
            schema_version: version,
        })
    }

    /// 머리만 담은 매핑을 읽습니다(머리 키 외에는 [`SchemaError::UnknownKey`]).
    pub fn from_node(node: &ManifestNode, path: &str) -> Result<ProfileHeader, SchemaError> {
        let header = ProfileHeader::read(node, path)?;
        reject_unknown_keys(expect_mapping(node, path)?, path, &ProfileHeader::KEYS)?;
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("유효한 id")
    }

    #[test]
    fn kind_numeric_codes_are_golden() {
        // 환경 key `platform` variant 번호(VIII.7.1)와 같은 값입니다. 바꾸지 않습니다.
        let golden: [(TargetProfileKind, u8, &str); 3] = [
            (TargetProfileKind::Web, 1, "web"),
            (TargetProfileKind::Apple, 2, "apple"),
            (TargetProfileKind::Android, 3, "android"),
        ];
        for (kind, code, name) in golden {
            assert_eq!(kind.code(), code);
            assert_eq!(TargetProfileKind::from_code(code), Some(kind));
            assert_eq!(kind.name(), name);
            assert_eq!(TargetProfileKind::from_name(name), Some(kind));
        }
        assert_eq!(TargetProfileKind::from_code(0), None);
        assert_eq!(TargetProfileKind::from_code(4), None);
        assert_eq!(TargetProfileKind::from_name("Apple"), None);
        assert_eq!(TargetProfileKind::from_name("ios"), None);
        assert_eq!(ProfileSchemaVersion::CURRENT.get(), 1);
    }

    #[test]
    fn header_round_trip_is_golden() {
        let header = ProfileHeader {
            id: id("apple-desktop"),
            kind: TargetProfileKind::Apple,
            schema_version: ProfileSchemaVersion::CURRENT,
        };
        let node = header.to_node();
        let golden = ManifestNode::Mapping(vec![
            ("id".to_string(), ManifestNode::scalar("apple-desktop")),
            ("platform".to_string(), ManifestNode::scalar("apple")),
            (
                "profile_schema_version".to_string(),
                ManifestNode::scalar("1"),
            ),
        ]);
        assert_eq!(node, golden);
        assert_eq!(ProfileHeader::from_node(&node, "p"), Ok(header.clone()));
        // 노드 → 모델 → 노드 동일.
        let reparsed = ProfileHeader::from_node(&golden, "p").expect("golden 읽기");
        assert_eq!(reparsed.to_node(), golden);
        // 모든 종류 round-trip.
        for kind in TargetProfileKind::ALL {
            let h = ProfileHeader {
                id: id("p1"),
                kind,
                schema_version: ProfileSchemaVersion::CURRENT,
            };
            assert_eq!(ProfileHeader::from_node(&h.to_node(), "p"), Ok(h));
        }
    }

    #[test]
    fn id_rules() {
        for ok in [
            "a",
            "apple-desktop",
            "web_2024",
            "android.phone",
            "010",
            "A1",
        ] {
            assert!(TargetProfileId::new(ok).is_ok(), "{ok}");
        }
        assert_eq!(TargetProfileId::new(""), Err(ValueError::Empty));
        for bad in ["-a", ".a", "a b", "a/b", "é", "a\n", "_a"] {
            assert_eq!(
                TargetProfileId::new(bad),
                Err(ValueError::InvalidCharacter),
                "{bad:?}"
            );
        }
        let long = "a".repeat(TargetProfileId::MAX_LEN + 1);
        assert_eq!(
            TargetProfileId::new(&long),
            Err(ValueError::TooLong { max: 64 })
        );
        assert!(TargetProfileId::new(&"a".repeat(TargetProfileId::MAX_LEN)).is_ok());
    }

    #[test]
    fn header_violations_are_rejected_with_paths() {
        let with = |pairs: &[(&str, &str)]| {
            ManifestNode::Mapping(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), ManifestNode::scalar(*v)))
                    .collect(),
            )
        };
        let base = [
            ("id", "web"),
            ("platform", "web"),
            ("profile_schema_version", "1"),
        ];
        assert!(ProfileHeader::from_node(&with(&base), "p").is_ok());
        assert_eq!(
            ProfileHeader::from_node(&with(&base[1..]), "p"),
            Err(SchemaError::MissingField {
                path: "p.id".to_string()
            })
        );
        assert_eq!(
            ProfileHeader::from_node(&with(&[base[0], base[1]]), "p"),
            Err(SchemaError::MissingField {
                path: "p.profile_schema_version".to_string()
            })
        );
        let mut unknown_kind = base;
        unknown_kind[1] = ("platform", "windows");
        assert_eq!(
            ProfileHeader::from_node(&with(&unknown_kind), "p"),
            Err(SchemaError::InvalidValue {
                path: "p.platform".to_string(),
                error: ValueError::UnknownKind
            })
        );
        for (version, error) in [
            ("0", ValueError::UnsupportedSchemaVersion),
            ("2", ValueError::UnsupportedSchemaVersion),
            ("01", ValueError::InvalidInteger),
            ("v1", ValueError::InvalidInteger),
        ] {
            let mut bad = base;
            bad[2] = ("profile_schema_version", version);
            assert_eq!(
                ProfileHeader::from_node(&with(&bad), "p"),
                Err(SchemaError::InvalidValue {
                    path: "p.profile_schema_version".to_string(),
                    error
                }),
                "{version}"
            );
        }
        let mut extra = base.to_vec();
        extra.push(("minimum_os", "26.0"));
        assert_eq!(
            ProfileHeader::from_node(&with(&extra), "p"),
            Err(SchemaError::UnknownKey {
                path: "p.minimum_os".to_string()
            })
        );
        assert_eq!(
            ProfileHeader::from_node(&ManifestNode::scalar("x"), "p"),
            Err(SchemaError::WrongType {
                path: "p".to_string(),
                expected: "mapping"
            })
        );
        let nested = ManifestNode::Mapping(vec![
            (
                "id".to_string(),
                ManifestNode::Sequence(vec![ManifestNode::scalar("a")]),
            ),
            ("platform".to_string(), ManifestNode::scalar("web")),
            (
                "profile_schema_version".to_string(),
                ManifestNode::scalar("1"),
            ),
        ]);
        assert_eq!(
            ProfileHeader::from_node(&nested, "p"),
            Err(SchemaError::WrongType {
                path: "p.id".to_string(),
                expected: "scalar"
            })
        );
    }
}
