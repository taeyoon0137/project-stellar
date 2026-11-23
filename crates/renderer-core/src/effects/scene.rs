//! scene frame plan → 그리기 목록 (제품 frame encoder, XI.11.1 후속, ADR-0116 결정 4).
//!
//! [`encode_plan`]은 pass compiler의 [`FramePlan`](draw 순서·layer 경계·clip 방식·효과, ADR-0116 결정 3)을 device 좌표의
//! backend 중립 그리기 목록([`Op`])으로 낮춥니다. 이 목록은 적합성 fixture와 같은 모델이라 CPU 기준
//! ([`super::raster`])과 GPU lowering([`super::lower`])이 그대로 실행합니다. 그래서 scene → pixel 경로가 fixture로 검증한
//! 수식·효과 순서(ADR-0102)를 그대로 씁니다.
//!
//! # 규칙
//!
//! - **순서**: main target의 draw는 depth(paint 순위) 오름차순, 곧 painter 순서입니다. opaque·transparent 분리는 같은
//!   결과를 내는 GPU 최적화라 여기서는 합칩니다(불투명 draw의 source-over는 덮어쓰기). layer 합성 draw는 그 layer pass의 draw를
//!   painter 순서로 담은 [`LayerOp`]입니다.
//! - **효과**: 한 draw의 효과는 [`LayerOp`] 하나의 순서(내용 → shadow → layer blur → clip → mask → opacity → blend)로
//!   표현합니다. shadow·σ·offset은 frame plan의 device 값([`DrawEffects`])이고 offset은 소수 device pixel 그대로입니다
//!   (ADR-0130 결정 3). shadow 원본은 plan의 [`ShadowShape`]이고, 상자 shadow(Frame, CSS `box-shadow`)는 테두리 상자 device
//!   mesh를 원본·비우기 모양으로 씁니다(ADR-0130 결정 1).
//!   합성 mode는 plan이 `resolve_or_normal`로 푼 값입니다. 내용 draw에 접힌 불투명도·blend도 같은 layer로 합성합니다(단일
//!   draw 합성과 같음).
//! - **clip**: [`ClipPlan::Scissor`]는 scissor, [`ClipPlan::Stencil`]·[`ClipPlan::Mask`]는 scissor(모든 clip의 경계)와 복합
//!   clip마다 path stencil입니다. 여러 clip은 layer를 겹쳐 교집합으로 만듭니다. blend는 바깥 layer에서만 합성하므로 backdrop이
//!   바뀌지 않습니다.
//! - **내용**: Rect는 모서리 둥근 사각형 path, Path는 자기 채우기 규칙, Image는 배치 상자, Text는 [`FrameResources::text`]가
//!   준 glyph 모양(run마다 하나, non-zero)입니다. 이미지 pixel·text run 색 → paint layer(아래 → 위) 순서로 칠합니다. fill은
//!   device 좌표로 tessellate하고, stroke는 node local로 tessellate한 뒤 device로 옮깁니다. `Inside`는 geometry 안으로,
//!   `Outside`는 geometry 밖(경계 상자에서 geometry를 even-odd로 뺀 영역)으로 자른 layer입니다.
//! - **미지원**: 축 정렬이 아닌 이미지 변환, 원형 gradient 같은 GPU 경로 없는 그리기는 [`EncodeError::Unsupported`]입니다.

use std::fmt;
use std::sync::Arc;

use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::node::{ImagePrimitive, SceneContent, SceneNode, TextPrimitive};
use stellar_scene::stack::{ClipShape, Paint, PaintLayer, SceneBlendMode, StrokeAlign};
use stellar_scene::visibility::Viewport;
use stellar_scene::{Rect, Transform2D, Vec2};

use super::fixture::{
    fill_mesh, solid, ClipSpec, Coverage, FixtureError, ImageData, ImageOp, LayerOp, Mesh, Op,
    ShadowSource,
};
use crate::clip_plan::ClipPlan;
use crate::output_color::OutputColor;
use crate::pass_compiler::effects::{DrawEffects, ShadowShape};
use crate::pass_compiler::{DrawSource, FramePlan, LayerId, PassClass, PlannedDraw};
use crate::placeholder::{TextureFilter, TextureWrap};
use crate::stroke::{tessellate_stroke, StrokeStyle};
use crate::tessellation::{FlattenTolerance, TessellationError};

/// 원호 cubic 근사 계수(4/3·(√2 − 1)).
pub const KAPPA: f32 = 0.552_284_8;

/// stroke 평탄화의 device 허용 오차(pixel).
pub const DEVICE_TOLERANCE: f32 = 0.25;

/// text run 하나의 glyph 모양(node local, non-zero)과 run 색.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphShape {
    /// glyph outline 합(node local).
    pub geometry: PathGeometry,
    /// run 색.
    pub color: OutputColor,
}

/// encoder가 scene 밖에서 얻는 내용(이미지 pixel, text glyph 모양).
pub trait FrameResources {
    /// image primitive의 decoded pixel. `Ok(None)`은 호출자가 생략을 확인한 이미지입니다.
    fn image(
        &self,
        node: &SceneNode,
        image: &ImagePrimitive,
    ) -> Result<Option<Arc<ImageData>>, EncodeError>;

    /// text primitive의 glyph 모양(run 순서). `Ok(None)`은 호출자가 생략을 확인한 layout입니다.
    fn text(
        &self,
        node: &SceneNode,
        text: &TextPrimitive,
    ) -> Result<Option<Vec<GlyphShape>>, EncodeError>;
}

/// encode 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeError {
    /// GPU 공통 경로가 없는 그리기.
    Unsupported {
        /// node source.
        node: stellar_scene::StableId,
        /// 무엇.
        what: &'static str,
    },
    /// tessellation 실패.
    Tessellation {
        /// node source.
        node: stellar_scene::StableId,
        /// 원인.
        error: TessellationError,
    },
    /// 내용 조회 실패(호출자 메시지).
    Resource {
        /// node source.
        node: stellar_scene::StableId,
        /// 원인.
        message: String,
    },
    /// 그리기 목록 값이 계약을 벗어남.
    Fixture(FixtureError),
    /// frame plan이 일관되지 않습니다(compiler 버그).
    Plan(&'static str),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::Unsupported { node, what } => {
                write!(f, "node {node}: GPU 경로가 없는 그리기({what})")
            }
            EncodeError::Tessellation { node, error } => {
                write!(f, "node {node}: tessellation 실패({error})")
            }
            EncodeError::Resource { node, message } => write!(f, "node {node}: {message}"),
            EncodeError::Fixture(error) => write!(f, "그리기 목록 오류: {error:?}"),
            EncodeError::Plan(detail) => write!(f, "frame plan 오류: {detail}"),
        }
    }
}

impl std::error::Error for EncodeError {}

impl From<FixtureError> for EncodeError {
    fn from(error: FixtureError) -> EncodeError {
        EncodeError::Fixture(error)
    }
}

/// frame plan을 painter 순서 그리기 목록으로 낮춥니다(모듈 문서 규칙).
pub fn encode_plan(
    plan: &FramePlan,
    viewport: &Viewport,
    resources: &dyn FrameResources,
) -> Result<Vec<Op>, EncodeError> {
    let encoder = Encoder {
        plan,
        viewport,
        resources,
    };
    let mut main: Vec<(&PlannedDraw, crate::render_graph::PassId)> = plan
        .passes
        .iter()
        .filter(|pass| matches!(pass.class, PassClass::Opaque | PassClass::Transparent))
        .flat_map(|pass| pass.draws.iter().map(move |draw| (draw, pass.pass)))
        .collect();
    main.sort_by_key(|(draw, _)| draw.depth);
    let mut ops = Vec::new();
    for (draw, pass) in main {
        ops.extend(encoder.draw(pass, draw)?);
    }
    Ok(ops)
}

struct Encoder<'a> {
    plan: &'a FramePlan,
    viewport: &'a Viewport,
    resources: &'a dyn FrameResources,
}

fn unsupported(node: &SceneNode, what: &'static str) -> EncodeError {
    EncodeError::Unsupported {
        node: node.source(),
        what,
    }
}

fn tessellation(node: &SceneNode) -> impl Fn(FixtureError) -> EncodeError + '_ {
    move |error| match error {
        FixtureError::Tessellation(error) => EncodeError::Tessellation {
            node: node.source(),
            error,
        },
        other => EncodeError::Fixture(other),
    }
}

/// 모서리 둥근 사각형 path(시계 방향, 원호는 cubic 하나).
pub fn rounded_rect_path(rect: Rect, radii: (f32, f32, f32, f32)) -> Option<PathGeometry> {
    let (x, y) = (rect.origin().x(), rect.origin().y());
    let (w, h) = (rect.size().width(), rect.size().height());
    let (tl, tr, br, bl) = radii;
    let v = |px: f32, py: f32| Vec2::new(px, py).ok();
    let k = KAPPA;
    let mut commands = vec![PathCommand::MoveTo(v(x + tl, y)?)];
    let mut corner = |line: (f32, f32), c1: (f32, f32), c2: (f32, f32), to: (f32, f32), r: f32| {
        commands.push(PathCommand::LineTo(v(line.0, line.1)?));
        if r > 0.0 {
            commands.push(PathCommand::CubicTo {
                c1: v(c1.0, c1.1)?,
                c2: v(c2.0, c2.1)?,
                to: v(to.0, to.1)?,
            });
        }
        Some(())
    };
    corner(
        (x + w - tr, y),
        (x + w - tr + tr * k, y),
        (x + w, y + tr - tr * k),
        (x + w, y + tr),
        tr,
    )?;
    corner(
        (x + w, y + h - br),
        (x + w, y + h - br + br * k),
        (x + w - br + br * k, y + h),
        (x + w - br, y + h),
        br,
    )?;
    corner(
        (x + bl, y + h),
        (x + bl - bl * k, y + h),
        (x, y + h - bl + bl * k),
        (x, y + h - bl),
        bl,
    )?;
    corner(
        (x, y + tl),
        (x, y + tl - tl * k),
        (x + tl - tl * k, y),
        (x + tl, y),
        tl,
    )?;
    commands.push(PathCommand::Close);
    PathGeometry::new(commands).ok()
}

/// path의 모든 점을 `transform`으로 옮깁니다.
pub fn transform_path(path: &PathGeometry, transform: &Transform2D) -> Option<PathGeometry> {
    let map = |p: Vec2| transform.apply(p).ok();
    let commands = path
        .commands()
        .iter()
        .map(|command| {
            Some(match *command {
                PathCommand::MoveTo(p) => PathCommand::MoveTo(map(p)?),
                PathCommand::LineTo(p) => PathCommand::LineTo(map(p)?),
                PathCommand::CubicTo { c1, c2, to } => PathCommand::CubicTo {
                    c1: map(c1)?,
                    c2: map(c2)?,
                    to: map(to)?,
                },
                PathCommand::Close => PathCommand::Close,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    PathGeometry::new(commands).ok()
}

fn rect_path(rect: Rect) -> Option<PathGeometry> {
    rounded_rect_path(rect, (0.0, 0.0, 0.0, 0.0))
}

/// mesh 정점의 축 정렬 경계(x0, y0, x1, y1).
fn mesh_bounds(mesh: &Mesh) -> Option<[f32; 4]> {
    let mut iter = mesh.positions.iter();
    let first = iter.next()?;
    let mut b = [first[0], first[1], first[0], first[1]];
    for p in iter {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    Some(b)
}

impl Encoder<'_> {
    fn to_device(&self, node: &SceneNode) -> Option<Transform2D> {
        node.world_transform().then(&self.viewport.view()).ok()
    }

    fn effects(
        &self,
        pass: crate::render_graph::PassId,
        draw: &PlannedDraw,
    ) -> Result<&DrawEffects, EncodeError> {
        self.plan
            .draw_effects(pass, draw.depth)
            .ok_or(EncodeError::Plan("draw 효과가 없습니다"))
    }

    fn draw(
        &self,
        pass: crate::render_graph::PassId,
        draw: &PlannedDraw,
    ) -> Result<Vec<Op>, EncodeError> {
        if draw.clip == ClipPlan::Culled {
            return Ok(Vec::new());
        }
        let effects = self.effects(pass, draw)?;
        let mut top = LayerOp::new(Vec::new(), effects.composite.mode)
            .with_opacity(draw.opacity.get() * effects.node_opacity.get());
        let content = match &draw.source {
            DrawSource::Layer { layer, node } => {
                if let Some(blur) = self.plan.layer_effects(*layer).and_then(|l| l.blur) {
                    top = top.with_blur_sigmas([blur.sigma_x, blur.sigma_y]);
                }
                let _ = node;
                self.layer(*layer)?
            }
            DrawSource::Content(node) => {
                let paint = node.paint().get();
                let mut border_box: Option<Arc<Mesh>> = None;
                for shadow in &effects.shadows {
                    let color = paint
                        .shadows
                        .get(shadow.index)
                        .map(|s| OutputColor::from_paint(Paint::Solid(s.color())))
                        .ok_or(EncodeError::Plan("shadow 번호가 paint stack 밖입니다"))?;
                    let source = match shadow.shape {
                        ShadowShape::ContentAlpha => ShadowSource::Content,
                        ShadowShape::BorderBox => {
                            let mesh = match &border_box {
                                Some(mesh) => Arc::clone(mesh),
                                None => {
                                    let mesh = Arc::new(self.border_box(node)?);
                                    border_box = Some(Arc::clone(&mesh));
                                    mesh
                                }
                            };
                            ShadowSource::Box(mesh)
                        }
                    };
                    // offset은 소수 device pixel 그대로 둡니다(합성이 bilinear로 읽음, ADR-0130 결정 3).
                    top = top.with_shadow_sigmas(
                        shadow.offset,
                        [shadow.blur.sigma_x, shadow.blur.sigma_y],
                        color,
                        source,
                    );
                }
                self.content(node)?
            }
        };
        top.content = content;
        self.clipped(draw.source.node(), &draw.clip, top)
    }

    /// layer pass의 draw를 painter 순서로 낮춥니다.
    fn layer(&self, layer: LayerId) -> Result<Vec<Op>, EncodeError> {
        let pass = self
            .plan
            .passes
            .iter()
            .find(|pass| pass.layer == Some(layer))
            .ok_or(EncodeError::Plan("layer pass가 없습니다"))?;
        let mut draws: Vec<&PlannedDraw> = pass.draws.iter().collect();
        draws.sort_by_key(|draw| draw.depth);
        let mut ops = Vec::new();
        for draw in draws {
            ops.extend(self.draw(pass.pass, draw)?);
        }
        Ok(ops)
    }

    /// `top`(내용·효과·불투명도·blend)에 clip을 적용합니다(모듈 문서 "clip").
    fn clipped(
        &self,
        node: &SceneNode,
        clip: &ClipPlan,
        mut top: LayerOp,
    ) -> Result<Vec<Op>, EncodeError> {
        let trivial = |layer: &LayerOp| {
            layer.blend == SceneBlendMode::Normal
                && layer.opacity >= 1.0
                && layer.shadows.is_empty()
                && layer.blur <= 0.0
        };
        let (scissor, complex) = match clip {
            ClipPlan::None => {
                return Ok(if trivial(&top) {
                    top.content
                } else {
                    vec![Op::Layer(Box::new(top))]
                });
            }
            ClipPlan::Culled => return Ok(Vec::new()),
            ClipPlan::Scissor(scissor) => (*scissor, None),
            ClipPlan::Stencil { scissor, clips, .. } | ClipPlan::Mask { scissor, clips } => {
                (*scissor, Some(clips.clone()))
            }
        };
        let scissor = ClipSpec::Scissor {
            x: scissor.x,
            y: scissor.y,
            width: scissor.width,
            height: scissor.height,
        };
        let Some(clips) = complex else {
            top.clip = Some(scissor);
            return Ok(vec![Op::Layer(Box::new(top))]);
        };
        // shadow·blur는 clip보다 먼저입니다. 안쪽 layer에서 끝내고 clip은 그 결과에 겁니다.
        let mut inner = top.content;
        top.content = Vec::new();
        if !top.shadows.is_empty() || top.blur > 0.0 {
            let mut effects = LayerOp::new(inner, SceneBlendMode::Normal);
            effects.shadows = std::mem::take(&mut top.shadows);
            effects.blur = std::mem::replace(&mut top.blur, 0.0);
            effects.blur_axis_scale = top.blur_axis_scale;
            inner = vec![Op::Layer(Box::new(effects))];
        }
        let shapes = node.clip().to_outer_first();
        let mut stencils = Vec::with_capacity(clips.len());
        for index in clips.iter() {
            let shape = shapes
                .get(*index)
                .ok_or(EncodeError::Plan("clip index가 clip stack 밖입니다"))?;
            stencils.push(self.clip_mesh(node, shape)?);
        }
        inner = vec![Op::Layer(Box::new(
            LayerOp::new(inner, SceneBlendMode::Normal).with_clip(scissor),
        ))];
        let last = stencils.pop();
        for mesh in stencils {
            inner = vec![Op::Layer(Box::new(
                LayerOp::new(inner, SceneBlendMode::Normal).with_clip(ClipSpec::Stencil(mesh)),
            ))];
        }
        top.content = inner;
        top.clip = last.map(ClipSpec::Stencil);
        Ok(vec![Op::Layer(Box::new(top))])
    }

    fn clip_mesh(&self, node: &SceneNode, shape: &ClipShape) -> Result<Mesh, EncodeError> {
        let view = self.viewport.view();
        let (path, rule, transform) = match shape {
            ClipShape::Rect {
                rect,
                radii,
                transform,
            } => (
                rounded_rect_path(*rect, radii.components())
                    .ok_or(unsupported(node, "clip 사각형 값"))?,
                FillRule::NonZero,
                *transform,
            ),
            ClipShape::Path {
                geometry,
                fill_rule,
                transform,
            } => (geometry.clone(), *fill_rule, *transform),
        };
        let to_device = transform
            .then(&view)
            .map_err(|_| unsupported(node, "clip 변환 값"))?;
        let device = transform_path(&path, &to_device).ok_or(unsupported(node, "clip 변환 값"))?;
        fill_mesh(&device, rule).map_err(tessellation(node))
    }

    /// Frame 테두리 상자(모서리 반경 포함)의 device fill mesh(CSS `box-shadow` 원본, ADR-0130 결정 1).
    fn border_box(&self, node: &SceneNode) -> Result<Mesh, EncodeError> {
        let SceneContent::Rect(rect) = node.content() else {
            return Err(EncodeError::Plan("상자 shadow는 사각형 내용만 갖습니다"));
        };
        let to_device = self
            .to_device(node)
            .ok_or(unsupported(node, "node 변환 값"))?;
        let path = rounded_rect_path(rect.rect, rect.radii.components())
            .ok_or(unsupported(node, "사각형 값"))?;
        let device = transform_path(&path, &to_device).ok_or(unsupported(node, "변환 값"))?;
        fill_mesh(&device, FillRule::NonZero).map_err(tessellation(node))
    }

    /// node 자기 내용(이미지 pixel·text run 색 → paint layer).
    fn content(&self, node: &SceneNode) -> Result<Vec<Op>, EncodeError> {
        let to_device = self
            .to_device(node)
            .ok_or(unsupported(node, "node 변환 값"))?;
        let mut ops = Vec::new();
        // paint layer가 칠할 local geometry와 채우기 규칙.
        let (geometry, rule) = match node.content() {
            SceneContent::Rect(rect) => (
                rounded_rect_path(rect.rect, rect.radii.components())
                    .ok_or(unsupported(node, "사각형 값"))?,
                FillRule::NonZero,
            ),
            SceneContent::Path(path) => (path.geometry.clone(), path.fill_rule),
            SceneContent::Image(image) => {
                if let Some(op) = self.image(node, image, &to_device)? {
                    ops.push(op);
                }
                (
                    rect_path(image.bounds).ok_or(unsupported(node, "이미지 상자 값"))?,
                    FillRule::NonZero,
                )
            }
            SceneContent::Text(text) => {
                let glyphs = self.resources.text(node, text)?.unwrap_or_default();
                let mut all = Vec::new();
                for glyph in &glyphs {
                    let device = transform_path(&glyph.geometry, &to_device)
                        .ok_or(unsupported(node, "glyph 변환 값"))?;
                    let mesh = fill_mesh(&device, FillRule::NonZero).map_err(tessellation(node))?;
                    ops.push(solid(mesh, glyph.color, Coverage::Disjoint));
                    all.extend(glyph.geometry.commands().iter().copied());
                }
                (
                    PathGeometry::new(all).map_err(|_| unsupported(node, "glyph 모양"))?,
                    FillRule::NonZero,
                )
            }
        };
        let paint = node.paint().get();
        if paint.layers.is_empty() {
            return Ok(ops);
        }
        let device = transform_path(&geometry, &to_device).ok_or(unsupported(node, "변환 값"))?;
        let fill = fill_mesh(&device, rule).map_err(tessellation(node))?;
        let (a, b, c, d, _, _) = to_device.components();
        let scale = (a * d - b * c).abs().sqrt().max(1e-6);
        for layer in &paint.layers {
            match layer {
                PaintLayer::Fill(paint) => ops.push(solid(
                    fill.clone(),
                    OutputColor::from_paint(*paint),
                    Coverage::Disjoint,
                )),
                PaintLayer::Stroke(stroke) => {
                    let tolerance = FlattenTolerance::new(DEVICE_TOLERANCE / scale)
                        .unwrap_or(FlattenTolerance::DEFAULT);
                    let style = StrokeStyle::from_layer(stroke)
                        .map_err(|error| EncodeError::Tessellation {
                            node: node.source(),
                            error,
                        })?
                        .with_tolerance(tolerance);
                    let local = tessellate_stroke(&geometry, &style).map_err(|error| {
                        EncodeError::Tessellation {
                            node: node.source(),
                            error,
                        }
                    })?;
                    let mut mesh = Mesh::from_fill_mesh(&local);
                    for p in &mut mesh.positions {
                        let mapped = to_device
                            .apply(
                                Vec2::new(p[0], p[1])
                                    .map_err(|_| unsupported(node, "stroke 값"))?,
                            )
                            .map_err(|_| unsupported(node, "stroke 값"))?;
                        *p = [mapped.x(), mapped.y()];
                    }
                    let color = OutputColor::from_paint(stroke.paint());
                    let union = solid(mesh.clone(), color, Coverage::Union);
                    match stroke.align() {
                        StrokeAlign::Center => ops.push(union),
                        StrokeAlign::Inside => ops.push(Op::Layer(Box::new(
                            LayerOp::new(vec![union], SceneBlendMode::Normal)
                                .with_clip(ClipSpec::Stencil(fill.clone())),
                        ))),
                        StrokeAlign::Outside => {
                            let outside = self.outside(node, &device, rule, &mesh)?;
                            ops.push(Op::Layer(Box::new(
                                LayerOp::new(vec![union], SceneBlendMode::Normal)
                                    .with_clip(ClipSpec::Stencil(outside)),
                            )));
                        }
                    }
                }
            }
        }
        Ok(ops)
    }

    /// stroke 경계 상자에서 geometry를 뺀 영역 mesh(even-odd 구멍).
    fn outside(
        &self,
        node: &SceneNode,
        device: &PathGeometry,
        rule: FillRule,
        stroke: &Mesh,
    ) -> Result<Mesh, EncodeError> {
        if rule != FillRule::NonZero && rule != FillRule::EvenOdd {
            return Err(unsupported(node, "채우기 규칙"));
        }
        let Some([x0, y0, x1, y1]) = mesh_bounds(stroke) else {
            return Ok(Mesh::default());
        };
        let frame = Rect::new(
            Vec2::new(x0 - 1.0, y0 - 1.0).map_err(|_| unsupported(node, "stroke 값"))?,
            stellar_scene::Size2::new(x1 - x0 + 2.0, y1 - y0 + 2.0)
                .map_err(|_| unsupported(node, "stroke 값"))?,
        );
        let mut commands = rect_path(frame)
            .ok_or(unsupported(node, "stroke 값"))?
            .commands()
            .to_vec();
        commands.extend(device.commands().iter().copied());
        let path = PathGeometry::new(commands).map_err(|_| unsupported(node, "stroke 값"))?;
        fill_mesh(&path, FillRule::EvenOdd).map_err(tessellation(node))
    }

    /// 이미지 quad(축 정렬 변환만).
    fn image(
        &self,
        node: &SceneNode,
        image: &ImagePrimitive,
        to_device: &Transform2D,
    ) -> Result<Option<Op>, EncodeError> {
        let Some(data) = self.resources.image(node, image)? else {
            return Ok(None);
        };
        let (a, b, c, d, tx, ty) = to_device.components();
        if b != 0.0 || c != 0.0 || a <= 0.0 || d <= 0.0 {
            return Err(unsupported(node, "축 정렬이 아닌 이미지 변환"));
        }
        let (cx, cy, cw, ch) = image.crop.components();
        let (tw, th) = (data.width as f32, data.height as f32);
        let (px, py, pw, ph) = (cx * tw, cy * th, cw * tw, ch * th);
        let bounds = image.bounds;
        let (bx, by) = (bounds.origin().x(), bounds.origin().y());
        let (bw, bh) = (bounds.size().width(), bounds.size().height());
        // 배치는 scene `ImageFit::place`(CSS object-fit)이고 원본 크기는 crop 영역입니다.
        let Some(placement) = image.fit.place([bx, by, bw, bh], [pw, ph]) else {
            return Ok(None);
        };
        let [dx, dy, dw, dh] = placement.dest;
        let (vx0, vy0) = (dx.max(bx), dy.max(by));
        let (vx1, vy1) = ((dx + dw).min(bx + bw), (dy + dh).min(by + bh));
        if vx1 <= vx0 || vy1 <= vy0 || dw <= 0.0 || dh <= 0.0 {
            return Ok(None);
        }
        let u = |x: f32| (px + (x - dx) * pw / dw) / tw;
        let v = |y: f32| (py + (y - dy) * ph / dh) / th;
        Ok(Some(Op::Image(ImageOp {
            rect: [a * vx0 + tx, d * vy0 + ty, a * vx1 + tx, d * vy1 + ty],
            uv: [u(vx0), v(vy0), u(vx1), v(vy1)],
            image: data,
            filter: match image.sampling {
                stellar_scene::node::ImageSampling::Nearest => TextureFilter::Nearest,
                stellar_scene::node::ImageSampling::Linear => TextureFilter::Linear,
            },
            wrap: TextureWrap::ClampToEdge,
            opacity: 1.0,
        })))
    }
}

#[cfg(test)]
mod tests;
