//! Component Context 원본 v4(`design/context.stellar.yaml`, XXX.7·ADR-0099 결정 2).
//!
//! 문서 하나의 `ContextSet`(key·provision·requirement·Slot 공개 목록)을 한 파일에 씁니다. Context가 비어 있으면 파일을
//! 쓰지 않습니다. 목록은 모두 StableId 순입니다. expression은 root가 리터럴 하나이고 그 값을 token 원본과 같은 리터럴
//! 문법으로 쓸 수 있으면 `expression`(root node id)·`type`·`value`로, 그 밖이면 `ast`(canonical AST bytes hex)로 씁니다.
//! 손실 저장은 없습니다.

use stellar_core_types::ids::StableId;
use stellar_document::context::key::{ContextKey, ContextScope};
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::context::set::ContextSet;
use stellar_document::expressions::ast::{
    ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload,
};
use stellar_document::expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

use super::super::text_codec::{field, fields, mapping, scalar, text};

fn map_vec(entries: Vec<(&str, YamlValue)>) -> YamlValue {
    YamlValue::Mapping(entries.into_iter().map(|(k, v)| (scalar(k), v)).collect())
}
use super::super::text_source::TextSourceError;
use super::css::{self, invalid, unsupported};
use super::node::{parse_typed_value, typed_value};
use super::VERSION;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

/// Context 파일 경로(`design/` 기준).
pub const CONTEXT_PATH: &str = "context.stellar.yaml";
/// 파일 형식 이름.
pub const CONTEXT_FORMAT: &str = "stellar-context";

/// 값 타입 이름([`parse_type`]의 역, CLI·MCP 조회 출력 공용).
pub fn type_name(spec: TypeSpec) -> String {
    let base = match spec.value_type() {
        ExpressionType::Boolean => "boolean".to_owned(),
        ExpressionType::Number => "number".to_owned(),
        ExpressionType::String => "string".to_owned(),
        ExpressionType::Enum(id) => format!("enum({})", id.raw()),
        ExpressionType::Length => "length".to_owned(),
        ExpressionType::Percentage => "percentage".to_owned(),
        ExpressionType::Angle => "angle".to_owned(),
        ExpressionType::Duration => "duration".to_owned(),
        ExpressionType::Color => "color".to_owned(),
        ExpressionType::Gradient => "gradient".to_owned(),
        ExpressionType::Shadow => "shadow".to_owned(),
        ExpressionType::Typography => "typography".to_owned(),
        ExpressionType::ResourceRef => "resource-ref".to_owned(),
    };
    if spec.is_optional() {
        format!("{base}?")
    } else {
        base
    }
}

/// 값 타입 이름(`number`, `color?`, `enum(3)` …)을 읽습니다. 원본 `type` 필드 문법이며 CLI·MCP Context 편집 입력도 이
/// 함수를 씁니다(ADR-0114 결정 3).
pub fn parse_type(name: &str) -> Result<TypeSpec, TextSourceError> {
    let (base, optional) = match name.strip_suffix('?') {
        Some(base) => (base, true),
        None => (name, false),
    };
    let value_type = match base {
        "boolean" => ExpressionType::Boolean,
        "number" => ExpressionType::Number,
        "string" => ExpressionType::String,
        "length" => ExpressionType::Length,
        "percentage" => ExpressionType::Percentage,
        "angle" => ExpressionType::Angle,
        "duration" => ExpressionType::Duration,
        "color" => ExpressionType::Color,
        "gradient" => ExpressionType::Gradient,
        "shadow" => ExpressionType::Shadow,
        "typography" => ExpressionType::Typography,
        "resource-ref" => ExpressionType::ResourceRef,
        other => {
            let raw = other
                .strip_prefix("enum(")
                .and_then(|rest| rest.strip_suffix(')'))
                .ok_or_else(|| invalid(format!("모르는 값 타입 {other}")))?;
            ExpressionType::Enum(EnumTypeId::new(css::parse_u32(raw)?))
        }
    };
    Ok(if optional {
        TypeSpec::optional(value_type)
    } else {
        TypeSpec::required(value_type)
    })
}

/// scope 문장([`parse_scope`]의 역).
pub fn scope_text(scope: ContextScope) -> String {
    match scope {
        ContextScope::Project => "project".to_owned(),
        ContextScope::Library(id) => format!("library {}", css::id(id)),
        ContextScope::Document(id) => format!("document {}", css::id(id)),
    }
}

/// scope 문장(`project`, `library <id>`, `document <id>`)을 읽습니다(원본 문법, CLI·MCP 입력 공용).
pub fn parse_scope(value: &str) -> Result<ContextScope, TextSourceError> {
    if value == "project" {
        return Ok(ContextScope::Project);
    }
    match value.split_once(' ') {
        Some(("library", id)) => Ok(ContextScope::Library(css::parse_id(id)?)),
        Some(("document", id)) => Ok(ContextScope::Document(css::parse_id(id)?)),
        _ => Err(invalid(format!("모르는 Context scope {value}"))),
    }
}

/// provision owner 문장([`parse_owner`]의 역).
pub fn owner_text(owner: ContextOwner) -> String {
    match owner {
        ContextOwner::View(id) => format!("view {}", css::id(id)),
        ContextOwner::Component(id) => format!("component {}", css::id(id)),
    }
}

/// provision owner 문장(`view <id>`, `component <id>`)을 읽습니다(원본 문법, CLI·MCP 입력 공용).
pub fn parse_owner(value: &str) -> Result<ContextOwner, TextSourceError> {
    match value.split_once(' ') {
        Some(("view", id)) => Ok(ContextOwner::View(css::parse_id(id)?)),
        Some(("component", id)) => Ok(ContextOwner::Component(css::parse_id(id)?)),
        _ => Err(invalid(format!("모르는 provision owner {value}"))),
    }
}

/// expression 필드: 리터럴 형식 또는 `ast` hex.
pub(crate) fn expression_fields(
    ast: &ExpressionAst,
) -> Result<Vec<(&'static str, YamlValue)>, TextSourceError> {
    if ast.len() == 1 {
        if let Some(NodePayload::Literal(Literal::Value(value))) =
            ast.node(ast.root()).map(ExprNode::payload)
        {
            if let Ok((kind, text)) = typed_value(value) {
                return Ok(vec![
                    ("expression", scalar(css::id(ast.root().stable_id()))),
                    ("type", scalar(kind)),
                    ("value", scalar(text)),
                ]);
            }
        }
    }
    let bytes = ast.to_canonical_bytes().map_err(invalid)?;
    Ok(vec![("ast", scalar(css::hex(&bytes)))])
}

/// 리터럴 하나의 expression: root id `root`, 값 종류 `kind`와 리터럴 `value`(token 원본과 같은 리터럴 문법).
pub fn literal_expression(
    root: StableId,
    kind: &str,
    value: &str,
) -> Result<ExpressionAst, TextSourceError> {
    let root = ExprNodeId::new(root);
    let value = parse_typed_value(kind, value)?;
    ExpressionAst::new(root, vec![ExprNode::literal(root, value)]).map_err(invalid)
}

/// 값 하나의 리터럴 문장 `<타입> <리터럴>`(텍스트 원본 리터럴 문법). 리터럴로 쓰지 않는 타입이면 `None`입니다.
pub fn value_text(value: &stellar_core_types::typed_value::TypedValue) -> Option<String> {
    typed_value(value)
        .ok()
        .map(|(kind, text)| format!("{kind} {text}"))
}

/// 식의 입력 문장(CLI·MCP Context 편집 입력 문법, ADR-0114 결정 3): 리터럴 하나면 `<타입> <리터럴>`, key 읽기 하나면
/// `context <key>`, 그 밖이면 `ast <hex>`.
pub fn expression_text(ast: &ExpressionAst) -> String {
    if ast.len() == 1 {
        match ast.node(ast.root()).map(ExprNode::payload) {
            Some(NodePayload::Literal(Literal::Value(value))) => {
                if let Some(text) = value_text(value) {
                    return text;
                }
            }
            Some(NodePayload::Reference(
                stellar_document::expressions::ast::ReferenceTarget::Context(key),
            )) => return format!("context {}", css::id(*key)),
            _ => {}
        }
    }
    format!(
        "ast {}",
        css::hex(&ast.to_canonical_bytes().unwrap_or_default())
    )
}

/// canonical AST bytes의 hex(원본 `ast` 필드 문법).
pub fn ast_expression(hex: &str) -> Result<ExpressionAst, TextSourceError> {
    let bytes = css::parse_hex(hex)?;
    ExpressionAst::from_canonical_bytes(&bytes).map_err(invalid)
}

pub(crate) fn parse_expression(item: &YamlValue) -> Result<ExpressionAst, TextSourceError> {
    let present = |key| field(item, key).is_ok();
    if present("ast") {
        return ast_expression(text(field(item, "ast")?)?);
    }
    literal_expression(
        css::parse_id(text(field(item, "expression")?)?)?,
        text(field(item, "type")?)?,
        text(field(item, "value")?)?,
    )
}

pub(crate) fn expression_keys(item: &YamlValue) -> &'static [&'static str] {
    if field(item, "ast").is_ok() {
        &["ast"]
    } else {
        &["expression", "type", "value"]
    }
}

fn members(entries: impl Iterator<Item = (StableId, Vec<StableId>)>, owner: &str) -> YamlValue {
    YamlValue::Sequence(
        entries
            .map(|(id, keys)| {
                YamlValue::Mapping(vec![
                    (scalar(owner), scalar(css::id(id))),
                    (scalar("keys"), scalar(css::id_list(keys))),
                ])
            })
            .collect(),
    )
}

/// ContextSet을 canonical 텍스트로 씁니다.
pub fn encode(set: &ContextSet) -> Result<String, TextSourceError> {
    let keys = set
        .keys()
        .map(|key| {
            let mut entries = vec![
                ("id", scalar(css::id(key.id()))),
                ("name", scalar(key.name())),
                ("scope", scalar(scope_text(key.scope()))),
                ("value_type", scalar(type_name(key.value_type()))),
            ];
            let default = expression_fields(key.default_expression().ast())?;
            entries.extend(default.into_iter().map(|(k, v)| match k {
                "expression" => ("default_expression", v),
                "type" => ("default_type", v),
                "value" => ("default", v),
                _ => ("default_ast", v),
            }));
            Ok(map_vec(entries))
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let provisions = set
        .provisions()
        .map(|provision| {
            let mut entries = vec![
                ("id", scalar(css::id(provision.id()))),
                ("owner", scalar(owner_text(provision.owner()))),
                ("provider", scalar(css::id(provision.provider()))),
                ("key", scalar(css::id(provision.key()))),
            ];
            entries.extend(expression_fields(provision.value())?);
            Ok(map_vec(entries))
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let value = mapping([
        ("format", scalar(CONTEXT_FORMAT)),
        ("schema_version", scalar(VERSION)),
        ("keys", YamlValue::Sequence(keys)),
        ("provisions", YamlValue::Sequence(provisions)),
        (
            "requirements",
            members(
                set.all_requirements()
                    .map(|(id, keys)| (id, keys.iter().copied().collect())),
                "component",
            ),
        ),
        (
            "slots",
            members(
                set.all_slot_keys()
                    .map(|(id, keys)| (id, keys.iter().copied().collect())),
                "slot",
            ),
        ),
    ]);
    let out = write_canonical_checked(&value).map_err(invalid)?;
    if decode(&out)? != *set {
        return Err(unsupported(
            "Context 원본으로 정확히 복원할 수 없는 의미가 있습니다",
        ));
    }
    Ok(out)
}

fn list<'a>(value: &'a YamlValue, key: &str) -> Result<&'a [YamlValue], TextSourceError> {
    field(value, key)?
        .as_sequence()
        .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
}

fn rename_default(item: &YamlValue) -> Result<YamlValue, TextSourceError> {
    let entries = item
        .as_mapping()
        .ok_or_else(|| invalid("key 항목은 mapping이어야 합니다"))?
        .iter()
        .filter_map(|(k, v)| {
            let name = k.as_scalar()?;
            let mapped = match name {
                "default_expression" => "expression",
                "default_type" => "type",
                "default" => "value",
                "default_ast" => "ast",
                _ => return None,
            };
            Some((scalar(mapped), v.clone()))
        })
        .collect();
    Ok(YamlValue::Mapping(entries))
}

/// Context 텍스트를 읽습니다.
pub fn decode(source: &str) -> Result<ContextSet, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    fields(
        &value,
        &[
            "format",
            "schema_version",
            "keys",
            "provisions",
            "requirements",
            "slots",
        ],
    )?;
    if text(field(&value, "format")?)? != CONTEXT_FORMAT {
        return Err(unsupported("Context 원본 형식"));
    }
    if text(field(&value, "schema_version")?)? != VERSION {
        return Err(unsupported("Context 원본 버전"));
    }
    let mut set = ContextSet::new();
    let mut last = None;
    for item in list(&value, "keys")? {
        let default = rename_default(item)?;
        let mut allowed = vec!["id", "name", "scope", "value_type"];
        allowed.extend(match expression_keys(&default) {
            ["ast"] => &["default_ast"][..],
            _ => &["default_expression", "default_type", "default"][..],
        });
        fields(item, &allowed)?;
        let get = |key| text(field(item, key)?);
        let id = css::parse_id(get("id")?)?;
        if last.is_some_and(|prev| prev >= id) {
            return Err(invalid("key는 id 오름차순, 중복 없이 씁니다"));
        }
        last = Some(id);
        let key = ContextKey::new(
            id,
            get("name")?,
            parse_scope(get("scope")?)?,
            parse_type(get("value_type")?)?,
            parse_expression(&default)?,
        )
        .map_err(invalid)?;
        set.insert_key(key).map_err(invalid)?;
    }
    let mut last = None;
    for item in list(&value, "provisions")? {
        let mut allowed = vec!["id", "owner", "provider", "key"];
        allowed.extend_from_slice(expression_keys(item));
        fields(item, &allowed)?;
        let get = |key| text(field(item, key)?);
        let id = css::parse_id(get("id")?)?;
        if last.is_some_and(|prev| prev >= id) {
            return Err(invalid("provision은 id 오름차순, 중복 없이 씁니다"));
        }
        last = Some(id);
        let provision = ContextProvision::new(
            id,
            parse_owner(get("owner")?)?,
            css::parse_id(get("provider")?)?,
            css::parse_id(get("key")?)?,
            parse_expression(item)?,
        )
        .map_err(invalid)?;
        set.insert_provision(provision).map_err(invalid)?;
    }
    for (section, owner) in [("requirements", "component"), ("slots", "slot")] {
        let mut last = None;
        for item in list(&value, section)? {
            fields(item, &[owner, "keys"])?;
            let id = css::parse_id(text(field(item, owner)?)?)?;
            if last.is_some_and(|prev| prev >= id) {
                return Err(invalid(format!(
                    "{section}는 id 오름차순, 중복 없이 씁니다"
                )));
            }
            last = Some(id);
            let keys = css::parse_id_list(text(field(item, "keys")?)?)?;
            if keys.is_empty() || keys.windows(2).any(|w| w[0] >= w[1]) {
                return Err(invalid(format!(
                    "{section} key 목록은 비지 않은 오름차순입니다"
                )));
            }
            for key in keys {
                if section == "requirements" {
                    set.add_requirement(id, key);
                } else {
                    set.add_slot_key(id, key);
                }
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::expressions::ast::ReferenceTarget;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn literal(seed: u64, value: TypedValue) -> ExpressionAst {
        let id = ExprNodeId::new(sid(seed));
        ExpressionAst::new(id, vec![ExprNode::literal(id, value)]).unwrap()
    }

    fn blue() -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 1.0, 1.0).unwrap())
    }

    #[test]
    fn context_source_round_trips_literal_and_ast_expressions() {
        // 검증 기준(XXX.7.1·XXX.7.2): round-trip(텍스트 원본).
        let mut set = ContextSet::new();
        let theme = sid(10);
        set.insert_key(
            ContextKey::new(
                theme,
                "theme",
                ContextScope::Document(sid(1)),
                TypeSpec::required(ExpressionType::Color),
                literal(50, blue()),
            )
            .unwrap(),
        )
        .unwrap();
        set.insert_key(
            ContextKey::new(
                sid(11),
                "flag",
                ContextScope::Library(sid(2)),
                TypeSpec::optional(ExpressionType::Enum(EnumTypeId::new(7))),
                {
                    let id = ExprNodeId::new(sid(51));
                    ExpressionAst::new(
                        id,
                        vec![ExprNode::null(id, ExpressionType::Enum(EnumTypeId::new(7)))],
                    )
                    .unwrap()
                },
            )
            .unwrap(),
        )
        .unwrap();
        set.insert_provision(
            ContextProvision::new(
                sid(20),
                ContextOwner::View(sid(3)),
                sid(4),
                theme,
                literal(52, blue()),
            )
            .unwrap(),
        )
        .unwrap();
        let read = ExprNodeId::new(sid(53));
        set.insert_provision(
            ContextProvision::new(
                sid(21),
                ContextOwner::Component(sid(5)),
                sid(6),
                theme,
                ExpressionAst::new(
                    read,
                    vec![ExprNode::reference(read, ReferenceTarget::Context(theme))],
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        set.add_requirement(sid(5), theme);
        set.add_slot_key(sid(7), theme);
        set.add_slot_key(sid(7), sid(11));
        let text = encode(&set).unwrap();
        assert!(text.contains("format: stellar-context"), "{text}");
        assert!(text.contains("value_type: \"enum(7)?\""), "{text}");
        assert!(text.contains("ast: "), "{text}");
        assert_eq!(decode(&text).unwrap(), set);
        assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
        for bad in [
            text.replacen("format: stellar-context", "format: other", 1),
            text.replacen("scope: \"document", "scope: \"planet", 1),
            text.replacen("value_type: color", "value_type: length", 1),
        ] {
            assert_ne!(bad, text);
            assert!(decode(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn project_source_and_chunk_cache_carry_context() {
        // 검증 기준(XXX.7.2): 텍스트 원본과 chunk cache 양쪽 round-trip. Context가 없으면 기존 파일·chunk 그대로.
        use crate::document_io::load::{load_document, MemoryChunks};
        use crate::document_io::serialize::serialize_document;
        use crate::document_io::source_v4::project::{
            decode as decode_project, encode as encode_project, TextProject,
        };
        use stellar_document::component::definition::ComponentLibrary;
        use stellar_document::context::fixtures::{literal as lit, rgb, sid as fsid, SceneBuilder};

        let build = || {
            SceneBuilder::new(fsid(1), fsid(2), fsid(3), fsid(10))
                .frame(fsid(11), fsid(10))
                .build()
        };
        let plain = TextProject {
            document: build(),
            components: ComponentLibrary::new(),
            token_sets: Vec::new(),
            images: Default::default(),
        };
        let plain_files = encode_project(&plain).unwrap().files;
        assert!(!plain_files.contains_key(CONTEXT_PATH));
        let plain_chunks = serialize_document(&plain.document).unwrap();
        assert!(plain_chunks.context.is_none());
        assert!(plain_chunks.manifest.schema_version <= 2);

        let mut document = build();
        let theme = fsid(900);
        let mut set = ContextSet::new();
        set.insert_key(
            ContextKey::new(
                theme,
                "theme",
                ContextScope::Document(fsid(1)),
                TypeSpec::required(ExpressionType::Color),
                lit(1, rgb(1.0, 1.0, 1.0)),
            )
            .unwrap(),
        )
        .unwrap();
        set.insert_provision(
            ContextProvision::new(
                fsid(901),
                ContextOwner::View(fsid(3)),
                fsid(11),
                theme,
                lit(2, rgb(0.0, 0.0, 1.0)),
            )
            .unwrap(),
        )
        .unwrap();
        document.replace_context(set.clone());
        let project = TextProject {
            document,
            components: ComponentLibrary::new(),
            token_sets: Vec::new(),
            images: Default::default(),
        };
        let files = encode_project(&project).unwrap().files;
        assert!(files.contains_key(CONTEXT_PATH));
        let decoded = decode_project(&files).unwrap();
        assert_eq!(decoded.document.context(), &set);
        assert_eq!(
            decoded.semantic_key().unwrap(),
            project.semantic_key().unwrap()
        );
        // 다른 파일은 Context가 없을 때와 같습니다.
        for (path, text) in &plain_files {
            assert_eq!(files.get(path), Some(text), "{path}");
        }

        let serialized = serialize_document(&project.document).unwrap();
        let context_chunk = serialized.context.clone().expect("context-set chunk");
        assert_eq!(context_chunk.kind.0, 0x8000_0007);
        assert_eq!(serialized.manifest.schema_version, 3);
        let mut chunks = MemoryChunks::new();
        chunks.insert_document(&serialized);
        let loaded = load_document(&serialized.manifest.payload, &mut chunks).unwrap();
        assert_eq!(loaded.model.context(), &set);
        // manifest bytes는 Context 절 16 bytes만 늘어납니다.
        assert_eq!(
            serialized.manifest.payload.len(),
            plain_chunks.manifest.payload.len() + 16
        );
    }
}
