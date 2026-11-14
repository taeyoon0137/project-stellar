//! profile 종류별 필드와 [`TargetProfile`] entry (IX.10.2).
//!
//! 기술 아키텍처 17.1의 종류별 필드를 정의하고 `stellar.yaml` `target_profiles[i]` 매핑으로 변환합니다. 종류마다
//! 필수 필드가 다르며, 다른 종류의 필드가 섞이면 [`SchemaError::UnknownKey`]로 거부합니다(조용히 무시하지 않음).
//!
//! | 종류 | 키 | 형식 | 필수 |
//! |---|---|---|---|
//! | Apple | `apple_platform` | [`ApplePlatform`] 이름(`ios`·`ipados`·`macos`·`watchos`·`tvos`·`visionos`) | 예 |
//! | Apple | `minimum_os` | [`DottedVersion`] (`26.0`, major 9999·minor 999 이하) | 예 |
//! | Android | `api_level` | [`AndroidApiLevel`] 10진 정수(1 이상) | 예 |
//! | Android | `ui_system_version` | [`DottedVersion`] | 예 |
//! | Web | `baseline` | [`WebBaseline`] Baseline 연도(2000~9999) | 예 |
//! | Web | `capability_set_version` | [`CapabilitySetVersion`] 10진 정수(1 이상) | 예 |
//!
//! 모든 종류는 profile registry pin `registry_version`([`ProfileRegistryVersion`], IX.10.4)도 필수로 가집니다.
//!
//! canonical 키 순서는 머리(`id`, `platform`, `profile_schema_version`), `registry_version`, 위 표의 종류별 순서입니다.
//! version 텍스트는 정규화합니다(`26` → `26.0`, `17.4.0` → `17.4`).

use std::fmt;

use crate::environment::{MAX_OS_MAJOR, MAX_OS_MINOR};
use crate::node::{
    expect_mapping, invalid, join, parse_u32, reject_unknown_keys, required_scalar, ManifestNode,
    SchemaError, ValueError,
};
use crate::profile::{ProfileHeader, ProfileSchemaVersion, TargetProfileId, TargetProfileKind};
use crate::registry::{ProfileRegistryVersion, KEY_REGISTRY_VERSION};

/// Apple 키: 대상 플랫폼.
const KEY_APPLE_PLATFORM: &str = "apple_platform";
/// Apple 키: 최소 OS 버전.
const KEY_MINIMUM_OS: &str = "minimum_os";
/// Android 키: API level.
const KEY_API_LEVEL: &str = "api_level";
/// Android 키: UI 시스템 버전.
const KEY_UI_SYSTEM_VERSION: &str = "ui_system_version";
/// Web 키: 브라우저 기준선.
const KEY_BASELINE: &str = "baseline";
/// Web 키: capability 집합 버전.
const KEY_CAPABILITY_SET_VERSION: &str = "capability_set_version";

/// `major[.minor[.patch]]` 10진 version(각 성분 `u16`). 비교는 (major, minor, patch) 사전순입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DottedVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl DottedVersion {
    /// 성분으로 만듭니다.
    pub const fn new(major: u16, minor: u16, patch: u16) -> DottedVersion {
        DottedVersion {
            major,
            minor,
            patch,
        }
    }

    /// 텍스트를 엄격히 읽습니다: 1~3개 성분, 각 성분은 선행 0 없는 10진수(`0` 허용), `u16` 범위.
    pub fn parse(text: &str) -> Result<DottedVersion, ValueError> {
        if text.is_empty() {
            return Err(ValueError::Empty);
        }
        let mut parts = [0u16; 3];
        for (index, part) in text.split('.').enumerate() {
            let slot = parts.get_mut(index).ok_or(ValueError::InvalidVersion)?;
            let value = parse_u32(part).map_err(|_| ValueError::InvalidVersion)?;
            *slot = u16::try_from(value).map_err(|_| ValueError::InvalidVersion)?;
        }
        Ok(DottedVersion::new(parts[0], parts[1], parts[2]))
    }

    /// major.
    pub const fn major(self) -> u16 {
        self.major
    }

    /// minor.
    pub const fn minor(self) -> u16 {
        self.minor
    }

    /// patch.
    pub const fn patch(self) -> u16 {
        self.patch
    }
}

impl fmt::Display for DottedVersion {
    /// canonical 텍스트: `major.minor`, patch가 0이 아니면 `major.minor.patch`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.patch == 0 {
            write!(f, "{}.{}", self.major, self.minor)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

/// Apple 대상 플랫폼. code는 안정 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ApplePlatform {
    /// iOS.
    Ios = 1,
    /// iPadOS.
    IpadOs = 2,
    /// macOS.
    MacOs = 3,
    /// watchOS.
    WatchOs = 4,
    /// tvOS.
    TvOs = 5,
    /// visionOS.
    VisionOs = 6,
}

impl ApplePlatform {
    /// 모든 플랫폼(code 오름차순).
    pub const ALL: [ApplePlatform; 6] = [
        ApplePlatform::Ios,
        ApplePlatform::IpadOs,
        ApplePlatform::MacOs,
        ApplePlatform::WatchOs,
        ApplePlatform::TvOs,
        ApplePlatform::VisionOs,
    ];

    /// 안정 numeric code.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// `stellar.yaml` 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ApplePlatform::Ios => "ios",
            ApplePlatform::IpadOs => "ipados",
            ApplePlatform::MacOs => "macos",
            ApplePlatform::WatchOs => "watchos",
            ApplePlatform::TvOs => "tvos",
            ApplePlatform::VisionOs => "visionos",
        }
    }

    /// 이름의 역(대소문자 구분).
    pub fn from_name(name: &str) -> Option<ApplePlatform> {
        ApplePlatform::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// Android API level(1 이상).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AndroidApiLevel(u16);

impl AndroidApiLevel {
    /// 범위(1..=u16::MAX)를 검사해 만듭니다.
    pub const fn new(level: u16) -> Result<AndroidApiLevel, ValueError> {
        if level == 0 {
            Err(ValueError::OutOfRange)
        } else {
            Ok(AndroidApiLevel(level))
        }
    }

    /// 숫자 값.
    pub const fn get(self) -> u16 {
        self.0
    }

    fn parse(text: &str) -> Result<AndroidApiLevel, ValueError> {
        let value = parse_u32(text)?;
        let level = u16::try_from(value).map_err(|_| ValueError::OutOfRange)?;
        AndroidApiLevel::new(level)
    }
}

/// Web 브라우저 기준선(Baseline 연도).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WebBaseline(u16);

impl WebBaseline {
    /// 최소 연도.
    pub const MIN_YEAR: u16 = 2000;
    /// 최대 연도.
    pub const MAX_YEAR: u16 = 9999;

    /// 범위를 검사해 만듭니다.
    pub const fn new(year: u16) -> Result<WebBaseline, ValueError> {
        if year < WebBaseline::MIN_YEAR || year > WebBaseline::MAX_YEAR {
            Err(ValueError::OutOfRange)
        } else {
            Ok(WebBaseline(year))
        }
    }

    /// 연도.
    pub const fn year(self) -> u16 {
        self.0
    }

    fn parse(text: &str) -> Result<WebBaseline, ValueError> {
        let value = parse_u32(text)?;
        let year = u16::try_from(value).map_err(|_| ValueError::OutOfRange)?;
        WebBaseline::new(year)
    }
}

/// Web CSS·접근성 capability 집합 버전(1 이상).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilitySetVersion(u32);

impl CapabilitySetVersion {
    /// 범위(1 이상)를 검사해 만듭니다.
    pub const fn new(value: u32) -> Result<CapabilitySetVersion, ValueError> {
        if value == 0 {
            Err(ValueError::OutOfRange)
        } else {
            Ok(CapabilitySetVersion(value))
        }
    }

    /// 숫자 값.
    pub const fn get(self) -> u32 {
        self.0
    }

    fn parse(text: &str) -> Result<CapabilitySetVersion, ValueError> {
        CapabilitySetVersion::new(parse_u32(text)?)
    }
}

/// Apple profile 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppleProfile {
    /// 대상 플랫폼.
    pub platform: ApplePlatform,
    /// 최소 OS 버전.
    pub minimum_os: DottedVersion,
}

/// Android profile 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AndroidProfile {
    /// API level.
    pub api_level: AndroidApiLevel,
    /// UI 시스템 버전.
    pub ui_system_version: DottedVersion,
}

/// Web profile 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WebProfile {
    /// 브라우저 기준선.
    pub baseline: WebBaseline,
    /// capability 집합 버전.
    pub capability_set_version: CapabilitySetVersion,
}

/// 종류별 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileSpec {
    /// Web.
    Web(WebProfile),
    /// Apple.
    Apple(AppleProfile),
    /// Android.
    Android(AndroidProfile),
}

impl ProfileSpec {
    /// 종류.
    pub const fn kind(&self) -> TargetProfileKind {
        match self {
            ProfileSpec::Web(_) => TargetProfileKind::Web,
            ProfileSpec::Apple(_) => TargetProfileKind::Apple,
            ProfileSpec::Android(_) => TargetProfileKind::Android,
        }
    }

    /// 종류의 필수 키(기록 순서).
    pub const fn keys(kind: TargetProfileKind) -> [&'static str; 2] {
        match kind {
            TargetProfileKind::Web => [KEY_BASELINE, KEY_CAPABILITY_SET_VERSION],
            TargetProfileKind::Apple => [KEY_APPLE_PLATFORM, KEY_MINIMUM_OS],
            TargetProfileKind::Android => [KEY_API_LEVEL, KEY_UI_SYSTEM_VERSION],
        }
    }

    fn to_pairs(self) -> [(String, ManifestNode); 2] {
        let pair = |key: &str, text: String| (key.to_string(), ManifestNode::Scalar(text));
        match self {
            ProfileSpec::Web(web) => [
                pair(KEY_BASELINE, web.baseline.year().to_string()),
                pair(
                    KEY_CAPABILITY_SET_VERSION,
                    web.capability_set_version.get().to_string(),
                ),
            ],
            ProfileSpec::Apple(apple) => [
                pair(KEY_APPLE_PLATFORM, apple.platform.name().to_string()),
                pair(KEY_MINIMUM_OS, apple.minimum_os.to_string()),
            ],
            ProfileSpec::Android(android) => [
                pair(KEY_API_LEVEL, android.api_level.get().to_string()),
                pair(KEY_UI_SYSTEM_VERSION, android.ui_system_version.to_string()),
            ],
        }
    }

    fn read(
        kind: TargetProfileKind,
        node: &ManifestNode,
        path: &str,
    ) -> Result<ProfileSpec, SchemaError> {
        let field = |key: &str| -> Result<(&str, String), SchemaError> {
            Ok((required_scalar(node, path, key)?, join(path, key)))
        };
        Ok(match kind {
            TargetProfileKind::Web => {
                let (text, p) = field(KEY_BASELINE)?;
                let baseline = WebBaseline::parse(text).map_err(|e| invalid(&p, e))?;
                let (text, p) = field(KEY_CAPABILITY_SET_VERSION)?;
                let capability_set_version =
                    CapabilitySetVersion::parse(text).map_err(|e| invalid(&p, e))?;
                ProfileSpec::Web(WebProfile {
                    baseline,
                    capability_set_version,
                })
            }
            TargetProfileKind::Apple => {
                let (text, p) = field(KEY_APPLE_PLATFORM)?;
                let platform = ApplePlatform::from_name(text)
                    .ok_or_else(|| invalid(&p, ValueError::UnknownApplePlatform))?;
                let (text, p) = field(KEY_MINIMUM_OS)?;
                let minimum_os = DottedVersion::parse(text)
                    .and_then(apple_os_in_range)
                    .map_err(|e| invalid(&p, e))?;
                ProfileSpec::Apple(AppleProfile {
                    platform,
                    minimum_os,
                })
            }
            TargetProfileKind::Android => {
                let (text, p) = field(KEY_API_LEVEL)?;
                let api_level = AndroidApiLevel::parse(text).map_err(|e| invalid(&p, e))?;
                let (text, p) = field(KEY_UI_SYSTEM_VERSION)?;
                let ui_system_version = DottedVersion::parse(text).map_err(|e| invalid(&p, e))?;
                ProfileSpec::Android(AndroidProfile {
                    api_level,
                    ui_system_version,
                })
            }
        })
    }
}

/// Apple `minimum_os`는 Binding 환경 `os_version` 인코딩(IX.10.7, [`crate::environment`])이 정확히 담는 범위만
/// 받습니다(major [`MAX_OS_MAJOR`]·minor [`MAX_OS_MINOR`] 이하).
fn apple_os_in_range(version: DottedVersion) -> Result<DottedVersion, ValueError> {
    if version.major() > MAX_OS_MAJOR || version.minor() > MAX_OS_MINOR {
        Err(ValueError::OutOfRange)
    } else {
        Ok(version)
    }
}

/// `stellar.yaml` `target_profiles[i]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetProfile {
    /// 식별자.
    pub id: TargetProfileId,
    /// entry 형식 버전.
    pub schema_version: ProfileSchemaVersion,
    /// 프로젝트가 고정한 profile registry 버전(IX.10.4). 앱 업데이트로 바뀌지 않습니다.
    pub registry_version: ProfileRegistryVersion,
    /// 종류별 필드(종류는 여기서 정해집니다).
    pub spec: ProfileSpec,
}

impl TargetProfile {
    /// 현재 schema version으로 만듭니다.
    pub fn new(
        id: TargetProfileId,
        registry_version: ProfileRegistryVersion,
        spec: ProfileSpec,
    ) -> TargetProfile {
        TargetProfile {
            id,
            schema_version: ProfileSchemaVersion::CURRENT,
            registry_version,
            spec,
        }
    }

    /// 종류.
    pub const fn kind(&self) -> TargetProfileKind {
        self.spec.kind()
    }

    /// 공통 머리.
    pub fn header(&self) -> ProfileHeader {
        ProfileHeader {
            id: self.id.clone(),
            kind: self.kind(),
            schema_version: self.schema_version,
        }
    }

    /// canonical 매핑 노드.
    pub fn to_node(&self) -> ManifestNode {
        let mut pairs = self.header().to_pairs();
        pairs.push((
            KEY_REGISTRY_VERSION.to_string(),
            ManifestNode::scalar(self.registry_version.get().to_string()),
        ));
        pairs.extend(self.spec.to_pairs());
        ManifestNode::Mapping(pairs)
    }

    /// 매핑을 읽습니다. 종류별 필수 필드 누락은 [`SchemaError::MissingField`], 다른 종류 필드·알 수 없는 키는
    /// [`SchemaError::UnknownKey`]입니다.
    pub fn from_node(node: &ManifestNode, path: &str) -> Result<TargetProfile, SchemaError> {
        let header = ProfileHeader::read(node, path)?;
        let pairs = expect_mapping(node, path)?;
        let mut allowed: Vec<&str> = ProfileHeader::KEYS.to_vec();
        allowed.push(KEY_REGISTRY_VERSION);
        allowed.extend(ProfileSpec::keys(header.kind));
        reject_unknown_keys(pairs, path, &allowed)?;
        let pin_path = join(path, KEY_REGISTRY_VERSION);
        let registry_version =
            ProfileRegistryVersion::parse(required_scalar(node, path, KEY_REGISTRY_VERSION)?)
                .map_err(|e| invalid(&pin_path, e))?;
        let spec = ProfileSpec::read(header.kind, node, path)?;
        Ok(TargetProfile {
            id: header.id,
            schema_version: header.schema_version,
            registry_version,
            spec,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("유효한 id")
    }

    fn mapping(pairs: &[(&str, &str)]) -> ManifestNode {
        ManifestNode::Mapping(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), ManifestNode::scalar(*v)))
                .collect(),
        )
    }

    fn v1() -> ProfileRegistryVersion {
        ProfileRegistryVersion::new(1).expect("v1")
    }

    fn samples() -> [TargetProfile; 3] {
        [
            TargetProfile::new(
                id("apple-desktop"),
                v1(),
                ProfileSpec::Apple(AppleProfile {
                    platform: ApplePlatform::MacOs,
                    minimum_os: DottedVersion::new(26, 0, 0),
                }),
            ),
            TargetProfile::new(
                id("android-phone"),
                v1(),
                ProfileSpec::Android(AndroidProfile {
                    api_level: AndroidApiLevel::new(35).expect("api"),
                    ui_system_version: DottedVersion::new(1, 7, 0),
                }),
            ),
            TargetProfile::new(
                id("web-modern"),
                v1(),
                ProfileSpec::Web(WebProfile {
                    baseline: WebBaseline::new(2024).expect("baseline"),
                    capability_set_version: CapabilitySetVersion::new(1).expect("set"),
                }),
            ),
        ]
    }

    #[test]
    fn every_kind_round_trips_with_canonical_keys() {
        let [apple, android, web] = samples();
        assert_eq!(
            apple.to_node(),
            mapping(&[
                ("id", "apple-desktop"),
                ("platform", "apple"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("apple_platform", "macos"),
                ("minimum_os", "26.0"),
            ])
        );
        assert_eq!(
            android.to_node(),
            mapping(&[
                ("id", "android-phone"),
                ("platform", "android"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("api_level", "35"),
                ("ui_system_version", "1.7"),
            ])
        );
        assert_eq!(
            web.to_node(),
            mapping(&[
                ("id", "web-modern"),
                ("platform", "web"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("baseline", "2024"),
                ("capability_set_version", "1"),
            ])
        );
        for profile in samples() {
            let node = profile.to_node();
            let restored = TargetProfile::from_node(&node, "p").expect("round-trip");
            assert_eq!(restored, profile);
            assert_eq!(restored.to_node(), node);
        }
    }

    #[test]
    fn required_fields_are_validated_per_kind() {
        let header = |kind: &'static str| {
            vec![
                ("id", "p1"),
                ("platform", kind),
                ("profile_schema_version", "1"),
                ("registry_version", "3"),
            ]
        };
        let cases: [(&str, [(&str, &str); 2]); 3] = [
            ("apple", [("apple_platform", "ios"), ("minimum_os", "17.4")]),
            (
                "android",
                [("api_level", "34"), ("ui_system_version", "1.6")],
            ),
            (
                "web",
                [("baseline", "2023"), ("capability_set_version", "2")],
            ),
        ];
        for (kind, fields) in cases {
            let mut full = header(kind);
            full.extend(fields);
            assert!(
                TargetProfile::from_node(&mapping(&full), "p").is_ok(),
                "{kind}"
            );
            // registry pin은 모든 종류에서 필수이고 1 이상입니다.
            let unpinned: Vec<_> = full
                .iter()
                .copied()
                .filter(|(k, _)| *k != "registry_version")
                .collect();
            assert_eq!(
                TargetProfile::from_node(&mapping(&unpinned), "p"),
                Err(SchemaError::MissingField {
                    path: "p.registry_version".to_string()
                }),
                "{kind}"
            );
            let zero: Vec<_> = full
                .iter()
                .map(|&(k, v)| {
                    if k == "registry_version" {
                        (k, "0")
                    } else {
                        (k, v)
                    }
                })
                .collect();
            assert_eq!(
                TargetProfile::from_node(&mapping(&zero), "p"),
                Err(SchemaError::InvalidValue {
                    path: "p.registry_version".to_string(),
                    error: ValueError::OutOfRange
                }),
                "{kind}"
            );
            // 종류별 필수 필드 하나씩 빼기.
            for missing in fields {
                let pairs: Vec<_> = full.iter().copied().filter(|p| *p != missing).collect();
                assert_eq!(
                    TargetProfile::from_node(&mapping(&pairs), "p"),
                    Err(SchemaError::MissingField {
                        path: format!("p.{}", missing.0)
                    }),
                    "{kind}: {}",
                    missing.0
                );
            }
            // 다른 종류의 필드는 거부합니다.
            for (other, other_fields) in cases {
                if other == kind {
                    continue;
                }
                let mut mixed = full.clone();
                mixed.push(other_fields[0]);
                assert_eq!(
                    TargetProfile::from_node(&mapping(&mixed), "p"),
                    Err(SchemaError::UnknownKey {
                        path: format!("p.{}", other_fields[0].0)
                    }),
                    "{kind} + {other}"
                );
            }
        }
    }

    #[test]
    fn field_values_are_validated() {
        let apple = |platform: &str, os: &str| {
            mapping(&[
                ("id", "a"),
                ("platform", "apple"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("apple_platform", platform),
                ("minimum_os", os),
            ])
        };
        let err = |path: &str, error: ValueError| {
            Err(SchemaError::InvalidValue {
                path: path.to_string(),
                error,
            })
        };
        assert_eq!(
            TargetProfile::from_node(&apple("iOS", "17"), "p"),
            err("p.apple_platform", ValueError::UnknownApplePlatform)
        );
        for bad in ["", "17.", ".1", "17.04", "1.2.3.4", "65536", "v17", "17-1"] {
            let expected = if bad.is_empty() {
                ValueError::Empty
            } else {
                ValueError::InvalidVersion
            };
            assert_eq!(
                TargetProfile::from_node(&apple("ios", bad), "p"),
                err("p.minimum_os", expected),
                "{bad:?}"
            );
        }
        // Binding 환경 `os_version` 인코딩(IX.10.7) 범위: major 9999·minor 999 이하.
        for bad in ["17.1000", "10000", "10000.0", "65535.65535"] {
            assert_eq!(
                TargetProfile::from_node(&apple("ios", bad), "p"),
                err("p.minimum_os", ValueError::OutOfRange),
                "{bad:?}"
            );
        }
        for good in ["9999.999", "17.999", "0", "10.15"] {
            assert!(
                TargetProfile::from_node(&apple("ios", good), "p").is_ok(),
                "{good:?}"
            );
        }
        // 정규화: `17` → `17.0`, `17.4.0` → `17.4`, patch 보존.
        for (text, canonical) in [("17", "17.0"), ("17.4.0", "17.4"), ("15.0.1", "15.0.1")] {
            let profile = TargetProfile::from_node(&apple("ios", text), "p").expect("유효");
            assert_eq!(
                profile.to_node().get("minimum_os"),
                Some(&ManifestNode::scalar(canonical))
            );
        }
        let android = |api: &str| {
            mapping(&[
                ("id", "a"),
                ("platform", "android"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("api_level", api),
                ("ui_system_version", "1.0"),
            ])
        };
        assert_eq!(
            TargetProfile::from_node(&android("0"), "p"),
            err("p.api_level", ValueError::OutOfRange)
        );
        assert_eq!(
            TargetProfile::from_node(&android("65536"), "p"),
            err("p.api_level", ValueError::OutOfRange)
        );
        assert_eq!(
            TargetProfile::from_node(&android("035"), "p"),
            err("p.api_level", ValueError::InvalidInteger)
        );
        let web = |baseline: &str, set: &str| {
            mapping(&[
                ("id", "a"),
                ("platform", "web"),
                ("profile_schema_version", "1"),
                ("registry_version", "1"),
                ("baseline", baseline),
                ("capability_set_version", set),
            ])
        };
        assert_eq!(
            TargetProfile::from_node(&web("1999", "1"), "p"),
            err("p.baseline", ValueError::OutOfRange)
        );
        assert_eq!(
            TargetProfile::from_node(&web("modern", "1"), "p"),
            err("p.baseline", ValueError::InvalidInteger)
        );
        assert_eq!(
            TargetProfile::from_node(&web("2024", "0"), "p"),
            err("p.capability_set_version", ValueError::OutOfRange)
        );
    }

    #[test]
    fn version_ordering_and_platform_codes() {
        assert!(DottedVersion::new(17, 4, 0) < DottedVersion::new(17, 10, 0));
        assert!(DottedVersion::new(26, 0, 0) > DottedVersion::new(18, 6, 2));
        assert_eq!(DottedVersion::parse("0"), Ok(DottedVersion::new(0, 0, 0)));
        let codes: Vec<u8> = ApplePlatform::ALL.iter().map(|p| p.code()).collect();
        assert_eq!(codes, vec![1, 2, 3, 4, 5, 6]);
        for platform in ApplePlatform::ALL {
            assert_eq!(ApplePlatform::from_name(platform.name()), Some(platform));
        }
    }
}
