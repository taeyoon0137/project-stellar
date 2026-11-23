//! token set 원본 v4(`design/tokens/<slug>.stellar.yaml`, ADR-0077 결정 1·3).
//!
//! token 모델(`stellar-bindings`의 `TokenSet`)은 crate 정책상 project가 의존할 수 없는 crate에 있습니다. 그래서
//! 이 모듈은 파일 의미를 core 값으로만 정의합니다: token 하나는 id·이름·기본 표현식 node id·리터럴 값이고, 값
//! 타입은 리터럴에서 정해집니다. 리터럴이 아닌 기본 표현식은 이 형식이 표현하지 않습니다. `TokenSet`과의 변환은
//! bindings를 쓰는 계층이 합니다.

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
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

/// token set 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSetSource {
    /// set StableId.
    pub id: StableId,
    /// 파일 이름 slug의 바탕이 되는 표시 이름.
    pub name: String,
    /// token 목록(id 오름차순).
    pub tokens: Vec<TokenSource>,
}

impl TokenSetSource {
    fn validate(&self) -> Result<(), TextSourceError> {
        let mut names = BTreeSet::new();
        let mut last = None;
        for token in &self.tokens {
            if last.is_some_and(|prev| prev >= token.id) {
                return Err(invalid("token은 id 오름차순, 중복 없이 씁니다"));
            }
            last = Some(token.id);
            if token.name.is_empty() || token.name.trim() != token.name {
                return Err(invalid("token 이름은 비지 않고 앞뒤 공백이 없어야 합니다"));
            }
            if !names.insert(token.name.as_str()) {
                return Err(invalid(format!(
                    "set 안에서 중복된 token 이름 {}",
                    token.name
                )));
            }
        }
        Ok(())
    }
}

/// token set을 canonical 텍스트로 씁니다.
pub fn encode(set: &TokenSetSource) -> Result<String, TextSourceError> {
    set.validate()?;
    let tokens = set
        .tokens
        .iter()
        .map(|token| {
            let (kind, value) = typed_value(&token.value)?;
            Ok(mapping([
                ("id", scalar(css::id(token.id))),
                ("name", scalar(&token.name)),
                ("expression", scalar(css::id(token.expression))),
                ("type", scalar(kind)),
                ("value", scalar(value)),
            ]))
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let value = mapping([
        ("format", scalar(TOKENS_FORMAT)),
        ("schema_version", scalar(VERSION)),
        (
            "set",
            mapping([("id", scalar(css::id(set.id))), ("name", scalar(&set.name))]),
        ),
        ("tokens", YamlValue::Sequence(tokens)),
    ]);
    write_canonical_checked(&value).map_err(invalid)
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
    let tokens = field(&value, "tokens")?
        .as_sequence()
        .ok_or_else(|| invalid("tokens는 sequence여야 합니다"))?
        .iter()
        .map(|item| {
            fields(item, &["id", "name", "expression", "type", "value"])?;
            let get = |key| text(field(item, key)?);
            Ok(TokenSource {
                id: css::parse_id(get("id")?)?,
                name: get("name")?.to_owned(),
                expression: css::parse_id(get("expression")?)?,
                value: parse_typed_value(get("type")?, get("value")?)?,
            })
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let set = TokenSetSource {
        id: css::parse_id(text(field(set, "id")?)?)?,
        name: text(field(set, "name")?)?.to_owned(),
        tokens,
    };
    set.validate()?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Length;

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
}
