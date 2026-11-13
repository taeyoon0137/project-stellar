//! draw item batching key와 순서 보존 batching (XI.4.5).
//!
//! **batch key = pipeline + texture/atlas + clip state + blend state**([`BatchKey`]).
//!
//! - **pipeline**([`PipelineKey`]): shader program(내용 종류: 단색·texture·glyph mask·layer 합성),
//!   고정 기능 blend(opaque는 덮어쓰기, 나머지는 premultiplied over), depth 상태(opaque pass test+write,
//!   transparent pass test, layer pass 없음), stencil 상태(stencil clip이면 test), target format·sample 수.
//! - **texture/atlas**([`TextureKey`]): 이미지 resource, glyph atlas, offscreen layer, 없음.
//! - **clip state**([`ClipKey`]): scissor 사각형, stencil reference·scissor, mask scissor.
//! - **blend state**: 합성 blend mode([`SceneBlendMode`]).
//!
//! ## 순서 보존
//!
//! batch는 pass draw 목록에서 **연속한** draw만 묶습니다. 떨어져 있는 같은 key의 draw를 앞으로 당겨 묶으면
//! 사이에 낀 draw와의 paint 순서(z-order)가 바뀌므로 하지 않습니다. depth test로 순서가 무관한 opaque
//! pass도 같은 규칙을 지켜 실행 순서가 언제나 compile 결과와 같습니다. 다음 draw는 key가 같아도 묶지
//! 않습니다.
//!
//! - Normal이 아닌 blend: backdrop을 읽으므로 batch 안 앞 draw의 결과를 봐야 합니다.
//! - stencil·mask clip: clip 도형이 같다는 것이 보장될 때(같은 clip stack 값)만 묶습니다. key의 scissor·
//!   reference가 같아도 clip 모양은 다를 수 있습니다.

use std::ops::Range;

use stellar_scene::stack::SceneBlendMode;
use stellar_scene::{SceneContent, SceneResourceRef};

use crate::backend::{BlendState, DepthMode, PipelineKey, ScissorRect, ShaderProgram, StencilMode};
use crate::clip_plan::ClipPlan;
use crate::pass_compiler::{
    DrawSource, FramePlan, LayerId, PassClass, PassPlan, PlannedDraw, PrimitiveClass, LAYER_FORMAT,
};
use crate::render_graph::{PassId, SampleCount, TextureFormat};

/// draw가 sampling하는 texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureKey {
    /// texture 없음(단색).
    None,
    /// scene resource(이미지 asset).
    Resource(SceneResourceRef),
    /// glyph atlas(page는 XI.6.5에서 key에 추가).
    GlyphAtlas,
    /// 같은 frame의 offscreen layer.
    Layer(LayerId),
}

/// draw의 clip 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipKey {
    /// clip 없음.
    None,
    /// scissor만.
    Scissor(ScissorRect),
    /// stencil test + scissor.
    Stencil {
        /// scissor.
        scissor: ScissorRect,
        /// stencil reference.
        reference: u8,
    },
    /// coverage mask + scissor.
    Mask {
        /// scissor.
        scissor: ScissorRect,
    },
}

impl ClipKey {
    /// clip plan의 key. `Culled`는 draw가 없으므로 `None`으로 둡니다.
    pub fn from_plan(plan: &ClipPlan) -> ClipKey {
        match plan {
            ClipPlan::None | ClipPlan::Culled => ClipKey::None,
            ClipPlan::Scissor(scissor) => ClipKey::Scissor(*scissor),
            ClipPlan::Stencil {
                scissor, reference, ..
            } => ClipKey::Stencil {
                scissor: *scissor,
                reference: *reference,
            },
            ClipPlan::Mask { scissor, .. } => ClipKey::Mask { scissor: *scissor },
        }
    }

    /// clip 도형 내용이 key에 다 담기지 않으면(stencil·mask) 참입니다.
    pub const fn depends_on_clip_shapes(&self) -> bool {
        matches!(self, ClipKey::Stencil { .. } | ClipKey::Mask { .. })
    }
}

/// batch key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatchKey {
    /// pipeline.
    pub pipeline: PipelineKey,
    /// texture/atlas.
    pub texture: TextureKey,
    /// clip state.
    pub clip: ClipKey,
    /// 합성 blend mode.
    pub blend: SceneBlendMode,
}

/// pass target 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetInfo {
    /// color format.
    pub format: TextureFormat,
    /// sample 수.
    pub samples: SampleCount,
}

/// pass 분류에 맞는 target(main은 surface, layer는 [`LAYER_FORMAT`]·1x).
pub const fn target_for(class: PassClass, surface: TargetInfo) -> TargetInfo {
    match class {
        PassClass::Opaque | PassClass::Transparent => surface,
        PassClass::Effect => TargetInfo {
            format: LAYER_FORMAT,
            samples: SampleCount::X1,
        },
    }
}

/// draw 하나의 batch key.
pub fn batch_key(draw: &PlannedDraw, class: PassClass, target: TargetInfo) -> BatchKey {
    let (program, texture) = match &draw.source {
        DrawSource::Layer { layer, .. } => {
            (ShaderProgram::LayerComposite, TextureKey::Layer(*layer))
        }
        DrawSource::Content(node) => match node.content() {
            SceneContent::Rect(_) | SceneContent::Path(_) => {
                (ShaderProgram::SolidFill, TextureKey::None)
            }
            SceneContent::Image(_) => (
                ShaderProgram::Texture,
                node.content()
                    .resource()
                    .map_or(TextureKey::None, TextureKey::Resource),
            ),
            SceneContent::Text(_) => (ShaderProgram::GlyphMask, TextureKey::GlyphAtlas),
        },
    };
    let blend = match draw.class {
        PrimitiveClass::Opaque => BlendState::Replace,
        PrimitiveClass::Transparent => BlendState::PremultipliedOver,
    };
    let depth = match class {
        PassClass::Opaque => DepthMode::TestAndWrite,
        PassClass::Transparent => DepthMode::TestOnly,
        PassClass::Effect => DepthMode::Disabled,
    };
    let clip = ClipKey::from_plan(&draw.clip);
    let stencil = match clip {
        ClipKey::Stencil { .. } => StencilMode::TestEqual,
        _ => StencilMode::Disabled,
    };
    // layer 합성은 경계 node 자기 blend가 우선이고, Normal이면 접힌 blend입니다.
    let composite_blend = match &draw.source {
        DrawSource::Layer { node, .. } if node.paint().get().blend != SceneBlendMode::Normal => {
            node.paint().get().blend
        }
        _ => draw.blend,
    };
    BatchKey {
        pipeline: PipelineKey {
            program,
            blend,
            depth,
            stencil,
            color_format: target.format,
            samples: target.samples,
        },
        texture,
        clip,
        blend: composite_blend,
    }
}

/// 연속한 같은 key draw 묶음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// key.
    pub key: BatchKey,
    /// pass draw index 범위(연속).
    pub draws: Range<usize>,
}

/// pass 하나의 batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassBatches {
    /// pass.
    pub pass: PassId,
    /// 실행 순서의 batch.
    pub batches: Vec<Batch>,
}

/// 앞 draw 뒤에 이어 붙일 수 있으면 참입니다(모듈 문서 규칙).
fn joinable(
    previous: &PlannedDraw,
    previous_key: &BatchKey,
    next: &PlannedDraw,
    key: &BatchKey,
) -> bool {
    if previous_key != key || key.blend != SceneBlendMode::Normal {
        return false;
    }
    if key.clip.depends_on_clip_shapes() {
        return previous.source.node().clip() == next.source.node().clip();
    }
    true
}

/// pass draw 목록을 순서를 바꾸지 않고 batch로 나눕니다.
pub fn batch_pass(pass: &PassPlan, surface: TargetInfo) -> Vec<Batch> {
    let target = target_for(pass.class, surface);
    let mut batches: Vec<Batch> = Vec::new();
    for (index, draw) in pass.draws.iter().enumerate() {
        let key = batch_key(draw, pass.class, target);
        if let Some(last) = batches.last_mut() {
            let previous = &pass.draws[last.draws.end - 1];
            if last.draws.end == index && joinable(previous, &last.key, draw, &key) {
                last.draws.end = index + 1;
                continue;
            }
        }
        batches.push(Batch {
            key,
            draws: index..index + 1,
        });
    }
    batches
}

/// frame의 모든 pass를 graph 실행 순서로 batch합니다.
pub fn batch_frame(plan: &FramePlan, surface: TargetInfo) -> Vec<PassBatches> {
    plan.passes_in_order()
        .map(|pass| PassBatches {
            pass: pass.pass,
            batches: batch_pass(pass, surface),
        })
        .collect()
}

/// batch 총수.
pub fn batch_count(batches: &[PassBatches]) -> usize {
    batches.iter().map(|pass| pass.batches.len()).sum()
}

#[cfg(test)]
mod tests;
