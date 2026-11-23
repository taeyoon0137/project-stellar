//! token set 원본 v4(`design/tokens/<slug>.stellar.yaml`, ADR-0077 결정 1·3, ADR-0131 결정 2).
//!
//! token 모델(`stellar-bindings`의 `TokenSet`)은 crate 정책상 project가 의존할 수 없는 crate에 있습니다. 그래서
//! 이 모듈은 파일 의미를 core 값으로만 정의합니다. token 하나는 둘 중 하나입니다.
//!
//! - 리터럴 token([`TokenSource`]): id·이름·기본 표현식 node id·리터럴 값. 값 타입은 리터럴에서 정해집니다.
//! - 계산 token([`ComputedTokenSource`]): id·이름·선언 타입·기본 표현식 AST. 리터럴 하나로 쓸 수 없는 기본
//!   표현식(다른 token 참조·연산·함수)을 담습니다. 파일에는 `value_type`과 canonical AST bytes hex(`ast`)로 씁니다
//!   (Component Context 원본과 같은 표현식 문법, ADR-0099). 리터럴 하나로 쓸 수 있는 식은 리터럴 token으로만 씁니다
//!   (같은 의미의 표기는 하나).
//!
//! 두 종류는 파일의 `tokens` 목록 하나에 id 오름차순으로 섞여 있고 이름은 set 안에서 유일합니다. 표현식 type 검사와
//! `TokenSet`과의 변환은 참조 타입을 아는 bindings 계층이 합니다.

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::expressions::ast::{ExprNode, ExpressionAst, Literal, NodePayload};
use stellar_document::expressions::types::TypeSpec;

use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::context::{ast_expression, parse_type, type_name};
use super::css::{self, invalid, unsupported};
use super::node::{parse_typed_value, typed_value};
use super::VERSION;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

/// token set 파일 형식 이름.
pub const TOKENS_FORMAT: &str = "stellar-tokens";

/// 리터럴 기본값을 가진 token 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSource {
    /// token StableId(`TokenRef`).
    pub id: StableId,
    /// set 안에서 유일한 표시 이름.
    pub name: String,
    /// 기본 표현식 root(리터럴) node id.
    pub expression: StableId,
    /// 기본값.
    pub value: TypedValue,
}

/// 리터럴 하나로 쓸 수 없는 기본 표현식을 가진 token 하나(ADR-0131 결정 2).
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTokenSource {
    /// token StableId(`TokenRef`).
    pub id: StableId,
    /// set 안에서 유일한 표시 이름.
    pub name: String,
    /// 선언 타입.
    pub value_type: TypeSpec,
    /// 기본 표현식.
    pub default: ExpressionAst,
}

/// token set 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSetSource {
    /// set StableId.
    pub id: StableId,
    /// 파일 이름 slug의 바탕이 되는 표시 이름.
    pub name: String,
    /// 리터럴 token 목록(id 오름차순).
    pub tokens: Vec<TokenSource>,
    /// 계산 token 목록(id 오름차순). 리터럴 token과 id·이름이 겹치지 않습니다.
    pub computed: Vec<ComputedTokenSource>,
}

/// 식이 리터럴 token으로 쓸 수 있는 리터럴 하나인지.
fn is_literal_token(ast: &ExpressionAst) -> bool {
    ast.len() == 1
        && matches!(
            ast.node(ast.root()).map(ExprNode::payload),
            Some(NodePayload::Literal(Literal::Value(value))) if typed_value(value).is_ok()
        )
}

impl TokenSetSource {
    /// set이 담은 모든 token id(리터럴·계산, 오름차순).
    pub fn token_ids(&self) -> Vec<StableId> {
        let mut ids: Vec<StableId> = self
            .tokens
            .iter()
            .map(|token| token.id)
            .chain(self.computed.iter().map(|token| token.id))
            .collect();
        ids.sort();
        ids
    }

    fn validate(&self) -> Result<(), TextSourceError> {
        let literal: Vec<StableId> = self.tokens.iter().map(|t| t.id).collect();
        let computed: Vec<StableId> = self.computed.iter().map(|t| t.id).collect();
        for list in [&literal, &computed] {
            if list.windows(2).any(|w| w[0] >= w[1]) {
                return Err(invalid("token은 id 오름차순, 중복 없이 씁니다"));
            }
        }
        if self.token_ids().windows(2).any(|w| w[0] == w[1]) {
            return Err(invalid("리터럴 token과 계산 token의 id가 겹칩니다"));
        }
        let mut names = BTreeSet::new();
        let all_names = self
            .tokens
            .iter()
            .map(|t| t.name.as_str())
            .chain(self.computed.iter().map(|t| t.name.as_str()));
        for name in all_names {
            if name.is_empty() || name.trim() != name {
                return Err(invalid("token 이름은 비지 않고 앞뒤 공백이 없어야 합니다"));
            }
            if !names.insert(name) {
                return Err(invalid(format!("set 안에서 중복된 token 이름 {name}")));
            }
        }
        if let Some(token) = self
            .computed
            .iter()
            .find(|token| is_literal_token(&token.default))
        {
            return Err(invalid(format!(
                "리터럴 하나인 기본 표현식은 리터럴 token으로 씁니다: {}",
                token.name
            )));
        }
        Ok(())
    }
}

/// token set을 canonical 텍스트로 씁니다. 다시 읽은 set이 같을 때만 씁니다(손실 저장 없음).
pub fn encode(set: &TokenSetSource) -> Result<String, TextSourceError> {
    set.validate()?;
    let mut items: Vec<(StableId, YamlValue)> = set
        .tokens
        .iter()
        .map(|token| {
            let (kind, value) = typed_value(&token.value)?;
            Ok((
                token.id,
                mapping([
                    ("id", scalar(css::id(token.id))),
                    ("name", scalar(&token.name)),
                    ("expression", scalar(css::id(token.expression))),
                    ("type", scalar(kind)),
                    ("value", scalar(value)),
                ]),
            ))
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    for token in &set.computed {
        let bytes = token.default.to_canonical_bytes().map_err(invalid)?;
        items.push((
            token.id,
            mapping([
                ("id", scalar(css::id(token.id))),
                ("name", scalar(&token.name)),
                ("value_type", scalar(type_name(token.value_type))),
                ("ast", scalar(css::hex(&bytes))),
            ]),
        ));
    }
    items.sort_by_key(|(id, _)| *id);
    let tokens: Vec<YamlValue> = items.into_iter().map(|(_, item)| item).collect();
    let value = mapping([
        ("format", scalar(TOKENS_FORMAT)),
        ("schema_version", scalar(VERSION)),
        (
            "set",
            mapping([("id", scalar(css::id(set.id))), ("name", scalar(&set.name))]),
        ),
        ("tokens", YamlValue::Sequence(tokens)),
    ]);
    let out = write_canonical_checked(&value).map_err(invalid)?;
    if decode(&out)? != *set {
        return Err(unsupported(
            "token set 원본으로 정확히 복원할 수 없는 의미가 있습니다",
        ));
    }
    Ok(out)
}

/// token set 텍스트를 읽습니다.
pub fn decode(source: &str) -> Result<TokenSetSource, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    fields(&value, &["format", "schema_version", "set", "tokens"])?;
    if text(field(&value, "format")?)? != TOKENS_FORMAT {
        return Err(unsupported("token set 형식"));
    }
    if text(field(&value, "schema_version")?)? != VERSION {
        return Err(unsupported("token set 버전"));
    }
    let set = field(&value, "set")?;
    fields(set, &["id", "name"])?;
    let mut tokens = Vec::new();
    let mut computed = Vec::new();
    let mut last = None;
    for item in field(&value, "tokens")?
        .as_sequence()
        .ok_or_else(|| invalid("tokens는 sequence여야 합니다"))?
    {
        let get = |key| text(field(item, key)?);
        let id = css::parse_id(get("id")?)?;
        if last.is_some_and(|prev| prev >= id) {
            return Err(invalid("token은 id 오름차순, 중복 없이 씁니다"));
        }
        last = Some(id);
        if item.get("ast").is_some() {
            fields(item, &["id", "name", "value_type", "ast"])?;
            let default = ast_expression(get("ast")?)?;
            if css::hex(&default.to_canonical_bytes().map_err(invalid)?) != get("ast")? {
                return Err(invalid("token ast는 canonical bytes hex여야 합니다"));
            }
            computed.push(ComputedTokenSource {
                id,
                name: get("name")?.to_owned(),
                value_type: parse_type(get("value_type")?)?,
                default,
            });
        } else {
            fields(item, &["id", "name", "expression", "type", "value"])?;
            tokens.push(TokenSource {
                id,
                name: get("name")?.to_owned(),
                expression: css::parse_id(get("expression")?)?,
                value: parse_typed_value(get("type")?, get("value")?)?,
            });
        }
    }
    let set = TokenSetSource {
        id: css::parse_id(text(field(set, "id")?)?)?,
        name: text(field(set, "name")?)?.to_owned(),
        tokens,
        computed,
    };
    set.validate()?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Length;
    use stellar_document::expressions::ast::{ExprNodeId, ReferenceTarget};
    use stellar_document::expressions::types::ExpressionType;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn literal_token_sets_round_trip_and_duplicates_are_rejected() {
        let set = TokenSetSource {
            id: sid(1),
            name: "Brand".into(),
            tokens: vec![
                TokenSource {
                    id: sid(2),
                    name: "color/primary".into(),
                    expression: sid(12),
                    value: TypedValue::Color(
                        Color::new(ColorSpace::DisplayP3, 0.1, 0.2, 0.9, 1.0).unwrap(),
                    ),
                },
                TokenSource {
                    id: sid(3),
                    name: "space/4".into(),
                    expression: sid(13),
                    value: TypedValue::Length(Length::new(16.0).unwrap()),
                },
            ],
            computed: Vec::new(),
        };
        let text = encode(&set).unwrap();
        assert!(text.contains("    value: \"16px\"\n"), "{text}");
        assert_eq!(decode(&text).unwrap(), set);
        for bad in [
            text.replacen("name: \"space/4\"", "name: \"color/primary\"", 1),
            text.replacen("type: length", "type: gradient", 1),
            text.replacen("format: stellar-tokens", "format: other", 1),
        ] {
            assert_ne!(bad, text);
            assert!(decode(&bad).is_err(), "{bad}");
        }
    }

    fn token_reference(seed: u64, target: StableId) -> ExpressionAst {
        let root = ExprNodeId::new(sid(seed));
        ExpressionAst::new(
            root,
            vec![ExprNode::reference(
                root,
                ReferenceTarget::Token(stellar_core_types::reference::TokenRef::new(target)),
            )],
        )
        .unwrap()
    }

    /// 리터럴 하나로 쓸 수 없는 기본 표현식(다른 token 참조)은 계산 token으로 왕복합니다. 리터럴 하나인 식을 계산
    /// token으로 쓰거나, 리터럴 token과 id·이름이 겹치면 거부합니다(ADR-0131 결정 2).
    #[test]
    fn computed_tokens_round_trip_next_to_literal_tokens() {
        let set = TokenSetSource {
            id: sid(1),
            name: "Brand".into(),
            tokens: vec![TokenSource {
                id: sid(2),
                name: "space/4".into(),
                expression: sid(12),
                value: TypedValue::Length(Length::new(16.0).unwrap()),
            }],
            computed: vec![ComputedTokenSource {
                id: sid(3),
                name: "space/gutter".into(),
                value_type: TypeSpec::required(ExpressionType::Length),
                default: token_reference(13, sid(2)),
            }],
        };
        let text = encode(&set).unwrap();
        assert!(text.contains("value_type: length"), "{text}");
        assert!(text.contains("ast: "), "{text}");
        assert_eq!(decode(&text).unwrap(), set);
        assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text, "같은 bytes");
        assert_eq!(set.token_ids(), vec![sid(2), sid(3)]);

        let mut literal = set.clone();
        literal.computed[0].default = {
            let root = ExprNodeId::new(sid(13));
            ExpressionAst::new(
                root,
                vec![ExprNode::literal(
                    root,
                    TypedValue::Length(Length::new(8.0).unwrap()),
                )],
            )
            .unwrap()
        };
        assert!(
            encode(&literal).is_err(),
            "리터럴 하나는 리터럴 token으로만 씁니다"
        );
        let mut clash = set.clone();
        clash.computed[0].name = "space/4".into();
        assert!(encode(&clash).is_err());
        let mut same_id = set.clone();
        same_id.computed[0].id = sid(2);
        assert!(encode(&same_id).is_err());
        // 모르는 field·잘못된 hex는 거부합니다.
        assert!(decode(&text.replacen("value_type:", "kind:", 1)).is_err());
        assert!(decode(&text.replacen("ast: ", "ast: zz", 1)).is_err());
    }
}
