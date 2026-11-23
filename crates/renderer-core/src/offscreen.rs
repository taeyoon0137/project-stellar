//! blend/opacity/filter 의존 분석과 최소 offscreen 경계 (XI.4.4).
//!
//! 합성 효과(opacity < 1, Normal이 아닌 blend, layer blur)가 있는 node는 의미상 자기 subtree를 따로
//! 합성한 뒤 그 결과에 효과를 겁니다. offscreen layer는 비싸므로, 결과가 같다는 것이 보장될 때는
//! layer를 만들지 않고 효과를 subtree의 draw에 **접어 넣습니다**(fold).
//!
//! ## 접기 조건
//!
//! 효과 node `N`의 subtree가 경계 결정 뒤 남기는 draw 단위(unit: 내용 draw 또는 안쪽 layer 합성 draw)
//! 목록을 아래에서 위로 계산합니다. 다음을 모두 만족하면 `N`을 접습니다(`Folded`).
//!
//! 1. `N`에 layer blur가 없습니다. blur는 subtree 전체 image가 있어야 계산됩니다(`Blur`).
//! 2. unit이 [`MAX_DISTRIBUTED_DRAWS`]개 이하입니다(`TooManyDraws`).
//! 3. 모든 unit이 **한 번에 합성되는 단일 draw**입니다. paint layer가 2개 이상이거나 shadow가 있는 내용,
//!    text(glyph끼리 겹칠 수 있음)는 draw 안에서 여러 번 칠하므로 opacity를 나눠 걸면 겹친 부분이
//!    달라집니다(`CompoundDraw`).
//! 4. 어떤 unit에도 이미 접힌 Normal 외 blend나 blend 경계 합성이 없습니다. `N`을 접으면 안쪽 blend의
//!    backdrop이 `N`의 빈 layer에서 바깥 target으로 바뀌기 때문입니다(`NestedBlend`).
//! 5. unit들의 device pixel 영역(render bounds를 pixel 경계로 바깥 정수화)이 서로 겹치지 않습니다.
//!    겹치지 않으면 각 pixel을 덮는 unit이 하나뿐이라 group 합성과 unit별 합성이 같습니다
//!    (`OverlappingDraws`).
//!
//! 접으면 `N`의 opacity는 unit opacity에 곱해지고, Normal이 아닌 blend는 unit이 그 blend로 바로
//! 합성됩니다. 접지 못하면 `N`은 layer 경계이며, 부모 쪽에는 `N` layer를 합성하는 단일 unit 하나가
//! 남습니다(그 합성이 Normal이 아닌 blend면 4번의 blend 표시를 가짐). subtree가 아무것도 그리지 않으면
//! layer도 없습니다(`NothingToDraw`).
//!
//! 예: `A(0.5) ⊃ B(0.5) ⊃ 사각형` → A·B 모두 접혀 사각형 하나를 opacity 0.25로 그립니다. 겹치는 두
//! 사각형을 담은 group은 경계입니다.
//!
//! 분석은 명시적 stack으로 tree를 순회하므로 재귀하지 않습니다.

use std::collections::HashMap;

use stellar_scene::node::Visibility;
use stellar_scene::stack::SceneBlendMode;
use stellar_scene::visibility::Viewport;
use stellar_scene::{SceneContent, SceneNode, SceneSnapshot, ZOrder};

use crate::clip_plan::PIXEL_EPSILON;
use crate::pass_compiler::{draws_content, has_compositing_effect};

/// 효과 하나를 여러 unit에 나눠 걸 수 있는 최대 unit 수.
pub const MAX_DISTRIBUTED_DRAWS: usize = 8;

/// layer 경계가 필요한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundaryReason {
    /// layer blur.
    Blur,
    /// unit이 너무 많음.
    TooManyDraws,
    /// 여러 번 칠하는 내용(paint layer 2개 이상·shadow·text).
    CompoundDraw,
    /// 안쪽에 접힌 blend 또는 blend 경계 합성이 있음.
    NestedBlend,
    /// unit의 pixel 영역이 겹침.
    OverlappingDraws,
}

/// 효과 node 하나의 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectDecision {
    /// layer 없이 효과를 draw에 접음.
    Folded,
    /// offscreen layer 경계.
    Boundary(BoundaryReason),
    /// subtree가 아무것도 그리지 않음(layer 없음).
    NothingToDraw,
}

/// 효과 node별 결정.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundaryPlan {
    decisions: HashMap<ZOrder, EffectDecision>,
}

impl BoundaryPlan {
    /// 결정들을 더합니다(같은 z면 나중 값).
    pub(crate) fn extend(&mut self, decisions: &[(ZOrder, EffectDecision)]) {
        self.decisions.extend(decisions.iter().copied());
    }

    /// 효과 node의 결정(효과 없는 node는 `None`).
    pub fn decision(&self, z: ZOrder) -> Option<EffectDecision> {
        self.decisions.get(&z).copied()
    }

    /// layer 경계면 참입니다.
    pub fn is_boundary(&self, z: ZOrder) -> bool {
        matches!(self.decision(z), Some(EffectDecision::Boundary(_)))
    }

    /// layer 경계 수.
    pub fn boundary_count(&self) -> usize {
        self.decisions
            .values()
            .filter(|decision| matches!(decision, EffectDecision::Boundary(_)))
            .count()
    }
}

/// 내용이 한 번에 합성되는 단일 draw면 참입니다(모듈 문서 3번).
pub fn draws_as_single_composite(node: &SceneNode) -> bool {
    let paint = node.paint().get();
    if !paint.shadows.is_empty() {
        return false;
    }
    match node.content() {
        SceneContent::Rect(_) | SceneContent::Path(_) => paint.layers.len() <= 1,
        SceneContent::Image(_) => paint.layers.is_empty(),
        SceneContent::Text(_) => false,
    }
}

/// device pixel 영역(반열린 구간).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelBox {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl PixelBox {
    fn overlaps(&self, other: &PixelBox) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    fn union(&self, other: &PixelBox) -> PixelBox {
        PixelBox {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

fn pixel_floor(value: f32) -> i64 {
    let snapped = if (value - value.round()).abs() <= PIXEL_EPSILON {
        value.round()
    } else {
        value.floor()
    };
    snapped.clamp(i64::MIN as f32, i64::MAX as f32) as i64
}

fn pixel_ceil(value: f32) -> i64 {
    let snapped = if (value - value.round()).abs() <= PIXEL_EPSILON {
        value.round()
    } else {
        value.ceil()
    };
    snapped.clamp(i64::MIN as f32, i64::MAX as f32) as i64
}

/// node 그리기 영역의 device pixel box. 변환이 넘치면 `None`(어디든 겹칠 수 있음).
fn pixel_box(node: &SceneNode, viewport: &Viewport) -> Option<PixelBox> {
    let device = node.world_render_bounds()?.transformed(&viewport.view())?;
    Some(PixelBox {
        x0: pixel_floor(device.min_x()),
        y0: pixel_floor(device.min_y()),
        x1: pixel_ceil(device.max_x()),
        y1: pixel_ceil(device.max_y()),
    })
}

/// 경계 결정 뒤 남는 draw 단위.
#[derive(Debug, Clone, Copy)]
struct Unit {
    pixels: Option<PixelBox>,
    single: bool,
    blend_sensitive: bool,
}

/// subtree의 unit 목록(상한 초과 시 요약만 유지).
#[derive(Debug, Clone, Default)]
struct Units {
    units: Vec<Unit>,
    overflow: bool,
    /// 모든 unit 영역의 합집합(`None` 원소가 있으면 `unknown`).
    union: Option<PixelBox>,
    unknown: bool,
    any_blend_sensitive: bool,
    count: usize,
}

impl Units {
    fn push(&mut self, unit: Unit) {
        self.count += 1;
        match unit.pixels {
            Some(pixels) => {
                self.union = Some(self.union.map_or(pixels, |acc| acc.union(&pixels)));
            }
            None => self.unknown = true,
        }
        self.any_blend_sensitive |= unit.blend_sensitive;
        if self.units.len() < MAX_DISTRIBUTED_DRAWS {
            self.units.push(unit);
        } else {
            self.overflow = true;
        }
    }

    fn append(&mut self, other: Units) {
        if other.overflow {
            self.overflow = true;
        }
        self.count += other.count.saturating_sub(other.units.len());
        if let Some(pixels) = other.union {
            self.union = Some(self.union.map_or(pixels, |acc| acc.union(&pixels)));
        }
        self.unknown |= other.unknown;
        self.any_blend_sensitive |= other.any_blend_sensitive;
        for unit in other.units {
            self.count += 1;
            if self.units.len() < MAX_DISTRIBUTED_DRAWS {
                self.units.push(unit);
            } else {
                self.overflow = true;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn pairwise_disjoint(&self) -> bool {
        if self.units.len() <= 1 {
            return true;
        }
        if self.unknown {
            return false;
        }
        for (index, a) in self.units.iter().enumerate() {
            for b in &self.units[index + 1..] {
                match (a.pixels, b.pixels) {
                    (Some(a), Some(b)) if !a.overlaps(&b) => {}
                    _ => return false,
                }
            }
        }
        true
    }

    /// layer 하나의 합성 unit으로 줄입니다.
    fn collapsed(&self, blend_sensitive: bool) -> Units {
        let mut out = Units::default();
        out.push(Unit {
            pixels: if self.unknown { None } else { self.union },
            single: true,
            blend_sensitive,
        });
        out
    }
}

fn decide(node: &SceneNode, units: &Units) -> EffectDecision {
    if units.is_empty() {
        return EffectDecision::NothingToDraw;
    }
    if node.paint().get().layer_blur.get() > 0.0 {
        return EffectDecision::Boundary(BoundaryReason::Blur);
    }
    if units.overflow {
        return EffectDecision::Boundary(BoundaryReason::TooManyDraws);
    }
    if units.units.iter().any(|unit| !unit.single) {
        return EffectDecision::Boundary(BoundaryReason::CompoundDraw);
    }
    if units.any_blend_sensitive {
        return EffectDecision::Boundary(BoundaryReason::NestedBlend);
    }
    if !units.pairwise_disjoint() {
        return EffectDecision::Boundary(BoundaryReason::OverlappingDraws);
    }
    EffectDecision::Folded
}

/// snapshot의 효과 node마다 접을지 layer 경계로 둘지 결정합니다.
///
/// `drawn`은 node 자기 내용을 그릴지(가시성·culling) 판단합니다. compiler와 같은 판정을 넘겨야
/// 결정이 실제 draw와 맞습니다. 결정은 효과 node의 subtree만 보므로 root마다 [`plan_subtree_boundaries`]를
/// 부른 결과와 같습니다.
pub fn plan_boundaries(
    snapshot: &SceneSnapshot,
    viewport: &Viewport,
    drawn: impl Fn(&SceneNode) -> bool,
) -> BoundaryPlan {
    let mut plan = BoundaryPlan::default();
    for root in snapshot.roots() {
        plan.decisions
            .extend(plan_subtree_boundaries(root, viewport, &drawn));
    }
    plan
}

/// `root` subtree 안 효과 node의 결정(역 pre-order, 자식이 부모보다 먼저).
///
/// 효과 node의 결정은 그 subtree의 unit만으로 정해지므로, 바깥 효과 node 하나의 subtree 결정은 snapshot
/// 전체 계획의 그 부분과 같습니다. 증분 compile([`crate::pass_compiler::FrameCompiler`])은 효과 node를 처음
/// 만날 때 이 함수로 그 subtree만 계획하고, 같은 node(같은 `Arc`)는 결과를 재사용합니다(ADR-0090).
pub fn plan_subtree_boundaries(
    root: &SceneNode,
    viewport: &Viewport,
    drawn: &dyn Fn(&SceneNode) -> bool,
) -> Vec<(ZOrder, EffectDecision)> {
    // pre-order 목록과 부모 index(재귀 없음).
    let mut order: Vec<(&SceneNode, Option<usize>)> = Vec::new();
    let mut stack: Vec<(&SceneNode, Option<usize>)> = vec![(root, None)];
    while let Some((node, parent)) = stack.pop() {
        let index = order.len();
        order.push((node, parent));
        let start = stack.len();
        stack.extend(node.children().map(|child| (&**child, Some(index))));
        stack[start..].reverse();
    }

    let mut pending: Vec<Units> = vec![Units::default(); order.len()];
    let mut decisions: Vec<(ZOrder, EffectDecision)> = Vec::new();
    // 역 pre-order: 자식이 부모보다 먼저 처리됩니다.
    for index in (0..order.len()).rev() {
        let (node, parent) = order[index];
        let mut units = Units::default();
        if drawn(node) && draws_content(node) {
            units.push(Unit {
                pixels: pixel_box(node, viewport),
                single: draws_as_single_composite(node),
                blend_sensitive: false,
            });
        }
        units.append(std::mem::take(&mut pending[index]));
        let shown = node.hit().visibility == Visibility::Visible;
        if shown && has_compositing_effect(node) {
            let decision = decide(node, &units);
            let blends = node.paint().get().blend != SceneBlendMode::Normal;
            units = match decision {
                EffectDecision::Folded => {
                    if blends {
                        for unit in &mut units.units {
                            unit.blend_sensitive = true;
                        }
                        units.any_blend_sensitive = true;
                    }
                    units
                }
                EffectDecision::Boundary(_) => units.collapsed(blends),
                EffectDecision::NothingToDraw => units,
            };
            decisions.push((node.z(), decision));
        }
        if let Some(parent) = parent {
            pending[parent].append(units);
        }
    }
    decisions
}

#[cfg(test)]
mod tests;
