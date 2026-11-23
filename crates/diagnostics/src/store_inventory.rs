//! 진단 schema feature inventory (XVI.8.6, ADR-0109 결정 8).
//!
//! 내장 터미널과 MCP(그리고 에이전트)는 초기 출시에서 꺼져 있습니다. 그 기능의 payload field가 진단 schema에
//! 있으면, 기능을 끈 빌드에서도 schema에 자리가 생겨 나중에 값이 들어갈 수 있습니다. 이 module은 진단 store·번들·
//! 동의·대기열 파일 schema의 모든 형식 이름, tag, field 이름, 닫힌 enum 이름을 모으고([`schema_inventory`]), 꺼진
//! 기능의 단어([`DISABLED_FEATURE_WORDS`])가 하나도 없는지 검사합니다([`check_feature_inventory`]).
//!
//! schema 정의에는 Cargo feature 조건부 컴파일이 없고 이 crate에는 Cargo feature가 없으므로, 기능을 끈 빌드와 켠 빌드의
//! schema가 같습니다(테스트가 소스와 manifest로 확인).

use crate::bundle::{BUNDLE_HEADER, BUNDLE_SCHEMAS};
use crate::consent::{CONSENT_HEADER, CONSENT_SCHEMA};
use crate::consent_gate::OUTBOX_HEADER;
use crate::store::{FieldType, RecordSchema, STORE_HEADER};

/// 초기 출시에서 꺼진 기능(내장 터미널, MCP, 에이전트)의 단어. inventory 항목에 부분 문자열로도 나오면 안 됩니다.
pub const DISABLED_FEATURE_WORDS: [&str; 20] = [
    "terminal",
    "pty",
    "tty",
    "shell",
    "console",
    "command_line",
    "stdin",
    "stdout",
    "stderr",
    "mcp",
    "jsonrpc",
    "json_rpc",
    "ipc",
    "tool_call",
    "agent",
    "assistant",
    "prompt",
    "transcript",
    "chat",
    "session",
];

/// inventory 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    /// 파일 형식 이름(머리 줄).
    Format,
    /// 줄 tag.
    Tag,
    /// field 이름.
    Field,
    /// 닫힌 enum 이름.
    Value,
}

impl EntryKind {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            EntryKind::Format => "format",
            EntryKind::Tag => "tag",
            EntryKind::Field => "field",
            EntryKind::Value => "value",
        }
    }
}

/// inventory 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InventoryEntry {
    /// 속한 schema(줄 tag, 형식 이름은 `format`).
    pub schema: &'static str,
    /// 종류.
    pub kind: EntryKind,
    /// 단어.
    pub word: &'static str,
}

/// 꺼진 기능의 단어를 담은 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryViolation {
    /// 항목.
    pub entry: InventoryEntry,
    /// 걸린 단어.
    pub disabled: &'static str,
}

/// 검사하는 줄 schema 전체(번들 schema + 동의 파일).
pub fn inventory_schemas() -> Vec<&'static RecordSchema> {
    let mut schemas: Vec<&'static RecordSchema> = BUNDLE_SCHEMAS.to_vec();
    schemas.push(&CONSENT_SCHEMA);
    schemas
}

/// schema 목록의 inventory.
pub fn inventory_of(schemas: &[&RecordSchema], formats: &[&'static str]) -> Vec<InventoryEntry> {
    let mut out: Vec<InventoryEntry> = formats
        .iter()
        .map(|word| InventoryEntry {
            schema: "format",
            kind: EntryKind::Format,
            word,
        })
        .collect();
    for schema in schemas {
        out.push(InventoryEntry {
            schema: schema.tag,
            kind: EntryKind::Tag,
            word: schema.tag,
        });
        for spec in schema.fields {
            out.push(InventoryEntry {
                schema: schema.tag,
                kind: EntryKind::Field,
                word: spec.name,
            });
            if let FieldType::Name(names) = spec.ty {
                out.extend(names.iter().map(|word| InventoryEntry {
                    schema: schema.tag,
                    kind: EntryKind::Value,
                    word,
                }));
            }
        }
    }
    out
}

/// 진단 파일 형식 이름 전체.
pub const FORMATS: [&str; 4] = [STORE_HEADER, BUNDLE_HEADER, CONSENT_HEADER, OUTBOX_HEADER];

/// 진단 schema inventory 전체.
pub fn schema_inventory() -> Vec<InventoryEntry> {
    inventory_of(&inventory_schemas(), &FORMATS)
}

/// 항목들에서 꺼진 기능의 단어를 찾습니다(ASCII 대소문자 무시).
pub fn check_entries(entries: &[InventoryEntry]) -> Vec<InventoryViolation> {
    let mut out = Vec::new();
    for entry in entries {
        let lower = entry.word.to_ascii_lowercase();
        for disabled in DISABLED_FEATURE_WORDS {
            if lower.contains(disabled) {
                out.push(InventoryViolation {
                    entry: *entry,
                    disabled,
                });
            }
        }
    }
    out
}

/// 진단 schema에 꺼진 기능의 field가 없는지 검사합니다(빈 Vec = 통과).
pub fn check_feature_inventory() -> Vec<InventoryViolation> {
    check_entries(&schema_inventory())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FieldSpec;

    #[test]
    fn diagnostic_schema_has_no_disabled_feature_fields() {
        // 검증 기준(XVI.8.6): feature inventory 통과.
        let inventory = schema_inventory();
        assert!(inventory.len() > 100, "{}", inventory.len());
        assert_eq!(check_feature_inventory(), Vec::new());
        // 모든 schema가 inventory에 있습니다.
        for schema in inventory_schemas() {
            assert!(inventory
                .iter()
                .any(|e| e.kind == EntryKind::Tag && e.word == schema.tag));
        }
        for format in FORMATS {
            assert!(inventory.iter().any(|e| e.word == format));
        }
    }

    #[test]
    fn violations_are_detected() {
        const BAD: RecordSchema = RecordSchema {
            tag: "perf",
            fields: &[
                FieldSpec {
                    name: "terminal_output_bytes",
                    ty: FieldType::U64,
                },
                FieldSpec {
                    name: "source",
                    ty: FieldType::Name(&["canvas", "mcp_tool", "Agent"]),
                },
            ],
        };
        let violations = check_entries(&inventory_of(&[&BAD], &["stellar.session-log v1"]));
        let words: Vec<(&str, &str)> = violations
            .iter()
            .map(|v| (v.entry.word, v.disabled))
            .collect();
        assert_eq!(
            words,
            vec![
                ("stellar.session-log v1", "session"),
                ("terminal_output_bytes", "terminal"),
                ("mcp_tool", "mcp"),
                ("Agent", "agent"),
            ]
        );
    }

    #[test]
    fn schema_does_not_depend_on_build_features() {
        // 기능을 끈 빌드와 켠 빌드의 schema가 같습니다: schema 소스에 feature 조건부 컴파일이 없고 crate에 feature가 없습니다.
        let needle = concat!("cfg(", "feature");
        for (name, source) in [
            ("store.rs", include_str!("store.rs")),
            ("bundle.rs", include_str!("bundle.rs")),
            ("consent.rs", include_str!("consent.rs")),
            ("consent_gate.rs", include_str!("consent_gate.rs")),
            ("scanner.rs", include_str!("scanner.rs")),
            ("store_inventory.rs", include_str!("store_inventory.rs")),
        ] {
            assert!(!source.contains(needle), "{name}");
        }
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("[features]"), "{manifest}");
    }
}
