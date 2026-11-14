//! 저장 generation의 지정 View → 고정 Free Frame HTML+CSS. 쓰기·schema 변환 없음.

use std::collections::BTreeMap;

use stellar_accessibility::generate::generate;
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::source::{SourceKind, SourceNode, SourceTree};
use stellar_codegen::ir::build_accessibility_ir;
use stellar_codegen::web::{
    aria::{element_id, map_accessibility},
    check::check_document,
    frame::FrameStyle,
};
use stellar_document::frame::{Clipping, FramePayload};
use stellar_document::modifier::{ModifierPayload, ModifierStack};
use stellar_document::records::NodePayload;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::validate::ViewNode;

use crate::accessibility_html::{HtmlDiagnostic, HtmlError};
use crate::core_types::{
    ids::StableId,
    value::{FiniteF32, Transform2D, Vec2},
};
use crate::revision::ProjectRevision;
use crate::snapshot::ProjectSnapshot;

/// 읽은 범위와 결과. 오류가 하나라도 있으면 부분 HTML도 반환하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFrameHtml {
    /// 열린 저장 generation.
    pub revision: ProjectRevision,
    /// 요청 문서.
    pub document: StableId,
    /// 요청 View.
    pub view: StableId,
    /// 선택한 View의 root.
    pub root_frame: StableId,
    /// CSS를 포함한 독립 HTML 문서. script·외부 자원 없음.
    pub html: Option<String>,
    /// 결정론 순서 진단. node 진단에는 StableId를 포함합니다.
    pub diagnostics: Vec<HtmlDiagnostic>,
}

impl WebFrameHtml {
    fn note(
        &mut self,
        error: bool,
        code: &'static str,
        node: StableId,
        message: impl std::fmt::Display,
    ) {
        self.diagnostics.push(HtmlDiagnostic {
            error,
            code,
            message: format!("node {node}: {message}"),
        });
    }

    fn failed(&self) -> bool {
        self.diagnostics.iter().any(|d| d.error)
    }
}

/// 저장된 문서의 View 하나를 읽습니다. GUI의 미저장 편집·활성 Variant는 입력이 아닙니다.
pub fn export_web_frame(
    snapshot: &ProjectSnapshot,
    document: StableId,
    view: StableId,
) -> Result<WebFrameHtml, HtmlError> {
    let generation = snapshot.generation().ok_or(HtmlError::Unsaved)?;
    if generation.manifest().schema_version != crate::migrate::CURRENT_SCHEMA_VERSION {
        return Err(HtmlError::UnsupportedSchema(
            generation.manifest().schema_version,
        ));
    }
    let model = snapshot
        .document_model(document)
        .map_err(HtmlError::Document)?;
    let selected = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)
        .ok_or(HtmlError::UnknownView(view))?;
    let root = selected.root_frame;
    let mut result = WebFrameHtml {
        revision: snapshot.revision(),
        document,
        view,
        root_frame: root,
        html: None,
        diagnostics: Vec::new(),
    };
    result.note(false, "fixed-free-frame-only", root,
        "저장된 고정 Free Frame·Fragment·Rectangle Shape를 출력합니다. root 캔버스 이동은 생략합니다. 이름에서 Text·control·의미를 추정하지 않으며 동작·반응형·Variant·Context·Motion·전체 코드 생성을 지원하지 않습니다.");
    if !snapshot.manifest().target_profiles.is_empty() {
        result.note(
            true,
            "unsupported-target-profile",
            root,
            "저장 profile을 해석할 수 없어 선택 View의 시각 결과를 확정할 수 없습니다",
        );
    }
    for kind in snapshot.chunk_kinds() {
        if !matches!(kind.name, Some("document-manifest" | "view-tree")) {
            result.note(
                true,
                "unsupported-record",
                root,
                format!(
                    "chunk kind {} ({})의 의미·영향 범위를 판정할 수 없습니다",
                    kind.code,
                    kind.name.unwrap_or("unknown")
                ),
            );
        }
    }
    if generation.manifest().chunk_pointers.iter().any(|p| {
        p.schema_version != 1 && !(p.kind == KIND_DOCUMENT_MANIFEST && p.schema_version == 2)
    }) {
        result.note(
            true,
            "unsupported-record-schema",
            root,
            "문서 manifest는 schema 1·2, 고정 도형을 담는 View와 subtree는 schema 1을 지원합니다",
        );
    }
    // 문서 전체를 매 node마다 훑지 않고 한 번만 OrderKey 순서를 구성합니다.
    let mut children_by_parent = BTreeMap::new();
    for handle in model.columns().iter_live() {
        if let Some(fields) = model.columns().get(handle) {
            if let Some(parent) = fields.parent.and_then(|p| model.stable_id(p)) {
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
    source.push_root(root);
    let mut nodes = Vec::new();
    let mut styles = BTreeMap::new();
    let mut stack = vec![(root, None, true, Transform2D::identity())];
    while let Some((id, parent_border, ancestor_visible, parent_transform)) = stack.pop() {
        let Some(record) = model.record(id) else {
            result.note(
                true,
                "invalid-view-tree",
                id,
                "저장 node를 읽을 수 없습니다",
            );
            continue;
        };
        let children: Vec<_> = children_by_parent
            .get(&id)
            .into_iter()
            .flatten()
            .map(|(_, child)| *child)
            .collect();
        let visible = ancestor_visible && record.visible;
        let mut child_border = parent_border;
        let mut child_transform = parent_transform;
        let rectangle = matches!(
            &record.payload,
            NodePayload::Shape(ShapeGeometry::Rectangle(_))
        );
        let projected = if let NodePayload::Shape(ShapeGeometry::Rectangle(shape)) = &record.payload
        {
            let (tl, tr, br, bl) = shape.effective_corner_radius();
            if [tl, tr, br, bl].iter().any(|radius| *radius > 0.0) {
                result.note(false, "browser-rectangle-curve", id,
                    "둥근 Rectangle의 모서리 안티앨리어싱은 브라우저와 캔버스의 래스터화 방식에 따라 달라질 수 있습니다");
            }
            match rectangle_frame(shape, model.modifier_stack(id)) {
                Ok(frame) => Some(NodePayload::Frame(frame)),
                Err(message) => {
                    result.note(true, "unsupported-modifier", id, message);
                    continue;
                }
            }
        } else {
            None
        };
        if !rectangle && model.modifier_stack(id).is_some() {
            result.note(
                true,
                "unsupported-modifier",
                id,
                "modifier stack을 CSS로 해석하지 않습니다",
            );
        }
        let supported = match projected.as_ref().unwrap_or(&record.payload) {
            NodePayload::Frame(frame) => {
                if frame.layout_mode.is_some() {
                    result.note(
                        true,
                        "unsupported-layout",
                        id,
                        "layout 참조가 없는 고정 Free Frame만 지원합니다",
                    );
                }
                if matches!(frame.clipping, Clipping::Path(_)) {
                    result.note(
                        true,
                        "unsupported-clip",
                        id,
                        "Path clipping은 지원하지 않습니다",
                    );
                }
                let (tl, tr, br, bl) = frame.corner_radius.components();
                let border = frame.border.as_ref().map(|b| {
                    (
                        b.color,
                        FiniteF32::new(b.thickness()).expect("validated border"),
                    )
                });
                let style = FrameStyle {
                    size: frame.size,
                    transform: frame.transform,
                    background: frame.background,
                    border,
                    radii: [tl, tr, br, bl].map(|v| FiniteF32::new(v).expect("validated radius")),
                    clip: frame.clipping == Clipping::FrameBounds,
                    visible,
                    parent_border,
                };
                if let Err(message) = style.validate() {
                    result.note(true, "unsupported-frame-value", id, message);
                }
                let (a, b, c, d, _, _) = frame.transform.components();
                let local = if id == root {
                    Transform2D::new(a, b, c, d, 0.0, 0.0).expect("validated affine")
                } else {
                    frame.transform
                };
                let world = local.then(&parent_transform);
                let bounded = world.as_ref().is_ok_and(|world| {
                    [
                        (0.0, 0.0),
                        (frame.size.width(), 0.0),
                        (0.0, frame.size.height()),
                        (frame.size.width(), frame.size.height()),
                    ]
                    .iter()
                    .all(|&(x, y)| {
                        world
                            .apply(Vec2::new(x, y).expect("validated size"))
                            .is_ok_and(|p| p.x().abs() <= 1_000_000.0 && p.y().abs() <= 1_000_000.0)
                    })
                });
                if !bounded {
                    result.note(
                        true,
                        "unsupported-frame-value",
                        id,
                        "누적 affine 이후 모서리 좌표가 유한한 ±1,000,000px 범위를 넘어갑니다",
                    );
                }
                if rectangle && world.as_ref().is_ok_and(|w| w.inverse().is_none()) {
                    result.note(
                        true,
                        "unsupported-frame-value",
                        id,
                        "Rectangle의 누적 affine은 역변환 가능해야 합니다",
                    );
                }
                if let Ok(world) = world {
                    if !style.size_rounding_fits(&world) {
                        result.note(true, "unsupported-frame-value", id,
                            "누적 affine이 CSS 크기의 LayoutUnit 반올림을 화면에서 1/64px보다 크게 확대할 수 있습니다");
                    }
                    child_transform = world;
                }
                if frame
                    .background
                    .into_iter()
                    .chain(frame.border.as_ref().map(|b| b.color))
                    .any(|c| c.alpha() > 0.0 && c.alpha() < 1.0)
                {
                    result.note(false, "browser-color-compositing", id, "색공간·straight RGBA 선언은 보존합니다. 브라우저 alpha 합성은 Stellar 선형 합성과 달라 최종 픽셀 동일성은 보장하지 않습니다");
                }
                child_border = Some(border.map_or(FiniteF32::ZERO, |(_, w)| w));
                styles.insert(id, style);
                Some(if rectangle {
                    (SourceKind::Graphic, ElementKind::SHAPE)
                } else {
                    (SourceKind::Frame, ElementKind::FRAME)
                })
            }
            NodePayload::Fragment(_) => Some((SourceKind::Fragment, ElementKind::FRAGMENT)),
            other => {
                result.note(
                    true,
                    "unsupported-node",
                    id,
                    format!(
                        "{:?}의 시각·텍스트 스타일·의미는 이 출력 범위에서 해석할 수 없습니다",
                        other.kind()
                    ),
                );
                None
            }
        };
        stack.extend(
            children
                .iter()
                .rev()
                .map(|child| (*child, child_border, visible, child_transform)),
        );
        if let Some((kind, element)) = supported {
            let mut node = SourceNode::new(id, kind).with_children(children);
            node.hidden = !visible;
            if let Err(e) = source.insert(node) {
                result.note(true, "invalid-view-tree", id, e);
            }
            nodes.push(ViewNode {
                id,
                parent: record.parent,
                element,
                contracts: Vec::new(),
            });
        }
    }
    if result.failed() {
        return Ok(result);
    }
    let mapped = (|| {
        let tree = generate(&source).map_err(|e| e.to_string())?;
        let accessibility = AccessibilitySnapshot::from_tree(&tree).map_err(|e| e.to_string())?;
        let ir = build_accessibility_ir(&nodes, &accessibility, &BTreeMap::new())
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(map_accessibility(&ir))
    })();
    let mut mapped = match mapped {
        Ok(mapped) => mapped,
        Err(e) => {
            result.note(true, "web-frame-projection", root, e);
            return Ok(result);
        }
    };
    for note in &mapped.notes {
        result.note(
            true,
            "accessibility-mapping",
            note.node,
            format!("{:?}", note.kind),
        );
    }
    let mut css = String::from("html, body {\n  margin: 0;\n  padding: 0;\n}\n");
    for index in 0..mapped.document.elements().len() as u32 {
        let id = mapped
            .document
            .element(index)
            .source
            .expect("mapped source");
        let name = element_id(id);
        mapped.document.set_attr(index, "id", &name);
        css.push_str(&format!("#{name} {{\n{}}}\n", styles[&id].declarations()));
    }
    for issue in check_document(&mapped.document) {
        result.note(true, "html-accessibility-check", root, format!("{issue:?}"));
    }
    if !result.failed() {
        result.html = Some(format!("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Stellar View {view}</title>\n<style>\n{css}</style>\n</head>\n<body>\n{}</body>\n</html>\n", mapped.to_html()));
    }
    Ok(result)
}

// 로컬 main의 runtime::canvas::shape::rectangle_style과 같은 지원 부분집합.
// Shape는 컨테이너가 아니지만 기존 CSS 값 검증·formatter 입력을 재사용합니다.
fn rectangle_frame(
    shape: &RectangleParams,
    stack: Option<&ModifierStack>,
) -> Result<FramePayload, &'static str> {
    let mut transform = Transform2D::identity();
    let mut background = None;
    if let Some(stack) = stack {
        for entry in stack.entries().iter().filter(|e| e.enabled) {
            match entry.payload {
                ModifierPayload::Transform(next) if background.is_none() => {
                    transform = transform
                        .then(&next)
                        .map_err(|_| "Rectangle affine 합성이 비유한합니다")?;
                }
                ModifierPayload::Fill { color } if background.is_none() => background = Some(color),
                _ => return Err("Rectangle은 활성 Transform 다음 단일 Fill만 지원합니다"),
            }
        }
    }
    Ok(FramePayload {
        size: shape.size,
        transform,
        background,
        corner_radius: shape.corner_radius,
        clipping: Clipping::Disabled,
        ..FramePayload::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{
        file_digest, project_id, save_documents, scaffold_project, web_frame as fixture, TempDir,
    };
    use stellar_storage::chunk::{envelope::ChunkKindCode, id::ChunkId, store::ChunkStore};
    use stellar_storage::generation::{
        current::write_current_pointer,
        loader::{load_current_generation, write_manifest_file},
    };
    use stellar_storage::hash::ContentHash;

    #[test]
    fn snapshot_stays_pinned_and_root_canvas_moves_do_not_change_html() {
        let dir = TempDir::new("web-revision");
        scaffold_project(&dir.path);
        let mut model = fixture::document();
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let first = export_web_frame(&snapshot, fixture::id(1), fixture::id(4)).unwrap();
        let NodePayload::Frame(mut frame) = model.payload(fixture::id(20)).unwrap().clone() else {
            unreachable!()
        };
        frame.transform = Transform2D::new(1.25, 0.0, 0.0, 1.5, f32::MAX, -f32::MAX).unwrap();
        model
            .set_payload(fixture::id(20), NodePayload::Frame(frame))
            .unwrap();
        save_documents(&dir.path, &[&model]);
        let before = file_digest(&dir.path);
        let moved = export_web_frame(
            &open_snapshot(&dir.path).unwrap(),
            fixture::id(1),
            fixture::id(4),
        )
        .unwrap();
        assert!(first.html.is_some());
        assert_eq!(moved.html, first.html);
        assert_ne!(moved.revision, first.revision);
        assert_eq!(
            export_web_frame(&snapshot, fixture::id(1), fixture::id(4)).unwrap(),
            first
        );
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn rectangle_snapshot_remains_pinned_after_modifier_save() {
        let dir = TempDir::new("web-rectangle-revision");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&fixture::rectangle("supported")]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let first = export_web_frame(&snapshot, fixture::id(1), fixture::id(4)).unwrap();
        assert!(first.html.is_some());
        save_documents(&dir.path, &[&fixture::rectangle("after-fill")]);
        let before = file_digest(&dir.path);
        let next = export_web_frame(
            &open_snapshot(&dir.path).unwrap(),
            fixture::id(1),
            fixture::id(4),
        )
        .unwrap();
        assert!(next.html.is_none());
        assert_ne!(first.revision, next.revision);
        assert_eq!(
            export_web_frame(&snapshot, fixture::id(1), fixture::id(4)).unwrap(),
            first
        );
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn document_typography_leaves_geometric_export_unchanged() {
        let dir = TempDir::new("web-document-typography");
        scaffold_project(&dir.path);
        let model = fixture::rectangle("supported");
        save_documents(&dir.path, &[&model]);
        let export = || {
            let snapshot = open_snapshot(&dir.path).unwrap();
            export_web_frame(&snapshot, fixture::id(1), fixture::id(4)).unwrap()
        };
        let before = export().html.expect("v1 geometry");
        let mut records = model.to_records().unwrap();
        records.definition.text_styles =
            stellar_document::text_styles::DocumentTextStyles::initial(fixture::id(99));
        let (styled, _) = stellar_document::model::DocumentModel::from_records(records).unwrap();
        save_documents(&dir.path, &[&styled]);
        let digest = file_digest(&dir.path);
        assert_eq!(export().html.as_deref(), Some(before.as_str()));
        assert_eq!(file_digest(&dir.path), digest);
    }

    #[test]
    fn opaque_records_and_chunk_schemas_cannot_silently_drop_semantics() {
        let dir = TempDir::new("web-records");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&fixture::document()]);
        let original = load_current_generation(&dir.path, project_id()).unwrap();
        for schema_case in [false, true] {
            let mut manifest = original.manifest.clone();
            if schema_case {
                manifest.chunk_pointers[0].schema_version = 2;
            } else {
                let mut extra = manifest.chunk_pointers[0];
                extra.chunk_id = ChunkId::from_stable_id(fixture::id(99));
                extra.kind = ChunkKindCode(999);
                let payload = b"opaque semantic data";
                ChunkStore::new(&dir.path)
                    .write_chunk(extra.kind, payload)
                    .unwrap();
                extra.content_hash = ContentHash::compute(payload);
                manifest.chunk_pointers.push(extra);
            }
            let hash = write_manifest_file(&dir.path, &manifest).unwrap();
            write_current_pointer(&dir.path, &hash).unwrap();
            let before = file_digest(&dir.path);
            let report = export_web_frame(
                &open_snapshot(&dir.path).unwrap(),
                fixture::id(1),
                fixture::id(3),
            )
            .unwrap();
            assert!(report.html.is_none());
            assert!(report.diagnostics.iter().any(|d| d.error
                && d.code
                    == if schema_case {
                        "unsupported-record-schema"
                    } else {
                        "unsupported-record"
                    }
                && d.message.contains(&fixture::id(10).to_string())));
            assert_eq!(file_digest(&dir.path), before);
        }
    }
}
