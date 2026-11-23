//! scene primitive → opaque/transparent/effect pass compiler (XI.4.2).
//!
//! [`compile_frame`]는 불변 [`SceneSnapshot`] 하나를 frame [`FramePlan`](render graph + pass별 draw
//! 목록)으로 바꿉니다.
//!
//! ## 분류
//!
//! - **effect**: node와 subtree를 합성한 결과에 효과를 거는 node(합성 불투명도 < 1, Normal이 아닌
//!   blend, layer blur > 0)는 **offscreen layer** 경계 후보입니다. 경계면 node 자기 내용과 자손을
//!   layer texture에 painter 순서로 그리는 effect pass를 따로 만들고, 부모 쪽에는 그 layer를 node z
//!   위치에서 합성하는 draw 하나만 남깁니다. 기본값([`EffectBoundaries::Minimal`], XI.4.4)은 결과가
//!   같음이 보장되는 효과를 layer 없이 draw의 `opacity`·`blend`에 접고 필요한 경계만 둡니다
//!   ([`crate::offscreen`]). 접힌 효과가 걸린 draw는 opaque가 아닙니다.
//! - **opaque**: 그린 pixel이 모두 완전 불투명하고 가장자리 부분 coverage(AA)가 없는 primitive입니다.
//!   모서리 없는 사각형, 최상위 paint layer가 alpha 1 fill, stroke·shadow·합성 효과 없음, world→device
//!   변환이 축을 보존하고 사각형 네 변이 device pixel 경계에 정확히 놓임, clip은 scissor로 정확히
//!   표현되는 것뿐([`ClipAnalysis::is_scissor_exact`](crate::clip_plan::ClipAnalysis::is_scissor_exact))
//!   이어야 합니다.
//! - **transparent**: 나머지 전부(반투명 fill, 모서리·회전·소수 pixel 가장자리, stroke, shadow, text,
//!   image, layer 합성).
//!
//! ## 순서 (painter 의미 보존)
//!
//! main target(surface)은 depth buffer로 두 pass를 겹칩니다. draw마다 main target 안 paint 순서 순위
//! (`depth`, 1부터, 클수록 위)를 줍니다.
//!
//! 1. effect pass들(안쪽 layer 먼저): layer 안은 depth 없이 painter 순서(z 오름차순)로 그립니다.
//! 2. opaque pass: surface·depth를 clear하고 opaque draw를 **앞 → 뒤**(depth 내림차순)로 depth test
//!    (`depth > 저장값`) + write하며 그립니다. 각 pixel에 남는 것은 그 pixel을 덮는 가장 위 opaque draw입니다.
//! 3. transparent pass: surface를 이어 그리며 transparent draw(layer 합성 포함)를 **뒤 → 앞**(depth
//!    오름차순)으로 depth test만 하고(write 없음) blend합니다. 가장 위 opaque보다 아래 draw는 test에서
//!    빠지고 위 draw는 순서대로 쌓이므로 결과는 전체 painter 순서와 같습니다.
//!
//! ## clip (XI.4.3)
//!
//! draw마다 [`ClipPlan`](scissor/stencil/mask, [`crate::clip_plan`])을 붙입니다. 보이는 영역이 없는
//! (`Culled`) draw는 만들지 않습니다. layer 안 draw는 경계 node **안쪽** clip만 담고, 경계 node를
//! 자르는 바깥 clip은 layer 합성 draw가 담습니다(blur 같은 효과 뒤에 바깥 clip을 적용).
//!
//! snapshot은 명시적 stack으로 순회하므로 깊은 tree에서도 호출 stack을 쓰지 않습니다.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use stellar_scene::node::Visibility;
use stellar_scene::stack::{Opacity, Paint, PaintLayer, SceneBlendMode};
use stellar_scene::visibility::{DrawList, Viewport};
use stellar_scene::{SceneContent, SceneNode, SceneSnapshot, ZOrder};

use crate::clip_plan::{
    analyze_clip, axis_aligned_rect, decide_clip, surface_extent, ClipAnalysisMemo,
    ClipCapabilities, ClipPlan,
};
use crate::offscreen::BoundaryPlan;
use crate::render_graph::{
    ClearColor, CompiledRenderGraph, GraphBuildError, ImportKind, LoadOp, PassId, PassKind,
    RenderGraphBuilder, RenderGraphError, ResourceVersion, StoreOp, TextureDesc, TextureFormat,
};

/// offscreen layer texture format(선형 working space, premultiplied).
pub const LAYER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// main target depth·stencil format.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32FloatStencil8;

/// draw의 content 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveClass {
    /// 완전 불투명, 가장자리 AA 없음.
    Opaque,
    /// blend가 필요함.
    Transparent,
}

/// pass 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PassClass {
    /// main target opaque pass(앞 → 뒤, depth write).
    Opaque,
    /// main target transparent pass(뒤 → 앞, depth test).
    Transparent,
    /// offscreen layer pass(painter 순서).
    Effect,
}

/// offscreen layer 식별자(pre-order 시작 순서, [`FramePlan::layer`]로 조회). 그릴 것이 없어 만들지
/// 않은 layer의 번호는 비어 있을 수 있습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerId(pub u32);

/// draw가 그리는 것.
#[derive(Debug, Clone)]
pub enum DrawSource {
    /// node 자기 내용(paint layer·shadow·image·text).
    Content(Arc<SceneNode>),
    /// offscreen layer를 node 위치에 합성.
    Layer {
        /// 합성할 layer.
        layer: LayerId,
        /// layer 경계 node(합성 opacity·blend·blur 출처).
        node: Arc<SceneNode>,
    },
}

impl DrawSource {
    /// draw를 만든 node.
    pub fn node(&self) -> &Arc<SceneNode> {
        match self {
            DrawSource::Content(node) | DrawSource::Layer { node, .. } => node,
        }
    }
}

/// draw 하나.
#[derive(Debug, Clone)]
pub struct PlannedDraw {
    /// scene paint 순서 key.
    pub z: ZOrder,
    /// 같은 target 안 paint 순위(1부터, 클수록 위). main target에서는 depth 값입니다.
    pub depth: u32,
    /// content 분류.
    pub class: PrimitiveClass,
    /// 그리는 것.
    pub source: DrawSource,
    /// 접어 넣은 합성 불투명도(XI.4.4). 경계를 접지 않으면 1입니다.
    pub opacity: Opacity,
    /// 접어 넣은 blend(XI.4.4). layer 합성 draw는 경계 node 자기 blend가 Normal일 때만 이 값을 씁니다.
    pub blend: SceneBlendMode,
    /// clip 적용 방식(XI.4.3). layer 안 draw는 경계 node 안쪽 clip만 담습니다.
    pub clip: ClipPlan,
}

/// pass와 그 draw 목록.
#[derive(Debug, Clone)]
pub struct PassPlan {
    /// graph pass.
    pub pass: PassId,
    /// 분류.
    pub class: PassClass,
    /// effect pass면 그리는 layer.
    pub layer: Option<LayerId>,
    /// 실행 순서의 draw.
    pub draws: Vec<PlannedDraw>,
}

/// offscreen layer.
#[derive(Debug, Clone)]
pub struct EffectLayer {
    /// 식별자.
    pub id: LayerId,
    /// 경계 node.
    pub node: Arc<SceneNode>,
    /// 감싸는 layer(없으면 main target에 합성).
    pub parent: Option<LayerId>,
    /// 이 layer pass가 생산하는 texture version.
    pub output: ResourceVersion,
}

/// compile된 frame.
#[derive(Debug, Clone)]
pub struct FramePlan {
    /// 검증된 graph.
    pub graph: CompiledRenderGraph,
    /// pass별 draw(`PassId` 순서 = graph pass 순서).
    pub passes: Vec<PassPlan>,
    /// offscreen layer.
    pub layers: Vec<EffectLayer>,
    /// 효과 pass(layer blur, shadow 원본·blur, clip mask, 만든 순서, [`effects`]).
    pub effect_passes: Vec<effects::EffectPass>,
    /// draw별 효과와 합성 program(pass·depth 순).
    pub draw_effects: Vec<effects::DrawEffects>,
    /// layer별 효과(layer id 순).
    pub layer_effects: Vec<effects::LayerEffects>,
    /// 지원 목록 밖 blend mode 진단(`render.blend.unsupported_mode`, XI.10.2).
    pub blend_diagnostics: Vec<crate::blend::BlendDiagnostic>,
    /// 효과 근사 진단.
    pub effect_diagnostics: Vec<effects::EffectDiagnostic>,
}

impl FramePlan {
    /// `pass` 안 depth `depth` draw의 효과.
    pub fn draw_effects(&self, pass: PassId, depth: u32) -> Option<&effects::DrawEffects> {
        self.draw_effects
            .binary_search_by_key(&(pass, depth), |effects| (effects.pass, effects.depth))
            .ok()
            .map(|index| &self.draw_effects[index])
    }

    /// layer 효과.
    pub fn layer_effects(&self, layer: LayerId) -> Option<&effects::LayerEffects> {
        self.layer_effects
            .binary_search_by_key(&layer, |effects| effects.layer)
            .ok()
            .map(|index| &self.layer_effects[index])
    }

    /// 효과 pass.
    pub fn effect_pass(&self, pass: PassId) -> Option<&effects::EffectPass> {
        self.effect_passes.iter().find(|effect| effect.pass == pass)
    }

    /// graph 실행 순서의 pass.
    pub fn passes_in_order(&self) -> impl Iterator<Item = &PassPlan> + '_ {
        self.graph
            .order()
            .iter()
            .filter_map(|pass| self.passes.iter().find(|plan| plan.pass == *pass))
    }

    /// 전체 draw 수.
    pub fn draw_count(&self) -> usize {
        self.passes.iter().map(|pass| pass.draws.len()).sum()
    }

    /// layer.
    pub fn layer(&self, id: LayerId) -> Option<&EffectLayer> {
        self.layers
            .binary_search_by_key(&id, |layer| layer.id)
            .ok()
            .map(|index| &self.layers[index])
    }
}

/// compile 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCompileError {
    /// 한 target의 draw 수가 `u32`를 넘었습니다.
    TooManyDraws,
    /// graph 구성 실패(compiler 버그).
    Build(GraphBuildError),
    /// graph 검증 실패(compiler 버그).
    Graph(RenderGraphError),
}

impl fmt::Display for FrameCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameCompileError::TooManyDraws => f.write_str("draw 수가 상한을 넘었습니다"),
            FrameCompileError::Build(error) => write!(f, "render graph 구성 실패: {error}"),
            FrameCompileError::Graph(error) => write!(f, "render graph 검증 실패: {error}"),
        }
    }
}

impl std::error::Error for FrameCompileError {}

/// effect 경계 결정 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EffectBoundaries {
    /// 합성 효과가 있는 모든 node를 layer 경계로 둡니다.
    Conservative,
    /// 결과가 같음이 보장되면 효과를 draw에 접고 필요한 최소 경계만 둡니다(XI.4.4,
    /// [`crate::offscreen`]).
    #[default]
    Minimal,
}

/// compile 입력.
#[derive(Debug, Clone, Copy)]
pub struct CompileInput<'a> {
    /// scene.
    pub snapshot: &'a SceneSnapshot,
    /// 화면 크기와 world → device 변환.
    pub viewport: &'a Viewport,
    /// 가시 후보(XI.3.2). `None`이면 숨김이 아닌 모든 node를 그립니다.
    pub visible: Option<&'a DrawList>,
    /// surface format.
    pub surface_format: TextureFormat,
    /// surface clear 색.
    pub clear: ClearColor,
    /// effect 경계 결정 방식.
    pub boundaries: EffectBoundaries,
    /// clip 결정 capability(XI.4.3).
    pub clip_caps: ClipCapabilities,
}

/// node가 합성 효과(layer 경계 후보)를 가지면 참입니다.
pub fn has_compositing_effect(node: &SceneNode) -> bool {
    let paint = node.paint().get();
    paint.opacity < Opacity::OPAQUE
        || paint.blend != SceneBlendMode::Normal
        || paint.layer_blur.get() > 0.0
}

/// node 자기 내용이 무언가를 그리면 참입니다(text·image는 고유 모습이 있어 항상 그림).
pub fn draws_content(node: &SceneNode) -> bool {
    match node.content() {
        SceneContent::Text(_) | SceneContent::Image(_) => true,
        SceneContent::Rect(_) | SceneContent::Path(_) => !node.paint().get().paints_nothing(),
    }
}

/// node 자기 내용의 분류(모듈 문서 규칙). `clip_from_depth`부터의 clip만 봅니다(XI.4.3).
pub fn classify_content(
    node: &SceneNode,
    viewport: &Viewport,
    clip_from_depth: usize,
) -> PrimitiveClass {
    classify_with_clip(
        node,
        viewport,
        analyze_clip(node.clip(), clip_from_depth, viewport).is_scissor_exact(),
    )
}

/// [`classify_content`]의 본문. clip 분석 결과(`scissor_exact`)는 호출자가 이미 계산한 값을 받습니다.
fn classify_with_clip(
    node: &SceneNode,
    viewport: &Viewport,
    scissor_exact: bool,
) -> PrimitiveClass {
    let SceneContent::Rect(primitive) = node.content() else {
        return PrimitiveClass::Transparent;
    };
    let device_rect_aligned = node
        .world_transform()
        .then(&viewport.view())
        .ok()
        .and_then(|to_device| {
            axis_aligned_rect(
                (
                    primitive.rect.origin().x(),
                    primitive.rect.origin().y(),
                    primitive.rect.size().width(),
                    primitive.rect.size().height(),
                ),
                to_device.components(),
            )
        })
        .is_some_and(|rect| rect.is_pixel_aligned());
    let paint = node.paint().get();
    let top_is_opaque_fill = matches!(
        paint.layers.last(),
        Some(PaintLayer::Fill(Paint::Solid(color))) if color.alpha() >= 1.0
    );
    let no_strokes = paint
        .layers
        .iter()
        .all(|layer| matches!(layer, PaintLayer::Fill(_)));
    let opaque = top_is_opaque_fill
        && no_strokes
        && paint.shadows.is_empty()
        && !has_compositing_effect(node)
        && primitive.radii.is_zero()
        && device_rect_aligned
        && scissor_exact;
    if opaque {
        PrimitiveClass::Opaque
    } else {
        PrimitiveClass::Transparent
    }
}

/// layer 경계로 둘지 결정합니다.
fn is_boundary(node: &SceneNode, plan: Option<&BoundaryPlan>) -> bool {
    match plan {
        None => has_compositing_effect(node),
        Some(plan) => plan.is_boundary(node.z()),
    }
}

/// layer 없이 draw에 접은 효과(XI.4.4).
#[derive(Debug, Clone, Copy)]
struct Fold {
    last_z: ZOrder,
    opacity: f32,
    blend: SceneBlendMode,
}

/// scope 안 접힌 효과의 합(opacity 곱, Normal이 아닌 blend). 분석은 한 scope 안 경로에 Normal 외
/// blend 접기를 하나만 허용합니다.
fn folded(folds: &[Fold]) -> (Opacity, SceneBlendMode) {
    let product = folds.iter().map(|fold| fold.opacity).product::<f32>();
    // [0, 1] 값의 곱이라 항상 범위 안입니다.
    let opacity = Opacity::new(product).unwrap_or(Opacity::OPAQUE);
    let blend = folds
        .iter()
        .map(|fold| fold.blend)
        .find(|blend| *blend != SceneBlendMode::Normal)
        .unwrap_or_default();
    (opacity, blend)
}

/// 순위가 정해지기 전 draw.
#[derive(Debug, Clone)]
struct Collected {
    z: ZOrder,
    class: PrimitiveClass,
    source: DrawSource,
    clip: ClipPlan,
    opacity: Opacity,
    blend: SceneBlendMode,
}

/// 수집 중인 target(main 또는 layer)의 draw.
struct Scope {
    layer: Option<LayerId>,
    /// 경계 node(main이면 None)와 그 subtree 마지막 z.
    boundary: Option<(Arc<SceneNode>, ZOrder)>,
    /// 이 target 안 draw가 고려할 clip 시작 depth(경계 node 바깥 clip은 합성 단계에서 적용).
    clip_depth: usize,
    /// 이 target 안 현재 경로의 접힌 효과(바깥 → 안).
    folds: Vec<Fold>,
    draws: Vec<Collected>,
}

/// 수집 결과 layer(아직 graph 없음).
struct LayerDraft {
    node: Arc<SceneNode>,
    parent: Option<LayerId>,
    draws: Vec<Collected>,
}

fn ranked(draws: Vec<Collected>) -> Result<Vec<PlannedDraw>, FrameCompileError> {
    draws
        .into_iter()
        .enumerate()
        .map(|(index, draw)| {
            let depth = u32::try_from(index + 1).map_err(|_| FrameCompileError::TooManyDraws)?;
            Ok(PlannedDraw {
                z: draw.z,
                depth,
                class: draw.class,
                source: draw.source,
                opacity: draw.opacity,
                blend: draw.blend,
                clip: draw.clip,
            })
        })
        .collect()
}

/// snapshot을 frame plan으로 compile합니다.
///
/// 이전 frame을 기억하지 않는 한 번짜리 compile입니다. frame마다 compile하는 쪽은 바뀐 subtree만 다시 보는
/// [`FrameCompiler`]를 씁니다. 두 경로는 같은 함수(`compile_with`)를 실행하고 결과가 같습니다(ADR-0090).
pub fn compile_frame(input: &CompileInput<'_>) -> Result<FramePlan, FrameCompileError> {
    FrameCompiler::one_shot().compile(input)
}

/// 한 compile 동안의 clip memo. key는 (clip stack 저장소 주소, 고려 시작 depth)이고, snapshot이 stack을 붙잡고
/// 있는 동안만 씁니다.
#[derive(Default)]
struct ClipMemo {
    /// clip 결정과 scissor 정확 여부.
    plans: HashMap<(usize, usize), (ClipPlan, bool), AddrBuild>,
    /// stack prefix 분석.
    analyses: ClipAnalysisMemo,
    /// 마지막 조회(연속한 형제는 같은 stack을 가짐).
    last: Option<((usize, usize), (ClipPlan, bool))>,
}

/// compile 한 번의 고정 입력.
struct FrameEnv<'i> {
    viewport: &'i Viewport,
    surface_width: u32,
    surface_height: u32,
    clip_caps: ClipCapabilities,
}

impl FrameEnv<'_> {
    /// `node`의 clip을 `from_depth`부터 계획하고, scissor로 정확한지도 돌려줍니다. 같은 clip stack(구조 공유)과
    /// 시작 depth는 결과가 같으므로 memo합니다.
    fn clip(&self, memo: &mut ClipMemo, node: &SceneNode, from_depth: usize) -> (ClipPlan, bool) {
        if node.clip().depth() <= from_depth {
            // 고려할 clip이 없습니다(decision table 첫 행).
            return (ClipPlan::None, true);
        }
        let key = (node.clip().storage_addr(), from_depth);
        if let Some((last, value)) = &memo.last {
            if *last == key {
                return value.clone();
            }
        }
        if let Some(hit) = memo.plans.get(&key) {
            memo.last = Some((key, hit.clone()));
            return hit.clone();
        }
        let analysis = memo
            .analyses
            .analyze(node.clip(), from_depth, self.viewport);
        let value = (
            decide_clip(
                &analysis,
                self.surface_width,
                self.surface_height,
                self.clip_caps,
            ),
            analysis.is_scissor_exact(),
        );
        memo.plans.insert(key, value.clone());
        memo.last = Some((key, value.clone()));
        value
    }
}

/// 끝난 layer scope를 닫고 부모 target에 합성 draw를 남깁니다.
fn close_scope(
    scopes: &mut Vec<Scope>,
    layers: &mut [LayerDraft],
    env: &FrameEnv<'_>,
    memo: &mut ClipMemo,
) {
    let Some(scope) = scopes.pop() else {
        return;
    };
    let (Some(layer), Some((node, _))) = (scope.layer, scope.boundary) else {
        return;
    };
    let Some(parent) = scopes.last_mut() else {
        return;
    };
    // 경계 node 바깥 clip은 합성 draw에 적용합니다. 보이는 곳이 없으면 layer도 만들지 않습니다.
    let (clip, _) = env.clip(memo, &node, parent.clip_depth);
    if scope.draws.is_empty() || clip == ClipPlan::Culled {
        // 그릴 것이 없는 layer는 만들지 않습니다(id는 비워 둠).
        layers[layer.0 as usize].draws = Vec::new();
        return;
    }
    layers[layer.0 as usize].draws = scope.draws;
    // 경계 node를 감싸는 접힌 효과(부모 target 쪽)는 합성 draw에 접습니다.
    let (opacity, blend) = folded(&parent.folds);
    parent.draws.push(Collected {
        z: node.z(),
        class: PrimitiveClass::Transparent,
        source: DrawSource::Layer {
            layer,
            node: Arc::clone(&node),
        },
        clip,
        opacity,
        blend,
    });
}

/// pre-order 순회 단계.
enum Visit<'a> {
    /// node에 들어감.
    Enter(&'a Arc<SceneNode>),
    /// 자식이 있는 node의 subtree가 끝남. subtree가 효과 node 없이 한 target에만 draw를 남겼으면 그 구간을
    /// 다음 frame용 segment로 남깁니다.
    Exit {
        node: &'a Arc<SceneNode>,
        key: cache::SegmentKey,
        start: usize,
        effects: usize,
    },
}

/// frame 하나를 compile합니다. `work`가 이전 frame의 재사용 가능한 결과를 들고 있으며, 비어 있으면 전체 compile과
/// 같습니다.
fn compile_with(
    input: &CompileInput<'_>,
    work: &mut cache::FrameWork<'_>,
) -> Result<FramePlan, FrameCompileError> {
    let visible: Option<HashSet<ZOrder>> = input
        .visible
        .map(|list| list.items.iter().map(|item| item.z).collect());
    let is_visible = |node: &SceneNode| {
        node.hit().visibility == Visibility::Visible
            && visible.as_ref().is_none_or(|set| set.contains(&node.z()))
    };

    // 0. 효과 경계 결정(XI.4.4): 접을 수 있는 효과는 layer 없이 draw에 접습니다. 결정은 효과 node subtree만
    //    보므로 바깥 효과 node를 처음 만날 때 그 subtree만 계획합니다(offscreen::plan_subtree_boundaries).
    let minimal = input.boundaries == EffectBoundaries::Minimal;
    let mut boundary_plan = BoundaryPlan::default();

    // 1. pre-order 순회로 draw를 target별로 모읍니다.
    let (surface_width, surface_height) = surface_extent(input.viewport);
    let env = FrameEnv {
        viewport: input.viewport,
        surface_width,
        surface_height,
        clip_caps: input.clip_caps,
    };
    let mut memo = ClipMemo::default();
    let mut layers: Vec<LayerDraft> = Vec::new();
    let mut scopes: Vec<Scope> = vec![Scope {
        layer: None,
        boundary: None,
        clip_depth: 0,
        folds: Vec::new(),
        draws: Vec::new(),
    }];
    // 지금까지 들어간 효과 node 수. subtree 앞뒤 값이 같으면 그 subtree에는 효과 node가 없습니다.
    let mut effects_seen = 0usize;

    let mut stack: Vec<Visit<'_>> = input.snapshot.roots().map(Visit::Enter).collect();
    stack.reverse();
    while let Some(visit) = stack.pop() {
        let node = match visit {
            Visit::Enter(node) => node,
            Visit::Exit {
                node,
                key,
                start,
                effects,
            } => {
                if effects == effects_seen {
                    if let Some(scope) = scopes.last() {
                        work.store_segment(key, node, &scope.draws[start..]);
                    }
                }
                continue;
            }
        };
        work.reuse.visited_nodes += 1;
        // 이 node 앞에서 끝난 layer scope를 닫습니다(subtree 마지막 z < 현재 z).
        while scopes
            .last()
            .and_then(|scope| scope.boundary.as_ref())
            .is_some_and(|(_, last_z)| *last_z < node.z())
        {
            close_scope(&mut scopes, &mut layers, &env, &mut memo);
        }
        // 현재 target에서 끝난 접힌 효과를 뺍니다.
        if let Some(scope) = scopes.last_mut() {
            while scope
                .folds
                .last()
                .is_some_and(|fold| fold.last_z < node.z())
            {
                scope.folds.pop();
            }
        }
        // 이전 frame과 같은 subtree(같은 `Arc`)를 같은 target 문맥(clip 시작 depth, 접힌 효과)에서 다시 만나면
        // 이전 draw 구간을 그대로 씁니다. 효과 node가 없는 subtree만 segment가 있습니다.
        let segment_key = (work.records() && node.child_count() > 0)
            .then(|| scopes.last())
            .flatten()
            .map(|scope| {
                let (opacity, blend) = folded(&scope.folds);
                cache::SegmentKey::new(node, scope.clip_depth, opacity, blend)
            });
        if let Some(key) = &segment_key {
            if let Some(draws) = work.reuse_segment(key, node) {
                if let Some(scope) = scopes.last_mut() {
                    scope.draws.extend(draws.iter().cloned());
                }
                continue;
            }
        }
        let start = scopes.last().map_or(0, |scope| scope.draws.len());
        let effects_before = effects_seen;
        let shown = node.hit().visibility == Visibility::Visible;
        let effect = shown && has_compositing_effect(node);
        if effect {
            effects_seen += 1;
            if minimal && boundary_plan.decision(node.z()).is_none() {
                let decisions = work.subtree_boundaries(node, input.viewport, &is_visible);
                boundary_plan.extend(&decisions);
            }
        }
        if effect && is_boundary(node, minimal.then_some(&boundary_plan)) {
            let id = LayerId(layers.len() as u32);
            layers.push(LayerDraft {
                node: Arc::clone(node),
                parent: scopes.last().and_then(|scope| scope.layer),
                draws: Vec::new(),
            });
            scopes.push(Scope {
                layer: Some(id),
                boundary: Some((Arc::clone(node), node.last_z())),
                clip_depth: node.clip().depth(),
                folds: Vec::new(),
                draws: Vec::new(),
            });
        } else if effect {
            // 접힌 효과: subtree draw에 opacity·blend를 곱해 넣습니다.
            let paint = node.paint().get();
            if let Some(scope) = scopes.last_mut() {
                scope.folds.push(Fold {
                    last_z: node.last_z(),
                    opacity: paint.opacity.get(),
                    blend: paint.blend,
                });
            }
        }
        if is_visible(node) && draws_content(node) {
            if let Some(scope) = scopes.last_mut() {
                let (clip, scissor_exact) = env.clip(&mut memo, node, scope.clip_depth);
                if clip != ClipPlan::Culled {
                    let (opacity, blend) = folded(&scope.folds);
                    // 접힌 효과가 걸린 draw는 불투명 덮어쓰기가 아닙니다.
                    let class = if opacity < Opacity::OPAQUE || blend != SceneBlendMode::Normal {
                        PrimitiveClass::Transparent
                    } else {
                        classify_with_clip(node, input.viewport, scissor_exact)
                    };
                    scope.draws.push(Collected {
                        z: node.z(),
                        class,
                        source: DrawSource::Content(Arc::clone(node)),
                        clip,
                        opacity,
                        blend,
                    });
                }
            }
        }
        if let Some(key) = segment_key {
            stack.push(Visit::Exit {
                node,
                key,
                start,
                effects: effects_before,
            });
        }
        let first = stack.len();
        stack.extend(node.children().map(Visit::Enter));
        stack[first..].reverse();
    }
    while scopes.len() > 1 {
        close_scope(&mut scopes, &mut layers, &env, &mut memo);
    }
    let main = scopes.pop().map(|scope| scope.draws).unwrap_or_default();

    // 2. graph: layer pass(안쪽 먼저) → opaque → transparent.
    let mut builder = RenderGraphBuilder::new();
    let surface = builder.import(
        "surface",
        TextureDesc::surface(input.surface_format),
        ImportKind::Surface,
    );
    let depth = builder.transient("main depth", TextureDesc::surface(DEPTH_FORMAT));
    let mut passes: Vec<PassPlan> = Vec::new();
    let mut outputs: Vec<Option<ResourceVersion>> = vec![None; layers.len()];
    let mut effect_layers: Vec<EffectLayer> = Vec::new();
    let graph_error = FrameCompileError::Build;
    let mut effect_builder = effects::EffectBuilder::new(input.viewport);

    // layer는 시작 순서(pre-order)로 id가 붙었으므로 역순이면 자식 layer가 부모보다 먼저입니다.
    for index in (0..layers.len()).rev() {
        if layers[index].draws.is_empty() {
            continue;
        }
        let draft = &layers[index];
        let texture = builder.transient("effect layer", TextureDesc::surface(LAYER_FORMAT));
        let pass = builder.pass("effect layer", PassKind::Raster);
        let output = builder
            .color(
                pass,
                texture,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .map_err(graph_error)?;
        let draws = ranked(draft.draws.clone())?;
        for draw in &draws {
            if let DrawSource::Layer { layer, .. } = draw.source {
                if let Some(child) = outputs[layer.0 as usize] {
                    builder.read(pass, child).map_err(graph_error)?;
                }
            }
        }
        effect_builder
            .draws(&mut builder, pass, &draws)
            .map_err(graph_error)?;
        // layer blur가 있으면 부모는 blur 결과를 합성합니다(ADR-0116 결정 3).
        outputs[index] = Some(
            effect_builder
                .layer(&mut builder, LayerId(index as u32), &draft.node, output)
                .map_err(graph_error)?,
        );
        passes.push(PassPlan {
            pass,
            class: PassClass::Effect,
            layer: Some(LayerId(index as u32)),
            draws,
        });
        effect_layers.push(EffectLayer {
            id: LayerId(index as u32),
            node: Arc::clone(&draft.node),
            parent: draft.parent,
            output,
        });
    }
    effect_layers.sort_by_key(|layer| layer.id);

    let main = ranked(main)?;
    let (mut opaque, mut transparent): (Vec<PlannedDraw>, Vec<PlannedDraw>) = main
        .into_iter()
        .partition(|draw| draw.class == PrimitiveClass::Opaque);
    opaque.sort_by_key(|draw| std::cmp::Reverse(draw.depth));
    transparent.sort_by_key(|draw| draw.depth);

    let opaque_pass = builder.pass("opaque", PassKind::Raster);
    let surface_after_opaque = builder
        .color(
            opaque_pass,
            surface,
            LoadOp::Clear,
            input.clear,
            StoreOp::Store,
        )
        .map_err(graph_error)?;
    let depth_after_opaque = builder
        .depth_stencil(opaque_pass, depth, LoadOp::Clear, StoreOp::Store)
        .map_err(graph_error)?;
    effect_builder
        .draws(&mut builder, opaque_pass, &opaque)
        .map_err(graph_error)?;
    passes.push(PassPlan {
        pass: opaque_pass,
        class: PassClass::Opaque,
        layer: None,
        draws: opaque,
    });
    let mut output = surface_after_opaque;
    if !transparent.is_empty() {
        let transparent_pass = builder.pass("transparent", PassKind::Raster);
        output = builder
            .color(
                transparent_pass,
                surface_after_opaque,
                LoadOp::Load,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .map_err(graph_error)?;
        builder
            .depth_stencil(
                transparent_pass,
                depth_after_opaque,
                LoadOp::Load,
                StoreOp::Discard,
            )
            .map_err(graph_error)?;
        for draw in &transparent {
            if let DrawSource::Layer { layer, .. } = draw.source {
                if let Some(version) = outputs[layer.0 as usize] {
                    builder
                        .read(transparent_pass, version)
                        .map_err(graph_error)?;
                }
            }
        }
        effect_builder
            .draws(&mut builder, transparent_pass, &transparent)
            .map_err(graph_error)?;
        passes.push(PassPlan {
            pass: transparent_pass,
            class: PassClass::Transparent,
            layer: None,
            draws: transparent,
        });
    }
    builder.set_output(output);
    let graph = builder
        .finish()
        .compile()
        .map_err(FrameCompileError::Graph)?;
    passes.sort_by_key(|plan| plan.pass);
    let effects::EffectBuilder {
        passes: effect_passes,
        draws: mut draw_effects,
        layers: mut layer_effects,
        blend_diagnostics,
        diagnostics: effect_diagnostics,
        ..
    } = effect_builder;
    draw_effects.sort_by_key(|effects| (effects.pass, effects.depth));
    layer_effects.sort_by_key(|effects| effects.layer);
    Ok(FramePlan {
        graph,
        passes,
        layers: effect_layers,
        effect_passes,
        draw_effects,
        layer_effects,
        blend_diagnostics,
        effect_diagnostics,
    })
}

/// 효과 pass(layer blur, drop shadow, clip mask)와 합성 program (ADR-0116 결정 3).
pub mod effects;

/// 증분 frame compile 상태(ADR-0090).
mod cache;

pub(crate) use cache::AddrBuild;
pub use cache::{CompileReuse, FrameCompiler};

#[cfg(test)]
pub(crate) mod tests;
