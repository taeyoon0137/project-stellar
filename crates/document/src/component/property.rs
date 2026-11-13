//! ComponentProperty record (IX.1.2).
//!
//! 컴포넌트 정의(IX.1.1)가 선언한 Property 하나의 상세 record입니다: StableId, 이름, 타입, **필수**
//! default expression, 공개 여부.
//!
//! - default는 생성자의 필수 인자이므로 default 없는 property는 만들 수 없습니다. 저장 형식에서도
//!   default bytes가 비면(node 없는 AST) 디코딩이 거부합니다.
//! - default는 **닫힌 표현식**(참조 없음)이어야 합니다. orphaned binding은 대상 Property의 선언된
//!   기본값으로 렌더링합니다(기술 아키텍처 14.4) — 그 기본값이 다시 다른 원천(token·속성)에 묶이면
//!   그 원천도 끊길 수 있으므로, 기본값은 항상 스스로 평가 가능해야 합니다. 원천에 묶인 값은
//!   binding(Phase VIII)의 몫입니다.
//! - default의 추론 타입이 선언 타입에 대입되지 않으면(type mismatch) 거부합니다. 평가 오류
//!   (예: 0으로 나누기)도 "유효한 기본값 필수" 규칙에 따라 거부합니다.
//! - 이름은 표시·drilling 제안(IX.4.4)에 쓰입니다. 식별은 StableId로만 합니다. 한 컴포넌트 안에서
//!   이름은 유일해야 합니다([`ComponentLibrary::add_property`]가 강제).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::codec::CodecError;
use stellar_expressions::eval::EvalError;
use stellar_expressions::types::TypeSpec;

use super::codec::{put_id, put_str, put_u8, Malformed, Reader};
use super::definition::{ComponentEntry, ComponentLibrary, ComponentPropertyId};
use super::typed_expr::{put_expression, put_type_spec, CheckedExpression, ExpressionCheckError};

/// 이름 최대 길이(UTF-8 bytes).
pub const MAX_PROPERTY_NAME_BYTES: usize = 256;

/// Property 공개 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropertyVisibility {
    /// 인스턴스가 override할 수 있고 상위 컴포넌트가 drilling할 수 있습니다.
    Public,
    /// 정의 내부 전용입니다. 인스턴스 override·외부 binding 대상이 아닙니다.
    Private,
}

impl PropertyVisibility {
    /// 안정 numeric tag(저장 형식 계약).
    pub const fn tag(&self) -> u8 {
        match self {
            PropertyVisibility::Public => 0x01,
            PropertyVisibility::Private => 0x02,
        }
    }

    /// tag에서 복원합니다. 알 수 없는 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<PropertyVisibility> {
        match tag {
            0x01 => Some(PropertyVisibility::Public),
            0x02 => Some(PropertyVisibility::Private),
            _ => None,
        }
    }
}

/// ComponentProperty 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentPropertyError {
    /// 이름이 비었거나, 너무 길거나, 제어 문자를 포함합니다.
    InvalidName {
        /// 원인.
        reason: &'static str,
    },
    /// default expression이 참조를 포함합니다(닫힌 표현식이어야 함).
    DefaultNotClosed {
        /// 첫 참조 대상.
        target: ReferenceTarget,
    },
    /// default expression의 타입이 선언 타입과 맞지 않습니다.
    DefaultTypeMismatch {
        /// 선언 타입.
        expected: TypeSpec,
        /// default 추론 타입.
        found: TypeSpec,
    },
    /// default expression type 검사·크기 오류.
    DefaultInvalid(ExpressionCheckError),
    /// default expression 평가 오류(유효한 기본값 필수).
    DefaultEvaluation(EvalError),
    /// 컴포넌트가 library에 없습니다.
    ComponentNotFound {
        /// 컴포넌트.
        component: ComponentRef,
    },
    /// 정의의 Property 목록에 선언되지 않은 record입니다.
    NotDeclared {
        /// property.
        property: ComponentPropertyId,
    },
    /// 같은 property record가 이미 있습니다.
    DuplicateRecord {
        /// property.
        property: ComponentPropertyId,
    },
    /// 같은 컴포넌트 안에 같은 이름의 property가 있습니다.
    DuplicateName {
        /// 이름.
        name: String,
    },
    /// 선언된 property에 record가 없습니다.
    MissingRecord {
        /// property.
        property: ComponentPropertyId,
    },
    /// default AST bytes가 잘못됐습니다(빈 AST 포함).
    Codec(CodecError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ComponentPropertyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentPropertyError::InvalidName { reason } => {
                write!(f, "property 이름이 잘못됐습니다: {reason}")
            }
            ComponentPropertyError::DefaultNotClosed { target } => {
                write!(f, "property default는 참조를 가질 수 없습니다({target})")
            }
            ComponentPropertyError::DefaultTypeMismatch { expected, found } => write!(
                f,
                "property default 타입 {found}가 선언 타입 {expected}와 맞지 않습니다"
            ),
            ComponentPropertyError::DefaultInvalid(err) => write!(f, "property default: {err}"),
            ComponentPropertyError::DefaultEvaluation(err) => {
                write!(f, "property default 평가 실패: {err}")
            }
            ComponentPropertyError::ComponentNotFound { component } => {
                write!(f, "컴포넌트 {}가 없습니다", component.id())
            }
            ComponentPropertyError::NotDeclared { property } => write!(
                f,
                "property {}는 정의에 선언되지 않았습니다",
                property.stable_id()
            ),
            ComponentPropertyError::DuplicateRecord { property } => {
                write!(
                    f,
                    "property {} record가 이미 있습니다",
                    property.stable_id()
                )
            }
            ComponentPropertyError::DuplicateName { name } => {
                write!(f, "같은 이름의 property가 이미 있습니다: {name}")
            }
            ComponentPropertyError::MissingRecord { property } => write!(
                f,
                "선언된 property {}에 record가 없습니다",
                property.stable_id()
            ),
            ComponentPropertyError::Codec(err) => write!(f, "property default bytes 오류: {err}"),
            ComponentPropertyError::Malformed { reason } => {
                write!(f, "ComponentProperty binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for ComponentPropertyError {}

impl From<Malformed> for ComponentPropertyError {
    fn from(err: Malformed) -> ComponentPropertyError {
        ComponentPropertyError::Malformed { reason: err.0 }
    }
}

impl From<ExpressionCheckError> for ComponentPropertyError {
    fn from(err: ExpressionCheckError) -> ComponentPropertyError {
        match err {
            ExpressionCheckError::ForbiddenReference(target) => {
                ComponentPropertyError::DefaultNotClosed { target }
            }
            ExpressionCheckError::TypeMismatch { expected, found } => {
                ComponentPropertyError::DefaultTypeMismatch { expected, found }
            }
            ExpressionCheckError::Codec(err) => ComponentPropertyError::Codec(err),
            other => ComponentPropertyError::DefaultInvalid(other),
        }
    }
}

/// 표시용 이름 규칙: 앞뒤 공백 제외 비어 있지 않음, [`MAX_PROPERTY_NAME_BYTES`] 이하, 제어 문자 없음.
pub fn validate_property_name(name: &str) -> Result<(), ComponentPropertyError> {
    if name.trim().is_empty() {
        return Err(ComponentPropertyError::InvalidName {
            reason: "빈 이름"
        });
    }
    if name.len() > MAX_PROPERTY_NAME_BYTES {
        return Err(ComponentPropertyError::InvalidName {
            reason: "이름이 너무 김",
        });
    }
    if name.chars().any(char::is_control) {
        return Err(ComponentPropertyError::InvalidName {
            reason: "제어 문자 포함",
        });
    }
    Ok(())
}

/// ComponentProperty record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentProperty {
    id: ComponentPropertyId,
    name: String,
    visibility: PropertyVisibility,
    default: CheckedExpression,
    default_value: Option<TypedValue>,
}

impl ComponentProperty {
    /// property를 만듭니다. default는 필수이며 닫힌 표현식이고 선언 타입 `value_type`에 맞아야 합니다.
    pub fn new(
        id: ComponentPropertyId,
        name: String,
        value_type: TypeSpec,
        default: ExpressionAst,
        visibility: PropertyVisibility,
    ) -> Result<ComponentProperty, ComponentPropertyError> {
        validate_property_name(&name)?;
        let (default, default_value) = check_default(default, value_type)?;
        Ok(ComponentProperty {
            id,
            name,
            visibility,
            default,
            default_value,
        })
    }

    /// 식별자.
    pub fn id(&self) -> ComponentPropertyId {
        self.id
    }

    /// 이름(식별에 사용 금지).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 선언 타입.
    pub fn value_type(&self) -> TypeSpec {
        self.default.declared_type()
    }

    /// 공개 여부.
    pub fn visibility(&self) -> PropertyVisibility {
        self.visibility
    }

    /// 공개 property인지.
    pub fn is_public(&self) -> bool {
        self.visibility == PropertyVisibility::Public
    }

    /// default expression.
    pub fn default_expression(&self) -> &CheckedExpression {
        &self.default
    }

    /// default를 평가한 값(`None`이면 nullable property의 값 없음). 저장하지 않는 캐시입니다.
    pub fn default_value(&self) -> Option<&TypedValue> {
        self.default_value.as_ref()
    }

    /// 같은 id·타입으로 이름만 바꾼 사본을 검증해 만듭니다.
    pub fn renamed(&self, name: String) -> Result<ComponentProperty, ComponentPropertyError> {
        validate_property_name(&name)?;
        Ok(ComponentProperty {
            name,
            ..self.clone()
        })
    }

    /// 같은 id·이름·타입으로 default만 바꾼 사본을 검증해 만듭니다.
    pub fn with_default(
        &self,
        default: ExpressionAst,
    ) -> Result<ComponentProperty, ComponentPropertyError> {
        let (default, default_value) = check_default(default, self.value_type())?;
        Ok(ComponentProperty {
            default,
            default_value,
            ..self.clone()
        })
    }

    /// 공개 여부만 바꾼 사본.
    pub fn with_visibility(&self, visibility: PropertyVisibility) -> ComponentProperty {
        ComponentProperty {
            visibility,
            ..self.clone()
        }
    }

    pub(super) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        put_str(buf, &self.name);
        put_u8(buf, self.visibility.tag());
        put_type_spec(buf, self.value_type());
        put_expression(buf, &self.default);
    }

    pub(super) fn decode_from(
        reader: &mut Reader<'_>,
    ) -> Result<ComponentProperty, ComponentPropertyError> {
        let id = ComponentPropertyId::new(reader.id()?);
        let name = reader.string()?;
        let visibility = PropertyVisibility::from_tag(reader.u8()?).ok_or(
            ComponentPropertyError::Malformed {
                reason: "알 수 없는 visibility tag",
            },
        )?;
        let value_type = reader.type_spec()?;
        let default = reader
            .expression_ast()?
            .map_err(ComponentPropertyError::Codec)?;
        ComponentProperty::new(id, name, value_type, default, visibility)
    }

    /// canonical binary bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩하고 생성 규칙으로 재검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<ComponentProperty, ComponentPropertyError> {
        let mut reader = Reader::new(bytes);
        let property = ComponentProperty::decode_from(&mut reader)?;
        reader.expect_end()?;
        Ok(property)
    }
}

fn check_default(
    default: ExpressionAst,
    value_type: TypeSpec,
) -> Result<(CheckedExpression, Option<TypedValue>), ComponentPropertyError> {
    let checked = CheckedExpression::closed(default, value_type)?;
    let none: BTreeMap<ReferenceTarget, Option<TypedValue>> = BTreeMap::new();
    let value = checked
        .evaluate(&none)
        .map_err(ComponentPropertyError::DefaultEvaluation)?;
    if !value_type.accepts(value.as_ref()) {
        // type checker가 대입 가능하다고 판정한 값은 선언 타입을 만족해야 합니다(방어적 검사).
        return Err(ComponentPropertyError::DefaultTypeMismatch {
            expected: value_type,
            found: checked.result_type(),
        });
    }
    Ok((checked, value))
}

impl ComponentEntry {
    /// property record.
    pub fn property(&self, property: ComponentPropertyId) -> Option<&ComponentProperty> {
        self.properties_map().get(&property)
    }

    /// property record(property id 오름차순).
    pub fn property_records(&self) -> impl Iterator<Item = &ComponentProperty> + '_ {
        self.properties_map().values()
    }

    /// 이름으로 property record를 찾습니다.
    pub fn property_named(&self, name: &str) -> Option<&ComponentProperty> {
        self.properties_map().values().find(|p| p.name() == name)
    }
}

impl ComponentLibrary {
    /// 컴포넌트에 property record를 추가합니다.
    ///
    /// property는 정의 Property 목록에 선언돼 있어야 하고, record가 중복되거나 같은 컴포넌트 안에서
    /// 이름이 겹칠 수 없습니다. 실패 시 library는 변경되지 않습니다.
    pub fn add_property(
        &mut self,
        component: ComponentRef,
        property: ComponentProperty,
    ) -> Result<(), ComponentPropertyError> {
        let entry = self
            .entry_mut(component)
            .ok_or(ComponentPropertyError::ComponentNotFound { component })?;
        if !entry.definition().properties().contains(&property.id()) {
            return Err(ComponentPropertyError::NotDeclared {
                property: property.id(),
            });
        }
        if entry.property(property.id()).is_some() {
            return Err(ComponentPropertyError::DuplicateRecord {
                property: property.id(),
            });
        }
        if entry.property_named(property.name()).is_some() {
            return Err(ComponentPropertyError::DuplicateName {
                name: property.name().to_string(),
            });
        }
        entry.properties_map_mut().insert(property.id(), property);
        Ok(())
    }

    /// 컴포넌트의 선언 Property가 모두 record를 갖는지 검사합니다.
    pub fn validate_properties(
        &self,
        component: ComponentRef,
    ) -> Result<(), ComponentPropertyError> {
        let entry = self
            .get(component)
            .ok_or(ComponentPropertyError::ComponentNotFound { component })?;
        for property in entry.definition().properties() {
            if entry.property(*property).is_none() {
                return Err(ComponentPropertyError::MissingRecord {
                    property: *property,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::typed_expr::test_util::{binary, literal, null, reference};
    use super::*;
    use crate::component::definition::{
        ComponentDefinition, ComponentDefinitionError, DefinitionNode, DefinitionTree,
    };
    use crate::node_kind::NodeKind;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::eval::EvalErrorKind;
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::{EnumTypeId, ExpressionType};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn pid(ts: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(stable(ts))
    }

    fn number(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).unwrap())
    }

    fn label(id: u64, name: &str) -> ComponentProperty {
        ComponentProperty::new(
            pid(id),
            name.to_string(),
            TypeSpec::required(ExpressionType::String),
            literal(TypedValue::String("확인".into())),
            PropertyVisibility::Public,
        )
        .unwrap()
    }

    #[test]
    fn default_type_mismatch_is_rejected() {
        // 검증 기준(IX.1.2): default type mismatch 거부.
        let err = ComponentProperty::new(
            pid(500),
            "label".into(),
            TypeSpec::required(ExpressionType::String),
            literal(number(1.0)),
            PropertyVisibility::Public,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ComponentPropertyError::DefaultTypeMismatch {
                expected: TypeSpec::required(ExpressionType::String),
                found: TypeSpec::required(ExpressionType::Number),
            }
        );
        // enum 타입 ID가 다르면 mismatch.
        let other_enum = TypedValue::Enum(stellar_core_types::typed_value::EnumValue::new(9, 0));
        assert!(matches!(
            ComponentProperty::new(
                pid(501),
                "size".into(),
                TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(3))),
                literal(other_enum),
                PropertyVisibility::Public,
            ),
            Err(ComponentPropertyError::DefaultTypeMismatch { .. })
        ));
        // nullable default(null)는 필수 타입 property에 둘 수 없습니다.
        assert_eq!(
            ComponentProperty::new(
                pid(502),
                "count".into(),
                TypeSpec::required(ExpressionType::Number),
                null(ExpressionType::Number),
                PropertyVisibility::Public,
            )
            .unwrap_err(),
            ComponentPropertyError::DefaultTypeMismatch {
                expected: TypeSpec::required(ExpressionType::Number),
                found: TypeSpec::optional(ExpressionType::Number),
            }
        );
        // nullable property는 null default를 가질 수 있습니다.
        let maybe = ComponentProperty::new(
            pid(503),
            "badge".into(),
            TypeSpec::optional(ExpressionType::Number),
            null(ExpressionType::Number),
            PropertyVisibility::Private,
        )
        .unwrap();
        assert_eq!(maybe.default_value(), None);
        // 일치하는 default는 통과하고 평가 값을 캐시합니다.
        let color = Color::new(ColorSpace::Srgb, 0.1, 0.2, 0.3, 1.0).unwrap();
        let fill = ComponentProperty::new(
            pid(504),
            "fill".into(),
            TypeSpec::required(ExpressionType::Color),
            literal(TypedValue::Color(color)),
            PropertyVisibility::Public,
        )
        .unwrap();
        assert_eq!(fill.default_value(), Some(&TypedValue::Color(color)));
        assert!(fill.is_public());
    }

    #[test]
    fn default_must_be_closed_and_evaluable() {
        // 참조를 가진 default는 거부(orphan fallback이 항상 평가 가능해야 함).
        let target = ReferenceTarget::ComponentProperty(stable(42));
        assert_eq!(
            ComponentProperty::new(
                pid(500),
                "x".into(),
                TypeSpec::required(ExpressionType::Number),
                reference(target.clone()),
                PropertyVisibility::Public,
            )
            .unwrap_err(),
            ComponentPropertyError::DefaultNotClosed { target }
        );
        // 평가 오류(0으로 나누기)도 유효한 기본값이 아닙니다.
        let err = ComponentProperty::new(
            pid(501),
            "ratio".into(),
            TypeSpec::required(ExpressionType::Number),
            binary(BinaryOp::Divide, literal(number(1.0)), literal(number(0.0))),
            PropertyVisibility::Public,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ComponentPropertyError::DefaultEvaluation(EvalError {
                kind: EvalErrorKind::DivisionByZero,
                ..
            })
        ));
        // 계산식 default는 허용.
        let sum = ComponentProperty::new(
            pid(502),
            "sum".into(),
            TypeSpec::optional(ExpressionType::Number),
            binary(BinaryOp::Add, literal(number(1.0)), literal(number(2.0))),
            PropertyVisibility::Public,
        )
        .unwrap();
        assert_eq!(sum.default_value(), Some(&number(3.0)));
    }

    #[test]
    fn names_are_validated() {
        for bad in ["", "   ", "a\nb"] {
            assert!(matches!(
                validate_property_name(bad),
                Err(ComponentPropertyError::InvalidName { .. })
            ));
        }
        assert!(
            validate_property_name(&"가".repeat(100)).is_err(),
            "300 bytes"
        );
        assert!(validate_property_name("배경색").is_ok());
        assert!(label(500, "label").renamed(String::new()).is_err());
        let renamed = label(500, "label").renamed("title".into()).unwrap();
        assert_eq!(renamed.name(), "title");
        assert_eq!(renamed.id(), pid(500));
    }

    #[test]
    fn property_round_trips_and_default_is_required_in_bytes() {
        let property = label(500, "label");
        let bytes = property.encode();
        let decoded = ComponentProperty::decode(&bytes).unwrap();
        assert_eq!(decoded, property);
        assert_eq!(decoded.encode(), bytes);
        for cut in [0, 16, 20, bytes.len() - 1] {
            assert!(
                ComponentProperty::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        // default 없는(빈 AST) record는 디코딩이 거부합니다 — 생성 불가 규칙과 같습니다.
        let mut no_default = Vec::new();
        put_id(&mut no_default, stable(500));
        put_str(&mut no_default, "label");
        put_u8(&mut no_default, PropertyVisibility::Public.tag());
        put_type_spec(&mut no_default, TypeSpec::required(ExpressionType::String));
        super::super::codec::put_bytes(&mut no_default, &[]);
        assert!(matches!(
            ComponentProperty::decode(&no_default),
            Err(ComponentPropertyError::Codec(_))
        ));
        // 저장된 default가 타입과 맞지 않으면(파일 조작) 디코딩 거부.
        let mut tampered = Vec::new();
        put_id(&mut tampered, stable(500));
        put_str(&mut tampered, "label");
        put_u8(&mut tampered, PropertyVisibility::Public.tag());
        put_type_spec(&mut tampered, TypeSpec::required(ExpressionType::Boolean));
        super::super::codec::put_bytes(
            &mut tampered,
            &literal(TypedValue::String("확인".into()))
                .to_canonical_bytes()
                .unwrap(),
        );
        assert!(matches!(
            ComponentProperty::decode(&tampered),
            Err(ComponentPropertyError::DefaultTypeMismatch { .. })
        ));
        // 알 수 없는 visibility tag.
        let mut bad_tag = bytes.clone();
        let pos = 16 + 4 + "label".len();
        bad_tag[pos] = 0x7F;
        assert!(ComponentProperty::decode(&bad_tag).is_err());
    }

    fn library() -> (ComponentLibrary, ComponentRef) {
        let component = ComponentRef::new(stable(1));
        let mut tree = DefinitionTree::new(component);
        tree.insert(DefinitionNode {
            id: stable(100),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: Vec::new(),
        })
        .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(1), "Button".into(), NodeRef::new(stable(100)));
        definition.add_property(pid(500)).unwrap();
        definition.add_property(pid(501)).unwrap();
        let mut library = ComponentLibrary::new();
        library.insert(definition, tree).unwrap();
        (library, component)
    }

    #[test]
    fn library_registers_declared_properties_with_unique_names() {
        let (mut library, button) = library();
        assert_eq!(
            library.add_property(button, label(599, "x")).unwrap_err(),
            ComponentPropertyError::NotDeclared { property: pid(599) }
        );
        library.add_property(button, label(500, "label")).unwrap();
        assert_eq!(
            library
                .add_property(button, label(500, "other"))
                .unwrap_err(),
            ComponentPropertyError::DuplicateRecord { property: pid(500) }
        );
        assert_eq!(
            library
                .add_property(button, label(501, "label"))
                .unwrap_err(),
            ComponentPropertyError::DuplicateName {
                name: "label".into()
            }
        );
        // 선언 property 501에 record가 없으면 library 디코딩이 거부합니다.
        assert_eq!(
            library.validate_properties(button).unwrap_err(),
            ComponentPropertyError::MissingRecord { property: pid(501) }
        );
        assert_eq!(
            ComponentLibrary::decode(&library.encode()).unwrap_err(),
            ComponentDefinitionError::Property(ComponentPropertyError::MissingRecord {
                property: pid(501)
            })
        );
        library.add_property(button, label(501, "title")).unwrap();
        let bytes = library.encode();
        let decoded = ComponentLibrary::decode(&bytes).unwrap();
        assert_eq!(decoded, library);
        assert_eq!(decoded.encode(), bytes);
        let entry = decoded.get(button).unwrap();
        assert_eq!(entry.property_named("title").unwrap().id(), pid(501));
        assert_eq!(entry.property_records().count(), 2);
    }
}
