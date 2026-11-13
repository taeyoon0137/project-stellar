//! 환경 입력 key registry (VIII.7.1).
//!
//! 표현식의 `ReferenceTarget::Environment(EnvironmentKey)`가 가리키는 환경 입력의 numeric ID와 값 타입을 고정합니다.
//! numeric ID는 저장 형식(canonical AST의 environment key, VIII.2.5)의 일부이므로 바꾸거나 재사용하지 않습니다.
//!
//! | ID | 이름 | 값 타입 |
//! |---|---|---|
//! | 1 | `platform` | Enum([`PLATFORM_TYPE`]) |
//! | 2 | `os_version` | Number |
//! | 3 | `api_level` | Number |
//! | 4 | `color_scheme` | Enum([`COLOR_SCHEME_TYPE`]) |
//! | 5 | `contrast` | Enum([`CONTRAST_TYPE`]) |
//! | 6 | `reduced_motion` | Boolean |
//! | 7 | `viewport_width` | Length |
//! | 8 | `viewport_height` | Length |
//! | 9 | `locale` | String(BCP 47 tag) |
//! | 10 | `density` | Number(display scale) |
//! | 11 | `interaction_state` | Enum([`INTERACTION_STATE_TYPE`]) |
//!
//! 환경 enum 타입은 `0xE000_0000` 대역의 [`EnumTypeId`]를 씁니다. 다른 enum registry는 이 대역을 쓰지 않습니다.
//! 각 enum의 variant 번호도 고정입니다([`platform`], [`color_scheme`], [`contrast`], [`interaction_state`]).

use crate::ast::{EnvironmentKey, ReferenceTarget};
use crate::typecheck::ReferenceTypes;
use crate::types::{EnumTypeId, ExpressionType, TypeSpec};

/// 대상 플랫폼 계열(Target Profile 종류, 아키텍처 §17.1).
pub const PLATFORM: EnvironmentKey = EnvironmentKey::new(1);
/// OS 버전(Apple: major.minor, 그 밖은 0).
pub const OS_VERSION: EnvironmentKey = EnvironmentKey::new(2);
/// API level(Android, 그 밖은 0).
pub const API_LEVEL: EnvironmentKey = EnvironmentKey::new(3);
/// 색 구성(밝음·어두움).
pub const COLOR_SCHEME: EnvironmentKey = EnvironmentKey::new(4);
/// 대비(표준·높음).
pub const CONTRAST: EnvironmentKey = EnvironmentKey::new(5);
/// 동작 줄이기 설정.
pub const REDUCED_MOTION: EnvironmentKey = EnvironmentKey::new(6);
/// viewport 너비.
pub const VIEWPORT_WIDTH: EnvironmentKey = EnvironmentKey::new(7);
/// viewport 높이.
pub const VIEWPORT_HEIGHT: EnvironmentKey = EnvironmentKey::new(8);
/// locale(BCP 47 tag).
pub const LOCALE: EnvironmentKey = EnvironmentKey::new(9);
/// 화면 밀도(display scale).
pub const DENSITY: EnvironmentKey = EnvironmentKey::new(10);
/// 상호작용 상태(hover·pressed 등).
pub const INTERACTION_STATE: EnvironmentKey = EnvironmentKey::new(11);

/// 등록된 모든 key(numeric ID 순).
pub const ALL_KEYS: [EnvironmentKey; 11] = [
    PLATFORM,
    OS_VERSION,
    API_LEVEL,
    COLOR_SCHEME,
    CONTRAST,
    REDUCED_MOTION,
    VIEWPORT_WIDTH,
    VIEWPORT_HEIGHT,
    LOCALE,
    DENSITY,
    INTERACTION_STATE,
];

/// `platform` enum 타입.
pub const PLATFORM_TYPE: EnumTypeId = EnumTypeId::new(0xE000_0001);
/// `color_scheme` enum 타입.
pub const COLOR_SCHEME_TYPE: EnumTypeId = EnumTypeId::new(0xE000_0002);
/// `contrast` enum 타입.
pub const CONTRAST_TYPE: EnumTypeId = EnumTypeId::new(0xE000_0003);
/// `interaction_state` enum 타입.
pub const INTERACTION_STATE_TYPE: EnumTypeId = EnumTypeId::new(0xE000_0004);

/// `platform` variant 번호.
pub mod platform {
    /// Target Profile 없음(일반 미리보기).
    pub const GENERIC: u32 = 0;
    /// Web.
    pub const WEB: u32 = 1;
    /// Apple.
    pub const APPLE: u32 = 2;
    /// Android.
    pub const ANDROID: u32 = 3;
}

/// `color_scheme` variant 번호.
pub mod color_scheme {
    /// 밝음.
    pub const LIGHT: u32 = 0;
    /// 어두움.
    pub const DARK: u32 = 1;
}

/// `contrast` variant 번호.
pub mod contrast {
    /// 표준 대비.
    pub const STANDARD: u32 = 0;
    /// 높은 대비.
    pub const HIGH: u32 = 1;
}

/// `interaction_state` variant 번호.
pub mod interaction_state {
    /// 기본.
    pub const DEFAULT: u32 = 0;
    /// pointer hover.
    pub const HOVER: u32 = 1;
    /// 누름.
    pub const PRESSED: u32 = 2;
    /// focus.
    pub const FOCUSED: u32 = 3;
    /// 비활성.
    pub const DISABLED: u32 = 4;
}

/// key 하나의 registry 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnvironmentKeyInfo {
    /// numeric key.
    pub key: EnvironmentKey,
    /// 안정 이름(snake_case).
    pub name: &'static str,
    /// 값 타입(비-nullable).
    pub value_type: ExpressionType,
    /// enum key의 variant 수(그 밖은 0).
    pub variants: u32,
}

/// key의 registry 정보. 미등록 key는 `None`.
pub const fn key_info(key: EnvironmentKey) -> Option<EnvironmentKeyInfo> {
    let (name, value_type, variants) = match key.raw() {
        1 => ("platform", ExpressionType::Enum(PLATFORM_TYPE), 4),
        2 => ("os_version", ExpressionType::Number, 0),
        3 => ("api_level", ExpressionType::Number, 0),
        4 => ("color_scheme", ExpressionType::Enum(COLOR_SCHEME_TYPE), 2),
        5 => ("contrast", ExpressionType::Enum(CONTRAST_TYPE), 2),
        6 => ("reduced_motion", ExpressionType::Boolean, 0),
        7 => ("viewport_width", ExpressionType::Length, 0),
        8 => ("viewport_height", ExpressionType::Length, 0),
        9 => ("locale", ExpressionType::String, 0),
        10 => ("density", ExpressionType::Number, 0),
        11 => (
            "interaction_state",
            ExpressionType::Enum(INTERACTION_STATE_TYPE),
            5,
        ),
        _ => return None,
    };
    Some(EnvironmentKeyInfo {
        key,
        name,
        value_type,
        variants,
    })
}

/// 이름으로 key를 찾습니다.
pub fn key_by_name(name: &str) -> Option<EnvironmentKey> {
    ALL_KEYS
        .iter()
        .copied()
        .find(|key| key_info(*key).is_some_and(|info| info.name == name))
}

/// 환경 입력 참조의 타입 제공자. 등록된 key는 `Required(value_type)`이고 그 밖의 참조는 모릅니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnvironmentTypes;

impl ReferenceTypes for EnvironmentTypes {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Environment(key) => {
                key_info(*key).map(|info| TypeSpec::required(info.value_type))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// numeric ID·이름·타입·variant 수 golden. 값을 바꾸면 저장된 표현식의 의미가 바뀌므로 이 표는 고정입니다.
    const GOLDEN: &str = "\
1 platform Enum(3758096385) 4
2 os_version Number 0
3 api_level Number 0
4 color_scheme Enum(3758096386) 2
5 contrast Enum(3758096387) 2
6 reduced_motion Boolean 0
7 viewport_width Length 0
8 viewport_height Length 0
9 locale String 0
10 density Number 0
11 interaction_state Enum(3758096388) 5
";

    #[test]
    fn key_numeric_ids_match_golden() {
        let mut rendered = String::new();
        for key in ALL_KEYS {
            let info = key_info(key).expect("등록 key");
            assert_eq!(info.key, key);
            rendered.push_str(&format!(
                "{} {} {} {}\n",
                key.raw(),
                info.name,
                info.value_type,
                info.variants
            ));
        }
        assert_eq!(rendered, GOLDEN);
        // enum variant 번호도 고정입니다.
        assert_eq!(
            [
                platform::GENERIC,
                platform::WEB,
                platform::APPLE,
                platform::ANDROID
            ],
            [0, 1, 2, 3]
        );
        assert_eq!([color_scheme::LIGHT, color_scheme::DARK], [0, 1]);
        assert_eq!([contrast::STANDARD, contrast::HIGH], [0, 1]);
        assert_eq!(
            [
                interaction_state::DEFAULT,
                interaction_state::HOVER,
                interaction_state::PRESSED,
                interaction_state::FOCUSED,
                interaction_state::DISABLED
            ],
            [0, 1, 2, 3, 4]
        );
        assert_eq!(PLATFORM_TYPE.raw(), 0xE000_0001);
        assert_eq!(INTERACTION_STATE_TYPE.raw(), 0xE000_0004);
    }

    #[test]
    fn registry_lookup_and_types() {
        assert!(key_info(EnvironmentKey::new(0)).is_none());
        assert!(key_info(EnvironmentKey::new(12)).is_none());
        assert_eq!(key_by_name("color_scheme"), Some(COLOR_SCHEME));
        assert_eq!(key_by_name("dark_mode"), None);
        // ID는 1부터 빈틈없이 증가하고 이름은 유일합니다.
        for (index, key) in ALL_KEYS.iter().enumerate() {
            assert_eq!(key.raw() as usize, index + 1);
        }
        let names: std::collections::BTreeSet<&str> = ALL_KEYS
            .iter()
            .map(|k| key_info(*k).expect("등록").name)
            .collect();
        assert_eq!(names.len(), ALL_KEYS.len());
        assert_eq!(
            EnvironmentTypes.reference_type(&ReferenceTarget::Environment(REDUCED_MOTION)),
            Some(TypeSpec::required(ExpressionType::Boolean))
        );
        assert_eq!(
            EnvironmentTypes.reference_type(&ReferenceTarget::Environment(EnvironmentKey::new(99))),
            None
        );
    }
}
