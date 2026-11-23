//! 문서 → layout 입력 → scene 배치 (ADR-0125, ADR-0086 결정 3 후속, IX.11.9, XII.10.24).
//!
//! [`DocumentLayout`]은 View root subtree의 layout 입력을 문서에서 만들고 [`IncrementalLayout`]으로 계산한 결과를
//! 보관합니다. scene 변환([`super::document_scene_source`], [`super::scene_edits`])은 이 결과 하나로 node 위치·크기를
//! 정합니다. 같은 layout 계산을 다른 곳에서 따로 하지 않습니다.
//!
//! # 입력 규칙
//!
//! - 값: node마다 활성 기본값 세트로 푼 선언 목록(`style_defaults::resolve` → `ResolvedLayout::declarations`)을
//!   `stellar_layout::declared::style_from_declarations`로 옮깁니다(ADR-0086 결정 8). 엔진이 생략값을 채우지 않습니다.
//! - tree: Fragment는 `stellar_layout::view`가 펼칩니다([`build_layout_tree`], [`flatten_children`]). 숨긴 node(문서
//!   `visible`과 View Variant 표시 patch)와 Boolean operand(결과에 흡수됨)는 `display: none`입니다. 옛 `layout_mode`
//!   Frame과 ComponentInstance는 자식을 넣지 않습니다(scene이 그 subtree를 따로 다룸).
//! - 콘텐츠: Text는 텍스트 측정 hook(`SceneSourceOptions::text_measure`)이 있으면 그 hook으로, 없으면 scene Text hook이
//!   낸 줄바꿈 없는 크기(max-content)로 잽니다. Line·Path는 geometry bounds, ComponentInstance는 정의 root Frame
//!   크기입니다. payload 크기 종류(Frame·Rectangle·Ellipse·Polygon·Image)는 명시 `width`·`height`입니다.
//!
//! # 배치 규칙([`PlacementRule`])
//!
//! - **Layout**: 흐름 컨테이너(flex·grid, `commands::arrange::align::flow_parent`) 자식과 layout record에 `position`을
//!   명시한 node. 위치는 layout 결과 border box 원점이고, 저장 transform(Frame transform, 도형 Transform modifier,
//!   인스턴스 배치)은 그 뒤 CSS `translate`·`transform`처럼 합성합니다(`css_node_transform`, origin `0 0`). 크기는
//!   layout 결과 크기입니다.
//! - **Stored**: 그 밖의 자식(옛 자유 배치). IX.11.9 migration 전 위치 원본이 저장 transform이므로 이전처럼 저장
//!   transform·payload 크기로 그립니다. layout 입력에서는 `position: absolute; left: 0; top: 0`으로 두어 부모 흐름에
//!   참여하지 않게 합니다. migration이 그 node에 `position: absolute; left; top`과 선형 `transform`을 쓰면 Layout
//!   규칙으로 같은 화면 위치에 그려집니다.
//! - **Root**: View root는 캔버스 배치(저장 transform)와 payload 크기입니다.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

pub use stellar_layout as engine;

use stellar_commands::arrange::align::{flow_parent, model_style_set, FlowParent};
use stellar_commands::change::{node_keys, ChangeSet};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Length, Size2, Transform2D};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::instance::placed_free_frame;
use stellar_document::css::style_defaults::{resolve, FlatSet};
use stellar_document::layout_record::{payload_size, LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;
use stellar_layout::declared::style_from_declarations;
use stellar_layout::error::LayoutError;
use stellar_layout::incremental::{IncrementalLayout, LayoutEdit, UpdateStats};
use stellar_layout::input::{IntrinsicContent, LayoutChild, LayoutContent, LayoutNode};
use stellar_layout::intrinsic::{
    ContentMeasurer, DefaultMeasurer, MeasuredContent, TextHookMeasurer, TextIntrinsic,
    TextLayoutHook,
};
use stellar_layout::output::LayoutBox;
use stellar_layout::style::{Display, LayoutStyle, LengthPercentageAuto, Position};
use stellar_layout::transform::css_node_transform;
use stellar_layout::view::{
    build_layout_tree, flatten_children, view_order_keys, LayoutSource, SourceNode,
};

/// node 하나의 위치·크기 원천(모듈 문서).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementRule {
    /// View root: 캔버스 배치(저장 transform)와 payload 크기.
    Root,
    /// 옛 자유 배치: 저장 transform과 payload 크기.
    Stored,
    /// layout 결과 위치·크기.
    Layout,
}

/// `id`의 배치 규칙. `set`은 활성 기본값 세트입니다.
pub fn placement_rule(
    model: &DocumentModel,
    set: &FlatSet,
    root: StableId,
    id: StableId,
) -> PlacementRule {
    if id == root {
        return PlacementRule::Root;
    }
    if model
        .layout_record(id)
        .is_some_and(|record| record.get(LayoutLonghand::Position).is_some())
    {
        return PlacementRule::Layout;
    }
    match flow_parent(model, set, id) {
        FlowParent::Flex { .. } | FlowParent::Grid => PlacementRule::Layout,
        FlowParent::Free | FlowParent::Legacy => PlacementRule::Stored,
    }
}

/// layout 결과 border box 원점과 저장 local transform을 CSS 순서로 합성합니다(IX.11.9 `css_node_transform`, origin
/// `0 0`). `transform-origin` longhand는 아직 저장하지 않아 언제나 `0 0`입니다.
pub fn placed_transform(layout_box: &LayoutBox, stored: &Transform2D) -> Option<Transform2D> {
    let rect = layout_box.local_rect();
    css_node_transform(
        (rect.x().to_px_f32().get(), rect.y().to_px_f32().get()),
        (0.0, 0.0),
        stored,
    )
    .ok()
}

/// layout 결과 border box 크기.
pub fn box_size(layout_box: &LayoutBox) -> Option<Size2> {
    let rect = layout_box.local_rect();
    Size2::new(
        rect.width().to_px_f32().get(),
        rect.height().to_px_f32().get(),
    )
    .ok()
}

/// layout 입력을 만들 때 쓰는 scene 변환 값.
pub struct LayoutInputs<'a> {
    /// View Variant 표시 patch(node → 표시 여부).
    pub visibility: &'a HashMap<StableId, bool>,
    /// Boolean operand(결과에 흡수되어 상자가 없음).
    pub operands: &'a HashSet<StableId>,
    /// ComponentInstance 정의 집합(인스턴스 크기).
    pub library: Option<&'a ComponentLibrary>,
    /// 텍스트 측정 hook(줄바꿈 측정). 없으면 [`TextSizes`]의 max-content 크기를 씁니다.
    pub text_measure: Option<&'a dyn TextLayoutHook>,
}

/// Text node의 줄바꿈 없는(max-content) 크기를 재는 함수(scene Text hook). 잴 수 없으면 `None`입니다.
pub type TextSizes<'m> = dyn FnMut(StableId) -> Option<Size2> + 'm;

/// layout 원천 읽기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutSourceError {
    /// node.
    pub node: StableId,
    /// 이유.
    pub reason: String,
}

impl fmt::Display for LayoutSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node {}: {}", self.node, self.reason)
    }
}

/// 문서 View root subtree의 layout 원천(`stellar_layout::view::LayoutSource`).
struct DocumentSource<'a> {
    model: &'a DocumentModel,
    root: StableId,
    set: &'a FlatSet,
    inputs: &'a LayoutInputs<'a>,
    texts: &'a HashMap<StableId, Size2>,
    /// payload 크기·종류·배치 규칙이 같고 record가 없는 node의 style(대부분의 node).
    styles: RefCell<HashMap<(NodeKind, u32, u32, bool), LayoutStyle>>,
}

impl DocumentSource<'_> {
    fn error(node: StableId, reason: impl Into<String>) -> LayoutSourceError {
        LayoutSourceError {
            node,
            reason: reason.into(),
        }
    }

    fn visible(&self, id: StableId) -> Option<bool> {
        let fields = self
            .model
            .runtime_id(id)
            .and_then(|r| self.model.columns().get(r))?;
        Some(
            self.inputs
                .visibility
                .get(&id)
                .copied()
                .unwrap_or(fields.visible),
        )
    }

    fn payload(&self, id: StableId) -> Result<&NodePayload, LayoutSourceError> {
        self.model
            .payload(id)
            .ok_or_else(|| Self::error(id, "payload가 없습니다"))
    }

    /// 활성 세트로 푼 style에 배치 규칙을 더합니다.
    fn style(&self, id: StableId, payload: &NodePayload) -> Result<LayoutStyle, LayoutSourceError> {
        let record = self.model.layout_record(id);
        let stored = placement_rule(self.model, self.set, self.root, id) == PlacementRule::Stored;
        let size = payload_size(payload).unwrap_or((0.0, 0.0));
        let key = (payload.kind(), size.0.to_bits(), size.1.to_bits(), stored);
        if record.is_none() {
            if let Some(style) = self.styles.borrow().get(&key) {
                return Ok(style.clone());
            }
        }
        let resolved = resolve(self.set, payload, record)
            .ok_or_else(|| Self::error(id, "상자가 없는 node 종류입니다"))?;
        let declarations = resolved.declarations();
        let pairs: Vec<(&str, &str)> = declarations
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let mut style =
            style_from_declarations(&pairs).map_err(|e| Self::error(id, e.to_string()))?;
        if stored {
            // 옛 자유 배치 자식: 위치 원본은 저장 transform이므로 부모 흐름에 참여하지 않는 상자로 둡니다.
            style.position = Position::Absolute;
            let zero = LengthPercentageAuto::Length(Length::new(0.0).expect("0은 유한"));
            style.left = zero;
            style.top = zero;
            style.right = LengthPercentageAuto::Auto;
            style.bottom = LengthPercentageAuto::Auto;
        }
        if record.is_none() {
            self.styles.borrow_mut().insert(key, style.clone());
        }
        Ok(style)
    }

    fn content(&self, id: StableId, payload: &NodePayload) -> LayoutContent {
        let intrinsic = |size: Size2| {
            Some(LayoutContent::Intrinsic(IntrinsicContent {
                width: Length::new(size.width()).ok()?,
                height: Length::new(size.height()).ok()?,
                baseline: None,
            }))
        };
        let content = match payload {
            NodePayload::Text(_) => {
                if self.inputs.text_measure.is_some() {
                    let zero = Length::new(0.0).expect("0은 유한");
                    Some(LayoutContent::Measured(MeasuredContent::Text(
                        TextIntrinsic {
                            words: Vec::new(),
                            space: zero,
                            line_height: zero,
                            ascent: zero,
                        },
                    )))
                } else {
                    self.texts.get(&id).and_then(|size| intrinsic(*size))
                }
            }
            NodePayload::Shape(ShapeGeometry::Line(line)) => Size2::new(
                (line.end.x() - line.start.x()).abs(),
                (line.end.y() - line.start.y()).abs(),
            )
            .ok()
            .and_then(intrinsic),
            NodePayload::Path(path) => {
                let commands = path
                    .data()
                    .commands()
                    .iter()
                    .copied()
                    .map(super::inputs::scene_command)
                    .collect();
                crate::geometry::PathGeometry::new(commands)
                    .ok()
                    .and_then(|geometry| crate::bounds::path_control_bounds(&geometry))
                    .and_then(|aabb| Size2::new(aabb.width(), aabb.height()).ok())
                    .and_then(intrinsic)
            }
            NodePayload::ComponentInstance(instance) => self
                .inputs
                .library
                .and_then(|library| library.get(instance.component()))
                .and_then(|entry| {
                    let tree = entry.tree();
                    tree.node(tree.tree_root()?)
                })
                .and_then(|root| placed_free_frame(root, None))
                .and_then(|frame| intrinsic(frame.size)),
            _ => None,
        };
        content.unwrap_or_default()
    }

    /// box node 하나(자식 목록 없음, view가 채움).
    fn box_node(&self, id: StableId) -> Result<LayoutNode, LayoutSourceError> {
        let payload = self.payload(id)?;
        let mut style = self.style(id, payload)?;
        let hidden = !self
            .visible(id)
            .ok_or_else(|| Self::error(id, "node가 없습니다"))?;
        if id != self.root && (hidden || self.inputs.operands.contains(&id)) {
            style.display = Display::None;
        }
        Ok(LayoutNode::new(id)
            .with_style(style)
            .with_content(self.content(id, payload)))
    }
}

impl LayoutSource for DocumentSource<'_> {
    type Key = StableId;
    type Error = LayoutSourceError;

    fn root(&self) -> StableId {
        self.root
    }

    fn node(&self, key: &StableId) -> Result<SourceNode, LayoutSourceError> {
        if let Some(visible) = self.fragment(key)? {
            return Ok(SourceNode::Fragment { visible });
        }
        Ok(SourceNode::Node(Box::new(self.box_node(*key)?)))
    }

    fn children(&self, key: &StableId) -> Result<Vec<StableId>, LayoutSourceError> {
        Ok(match self.payload(*key)? {
            NodePayload::Frame(frame) if frame.layout_mode.is_none() => {
                self.model.children_in_order(*key)
            }
            NodePayload::Fragment(_) => self.model.children_in_order(*key),
            _ => Vec::new(),
        })
    }

    fn layout_id(&self, key: &StableId) -> StableId {
        *key
    }

    fn fragment(&self, key: &StableId) -> Result<Option<bool>, LayoutSourceError> {
        Ok(match self.payload(*key)? {
            NodePayload::Fragment(_) => Some(self.visible(*key).unwrap_or(true)),
            _ => None,
        })
    }
}

/// View root subtree의 layout 상태와 결과(모듈 문서).
#[derive(Debug, Clone)]
pub struct DocumentLayout {
    root: StableId,
    set: Option<FlatSet>,
    set_name: String,
    measured_text: bool,
    session: Option<IncrementalLayout>,
    error: Option<String>,
    boxes: HashMap<StableId, LayoutBox>,
    /// layout 자식 → layout 부모(Fragment를 펼친 box 부모).
    parent: HashMap<StableId, StableId>,
    /// 숨긴 Fragment 아래라 `display: none`인 box.
    fragment_hidden: HashSet<StableId>,
    /// 입력에 쓴 View Variant 표시 patch.
    visibility: HashMap<StableId, bool>,
    last: UpdateStats,
}

impl PartialEq for DocumentLayout {
    fn eq(&self, other: &DocumentLayout) -> bool {
        self.root == other.root
            && self.set_name == other.set_name
            && self.error == other.error
            && self.boxes == other.boxes
    }
}

impl Default for DocumentLayout {
    /// 결과가 없는 빈 상태(`mem::take` 자리 표시).
    fn default() -> DocumentLayout {
        DocumentLayout {
            root: StableId::NIL,
            set: None,
            set_name: String::new(),
            measured_text: false,
            session: None,
            error: None,
            boxes: HashMap::new(),
            parent: HashMap::new(),
            fragment_hidden: HashSet::new(),
            visibility: HashMap::new(),
            last: UpdateStats::default(),
        }
    }
}

fn measurer<'h>(hook: Option<&'h dyn TextLayoutHook>) -> Box<dyn ContentMeasurer + 'h> {
    match hook {
        Some(hook) => Box::new(TextHookMeasurer(hook)),
        None => Box::new(DefaultMeasurer),
    }
}

/// `id`가 View root subtree에 있는지.
fn in_subtree(model: &DocumentModel, root: StableId, id: StableId) -> bool {
    let mut current = Some(id);
    while let Some(node) = current {
        if node == root {
            return true;
        }
        current = parent_of(model, node);
    }
    false
}

fn parent_of(model: &DocumentModel, id: StableId) -> Option<StableId> {
    let fields = model.runtime_id(id).and_then(|r| model.columns().get(r))?;
    let parent = fields.parent?;
    model.columns().get(parent).map(|p| p.stable_id)
}

/// Fragment를 건너뛴 첫 box 조상.
fn box_host(model: &DocumentModel, id: StableId) -> Option<StableId> {
    let mut current = parent_of(model, id);
    while let Some(node) = current {
        if !matches!(model.payload(node), Some(NodePayload::Fragment(_))) {
            return Some(node);
        }
        current = parent_of(model, node);
    }
    None
}

/// container longhand record key(`0x0110`–`0x0121`, ADR-0086 결정 2): 바뀌면 자식 배치 규칙이 바뀔 수 있습니다.
fn is_container_key(key: u32) -> bool {
    (0x0110..=0x0121).contains(&key)
}

impl DocumentLayout {
    /// View root subtree 전체를 처음부터 계산합니다. 입력을 만들지 못하거나 계산이 실패하면 오류를 남기고 결과가
    /// 없는 상태입니다(Layout 규칙 node는 scene에서 빠짐).
    pub fn build(
        model: &DocumentModel,
        root: StableId,
        inputs: &LayoutInputs<'_>,
        texts: &mut TextSizes<'_>,
    ) -> DocumentLayout {
        let set = model_style_set(model);
        let mut layout = DocumentLayout {
            root,
            set: Some(set),
            set_name: model.style_defaults().to_string(),
            measured_text: inputs.text_measure.is_some(),
            visibility: inputs.visibility.clone(),
            ..DocumentLayout::default()
        };
        let sizes = if inputs.text_measure.is_some() {
            HashMap::new()
        } else {
            let mut sizes = HashMap::new();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                if matches!(model.payload(id), Some(NodePayload::Text(_))) {
                    if let Some(size) = texts(id) {
                        sizes.insert(id, size);
                    }
                }
                stack.extend(model.children_in_order(id));
            }
            sizes
        };
        let set = layout.set.as_ref().expect("방금 넣음");
        let source = DocumentSource {
            model,
            root,
            set,
            inputs,
            texts: &sizes,
            styles: RefCell::new(HashMap::new()),
        };
        let view = match build_layout_tree(&source) {
            Ok(view) => view,
            Err(error) => {
                layout.error = Some(error.to_string());
                return layout;
            }
        };
        let input = view.into_input();
        for node in &input.nodes {
            for child in &node.children {
                layout.parent.insert(child.id, node.id);
            }
            if node.style.display == Display::None
                && node.id != root
                && source.visible(node.id) == Some(true)
                && !inputs.operands.contains(&node.id)
            {
                layout.fragment_hidden.insert(node.id);
            }
        }
        let ids: Vec<StableId> = input.nodes.iter().map(|n| n.id).collect();
        let measure = measurer(inputs.text_measure);
        match IncrementalLayout::new(input, measure.as_ref()) {
            Ok((session, stats)) => {
                for id in ids {
                    if let Some(layout_box) = session.layout_box(id) {
                        layout.boxes.insert(id, layout_box);
                    }
                }
                layout.last = stats;
                layout.session = Some(session);
            }
            Err(error) => layout.error = Some(error.to_string()),
        }
        layout
    }

    /// View root.
    pub const fn root(&self) -> StableId {
        self.root
    }

    /// 입력을 만들거나 계산하지 못한 이유. 결과가 있으면 `None`입니다.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 계산 결과가 있는지.
    pub fn is_available(&self) -> bool {
        self.session.is_some()
    }

    /// 활성 기본값 세트.
    pub fn style_set(&self) -> Option<&FlatSet> {
        self.set.as_ref()
    }

    /// 표시되는 node의 layout 결과 상자(부모 border box 좌표).
    pub fn layout_box(&self, id: StableId) -> Option<LayoutBox> {
        self.boxes.get(&id).copied()
    }

    /// 마지막 계산의 집계.
    pub const fn last_stats(&self) -> UpdateStats {
        self.last
    }

    /// 증분 layout 상태(진단·preview용).
    pub fn session(&self) -> Option<&IncrementalLayout> {
        self.session.as_ref()
    }

    /// `id`의 배치 규칙.
    pub fn rule(&self, model: &DocumentModel, id: StableId) -> PlacementRule {
        match &self.set {
            Some(set) => placement_rule(model, set, self.root, id),
            None => PlacementRule::Stored,
        }
    }

    /// Layout 규칙 node의 (부모 좌표 transform, 크기, content box 너비). `stored`는 저장 local transform입니다.
    ///
    /// - `Ok(None)`: 저장 값으로 그립니다(Root·Stored 규칙, 또는 숨겨 상자가 없는 node).
    /// - `Err(())`: Layout 규칙인데 결과가 없습니다(입력·계산 실패). scene은 그 node를 넣지 않습니다.
    #[allow(clippy::result_unit_err)]
    pub fn placement(
        &self,
        model: &DocumentModel,
        id: StableId,
        stored: &Transform2D,
    ) -> Result<Option<Placed>, ()> {
        if self.rule(model, id) != PlacementRule::Layout {
            return Ok(None);
        }
        if self.session.is_none() {
            return Err(());
        }
        let Some(layout_box) = self.boxes.get(&id) else {
            return Ok(None);
        };
        let transform = placed_transform(layout_box, stored).ok_or(())?;
        let size = box_size(layout_box).ok_or(())?;
        let content = layout_box.content_rect();
        Ok(Some(Placed {
            transform,
            size,
            content_width: content.width().to_px_f32().get(),
        }))
    }

    /// Text를 그릴 줄 너비. 풀린 `width`가 `max-content`면 줄바꿈 없음(`None`), 그 밖에는 layout content box 너비
    /// (`<length>`는 그 px, 흐름이 줄이거나 늘린 값 포함)입니다.
    pub fn text_width(&self, model: &DocumentModel, id: StableId, placed: &Placed) -> Option<f32> {
        let set = self.set.as_ref()?;
        let resolved = resolve(set, model.payload(id)?, model.layout_record(id))?;
        match resolved.get(LayoutLonghand::Width).value {
            LayoutValue::Keyword("max-content") => None,
            _ => Some(placed.content_width),
        }
    }

    /// change set을 반영합니다. 결과 상자나 배치 입력이 바뀐 node(삽입·삭제 포함)를 돌려줍니다. 이 node는 scene에서
    /// 다시 변환해야 합니다.
    pub fn update(
        &mut self,
        model: &DocumentModel,
        changes: &ChangeSet,
        inputs: &LayoutInputs<'_>,
        texts: &mut TextSizes<'_>,
    ) -> BTreeSet<StableId> {
        let rebuild = self.session.is_none()
            || self.set_name != model.style_defaults()
            || self.measured_text != inputs.text_measure.is_some();
        if rebuild {
            return self.replace(DocumentLayout::build(model, self.root, inputs, texts));
        }
        match self.update_incremental(model, changes, inputs, texts) {
            Ok(moved) => moved,
            Err(_) => self.replace(DocumentLayout::build(model, self.root, inputs, texts)),
        }
    }

    /// 전체 다시 계산한 결과로 바꾸고 상자가 달라진 node를 돌려줍니다.
    fn replace(&mut self, next: DocumentLayout) -> BTreeSet<StableId> {
        let mut moved: BTreeSet<StableId> = BTreeSet::new();
        for (id, old) in &self.boxes {
            if !same_place(next.boxes.get(id), Some(old)) {
                moved.insert(*id);
            }
        }
        for id in next.boxes.keys() {
            if !self.boxes.contains_key(id) {
                moved.insert(*id);
            }
        }
        *self = next;
        moved
    }

    fn update_incremental(
        &mut self,
        model: &DocumentModel,
        changes: &ChangeSet,
        inputs: &LayoutInputs<'_>,
        texts: &mut TextSizes<'_>,
    ) -> Result<BTreeSet<StableId>, LayoutError> {
        let root = self.root;
        let removed: BTreeSet<StableId> = changes.removed_nodes().into_iter().collect();
        let mut dirty: BTreeSet<StableId> = changes.changed_nodes().into_iter().collect();
        dirty.extend(changes.inserted_nodes());
        // View Variant 표시 patch가 바뀐 node.
        for (id, visible) in inputs.visibility {
            if self.visibility.get(id) != Some(visible) {
                dirty.insert(*id);
            }
        }
        for id in self.visibility.keys() {
            if !inputs.visibility.contains_key(id) {
                dirty.insert(*id);
            }
        }
        let paths = changes.changed_properties();
        let mut keys_of: HashMap<StableId, Vec<u32>> = HashMap::new();
        for path in &paths {
            if let [key] = path.keys() {
                keys_of.entry(path.root()).or_default().push(key.0);
            }
        }
        let session = self
            .session
            .as_ref()
            .ok_or(LayoutError::LayoutUnavailable)?;
        // 자식 목록을 다시 펼칠 box, 다시 만들 box.
        let mut hosts: BTreeSet<StableId> = BTreeSet::new();
        let mut build: BTreeSet<StableId> = BTreeSet::new();
        for id in &dirty {
            let id = *id;
            if removed.contains(&id) || !in_subtree(model, root, id) {
                if let Some(parent) = self.parent.get(&id) {
                    hosts.insert(*parent);
                }
                continue;
            }
            let Some(payload) = model.payload(id) else {
                continue;
            };
            let host = box_host(model, id);
            if matches!(payload, NodePayload::Fragment(_)) {
                hosts.extend(host);
                continue;
            }
            build.insert(id);
            let keys = keys_of.get(&id);
            // 경로 없는 구조 변경(종류 변환 등), 부모·순서·표시 변경, 처음 보는 node는 부모 자식 목록을 다시 펼칩니다.
            let structural = keys.is_none_or(|keys| {
                keys.iter().any(|k| {
                    [
                        node_keys::PARENT.0,
                        node_keys::ORDER_KEY.0,
                        node_keys::VISIBLE.0,
                    ]
                    .contains(k)
                })
            }) || self.parent.get(&id).copied() != host;
            if structural {
                hosts.extend(host);
                if let Some(old) = self.parent.get(&id) {
                    hosts.insert(*old);
                }
                hosts.insert(id);
            }
            // container longhand·구조 변경은 자식 배치 규칙(흐름 여부)을 바꿀 수 있습니다.
            let container = keys.is_none_or(|keys| keys.iter().any(|k| is_container_key(*k)));
            if container {
                if let Some(node) = session.node(id) {
                    build.extend(node.children.iter().map(|c| c.id));
                }
                hosts.insert(id);
            }
        }
        for id in &removed {
            if let Some(parent) = self.parent.get(id) {
                hosts.insert(*parent);
            }
        }
        hosts.retain(|h| model.payload(*h).is_some() && in_subtree(model, root, *h));
        let set = self.set.clone().ok_or(LayoutError::LayoutUnavailable)?;
        let mut sizes: HashMap<StableId, Size2> = HashMap::new();
        let empty_sizes: HashMap<StableId, Size2> = HashMap::new();
        // 펼친 자식 목록과 표시 여부.
        let mut flattened: HashMap<StableId, Vec<(StableId, bool)>> = HashMap::new();
        {
            let source = DocumentSource {
                model,
                root,
                set: &set,
                inputs,
                texts: &empty_sizes,
                styles: RefCell::new(HashMap::new()),
            };
            let mut work: Vec<StableId> = hosts.iter().copied().collect();
            while let Some(host) = work.pop() {
                if flattened.contains_key(&host) {
                    continue;
                }
                let mut fragments = Vec::new();
                let flat = flatten_children(&source, &host, &mut fragments)
                    .map_err(|_| LayoutError::LayoutUnavailable)?;
                for (child, shown) in &flat {
                    let known = session.node(*child).is_some();
                    let was_shown = !self.fragment_hidden.contains(child);
                    if !known || was_shown != *shown {
                        build.insert(*child);
                    }
                    // 새로 들어온 subtree는 자식도 펼쳐 만듭니다.
                    if !known {
                        work.push(*child);
                    }
                }
                flattened.insert(host, flat);
            }
        }
        // 다시 만들 node 중 처음 보는 box의 자식도 펼칩니다(삽입한 subtree 안쪽).
        let mut pending: Vec<StableId> = build
            .iter()
            .copied()
            .filter(|id| session.node(*id).is_none() && !flattened.contains_key(id))
            .collect();
        while let Some(id) = pending.pop() {
            if flattened.contains_key(&id) {
                continue;
            }
            let source = DocumentSource {
                model,
                root,
                set: &set,
                inputs,
                texts: &empty_sizes,
                styles: RefCell::new(HashMap::new()),
            };
            let mut fragments = Vec::new();
            let flat = flatten_children(&source, &id, &mut fragments)
                .map_err(|_| LayoutError::LayoutUnavailable)?;
            for (child, _) in &flat {
                build.insert(*child);
                if session.node(*child).is_none() {
                    pending.push(*child);
                }
            }
            flattened.insert(id, flat);
        }
        build.retain(|id| model.payload(*id).is_some() && in_subtree(model, root, *id));
        if inputs.text_measure.is_none() {
            for id in &build {
                if matches!(model.payload(*id), Some(NodePayload::Text(_))) {
                    if let Some(size) = texts(*id) {
                        sizes.insert(*id, size);
                    }
                }
            }
        }
        let source = DocumentSource {
            model,
            root,
            set: &set,
            inputs,
            texts: &sizes,
            styles: RefCell::new(HashMap::new()),
        };
        // 각 child의 Fragment 표시 여부(펼친 목록에서).
        let mut shown_of: HashMap<StableId, bool> = HashMap::new();
        for flat in flattened.values() {
            for (child, shown) in flat {
                shown_of.insert(*child, *shown);
            }
        }
        let mut edits = Vec::new();
        let mut built_nodes: HashMap<StableId, LayoutNode> = HashMap::new();
        let mut targets: BTreeSet<StableId> = build.clone();
        targets.extend(flattened.keys().copied());
        for id in &targets {
            let mut node = if build.contains(id) {
                let mut node = source
                    .box_node(*id)
                    .map_err(|_| LayoutError::LayoutUnavailable)?;
                let shown = shown_of
                    .get(id)
                    .copied()
                    .unwrap_or_else(|| !self.fragment_hidden.contains(id));
                if !shown && *id != root {
                    node.style.display = Display::None;
                }
                node
            } else {
                session
                    .node(*id)
                    .cloned()
                    .ok_or(LayoutError::LayoutUnavailable)?
            };
            node.children = match flattened.get(id) {
                Some(flat) => flat
                    .iter()
                    .zip(view_order_keys(flat.len()))
                    .map(|((child, _), order_key)| LayoutChild {
                        id: *child,
                        order_key,
                    })
                    .collect(),
                None => session
                    .node(*id)
                    .map(|n| n.children.clone())
                    .unwrap_or_default(),
            };
            if session.node(*id) != Some(&node) {
                built_nodes.insert(*id, node.clone());
                edits.push(LayoutEdit::upsert(node));
            }
        }
        // View subtree를 벗어난 node와 그 이전 자손을 지웁니다(다른 box로 옮긴 node는 새 부모 자식 목록에 있음).
        let mut gone: Vec<StableId> = Vec::new();
        for (host, flat) in &flattened {
            let now: HashSet<StableId> = flat.iter().map(|c| c.0).collect();
            if let Some(old) = session.node(*host) {
                for child in &old.children {
                    if !now.contains(&child.id) && !in_subtree(model, root, child.id) {
                        gone.push(child.id);
                    }
                }
            }
        }
        for id in &removed {
            if session.node(*id).is_some() && !in_subtree(model, root, *id) {
                gone.push(*id);
            }
        }
        let mut removed_layout: BTreeSet<StableId> = BTreeSet::new();
        while let Some(id) = gone.pop() {
            if !removed_layout.insert(id) {
                continue;
            }
            if let Some(node) = session.node(id) {
                gone.extend(node.children.iter().map(|c| c.id));
            }
        }
        for id in &removed_layout {
            edits.push(LayoutEdit::Remove(*id));
        }
        if edits.is_empty() {
            self.visibility = inputs.visibility.clone();
            return Ok(BTreeSet::new());
        }
        // 표시 여부가 바뀐 box는 자손 상자도 확인합니다(숨김 subtree는 상자가 없음).
        let mut display_changed: Vec<StableId> = Vec::new();
        for (id, node) in &built_nodes {
            let before = session.node(*id).map(|n| n.style.display == Display::None);
            if before != Some(node.style.display == Display::None) {
                display_changed.push(*id);
            }
        }
        let measure = measurer(inputs.text_measure);
        let session = self
            .session
            .as_mut()
            .ok_or(LayoutError::LayoutUnavailable)?;
        let stats = session.apply(edits, measure.as_ref())?;
        self.last = stats;
        // 부모 표·Fragment 숨김 표 갱신.
        for id in &removed_layout {
            self.parent.remove(id);
            self.fragment_hidden.remove(id);
        }
        for (host, flat) in &flattened {
            for (child, shown) in flat {
                self.parent.insert(*child, *host);
                if *shown {
                    self.fragment_hidden.remove(child);
                } else {
                    self.fragment_hidden.insert(*child);
                }
            }
        }
        self.visibility = inputs.visibility.clone();
        // 상자가 바뀌었을 수 있는 node: 다시 배치한 node와 그 자식, 다시 만든 node, 지운 node.
        let mut check: BTreeSet<StableId> = BTreeSet::new();
        for id in session.last_arranged() {
            check.insert(*id);
            if let Some(node) = session.node(*id) {
                check.extend(node.children.iter().map(|c| c.id));
            }
        }
        check.extend(built_nodes.keys().copied());
        check.extend(removed_layout.iter().copied());
        let mut stack = display_changed;
        while let Some(id) = stack.pop() {
            check.insert(id);
            if let Some(node) = session.node(id) {
                stack.extend(node.children.iter().map(|c| c.id));
            }
        }
        let mut moved: BTreeSet<StableId> = build;
        for id in check {
            let now = session.layout_box(id);
            let before = self.boxes.get(&id).copied();
            if now != before {
                // baseline·overflow만 바뀐 상자는 그리기와 무관하므로 다시 변환하지 않습니다.
                if !same_place(now.as_ref(), before.as_ref()) {
                    moved.insert(id);
                }
                match now {
                    Some(layout_box) => {
                        self.boxes.insert(id, layout_box);
                    }
                    None => {
                        self.boxes.remove(&id);
                    }
                }
            }
        }
        Ok(moved)
    }

    /// 흐름 컨테이너 `container`(box)의 자식 순서를 `order`(box 자식 id, 새 흐름 순서)로 바꾼 preview 결과.
    /// 문서와 이 상태는 바꾸지 않습니다. 상자가 바뀐 node와 새 상자를 돌려줍니다.
    pub fn reorder_preview(
        &self,
        container: StableId,
        order: &[StableId],
        text_measure: Option<&dyn TextLayoutHook>,
    ) -> Result<DocumentLayout, LayoutError> {
        let session = self
            .session
            .as_ref()
            .ok_or(LayoutError::LayoutUnavailable)?;
        let mut node = session
            .node(container)
            .cloned()
            .ok_or(LayoutError::UnknownNode { node: container })?;
        let current: HashSet<StableId> = node.children.iter().map(|c| c.id).collect();
        let next: HashSet<StableId> = order.iter().copied().collect();
        if current != next || order.len() != node.children.len() {
            return Err(LayoutError::UnknownNode { node: container });
        }
        node.children = order
            .iter()
            .zip(view_order_keys(order.len()))
            .map(|(id, order_key)| LayoutChild { id: *id, order_key })
            .collect();
        let mut preview = self.clone();
        let measure = measurer(text_measure);
        let session = preview
            .session
            .as_mut()
            .ok_or(LayoutError::LayoutUnavailable)?;
        session.apply(vec![LayoutEdit::upsert(node)], measure.as_ref())?;
        let mut check: Vec<StableId> = Vec::new();
        for id in session.last_arranged() {
            check.push(*id);
            if let Some(n) = session.node(*id) {
                check.extend(n.children.iter().map(|c| c.id));
            }
        }
        for id in check {
            match session.layout_box(id) {
                Some(layout_box) => {
                    preview.boxes.insert(id, layout_box);
                }
                None => {
                    preview.boxes.remove(&id);
                }
            }
        }
        Ok(preview)
    }

    /// 두 결과에서 상자가 다른 node.
    pub fn changed_boxes(&self, other: &DocumentLayout) -> BTreeSet<StableId> {
        let mut out = BTreeSet::new();
        for (id, b) in &self.boxes {
            if !same_place(other.boxes.get(id), Some(b)) {
                out.insert(*id);
            }
        }
        for id in other.boxes.keys() {
            if !self.boxes.contains_key(id) {
                out.insert(*id);
            }
        }
        out
    }

    /// layout 자식 목록(흐름 순서, Fragment를 펼친 box). 없으면 빈 목록.
    pub fn layout_children(&self, id: StableId) -> Vec<StableId> {
        self.session
            .as_ref()
            .and_then(|s| s.node(id))
            .map(|n| n.children.iter().map(|c| c.id).collect())
            .unwrap_or_default()
    }

    /// layout 부모(Fragment를 건너뛴 box).
    pub fn layout_parent(&self, id: StableId) -> Option<StableId> {
        self.parent.get(&id).copied()
    }
}

/// 두 상자의 그리는 위치·크기(border box, content box)가 같은지. baseline·overflow는 보지 않습니다.
fn same_place(a: Option<&LayoutBox>, b: Option<&LayoutBox>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.local_rect() == b.local_rect() && a.content_rect() == b.content_rect()
        }
        (None, None) => true,
        _ => false,
    }
}

/// Layout 규칙 node의 배치 결과.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// 부모 좌표 local transform(layout 위치 ∘ 저장 transform).
    pub transform: Transform2D,
    /// border box 크기.
    pub size: Size2,
    /// content box 너비(Text 줄 너비).
    pub content_width: f32,
}

#[cfg(test)]
mod tests;
