//! 저장 generation의 지정 View → 고정 Free Frame HTML+CSS. 쓰기·schema 변환 없음.

use std::collections::BTreeMap;

use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::instance::{frame_instance_transform, placed_free_frame};

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
use stellar_document::modifier::ModifierStack;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    evaluate_shape_modifiers, RectangleParams, ShapeGeometry, ShapeModifierError,
};
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_storage::hash::ContentHash;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::validate::ViewNode;

use crate::core_types::{
    ids::StableId,
    value::{FiniteF32, Transform2D, Vec2},
};
use crate::error::DocumentReadError;
use crate::revision::ProjectRevision;
use crate::snapshot::ProjectSnapshot;

/// 정보 손실·미지원 데이터 또는 정적 검사 진단(결정론 순서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDiagnostic {
    /// 실패를 일으키는 진단인지.
    pub error: bool,
    /// 진단 대상 node. 선택·이동에 사용합니다.
    pub node: StableId,
    /// 안정 분류.
    pub code: &'static str,
    /// 진단 문맥과 설명. 원본 텍스트·이름은 복사하지 않습니다.
    pub message: String,
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
    /// 요청 Frame이 View 안에 없거나 Frame이 아님.
    UnknownFrame(StableId),
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
            Self::UnknownFrame(id) => write!(f, "요청 View에 Frame {id}가 없습니다"),
        }
    }
}

impl std::error::Error for HtmlError {}

/// 읽은 범위와 결과. 오류가 하나라도 있으면 부분 HTML도 반환하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFrameHtml {
    /// 열린 저장 generation.
    pub revision: ProjectRevision,
    /// 독립 원본을 사용한 경우 그 파일 집합의 hash. 문서 generation과 독립적으로 바뀝니다.
    pub component_source_hash: Option<ContentHash>,
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
            node,
            code,
            message: format!("node {node}: {message}"),
        });
    }

    fn note_source(
        &mut self,
        components: &ComponentFrames,
        error: bool,
        code: &'static str,
        id: StableId,
        message: impl std::fmt::Display,
    ) {
        if let Some((instance, definition_node)) = components.sources.get(&id) {
            self.note(
                error,
                code,
                *instance,
                format!("정의 node {definition_node}: {message}"),
            );
        } else {
            self.note(error, code, id, message);
        }
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
    export_web_frame_node(snapshot, document, view, None)
}

/// 저장된 View 또는 그 안의 Frame 한 개를 HTML로 투영합니다.
pub fn export_web_frame_node(
    snapshot: &ProjectSnapshot,
    document: StableId,
    view: StableId,
    frame: Option<StableId>,
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
    let root = if let Some(frame) = frame {
        let mut current = frame;
        let mut belongs = false;
        loop {
            if current == selected.root_frame {
                belongs = true;
                break;
            }
            let Some(parent) = model.record(current).and_then(|record| record.parent) else {
                break;
            };
            current = parent;
        }
        if !belongs
            || !matches!(
                model.record(frame).map(|record| record.payload),
                Some(NodePayload::Frame(_))
            )
        {
            return Err(HtmlError::UnknownFrame(frame));
        }
        frame
    } else {
        selected.root_frame
    };
    let mut result = WebFrameHtml {
        revision: snapshot.revision(),
        component_source_hash: None,
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
    if !generation.manifest().chunk_pointers.iter().all(|p| {
        matches!(
            (p.kind, p.schema_version),
            (_, 1) | (KIND_DOCUMENT_MANIFEST, 2)
        )
    }) {
        result.note(
            true,
            "unsupported-record-schema",
            root,
            "문서 manifest 1·2와 View/subtree 1을 지원합니다",
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
    let mut components = ComponentFrames::default();
    let mut stack = vec![(root, None, true, Transform2D::identity())];
    while let Some((id, parent_border, ancestor_visible, parent_transform)) = stack.pop() {
        let (mut record, mut children) = if let Some(projected) = components.pending.remove(&id) {
            projected
        } else {
            let Some(record) = model.record(id) else {
                result.note(
                    true,
                    "invalid-view-tree",
                    id,
                    "저장 node를 읽을 수 없습니다",
                );
                continue;
            };
            let children = children_by_parent
                .get(&id)
                .into_iter()
                .flatten()
                .map(|(_, child)| *child)
                .collect();
            (record, children)
        };
        let instance = matches!(record.payload, NodePayload::ComponentInstance(_));
        if let NodePayload::ComponentInstance(ref payload) = record.payload {
            if !payload.has_no_overrides() || !children.is_empty() {
                result.note(
                    true,
                    "unsupported-component-instance",
                    id,
                    "override나 인스턴스 소유 자식이 있습니다",
                );
                continue;
            }
            if components.library.is_none() {
                let library = crate::component_source::project_library(snapshot).and_then(
                    |(library, files)| {
                        let hash = stellar_project::document_io::component_source::source_set_hash(
                            files.into_values(),
                        )?;
                        result.component_source_hash = Some(hash);
                        Ok(library)
                    },
                );
                match library {
                    Ok(library) => components.library = Some(library),
                    Err(error) => {
                        result.note(true, "invalid-component-source", id, error);
                        return Ok(result);
                    }
                }
            }
            let Some(position) = frame_instance_transform(model.modifier_stack(id)) else {
                result.note(
                    true,
                    "unsupported-modifier",
                    id,
                    "컴포넌트 인스턴스는 유한한 활성 Transform만 지원합니다",
                );
                continue;
            };
            let definition = payload.component();
            match components.expand(&record, definition, position, &model) {
                Ok(projected) => (record, children) = projected,
                Err(reason) => {
                    result.note(
                        true,
                        "unsupported-component-instance",
                        id,
                        format!("정의 {}: {reason}", definition.id()),
                    );
                    return Ok(result);
                }
            }
        }
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
        if !rectangle && !instance && model.modifier_stack(id).is_some() {
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
                    result.note_source(
                        &components,
                        true,
                        "unsupported-layout",
                        id,
                        "layout 참조가 없는 고정 Free Frame만 지원합니다",
                    );
                }
                if matches!(frame.clipping, Clipping::Path(_)) {
                    result.note_source(
                        &components,
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
                    result.note_source(&components, true, "unsupported-frame-value", id, message);
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
                    result.note_source(
                        &components,
                        true,
                        "unsupported-frame-value",
                        id,
                        "누적 affine 이후 모서리 좌표가 유한한 ±1,000,000px 범위를 넘어갑니다",
                    );
                }
                if (rectangle || instance || components.sources.contains_key(&id))
                    && world.as_ref().is_ok_and(|w| w.inverse().is_none())
                {
                    result.note_source(
                        &components,
                        true,
                        "unsupported-frame-value",
                        id,
                        "Rectangle·배치된 컴포넌트 Frame의 누적 affine은 역변환 가능해야 합니다",
                    );
                }
                if let Ok(world) = world {
                    if !style.size_rounding_fits(&world) {
                        result.note_source(&components, true, "unsupported-frame-value", id,
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
                    result.note_source(&components, false, "browser-color-compositing", id, "색공간·straight RGBA 선언은 보존합니다. 브라우저 alpha 합성은 Stellar 선형 합성과 달라 최종 픽셀 동일성은 보장하지 않습니다");
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
                result.note_source(
                    &components,
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
                result.note_source(&components, true, "invalid-view-tree", id, e);
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
        let name = components
            .sources
            .get(&id)
            .map(|(instance, node)| format!("{}-{}", element_id(*instance), element_id(*node)))
            .unwrap_or_else(|| element_id(id));
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

/// 가상 자식에만 내보내기 중 사용할 로컬 키를 부여합니다. HTML의 식별자는 실제 인스턴스·정의 node 쌍입니다.
#[derive(Default)]
struct ComponentFrames {
    library: Option<ComponentLibrary>,
    pending: BTreeMap<StableId, (NodeRecord, Vec<StableId>)>,
    sources: BTreeMap<StableId, (StableId, StableId)>,
    expanded_frames: usize,
}

impl ComponentFrames {
    fn expand(
        &mut self,
        instance: &NodeRecord,
        component: crate::core_types::reference::ComponentRef,
        position: Transform2D,
        model: &stellar_document::model::DocumentModel,
    ) -> Result<(NodeRecord, Vec<StableId>), &'static str> {
        let entry = self
            .library
            .as_ref()
            .and_then(|lib| lib.get(component))
            .ok_or("컴포넌트 원본을 찾을 수 없습니다")?;
        let tree = entry.tree();
        // 작은 원본을 수많은 인스턴스가 공유해 HTML이 무제한으로 커지는 경우를 제한합니다.
        const MAX_INSTANCE_FRAMES: usize = 16_384;
        if tree.len() > MAX_INSTANCE_FRAMES - self.expanded_frames {
            return Err("펼친 컴포넌트 Frame 상한 16384개를 초과합니다");
        }
        self.expanded_frames += tree.len();
        let root = entry.definition().root.id();
        let mut ids = BTreeMap::from([(root, instance.id)]);
        for node in tree.iter().filter(|node| node.id != root) {
            // IR의 내부 키만 digest로 축약합니다. 문서 키나 이미 펼친 자식과 겹치면 거부합니다.
            let mut pair = [0u8; 32];
            pair[..16].copy_from_slice(&instance.id.to_bytes());
            pair[16..].copy_from_slice(&node.id.to_bytes());
            let key = StableId::from_bytes(
                ContentHash::compute(&pair).digest()[..16]
                    .try_into()
                    .expect("16 bytes"),
            );
            if model.runtime_id(key).is_some() || self.sources.contains_key(&key) {
                return Err("내보내기 내부 node 키가 중복됩니다");
            }
            self.sources.insert(key, (instance.id, node.id));
            ids.insert(node.id, key);
        }
        for node in tree.iter() {
            let is_root = node.id == root;
            let frame = placed_free_frame(node, is_root.then_some(position))
                .ok_or("고정 Free Frame을 배치할 수 없습니다")?;
            let id = ids[&node.id];
            let record = NodeRecord {
                id,
                parent: node.parent.map(|p| ids[&p]).or(instance.parent),
                order_key: if is_root {
                    instance.order_key.clone()
                } else {
                    node.order_key.clone()
                },
                visible: node.visible && (!is_root || instance.visible),
                locked: instance.locked,
                display_name: String::new(),
                payload: NodePayload::Frame(frame),
            };
            self.pending.insert(
                id,
                (
                    record,
                    tree.children(node.id)
                        .iter()
                        .map(|child| ids[child])
                        .collect(),
                ),
            );
        }
        self.pending
            .remove(&instance.id)
            .ok_or("정의 root를 찾을 수 없습니다")
    }
}

// modifier 평가 규칙은 캔버스와 공유하는 `evaluate_shape_modifiers` 한 곳에 있습니다. 웹 출력은 그 결과 중
// Stroke(path 가운데 정렬 선)를 CSS border로 같게 낼 수 없어 거부합니다.
// Shape는 컨테이너가 아니지만 기존 CSS 값 검증·formatter 입력을 재사용합니다.
fn rectangle_frame(
    shape: &RectangleParams,
    stack: Option<&ModifierStack>,
) -> Result<FramePayload, &'static str> {
    let style = evaluate_shape_modifiers(stack).map_err(|error| match error {
        ShapeModifierError::NonFiniteTransform => "Rectangle affine 합성이 비유한합니다",
        ShapeModifierError::UnsupportedOrder | ShapeModifierError::InvalidStroke => {
            "Rectangle은 활성 Transform 0개 이상 → Fill 최대 1개 → Stroke 최대 1개 순서만 평가합니다"
        }
    })?;
    if style.stroke.is_some() {
        return Err("웹 출력은 Rectangle Stroke를 지원하지 않습니다");
    }
    Ok(FramePayload {
        size: shape.size,
        transform: style.transform,
        background: style.fill,
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
