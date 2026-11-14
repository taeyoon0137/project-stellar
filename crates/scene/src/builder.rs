//! node 종류별 scene builder (XI.2.2).
//!
//! document 계층이 binding·modifier·layout 평가를 끝낸 값을 node 종류별 입력 구조체
//! ([`FrameInput`], [`ShapeInput`], [`PathInput`], [`TextInput`], [`ImageInput`])로 넘기면,
//! 종류마다 **별도 함수**([`SceneBuilder::begin_frame`]/[`SceneBuilder::finish_frame`],
//! [`SceneBuilder::build_shape`], [`SceneBuilder::build_path`], [`SceneBuilder::build_text`],
//! [`SceneBuilder::build_image`])가 scene primitive로 바꿉니다. scene은 document를 import하지
//! 않으므로(II.2.8) 입력은 document 값의 의미를 옮긴 독립 타입입니다.
//!
//! ## 공통 규칙
//!
//! - **world transform**: `node local transform`을 먼저, 부모 scope의 world transform을 나중에
//!   적용합니다(`local.then(parent_world)`).
//! - **z-order**: 호출 순서대로 1부터 증가합니다. Frame은 자식보다 먼저(`begin_frame`) 번호를
//!   받으므로 pre-order 불변식(XI.1.2)이 구성상 성립합니다.
//! - **표시·잠금**: 자신이나 조상이 숨김이면 [`Visibility::Hidden`], 자신이나 조상이 잠금이면
//!   [`PointerEvents::None`](canvas hit test 통과)입니다. 숨긴 node도 scene에 남겨 표시 전환이
//!   구조 변경이 되지 않게 합니다.
//! - **paint 순서**: payload 고유 paint(Frame 배경·테두리)가 먼저, modifier 평가 결과가 그 뒤에
//!   modifier 순서대로 쌓입니다. layer는 content geometry(Text는 glyph 윤곽, Image는 배치 상자)를
//!   칠하며, 이미지 픽셀·text run 색 같은 content 고유 모습 **위**에 칠합니다.
//! - **stroke 정렬**: 상자형 content(Frame, Image)는 [`StrokeAlign::Inside`](크기 상자 안),
//!   선형 content(Shape, Path, Text)는 [`StrokeAlign::Center`](document Line의 중심선 규칙)입니다.
//! - **clip**: node는 부모 scope의 clip을 받습니다. Frame은 자기 clip을 **자식 scope**에만 겁니다.
//!
//! ## Fragment flatten (XI.2.3)
//!
//! Fragment는 렌더·layout 필드가 없고 좌표계도 만들지 않는 marker입니다(아키텍처 §12.4, VI.4).
//! 그래서 scene node를 만들지 않습니다. [`SceneBuilder::begin_fragment`]가 연 scope는 부모와 같은
//! world transform·clip을 쓰고, [`SceneBuilder::finish_fragment`]가 모은 자식을 **부모 scope에 그대로
//! 이어 붙입니다**. Fragment는 z-order도 받지 않으므로, Fragment가 있든 없든 자식의 primitive·순서·
//! z-order가 같습니다. Fragment가 가질 수 있는 상태 중 렌더에 영향을 주는 것은 표시·잠금뿐이며,
//! 이는 자식의 실효 visibility·pointer-events에 반영됩니다(Fragment 대신 자식마다 숨김·잠금을
//! 준 것과 같은 결과).

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};

use crate::geometry::{FillRule, PathGeometry};
use crate::node::{
    CornerRadii, HitTestMeta, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PathPrimitive,
    PointerEvents, RectPrimitive, SceneContent, SceneNode, SceneNodeSpec, SceneTreeError,
    SceneValueError, TextLayoutKey, TextPrimitive, Visibility, ZOrder,
};
use crate::stack::{
    ClipShape, ClipStackRef, Opacity, Paint, PaintLayer, PaintStack, PaintStackRef, SceneBlendMode,
    StrokeAlign, StrokeCap, StrokeLayer,
};

/// scene build 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneBuildError {
    /// world transform 합성 결과가 유한하지 않습니다(오버플로).
    NonFiniteTransform {
        /// 문제 node.
        source: StableId,
    },
    /// 값 규칙 위반(stroke 폭 등).
    InvalidValue {
        /// 문제 node.
        source: StableId,
        /// 원인.
        error: SceneValueError,
    },
    /// z-order 공간(`u64`)을 다 썼습니다.
    ZOrderExhausted,
    /// scene tree 불변식 위반(builder 오용).
    Tree(SceneTreeError),
}

impl fmt::Display for SceneBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneBuildError::NonFiniteTransform { source } => {
                write!(f, "node {source}의 world transform이 유한하지 않습니다")
            }
            SceneBuildError::InvalidValue { source, error } => {
                write!(f, "node {source}의 값이 잘못되었습니다: {error}")
            }
            SceneBuildError::ZOrderExhausted => f.write_str("z-order를 더 할당할 수 없습니다"),
            SceneBuildError::Tree(error) => write!(f, "scene tree 오류: {error}"),
        }
    }
}

impl std::error::Error for SceneBuildError {}

/// modifier 평가 결과의 stroke 하나(document `Stroke` modifier: 색 + 두께).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeInput {
    /// 칠하기 원천.
    pub paint: Paint,
    /// 두께(비음수).
    pub width: f32,
}

/// modifier stack 평가 결과(document Fill·Stroke·Shadow·Blur·Blend·Opacity modifier).
#[derive(Debug, Clone, PartialEq)]
pub struct ModifierPaint {
    /// 채우기(modifier 순서).
    pub fills: Vec<Paint>,
    /// 선(modifier 순서).
    pub strokes: Vec<StrokeInput>,
    /// drop shadow.
    pub shadows: Vec<Shadow>,
    /// 합성 불투명도.
    pub opacity: Opacity,
    /// 합성 blend mode.
    pub blend: SceneBlendMode,
    /// 합성 결과 blur.
    pub layer_blur: BlurRadius,
}

impl Default for ModifierPaint {
    /// modifier 없음: layer·shadow 없음, 불투명, Normal, blur 0.
    fn default() -> ModifierPaint {
        ModifierPaint {
            fills: Vec::new(),
            strokes: Vec::new(),
            shadows: Vec::new(),
            opacity: Opacity::OPAQUE,
            blend: SceneBlendMode::Normal,
            layer_blur: BlurRadius::ZERO,
        }
    }
}

/// 모든 node 종류 공통 입력.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeInput {
    /// 원본 document node.
    pub source: StableId,
    /// 부모 좌표계 기준 local transform(layout 위치 포함).
    pub transform: Transform2D,
    /// document `visible`.
    pub visible: bool,
    /// document `locked`.
    pub locked: bool,
    /// modifier 평가 결과.
    pub paint: ModifierPaint,
}

impl NodeInput {
    /// 위치 `transform`, 표시·비잠금, modifier 없음인 입력.
    pub fn new(source: StableId, transform: Transform2D) -> NodeInput {
        NodeInput {
            source,
            transform,
            visible: true,
            locked: false,
            paint: ModifierPaint::default(),
        }
    }
}

/// Frame clip 입력. document `Clipping`을 해석한 결과입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameClipInput {
    /// clip 없음.
    None,
    /// frame bounds(모서리 radius 포함) clip.
    Bounds,
    /// PathRef가 가리키는 Path node geometry(frame local 좌표) clip.
    Path {
        /// clip geometry.
        geometry: PathGeometry,
        /// 채우기 규칙.
        fill_rule: FillRule,
    },
}

/// Frame 테두리(document `Border`: 색 + 비음수 두께).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderInput {
    /// 색.
    pub color: Color,
    /// 두께.
    pub thickness: f32,
}

/// Frame 입력(document `FramePayload` 평가 결과).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInput {
    /// 공통 입력.
    pub node: NodeInput,
    /// layout 결과 크기.
    pub size: Size2,
    /// 실효 모서리 radius.
    pub corner_radii: CornerRadii,
    /// 배경 색.
    pub background: Option<Color>,
    /// 테두리.
    pub border: Option<BorderInput>,
    /// clip.
    pub clip: FrameClipInput,
}

/// Shape 입력. geometry는 document Shape 순수 평가(VI.5.6) 결과입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeInput {
    /// 공통 입력.
    pub node: NodeInput,
    /// shape local layout 상자.
    pub layout_rect: Rect,
    /// 평가된 path(Shape는 NonZero).
    pub geometry: PathGeometry,
    /// 열린 subpath 끝 모양(Line shape의 document `LineCap`, 그 외 Butt).
    pub cap: StrokeCap,
}

/// Path 입력(document `PathData`).
#[derive(Debug, Clone, PartialEq)]
pub struct PathInput {
    /// 공통 입력.
    pub node: NodeInput,
    /// path local layout 상자.
    pub layout_rect: Rect,
    /// geometry.
    pub geometry: PathGeometry,
    /// document winding rule.
    pub fill_rule: FillRule,
}

/// Text 입력. glyph는 text layout 결과(`layout`)에 있습니다.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInput {
    /// 공통 입력.
    pub node: NodeInput,
    /// text layout cache key.
    pub layout: TextLayoutKey,
    /// text local layout 상자.
    pub size: Size2,
}

/// Fragment 입력. document Fragment는 공통 column(표시·잠금 등) 외 필드가 없습니다(VI.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentInput {
    /// 원본 document node(scene에는 남지 않음).
    pub source: StableId,
    /// document `visible`.
    pub visible: bool,
    /// document `locked`.
    pub locked: bool,
}

/// [`SceneBuilder::begin_fragment`]가 연 Fragment scope. 자식을 [`FragmentScope::scope_mut`]에
/// 쌓은 뒤 [`SceneBuilder::finish_fragment`]로 부모에 펼칩니다.
#[derive(Debug)]
pub struct FragmentScope {
    source: StableId,
    children: SceneScope,
}

impl FragmentScope {
    /// 원본 Fragment node.
    pub const fn source(&self) -> StableId {
        self.source
    }

    /// 자식 scope.
    pub fn scope_mut(&mut self) -> &mut SceneScope {
        &mut self.children
    }

    /// 자식 scope(읽기).
    pub fn scope(&self) -> &SceneScope {
        &self.children
    }
}

/// Image 입력(document `ImagePayload`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInput {
    /// 공통 입력.
    pub node: NodeInput,
    /// asset.
    pub asset: AssetRef,
    /// 배치 상자 크기.
    pub size: Size2,
    /// fit.
    pub fit: ImageFit,
    /// crop.
    pub crop: ImageCrop,
    /// sampling.
    pub sampling: ImageSampling,
    /// 색공간 override.
    pub color_space: Option<ColorSpace>,
}

/// 자식 node를 모으는 scope. 부모의 world transform·clip·숨김·잠금 상태를 물려줍니다.
#[derive(Debug, Clone)]
pub struct SceneScope {
    pub(crate) world: Transform2D,
    pub(crate) clip: ClipStackRef,
    pub(crate) hidden: bool,
    pub(crate) locked: bool,
    pub(crate) nodes: Vec<Arc<SceneNode>>,
}

impl SceneScope {
    /// 지금까지 이 scope에 쌓인 node(paint 순서).
    pub fn nodes(&self) -> &[Arc<SceneNode>] {
        &self.nodes
    }

    /// scope의 world transform.
    pub const fn world(&self) -> Transform2D {
        self.world
    }

    /// scope 자식이 받을 clip.
    pub const fn clip(&self) -> &ClipStackRef {
        &self.clip
    }

    /// node를 모두 꺼냅니다.
    pub fn into_nodes(self) -> Vec<Arc<SceneNode>> {
        self.nodes
    }
}

/// [`SceneBuilder::begin_frame`]이 연 Frame. 자식을 [`FrameScope::scope_mut`]에 쌓은 뒤
/// [`SceneBuilder::finish_frame`]으로 닫습니다.
#[derive(Debug)]
pub struct FrameScope {
    spec: SceneNodeSpec,
    children: SceneScope,
}

impl FrameScope {
    /// 자식을 추가하기 전에도 확정되는 이 Frame의 scene primitive·좌표·입력 상태.
    pub const fn spec(&self) -> &SceneNodeSpec {
        &self.spec
    }

    /// 자식 scope.
    pub fn scope_mut(&mut self) -> &mut SceneScope {
        &mut self.children
    }

    /// 자식 scope(읽기).
    pub fn scope(&self) -> &SceneScope {
        &self.children
    }
}

/// node 종류별 scene builder. z-order를 호출 순서대로 발급합니다.
#[derive(Debug)]
pub struct SceneBuilder {
    next_z: u64,
    built: u64,
}

impl Default for SceneBuilder {
    fn default() -> SceneBuilder {
        SceneBuilder::new()
    }
}

impl SceneBuilder {
    /// z-order 1부터 발급하는 builder.
    pub const fn new() -> SceneBuilder {
        SceneBuilder {
            next_z: 1,
            built: 0,
        }
    }

    /// 최상위 scope(항등 변환, clip 없음, 표시, 비잠금).
    pub fn root_scope(&self) -> SceneScope {
        SceneScope {
            world: Transform2D::identity(),
            clip: ClipStackRef::none(),
            hidden: false,
            locked: false,
            nodes: Vec::new(),
        }
    }

    /// 지금까지 만든 node 수.
    pub const fn built_count(&self) -> u64 {
        self.built
    }

    fn allocate_z(&mut self) -> Result<ZOrder, SceneBuildError> {
        let z = self.next_z;
        self.next_z = z.checked_add(1).ok_or(SceneBuildError::ZOrderExhausted)?;
        self.built += 1;
        Ok(ZOrder(z))
    }

    /// Frame을 엽니다. Frame node는 자식보다 먼저 z-order를 받습니다.
    pub fn begin_frame(
        &mut self,
        parent: &SceneScope,
        input: &FrameInput,
    ) -> Result<FrameScope, SceneBuildError> {
        let world = world_of(parent, &input.node)?;
        let rect = Rect::new(Vec2::ZERO, input.size);
        let mut layers = Vec::new();
        if let Some(background) = input.background {
            layers.push(PaintLayer::Fill(Paint::Solid(background)));
        }
        if let Some(border) = input.border {
            layers.push(PaintLayer::Stroke(stroke(
                &input.node,
                Paint::Solid(border.color),
                border.thickness,
                StrokeAlign::Inside,
                StrokeCap::Butt,
            )?));
        }
        let paint = paint_stack(&input.node, layers, StrokeAlign::Inside, StrokeCap::Butt)?;
        let child_clip = match &input.clip {
            FrameClipInput::None => parent.clip.clone(),
            FrameClipInput::Bounds => parent.clip.push(ClipShape::Rect {
                rect,
                radii: input.corner_radii,
                transform: world,
            }),
            FrameClipInput::Path {
                geometry,
                fill_rule,
            } => parent.clip.push(ClipShape::Path {
                geometry: geometry.clone(),
                fill_rule: *fill_rule,
                transform: world,
            }),
        };
        let spec = SceneNodeSpec {
            source: input.node.source,
            content: SceneContent::Rect(RectPrimitive {
                rect,
                radii: input.corner_radii,
            }),
            world_transform: world,
            layout_rect: rect,
            clip: parent.clip.clone(),
            paint,
            z: self.allocate_z()?,
            hit: hit_of(parent, &input.node),
        };
        Ok(FrameScope {
            spec,
            children: SceneScope {
                world,
                clip: child_clip,
                hidden: parent.hidden || !input.node.visible,
                locked: parent.locked || input.node.locked,
                nodes: Vec::new(),
            },
        })
    }

    /// Frame을 닫아 `parent`에 추가하고 만든 node를 돌려줍니다.
    pub fn finish_frame(
        &mut self,
        parent: &mut SceneScope,
        frame: FrameScope,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let node = Arc::new(
            SceneNode::new(frame.spec, frame.children.nodes).map_err(SceneBuildError::Tree)?,
        );
        parent.nodes.push(Arc::clone(&node));
        Ok(node)
    }

    /// Shape를 path primitive(NonZero)로 만들어 `parent`에 추가합니다.
    pub fn build_shape(
        &mut self,
        parent: &mut SceneScope,
        input: &ShapeInput,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let content = SceneContent::Path(PathPrimitive {
            geometry: input.geometry.clone(),
            fill_rule: FillRule::NonZero,
        });
        self.leaf(
            parent,
            &input.node,
            content,
            input.layout_rect,
            StrokeAlign::Center,
            input.cap,
        )
    }

    /// Path를 path primitive(document winding rule 유지)로 만들어 `parent`에 추가합니다.
    pub fn build_path(
        &mut self,
        parent: &mut SceneScope,
        input: &PathInput,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let content = SceneContent::Path(PathPrimitive {
            geometry: input.geometry.clone(),
            fill_rule: input.fill_rule,
        });
        self.leaf(
            parent,
            &input.node,
            content,
            input.layout_rect,
            StrokeAlign::Center,
            StrokeCap::Butt,
        )
    }

    /// Text를 text primitive로 만들어 `parent`에 추가합니다.
    pub fn build_text(
        &mut self,
        parent: &mut SceneScope,
        input: &TextInput,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let bounds = Rect::new(Vec2::ZERO, input.size);
        let content = SceneContent::Text(TextPrimitive {
            layout: input.layout,
            bounds,
        });
        self.leaf(
            parent,
            &input.node,
            content,
            bounds,
            StrokeAlign::Center,
            StrokeCap::Butt,
        )
    }

    /// Image를 image primitive로 만들어 `parent`에 추가합니다.
    pub fn build_image(
        &mut self,
        parent: &mut SceneScope,
        input: &ImageInput,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let bounds = Rect::new(Vec2::ZERO, input.size);
        let content = SceneContent::Image(ImagePrimitive {
            asset: input.asset,
            bounds,
            fit: input.fit,
            crop: input.crop,
            sampling: input.sampling,
            color_space: input.color_space,
        });
        self.leaf(
            parent,
            &input.node,
            content,
            bounds,
            StrokeAlign::Inside,
            StrokeCap::Butt,
        )
    }

    /// Fragment scope를 엽니다(XI.2.3). scene node·z-order를 만들지 않고, 부모의 world
    /// transform·clip을 그대로 물려주며 Fragment의 숨김·잠금만 자식 상태에 더합니다.
    pub fn begin_fragment(&self, parent: &SceneScope, input: &FragmentInput) -> FragmentScope {
        FragmentScope {
            source: input.source,
            children: SceneScope {
                world: parent.world,
                clip: parent.clip.clone(),
                hidden: parent.hidden || !input.visible,
                locked: parent.locked || input.locked,
                nodes: Vec::new(),
            },
        }
    }

    /// Fragment 자식을 순서대로 `parent` scope에 이어 붙입니다. 붙인 node 수를 돌려줍니다.
    pub fn finish_fragment(&self, parent: &mut SceneScope, fragment: FragmentScope) -> usize {
        let count = fragment.children.nodes.len();
        parent.nodes.extend(fragment.children.nodes);
        count
    }

    /// 최상위 scope를 닫고 snapshot 최상위 node 목록을 돌려줍니다.
    pub fn finish(self, root: SceneScope) -> Vec<Arc<SceneNode>> {
        root.nodes
    }

    fn leaf(
        &mut self,
        parent: &mut SceneScope,
        node: &NodeInput,
        content: SceneContent,
        layout_rect: Rect,
        align: StrokeAlign,
        cap: StrokeCap,
    ) -> Result<Arc<SceneNode>, SceneBuildError> {
        let spec = SceneNodeSpec {
            source: node.source,
            content,
            world_transform: world_of(parent, node)?,
            layout_rect,
            clip: parent.clip.clone(),
            paint: paint_stack(node, Vec::new(), align, cap)?,
            z: self.allocate_z()?,
            hit: hit_of(parent, node),
        };
        let leaf = Arc::new(SceneNode::leaf(spec));
        parent.nodes.push(Arc::clone(&leaf));
        Ok(leaf)
    }
}

fn world_of(parent: &SceneScope, node: &NodeInput) -> Result<Transform2D, SceneBuildError> {
    node.transform
        .then(&parent.world)
        .map_err(|_| SceneBuildError::NonFiniteTransform {
            source: node.source,
        })
}

fn hit_of(parent: &SceneScope, node: &NodeInput) -> HitTestMeta {
    HitTestMeta {
        visibility: if parent.hidden || !node.visible {
            Visibility::Hidden
        } else {
            Visibility::Visible
        },
        pointer_events: if parent.locked || node.locked {
            PointerEvents::None
        } else {
            PointerEvents::Auto
        },
    }
}

fn stroke(
    node: &NodeInput,
    paint: Paint,
    width: f32,
    align: StrokeAlign,
    cap: StrokeCap,
) -> Result<StrokeLayer, SceneBuildError> {
    StrokeLayer::new(paint, width, align, cap).map_err(|error| SceneBuildError::InvalidValue {
        source: node.source,
        error,
    })
}

/// payload 고유 layer 뒤에 modifier fill·stroke를 쌓은 paint stack.
fn paint_stack(
    node: &NodeInput,
    mut layers: Vec<PaintLayer>,
    align: StrokeAlign,
    cap: StrokeCap,
) -> Result<PaintStackRef, SceneBuildError> {
    let modifiers = &node.paint;
    layers.extend(modifiers.fills.iter().copied().map(PaintLayer::Fill));
    for input in &modifiers.strokes {
        layers.push(PaintLayer::Stroke(stroke(
            node,
            input.paint,
            input.width,
            align,
            cap,
        )?));
    }
    Ok(PaintStackRef::new(PaintStack {
        layers,
        shadows: modifiers.shadows.clone(),
        opacity: modifiers.opacity,
        blend: modifiers.blend,
        layer_blur: modifiers.layer_blur,
    }))
}

#[cfg(test)]
mod tests;
