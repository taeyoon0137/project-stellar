//! 저장된 문서의 지정 View만 정적 HTML/ARIA로 투영합니다.
//!
//! Frame·Fragment·Text의 구조, OrderKey 순서, 가시성과 저장 문자열만 사용합니다.
//! 저장 모델에 없는 control·상태·token·profile capability·layout 값을 추정하지 않습니다.
//! `CodegenSources` 대신 codegen의 접근성 전용 IR 입력과 기존 mapper·정적 검사를 씁니다.

use std::collections::BTreeMap;

use stellar_accessibility::generate::generate;
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::source::{SourceKind, SourceNode, SourceTree};
use stellar_codegen::ir::build_accessibility_ir;
use stellar_codegen::web::{aria::map_accessibility, check::check_document};
use stellar_document::records::NodePayload;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::validate::ViewNode;

use crate::core_types::ids::StableId;
use crate::error::DocumentReadError;
use crate::revision::ProjectRevision;
use crate::snapshot::ProjectSnapshot;

/// 정보 손실·미지원 데이터 또는 정적 검사 진단(결정론 순서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDiagnostic {
    /// 실패를 일으키는 진단인지.
    pub error: bool,
    /// 안정 분류.
    pub code: &'static str,
    /// 진단 문맥과 설명. 원본 텍스트·이름은 복사하지 않습니다.
    pub message: String,
}

/// 읽은 revision과 지정 범위의 결과. 오류 진단이 있으면 HTML은 제공하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityHtml {
    /// 열린 저장 generation의 revision.
    pub revision: ProjectRevision,
    /// 요청 문서.
    pub document: StableId,
    /// 요청 View.
    pub view: StableId,
    /// View의 실제 root Frame.
    pub root_frame: StableId,
    /// 성공 시 HTML 조각. CSS·script·에셋을 포함하지 않습니다.
    pub html: Option<String>,
    /// 범위·누락·매핑·정적 검사 진단.
    pub diagnostics: Vec<HtmlDiagnostic>,
}

/// HTML을 만들기 전 입력을 읽거나 찾지 못했습니다.
#[derive(Debug)]
pub enum HtmlError {
    /// 저장된 generation 없음.
    Unsaved,
    /// 이 도구가 해석하지 않는 generation 스키마.
    UnsupportedSchema(u32),
    /// 문서 로드 실패.
    Document(DocumentReadError),
    /// 그 문서에 요청 View가 없음.
    UnknownView(StableId),
}

impl std::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsaved => f.write_str("저장된 generation이 없습니다"),
            Self::UnsupportedSchema(version) => write!(
                f,
                "generation schema {version}은 지원하지 않습니다(지원: {})",
                crate::migrate::CURRENT_SCHEMA_VERSION
            ),
            Self::Document(e) => write!(f, "{e}"),
            Self::UnknownView(id) => write!(f, "요청 문서에 View {id}가 없습니다"),
        }
    }
}

impl std::error::Error for HtmlError {}

impl AccessibilityHtml {
    fn note(&mut self, error: bool, code: &'static str, message: impl Into<String>) {
        self.diagnostics.push(HtmlDiagnostic {
            error,
            code,
            message: message.into(),
        });
    }
}

/// snapshot에 고정된 문서를 읽고, 정의의 `View.root_frame` 하위만 내보냅니다. 파일 쓰기 없음.
pub fn export_accessibility(
    snapshot: &ProjectSnapshot,
    document: StableId,
    view: StableId,
) -> Result<AccessibilityHtml, HtmlError> {
    if snapshot.revision() == ProjectRevision::Unsaved {
        return Err(HtmlError::Unsaved);
    }
    if let Some(generation) = snapshot.generation() {
        let version = generation.manifest().schema_version;
        if version != crate::migrate::CURRENT_SCHEMA_VERSION {
            return Err(HtmlError::UnsupportedSchema(version));
        }
    }
    let model = snapshot
        .document_model(document)
        .map_err(HtmlError::Document)?;
    let definition = model.definition();
    let selected = definition
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)
        .ok_or(HtmlError::UnknownView(view))?;
    let root_frame = selected.root_frame;
    let mut result = AccessibilityHtml {
        revision: snapshot.revision(),
        document,
        view,
        root_frame,
        html: None,
        diagnostics: Vec::new(),
    };
    result.note(false, "static-accessibility-only",
        "정적 접근성 HTML 조각만 생성합니다. Frame은 generic, Text는 staticText, Fragment는 펼칩니다. 표시 이름으로 role·label을 추정하지 않습니다. control·action·binding·상태·token·Variant·Context·Motion은 저장 모델에서 연결되지 않아 내보내지 않습니다. 정적 검사는 브라우저·스크린리더 적합성 보증이 아닙니다.");
    result.note(false, "visuals-not-exported",
        "크기·transform·배치·clipping·paint·텍스트 스타일·에셋·CSS는 내보내지 않습니다. visible=false는 aria-hidden으로만 표현하며 브라우저 시각 레이아웃을 재현하지 않습니다.");
    for other in snapshot.documents() {
        for excluded in other.pages.iter().flat_map(|p| &p.views) {
            if other.id != document || excluded.id != view {
                result.note(
                    false,
                    "view-not-selected",
                    format!(
                        "문서 {} View {}는 선택 범위 밖이므로 내보내지 않습니다",
                        other.id, excluded.id
                    ),
                );
            }
        }
    }
    if !snapshot.manifest().target_profiles.is_empty() {
        result.note(false, "profiles-not-applied", "프로젝트 target_profiles를 적용하지 않습니다. 저장 profile 형식과 codegen capability pin의 연결이 없으며 대상 profile을 가정하지 않습니다.");
    }
    for kind in snapshot.chunk_kinds() {
        if !matches!(kind.name, Some("document-manifest" | "view-tree")) {
            result.note(
                false,
                "chunk-not-exported",
                format!(
                    "chunk kind {} ({}) {}개는 이 정적 문서 투영의 입력이 아닙니다",
                    kind.code,
                    kind.name.unwrap_or("unknown"),
                    kind.count
                ),
            );
        }
    }
    // children_in_order는 호출마다 문서 전체를 훑으므로 한 번만 부모별 순서를 구성합니다.
    let mut children_by_parent = BTreeMap::new();
    for handle in model.columns().iter_live() {
        if let Some(fields) = model.columns().get(handle) {
            if let Some(parent) = fields.parent.and_then(|parent| model.stable_id(parent)) {
                children_by_parent
                    .entry(parent)
                    .or_insert_with(Vec::new)
                    .push((fields.order_key, fields.stable_id));
            }
        }
    }
    for children in children_by_parent.values_mut() {
        children.sort();
    }
    let mut source = SourceTree::new();
    source.push_root(root_frame);
    let mut nodes = Vec::new();
    let mut texts = BTreeMap::new();
    let mut stack = vec![root_frame];
    while let Some(id) = stack.pop() {
        let Some(record) = model.record(id) else {
            result.note(
                true,
                "invalid-view-tree",
                format!("View node {id}를 읽을 수 없습니다"),
            );
            return Ok(result);
        };
        let children: Vec<_> = children_by_parent
            .get(&id)
            .into_iter()
            .flatten()
            .map(|(_, child)| *child)
            .collect();
        stack.extend(children.iter().rev().copied());
        let (kind, element) = match &record.payload {
            NodePayload::Frame(frame) => {
                if frame.layout_mode.is_some() {
                    result.note(
                        false,
                        "layout-reference-not-exported",
                        format!("node {id}의 layout 참조는 해석하지 않습니다"),
                    );
                }
                (SourceKind::Frame, ElementKind::FRAME)
            }
            NodePayload::Fragment(_) => (SourceKind::Fragment, ElementKind::FRAGMENT),
            NodePayload::Text(text) => {
                if text.content().contains('\0') {
                    result.note(
                        true,
                        "unsupported-text-character",
                        format!("node {id}의 U+0000은 HTML 본문에서 보존할 수 없습니다"),
                    );
                    continue;
                }
                texts.insert(id, text.content().to_string());
                (SourceKind::Text, ElementKind::TEXT)
            }
            unsupported => {
                result.note(true, "unsupported-node", format!("node {id}의 {:?}는 접근성 이름·에셋·그래픽 의미를 복원할 수 없어 지원하지 않습니다", unsupported.kind()));
                continue;
            }
        };
        if model.modifier_stack(id).is_some() {
            result.note(
                false,
                "modifiers-not-exported",
                format!("node {id}의 modifier stack은 HTML에 반영하지 않습니다"),
            );
        }
        let mut node = SourceNode::new(id, kind).with_children(children);
        node.hidden = !record.visible;
        if let Err(e) = source.insert(node) {
            result.note(true, "invalid-view-tree", e.to_string());
            return Ok(result);
        }
        nodes.push(ViewNode {
            id,
            parent: record.parent,
            element,
            contracts: Vec::new(),
        });
    }
    if result.diagnostics.iter().any(|d| d.error) {
        return Ok(result);
    }
    let mapped = (|| {
        let tree = generate(&source).map_err(|e| e.to_string())?;
        let accessibility = AccessibilitySnapshot::from_tree(&tree).map_err(|e| e.to_string())?;
        let ir =
            build_accessibility_ir(&nodes, &accessibility, &texts).map_err(|e| e.to_string())?;
        Ok::<_, String>(map_accessibility(&ir))
    })();
    let mapped = match mapped {
        Ok(mapped) => mapped,
        Err(e) => {
            result.note(true, "accessibility-projection", e);
            return Ok(result);
        }
    };
    for note in &mapped.notes {
        result.note(
            true,
            "accessibility-mapping",
            format!("node {}: {:?}", note.node, note.kind),
        );
    }
    for issue in check_document(&mapped.document) {
        result.note(true, "html-accessibility-check", format!("{issue:?}"));
    }
    if !result.diagnostics.iter().any(|d| d.error) {
        result.html = Some(mapped.to_html());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{
        accessibility as fixture, file_digest, save_documents, scaffold_project, TempDir,
    };

    #[test]
    fn export_uses_the_opened_revision_even_after_another_save() {
        let dir = TempDir::new("html-revision");
        scaffold_project(&dir.path);
        let mut model = fixture::document();
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let first = export_accessibility(&snapshot, fixture::id(1), fixture::id(3)).unwrap();
        model
            .set_display_name(fixture::id(12), "나중 저장".into())
            .unwrap();
        save_documents(&dir.path, &[&model]);
        let before = file_digest(&dir.path);
        assert_eq!(
            export_accessibility(&snapshot, fixture::id(1), fixture::id(3)).unwrap(),
            first
        );
        assert_ne!(open_snapshot(&dir.path).unwrap().revision(), first.revision);
        assert_eq!(file_digest(&dir.path), before);
    }
}
