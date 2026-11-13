//! scene primitive → opaque/transparent/effect pass compiler (XI.4.2).
//!
//! [`compile_frame`]는 불변 [`SceneSnapshot`] 하나를 frame [`FramePlan`](render graph + pass별 draw
//! 목록)으로 바꿉니다.
//!
//! ## 분류
//!
//! - **effect**: node와 subtree를 합성한 결과에 효과를 거는 node(합성 불투명도 < 1, Normal이 아닌
//!   blend, layer blur > 0)는 **offscreen layer** 경계입니다. node 자기 내용과 자손을 layer texture에
//!   painter 순서로 그리는 effect pass를 따로 만들고, 부모 쪽에는 그 layer를 node z 위치에서 합성하는
//!   draw 하나만 남깁니다. layer 경계의 최소화는 XI.4.4가 담당하며, 이 compiler는 결정된 경계를
//!   그대로 씁니다([`EffectBoundaries`]).
//! - **opaque**: 그린 pixel이 모두 완전 불투명하고 가장자리 부분 coverage(AA)가 없는 primitive입니다.
//!   모서리 없는 사각형, 최상위 paint layer가 alpha 1 fill, stroke·shadow·합성 효과 없음, world→device
//!   변환이 축 정렬이고 사각형 네 변이 device pixel 경계에 정확히 놓임, clip은 같은 조건의 사각형
//!   뿐이어야 합니다.
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
//! snapshot은 명시적 stack으로 순회하므로 깊은 tree에서도 호출 stack을 쓰지 않습니다.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use stellar_scene::node::Visibility;
use stellar_scene::stack::{ClipShape, Opacity, Paint, PaintLayer, SceneBlendMode};
use stellar_scene::visibility::{DrawList, Viewport};
use stellar_scene::{SceneContent, SceneNode, SceneSnapshot, ZOrder};

use crate::render_graph::{
    ClearColor, CompiledRenderGraph, GraphBuildError, ImportKind, LoadOp, PassId, PassKind,
    RenderGraphBuilder, RenderGraphError, ResourceVersion, StoreOp, TextureDesc, TextureFormat,
};

/// offscreen layer texture format(선형 working space, premultiplied).
pub const LAYER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// main target depth·stencil format.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32FloatStencil8;

/// pixel 경계 판정 허용 오차(device pixel).
pub const PIXEL_EPSILON: f32 = 1.0 / 256.0;

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
}

impl FramePlan {
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
    #[default]
    Conservative,
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

fn near_integer(value: f32) -> bool {
    (value - value.round()).abs() <= PIXEL_EPSILON
}

/// `local → world` 변환과 view를 합쳐 축 정렬 사각형이 device pixel 경계에 정확히 놓이면 참입니다.
fn pixel_aligned(
    (x, y, width, height): (f32, f32, f32, f32),
    to_world: impl FnOnce() -> Option<(f32, f32, f32, f32, f32, f32)>,
) -> bool {
    let Some((a, b, c, d, tx, ty)) = to_world() else {
        return false;
    };
    if b != 0.0 || c != 0.0 {
        return false;
    }
    [
        a * x + tx,
        a * (x + width) + tx,
        d * y + ty,
        d * (y + height) + ty,
    ]
    .into_iter()
    .all(|value| value.is_finite() && near_integer(value))
}

/// clip이 모두 모서리 없는 축 정렬 사각형이고 device pixel 경계에 놓이면 참입니다.
pub fn clips_are_pixel_aligned_rects(node: &SceneNode, viewport: &Viewport) -> bool {
    node.clip().iter_inner_first().all(|clip| match clip {
        ClipShape::Rect {
            rect,
            radii,
            transform,
        } => {
            radii.is_zero()
                && pixel_aligned(
                    (
                        rect.origin().x(),
                        rect.origin().y(),
                        rect.size().width(),
                        rect.size().height(),
                    ),
                    || {
                        transform
                            .then(&viewport.view())
                            .ok()
                            .map(|t| t.components())
                    },
                )
        }
        ClipShape::Path { .. } => false,
    })
}

/// node 자기 내용의 분류(모듈 문서 규칙).
pub fn classify_content(node: &SceneNode, viewport: &Viewport) -> PrimitiveClass {
    let SceneContent::Rect(primitive) = node.content() else {
        return PrimitiveClass::Transparent;
    };
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
        && pixel_aligned(
            (
                primitive.rect.origin().x(),
                primitive.rect.origin().y(),
                primitive.rect.size().width(),
                primitive.rect.size().height(),
            ),
            || {
                node.world_transform()
                    .then(&viewport.view())
                    .ok()
                    .map(|t| t.components())
            },
        )
        && clips_are_pixel_aligned_rects(node, viewport);
    if opaque {
        PrimitiveClass::Opaque
    } else {
        PrimitiveClass::Transparent
    }
}

/// layer 경계로 둘지 결정합니다.
fn is_boundary(node: &SceneNode, boundaries: EffectBoundaries) -> bool {
    match boundaries {
        EffectBoundaries::Conservative => has_compositing_effect(node),
    }
}

/// 수집 중인 target(main 또는 layer)의 draw.
struct Scope {
    layer: Option<LayerId>,
    /// 경계 node(main이면 None)와 그 subtree 마지막 z.
    boundary: Option<(Arc<SceneNode>, ZOrder)>,
    draws: Vec<(ZOrder, PrimitiveClass, DrawSource)>,
}

/// 수집 결과 layer(아직 graph 없음).
struct LayerDraft {
    node: Arc<SceneNode>,
    parent: Option<LayerId>,
    draws: Vec<(ZOrder, PrimitiveClass, DrawSource)>,
}

fn ranked(
    draws: Vec<(ZOrder, PrimitiveClass, DrawSource)>,
) -> Result<Vec<PlannedDraw>, FrameCompileError> {
    draws
        .into_iter()
        .enumerate()
        .map(|(index, (z, class, source))| {
            let depth = u32::try_from(index + 1).map_err(|_| FrameCompileError::TooManyDraws)?;
            Ok(PlannedDraw {
                z,
                depth,
                class,
                source,
                opacity: Opacity::OPAQUE,
            })
        })
        .collect()
}

/// snapshot을 frame plan으로 compile합니다.
pub fn compile_frame(input: &CompileInput<'_>) -> Result<FramePlan, FrameCompileError> {
    let visible: Option<HashSet<ZOrder>> = input
        .visible
        .map(|list| list.items.iter().map(|item| item.z).collect());
    let is_visible = |node: &SceneNode| {
        node.hit().visibility == Visibility::Visible
            && visible.as_ref().is_none_or(|set| set.contains(&node.z()))
    };

    // 1. pre-order 순회로 draw를 target별로 모읍니다.
    let mut layers: Vec<LayerDraft> = Vec::new();
    let mut scopes: Vec<Scope> = vec![Scope {
        layer: None,
        boundary: None,
        draws: Vec::new(),
    }];
    let close_scope = |scopes: &mut Vec<Scope>, layers: &mut Vec<LayerDraft>| {
        let Some(scope) = scopes.pop() else {
            return;
        };
        let (Some(layer), Some((node, _))) = (scope.layer, scope.boundary) else {
            return;
        };
        let parent = scopes.last_mut();
        if scope.draws.is_empty() {
            // 아무것도 그리지 않는 layer는 만들지 않습니다(id는 비워 둠).
            layers[layer.0 as usize].draws = Vec::new();
            return;
        }
        layers[layer.0 as usize].draws = scope.draws;
        if let Some(parent) = parent {
            parent.draws.push((
                node.z(),
                PrimitiveClass::Transparent,
                DrawSource::Layer {
                    layer,
                    node: Arc::clone(&node),
                },
            ));
        }
    };

    let mut stack: Vec<&Arc<SceneNode>> = input.snapshot.roots().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        // 이 node 앞에서 끝난 layer scope를 닫습니다(subtree 마지막 z < 현재 z).
        while scopes
            .last()
            .and_then(|scope| scope.boundary.as_ref())
            .is_some_and(|(_, last_z)| *last_z < node.z())
        {
            close_scope(&mut scopes, &mut layers);
        }
        let shown = node.hit().visibility == Visibility::Visible;
        if shown && is_boundary(node, input.boundaries) {
            let id = LayerId(layers.len() as u32);
            layers.push(LayerDraft {
                node: Arc::clone(node),
                parent: scopes.last().and_then(|scope| scope.layer),
                draws: Vec::new(),
            });
            scopes.push(Scope {
                layer: Some(id),
                boundary: Some((Arc::clone(node), node.last_z())),
                draws: Vec::new(),
            });
        }
        if is_visible(node) && draws_content(node) {
            let class = classify_content(node, input.viewport);
            if let Some(scope) = scopes.last_mut() {
                scope
                    .draws
                    .push((node.z(), class, DrawSource::Content(Arc::clone(node))));
            }
        }
        let start = stack.len();
        stack.extend(node.children());
        stack[start..].reverse();
    }
    while scopes.len() > 1 {
        close_scope(&mut scopes, &mut layers);
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
        outputs[index] = Some(output);
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
    Ok(FramePlan {
        graph,
        passes,
        layers: effect_layers,
    })
}

#[cfg(test)]
pub(crate) mod tests;
