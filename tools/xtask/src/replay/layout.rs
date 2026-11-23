//! 재생 layout 단계: Free node를 absolute box로 옮긴 증분 layout (ADR-0084 결정 9).
//!
//! 문서 → layout 입력 연결은 아직 없습니다. 재생은 scene source의 Free node(Frame·Shape·Path·Text·Image)를
//! `position: absolute`, `left`·`top` = local transform 이동, `width`·`height` = layout 크기인 box로 옮겨
//! `IncrementalLayout`을 돌립니다. ADR-0076의 Free 대응(absolute + inset)과 같은 입력입니다. Fragment는 box를 만들지
//! 않고 자식을 가장 가까운 box 조상에 붙입니다. 확정 command가 바꾼 node만 upsert하고, 바뀐 box의 위치·크기를 scene
//! 무효화(`LayoutRect`)로 돌려줍니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{FiniteF32, Length, Transform2D};
use stellar_layout::incremental::{IncrementalLayout, LayoutEdit, UpdateStats};
use stellar_layout::input::{LayoutInput, LayoutNode};
use stellar_layout::intrinsic::DefaultMeasurer;
use stellar_layout::output::LayoutBox;
use stellar_layout::style::{LayoutStyle, LengthPercentageAuto, Position, Size};
use stellar_scene::dirty::SceneInvalidation;
use stellar_scene::source::{SceneKey, SceneSource, SourceKind};

/// 재생 layout 상태.
pub struct ReplayLayout {
    session: IncrementalLayout,
    /// box를 가진 key → 그 box 자식 key.
    children: HashMap<SceneKey, Vec<SceneKey>>,
}

/// source node 하나의 (local transform, 너비, 높이). Fragment는 `None`.
fn box_of(kind: &SourceKind) -> Option<(Transform2D, f32, f32)> {
    match kind {
        SourceKind::Frame(input) => Some((
            input.node.transform,
            input.size.width(),
            input.size.height(),
        )),
        SourceKind::Shape(input) => Some((
            input.node.transform,
            input.layout_rect.size().width(),
            input.layout_rect.size().height(),
        )),
        SourceKind::Path(input) => Some((
            input.node.transform,
            input.layout_rect.size().width(),
            input.layout_rect.size().height(),
        )),
        SourceKind::Text(input) => Some((
            input.node.transform,
            input.size.width(),
            input.size.height(),
        )),
        SourceKind::Image(input) => Some((
            input.node.transform,
            input.size.width(),
            input.size.height(),
        )),
        SourceKind::Fragment(_) => None,
    }
}

fn length(px: f32) -> Length {
    Length::new(px).unwrap_or(Length::from_raw(FiniteF32::ZERO))
}

/// `key` 아래 box 자식(Fragment를 펼침).
fn box_children(source: &SceneSource, key: SceneKey, out: &mut Vec<SceneKey>) {
    let Some(node) = source.get(key) else {
        return;
    };
    for child in &node.children {
        match source.get(*child).map(|n| &n.kind) {
            Some(SourceKind::Fragment(_)) => box_children(source, *child, out),
            Some(_) => out.push(*child),
            None => {}
        }
    }
}

fn layout_node(
    source: &SceneSource,
    key: SceneKey,
    root: bool,
    children: &[SceneKey],
) -> Option<LayoutNode> {
    let node = source.get(key)?;
    let (transform, width, height) = box_of(&node.kind)?;
    let (_, _, _, _, tx, ty) = transform.components();
    let mut style = LayoutStyle {
        width: Size::Length(length(width)),
        height: Size::Length(length(height)),
        ..LayoutStyle::default()
    };
    if !root {
        style.position = Position::Absolute;
        style.left = LengthPercentageAuto::Length(
            Length::new(tx).unwrap_or(Length::from_raw(FiniteF32::ZERO)),
        );
        style.top = LengthPercentageAuto::Length(
            Length::new(ty).unwrap_or(Length::from_raw(FiniteF32::ZERO)),
        );
    }
    let mut out = LayoutNode::new(key.id()).with_style(style);
    for (child, order) in children.iter().zip(OrderKey::evenly_spaced(children.len())) {
        out = out.with_child(child.id(), order);
    }
    Some(out)
}

impl ReplayLayout {
    /// source tree(첫 root)로 layout을 만듭니다.
    pub fn new(source: &SceneSource) -> Result<(ReplayLayout, UpdateStats), String> {
        let root = *source
            .roots()
            .first()
            .ok_or("scene source에 root가 없습니다")?;
        let mut nodes = Vec::new();
        let mut children = HashMap::new();
        let mut stack = vec![(root, true)];
        while let Some((key, is_root)) = stack.pop() {
            let mut kids = Vec::new();
            box_children(source, key, &mut kids);
            let node = layout_node(source, key, is_root, &kids).ok_or("root가 box가 아닙니다")?;
            nodes.push(node);
            stack.extend(kids.iter().map(|k| (*k, false)));
            children.insert(key, kids);
        }
        let input = LayoutInput::new(root.id(), nodes);
        let (session, stats) =
            IncrementalLayout::new(input, &DefaultMeasurer).map_err(|e| format!("layout: {e}"))?;
        Ok((ReplayLayout { session, children }, stats))
    }

    /// box.
    pub fn layout_box(&self, key: SceneKey) -> Option<LayoutBox> {
        self.session.layout_box(key.id())
    }

    /// 바뀐 key의 box를 다시 넣고 위치·크기가 바뀐 box를 scene 무효화로 돌려줍니다.
    pub fn apply(
        &mut self,
        source: &SceneSource,
        changed: &[SceneKey],
    ) -> Result<(UpdateStats, Vec<SceneInvalidation>), String> {
        let root = *source
            .roots()
            .first()
            .ok_or("scene source에 root가 없습니다")?;
        let mut edits = Vec::new();
        let mut before: Vec<(StableId, Option<LayoutBox>)> = Vec::new();
        for key in changed {
            if !self.children.contains_key(key)
                && source
                    .get(*key)
                    .is_some_and(|n| matches!(n.kind, SourceKind::Fragment(_)))
            {
                continue;
            }
            let mut kids = Vec::new();
            box_children(source, *key, &mut kids);
            let Some(node) = layout_node(source, *key, *key == root, &kids) else {
                continue;
            };
            before.push((key.id(), self.session.layout_box(key.id())));
            self.children.insert(*key, kids);
            edits.push(LayoutEdit::upsert(node));
        }
        if edits.is_empty() {
            return Ok((UpdateStats::default(), Vec::new()));
        }
        let stats = self
            .session
            .apply(edits, &DefaultMeasurer)
            .map_err(|e| format!("layout: {e}"))?;
        let mut invalidations = Vec::new();
        for (id, old) in before {
            let new = self.session.layout_box(id);
            let (moved, resized) = match (old, new) {
                (Some(a), Some(b)) => {
                    let (ra, rb) = (a.local_rect(), b.local_rect());
                    (ra.x() != rb.x() || ra.y() != rb.y(), ra.size() != rb.size())
                }
                (None, Some(_)) => (true, true),
                _ => (false, false),
            };
            invalidations.push(SceneInvalidation::LayoutRect {
                node: id,
                moved,
                resized,
            });
        }
        Ok((stats, invalidations))
    }
}
