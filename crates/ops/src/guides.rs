//! Frame 가이드·레이아웃 그리드와 View 변수·Variant headless 조회 (XII.10.19·XII.10.20·XVI.11.1, ADR-0126).
//!
//! `stellar guides`와 MCP `document.guides`가 같은 [`read_snapshot_guides`]를 씁니다. 줄 문장은 텍스트 원본과 같은 문서
//! crate formatter(`RulerGuide::to_line`·`LayoutGrid::to_line`)이고, 값 문장은 텍스트 원본 View 변수 표기
//! (`source_v4::view_data::value_text`)입니다. 레이아웃 그리드 트랙은 Frame 크기로 `LayoutGrid::tracks`가 계산한 값입니다.

use std::fmt;
use std::path::Path;

use stellar_document::css::value::format_length;
use stellar_document::guides::{GuideAxis, NodeGuides};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_document::view_variant::commands::{VariantDiagnostic, ViewVariantModel};
use stellar_document::view_variant::condition::ConditionTerm;
use stellar_project::document_io::source_v4::context::type_name;
use stellar_project::document_io::source_v4::view_data::value_text;
use stellar_semantic_diff::core_types::ids::StableId;

use crate::error::{DocumentReadError, OpenError};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};

/// 가이드 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideRow {
    /// 가이드 id.
    pub id: StableId,
    /// `x`(세로선) 또는 `y`(가로선).
    pub axis: &'static str,
    /// 소유 Frame local 위치(CSS 길이 문장, 예: `120px`).
    pub position: String,
    /// 텍스트 원본 한 줄.
    pub line: String,
}

/// 레이아웃 그리드 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    /// 텍스트 원본 한 줄.
    pub line: String,
    /// 표시 여부.
    pub visible: bool,
    /// Frame 크기로 계산한 x 축 트랙 `(시작, 끝)`(CSS 길이 문장).
    pub columns: Vec<(String, String)>,
    /// Frame 크기로 계산한 y 축 트랙 `(시작, 끝)`(CSS 길이 문장).
    pub rows: Vec<(String, String)>,
}

/// Frame 하나의 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGuidesRow {
    /// 소유 Frame.
    pub owner: StableId,
    /// 가이드(id 순).
    pub guides: Vec<GuideRow>,
    /// 그리드(그리는 순).
    pub grids: Vec<GridRow>,
}

/// View 변수 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableRow {
    /// 변수 id.
    pub id: StableId,
    /// 이름.
    pub name: String,
    /// 타입 이름(Context 원본과 같은 문법).
    pub value_type: String,
    /// default 값 문장(문장으로 쓸 수 없으면 `None`).
    pub default: Option<String>,
    /// 허용 enum 값 문장.
    pub allowed: Vec<String>,
}

/// Variant 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantRow {
    /// Variant id.
    pub id: StableId,
    /// 조건 term 문장(`<변수 id> any`, `<변수 id> = <값 문장>`, 쓸 수 없는 값은 `<변수 id> = ?`).
    pub when: Vec<String>,
    /// property patch 수.
    pub property_patches: usize,
    /// 구조 patch 수.
    pub structure_patches: usize,
    /// Slot change 수.
    pub slot_changes: usize,
}

/// View 하나의 변수·Variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVariantsRow {
    /// View.
    pub view: StableId,
    /// 변수(id 순).
    pub variables: Vec<VariableRow>,
    /// Variant(id 순).
    pub variants: Vec<VariantRow>,
    /// orphan·무효 조건 진단 문장.
    pub diagnostics: Vec<String>,
}

/// 조회 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidesReport {
    /// 읽은 프로젝트 revision.
    pub revision: ProjectRevision,
    /// 문서.
    pub document: StableId,
    /// 가이드·그리드가 있는 Frame(StableId 순, `node`를 주면 그 Frame만).
    pub frames: Vec<FrameGuidesRow>,
    /// 변수·Variant가 있는 View(StableId 순, `node`를 주면 비움).
    pub views: Vec<ViewVariantsRow>,
}

/// 조회 실패.
#[derive(Debug)]
pub enum GuidesReadError {
    /// 프로젝트를 열 수 없습니다.
    Open(OpenError),
    /// 문서를 읽을 수 없습니다.
    Document(DocumentReadError),
    /// `node`가 문서에 없거나 Frame이 아닙니다.
    NotAFrame(StableId),
}

impl fmt::Display for GuidesReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuidesReadError::Open(e) => write!(f, "{e}"),
            GuidesReadError::Document(e) => write!(f, "{e}"),
            GuidesReadError::NotAFrame(id) => write!(f, "node {id}가 문서의 Frame이 아닙니다"),
        }
    }
}

impl std::error::Error for GuidesReadError {}

fn frame_row(model: &DocumentModel, record: &NodeGuides) -> FrameGuidesRow {
    let size = match model.payload(record.owner()) {
        Some(NodePayload::Frame(frame)) => (frame.size.width(), frame.size.height()),
        _ => (0.0, 0.0),
    };
    FrameGuidesRow {
        owner: record.owner(),
        guides: record
            .guides()
            .map(|guide| GuideRow {
                id: guide.id,
                axis: match guide.axis {
                    GuideAxis::X => "x",
                    GuideAxis::Y => "y",
                },
                position: format_length(guide.position),
                line: guide.to_line(),
            })
            .collect(),
        grids: record
            .grids()
            .iter()
            .map(|grid| {
                let [columns, rows] = grid.tracks(size.0, size.1);
                let text = |tracks: Vec<(f32, f32)>| -> Vec<(String, String)> {
                    tracks
                        .into_iter()
                        .map(|(start, end)| (format_length(start), format_length(end)))
                        .collect()
                };
                GridRow {
                    line: grid.to_line(),
                    visible: grid.visible,
                    columns: text(columns),
                    rows: text(rows),
                }
            })
            .collect(),
    }
}

fn view_row(model: &ViewVariantModel) -> ViewVariantsRow {
    ViewVariantsRow {
        view: model.variables().view(),
        variables: model
            .variables()
            .iter()
            .map(|variable| VariableRow {
                id: variable.id().stable_id(),
                name: variable.name().to_owned(),
                value_type: type_name(variable.value_type()),
                default: value_text(variable.default_value()),
                allowed: variable
                    .allowed_values()
                    .filter_map(|v| {
                        value_text(
                            &stellar_document::view_variant::variable::VariableValue::Value(
                                stellar_semantic_diff::core_types::typed_value::TypedValue::Enum(v),
                            ),
                        )
                    })
                    .collect(),
            })
            .collect(),
        variants: model
            .variants()
            .map(|variant| VariantRow {
                id: variant.id.stable_id(),
                when: variant
                    .condition
                    .terms()
                    .map(|(variable, term)| {
                        let id = variable.stable_id();
                        match term {
                            ConditionTerm::Any => format!("{id} any"),
                            ConditionTerm::Equals(value) => format!(
                                "{id} = {}",
                                value_text(value).unwrap_or_else(|| "?".to_owned())
                            ),
                        }
                    })
                    .collect(),
                property_patches: variant.overrides.property_patches().count(),
                structure_patches: variant.overrides.structure_patches().count(),
                slot_changes: variant.overrides.slot_changes().count(),
            })
            .collect(),
        diagnostics: model
            .diagnostics()
            .into_iter()
            .map(|diagnostic| match diagnostic {
                VariantDiagnostic::OrphanCondition { variant, variable } => format!(
                    "Variant {}의 조건이 없는 변수 {}를 씁니다",
                    variant.stable_id(),
                    variable.stable_id()
                ),
                VariantDiagnostic::InvalidConditionValue { variant, variable } => format!(
                    "Variant {}의 변수 {} 조건 값이 허용되지 않습니다",
                    variant.stable_id(),
                    variable.stable_id()
                ),
            })
            .collect(),
    }
}

/// 문서 모델의 가이드·View 변수 조회(모듈 문서). `node`를 주면 그 Frame의 기록만 냅니다(없으면 빈 목록).
pub fn guides_of_model(
    model: &DocumentModel,
    node: Option<StableId>,
) -> Result<(Vec<FrameGuidesRow>, Vec<ViewVariantsRow>), GuidesReadError> {
    if let Some(node) = node {
        if !matches!(model.payload(node), Some(NodePayload::Frame(_))) {
            return Err(GuidesReadError::NotAFrame(node));
        }
        return Ok((
            model
                .node_guides(node)
                .map(|record| frame_row(model, record))
                .into_iter()
                .collect(),
            Vec::new(),
        ));
    }
    let records = model
        .to_records()
        .map_err(|e| GuidesReadError::Document(DocumentReadError::Records(e.to_string())))?;
    Ok((
        records
            .node_guides
            .iter()
            .map(|record| frame_row(model, record))
            .collect(),
        records.view_variants.iter().map(view_row).collect(),
    ))
}

/// 열린 snapshot의 문서 하나에서 조회합니다.
pub fn read_snapshot_guides(
    snapshot: &ProjectSnapshot,
    document: StableId,
    node: Option<StableId>,
) -> Result<GuidesReport, GuidesReadError> {
    let model = snapshot
        .document_model(document)
        .map_err(GuidesReadError::Document)?;
    let (frames, views) = guides_of_model(&model, node)?;
    Ok(GuidesReport {
        revision: snapshot.revision(),
        document,
        frames,
        views,
    })
}

/// `path` 프로젝트의 문서 하나를 읽어 [`read_snapshot_guides`]를 부릅니다.
pub fn read_guides(
    path: &Path,
    document: StableId,
    node: Option<StableId>,
) -> Result<GuidesReport, GuidesReadError> {
    let snapshot = open_snapshot(path).map_err(GuidesReadError::Open)?;
    read_snapshot_guides(&snapshot, document, node)
}
