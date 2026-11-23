//! command invalidation → layout dirty set (X.7.1).
//!
//! command 실행 결과(VII.1.4 change set: 바뀐 node·속성·삽입·삭제·무효화 class)를 레이아웃이 다시 계산할
//! node 집합 [`LayoutDirtySet`]으로 바꿉니다. 레이아웃은 문서·command crate에 의존하지 않으므로(core-types만
//! 의존) change set과 속성 registry는 trait([`LayoutChangeSource`], [`LayoutPropertyMap`])으로 받습니다.
//!
//! # 변환 규칙
//!
//! 1. **속성 변경** `(node, key)`: [`LayoutPropertyMap::effect`]로 효과를 찾습니다.
//!    - `Layout { node, parent }`: node 속성은 그 node에, parent 속성(형제 순서·부모 변경 등)은 변경 **전후**
//!      부모 각각에 [`node_dirty_classes`]로 class를 계산해 더합니다. class는 node의 변경 전·후 상태 양쪽으로
//!      구한 합집합입니다(X.1.3: 콘텐츠 크기 node의 DescendantConstraint는 SelfSize 포함).
//!    - `PaintOnly`: 렌더 dirty 집합에만 넣습니다.
//!    - `None`(잠금·이름 등 layout·render 입력이 아님): 무시합니다.
//!    - 알 수 없는 key(registry에 없음): 그 node의 무효화 class로 보수적으로 처리합니다(아래 4·5).
//! 2. **삽입**: 새 node에 SelfSize·ParentFlow·DescendantConstraint를 모두, 변경 후 부모에 자식 변경(Children)을
//!    표시합니다.
//! 3. **삭제**: 변경 전 부모에 자식 변경을 표시합니다(삭제된 node 자신은 레이아웃 대상이 아님).
//! 4. **node 무효화**: `layout` class가 있는데 위 1~3의 layout 속성·자식 구조 변경으로 설명되지 않는 node(종류
//!    변환, 알 수 없는 key)는 세 class를 모두 표시합니다. `layout` 없이 `paint`·`geometry`만 있으면 렌더
//!    dirty입니다(무효화 class는 문서 속성 metadata registry가 정한 값입니다, VI.9.7). 같은 command에서 종류
//!    변환과 layout 속성 변경이 함께 기록된 node는 기록된 속성의 class만 얻지만 layout queue에는 들어가므로, 다시
//!    읽은 입력의 차이(X.7.3, [`changed_properties`])가 정확한 class를 정합니다.
//! 5. **무효화가 보고되지 않은 알 수 없는 key**: [`LayoutChangeSource::changed_nodes`]에 없는 node의 알 수 없는
//!    key는 판단 근거가 없으므로 레이아웃 node면 세 class를 모두, 아니면 렌더 dirty로 표시합니다(버리지 않음).
//!
//! **PaintOnly는 layout queue에 넣지 않습니다**: [`LayoutDirtySet::layout_queue`]는 레이아웃에 영향을 주는
//! class가 있는 node만 돌려줍니다. PaintOnly class(예: transform)는 [`LayoutDirtySet::paint`]에만 남습니다.
//!
//! 증분 레이아웃(X.7.3)은 node 단위 입력 edit의 차이를 [`changed_properties`]로 직접 구해 같은 규칙의 class를
//! 씁니다. 이 모듈의 변환은 command 결과에서 "어느 node를 다시 읽어야 하는가"(layout queue)를 정합니다.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;

use crate::input::{LayoutInput, LayoutNode};
use crate::property::{
    dirty_class, dirty_classes, DirtyClass, DirtyClassSet, LayoutProperty, LAYOUT_PROPERTIES,
};
use crate::style::LayoutStyle;

/// [`LayoutProperty`] 집합 (X.7.1). 순회 순서는 registry tag 순입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayoutPropertySet(u64);

impl LayoutPropertySet {
    /// 빈 집합.
    pub const EMPTY: LayoutPropertySet = LayoutPropertySet(0);

    const fn bit(property: LayoutProperty) -> u64 {
        // registry tag와 같은 고정 위치(tag < 64).
        1 << property.tag()
    }

    /// 속성 하나의 집합.
    pub const fn of(property: LayoutProperty) -> LayoutPropertySet {
        LayoutPropertySet(LayoutPropertySet::bit(property))
    }

    /// 속성을 더한 집합.
    pub const fn with(self, property: LayoutProperty) -> LayoutPropertySet {
        LayoutPropertySet(self.0 | LayoutPropertySet::bit(property))
    }

    /// 합집합.
    pub const fn union(self, other: LayoutPropertySet) -> LayoutPropertySet {
        LayoutPropertySet(self.0 | other.0)
    }

    /// 포함 여부.
    pub const fn contains(self, property: LayoutProperty) -> bool {
        self.0 & LayoutPropertySet::bit(property) != 0
    }

    /// 비어 있는지 여부.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 포함된 속성(registry tag 순).
    pub fn iter(self) -> impl Iterator<Item = LayoutProperty> {
        LAYOUT_PROPERTIES
            .iter()
            .map(|metadata| metadata.property)
            .filter(move |property| self.contains(*property))
    }
}

impl FromIterator<LayoutProperty> for LayoutPropertySet {
    fn from_iter<I: IntoIterator<Item = LayoutProperty>>(iter: I) -> LayoutPropertySet {
        iter.into_iter()
            .fold(LayoutPropertySet::EMPTY, LayoutPropertySet::with)
    }
}

/// 자식 목록을 visual order(OrderKey, StableId tie-break)로 정렬한 비교용 사본.
fn visual_children(node: &LayoutNode) -> Vec<(&OrderKey, StableId)> {
    let mut children: Vec<(&OrderKey, StableId)> = node
        .children
        .iter()
        .map(|child| (&child.order_key, child.id))
        .collect();
    children.sort();
    children
}

/// 같은 node의 두 입력에서 바뀐 레이아웃 속성 (X.7.1).
///
/// `..` 없는 구조 분해라 [`LayoutNode`]에 필드가 추가되면 컴파일이 실패합니다(누락 방지). 자식 목록은 삽입
/// 순서가 아니라 visual order로 비교합니다(삽입 순서는 결과에 영향이 없음).
pub fn changed_properties(before: &LayoutNode, after: &LayoutNode) -> LayoutPropertySet {
    let LayoutNode {
        id: _,
        children: _,
        style,
        content,
        transform,
    } = before;
    let LayoutStyle {
        display,
        position,
        top,
        right,
        bottom,
        left,
        width,
        height,
        min_width,
        min_height,
        max_width,
        max_height,
        aspect_ratio,
        padding,
        flex_direction,
        flex_wrap,
        justify_content,
        align_content,
        align_items,
        justify_items,
        row_gap,
        column_gap,
        flex_grow,
        flex_shrink,
        flex_basis,
        align_self,
        justify_self,
        grid_template_columns,
        grid_template_rows,
        grid_auto_columns,
        grid_auto_rows,
        grid_auto_flow,
        grid_column_start,
        grid_column_end,
        grid_row_start,
        grid_row_end,
    } = style;
    let a = &after.style;
    let mut set = LayoutPropertySet::EMPTY;
    let mut mark = |changed: bool, property: LayoutProperty| {
        if changed {
            set = set.with(property);
        }
    };
    use LayoutProperty as P;
    mark(
        visual_children(before) != visual_children(after),
        P::Children,
    );
    mark(*display != a.display, P::Display);
    mark(*position != a.position, P::Position);
    mark(*top != a.top, P::Top);
    mark(*right != a.right, P::Right);
    mark(*bottom != a.bottom, P::Bottom);
    mark(*left != a.left, P::Left);
    mark(*width != a.width, P::Width);
    mark(*height != a.height, P::Height);
    mark(*min_width != a.min_width, P::MinWidth);
    mark(*min_height != a.min_height, P::MinHeight);
    mark(*max_width != a.max_width, P::MaxWidth);
    mark(*max_height != a.max_height, P::MaxHeight);
    mark(*aspect_ratio != a.aspect_ratio, P::AspectRatio);
    let (bt, br, bb, bl) = padding.components();
    let (at, ar, ab, al) = a.padding.components();
    mark(bt != at, P::PaddingTop);
    mark(br != ar, P::PaddingRight);
    mark(bb != ab, P::PaddingBottom);
    mark(bl != al, P::PaddingLeft);
    mark(*flex_direction != a.flex_direction, P::FlexDirection);
    mark(*flex_wrap != a.flex_wrap, P::FlexWrap);
    mark(*justify_content != a.justify_content, P::JustifyContent);
    mark(*align_content != a.align_content, P::AlignContent);
    mark(*align_items != a.align_items, P::AlignItems);
    mark(*justify_items != a.justify_items, P::JustifyItems);
    mark(*row_gap != a.row_gap, P::RowGap);
    mark(*column_gap != a.column_gap, P::ColumnGap);
    mark(*flex_grow != a.flex_grow, P::FlexGrow);
    mark(*flex_shrink != a.flex_shrink, P::FlexShrink);
    mark(*flex_basis != a.flex_basis, P::FlexBasis);
    mark(*align_self != a.align_self, P::AlignSelf);
    mark(*justify_self != a.justify_self, P::JustifySelf);
    mark(
        *grid_template_columns != a.grid_template_columns,
        P::GridTemplateColumns,
    );
    mark(
        *grid_template_rows != a.grid_template_rows,
        P::GridTemplateRows,
    );
    mark(
        *grid_auto_columns != a.grid_auto_columns,
        P::GridAutoColumns,
    );
    mark(*grid_auto_rows != a.grid_auto_rows, P::GridAutoRows);
    mark(*grid_auto_flow != a.grid_auto_flow, P::GridAutoFlow);
    mark(
        *grid_column_start != a.grid_column_start,
        P::GridColumnStart,
    );
    mark(*grid_column_end != a.grid_column_end, P::GridColumnEnd);
    mark(*grid_row_start != a.grid_row_start, P::GridRowStart);
    mark(*grid_row_end != a.grid_row_end, P::GridRowEnd);
    mark(*content != after.content, P::Content);
    mark(*transform != after.transform, P::Transform);
    set
}

/// 속성 집합 변경의 dirty class (X.7.1). node의 변경 전·후 상태(있는 쪽)로 각각 구한 합집합입니다.
///
/// 두 상태가 모두 없으면 registry 기본 class([`dirty_class`])를 씁니다.
pub fn node_dirty_classes(
    properties: LayoutPropertySet,
    before: Option<&LayoutNode>,
    after: Option<&LayoutNode>,
) -> DirtyClassSet {
    let mut set = DirtyClassSet::EMPTY;
    for property in properties.iter() {
        let states: Vec<&LayoutNode> = before.into_iter().chain(after).collect();
        if states.is_empty() {
            set = set.with(dirty_class(property));
        }
        for node in states {
            for class in dirty_classes(property, node).iter() {
                set = set.with(class);
            }
        }
    }
    set
}

/// 새 node·종류 변환처럼 모든 레이아웃 입력이 바뀐 node의 class.
pub const ALL_LAYOUT_CLASSES: DirtyClassSet = DirtyClassSet::EMPTY
    .with(DirtyClass::SelfSize)
    .with(DirtyClass::ParentFlow)
    .with(DirtyClass::DescendantConstraint);

/// 레이아웃 dirty set (X.7.1).
///
/// `layout`은 레이아웃에 영향을 주는 class(PaintOnly 제외)만, `paint`는 PaintOnly·렌더 전용 변경 node를
/// 담습니다. 순회는 StableId 순(결정적)입니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayoutDirtySet {
    layout: BTreeMap<StableId, DirtyClassSet>,
    paint: BTreeSet<StableId>,
}

impl LayoutDirtySet {
    /// 빈 집합.
    pub fn new() -> LayoutDirtySet {
        LayoutDirtySet::default()
    }

    /// node에 class를 더합니다. PaintOnly는 렌더 집합에만, 나머지는 layout 집합에 들어갑니다.
    pub fn mark(&mut self, node: StableId, classes: DirtyClassSet) {
        if classes.contains(DirtyClass::PaintOnly) {
            self.paint.insert(node);
        }
        let mut layout = DirtyClassSet::EMPTY;
        for class in classes.iter().filter(|class| class.affects_layout()) {
            layout = layout.with(class);
        }
        if layout.affects_layout() {
            let entry = self.layout.entry(node).or_default();
            for class in layout.iter() {
                *entry = entry.with(class);
            }
        }
    }

    /// node를 렌더 dirty로 표시합니다(레이아웃 queue에 넣지 않음).
    pub fn mark_paint(&mut self, node: StableId) {
        self.paint.insert(node);
    }

    /// 다른 집합을 합칩니다.
    pub fn merge(&mut self, other: &LayoutDirtySet) {
        for (node, classes) in &other.layout {
            self.mark(*node, *classes);
        }
        self.paint.extend(other.paint.iter().copied());
    }

    /// node의 레이아웃 class(PaintOnly 제외). 없으면 빈 집합.
    pub fn classes(&self, node: StableId) -> DirtyClassSet {
        self.layout.get(&node).copied().unwrap_or_default()
    }

    /// layout queue: 레이아웃을 다시 계산할 node와 class(StableId 순). PaintOnly만 바뀐 node는 없습니다.
    pub fn layout_queue(&self) -> Vec<(StableId, DirtyClassSet)> {
        self.layout
            .iter()
            .map(|(node, classes)| (*node, *classes))
            .collect()
    }

    /// 렌더만 다시 할 node(StableId 순). layout queue에 있는 node도 포함될 수 있습니다.
    pub fn paint(&self) -> Vec<StableId> {
        self.paint.iter().copied().collect()
    }

    /// 레이아웃을 다시 계산할 node가 없는지 여부.
    pub fn is_layout_clean(&self) -> bool {
        self.layout.is_empty()
    }

    /// layout queue node 수.
    pub fn layout_len(&self) -> usize {
        self.layout.len()
    }
}

/// document 속성 하나의 레이아웃 효과 (X.7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyLayoutEffect {
    /// layout·render 입력이 아닙니다(잠금, 표시 이름 등).
    None,
    /// 렌더만 바뀝니다(색 등). layout queue에 넣지 않습니다.
    PaintOnly,
    /// 레이아웃 입력이 바뀝니다.
    Layout {
        /// node 자신의 레이아웃 속성.
        node: LayoutPropertySet,
        /// 변경 전·후 부모의 레이아웃 속성(형제 순서·부모 변경은 부모의 Children).
        parent: LayoutPropertySet,
    },
}

impl PropertyLayoutEffect {
    /// node 자신의 속성만 바뀌는 효과.
    pub fn on_node(properties: LayoutPropertySet) -> PropertyLayoutEffect {
        PropertyLayoutEffect::Layout {
            node: properties,
            parent: LayoutPropertySet::EMPTY,
        }
    }

    /// 부모의 속성만 바뀌는 효과.
    pub fn on_parent(properties: LayoutPropertySet) -> PropertyLayoutEffect {
        PropertyLayoutEffect::Layout {
            node: LayoutPropertySet::EMPTY,
            parent: properties,
        }
    }
}

/// document 속성 key → 레이아웃 효과 매핑 (X.7.1). 문서 속성 registry(VI.3.5 등)를 가진 쪽이 구현합니다.
pub trait LayoutPropertyMap {
    /// key의 효과. registry에 없는 key는 `None`(변환은 node 무효화로 보수 처리).
    fn effect(&self, key: PropertyKey) -> Option<PropertyLayoutEffect>;
}

/// command 결과의 node 무효화 class (VI.9.7 `InvalidationClass`의 레이아웃 쪽 사본, X.7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CommandInvalidation {
    /// geometry·visual bounds.
    pub geometry: bool,
    /// layout.
    pub layout: bool,
    /// paint.
    pub paint: bool,
}

impl CommandInvalidation {
    /// 구조 변경(삽입·삭제·이동·변환) 무효화: geometry + layout + paint.
    pub const STRUCTURE: CommandInvalidation = CommandInvalidation {
        geometry: true,
        layout: true,
        paint: true,
    };

    /// paint만.
    pub const PAINT: CommandInvalidation = CommandInvalidation {
        geometry: false,
        layout: false,
        paint: true,
    };
}

/// 레이아웃이 읽는 command 결과 (VII.1.4 `ChangeSet`을 adapter가 감쌈, X.7.1). id는 layout id입니다.
pub trait LayoutChangeSource {
    /// 상태가 바뀐 node와 무효화 class(삽입 포함, 삭제 제외).
    fn changed_nodes(&self) -> Vec<(StableId, CommandInvalidation)>;
    /// 바뀐 속성(경로 root node, 첫 key).
    fn changed_properties(&self) -> Vec<(StableId, PropertyKey)>;
    /// 새로 생긴 node.
    fn inserted_nodes(&self) -> Vec<StableId>;
    /// 사라진 node.
    fn removed_nodes(&self) -> Vec<StableId>;
}

/// 레이아웃 입력 조회 (X.7.1). 변환은 변경 전·후 두 조회를 받습니다.
pub trait LayoutNodeLookup {
    /// node 입력.
    fn layout_node(&self, id: StableId) -> Option<&LayoutNode>;
    /// 부모 node.
    fn layout_parent(&self, id: StableId) -> Option<StableId>;
}

/// [`LayoutInput`]의 id·부모 index (X.7.1 조회용). 만들 때 node 수에 선형입니다.
#[derive(Debug, Clone)]
pub struct LayoutInputIndex<'a> {
    nodes: HashMap<StableId, &'a LayoutNode>,
    parents: HashMap<StableId, StableId>,
}

impl<'a> LayoutInputIndex<'a> {
    /// 입력 index를 만듭니다(검증하지 않음, 중복 id는 마지막 node).
    pub fn new(input: &'a LayoutInput) -> LayoutInputIndex<'a> {
        let mut nodes = HashMap::with_capacity(input.nodes.len());
        let mut parents = HashMap::with_capacity(input.nodes.len());
        for node in &input.nodes {
            nodes.insert(node.id, node);
            for child in &node.children {
                parents.insert(child.id, node.id);
            }
        }
        LayoutInputIndex { nodes, parents }
    }
}

impl LayoutNodeLookup for LayoutInputIndex<'_> {
    fn layout_node(&self, id: StableId) -> Option<&LayoutNode> {
        self.nodes.get(&id).copied()
    }

    fn layout_parent(&self, id: StableId) -> Option<StableId> {
        self.parents.get(&id).copied()
    }
}

/// command 결과를 레이아웃 dirty set으로 변환합니다 (X.7.1). 규칙은 모듈 문서를 따릅니다.
pub fn dirty_set_from_changes(
    changes: &dyn LayoutChangeSource,
    map: &dyn LayoutPropertyMap,
    before: &dyn LayoutNodeLookup,
    after: &dyn LayoutNodeLookup,
) -> LayoutDirtySet {
    let mut dirty = LayoutDirtySet::new();
    let classes_of = |properties: LayoutPropertySet, node: StableId| {
        node_dirty_classes(
            properties,
            before.layout_node(node),
            after.layout_node(node),
        )
    };
    // 변경 전·후 부모(중복 제거, 결정적 순서).
    let parents_of = |node: StableId| {
        let parents: BTreeSet<StableId> = [before.layout_parent(node), after.layout_parent(node)]
            .into_iter()
            .flatten()
            .collect();
        parents
    };
    let is_layout_node =
        |node: StableId| before.layout_node(node).is_some() || after.layout_node(node).is_some();

    // 1. 속성 변경. `explained`는 layout 무효화가 기록된 layout 속성·자식 구조 변경으로 설명되는 node입니다.
    let mut explained: BTreeSet<StableId> = BTreeSet::new();
    let mut unknown: BTreeSet<StableId> = BTreeSet::new();
    for (node, key) in changes.changed_properties() {
        match map.effect(key) {
            Some(PropertyLayoutEffect::None) => {}
            Some(PropertyLayoutEffect::PaintOnly) => dirty.mark_paint(node),
            Some(PropertyLayoutEffect::Layout { node: own, parent }) => {
                // 부모 쪽 속성(순서·부모 변경)만 있어도 node 자신의 layout 무효화는 이것으로 설명됩니다.
                explained.insert(node);
                if !own.is_empty() && is_layout_node(node) {
                    dirty.mark(node, classes_of(own, node));
                }
                if !parent.is_empty() {
                    for host in parents_of(node) {
                        explained.insert(host);
                        dirty.mark(host, classes_of(parent, host));
                    }
                }
            }
            // 알 수 없는 key: 아래 node 무효화 규칙으로 보수 처리합니다.
            None => {
                unknown.insert(node);
            }
        }
    }
    // 2. 삽입: 새 node는 모든 class, 변경 후 부모는 자식 변경.
    let children = LayoutPropertySet::of(LayoutProperty::Children);
    for node in changes.inserted_nodes() {
        if after.layout_node(node).is_some() {
            explained.insert(node);
            dirty.mark(node, ALL_LAYOUT_CLASSES);
        }
        if let Some(parent) = after.layout_parent(node) {
            explained.insert(parent);
            dirty.mark(parent, classes_of(children, parent));
        }
    }
    // 3. 삭제: 변경 전 부모는 자식 변경.
    for node in changes.removed_nodes() {
        if let Some(parent) = before.layout_parent(node) {
            explained.insert(parent);
            dirty.mark(parent, classes_of(children, parent));
        }
    }
    // 4. node 무효화: 설명되지 않은 layout 무효화(종류 변환 등)와 알 수 없는 key는 보수 처리합니다.
    let mut reported: BTreeSet<StableId> = BTreeSet::new();
    for (node, invalidation) in changes.changed_nodes() {
        reported.insert(node);
        if explained.contains(&node) && !unknown.contains(&node) {
            continue;
        }
        if invalidation.layout && is_layout_node(node) {
            dirty.mark(node, ALL_LAYOUT_CLASSES);
        } else if invalidation.paint || invalidation.geometry || invalidation.layout {
            dirty.mark_paint(node);
        }
    }
    // 5. 무효화가 보고되지 않은 node의 알 수 없는 key: 판단 근거가 없으므로 레이아웃 node면 세 class 모두,
    //    아니면 렌더 dirty로 둡니다(변경을 버리지 않음).
    for node in unknown.difference(&reported) {
        if is_layout_node(*node) {
            dirty.mark(*node, ALL_LAYOUT_CLASSES);
        } else {
            dirty.mark_paint(*node);
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{IntrinsicContent, LayoutContent};
    use crate::style::{
        AspectRatio, Display, FlexDirection, GridLine, LengthPercentageAuto, MaxSize, Position,
        SelfAlignment, Size, TrackSize,
    };
    use crate::test_support::{finite, fixed, id, key, len, size_px, style};
    use stellar_core_types::value::{Insets, Transform2D};

    // 문서 registry를 흉내 낸 key(공통 node 필드 0x00xx, Frame 0x01xx, Text 0x07xx 대역).
    const VISIBLE: PropertyKey = PropertyKey(0x0001);
    const LOCKED: PropertyKey = PropertyKey(0x0002);
    const PARENT: PropertyKey = PropertyKey(0x0004);
    const ORDER_KEY: PropertyKey = PropertyKey(0x0005);
    const SIZE: PropertyKey = PropertyKey(0x0101);
    const PADDING: PropertyKey = PropertyKey(0x0102);
    const LAYOUT_MODE: PropertyKey = PropertyKey(0x0103);
    const TRANSFORM: PropertyKey = PropertyKey(0x0104);
    const FILL: PropertyKey = PropertyKey(0x0105);
    const FLEX: PropertyKey = PropertyKey(0x0106);
    const TEXT: PropertyKey = PropertyKey(0x0701);
    const UNKNOWN: PropertyKey = PropertyKey(0x7777);

    struct Registry;

    impl LayoutPropertyMap for Registry {
        fn effect(&self, key: PropertyKey) -> Option<PropertyLayoutEffect> {
            use LayoutProperty as P;
            let node = |properties: &[LayoutProperty]| {
                PropertyLayoutEffect::on_node(properties.iter().copied().collect())
            };
            Some(match key {
                VISIBLE => node(&[P::Display]),
                LOCKED => PropertyLayoutEffect::None,
                PARENT | ORDER_KEY => {
                    PropertyLayoutEffect::on_parent(LayoutPropertySet::of(P::Children))
                }
                SIZE => node(&[P::Width, P::Height]),
                PADDING => node(&[
                    P::PaddingTop,
                    P::PaddingRight,
                    P::PaddingBottom,
                    P::PaddingLeft,
                ]),
                LAYOUT_MODE => node(&[P::Display, P::FlexDirection]),
                TRANSFORM => node(&[P::Transform]),
                FILL => PropertyLayoutEffect::PaintOnly,
                FLEX => node(&[P::FlexGrow, P::FlexShrink, P::FlexBasis]),
                TEXT => node(&[P::Content]),
                _ => return None,
            })
        }
    }

    #[derive(Default)]
    struct Changes {
        nodes: Vec<(StableId, CommandInvalidation)>,
        properties: Vec<(StableId, PropertyKey)>,
        inserted: Vec<StableId>,
        removed: Vec<StableId>,
    }

    impl LayoutChangeSource for Changes {
        fn changed_nodes(&self) -> Vec<(StableId, CommandInvalidation)> {
            self.nodes.clone()
        }
        fn changed_properties(&self) -> Vec<(StableId, PropertyKey)> {
            self.properties.clone()
        }
        fn inserted_nodes(&self) -> Vec<StableId> {
            self.inserted.clone()
        }
        fn removed_nodes(&self) -> Vec<StableId> {
            self.removed.clone()
        }
    }

    fn property_change(node: u64, key: PropertyKey, invalidation: CommandInvalidation) -> Changes {
        Changes {
            nodes: vec![(id(node), invalidation)],
            properties: vec![(id(node), key)],
            ..Changes::default()
        }
    }

    const LAYOUT: CommandInvalidation = CommandInvalidation::STRUCTURE;
    const PAINT: CommandInvalidation = CommandInvalidation::PAINT;
    const NONE: CommandInvalidation = CommandInvalidation {
        geometry: false,
        layout: false,
        paint: false,
    };

    /// root 1(px 크기 세로 flex) → 2(`auto` 크기 가로 flex) → 3(px leaf), 4(`auto` text leaf);
    /// root → 5(px 크기 block) → 6.
    fn document() -> LayoutInput {
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_style(style(|s| {
                        *s = fixed(400.0, 300.0);
                        s.display = Display::Flex;
                        s.flex_direction = FlexDirection::Column;
                    }))
                    .with_child(id(2), key(1))
                    .with_child(id(5), key(2)),
                LayoutNode::new(id(2))
                    .with_style(style(|s| s.display = Display::Flex))
                    .with_child(id(3), key(1))
                    .with_child(id(4), key(2)),
                LayoutNode::new(id(3)).with_style(fixed(10.0, 10.0)),
                LayoutNode::new(id(4)).with_content(LayoutContent::Intrinsic(IntrinsicContent {
                    width: len(30.0),
                    height: len(12.0),
                    baseline: Some(len(9.0)),
                })),
                LayoutNode::new(id(5))
                    .with_style(fixed(100.0, 50.0))
                    .with_child(id(6), key(1)),
                LayoutNode::new(id(6)).with_style(fixed(5.0, 5.0)),
            ],
        )
    }

    fn convert(changes: &Changes, before: &LayoutInput, after: &LayoutInput) -> LayoutDirtySet {
        dirty_set_from_changes(
            changes,
            &Registry,
            &LayoutInputIndex::new(before),
            &LayoutInputIndex::new(after),
        )
    }

    fn set(classes: &[DirtyClass]) -> DirtyClassSet {
        classes
            .iter()
            .fold(DirtyClassSet::EMPTY, |acc, class| acc.with(*class))
    }

    fn queue(entries: &[(u64, &[DirtyClass])]) -> Vec<(StableId, DirtyClassSet)> {
        entries
            .iter()
            .map(|(node, classes)| (id(*node), set(classes)))
            .collect()
    }

    #[test]
    fn property_changes_map_to_expected_dirty_sets() {
        use DirtyClass::{DescendantConstraint as Desc, ParentFlow as Flow, SelfSize as Size};
        let doc = document();
        type Case = (
            u64,
            PropertyKey,
            CommandInvalidation,
            Vec<(StableId, DirtyClassSet)>,
            Vec<u64>,
        );
        let cases: Vec<Case> = vec![
            (3, SIZE, LAYOUT, queue(&[(3, &[Size])]), vec![]),
            (4, TEXT, LAYOUT, queue(&[(4, &[Size])]), vec![]),
            // px 크기 node의 padding: DescendantConstraint만.
            (5, PADDING, LAYOUT, queue(&[(5, &[Desc])]), vec![]),
            // `auto` 크기 node의 padding: 자기 크기도 바뀌어 SelfSize 포함.
            (2, PADDING, LAYOUT, queue(&[(2, &[Size, Desc])]), vec![]),
            // display는 부모 flow 참여도 바꿉니다.
            (
                2,
                LAYOUT_MODE,
                LAYOUT,
                queue(&[(2, &[Size, Flow, Desc])]),
                vec![],
            ),
            (3, VISIBLE, LAYOUT, queue(&[(3, &[Flow, Desc])]), vec![]),
            (3, FLEX, LAYOUT, queue(&[(3, &[Flow])]), vec![]),
            (3, ORDER_KEY, LAYOUT, queue(&[(2, &[Size, Desc])]), vec![]),
            (3, TRANSFORM, PAINT, vec![], vec![3]),
            (3, FILL, PAINT, vec![], vec![3]),
            (3, LOCKED, NONE, vec![], vec![]),
        ];
        for (node, key, invalidation, expected_queue, expected_paint) in cases {
            let dirty = convert(&property_change(node, key, invalidation), &doc, &doc);
            assert_eq!(dirty.layout_queue(), expected_queue, "{key:?} on {node}");
            assert_eq!(
                dirty.paint(),
                expected_paint.into_iter().map(id).collect::<Vec<_>>(),
                "{key:?} on {node}"
            );
        }
    }

    #[test]
    fn classes_use_both_before_and_after_states() {
        use DirtyClass::{DescendantConstraint as Desc, SelfSize as Size};
        let before = document();
        let mut after = document();
        after.nodes[4].style.width = crate::style::Size::Auto;
        let changes = Changes {
            nodes: vec![(id(5), CommandInvalidation::STRUCTURE)],
            properties: vec![(id(5), SIZE), (id(5), PADDING)],
            ..Changes::default()
        };
        let dirty = convert(&changes, &before, &after);
        assert_eq!(dirty.layout_queue(), queue(&[(5, &[Size, Desc])]));
        let dirty = convert(&changes, &after, &before);
        assert_eq!(dirty.layout_queue(), queue(&[(5, &[Size, Desc])]));
    }

    #[test]
    fn structure_insert_remove_and_move_mark_parents() {
        use DirtyClass::{DescendantConstraint as Desc, SelfSize as Size};
        let before = document();
        let mut after = document();
        after.nodes[4] = after.nodes[4].clone().with_child(id(7), key(2));
        after
            .nodes
            .push(LayoutNode::new(id(7)).with_style(fixed(1.0, 1.0)));
        let insert = Changes {
            nodes: vec![
                (id(5), CommandInvalidation::STRUCTURE),
                (id(7), CommandInvalidation::STRUCTURE),
            ],
            inserted: vec![id(7)],
            ..Changes::default()
        };
        let dirty = convert(&insert, &before, &after);
        assert_eq!(
            dirty.layout_queue(),
            vec![(id(5), set(&[Desc])), (id(7), ALL_LAYOUT_CLASSES)]
        );
        let remove = Changes {
            nodes: vec![(id(5), CommandInvalidation::STRUCTURE)],
            removed: vec![id(7)],
            ..Changes::default()
        };
        let dirty = convert(&remove, &after, &before);
        assert_eq!(dirty.layout_queue(), vec![(id(5), set(&[Desc]))]);
        let converted = Changes {
            nodes: vec![(id(5), CommandInvalidation::STRUCTURE)],
            properties: vec![(id(5), LOCKED)],
            ..Changes::default()
        };
        assert_eq!(
            convert(&converted, &before, &before).layout_queue(),
            vec![(id(5), ALL_LAYOUT_CLASSES)]
        );
        let mut moved = document();
        moved.nodes[1].children.retain(|c| c.id != id(3));
        moved.nodes[4] = moved.nodes[4].clone().with_child(id(3), key(5));
        let change = Changes {
            nodes: vec![(id(3), CommandInvalidation::STRUCTURE)],
            properties: vec![(id(3), PARENT)],
            ..Changes::default()
        };
        let dirty = convert(&change, &before, &moved);
        assert_eq!(
            dirty.layout_queue(),
            queue(&[(2, &[Size, Desc]), (5, &[Desc])])
        );
    }

    #[test]
    fn unknown_keys_fall_back_to_node_invalidation() {
        let doc = document();
        let dirty = convert(&property_change(3, UNKNOWN, LAYOUT), &doc, &doc);
        assert_eq!(dirty.layout_queue(), vec![(id(3), ALL_LAYOUT_CLASSES)]);
        let dirty = convert(&property_change(3, UNKNOWN, PAINT), &doc, &doc);
        assert!(dirty.is_layout_clean());
        assert_eq!(dirty.paint(), vec![id(3)]);
        let dirty = convert(&property_change(99, UNKNOWN, LAYOUT), &doc, &doc);
        assert!(dirty.is_layout_clean());
        assert_eq!(dirty.paint(), vec![id(99)]);
        let mixed = Changes {
            nodes: vec![(id(3), CommandInvalidation::STRUCTURE)],
            properties: vec![(id(3), SIZE), (id(3), UNKNOWN)],
            ..Changes::default()
        };
        assert_eq!(
            convert(&mixed, &doc, &doc).layout_queue(),
            vec![(id(3), ALL_LAYOUT_CLASSES)]
        );
    }

    #[test]
    fn unknown_keys_without_node_invalidation_are_conservative() {
        let doc = document();
        let orphan = Changes {
            properties: vec![(id(3), UNKNOWN)],
            ..Changes::default()
        };
        let dirty = convert(&orphan, &doc, &doc);
        assert_eq!(dirty.layout_queue(), vec![(id(3), ALL_LAYOUT_CLASSES)]);
        let mixed = Changes {
            properties: vec![(id(4), TEXT), (id(4), UNKNOWN)],
            ..Changes::default()
        };
        assert_eq!(
            convert(&mixed, &doc, &doc).layout_queue(),
            vec![(id(4), ALL_LAYOUT_CLASSES)]
        );
        let modifier = Changes {
            properties: vec![(id(99), UNKNOWN)],
            ..Changes::default()
        };
        let dirty = convert(&modifier, &doc, &doc);
        assert!(dirty.is_layout_clean());
        assert_eq!(dirty.paint(), vec![id(99)]);
        let other = Changes {
            nodes: vec![(id(5), CommandInvalidation::PAINT)],
            properties: vec![(id(3), UNKNOWN), (id(5), FILL)],
            ..Changes::default()
        };
        let dirty = convert(&other, &doc, &doc);
        assert_eq!(dirty.layout_queue(), vec![(id(3), ALL_LAYOUT_CLASSES)]);
        assert_eq!(dirty.paint(), vec![id(5)]);
    }

    #[test]
    fn changed_properties_detects_each_field() {
        use LayoutProperty as P;
        let base = LayoutNode::new(id(1))
            .with_style(fixed(10.0, 10.0))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        assert!(changed_properties(&base, &base).is_empty());
        let mut reordered = base.clone();
        reordered.children.reverse();
        assert!(changed_properties(&base, &reordered).is_empty());
        let edit = |f: &dyn Fn(&mut crate::style::LayoutStyle)| {
            let mut node = base.clone();
            f(&mut node.style);
            node
        };
        let length = LengthPercentageAuto::Length(len(1.0));
        let cases: Vec<(LayoutNode, LayoutProperty)> = vec![
            (base.clone().with_child(id(4), key(3)), P::Children),
            (edit(&|s| s.display = Display::Flex), P::Display),
            (edit(&|s| s.position = Position::Absolute), P::Position),
            (edit(&|s| s.top = length), P::Top),
            (edit(&|s| s.right = length), P::Right),
            (edit(&|s| s.bottom = length), P::Bottom),
            (edit(&|s| s.left = length), P::Left),
            (edit(&|s| s.width = Size::FitContent), P::Width),
            (edit(&|s| s.height = Size::Auto), P::Height),
            (edit(&|s| s.min_width = size_px(1.0)), P::MinWidth),
            (edit(&|s| s.min_height = size_px(1.0)), P::MinHeight),
            (
                edit(&|s| s.max_width = MaxSize::Length(len(1.0))),
                P::MaxWidth,
            ),
            (
                edit(&|s| s.max_height = MaxSize::Length(len(1.0))),
                P::MaxHeight,
            ),
            (
                edit(&|s| s.aspect_ratio = AspectRatio::new(1.0, 1.0)),
                P::AspectRatio,
            ),
            (
                edit(&|s| s.padding = Insets::new(1.0, 0.0, 0.0, 0.0).unwrap()),
                P::PaddingTop,
            ),
            (
                edit(&|s| s.padding = Insets::new(0.0, 1.0, 0.0, 0.0).unwrap()),
                P::PaddingRight,
            ),
            (
                edit(&|s| s.padding = Insets::new(0.0, 0.0, 1.0, 0.0).unwrap()),
                P::PaddingBottom,
            ),
            (
                edit(&|s| s.padding = Insets::new(0.0, 0.0, 0.0, 1.0).unwrap()),
                P::PaddingLeft,
            ),
            (
                edit(&|s| s.flex_direction = FlexDirection::Column),
                P::FlexDirection,
            ),
            (
                edit(&|s| s.flex_wrap = crate::style::FlexWrap::Wrap),
                P::FlexWrap,
            ),
            (
                edit(&|s| s.justify_content = crate::style::ContentAlignment::Center),
                P::JustifyContent,
            ),
            (
                edit(&|s| s.align_content = crate::style::ContentAlignment::Center),
                P::AlignContent,
            ),
            (
                edit(&|s| s.align_items = crate::style::ItemAlignment::Center),
                P::AlignItems,
            ),
            (
                edit(&|s| s.justify_items = crate::style::ItemAlignment::Center),
                P::JustifyItems,
            ),
            (edit(&|s| s.row_gap = len(1.0)), P::RowGap),
            (edit(&|s| s.column_gap = len(1.0)), P::ColumnGap),
            (edit(&|s| s.flex_grow = finite(1.0)), P::FlexGrow),
            (edit(&|s| s.flex_shrink = finite(0.0)), P::FlexShrink),
            (
                edit(&|s| s.flex_basis = crate::style::FlexBasis::Content),
                P::FlexBasis,
            ),
            (
                edit(&|s| s.align_self = SelfAlignment::Item(crate::style::ItemAlignment::End)),
                P::AlignSelf,
            ),
            (
                edit(&|s| s.justify_self = SelfAlignment::Item(crate::style::ItemAlignment::End)),
                P::JustifySelf,
            ),
            (
                edit(&|s| s.grid_template_columns = vec![TrackSize::AUTO]),
                P::GridTemplateColumns,
            ),
            (
                edit(&|s| s.grid_template_rows = vec![TrackSize::AUTO]),
                P::GridTemplateRows,
            ),
            (
                edit(&|s| s.grid_auto_columns = vec![TrackSize::MIN_CONTENT]),
                P::GridAutoColumns,
            ),
            (
                edit(&|s| s.grid_auto_rows = vec![TrackSize::MIN_CONTENT]),
                P::GridAutoRows,
            ),
            (
                edit(&|s| s.grid_auto_flow = crate::style::GridAutoFlow::Column),
                P::GridAutoFlow,
            ),
            (
                edit(&|s| s.grid_column_start = GridLine::Line(1)),
                P::GridColumnStart,
            ),
            (
                edit(&|s| s.grid_column_end = GridLine::Line(2)),
                P::GridColumnEnd,
            ),
            (
                edit(&|s| s.grid_row_start = GridLine::Line(1)),
                P::GridRowStart,
            ),
            (edit(&|s| s.grid_row_end = GridLine::Span(2)), P::GridRowEnd),
            (
                base.clone()
                    .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                        width: len(1.0),
                        height: len(1.0),
                        baseline: None,
                    })),
                P::Content,
            ),
            (
                base.clone()
                    .with_transform(Transform2D::new(1.0, 0.0, 0.0, 1.0, 3.0, 0.0).unwrap()),
                P::Transform,
            ),
        ];
        assert_eq!(cases.len(), LAYOUT_PROPERTIES.len());
        for (changed, property) in cases {
            assert_eq!(
                changed_properties(&base, &changed),
                LayoutPropertySet::of(property),
                "{property}"
            );
        }
    }

    #[test]
    fn dirty_set_and_property_set_operations() {
        let mut dirty = LayoutDirtySet::new();
        dirty.mark(id(1), DirtyClassSet::of(DirtyClass::PaintOnly));
        assert!(dirty.is_layout_clean());
        assert_eq!(dirty.paint(), vec![id(1)]);
        dirty.mark(
            id(2),
            DirtyClassSet::of(DirtyClass::SelfSize).with(DirtyClass::PaintOnly),
        );
        assert_eq!(
            dirty.layout_queue(),
            vec![(id(2), DirtyClassSet::of(DirtyClass::SelfSize))]
        );
        assert_eq!(dirty.paint(), vec![id(1), id(2)]);
        let mut other = LayoutDirtySet::new();
        other.mark(id(2), DirtyClassSet::of(DirtyClass::ParentFlow));
        dirty.merge(&other);
        assert_eq!(
            dirty.classes(id(2)),
            DirtyClassSet::of(DirtyClass::SelfSize).with(DirtyClass::ParentFlow)
        );
        assert_eq!(dirty.layout_len(), 1);
        assert_eq!(dirty.classes(id(9)), DirtyClassSet::EMPTY);
        for metadata in LAYOUT_PROPERTIES {
            assert_eq!(
                LayoutPropertySet::of(metadata.property).0,
                1 << metadata.tag
            );
        }
        let all: LayoutPropertySet = LAYOUT_PROPERTIES.iter().map(|m| m.property).collect();
        assert_eq!(all.iter().count(), LAYOUT_PROPERTIES.len());
        let pair = LayoutPropertySet::of(LayoutProperty::Width)
            .union(LayoutPropertySet::of(LayoutProperty::PaddingTop));
        assert!(pair.contains(LayoutProperty::Width) && !pair.contains(LayoutProperty::Height));
        assert_eq!(
            node_dirty_classes(
                LayoutPropertySet::of(LayoutProperty::PaddingTop),
                None,
                None
            ),
            DirtyClassSet::of(DirtyClass::DescendantConstraint)
        );
    }
}
