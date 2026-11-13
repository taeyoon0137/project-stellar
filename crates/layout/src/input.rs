//! 레이아웃 입력 (X.1.1).
//!
//! [`LayoutInput`]은 레이아웃 엔진이 읽는 node tree 하나입니다. 문서 모델과 분리된 레이아웃
//! 전용 값만 담으며(문서를 수정하지 않음), 모든 float은 [`Length`]·[`Insets`] 등 finite 타입이라
//! NaN·±무한대를 담을 수 없습니다. [`LayoutInput::validate`]는 구조(tree)와 값 범위를 검사합니다.
//!
//! # 자식 순서
//!
//! 자식은 [`LayoutChild::order_key`] 오름차순(같은 키는 `StableId` tie-break, III.5)으로 정렬한
//! **visual order**로 배치합니다. 입력 목록의 삽입 순서는 결과에 영향을 주지 않습니다.
//!
//! # canonical bytes
//!
//! [`LayoutInput::encode`]/[`LayoutInput::decode`]는 runtime·fixture 비교용 little-endian 형식
//! (magic `SLYI`, version 1)입니다. 저장 포맷이 아니며, 디코딩은 비유한 float과 비정규
//! OrderKey를 거부합니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Insets, Length, Percentage};

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::property::LayoutProperty;
use crate::rounding::LayoutUnit;
use crate::sizing::{validate_sizing, SIZING_FILL, SIZING_FIXED, SIZING_HUG, SIZING_PERCENTAGE};

pub use crate::sizing::SizingMode;

/// 레이아웃 재귀가 허용하는 최대 tree 깊이(root = 0).
pub const MAX_LAYOUT_DEPTH: usize = 256;

/// 입력 canonical bytes magic.
const INPUT_MAGIC: &[u8; 4] = b"SLYI";
/// 입력 canonical bytes version.
const INPUT_VERSION: u16 = 1;

/// 부모가 가진 자식 참조 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutChild {
    /// 자식 node.
    pub id: StableId,
    /// 형제 사이 visual order 키.
    pub order_key: OrderKey,
}

/// 자식 배치 방식 (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum ContainerLayout {
    /// 자유 배치(자식 local position·size 사용).
    #[default]
    Free,
}

/// leaf 콘텐츠의 intrinsic 크기와 baseline (X.1.1).
///
/// Text·Image 등 콘텐츠 측정 결과를 레이아웃 입력으로 전달합니다. baseline은 콘텐츠 top에서의
/// 거리입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntrinsicContent {
    /// intrinsic 너비(비음수).
    pub width: Length,
    /// intrinsic 높이(비음수).
    pub height: Length,
    /// 첫 baseline(콘텐츠 top 기준). 없으면 `None`.
    pub baseline: Option<Length>,
}

/// node 콘텐츠 (X.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutContent {
    /// intrinsic 콘텐츠 없음(container 또는 내용 크기 없는 leaf).
    #[default]
    None,
    /// intrinsic 콘텐츠 leaf. 자식을 가질 수 없습니다.
    Intrinsic(IntrinsicContent),
}

/// 레이아웃 node 하나의 입력 (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    /// node 식별자.
    pub id: StableId,
    /// 자식(삽입 순서 무관, visual order는 OrderKey).
    pub children: Vec<LayoutChild>,
    /// 자식 배치 방식.
    pub container: ContainerLayout,
    /// 안쪽 여백(비음수).
    pub padding: Insets,
    /// 너비 sizing.
    pub width: SizingMode,
    /// 높이 sizing.
    pub height: SizingMode,
    /// 콘텐츠.
    pub content: LayoutContent,
}

impl LayoutNode {
    /// 기본값(자식 없음, Free, padding 0, 0×0 고정, 콘텐츠 없음) node를 만듭니다.
    pub fn new(id: StableId) -> LayoutNode {
        LayoutNode {
            id,
            children: Vec::new(),
            container: ContainerLayout::default(),
            padding: zero_insets(),
            width: SizingMode::default(),
            height: SizingMode::default(),
            content: LayoutContent::None,
        }
    }

    /// 자식을 추가합니다.
    pub fn with_child(mut self, id: StableId, order_key: OrderKey) -> LayoutNode {
        self.children.push(LayoutChild { id, order_key });
        self
    }

    /// 고정 크기를 지정합니다.
    pub fn with_fixed_size(mut self, width: Length, height: Length) -> LayoutNode {
        self.width = SizingMode::Fixed(width);
        self.height = SizingMode::Fixed(height);
        self
    }

    /// 두 축 sizing을 지정합니다.
    pub fn with_sizing(mut self, width: SizingMode, height: SizingMode) -> LayoutNode {
        self.width = width;
        self.height = height;
        self
    }

    /// padding을 지정합니다.
    pub fn with_padding(mut self, padding: Insets) -> LayoutNode {
        self.padding = padding;
        self
    }

    /// 자식 배치 방식을 지정합니다.
    pub fn with_container(mut self, container: ContainerLayout) -> LayoutNode {
        self.container = container;
        self
    }

    /// 콘텐츠를 지정합니다.
    pub fn with_content(mut self, content: LayoutContent) -> LayoutNode {
        self.content = content;
        self
    }
}

/// 0 inset.
fn zero_insets() -> Insets {
    Insets::new(0.0, 0.0, 0.0, 0.0).expect("0은 유한")
}

/// 레이아웃 입력 tree (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutInput {
    /// root node.
    pub root: StableId,
    /// 모든 node(순서 무관).
    pub nodes: Vec<LayoutNode>,
}

/// 검증된 tree index (crate 내부).
#[derive(Debug, Clone)]
// 레이아웃 엔진(X.2.1)이 읽기 전까지는 검증 결과로만 사용합니다.
#[allow(dead_code)]
pub(crate) struct TreeIndex {
    /// StableId → `nodes` index.
    pub(crate) index: HashMap<StableId, usize>,
    /// 부모 index.
    pub(crate) parent: Vec<Option<usize>>,
    /// visual order로 정렬한 자식 index.
    pub(crate) children: Vec<Vec<usize>>,
    /// root index.
    pub(crate) root: usize,
}

impl LayoutInput {
    /// 입력을 만듭니다(검증은 [`LayoutInput::validate`]).
    pub fn new(root: StableId, nodes: Vec<LayoutNode>) -> LayoutInput {
        LayoutInput { root, nodes }
    }

    /// StableId로 node를 찾습니다.
    pub fn node(&self, id: StableId) -> Option<&LayoutNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// 구조(tree)와 값 범위를 검증합니다.
    pub fn validate(&self) -> Result<(), LayoutError> {
        self.tree().map(|_| ())
    }

    /// 검증하고 tree index를 만듭니다.
    pub(crate) fn tree(&self) -> Result<TreeIndex, LayoutError> {
        let mut index = HashMap::with_capacity(self.nodes.len());
        for (position, node) in self.nodes.iter().enumerate() {
            if index.insert(node.id, position).is_some() {
                return Err(LayoutError::DuplicateNode { node: node.id });
            }
        }
        let root = *index
            .get(&self.root)
            .ok_or(LayoutError::MissingRoot { root: self.root })?;

        let mut parent: Vec<Option<usize>> = vec![None; self.nodes.len()];
        let mut children: Vec<Vec<usize>> = Vec::with_capacity(self.nodes.len());
        for (position, node) in self.nodes.iter().enumerate() {
            if !node.children.is_empty() && matches!(node.content, LayoutContent::Intrinsic(_)) {
                return Err(LayoutError::ChildrenOnContentLeaf { node: node.id });
            }
            let mut ordered: Vec<(&OrderKey, StableId, usize)> =
                Vec::with_capacity(node.children.len());
            for child in &node.children {
                let child_index = *index.get(&child.id).ok_or(LayoutError::MissingChild {
                    parent: node.id,
                    child: child.id,
                })?;
                if child_index == root {
                    return Err(LayoutError::Cycle { node: child.id });
                }
                if parent[child_index].is_some() {
                    return Err(LayoutError::MultipleParents { node: child.id });
                }
                parent[child_index] = Some(position);
                ordered.push((&child.order_key, child.id, child_index));
            }
            // visual order: OrderKey 오름차순, 같은 키는 StableId tie-break(III.5).
            ordered.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
            children.push(ordered.into_iter().map(|(_, _, i)| i).collect());
        }

        // root에서 반복 DFS로 도달 가능성과 깊이를 검사합니다.
        let mut visited = vec![false; self.nodes.len()];
        let mut stack = vec![(root, 0usize)];
        while let Some((current, depth)) = stack.pop() {
            if depth > MAX_LAYOUT_DEPTH {
                return Err(LayoutError::DepthLimitExceeded {
                    node: self.nodes[current].id,
                    limit: MAX_LAYOUT_DEPTH,
                });
            }
            visited[current] = true;
            for &child in &children[current] {
                stack.push((child, depth + 1));
            }
        }
        for (position, seen) in visited.iter().enumerate() {
            if !*seen {
                let node = self.nodes[position].id;
                return Err(if parent_chain_loops(&parent, position) {
                    LayoutError::Cycle { node }
                } else {
                    LayoutError::Unreachable { node }
                });
            }
        }

        for (position, node) in self.nodes.iter().enumerate() {
            validate_node_values(node, position == root)?;
        }
        Ok(TreeIndex {
            index,
            parent,
            children,
            root,
        })
    }

    /// canonical bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(INPUT_MAGIC);
        w.u16(INPUT_VERSION);
        w.id(self.root);
        w.len(self.nodes.len());
        for node in &self.nodes {
            encode_node(&mut w, node);
        }
        w.finish()
    }

    /// canonical bytes에서 디코딩합니다. 형식·finite 검사만 하며 tree 검증은
    /// [`LayoutInput::validate`]가 합니다.
    pub fn decode(bytes: &[u8]) -> Result<LayoutInput, LayoutError> {
        let mut r = Reader::new(bytes);
        r.expect_magic(INPUT_MAGIC, INPUT_VERSION)?;
        let root = r.id()?;
        let count = r.len(16)?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            nodes.push(decode_node(&mut r)?);
        }
        r.expect_end()?;
        Ok(LayoutInput { root, nodes })
    }
}

/// 부모 사슬이 root(부모 없음)에 닿지 않고 순환하는지 검사합니다.
fn parent_chain_loops(parent: &[Option<usize>], start: usize) -> bool {
    let mut current = start;
    for _ in 0..=parent.len() {
        match parent[current] {
            Some(next) => current = next,
            None => return false,
        }
    }
    true
}

/// 길이 값을 subpixel로 변환합니다. 범위 밖은 `OutOfRange`, 음수 불허 시 음수는 `NegativeValue`.
pub(crate) fn length_unit(
    node: StableId,
    property: LayoutProperty,
    value: Length,
    allow_negative: bool,
) -> Result<LayoutUnit, LayoutError> {
    if !allow_negative && value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    LayoutUnit::from_length(value).map_err(|_| LayoutError::OutOfRange { node, property })
}

/// node 하나의 값 범위와 속성 조합을 검증합니다.
fn validate_node_values(node: &LayoutNode, is_root: bool) -> Result<(), LayoutError> {
    let (top, right, bottom, left) = node.padding.components();
    for value in [top, right, bottom, left] {
        let length = Length::new(value).map_err(|_| LayoutError::OutOfRange {
            node: node.id,
            property: LayoutProperty::Padding,
        })?;
        length_unit(node.id, LayoutProperty::Padding, length, false)?;
    }
    validate_sizing(node.id, Axis::Horizontal, node.width, is_root)?;
    validate_sizing(node.id, Axis::Vertical, node.height, is_root)?;
    if let LayoutContent::Intrinsic(content) = node.content {
        length_unit(node.id, LayoutProperty::Content, content.width, false)?;
        length_unit(node.id, LayoutProperty::Content, content.height, false)?;
        if let Some(baseline) = content.baseline {
            length_unit(node.id, LayoutProperty::Content, baseline, true)?;
        }
    }
    Ok(())
}

// --- canonical bytes ---

const CONTAINER_FREE: u8 = 0;

const CONTENT_NONE: u8 = 0;
const CONTENT_INTRINSIC: u8 = 1;

fn encode_node(w: &mut Writer, node: &LayoutNode) {
    w.id(node.id);
    w.len(node.children.len());
    for child in &node.children {
        w.id(child.id);
        w.order_key(&child.order_key);
    }
    match node.container {
        ContainerLayout::Free => w.u8(CONTAINER_FREE),
    }
    w.insets(&node.padding);
    encode_sizing(w, node.width);
    encode_sizing(w, node.height);
    match node.content {
        LayoutContent::None => w.u8(CONTENT_NONE),
        LayoutContent::Intrinsic(content) => {
            w.u8(CONTENT_INTRINSIC);
            w.length(content.width);
            w.length(content.height);
            match content.baseline {
                Some(baseline) => {
                    w.bool(true);
                    w.length(baseline);
                }
                None => w.bool(false),
            }
        }
    }
}

fn encode_sizing(w: &mut Writer, mode: SizingMode) {
    w.u8(mode.tag());
    match mode {
        SizingMode::Fixed(length) => w.length(length),
        SizingMode::Percentage(percent) => w.finite(percent.raw()),
        SizingMode::Fill | SizingMode::HugContent => {}
    }
}

fn decode_node(r: &mut Reader<'_>) -> Result<LayoutNode, LayoutError> {
    let id = r.id()?;
    let child_count = r.len(20)?;
    let mut children = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        let child = r.id()?;
        let order_key = r.order_key()?;
        children.push(LayoutChild {
            id: child,
            order_key,
        });
    }
    let container = match r.u8()? {
        CONTAINER_FREE => ContainerLayout::Free,
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 container tag",
            })
        }
    };
    let padding = r.insets("padding")?;
    let width = decode_sizing(r, "width")?;
    let height = decode_sizing(r, "height")?;
    let content = match r.u8()? {
        CONTENT_NONE => LayoutContent::None,
        CONTENT_INTRINSIC => {
            let width = r.length("content width")?;
            let height = r.length("content height")?;
            let baseline = if r.bool("baseline flag")? {
                Some(r.length("content baseline")?)
            } else {
                None
            };
            LayoutContent::Intrinsic(IntrinsicContent {
                width,
                height,
                baseline,
            })
        }
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 content tag",
            })
        }
    };
    Ok(LayoutNode {
        id,
        children,
        container,
        padding,
        width,
        height,
        content,
    })
}

fn decode_sizing(r: &mut Reader<'_>, field: &'static str) -> Result<SizingMode, LayoutError> {
    match r.u8()? {
        SIZING_FIXED => Ok(SizingMode::Fixed(r.length(field)?)),
        SIZING_FILL => Ok(SizingMode::Fill),
        SIZING_HUG => Ok(SizingMode::HugContent),
        SIZING_PERCENTAGE => Ok(SizingMode::Percentage(Percentage::from_raw(
            r.finite(field)?,
        ))),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 sizing tag",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{id, key, len};

    fn sample_input() -> LayoutInput {
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(200.0), len(100.0))
            .with_padding(Insets::new(4.0, 8.0, 4.0, 8.0).unwrap())
            .with_child(id(3), key(2))
            .with_child(id(2), key(1));
        let text = LayoutNode::new(id(2))
            .with_fixed_size(len(50.0), len(20.0))
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(48.5),
                height: len(18.25),
                baseline: Some(len(14.0)),
            }));
        let image = LayoutNode::new(id(3))
            .with_fixed_size(len(1.0 / 3.0), len(0.0))
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(640.0),
                height: len(480.0),
                baseline: None,
            }));
        LayoutInput::new(id(1), vec![root, text, image])
    }

    #[test]
    fn input_round_trips_through_canonical_bytes() {
        let input = sample_input();
        input.validate().unwrap();
        let bytes = input.encode();
        assert_eq!(&bytes[..4], b"SLYI");
        let decoded = LayoutInput::decode(&bytes).unwrap();
        assert_eq!(decoded, input);
        // 재인코딩도 같은 bytes(canonical).
        assert_eq!(decoded.encode(), bytes);
        decoded.validate().unwrap();
    }

    #[test]
    fn decode_rejects_non_finite_and_malformed_bytes() {
        let input = sample_input();
        let bytes = input.encode();
        // root width Fixed 값(f32)을 NaN·+∞로 바꾸면 거부. 위치: magic4+ver2+root16+count4
        // + node id16 + child count4 + 자식 2개(id16+key len4+key4)*2 + container1 + padding16
        // + sizing tag1.
        let width_offset = 4 + 2 + 16 + 4 + 16 + 4 + (16 + 4 + 4) * 2 + 1 + 16 + 1;
        assert_eq!(
            f32::from_le_bytes(bytes[width_offset..width_offset + 4].try_into().unwrap()),
            200.0
        );
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut corrupt = bytes.clone();
            corrupt[width_offset..width_offset + 4].copy_from_slice(&bad.to_le_bytes());
            assert_eq!(
                LayoutInput::decode(&corrupt),
                Err(LayoutError::NonFinite { field: "width" })
            );
        }
        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(matches!(
            LayoutInput::decode(&truncated),
            Err(LayoutError::Malformed { .. })
        ));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            LayoutInput::decode(&trailing),
            Err(LayoutError::Malformed {
                reason: "trailing bytes"
            })
        );
        let mut magic = bytes;
        magic[0] = b'X';
        assert!(LayoutInput::decode(&magic).is_err());
    }

    #[test]
    fn value_validation_rejects_negative_and_out_of_range() {
        let negative = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_fixed_size(len(-1.0), len(1.0))],
        );
        assert_eq!(
            negative.validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::Width
            })
        );
        // 2^25 px는 LayoutUnit(i32 subpixel) 범위 밖.
        let huge = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_fixed_size(len(1.0), len(33_554_432.0))],
        );
        assert_eq!(
            huge.validate(),
            Err(LayoutError::OutOfRange {
                node: id(1),
                property: LayoutProperty::Height
            })
        );
        let padding = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_padding(Insets::new(0.0, -2.0, 0.0, 0.0).unwrap())],
        );
        assert_eq!(
            padding.validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::Padding
            })
        );
        // baseline은 음수 허용, intrinsic 크기는 비음수.
        let content = |baseline: f32, width: f32| {
            LayoutInput::new(
                id(1),
                vec![
                    LayoutNode::new(id(1)).with_content(LayoutContent::Intrinsic(
                        IntrinsicContent {
                            width: len(width),
                            height: len(1.0),
                            baseline: Some(len(baseline)),
                        },
                    )),
                ],
            )
        };
        assert!(content(-3.0, 1.0).validate().is_ok());
        assert!(content(1.0, -1.0).validate().is_err());
        // 비유한 값은 core-types 생성 단계에서 거부되어 입력에 들어올 수 없습니다.
        assert!(Length::new(f32::NAN).is_err());
        assert!(Insets::new(f32::INFINITY, 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn structure_validation_errors() {
        let leaf = |n: u64| LayoutNode::new(id(n));
        // 중복 node.
        let dup = LayoutInput::new(id(1), vec![leaf(1), leaf(1)]);
        assert_eq!(
            dup.validate(),
            Err(LayoutError::DuplicateNode { node: id(1) })
        );
        // root 없음.
        assert_eq!(
            LayoutInput::new(id(9), vec![leaf(1)]).validate(),
            Err(LayoutError::MissingRoot { root: id(9) })
        );
        // 없는 자식.
        let missing = LayoutInput::new(id(1), vec![leaf(1).with_child(id(2), key(1))]);
        assert_eq!(
            missing.validate(),
            Err(LayoutError::MissingChild {
                parent: id(1),
                child: id(2)
            })
        );
        // 두 부모.
        let two = LayoutInput::new(
            id(1),
            vec![
                leaf(1).with_child(id(2), key(1)).with_child(id(3), key(2)),
                leaf(2).with_child(id(4), key(1)),
                leaf(3).with_child(id(4), key(1)),
                leaf(4),
            ],
        );
        assert_eq!(
            two.validate(),
            Err(LayoutError::MultipleParents { node: id(4) })
        );
        // root가 자식 → 순환.
        let root_child = LayoutInput::new(
            id(1),
            vec![
                leaf(1).with_child(id(2), key(1)),
                leaf(2).with_child(id(1), key(1)),
            ],
        );
        assert_eq!(
            root_child.validate(),
            Err(LayoutError::Cycle { node: id(1) })
        );
        // root와 분리된 순환.
        let detached = LayoutInput::new(
            id(1),
            vec![
                leaf(1),
                leaf(2).with_child(id(3), key(1)),
                leaf(3).with_child(id(2), key(1)),
            ],
        );
        assert_eq!(detached.validate(), Err(LayoutError::Cycle { node: id(2) }));
        // 도달 불가.
        let orphan = LayoutInput::new(id(1), vec![leaf(1), leaf(2)]);
        assert_eq!(
            orphan.validate(),
            Err(LayoutError::Unreachable { node: id(2) })
        );
        // intrinsic leaf + 자식.
        let content_leaf = LayoutInput::new(
            id(1),
            vec![
                leaf(1)
                    .with_child(id(2), key(1))
                    .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                        width: len(1.0),
                        height: len(1.0),
                        baseline: None,
                    })),
                leaf(2),
            ],
        );
        assert_eq!(
            content_leaf.validate(),
            Err(LayoutError::ChildrenOnContentLeaf { node: id(1) })
        );
    }

    #[test]
    fn depth_limit_is_enforced_without_recursion() {
        let depth = MAX_LAYOUT_DEPTH + 1;
        let mut nodes = Vec::new();
        for n in 0..=depth as u64 {
            let mut node = LayoutNode::new(id(n + 1));
            if n < depth as u64 {
                node = node.with_child(id(n + 2), key(1));
            }
            nodes.push(node);
        }
        let input = LayoutInput::new(id(1), nodes.clone());
        assert_eq!(
            input.validate(),
            Err(LayoutError::DepthLimitExceeded {
                node: id(depth as u64 + 1),
                limit: MAX_LAYOUT_DEPTH
            })
        );
        nodes.pop();
        nodes.last_mut().unwrap().children.clear();
        assert!(LayoutInput::new(id(1), nodes).validate().is_ok());
    }

    #[test]
    fn visual_order_follows_order_key_not_insertion() {
        let input = sample_input();
        let tree = input.tree().unwrap();
        // 삽입 순서는 (3, 2)지만 key(1) < key(2)라 visual order는 (2, 3).
        let visual: Vec<StableId> = tree.children[tree.root]
            .iter()
            .map(|&i| input.nodes[i].id)
            .collect();
        assert_eq!(visual, vec![id(2), id(3)]);
        assert_eq!(tree.parent[tree.index[&id(2)]], Some(tree.root));
        assert!(input.node(id(3)).is_some());
        assert!(input.node(id(4)).is_none());
    }
}
