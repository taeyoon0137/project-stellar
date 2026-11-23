//! Frame 가이드·레이아웃 그리드와 View 변수·Variant의 텍스트 원본 v4 표기(ADR-0117 결정 2.4·ADR-0126).
//!
//! **가이드**: 문서 node(Frame) 항목의 `guides` 목록입니다. 한 줄이 가이드 하나 또는 그리드 하나이고 줄 문법은 문서 crate
//! [`NodeGuides::lines`] 그대로입니다(가이드 id 순, 그 뒤 그리드 그리는 순). 읽을 때 같은 함수로 다시 쓴 줄과 같아야 합니다
//! (정규형). 기록이 없으면 키를 쓰지 않습니다.
//!
//! ```yaml
//! guides:
//!   - guide x 120px 0195c530-3acd-7bc1-8414-c213f61cb965
//!   - grid columns 12 auto gap 20px margin 32px stretch
//! ```
//!
//! **View 변수·Variant**: View 파일 `view` mapping의 선택 키 `variables`·`variants`입니다(비어 있으면 쓰지 않음).
//!
//! - 변수: `id`, `name`, `type`(Context 원본과 같은 타입 이름, 예: `enum(3329)`·`boolean?`), `default`(값 문장),
//!   enum이면 `allowed`(값 문장 목록). 값 문장은 `null`, `enum(<타입>) <variant>`, 또는 token 원본과 같은 리터럴
//!   `<타입> <값>`(`boolean true`, `length 12px` …)입니다. 이 문장으로 쓸 수 없는 값이면 변수 전체를 `bytes`(canonical
//!   bytes hex)로 씁니다.
//! - Variant: `id`, `when`(조건 term 목록: `<변수 id> any` 또는 `<변수 id> = <값 문장>`), override가 있으면 `overrides`
//!   (`VariantOverrideSet` canonical bytes hex). 값 문장으로 쓸 수 없는 조건이면 Variant 전체를 `bytes`로 씁니다.
//!
//! 손실 저장은 없습니다. 목록은 StableId 순이고 읽을 때 순서·중복·정규형을 검사합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::guides::NodeGuides;
use stellar_document::view_variables::{
    model_from_parts, variable_bytes, variable_from_bytes, variant_bytes, variant_from_bytes,
};
use stellar_document::view_variant::commands::ViewVariantModel;
use stellar_document::view_variant::condition::{ConditionTerm, VariantCondition};
use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};
use stellar_document::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};

use super::super::text_codec::{field, fields, text};
use super::super::text_source::TextSourceError;
use super::context::{parse_type, type_name};
use super::css::{self, invalid};
use super::node::{parse_typed_value, typed_value};
use crate::yaml::YamlValue;

fn line(value: impl Into<String>) -> YamlValue {
    YamlValue::scalar(value.into())
}

fn pairs(entries: Vec<(&str, YamlValue)>) -> YamlValue {
    YamlValue::Mapping(
        entries
            .into_iter()
            .map(|(k, v)| (YamlValue::scalar(k), v))
            .collect(),
    )
}

fn sequence<'a>(value: &'a YamlValue, what: &str) -> Result<&'a [YamlValue], TextSourceError> {
    value
        .as_sequence()
        .ok_or_else(|| invalid(format!("{what}는 sequence여야 합니다")))
}

// --- 가이드 ---

/// Frame 가이드 기록의 `guides` 목록(빈 기록이면 `None`).
pub(crate) fn guides_value(record: Option<&NodeGuides>) -> Option<YamlValue> {
    let record = record.filter(|r| !r.is_empty())?;
    Some(YamlValue::Sequence(
        record.lines().into_iter().map(line).collect(),
    ))
}

/// `guides` 목록을 읽습니다. 줄은 정규형이어야 합니다.
pub(crate) fn parse_guides(
    value: &YamlValue,
    owner: StableId,
) -> Result<NodeGuides, TextSourceError> {
    let items = sequence(value, "guides")?;
    let written: Vec<&str> = items.iter().map(text).collect::<Result<_, _>>()?;
    let record = NodeGuides::from_lines(owner, written.iter().copied())
        .map_err(|e| invalid(format!("guides: {e}")))?;
    if record.is_empty() {
        return Err(invalid("빈 guides는 쓰지 않습니다"));
    }
    if record.lines() != written {
        return Err(invalid(
            "guides 줄은 정규형(가이드 id 순, 그 뒤 그리드, CSS 길이 정규 표기)으로 씁니다",
        ));
    }
    Ok(record)
}

// --- 값 문장 ---

/// 변수 값 문장(`null`, `enum(<타입>) <variant>`, `<타입> <리터럴>`). 쓸 수 없는 값이면 `None`.
pub fn value_text(value: &VariableValue) -> Option<String> {
    match value {
        VariableValue::ExplicitNull => Some("null".to_owned()),
        VariableValue::Value(TypedValue::Enum(value)) => Some(enum_text(*value)),
        VariableValue::Value(value) => typed_value(value)
            .ok()
            .map(|(kind, text)| format!("{kind} {text}")),
    }
}

fn enum_text(value: EnumValue) -> String {
    format!("enum({}) {}", value.enum_type(), value.variant())
}

/// [`value_text`]의 역.
pub fn parse_value_text(source: &str) -> Result<VariableValue, TextSourceError> {
    if source == "null" {
        return Ok(VariableValue::ExplicitNull);
    }
    let (kind, rest) = source
        .split_once(' ')
        .ok_or_else(|| invalid(format!("값 문장이 아닙니다: {source}")))?;
    if let Some(raw) = kind.strip_prefix("enum(").and_then(|k| k.strip_suffix(')')) {
        let enum_type = css::parse_u32(raw)?;
        let variant = css::parse_u32(rest)?;
        return Ok(VariableValue::Value(TypedValue::Enum(EnumValue::new(
            enum_type, variant,
        ))));
    }
    Ok(VariableValue::Value(parse_typed_value(kind, rest)?))
}

fn parse_enum_text(source: &str) -> Result<EnumValue, TextSourceError> {
    match parse_value_text(source)? {
        VariableValue::Value(TypedValue::Enum(value)) => Ok(value),
        _ => Err(invalid(format!("enum 값 문장이 아닙니다: {source}"))),
    }
}

// --- View 변수·Variant ---

fn variable_value(variable: &ViewVariable) -> YamlValue {
    let readable = value_text(variable.default_value()).map(|default| {
        let mut entries = vec![
            ("id", line(css::id(variable.id().stable_id()))),
            ("name", line(variable.name())),
            ("type", line(type_name(variable.value_type()))),
            ("default", line(default)),
        ];
        let allowed: Vec<YamlValue> = variable
            .allowed_values()
            .map(|v| line(enum_text(v)))
            .collect();
        if !allowed.is_empty() {
            entries.push(("allowed", YamlValue::Sequence(allowed)));
        }
        pairs(entries)
    });
    readable.unwrap_or_else(|| {
        pairs(vec![
            ("id", line(css::id(variable.id().stable_id()))),
            ("bytes", line(css::hex(&variable_bytes(variable)))),
        ])
    })
}

fn parse_variable(item: &YamlValue) -> Result<ViewVariable, TextSourceError> {
    let id = css::parse_id(text(field(item, "id")?)?)?;
    let variable = if item.get("bytes").is_some() {
        fields(item, &["id", "bytes"])?;
        let bytes = css::parse_hex(text(field(item, "bytes")?)?)?;
        let variable =
            variable_from_bytes(&bytes).map_err(|e| invalid(format!("View 변수 bytes: {e}")))?;
        if value_text(variable.default_value()).is_some() {
            return Err(invalid(
                "문장으로 쓸 수 있는 View 변수는 bytes로 쓰지 않습니다",
            ));
        }
        variable
    } else {
        let mut keys = vec!["id", "name", "type", "default"];
        if item.get("allowed").is_some() {
            keys.push("allowed");
        }
        fields(item, &keys)?;
        let allowed = match item.get("allowed") {
            Some(list) => {
                let values = sequence(list, "allowed")?
                    .iter()
                    .map(|v| parse_enum_text(text(v)?))
                    .collect::<Result<Vec<_>, _>>()?;
                if values.is_empty() {
                    return Err(invalid("빈 allowed는 쓰지 않습니다"));
                }
                if values.windows(2).any(|w| w[0] >= w[1]) {
                    return Err(invalid("allowed는 오름차순, 중복 없이 씁니다"));
                }
                values
            }
            None => Vec::new(),
        };
        ViewVariable::new(
            ViewVariableId::new(id),
            text(field(item, "name")?)?.to_owned(),
            parse_type(text(field(item, "type")?)?)?,
            parse_value_text(text(field(item, "default")?)?)?,
            allowed,
        )
        .map_err(|e| invalid(format!("View 변수: {e}")))?
    };
    if variable.id().stable_id() != id {
        return Err(invalid("View 변수 id가 bytes의 id와 다릅니다"));
    }
    Ok(variable)
}

fn term_text(variable: ViewVariableId, term: &ConditionTerm) -> Option<String> {
    let id = css::id(variable.stable_id());
    match term {
        ConditionTerm::Any => Some(format!("{id} any")),
        ConditionTerm::Equals(value) => value_text(value).map(|v| format!("{id} = {v}")),
    }
}

fn variant_value(variant: &ViewVariant) -> YamlValue {
    let terms: Option<Vec<String>> = variant
        .condition
        .terms()
        .map(|(variable, term)| term_text(variable, term))
        .collect();
    match terms {
        Some(terms) => {
            let mut entries = vec![
                ("id", line(css::id(variant.id.stable_id()))),
                (
                    "when",
                    YamlValue::Sequence(terms.into_iter().map(line).collect()),
                ),
            ];
            if !variant.overrides.is_empty() {
                entries.push(("overrides", line(css::hex(&variant.overrides.encode()))));
            }
            pairs(entries)
        }
        None => pairs(vec![
            ("id", line(css::id(variant.id.stable_id()))),
            ("bytes", line(css::hex(&variant_bytes(variant)))),
        ]),
    }
}

fn parse_variant(item: &YamlValue) -> Result<ViewVariant, TextSourceError> {
    let id = css::parse_id(text(field(item, "id")?)?)?;
    let variant = if item.get("bytes").is_some() {
        fields(item, &["id", "bytes"])?;
        let bytes = css::parse_hex(text(field(item, "bytes")?)?)?;
        let variant =
            variant_from_bytes(&bytes).map_err(|e| invalid(format!("Variant bytes: {e}")))?;
        if variant
            .condition
            .terms()
            .all(|(variable, term)| term_text(variable, term).is_some())
        {
            return Err(invalid(
                "문장으로 쓸 수 있는 Variant는 bytes로 쓰지 않습니다",
            ));
        }
        variant
    } else {
        let mut keys = vec!["id", "when"];
        if item.get("overrides").is_some() {
            keys.push("overrides");
        }
        fields(item, &keys)?;
        let mut condition = VariantCondition::new();
        let mut last: Option<ViewVariableId> = None;
        for term in sequence(field(item, "when")?, "when")? {
            let source = text(term)?;
            let (variable, rest) = source
                .split_once(' ')
                .ok_or_else(|| invalid(format!("조건 term이 아닙니다: {source}")))?;
            let variable = ViewVariableId::new(css::parse_id(variable)?);
            if last.is_some_and(|prev| prev >= variable) {
                return Err(invalid("조건 term은 변수 id 오름차순, 중복 없이 씁니다"));
            }
            last = Some(variable);
            let term = match rest {
                "any" => ConditionTerm::Any,
                other => match other.strip_prefix("= ") {
                    Some(value) => ConditionTerm::Equals(parse_value_text(value)?),
                    None => return Err(invalid(format!("조건 term이 아닙니다: {source}"))),
                },
            };
            condition = condition.with(variable, term);
        }
        let overrides = match item.get("overrides") {
            Some(hex) => {
                let set = VariantOverrideSet::decode(&css::parse_hex(text(hex)?)?)
                    .map_err(|e| invalid(format!("Variant overrides: {e}")))?;
                if set.is_empty() {
                    return Err(invalid("빈 overrides는 쓰지 않습니다"));
                }
                set
            }
            None => VariantOverrideSet::new(),
        };
        ViewVariant::new(ViewVariantId::new(id), condition, overrides)
    };
    if variant.id.stable_id() != id {
        return Err(invalid("Variant id가 bytes의 id와 다릅니다"));
    }
    Ok(variant)
}

/// View mapping에 덧붙일 `variables`·`variants` 항목(빈 목록은 쓰지 않음).
pub(crate) fn variant_fields(model: Option<&ViewVariantModel>) -> Vec<(&'static str, YamlValue)> {
    let Some(model) = model else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if !model.variables().is_empty() {
        out.push((
            "variables",
            YamlValue::Sequence(model.variables().iter().map(variable_value).collect()),
        ));
    }
    let variants: Vec<YamlValue> = model.variants().map(variant_value).collect();
    if !variants.is_empty() {
        out.push(("variants", YamlValue::Sequence(variants)));
    }
    out
}

/// View mapping의 선택 키 이름.
pub(crate) const VIEW_KEYS: [&str; 2] = ["variables", "variants"];

/// View mapping에서 `variables`·`variants`를 읽습니다(둘 다 없으면 `None`).
pub(crate) fn parse_variant_fields(
    view_value: &YamlValue,
    view: StableId,
) -> Result<Option<ViewVariantModel>, TextSourceError> {
    let variables = match view_value.get("variables") {
        Some(list) => {
            let items = sequence(list, "variables")?;
            if items.is_empty() {
                return Err(invalid("빈 variables는 쓰지 않습니다"));
            }
            items
                .iter()
                .map(parse_variable)
                .collect::<Result<Vec<_>, _>>()?
        }
        None => Vec::new(),
    };
    let variants = match view_value.get("variants") {
        Some(list) => {
            let items = sequence(list, "variants")?;
            if items.is_empty() {
                return Err(invalid("빈 variants는 쓰지 않습니다"));
            }
            items
                .iter()
                .map(parse_variant)
                .collect::<Result<Vec<_>, _>>()?
        }
        None => Vec::new(),
    };
    if variables.is_empty() && variants.is_empty() {
        return Ok(None);
    }
    if variables.windows(2).any(|w| w[0].id() >= w[1].id())
        || variants.windows(2).any(|w| w[0].id >= w[1].id)
    {
        return Err(invalid(
            "View 변수·Variant는 StableId 오름차순, 중복 없이 씁니다",
        ));
    }
    model_from_parts(view, variables, variants)
        .map(Some)
        .map_err(|e| invalid(format!("View 변수 model: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_document::expressions::types::{EnumTypeId, ExpressionType, TypeSpec};
    use stellar_document::guides::{GuideAxis, GuideEdit, LayoutGrid, RulerGuide};
    use stellar_document::view_variant::commands::ViewVariantCommand;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0d, n)
    }

    pub(crate) fn sample_model(view: StableId) -> ViewVariantModel {
        let device = ViewVariable::new(
            ViewVariableId::new(sid(200)),
            "device".into(),
            TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(3329))),
            VariableValue::Value(TypedValue::Enum(EnumValue::new(3329, 0))),
            [EnumValue::new(3329, 0), EnumValue::new(3329, 1)],
        )
        .unwrap();
        let dark = ViewVariable::new(
            ViewVariableId::new(sid(201)),
            "dark".into(),
            TypeSpec::optional(ExpressionType::Boolean),
            VariableValue::ExplicitNull,
            [],
        )
        .unwrap();
        let mut overrides = VariantOverrideSet::new();
        overrides.set_property(
            PropertyPath::with_keys(sid(1), vec![PropertyKey(0x0110)]),
            TypedValue::Boolean(true),
        );
        let compact = ViewVariant::new(
            ViewVariantId::new(sid(300)),
            VariantCondition::new()
                .equals(
                    ViewVariableId::new(sid(200)),
                    VariableValue::Value(TypedValue::Enum(EnumValue::new(3329, 1))),
                )
                .with(ViewVariableId::new(sid(201)), ConditionTerm::Any),
            overrides,
        );
        let mut model = ViewVariantModel::new(view);
        for command in [
            ViewVariantCommand::CreateVariable(device),
            ViewVariantCommand::CreateVariable(dark),
            ViewVariantCommand::CreateVariant(compact),
        ] {
            command.apply(&mut model).unwrap();
        }
        model
    }

    #[test]
    fn guides_lines_round_trip_and_reject_non_canonical_lines() {
        let mut record = NodeGuides::new(sid(1));
        record
            .apply(&GuideEdit::Add(
                RulerGuide::new(sid(100), GuideAxis::Y, -4.5).unwrap(),
            ))
            .unwrap();
        record
            .apply(&GuideEdit::SetGrids(vec![LayoutGrid::columns(
                12, 20.0, 32.0,
            )
            .unwrap()]))
            .unwrap();
        let value = guides_value(Some(&record)).unwrap();
        assert_eq!(parse_guides(&value, sid(1)).unwrap(), record);
        assert!(guides_value(Some(&NodeGuides::new(sid(1)))).is_none());
        let reordered =
            YamlValue::Sequence(value.as_sequence().unwrap().iter().rev().cloned().collect());
        assert!(parse_guides(&reordered, sid(1)).is_err());
        let loose = YamlValue::Sequence(vec![line(format!("guide y -4.50px {}", sid(100)))]);
        assert!(parse_guides(&loose, sid(1)).is_err());
    }

    #[test]
    fn variables_and_variants_round_trip_readably() {
        let model = sample_model(sid(9));
        let fields_ = variant_fields(Some(&model));
        assert_eq!(fields_.len(), 2);
        let view = pairs(fields_.clone());
        assert_eq!(parse_variant_fields(&view, sid(9)).unwrap(), Some(model));
        // 사람이 읽는 문장입니다.
        let written = crate::yaml::write_canonical(&view);
        assert!(written.contains("enum(3329) 0"), "{written}");
        assert!(written.contains("any"), "{written}");
        assert!(written.contains("boolean?"), "{written}");
        assert_eq!(
            parse_variant_fields(&pairs(Vec::new()), sid(9)).unwrap(),
            None
        );
        for (text_, value) in [
            ("null", VariableValue::ExplicitNull),
            (
                "boolean true",
                VariableValue::Value(TypedValue::Boolean(true)),
            ),
            (
                "enum(7) 3",
                VariableValue::Value(TypedValue::Enum(EnumValue::new(7, 3))),
            ),
        ] {
            assert_eq!(parse_value_text(text_).unwrap(), value);
            assert_eq!(value_text(&value).unwrap(), text_);
        }
    }

    fn with_guides_and_variants(
        mut model: stellar_document::model::DocumentModel,
        frame: StableId,
        view: StableId,
    ) -> stellar_document::model::DocumentModel {
        let mut record = NodeGuides::new(frame);
        for (n, axis, position) in [(100, GuideAxis::X, 120.0), (101, GuideAxis::Y, 64.25)] {
            record
                .apply(&GuideEdit::Add(
                    RulerGuide::new(sid(n), axis, position).unwrap(),
                ))
                .unwrap();
        }
        record
            .apply(&GuideEdit::SetGrids(vec![LayoutGrid::columns(
                12, 20.0, 32.0,
            )
            .unwrap()]))
            .unwrap();
        model.set_node_guides(record).unwrap();
        model.set_view_variants(sample_model(view)).unwrap();
        model
    }

    #[test]
    fn single_view_source_carries_guides_and_view_variants() {
        // 검증 기준(XII.10.19·XII.10.20·XVI.11.1): 단일 View 원본 쓰기 → 읽기 왕복이 가이드·그리드·View 변수를 보존합니다.
        use super::super::super::text_codec::{decode, encode};
        use super::super::tests::{golden_model, sid as gsid};
        let plain = encode(&golden_model()).unwrap();
        let model = with_guides_and_variants(golden_model(), gsid(10), gsid(3));
        let text_ = encode(&model).unwrap();
        assert!(text_.contains("guides:"), "{text_}");
        assert!(text_.contains("grid columns 12 auto gap 20px margin 32px stretch"));
        assert!(text_.contains("variables:"));
        let decoded = decode(&text_).unwrap();
        assert_eq!(decoded.to_records().unwrap(), model.to_records().unwrap());
        assert_eq!(encode(&decoded).unwrap(), text_);
        // 가이드·View 변수가 없는 문서의 원본은 그대로입니다.
        assert!(!plain.contains("guides:") && !plain.contains("variables:"));
    }

    #[test]
    fn project_source_and_chunk_cache_carry_guides_and_view_variants() {
        // 검증 기준(ADR-0126): 다중 파일 원본과 chunk cache 양쪽 왕복.
        use crate::document_io::load::{load_document, MemoryChunks};
        use crate::document_io::serialize::serialize_document;
        use crate::document_io::source_v4::project::{decode, encode, TextProject};
        use stellar_document::component::definition::ComponentLibrary;
        use stellar_document::context::fixtures::{sid as fsid, SceneBuilder};

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
        let plain_files = encode(&plain).unwrap().files;
        let plain_chunks = serialize_document(&plain.document).unwrap();
        let project = TextProject {
            document: with_guides_and_variants(build(), fsid(11), fsid(3)),
            components: ComponentLibrary::new(),
            token_sets: Vec::new(),
            images: Default::default(),
        };
        let files = encode(&project).unwrap().files;
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            plain_files.keys().collect::<Vec<_>>()
        );
        let decoded = decode(&files).unwrap();
        assert_eq!(
            decoded.semantic_key().unwrap(),
            project.semantic_key().unwrap()
        );
        assert_eq!(
            decoded.document.node_guides(fsid(11)),
            project.document.node_guides(fsid(11))
        );

        let serialized = serialize_document(&project.document).unwrap();
        // manifest·ViewTree는 그대로이고 View root subtree chunk만 확장 절만큼 늘어납니다.
        assert_eq!(serialized.manifest.payload, plain_chunks.manifest.payload);
        let mut chunks = MemoryChunks::new();
        chunks.insert_document(&serialized);
        let loaded = load_document(&serialized.manifest.payload, &mut chunks).unwrap();
        assert_eq!(
            loaded.model.to_records().unwrap(),
            project.document.to_records().unwrap()
        );
    }
}
