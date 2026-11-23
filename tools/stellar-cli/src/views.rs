//! `stellar-ops` 결과 → 결과 JSON 변환과 오류 분류 (XXIV.4.3).
//!
//! 결과 JSON에는 경로를 넣지 않습니다. 경로는 [`CliError::with_context`]로 stderr에만 씁니다.

use stellar_ops::core_types::ids::StableId;
use stellar_ops::diff::{DiffEntry, DiffPosition, DiffReport};
use stellar_ops::error::{DocumentReadError, OpenError};
use stellar_ops::query::{
    DefinitionSet, DocumentOutline, NodeDetail, OpsValue, ProjectStructure, QueryError, TreePage,
};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::types::RootError;

use crate::exit::ExitStatus;
use crate::json::{self, JsonLimits, JsonNumber, JsonValue};
use crate::output::{CliError, Diagnostic, Severity};

/// StableId 문자열.
pub fn id(value: StableId) -> JsonValue {
    JsonValue::string(value.to_string())
}

/// 선택 StableId(`None`은 `null`).
pub fn opt_id(value: Option<StableId>) -> JsonValue {
    value.map_or(JsonValue::Null, id)
}

/// 부호 없는 정수.
pub fn uint(value: u64) -> JsonValue {
    JsonValue::Number(JsonNumber::from_u64(value))
}

/// `usize` 개수.
pub fn count(value: usize) -> JsonValue {
    uint(u64::try_from(value).unwrap_or(u64::MAX))
}

/// 유한 float. Rust의 `f32` 표기(지수 없음)는 JSON 숫자 문법을 만족합니다.
pub fn float(value: f32) -> JsonValue {
    json::parse(&value.to_string(), JsonLimits::default()).unwrap_or(JsonValue::Null)
}

/// bytes 소문자 16진수.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// revision object: `{"token", "generation", "manifest_hash"}`.
pub fn revision(value: ProjectRevision) -> JsonValue {
    JsonValue::object(vec![
        ("token", JsonValue::string(value.token())),
        ("generation", uint(value.generation())),
        (
            "manifest_hash",
            value
                .manifest_hash()
                .map_or(JsonValue::Null, |h| JsonValue::string(h.to_string())),
        ),
    ])
}

/// 문서 개요.
pub fn document_outline(outline: &DocumentOutline) -> JsonValue {
    JsonValue::object(vec![
        ("id", id(outline.id)),
        ("name", JsonValue::string(outline.display_name.clone())),
        (
            "pages",
            JsonValue::Array(
                outline
                    .pages
                    .iter()
                    .map(|page| {
                        JsonValue::object(vec![
                            ("id", id(page.id)),
                            ("name", JsonValue::string(page.display_name.clone())),
                            (
                                "views",
                                JsonValue::Array(
                                    page.views
                                        .iter()
                                        .map(|view| {
                                            JsonValue::object(vec![
                                                ("id", id(view.id)),
                                                (
                                                    "name",
                                                    JsonValue::string(view.display_name.clone()),
                                                ),
                                                ("root_frame", id(view.root_frame)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// 프로젝트 구조의 `documents`·`chunks` 항목.
pub fn structure_fields(structure: &ProjectStructure) -> Vec<(&'static str, JsonValue)> {
    vec![
        ("revision", revision(structure.revision)),
        (
            "documents",
            JsonValue::Array(structure.documents.iter().map(document_outline).collect()),
        ),
        (
            "chunks",
            JsonValue::Array(
                structure
                    .chunk_kinds
                    .iter()
                    .map(|(name, code, n)| {
                        JsonValue::object(vec![
                            ("kind", JsonValue::opt_string(*name)),
                            ("code", uint(u64::from(*code))),
                            ("count", count(*n)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]
}

/// 문서 트리 page.
pub fn tree_page(page: &TreePage, limit: usize) -> JsonValue {
    JsonValue::object(vec![
        ("revision", revision(page.revision)),
        ("document", id(page.document)),
        ("total", count(page.total)),
        ("offset", count(page.offset)),
        ("limit", count(limit)),
        (
            "nodes",
            JsonValue::Array(
                page.nodes
                    .iter()
                    .map(|node| {
                        JsonValue::object(vec![
                            ("id", id(node.id)),
                            ("parent", opt_id(node.parent)),
                            ("view", id(node.view)),
                            ("depth", uint(u64::from(node.depth))),
                            ("kind", JsonValue::string(node.kind)),
                            ("name", JsonValue::string(node.display_name.clone())),
                            ("visible", JsonValue::Bool(node.visible)),
                            ("locked", JsonValue::Bool(node.locked)),
                            ("child_count", count(node.child_count)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "next_cursor",
            page.next_cursor
                .map_or(JsonValue::Null, |c| JsonValue::string(c.to_string())),
        ),
    ])
}

/// 속성 값 본문. typed·bytes 값은 canonical bytes 16진수 object입니다.
pub fn value_inner(value: &OpsValue) -> JsonValue {
    match value {
        OpsValue::Bool(v) => JsonValue::Bool(*v),
        OpsValue::Uint(v) => uint(*v),
        OpsValue::Text(v) => JsonValue::string(v.clone()),
        OpsValue::Id(v) => id(*v),
        OpsValue::Ids(v) => JsonValue::Array(v.iter().copied().map(id).collect()),
        OpsValue::Floats(v) => JsonValue::Array(v.iter().copied().map(float).collect()),
        OpsValue::Typed { type_tag, bytes } => JsonValue::object(vec![
            ("type_tag", uint(u64::from(*type_tag))),
            ("hex", JsonValue::string(hex(bytes))),
        ]),
        OpsValue::Bytes(v) => JsonValue::object(vec![("hex", JsonValue::string(hex(v)))]),
    }
}

/// node 상세.
pub fn node_detail(detail: &NodeDetail) -> JsonValue {
    JsonValue::object(vec![
        ("revision", revision(detail.revision)),
        ("document", id(detail.document)),
        (
            "node",
            JsonValue::object(vec![
                ("id", id(detail.id)),
                ("parent", opt_id(detail.parent)),
                ("kind", JsonValue::string(detail.kind)),
                ("name", JsonValue::string(detail.display_name.clone())),
                ("visible", JsonValue::Bool(detail.visible)),
                ("locked", JsonValue::Bool(detail.locked)),
                ("order_key", JsonValue::string(detail.order_key_hex.clone())),
                (
                    "children",
                    JsonValue::Array(detail.children.iter().copied().map(id).collect()),
                ),
                ("modifier_count", count(detail.modifier_count)),
                (
                    "properties",
                    JsonValue::Array(
                        detail
                            .properties
                            .iter()
                            .map(|p| {
                                JsonValue::object(vec![
                                    ("path", JsonValue::string(p.path.clone())),
                                    ("key", JsonValue::string(p.key.clone())),
                                    ("type", JsonValue::string(p.value.type_name())),
                                    ("value", value_inner(&p.value)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ),
    ])
}

/// token·component 정의 집합.
pub fn definition_set(set: &DefinitionSet) -> JsonValue {
    JsonValue::object(vec![
        ("revision", revision(set.revision)),
        ("chunk_kind", JsonValue::string(set.chunk_kind)),
        ("chunk_count", count(set.chunk_count)),
        (
            "entries",
            JsonValue::Array(set.entries.iter().cloned().map(JsonValue::string).collect()),
        ),
    ])
}

fn not_a_project(diagnostic: &'static str, message: &str) -> CliError {
    CliError::new(
        ExitStatus::Failure,
        "not-a-stellar-project",
        "Stellar 프로젝트 root가 아닙니다",
    )
    .with_diagnostic(Diagnostic {
        severity: Severity::Error,
        code: diagnostic,
        message: message.to_string(),
    })
}

/// 프로젝트 열기 실패를 종료 상태·오류 코드로 분류합니다(XXIV.1.2 inspect 분류 유지).
pub fn open_error(error: &OpenError) -> CliError {
    match error {
        OpenError::Path { kind } => {
            let status = ExitStatus::from_io_kind(*kind);
            let (code, message) = match status {
                ExitStatus::NoInput => ("no-input", "입력 경로가 없습니다"),
                ExitStatus::NoPerm => ("no-permission", "입력 경로에 접근할 권한이 없습니다"),
                _ => ("io-error", "입력 경로를 읽을 수 없습니다"),
            };
            CliError::new(status, code, message)
        }
        OpenError::NotADirectory => {
            not_a_project("not-a-directory", "입력 경로가 디렉터리가 아닙니다")
        }
        OpenError::NotAProject(RootError::MissingManifest) => {
            not_a_project("missing-manifest", &RootError::MissingManifest.to_string())
        }
        OpenError::NotAProject(RootError::MissingGit) => {
            not_a_project("missing-git", &RootError::MissingGit.to_string())
        }
        OpenError::ManifestRead { kind } => CliError::new(
            ExitStatus::from_io_kind(*kind),
            "manifest-read",
            format!("stellar.yaml을 읽을 수 없습니다: {kind:?}"),
        ),
        OpenError::ManifestSyntax(e) => CliError::new(
            ExitStatus::DataErr,
            "manifest-syntax",
            "stellar.yaml YAML 문법 오류",
        )
        .with_diagnostic(Diagnostic {
            severity: Severity::Error,
            code: e.kind.code(),
            message: format!("{}행 {}열: {}", e.line, e.col, e.message),
        }),
        OpenError::ManifestSchema(e) => CliError::new(
            ExitStatus::DataErr,
            "manifest-schema",
            "stellar.yaml 스키마 오류",
        )
        .with_diagnostic(Diagnostic {
            severity: Severity::Error,
            code: "schema",
            message: e.to_string(),
        }),
        OpenError::InvalidProjectId { .. } => CliError::new(
            ExitStatus::DataErr,
            "invalid-project-id",
            "stellar.yaml의 project.id가 UUID가 아닙니다",
        ),
        OpenError::Storage(e) => CliError::new(
            ExitStatus::DataErr,
            "storage",
            "저장된 generation을 열 수 없습니다",
        )
        .with_diagnostic(Diagnostic {
            severity: Severity::Error,
            code: "storage",
            message: e.to_string(),
        }),
        OpenError::Document(e) => document_error(e),
    }
}

/// 문서 읽기 실패.
pub fn document_error(error: &DocumentReadError) -> CliError {
    match error {
        DocumentReadError::UnknownDocument { document } => CliError::new(
            ExitStatus::NoInput,
            "unknown-document",
            format!("현재 generation에 문서 {document}가 없습니다"),
        ),
        other => CliError::new(ExitStatus::DataErr, "document", "문서를 읽을 수 없습니다")
            .with_diagnostic(Diagnostic {
                severity: Severity::Error,
                code: "document",
                message: other.to_string(),
            }),
    }
}

/// 조회 실패.
pub fn query_error(error: &QueryError) -> CliError {
    match error {
        QueryError::Document(e) => document_error(e),
        QueryError::UnknownNode { .. } => {
            CliError::new(ExitStatus::NoInput, "unknown-node", error.to_string())
        }
        QueryError::InvalidLimit { .. } => CliError::usage(error.to_string()),
        QueryError::InvalidCursor
        | QueryError::CursorDocumentMismatch { .. }
        | QueryError::CursorOutOfRange { .. } => {
            CliError::new(ExitStatus::DataErr, "invalid-cursor", error.to_string())
        }
        QueryError::StaleCursor { .. } => {
            CliError::new(ExitStatus::DataErr, "stale-cursor", error.to_string())
        }
        QueryError::Projection(_) => {
            CliError::new(ExitStatus::Software, "projection", error.to_string())
        }
    }
}

/// semantic diff 위치.
fn diff_position(position: Option<&DiffPosition>) -> JsonValue {
    match position {
        None => JsonValue::Null,
        Some(p) => JsonValue::object(vec![
            ("parent", opt_id(p.parent)),
            (
                "order_key",
                JsonValue::opt_string(p.order_key_hex.as_deref()),
            ),
        ]),
    }
}

/// 선택 속성 값(`{"type","value"}` 또는 `null`).
fn opt_value(value: Option<&OpsValue>) -> JsonValue {
    value.map_or(JsonValue::Null, |v| {
        JsonValue::object(vec![
            ("type", JsonValue::string(v.type_name())),
            ("value", value_inner(v)),
        ])
    })
}

/// semantic diff 변경 한 건.
pub fn diff_entry(entry: &DiffEntry) -> JsonValue {
    let mut fields = vec![
        ("kind", JsonValue::string(entry.kind)),
        ("document", opt_id(entry.document)),
        ("object", id(entry.object)),
        (
            "object_kind",
            JsonValue::opt_string(entry.object_kind.as_deref()),
        ),
        ("path", JsonValue::opt_string(entry.path.as_deref())),
        ("key", JsonValue::opt_string(entry.key.as_deref())),
        ("old", opt_value(entry.old.as_ref())),
        ("new", opt_value(entry.new.as_ref())),
        ("from", diff_position(entry.from.as_ref())),
        ("to", diff_position(entry.to.as_ref())),
    ];
    if let Some(component) = entry.component {
        fields.push(("component", id(component)));
    }
    JsonValue::object(fields)
}

/// semantic diff: `{"count", "by_kind", "entries"}`(canonical 순서, XIV.3.8).
pub fn diff_report(report: &DiffReport) -> JsonValue {
    JsonValue::object(vec![
        ("count", count(report.entries.len())),
        (
            "by_kind",
            JsonValue::Object(
                report
                    .counts()
                    .into_iter()
                    .map(|(kind, n)| (kind.to_string(), count(n)))
                    .collect(),
            ),
        ),
        (
            "entries",
            JsonValue::Array(report.entries.iter().map(diff_entry).collect()),
        ),
    ])
}
