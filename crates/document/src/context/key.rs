//! `ContextKey` resource (XXX.7.1, ADR-0025 결정 3.1, ADR-0099 결정 1).
//!
//! key 하나는 StableId, 표시 이름, 값 타입([`TypeSpec`]), **필수** default expression, scope(프로젝트·라이브러리·
//! 문서)를 가집니다.
//!
//! - default는 Component Property default와 같은 **닫힌 식**입니다([`CheckedExpression::closed`], IX.1.2). 참조가
//!   있으면 거부하고, root 타입이 선언 타입에 맞아야 합니다. default 없는 key는 만들 수 없습니다(생성자가 default를
//!   요구하고, 디코더는 default 자리가 비면 거부합니다).
//! - 참조는 StableId로 합니다(VIII.5.2). 이름은 표시·검색용이라 rename해도 소비·provision 참조는 그대로입니다.
//!
//! canonical bytes(ADR-0099 결정 2):
//!
//! ```text
//! key   := id[16] ++ str(name) ++ scope ++ type_spec ++ bytes(default canonical AST)
//! scope := 0x01 (project) | 0x02 ++ library_id[16] | 0x03 ++ document_id[16]
//! ```

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_expressions::ast::ExpressionAst;
use stellar_expressions::types::TypeSpec;

use crate::component::codec::{put_id, put_str, put_u8, Malformed, Reader};
use crate::component::typed_expr::{put_type_spec, CheckedExpression, ExpressionCheckError};

/// key가 속한 scope(토큰 scope와 같은 세 종류, VIII.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextScope {
    /// 프로젝트 공통 key.
    Project,
    /// 외부 라이브러리 key. 값은 dependencies의 LibraryId와 같은 `StableId`입니다.
    Library(StableId),
    /// 문서 전용 key. 값은 문서 `StableId`입니다.
    Document(StableId),
}

impl ContextScope {
    /// canonical bytes(key bytes 안 scope 형식). semantic snapshot 투영(XXX.7.10)이 같은 형식을 씁니다.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// [`ContextScope::to_bytes`]를 읽습니다.
    pub fn from_bytes(bytes: &[u8]) -> Option<ContextScope> {
        let mut reader = Reader::new(bytes);
        let scope = ContextScope::decode_from(&mut reader).ok()?;
        reader.expect_end().ok()?;
        Some(scope)
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ContextScope::Project => put_u8(buf, 0x01),
            ContextScope::Library(id) => {
                put_u8(buf, 0x02);
                put_id(buf, *id);
            }
            ContextScope::Document(id) => {
                put_u8(buf, 0x03);
                put_id(buf, *id);
            }
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<ContextScope, Malformed> {
        match reader.u8()? {
            0x01 => Ok(ContextScope::Project),
            0x02 => Ok(ContextScope::Library(reader.id()?)),
            0x03 => Ok(ContextScope::Document(reader.id()?)),
            _ => Err(Malformed("알 수 없는 Context scope tag")),
        }
    }
}

impl fmt::Display for ContextScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextScope::Project => f.write_str("project"),
            ContextScope::Library(id) => write!(f, "library {id}"),
            ContextScope::Document(id) => write!(f, "document {id}"),
        }
    }
}

/// ContextKey 생성·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextKeyError {
    /// NIL은 key id로 쓸 수 없습니다.
    NilId,
    /// 이름이 비었거나 앞뒤 공백이 있습니다.
    InvalidName {
        /// 이름.
        name: String,
    },
    /// default expression이 닫힌 식이 아니거나 선언 타입에 맞지 않습니다.
    Default(ExpressionCheckError),
    /// default expression이 없습니다(default 없는 key는 만들 수 없음).
    MissingDefault,
    /// canonical bytes가 잘못됐습니다.
    Malformed(&'static str),
}

impl fmt::Display for ContextKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextKeyError::NilId => f.write_str("NIL은 ContextKey id로 쓸 수 없습니다"),
            ContextKeyError::InvalidName { name } => {
                write!(
                    f,
                    "ContextKey 이름 {name:?}은 비지 않고 앞뒤 공백이 없어야 합니다"
                )
            }
            ContextKeyError::Default(error) => write!(f, "ContextKey default: {error}"),
            ContextKeyError::MissingDefault => {
                f.write_str("ContextKey에는 default expression이 있어야 합니다")
            }
            ContextKeyError::Malformed(reason) => write!(f, "ContextKey bytes: {reason}"),
        }
    }
}

impl std::error::Error for ContextKeyError {}

impl From<Malformed> for ContextKeyError {
    fn from(value: Malformed) -> ContextKeyError {
        ContextKeyError::Malformed(value.0)
    }
}

/// 이름 규칙: 비어 있지 않고 앞뒤 공백이 없습니다(토큰·Property 이름과 같은 규칙).
pub(crate) fn check_name(name: &str) -> Result<(), ContextKeyError> {
    if name.is_empty() || name.trim() != name {
        return Err(ContextKeyError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// TypeSpec canonical bytes(nullable u8 + 값 타입 tag u8 [+ Enum이면 enum id u32 LE]). key bytes 안의 형식과 같고 command
/// codec(XXX.7.8)이 같은 함수를 씁니다.
pub fn type_spec_bytes(spec: TypeSpec) -> Vec<u8> {
    let mut buf = Vec::new();
    put_type_spec(&mut buf, spec);
    buf
}

/// [`type_spec_bytes`]를 읽습니다. 남는 bytes가 있거나 모르는 tag면 `None`.
pub fn type_spec_from_bytes(bytes: &[u8]) -> Option<TypeSpec> {
    let mut reader = Reader::new(bytes);
    let spec = reader.type_spec().ok()?;
    reader.expect_end().ok()?;
    Some(spec)
}

/// `ContextKey` resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextKey {
    id: StableId,
    name: String,
    scope: ContextScope,
    default: CheckedExpression,
}

impl ContextKey {
    /// key를 만듭니다. default는 필수이며 닫힌 식이고 `value_type`에 맞아야 합니다.
    pub fn new(
        id: StableId,
        name: impl Into<String>,
        scope: ContextScope,
        value_type: TypeSpec,
        default: ExpressionAst,
    ) -> Result<ContextKey, ContextKeyError> {
        if id == StableId::NIL {
            return Err(ContextKeyError::NilId);
        }
        let name = name.into();
        check_name(&name)?;
        let default =
            CheckedExpression::closed(default, value_type).map_err(ContextKeyError::Default)?;
        Ok(ContextKey {
            id,
            name,
            scope,
            default,
        })
    }

    /// StableId(참조 식별자).
    pub fn id(&self) -> StableId {
        self.id
    }

    /// 표시 이름(식별에 쓰지 않음).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// scope.
    pub fn scope(&self) -> ContextScope {
        self.scope
    }

    /// 값 타입.
    pub fn value_type(&self) -> TypeSpec {
        self.default.declared_type()
    }

    /// 검사를 통과한 default expression.
    pub fn default_expression(&self) -> &CheckedExpression {
        &self.default
    }

    /// 이름을 바꾼 key(id·타입·default·scope 그대로).
    pub fn renamed(&self, name: impl Into<String>) -> Result<ContextKey, ContextKeyError> {
        let name = name.into();
        check_name(&name)?;
        Ok(ContextKey {
            name,
            ..self.clone()
        })
    }

    /// default를 바꾼 key. 값 타입은 `value_type`으로 바뀌며 같은 검사를 다시 거칩니다.
    pub fn with_default(
        &self,
        value_type: TypeSpec,
        default: ExpressionAst,
    ) -> Result<ContextKey, ContextKeyError> {
        ContextKey::new(self.id, self.name.clone(), self.scope, value_type, default)
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id);
        put_str(buf, &self.name);
        self.scope.encode_into(buf);
        put_type_spec(buf, self.value_type());
        crate::component::typed_expr::put_expression(buf, &self.default);
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<ContextKey, ContextKeyError> {
        let id = reader.id()?;
        let name = reader.string()?;
        let scope = ContextScope::decode_from(reader)?;
        let value_type = reader.type_spec()?;
        let bytes = reader.bytes()?;
        if bytes.is_empty() {
            return Err(ContextKeyError::MissingDefault);
        }
        let default = ExpressionAst::from_canonical_bytes(bytes)
            .map_err(|error| ContextKeyError::Default(ExpressionCheckError::Codec(error)))?;
        ContextKey::new(id, name, scope, value_type, default)
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// canonical bytes에서 디코딩합니다. 다시 인코딩한 bytes가 입력과 달라도 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ContextKey, ContextKeyError> {
        let mut reader = Reader::new(bytes);
        let key = ContextKey::decode_from(&mut reader)?;
        reader.expect_end()?;
        if key.encode() != bytes {
            return Err(ContextKeyError::Malformed("canonical bytes가 아닙니다"));
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::typed_expr::test_util::{literal, reference};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::ast::ReferenceTarget;
    use stellar_expressions::types::ExpressionType;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0c0c, n)
    }

    fn blue() -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 1.0, 1.0).unwrap())
    }

    fn color() -> TypeSpec {
        TypeSpec::required(ExpressionType::Color)
    }

    #[test]
    fn key_requires_closed_default_of_declared_type() {
        // 검증 기준(XXX.7.1): default 누락·type mismatch·열린 식 거부.
        let key = ContextKey::new(
            sid(1),
            "theme accent",
            ContextScope::Document(sid(9)),
            color(),
            literal(blue()),
        )
        .unwrap();
        assert_eq!(key.value_type(), color());
        assert!(matches!(
            ContextKey::new(
                sid(1),
                "x",
                ContextScope::Project,
                color(),
                literal(TypedValue::Number(
                    stellar_core_types::value::FiniteF32::new(1.0).unwrap()
                )),
            ),
            Err(ContextKeyError::Default(
                ExpressionCheckError::TypeMismatch { .. }
            ))
        ));
        let open = reference(ReferenceTarget::Token(TokenRef::new(sid(5))));
        assert!(matches!(
            ContextKey::new(sid(1), "x", ContextScope::Project, color(), open),
            Err(ContextKeyError::Default(
                ExpressionCheckError::ForbiddenReference(_)
            ))
        ));
        // 다른 key를 읽는 default도 열린 식입니다.
        let context = reference(ReferenceTarget::Context(sid(2)));
        assert!(matches!(
            ContextKey::new(sid(1), "x", ContextScope::Project, color(), context),
            Err(ContextKeyError::Default(
                ExpressionCheckError::ForbiddenReference(ReferenceTarget::Context(_))
            ))
        ));
        assert_eq!(
            ContextKey::new(
                StableId::NIL,
                "x",
                ContextScope::Project,
                color(),
                literal(blue())
            ),
            Err(ContextKeyError::NilId)
        );
        assert!(matches!(
            ContextKey::new(
                sid(1),
                " x",
                ContextScope::Project,
                color(),
                literal(blue())
            ),
            Err(ContextKeyError::InvalidName { .. })
        ));
        // default 자리가 빈 bytes는 거부(default 없는 key 없음).
        let mut bytes = Vec::new();
        put_id(&mut bytes, sid(1));
        put_str(&mut bytes, "x");
        ContextScope::Project.encode_into(&mut bytes);
        put_type_spec(&mut bytes, color());
        crate::component::codec::put_bytes(&mut bytes, &[]);
        assert_eq!(
            ContextKey::decode(&bytes),
            Err(ContextKeyError::MissingDefault)
        );
    }

    #[test]
    fn key_round_trips_and_rename_keeps_identity() {
        // 검증 기준(XXX.7.1): round-trip, rename 후 참조 유지(참조는 StableId).
        for scope in [
            ContextScope::Project,
            ContextScope::Library(sid(7)),
            ContextScope::Document(sid(8)),
        ] {
            let key = ContextKey::new(sid(3), "density", scope, color(), literal(blue())).unwrap();
            let bytes = key.encode();
            assert_eq!(ContextKey::decode(&bytes).unwrap(), key);
            let renamed = key.renamed("밀도").unwrap();
            assert_eq!(renamed.id(), key.id());
            assert_eq!(renamed.name(), "밀도");
            assert_eq!(renamed.default_expression(), key.default_expression());
            // 소비 참조는 key StableId라 이름과 무관합니다.
            assert_eq!(
                ReferenceTarget::Context(renamed.id()),
                ReferenceTarget::Context(key.id())
            );
        }
        let mut trailing =
            ContextKey::new(sid(3), "x", ContextScope::Project, color(), literal(blue()))
                .unwrap()
                .encode();
        trailing.push(0);
        assert!(ContextKey::decode(&trailing).is_err());
    }
}
