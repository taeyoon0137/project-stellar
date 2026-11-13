//! 표현식 타입 (VIII.1.1).
//!
//! - [`ExpressionType`]: [`TypedValue`]의 variant와 1:1로 대응하는 **비-nullable** 값 타입입니다.
//!   [`ExpressionType::of`]는 `TypedValue`의 모든 variant를 wildcard 없이 매핑하므로, 값 variant가
//!   추가되면 이 매핑을 갱신하기 전까지 컴파일되지 않습니다.
//! - [`Optional<T>`]: nullable 타입입니다. 일반 타입에 "null 허용" 플래그를 섞지 않고 별도 타입으로만
//!   표현합니다. 내부 타입 `T`는 봉인된 [`NonNullType`]만 허용하므로 `Optional<Optional<T>>` 중첩은
//!   컴파일되지 않습니다.
//! - [`TypeSpec`]: 표현식 위치(노드 결과·인자·property)의 전체 타입으로, 값이 반드시 있어야 하는
//!   `Required`와 값 없음을 허용하는 `Optional`을 구분합니다.
//!
//! 이 모듈은 새 numeric registry 번호를 만들지 않습니다. 타입과 값의 대응은 `TypedValue`의 안정
//! type tag(III.3.5)를 그대로 따릅니다.

use std::fmt;

use stellar_core_types::typed_value::TypedValue;

/// enum 타입 식별자. [`stellar_core_types::typed_value::EnumValue::enum_type`]과 같은 numeric ID입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnumTypeId(u32);

impl EnumTypeId {
    /// numeric enum 타입 ID로부터 만듭니다.
    pub const fn new(raw: u32) -> EnumTypeId {
        EnumTypeId(raw)
    }

    /// numeric enum 타입 ID.
    pub const fn raw(&self) -> u32 {
        self.0
    }
}

/// 비-nullable 표현식 값 타입 (VIII.1.1).
///
/// variant는 [`TypedValue`]의 variant와 같은 순서·같은 개수(13개)로 1:1 대응합니다. `Enum`은 서로
/// 다른 enum 타입을 구분하기 위해 [`EnumTypeId`]를 가집니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpressionType {
    /// 불리언(`TypedValue::Boolean`).
    Boolean,
    /// 무차원 수치(`TypedValue::Number`).
    Number,
    /// 문자열(`TypedValue::String`).
    String,
    /// 특정 enum 타입의 값(`TypedValue::Enum`).
    Enum(EnumTypeId),
    /// 길이(`TypedValue::Length`).
    Length,
    /// 백분율(`TypedValue::Percentage`).
    Percentage,
    /// 각도(`TypedValue::Angle`).
    Angle,
    /// 지속 시간(`TypedValue::Duration`).
    Duration,
    /// 색상(`TypedValue::Color`).
    Color,
    /// 그라디언트(`TypedValue::Gradient`).
    Gradient,
    /// 그림자(`TypedValue::Shadow`).
    Shadow,
    /// 타이포그래피(`TypedValue::Typography`).
    Typography,
    /// 리소스 참조(`TypedValue::ResourceRef`).
    ResourceRef,
}

impl ExpressionType {
    /// variant 수. `TypedValue` variant 수(13)와 같아야 합니다.
    pub const VARIANT_COUNT: usize = 13;

    /// 값의 표현식 타입. `TypedValue`의 모든 variant를 wildcard 없이 매핑합니다.
    pub fn of(value: &TypedValue) -> ExpressionType {
        match value {
            TypedValue::Boolean(_) => ExpressionType::Boolean,
            TypedValue::Number(_) => ExpressionType::Number,
            TypedValue::String(_) => ExpressionType::String,
            TypedValue::Enum(e) => ExpressionType::Enum(EnumTypeId::new(e.enum_type())),
            TypedValue::Length(_) => ExpressionType::Length,
            TypedValue::Percentage(_) => ExpressionType::Percentage,
            TypedValue::Angle(_) => ExpressionType::Angle,
            TypedValue::Duration(_) => ExpressionType::Duration,
            TypedValue::Color(_) => ExpressionType::Color,
            TypedValue::Gradient(_) => ExpressionType::Gradient,
            TypedValue::Shadow(_) => ExpressionType::Shadow,
            TypedValue::Typography(_) => ExpressionType::Typography,
            TypedValue::ResourceRef(_) => ExpressionType::ResourceRef,
        }
    }

    /// 값이 이 타입인지 여부. `Enum`은 enum 타입 ID까지 같아야 합니다.
    pub fn matches(&self, value: &TypedValue) -> bool {
        ExpressionType::of(value) == *self
    }

    /// 진단용 타입 이름(`Enum`은 ID 없이 `Enum`).
    pub const fn name(&self) -> &'static str {
        match self {
            ExpressionType::Boolean => "Boolean",
            ExpressionType::Number => "Number",
            ExpressionType::String => "String",
            ExpressionType::Enum(_) => "Enum",
            ExpressionType::Length => "Length",
            ExpressionType::Percentage => "Percentage",
            ExpressionType::Angle => "Angle",
            ExpressionType::Duration => "Duration",
            ExpressionType::Color => "Color",
            ExpressionType::Gradient => "Gradient",
            ExpressionType::Shadow => "Shadow",
            ExpressionType::Typography => "Typography",
            ExpressionType::ResourceRef => "ResourceRef",
        }
    }
}

impl fmt::Display for ExpressionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpressionType::Enum(id) => write!(f, "Enum({})", id.raw()),
            other => f.write_str(other.name()),
        }
    }
}

/// [`NonNullType`] 봉인. 외부 crate는 구현할 수 없습니다.
mod sealed {
    /// 봉인 marker.
    pub trait Sealed {}

    impl Sealed for super::ExpressionType {}
}

/// 비-nullable 타입 marker. [`ExpressionType`]만 구현하며 봉인되어 있습니다.
///
/// [`Optional`]은 `NonNullType`이 아니므로 `Optional<Optional<T>>`는 만들 수 없습니다.
///
/// ```compile_fail
/// use stellar_expressions::types::{ExpressionType, Optional};
/// let _nested: Optional<Optional<ExpressionType>> =
///     Optional::new(Optional::new(ExpressionType::Length));
/// ```
///
/// 외부 crate는 봉인된 trait을 구현할 수 없습니다.
///
/// ```compile_fail
/// use stellar_expressions::types::NonNullType;
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// struct Custom;
/// impl NonNullType for Custom {}
/// ```
pub trait NonNullType: sealed::Sealed + Copy + Eq + fmt::Debug {}

impl NonNullType for ExpressionType {}

/// nullable 타입 `Optional<T>` (VIII.1.1). 값이 없거나 `T` 타입 값입니다.
///
/// ```
/// use stellar_expressions::types::{ExpressionType, Optional};
/// let maybe_length = Optional::new(ExpressionType::Length);
/// assert_eq!(maybe_length.inner(), ExpressionType::Length);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Optional<T: NonNullType>(T);

impl<T: NonNullType> Optional<T> {
    /// 비-nullable 타입을 nullable로 감쌉니다.
    pub const fn new(inner: T) -> Optional<T> {
        Optional(inner)
    }

    /// 감싼 비-nullable 타입.
    pub const fn inner(&self) -> T {
        self.0
    }
}

/// 표현식 위치의 전체 타입 (VIII.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeSpec {
    /// 값이 반드시 있어야 합니다.
    Required(ExpressionType),
    /// 값이 없을 수 있습니다.
    Optional(Optional<ExpressionType>),
}

impl TypeSpec {
    /// 값 필수 타입.
    pub const fn required(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::Required(value_type)
    }

    /// nullable 타입.
    pub const fn optional(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::Optional(Optional::new(value_type))
    }

    /// 값이 있을 때의 비-nullable 타입.
    pub const fn value_type(&self) -> ExpressionType {
        match self {
            TypeSpec::Required(value_type) => *value_type,
            TypeSpec::Optional(optional) => optional.inner(),
        }
    }

    /// nullable이면 `true`.
    pub const fn is_optional(&self) -> bool {
        matches!(self, TypeSpec::Optional(_))
    }

    /// nullable로 바꿉니다. 이미 nullable이면 그대로입니다(중첩하지 않음).
    pub const fn to_optional(self) -> TypeSpec {
        TypeSpec::optional(self.value_type())
    }

    /// 값(없음은 `None`)을 이 타입 위치에 둘 수 있는지 여부.
    pub fn accepts(&self, value: Option<&TypedValue>) -> bool {
        match value {
            Some(value) => self.value_type().matches(value),
            None => self.is_optional(),
        }
    }
}

impl From<ExpressionType> for TypeSpec {
    fn from(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::Required(value_type)
    }
}

impl From<Optional<ExpressionType>> for TypeSpec {
    fn from(optional: Optional<ExpressionType>) -> TypeSpec {
        TypeSpec::Optional(optional)
    }
}

impl fmt::Display for TypeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeSpec::Required(value_type) => write!(f, "{value_type}"),
            TypeSpec::Optional(optional) => write!(f, "Optional<{}>", optional.inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::gradient::Gradient;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::shadow::{BlurRadius, Shadow};
    use stellar_core_types::typed_value::{EnumValue, ResourceRef, TypographyValue};
    use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage, Vec2};

    use super::*;

    fn black() -> Color {
        Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap()
    }

    /// 표현식 타입마다 그 타입의 대표 값. wildcard 없는 match라 `ExpressionType` variant가 추가되면
    /// 대표 값(=대응하는 `TypedValue` variant)을 정하기 전까지 컴파일되지 않습니다.
    fn representative(value_type: ExpressionType) -> TypedValue {
        match value_type {
            ExpressionType::Boolean => TypedValue::Boolean(true),
            ExpressionType::Number => TypedValue::Number(FiniteF32::new(1.5).unwrap()),
            ExpressionType::String => TypedValue::String("텍스트".to_string()),
            ExpressionType::Enum(id) => TypedValue::Enum(EnumValue::new(id.raw(), 0)),
            ExpressionType::Length => TypedValue::Length(Length::new(8.0).unwrap()),
            ExpressionType::Percentage => TypedValue::Percentage(Percentage::new(50.0).unwrap()),
            ExpressionType::Angle => TypedValue::Angle(Angle::new(90.0).unwrap()),
            ExpressionType::Duration => TypedValue::Duration(Duration::new(0.25).unwrap()),
            ExpressionType::Color => TypedValue::Color(black()),
            ExpressionType::Gradient => TypedValue::Gradient(Gradient::default()),
            ExpressionType::Shadow => {
                TypedValue::Shadow(Shadow::new(Vec2::ZERO, BlurRadius::ZERO, black()))
            }
            ExpressionType::Typography => TypedValue::Typography(TypographyValue::default()),
            ExpressionType::ResourceRef => TypedValue::ResourceRef(ResourceRef::new(StableId::NIL)),
        }
    }

    /// 안정 type tag(III.3.5) ↔ 표현식 타입 golden 표. 13개 variant 전부.
    fn golden_mapping() -> Vec<(u8, ExpressionType)> {
        vec![
            (0x01, ExpressionType::Boolean),
            (0x02, ExpressionType::Number),
            (0x03, ExpressionType::String),
            (0x04, ExpressionType::Enum(EnumTypeId::new(7))),
            (0x05, ExpressionType::Length),
            (0x06, ExpressionType::Percentage),
            (0x07, ExpressionType::Angle),
            (0x08, ExpressionType::Duration),
            (0x09, ExpressionType::Color),
            (0x0A, ExpressionType::Gradient),
            (0x0B, ExpressionType::Shadow),
            (0x0C, ExpressionType::Typography),
            (0x0D, ExpressionType::ResourceRef),
        ]
    }

    #[test]
    fn type_mapping_is_complete_and_one_to_one() {
        let mapping = golden_mapping();
        assert_eq!(mapping.len(), ExpressionType::VARIANT_COUNT);

        let mut tags = BTreeSet::new();
        let mut names = BTreeSet::new();
        for (tag, value_type) in &mapping {
            let value = representative(*value_type);
            // 타입 → 값 → 타입 왕복(표현식 타입마다 대응 값 variant가 존재).
            assert_eq!(ExpressionType::of(&value), *value_type, "{value_type}");
            assert!(value_type.matches(&value));
            // 대응 값 variant가 golden type tag와 일치.
            assert_eq!(value.type_tag(), *tag, "{value_type}");
            tags.insert(value.type_tag());
            names.insert(value_type.name());
        }
        // 값 variant 13개 tag(0x01~0x0D)를 빠짐없이·중복 없이 덮음 → 전단사.
        let expected_tags: BTreeSet<u8> = (0x01..=0x0D).collect();
        assert_eq!(tags, expected_tags);
        assert_eq!(names.len(), ExpressionType::VARIANT_COUNT);
    }

    #[test]
    fn every_typed_value_variant_has_exactly_one_type() {
        // 값 쪽에서 출발: 각 값은 정확히 하나의 타입에만 맞습니다.
        let types: Vec<ExpressionType> = golden_mapping().into_iter().map(|(_, t)| t).collect();
        for value_type in &types {
            let value = representative(*value_type);
            let matching = types.iter().filter(|t| t.matches(&value)).count();
            assert_eq!(matching, 1, "{value_type}");
        }
    }

    #[test]
    fn enum_type_identity_is_part_of_the_type() {
        let value = TypedValue::Enum(EnumValue::new(7, 3));
        assert_eq!(
            ExpressionType::of(&value),
            ExpressionType::Enum(EnumTypeId::new(7))
        );
        assert!(ExpressionType::Enum(EnumTypeId::new(7)).matches(&value));
        assert!(!ExpressionType::Enum(EnumTypeId::new(8)).matches(&value));
        assert_eq!(
            ExpressionType::Enum(EnumTypeId::new(7)).to_string(),
            "Enum(7)"
        );
    }

    #[test]
    fn optional_is_separate_from_plain_type() {
        let length = TypedValue::Length(Length::new(4.0).unwrap());
        let color = TypedValue::Color(black());

        let required = TypeSpec::required(ExpressionType::Length);
        assert!(!required.is_optional());
        assert!(required.accepts(Some(&length)));
        assert!(
            !required.accepts(None),
            "일반 타입은 값 없음을 허용하지 않음"
        );
        assert!(!required.accepts(Some(&color)));

        let optional = TypeSpec::optional(ExpressionType::Length);
        assert!(optional.is_optional());
        assert!(optional.accepts(Some(&length)));
        assert!(optional.accepts(None));
        assert!(!optional.accepts(Some(&color)));

        // 일반 타입과 Optional은 서로 다른 타입.
        assert_ne!(required, optional);
        assert_eq!(required.value_type(), optional.value_type());
    }

    #[test]
    fn to_optional_does_not_nest() {
        let once = TypeSpec::required(ExpressionType::Color).to_optional();
        let twice = once.to_optional();
        assert_eq!(once, twice);
        assert_eq!(
            twice,
            TypeSpec::Optional(Optional::new(ExpressionType::Color))
        );
        assert_eq!(twice.to_string(), "Optional<Color>");
    }

    #[test]
    fn every_type_has_an_optional_form() {
        for (_, value_type) in golden_mapping() {
            let optional = TypeSpec::from(Optional::new(value_type));
            assert!(optional.is_optional());
            assert_eq!(optional.value_type(), value_type);
            assert!(optional.accepts(None));
            assert!(optional.accepts(Some(&representative(value_type))));
            assert_eq!(TypeSpec::from(value_type), TypeSpec::required(value_type));
            assert_eq!(optional.to_string(), format!("Optional<{value_type}>"));
        }
    }

    #[test]
    fn display_names_are_stable() {
        assert_eq!(ExpressionType::Length.to_string(), "Length");
        assert_eq!(ExpressionType::ResourceRef.to_string(), "ResourceRef");
        assert_eq!(
            TypeSpec::required(ExpressionType::Number).to_string(),
            "Number"
        );
        assert_eq!(EnumTypeId::new(3).raw(), 3);
    }
}
