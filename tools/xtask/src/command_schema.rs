//! CommandTransaction 입력 JSON Schema 생성 (XIX.9.2, ADR-0085 결정 2).
//!
//! commands crate의 닫힌 enum([`stellar_ops::mcp_command_input::canonical_kinds`])과 ops 필드 형식 종류 표
//! ([`stellar_ops::mcp_command_input::FIELD_SPECS`])에서 `schemas/command-transaction.schema.json`을 만듭니다.
//! `stellar-agent-protocol`은 이 산출물만 `include_str!`로 포함하고 commands crate에 의존하지 않습니다.
//!
//! - `cargo xtask generate-command-schema`: 파일을 다시 씁니다.
//! - `cargo xtask generate-command-schema --check`: 파일이 생성 결과와 다르면 65로 끝납니다(drift).
//!
//! 출력은 결정적입니다: key 순서 고정, 2칸 들여쓰기, 끝 개행 하나.

use std::path::Path;

use stellar_ops::apply::MAX_COMMANDS;
use stellar_ops::mcp_command_input::{canonical_kinds, FieldType, CANONICAL_FIELD, FIELD_SPECS};

/// 저장소 기준 산출물 경로.
pub const SCHEMA_PATH: &str = "schemas/command-transaction.schema.json";

/// 최소 JSON 값(생성 전용).
enum J {
    Obj(Vec<(&'static str, J)>),
    Arr(Vec<J>),
    Str(String),
    Int(i64),
    Bool(bool),
}

fn s(text: impl Into<String>) -> J {
    J::Str(text.into())
}

fn obj(pairs: Vec<(&'static str, J)>) -> J {
    J::Obj(pairs)
}

fn escape(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write(value: &J, indent: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match value {
        J::Str(text) => escape(text, out),
        J::Int(n) => out.push_str(&n.to_string()),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Arr(items) => {
            if items.iter().all(|i| matches!(i, J::Str(_))) {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write(item, indent, out);
                }
                out.push(']');
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                write(item, indent + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(indent));
            out.push(']');
        }
        J::Obj(pairs) => {
            out.push_str("{\n");
            for (i, (key, item)) in pairs.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                escape(key, out);
                out.push_str(": ");
                write(item, indent + 1, out);
                if i + 1 < pairs.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(indent));
            out.push('}');
        }
    }
}

fn def_ref(field: FieldType) -> J {
    obj(vec![("$ref", s(format!("#/$defs/{}", field.name())))])
}

fn field_def(field: FieldType) -> J {
    match field {
        FieldType::NodeId => obj(vec![
            ("type", s("string")),
            (
                "pattern",
                s("^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$"),
            ),
        ]),
        FieldType::Bool => obj(vec![("type", s("boolean"))]),
        FieldType::Text => obj(vec![("type", s("string"))]),
        FieldType::U32 => obj(vec![
            ("type", s("integer")),
            ("minimum", J::Int(0)),
            ("maximum", J::Int(i64::from(u32::MAX))),
        ]),
        FieldType::F32 => obj(vec![("type", s("number"))]),
        FieldType::NonNegativeF32 => obj(vec![("type", s("number")), ("minimum", J::Int(0))]),
        FieldType::Hex => obj(vec![
            ("type", s("string")),
            ("pattern", s("^([0-9a-f]{2})*$")),
        ]),
        FieldType::ContentHash => obj(vec![
            ("type", s("string")),
            ("pattern", s("^sha256:[0-9a-f]{64}$")),
        ]),
    }
}

/// 필드 형식에 나오는 모든 값 형식(고정 순서).
const ALL_FIELD_TYPES: [FieldType; 8] = [
    FieldType::NodeId,
    FieldType::Bool,
    FieldType::Text,
    FieldType::U32,
    FieldType::F32,
    FieldType::NonNegativeF32,
    FieldType::Hex,
    FieldType::ContentHash,
];

/// 생성한 schema 원문.
pub fn generate() -> String {
    let mut variants: Vec<J> = FIELD_SPECS
        .iter()
        .map(|spec| {
            let mut required = vec![s("kind")];
            required.extend(spec.fields.iter().map(|(name, _)| s(*name)));
            let mut properties = vec![("kind", obj(vec![("const", s(spec.kind))]))];
            properties.extend(spec.fields.iter().map(|(name, ty)| (*name, def_ref(*ty))));
            obj(vec![
                ("type", s("object")),
                ("additionalProperties", J::Bool(false)),
                ("required", J::Arr(required)),
                ("properties", J::Obj(properties)),
            ])
        })
        .collect();
    variants.push(obj(vec![
        ("type", s("object")),
        ("additionalProperties", J::Bool(false)),
        ("required", J::Arr(vec![s("kind"), s(CANONICAL_FIELD)])),
        (
            "properties",
            obj(vec![
                (
                    "kind",
                    obj(vec![("enum", J::Arr(canonical_kinds().map(s).collect()))]),
                ),
                (CANONICAL_FIELD, def_ref(FieldType::Hex)),
            ]),
        ),
    ]));
    let defs: Vec<(&'static str, J)> = ALL_FIELD_TYPES
        .iter()
        .map(|ty| (ty.name(), field_def(*ty)))
        .collect();
    let schema = obj(vec![
        ("$schema", s("https://json-schema.org/draft/2020-12/schema")),
        ("$id", s("stellar://schemas/command-transaction")),
        ("title", s("CommandTransaction")),
        (
            "description",
            s("cargo xtask generate-command-schema가 commands closed enum과 ops 필드 종류 표에서 생성합니다. 직접 고치지 않습니다."),
        ),
        ("type", s("object")),
        ("additionalProperties", J::Bool(false)),
        (
            "required",
            J::Arr(vec![s("base_revision"), s("document"), s("commands")]),
        ),
        (
            "properties",
            obj(vec![
                (
                    "base_revision",
                    obj(vec![
                        ("type", s("string")),
                        ("pattern", s("^g(0|[1-9][0-9]*:sha256:[0-9a-f]{64})$")),
                    ]),
                ),
                ("document", def_ref(FieldType::NodeId)),
                (
                    "commands",
                    obj(vec![
                        ("type", s("array")),
                        ("minItems", J::Int(1)),
                        (
                            "maxItems",
                            J::Int(i64::try_from(MAX_COMMANDS).unwrap_or(i64::MAX)),
                        ),
                        ("items", obj(vec![("oneOf", J::Arr(variants))])),
                    ]),
                ),
            ]),
        ),
        ("$defs", J::Obj(defs)),
    ]);
    let mut out = String::new();
    write(&schema, 0, &mut out);
    out.push('\n');
    out
}

/// 산출물을 쓰거나(`check`가 거짓) 비교합니다. `check`면 같을 때 `Ok(true)`입니다.
pub fn generate_file(repo_root: &Path, check: bool) -> Result<bool, String> {
    let path = repo_root.join(SCHEMA_PATH);
    let text = generate();
    if check {
        let current = std::fs::read_to_string(&path).map_err(|e| format!("{SCHEMA_PATH}: {e}"))?;
        return Ok(current == text);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{SCHEMA_PATH}: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    #[test]
    fn checked_in_schema_has_no_drift() {
        // 검증 기준(XIX.9.2): 생성 schema와 enum variant 일치.
        assert_eq!(
            generate_file(repo_root(), true),
            Ok(true),
            "{SCHEMA_PATH}가 생성 결과와 다릅니다. `cargo xtask generate-command-schema`로 재생성하세요."
        );
    }

    #[test]
    fn every_closed_enum_kind_appears_exactly_once() {
        let text = generate();
        for spec in FIELD_SPECS {
            assert_eq!(
                text.matches(&format!("\"const\": \"{}\"", spec.kind))
                    .count(),
                1,
                "{}",
                spec.kind
            );
        }
        for kind in canonical_kinds() {
            assert_eq!(text.matches(&format!("\"{kind}\"")).count(), 1, "{kind}");
        }
        assert!(text.ends_with("}\n"));
        assert_eq!(generate(), text, "결정적 출력");
    }
}
