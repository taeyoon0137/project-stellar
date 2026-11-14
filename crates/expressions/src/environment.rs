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
//! 환경 enum 타입은 `0xE000_0000`부터 `0xE000_FFFF`까지의 대역(상위 16 bit가 `0xE000`, ADR-0026 4절)에서
//! [`EnumTypeId`]를 받습니다. `0xE000_0000`은 할당하지 않고, 새 타입은 대역 안의 다음 번호(`0xE000_0005`, …)를
//! 받습니다. 다른 enum registry는 이 대역을 쓰지 않습니다.
//! 각 enum의 variant 번호도 고정입니다([`platform`], [`color_scheme`], [`contrast`], [`interaction_state`]).
//!
//! **missing key 기본값 정책(VIII.7.2)**: [`EvaluationEnvironment`]에 값이 없는 등록 key는 registry가 고정한 기본값
//! ([`missing_default`])으로 평가합니다. 값 없음(null)으로 두지 않으므로 환경 참조의 타입은 항상 `Required`입니다.
//!
//! | key | 기본값 |
//! |---|---|
//! | platform | `generic`(0) |
//! | os_version · api_level | 0 |
//! | color_scheme | `light`(0) |
//! | contrast | `standard`(0) |
//! | reduced_motion | `false` |
//! | viewport_width × viewport_height | 1280px × 800px |
//! | locale | `"und"`(BCP 47 미정) |
//! | density | 1 |
//! | interaction_state | `default`(0) |

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::{FiniteF32, Length};

use crate::ast::{EnvironmentKey, ReferenceTarget};
use crate::eval::ReferenceValues;
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

/// 등록 key의 missing 기본값(모듈 문서 표). 미등록 key는 `None`.
pub fn missing_default(key: EnvironmentKey) -> Option<TypedValue> {
    let number = |v: f32| FiniteF32::new(v).ok().map(TypedValue::Number);
    let length = |v: f32| Length::new(v).ok().map(TypedValue::Length);
    let variant = |enum_type: EnumTypeId, index: u32| {
        Some(TypedValue::Enum(EnumValue::new(enum_type.raw(), index)))
    };
    match key {
        PLATFORM => variant(PLATFORM_TYPE, platform::GENERIC),
        OS_VERSION | API_LEVEL => number(0.0),
        COLOR_SCHEME => variant(COLOR_SCHEME_TYPE, color_scheme::LIGHT),
        CONTRAST => variant(CONTRAST_TYPE, contrast::STANDARD),
        REDUCED_MOTION => Some(TypedValue::Boolean(false)),
        VIEWPORT_WIDTH => length(1280.0),
        VIEWPORT_HEIGHT => length(800.0),
        LOCALE => Some(TypedValue::String("und".to_string())),
        DENSITY => number(1.0),
        INTERACTION_STATE => variant(INTERACTION_STATE_TYPE, interaction_state::DEFAULT),
        _ => None,
    }
}

/// 환경 값 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnvironmentError {
    /// registry에 없는 key.
    UnknownKey(EnvironmentKey),
    /// 값 타입이 key 타입과 다릅니다.
    TypeMismatch {
        /// key.
        key: EnvironmentKey,
        /// key 타입.
        expected: ExpressionType,
    },
    /// enum variant 번호가 key의 variant 수를 넘습니다.
    InvalidVariant {
        /// key.
        key: EnvironmentKey,
        /// 주어진 variant.
        variant: u32,
    },
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvironmentError::UnknownKey(key) => write!(f, "등록되지 않은 환경 key: {}", key.raw()),
            EnvironmentError::TypeMismatch { key, expected } => {
                write!(
                    f,
                    "환경 key {}의 값은 {expected} 타입이어야 합니다",
                    key.raw()
                )
            }
            EnvironmentError::InvalidVariant { key, variant } => {
                write!(f, "환경 key {}에 없는 variant {variant}", key.raw())
            }
        }
    }
}

impl std::error::Error for EnvironmentError {}

/// 값이 key의 registry 타입·variant 범위를 지키는지 검사합니다.
pub fn check_value(key: EnvironmentKey, value: &TypedValue) -> Result<(), EnvironmentError> {
    let info = key_info(key).ok_or(EnvironmentError::UnknownKey(key))?;
    if !info.value_type.matches(value) {
        return Err(EnvironmentError::TypeMismatch {
            key,
            expected: info.value_type,
        });
    }
    if let TypedValue::Enum(e) = value {
        if e.variant() >= info.variants {
            return Err(EnvironmentError::InvalidVariant {
                key,
                variant: e.variant(),
            });
        }
    }
    Ok(())
}

/// 표현식 평가 환경 (VIII.7.2). 불변 map이며 바꿀 때는 새 환경을 만듭니다(복제는 공유 참조라 가볍습니다).
///
/// 명시한 값만 보관하고, 없는 등록 key는 [`missing_default`]로 평가합니다. 모든 값은 생성 시 registry 타입·variant
/// 범위를 검사합니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvaluationEnvironment {
    values: Arc<BTreeMap<EnvironmentKey, TypedValue>>,
}

impl EvaluationEnvironment {
    /// 값이 하나도 없는 환경(모든 key가 기본값).
    pub fn new() -> EvaluationEnvironment {
        EvaluationEnvironment::default()
    }

    /// (key, 값) 목록으로 만듭니다. 같은 key가 반복되면 마지막 값입니다.
    pub fn from_values(
        values: impl IntoIterator<Item = (EnvironmentKey, TypedValue)>,
    ) -> Result<EvaluationEnvironment, EnvironmentError> {
        let mut map = BTreeMap::new();
        for (key, value) in values {
            check_value(key, &value)?;
            map.insert(key, value);
        }
        Ok(EvaluationEnvironment {
            values: Arc::new(map),
        })
    }

    /// `key`를 `value`로 바꾼 새 환경. 원래 환경은 그대로입니다.
    pub fn with(
        &self,
        key: EnvironmentKey,
        value: TypedValue,
    ) -> Result<EvaluationEnvironment, EnvironmentError> {
        check_value(key, &value)?;
        let mut map = (*self.values).clone();
        map.insert(key, value);
        Ok(EvaluationEnvironment {
            values: Arc::new(map),
        })
    }

    /// `key`의 명시 값을 지운 새 환경(이후 기본값으로 평가).
    pub fn without(&self, key: EnvironmentKey) -> EvaluationEnvironment {
        let mut map = (*self.values).clone();
        map.remove(&key);
        EvaluationEnvironment {
            values: Arc::new(map),
        }
    }

    /// 명시한 값(기본값 제외).
    pub fn explicit(&self, key: EnvironmentKey) -> Option<&TypedValue> {
        self.values.get(&key)
    }

    /// 명시 여부.
    pub fn is_present(&self, key: EnvironmentKey) -> bool {
        self.values.contains_key(&key)
    }

    /// 평가에 쓰는 값: 명시 값 또는 registry 기본값. 미등록 key는 `None`.
    pub fn value(&self, key: EnvironmentKey) -> Option<TypedValue> {
        match self.values.get(&key) {
            Some(value) => Some(value.clone()),
            None => missing_default(key),
        }
    }

    /// 평가 값이 `other`와 다른 등록 key(ID 순). 기본값과 같은 값을 명시해도 바뀐 것으로 보지 않습니다.
    pub fn changed_keys(&self, other: &EvaluationEnvironment) -> BTreeSet<EnvironmentKey> {
        let candidates: BTreeSet<EnvironmentKey> = self
            .values
            .keys()
            .chain(other.values.keys())
            .copied()
            .collect();
        candidates
            .into_iter()
            .filter(|key| self.value(*key) != other.value(*key))
            .collect()
    }
}

impl ReferenceValues for EvaluationEnvironment {
    /// 환경 참조만 해석합니다(그 밖의 참조는 모르는 대상).
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        match target {
            ReferenceTarget::Environment(key) => self.value(*key).map(Some),
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
    fn environment_enum_types_stay_in_reserved_band() {
        // ADR-0026 4절: 대역 0xE000_0000..=0xE000_FFFF, 0xE000_0000은 할당하지 않음, 0xE000_0001부터 연속.
        let mut enum_types = Vec::new();
        for key in ALL_KEYS {
            if let ExpressionType::Enum(id) = key_info(key).expect("등록").value_type {
                assert_eq!(
                    id.raw() >> 16,
                    0xE000,
                    "{key:?} 타입 {:#X}가 대역 밖",
                    id.raw()
                );
                assert_ne!(id.raw(), 0xE000_0000, "{key:?}가 예약 번호를 씀");
                enum_types.push(id.raw());
            }
        }
        enum_types.sort_unstable();
        enum_types.dedup();
        let expected: Vec<u32> = (0..enum_types.len() as u32)
            .map(|n| 0xE000_0001 + n)
            .collect();
        assert_eq!(enum_types, expected);
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

    fn dark() -> TypedValue {
        TypedValue::Enum(EnumValue::new(COLOR_SCHEME_TYPE.raw(), color_scheme::DARK))
    }

    #[test]
    fn missing_keys_use_registry_defaults() {
        let empty = EvaluationEnvironment::new();
        for key in ALL_KEYS {
            let info = key_info(key).expect("등록");
            let value = empty.value(key).expect("기본값 고정");
            assert!(info.value_type.matches(&value), "{} 기본값 타입", info.name);
            assert_eq!(check_value(key, &value), Ok(()));
            assert!(!empty.is_present(key));
        }
        assert_eq!(
            empty.value(VIEWPORT_WIDTH),
            Some(TypedValue::Length(Length::new(1280.0).expect("유한")))
        );
        assert_eq!(
            empty.value(LOCALE),
            Some(TypedValue::String("und".to_string()))
        );
        assert_eq!(empty.value(EnvironmentKey::new(99)), None);
        assert_eq!(
            empty.reference_value(&ReferenceTarget::Environment(REDUCED_MOTION)),
            Some(Some(TypedValue::Boolean(false)))
        );
    }

    #[test]
    fn present_values_override_and_environment_is_immutable() {
        let base = EvaluationEnvironment::from_values([(COLOR_SCHEME, dark())]).expect("유효");
        assert!(base.is_present(COLOR_SCHEME));
        assert_eq!(base.value(COLOR_SCHEME), Some(dark()));
        assert_eq!(base.explicit(CONTRAST), None);
        // with는 새 환경을 만들고 원래 환경은 그대로입니다.
        let wide = base
            .with(
                VIEWPORT_WIDTH,
                TypedValue::Length(Length::new(1920.0).expect("유한")),
            )
            .expect("유효");
        assert!(!base.is_present(VIEWPORT_WIDTH));
        assert_eq!(wide.changed_keys(&base), BTreeSet::from([VIEWPORT_WIDTH]));
        // 기본값과 같은 명시 값은 변화가 아닙니다.
        let explicit_light = base
            .with(
                CONTRAST,
                TypedValue::Enum(EnumValue::new(CONTRAST_TYPE.raw(), contrast::STANDARD)),
            )
            .expect("유효");
        assert!(explicit_light.changed_keys(&base).is_empty());
        assert_eq!(
            base.without(COLOR_SCHEME).changed_keys(&base),
            BTreeSet::from([COLOR_SCHEME])
        );
        // 잘못된 값은 거부합니다.
        assert_eq!(
            base.with(REDUCED_MOTION, TypedValue::Number(FiniteF32::ONE)),
            Err(EnvironmentError::TypeMismatch {
                key: REDUCED_MOTION,
                expected: ExpressionType::Boolean
            })
        );
        assert_eq!(
            base.with(
                COLOR_SCHEME,
                TypedValue::Enum(EnumValue::new(COLOR_SCHEME_TYPE.raw(), 2))
            ),
            Err(EnvironmentError::InvalidVariant {
                key: COLOR_SCHEME,
                variant: 2
            })
        );
        assert_eq!(
            EvaluationEnvironment::from_values([(
                EnvironmentKey::new(42),
                TypedValue::Boolean(true)
            )]),
            Err(EnvironmentError::UnknownKey(EnvironmentKey::new(42)))
        );
        // 다른 enum 타입의 값도 타입 불일치입니다.
        assert!(matches!(
            base.with(
                COLOR_SCHEME,
                TypedValue::Enum(EnumValue::new(CONTRAST_TYPE.raw(), 0))
            ),
            Err(EnvironmentError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn expressions_evaluate_against_missing_and_present_values() {
        use crate::ast::{ExprNode, ExprNodeId, ExpressionAst};
        use crate::bytecode::Program;
        use crate::ops::BinaryOp;
        use stellar_core_types::ids::StableId;
        let id = |n: u64| ExprNodeId::new(StableId::new_v7(n, 0, n));
        // if color_scheme == dark then 1 else 2
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::if_else(id(1), id(2), id(5), id(6)),
                ExprNode::binary(id(2), BinaryOp::Equal, id(3), id(4)),
                ExprNode::reference(id(3), ReferenceTarget::Environment(COLOR_SCHEME)),
                ExprNode::literal(id(4), dark()),
                ExprNode::literal(id(5), TypedValue::Number(FiniteF32::ONE)),
                ExprNode::literal(
                    id(6),
                    TypedValue::Number(FiniteF32::new(2.0).expect("유한")),
                ),
            ],
        )
        .expect("유효");
        let program = Program::compile(&ast, &EnvironmentTypes).expect("컴파일");
        let missing = EvaluationEnvironment::new();
        let present = missing.with(COLOR_SCHEME, dark()).expect("유효");
        assert_eq!(
            program.evaluate(&missing),
            Ok(Some(TypedValue::Number(FiniteF32::new(2.0).expect("유한"))))
        );
        assert_eq!(
            program.evaluate(&present),
            Ok(Some(TypedValue::Number(FiniteF32::ONE)))
        );
    }
}
