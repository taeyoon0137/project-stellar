//! 증분 frame compile 상태 (ADR-0090).
//!
//! [`FrameCompiler`]는 frame마다 [`compile_frame`](super::compile_frame)과 같은 [`FramePlan`]을 만들되, 앞 frame에서
//! 이미 계산한 결과를 다시 씁니다. scene 증분 build(ADR-0084 결정 3)는 바뀌지 않은 subtree를 같은 `Arc`로
//! 공유하므로, 같은 `Arc`는 같은 내용이라는 사실만으로 재사용을 판단합니다.
//!
//! ## 재사용하는 것
//!
//! - **draw segment**: 효과 node(합성 효과가 있는 표시 node)가 없는 subtree는 draw가 한 target에만 연속으로
//!   쌓입니다. 그 구간은 subtree 자신과 문맥(target 안 clip 시작 depth, 바깥 효과를 접은 opacity·blend)만으로
//!   정해지므로 `(subtree root 주소, 문맥)`을 key로 남깁니다. 다음 frame에서 같은 key를 만나면 subtree를 내려가지
//!   않고 구간을 그대로 붙입니다. 자식이 없는 node는 segment를 남기지 않습니다(다시 계산하는 비용이 같음).
//! - **효과 경계 결정**: 효과 node의 결정(XI.4.4)은 그 subtree만 보므로, 바깥 효과 node의 subtree 결정 목록을
//!   root 주소로 남깁니다.
//!
//! ## 무효화
//!
//! 재사용 결과는 viewport(크기·world → device 변환), 가시 후보 목록, 효과 경계 방식, clip capability가 앞
//! frame과 bit 단위로 같을 때만 씁니다. 하나라도 다르면 이번 frame은 전체 compile이고 다음 frame용 상태도
//! 남기지 않습니다. 문맥이 같은 다음 frame이 전체 compile하며 상태를 새로 만듭니다(Pan·Zoom은 frame마다 전체
//! compile).
//!
//! cache는 직전 frame에서 쓰거나 만든 항목만 남깁니다. 항목은 subtree `Arc`를 붙잡고 있어 주소가 다른 node에 다시
//! 쓰이지 않습니다. 그래서 frame이 이어져도 계속 늘지 않습니다. 기록하는 frame은 자식 있는 subtree마다 draw 구간을
//! 따로 복사하므로 그 frame의 상태는 draw 수 × subtree 깊이에 비례하고, 다음 재사용 frame에서 쓰이지 않은 안쪽
//! 구간이 정리됩니다.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use stellar_scene::stack::{Opacity, SceneBlendMode};
use stellar_scene::visibility::Viewport;
use stellar_scene::{SceneNode, ZOrder};

use super::{
    compile_with, Collected, CompileInput, EffectBoundaries, FrameCompileError, FramePlan,
};
use crate::clip_plan::ClipCapabilities;
use crate::offscreen::{plan_subtree_boundaries, EffectDecision};

/// 주소·작은 정수 key용 hasher(곱셈·회전). 외부 입력이 key가 아니므로 충돌 공격을 고려하지 않습니다.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AddrHasher(u64);

impl Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_u8(&mut self, value: u8) {
        self.write_u64(u64::from(value));
    }
}

/// [`AddrHasher`] builder.
pub(crate) type AddrBuild = BuildHasherDefault<AddrHasher>;

/// draw segment key: subtree root 주소와 target 문맥.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SegmentKey {
    node: usize,
    clip_depth: usize,
    opacity_bits: u32,
    blend: SceneBlendMode,
}

impl SegmentKey {
    /// `node` subtree를 `clip_depth`부터의 clip, 접힌 `opacity`·`blend` 문맥에서 모은 구간의 key.
    pub(crate) fn new(
        node: &Arc<SceneNode>,
        clip_depth: usize,
        opacity: Opacity,
        blend: SceneBlendMode,
    ) -> SegmentKey {
        SegmentKey {
            node: Arc::as_ptr(node) as usize,
            clip_depth,
            opacity_bits: opacity.get().to_bits(),
            blend,
        }
    }
}

/// 효과 node가 없는 subtree의 draw 구간.
#[derive(Debug, Clone)]
struct Segment {
    /// 주소 재사용을 막는 참조.
    node: Arc<SceneNode>,
    draws: Arc<[Collected]>,
    /// 마지막으로 쓰거나 만든 frame 번호.
    frame: u64,
}

/// 바깥 효과 node subtree의 경계 결정.
#[derive(Debug, Clone)]
struct SubtreeBoundaries {
    /// 주소 재사용을 막는 참조.
    node: Arc<SceneNode>,
    decisions: Arc<[(ZOrder, EffectDecision)]>,
    /// 마지막으로 쓰거나 만든 frame 번호.
    frame: u64,
}

/// 재사용 판단에 쓰는 frame 문맥(bit 단위 비교).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameContext {
    size: [u32; 2],
    view: [u32; 6],
    visible: Option<Vec<ZOrder>>,
    boundaries: EffectBoundaries,
    clip_caps: ClipCapabilities,
}

impl FrameContext {
    fn of(input: &CompileInput<'_>) -> FrameContext {
        FrameContext {
            size: viewport_size_bits(input.viewport),
            view: view_bits(input.viewport),
            visible: input.visible.map(|list| list.z_orders()),
            boundaries: input.boundaries,
            clip_caps: input.clip_caps,
        }
    }
}

fn viewport_size_bits(viewport: &Viewport) -> [u32; 2] {
    let size = viewport.size();
    [size.width().to_bits(), size.height().to_bits()]
}

fn view_bits(viewport: &Viewport) -> [u32; 6] {
    let (a, b, c, d, tx, ty) = viewport.view().components();
    [a, b, c, d, tx, ty].map(f32::to_bits)
}

/// 마지막 compile의 재사용 집계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CompileReuse {
    /// 들어가 처리한 node 수(재사용한 subtree의 root 포함, 그 자손 제외).
    pub visited_nodes: u64,
    /// 이전 frame 구간을 그대로 쓴 subtree 수.
    pub reused_subtrees: u64,
    /// 재사용한 subtree가 담은 draw 수.
    pub reused_draws: u64,
    /// 이전 frame 경계 결정을 그대로 쓴 효과 subtree 수.
    pub reused_boundary_plans: u64,
    /// frame 문맥(viewport·가시 후보·경계 방식·clip capability)이 직전 compile과 달라 재사용 없이 compile했으면
    /// 참. 이 frame은 다음 frame용 상태도 남기지 않습니다.
    pub context_changed: bool,
}

/// frame 하나를 compile하는 동안의 재사용 상태.
pub(crate) struct FrameWork<'c> {
    segments: &'c mut HashMap<SegmentKey, Segment, AddrBuild>,
    boundaries: &'c mut HashMap<usize, SubtreeBoundaries, AddrBuild>,
    frame: u64,
    record: bool,
    pub(crate) reuse: CompileReuse,
}

impl FrameWork<'_> {
    /// 다음 frame용 segment를 남기는지.
    pub(crate) fn records(&self) -> bool {
        self.record
    }

    /// 이전 frame의 같은 subtree·문맥 구간.
    pub(crate) fn reuse_segment(
        &mut self,
        key: &SegmentKey,
        node: &Arc<SceneNode>,
    ) -> Option<Arc<[Collected]>> {
        let segment = self.segments.get_mut(key)?;
        if !Arc::ptr_eq(&segment.node, node) {
            return None;
        }
        segment.frame = self.frame;
        self.reuse.reused_subtrees += 1;
        self.reuse.reused_draws += segment.draws.len() as u64;
        Some(Arc::clone(&segment.draws))
    }

    /// 효과 node가 없던 subtree의 구간을 다음 frame용으로 남깁니다.
    pub(crate) fn store_segment(
        &mut self,
        key: SegmentKey,
        node: &Arc<SceneNode>,
        draws: &[Collected],
    ) {
        if self.record {
            self.segments.insert(
                key,
                Segment {
                    node: Arc::clone(node),
                    draws: draws.into(),
                    frame: self.frame,
                },
            );
        }
    }

    /// 바깥 효과 node `node` subtree의 경계 결정(이전 frame과 같은 subtree면 재사용).
    pub(crate) fn subtree_boundaries(
        &mut self,
        node: &Arc<SceneNode>,
        viewport: &Viewport,
        drawn: &dyn Fn(&SceneNode) -> bool,
    ) -> Arc<[(ZOrder, EffectDecision)]> {
        let key = Arc::as_ptr(node) as usize;
        if let Some(previous) = self.boundaries.get_mut(&key) {
            if Arc::ptr_eq(&previous.node, node) {
                previous.frame = self.frame;
                self.reuse.reused_boundary_plans += 1;
                return Arc::clone(&previous.decisions);
            }
        }
        let decisions: Arc<[(ZOrder, EffectDecision)]> =
            plan_subtree_boundaries(node, viewport, drawn).into();
        if self.record {
            self.boundaries.insert(
                key,
                SubtreeBoundaries {
                    node: Arc::clone(node),
                    decisions: Arc::clone(&decisions),
                    frame: self.frame,
                },
            );
        }
        decisions
    }
}

/// frame마다 바뀐 scene 부분만 다시 compile하는 compiler (ADR-0090).
///
/// 결과는 같은 입력의 [`compile_frame`](super::compile_frame)과 항상 같습니다. 재사용은 계산을 줄일 뿐 plan을 바꾸지
/// 않습니다.
///
/// 다음 frame용 상태는 frame 문맥이 직전 compile과 같을 때만 남깁니다. Pan·Zoom처럼 frame마다 문맥이 바뀌는
/// 동안에는 남겨도 쓰이지 않기 때문입니다. 그래서 문맥이 바뀐 뒤 첫 frame과 그다음 frame은 전체 compile이고,
/// 세 번째 frame부터 재사용합니다.
#[derive(Debug, Default)]
pub struct FrameCompiler {
    context: Option<FrameContext>,
    segments: HashMap<SegmentKey, Segment, AddrBuild>,
    boundaries: HashMap<usize, SubtreeBoundaries, AddrBuild>,
    frame: u64,
    record: bool,
    reuse: CompileReuse,
}

impl FrameCompiler {
    /// 빈 compiler. 첫 compile은 전체 compile입니다.
    pub fn new() -> FrameCompiler {
        FrameCompiler {
            record: true,
            ..FrameCompiler::default()
        }
    }

    /// 다음 frame용 상태를 남기지 않는 compiler(한 번짜리 compile).
    pub(crate) fn one_shot() -> FrameCompiler {
        FrameCompiler::default()
    }

    /// frame을 compile합니다. 실패하면 재사용 상태를 비웁니다.
    pub fn compile(&mut self, input: &CompileInput<'_>) -> Result<FramePlan, FrameCompileError> {
        let context = FrameContext::of(input);
        let context_changed = self.context.as_ref() != Some(&context);
        if context_changed {
            self.segments.clear();
            self.boundaries.clear();
        }
        self.frame = self.frame.wrapping_add(1);
        let mut work = FrameWork {
            segments: &mut self.segments,
            boundaries: &mut self.boundaries,
            frame: self.frame,
            record: self.record && !context_changed,
            reuse: CompileReuse {
                context_changed,
                ..CompileReuse::default()
            },
        };
        let result = compile_with(input, &mut work);
        self.reuse = work.reuse;
        if result.is_ok() && self.record {
            // 이번 frame에서 쓰거나 만든 항목만 남깁니다.
            let frame = self.frame;
            self.segments.retain(|_, segment| segment.frame == frame);
            self.boundaries.retain(|_, plan| plan.frame == frame);
            self.context = Some(context);
        } else {
            self.reset();
        }
        result
    }

    /// 마지막 compile의 재사용 집계.
    pub fn last_reuse(&self) -> CompileReuse {
        self.reuse
    }

    /// 재사용 상태를 모두 버립니다(다음 compile은 전체 compile).
    pub fn reset(&mut self) {
        self.context = None;
        self.segments.clear();
        self.boundaries.clear();
    }
}

#[cfg(test)]
mod tests;
