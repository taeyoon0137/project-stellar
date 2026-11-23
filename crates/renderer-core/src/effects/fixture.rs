//! backend 중립 적합성 fixture 모델 (XI.11.1, ADR-0102 결정 6).
//!
//! fixture 하나는 크기·surface 색공간·target format·clear 색과 그리기 목록([`Op`])입니다. 같은 fixture를 CPU 기준
//! rasterizer([`super::raster`])와 backend lowering([`super::lower`])이 읽습니다. 색은 문서·scene 값([`OutputColor`])으로
//! 적고, 두 경로 모두 [`Fixture::plan`]과 [`crate::color::to_working`]으로 working 값을 얻습니다.
//!
//! 좌표는 target pixel 좌표(좌상단 원점, y 아래로)입니다. mesh는 삼각형 목록입니다.

use std::sync::Arc;

use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::Vec2;

/// fixture 작성에 쓰는 scene 값 재노출(backend crate는 scene을 직접 의존하지 않음).
pub use stellar_scene::geometry::FillRule;
pub use stellar_scene::stack::{SceneBlendMode, StrokeCap};

use crate::backend::{BackendKind, DeviceCapabilities};
use crate::color::{encode_pixel, image_texel, ColorSpace, WorkingColor};
use crate::output_color::{OutputColor, OutputColorPlan};
use crate::placeholder::TextureFilter;
use crate::render_graph::TextureFormat;
use crate::stroke::{tessellate_stroke, StrokeStyle};
use crate::tessellation::{FillMesh, FillStyle, FillTessellator, TessellationError};

use super::fill::SweepFillTessellator;
use super::gradient::{GradientError, GradientGeometry, GradientStop};

/// fixture 분류(XI.11.3 inventory 단위).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Frame(사각형 fill·모서리·테두리).
    Frame,
    /// Shape(타원·다각형·별·선).
    Shape,
    /// Path(fill 규칙·곡선·stroke join·cap·dash).
    Path,
    /// Text(glyph coverage 합성).
    Text,
    /// Image(texture sampling).
    Image,
    /// Modifier(opacity·blend·shadow·blur).
    Modifier,
    /// clip(scissor·stencil·mask).
    Clip,
    /// Display P3 색·surface.
    P3,
}

impl Category {
    /// 모든 분류.
    pub const ALL: [Category; 8] = [
        Category::Frame,
        Category::Shape,
        Category::Path,
        Category::Text,
        Category::Image,
        Category::Modifier,
        Category::Clip,
        Category::P3,
    ];

    /// 정적 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Category::Frame => "frame",
            Category::Shape => "shape",
            Category::Path => "path",
            Category::Text => "text",
            Category::Image => "image",
            Category::Modifier => "modifier",
            Category::Clip => "clip",
            Category::P3 => "p3",
        }
    }
}

/// fixture가 검사하는 렌더 기능(기능별 독립 fixture 색인, XI.10.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    /// 단색 mesh fill.
    SolidFill,
    /// 겹치는 mesh를 pixel마다 한 번 칠하는 stencil union(stroke).
    StrokeUnion,
    /// 이미지 sampling.
    Image,
    /// glyph coverage mask.
    Glyph,
    /// Display P3 입력 또는 surface.
    DisplayP3,
    /// offscreen layer 합성과 blend mode(XI.10.2).
    Blend,
    /// gradient fill(XI.10.3).
    Gradient,
    /// 합성 불투명도(XI.10.4).
    Opacity,
    /// scissor clip(XI.10.4).
    ScissorClip,
    /// stencil path clip(XI.10.4).
    StencilClip,
    /// coverage mask clip(XI.10.4).
    MaskClip,
    /// mask(XI.10.4).
    Mask,
    /// drop shadow(XI.10.4).
    Shadow,
    /// layer blur(XI.10.4).
    Blur,
}

/// mesh 겹침 처리.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coverage {
    /// 삼각형이 겹치지 않음(그대로 그림).
    Disjoint,
    /// 삼각형이 겹칠 수 있음(pixel마다 한 번만 칠함).
    Union,
}

/// 삼각형 목록 mesh(pixel 좌표, 3개씩).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    /// 삼각형 정점.
    pub positions: Vec<[f32; 2]>,
}

impl Mesh {
    /// 축 정렬 사각형.
    pub fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Mesh {
        Mesh {
            positions: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]],
        }
    }

    /// tessellation 결과를 삼각형 목록으로 펼칩니다.
    pub fn from_fill_mesh(mesh: &FillMesh) -> Mesh {
        Mesh {
            positions: mesh
                .indices()
                .iter()
                .map(|&i| {
                    let v = mesh.vertices()[i as usize];
                    [v.x(), v.y()]
                })
                .collect(),
        }
    }

    /// 삼각형 수.
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }
}

/// fill paint.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// 단색.
    Solid(OutputColor),
    /// gradient(stop은 시각 순서, XI.10.3). GPU 경로는 선형 gradient만 있습니다.
    Gradient {
        /// 기하.
        geometry: GradientGeometry,
        /// 시각 순서 stop.
        stops: Vec<GradientStop>,
    },
}

/// mesh fill.
#[derive(Debug, Clone, PartialEq)]
pub struct FillOp {
    /// 도형.
    pub mesh: Mesh,
    /// paint.
    pub paint: Paint,
    /// 겹침 처리.
    pub coverage: Coverage,
}

/// decoded 이미지(straight alpha, 비선형 8-bit RGBA, XI.7.1 규약).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// 이미지 색공간.
    pub space: ColorSpace,
    /// texel(행 우선).
    pub texels: Vec<[u8; 4]>,
}

impl ImageData {
    /// working 값을 `Rgba8UnormSrgb` texture에 올릴 byte(transfer encode한 premultiplied 값, ADR-0102 결정 1).
    pub fn texture_bytes(&self, plan: &OutputColorPlan) -> Vec<u8> {
        self.texels
            .iter()
            .flat_map(|texel| {
                encode_pixel(
                    image_texel(plan, self.space, *texel),
                    IMAGE_TEXTURE_FORMAT,
                    plan.surface(),
                )
                .expect("color format")
            })
            .collect()
    }
}

/// 이미지 texture format.
pub const IMAGE_TEXTURE_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

/// 이미지 quad.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageOp {
    /// 그릴 사각형(x0, y0, x1, y1).
    pub rect: [f32; 4],
    /// texture 좌표(u0, v0, u1, v1).
    pub uv: [f32; 4],
    /// 이미지.
    pub image: Arc<ImageData>,
    /// sampling filter(가장자리는 clamp).
    pub filter: TextureFilter,
    /// 곱할 불투명도.
    pub opacity: f32,
}

/// glyph coverage mask(8-bit, 행 우선).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageMask {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// coverage.
    pub data: Vec<u8>,
}

/// glyph run 하나(정수 pixel 원점에 mask를 1:1로 놓음).
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphOp {
    /// mask 좌상단 pixel.
    pub origin: [i32; 2],
    /// coverage mask.
    pub mask: Arc<CoverageMask>,
    /// 글자 색.
    pub color: OutputColor,
}

/// offscreen layer: 내용을 투명 layer에 그린 뒤 부모에 합성합니다(ADR-0102 결정 3·5).
#[derive(Debug, Clone, PartialEq)]
pub struct LayerOp {
    /// layer 내용(painter 순서).
    pub content: Vec<Op>,
    /// 합성 blend mode.
    pub blend: SceneBlendMode,
    /// 합성 불투명도(`[0, 1]`).
    pub opacity: f32,
    /// drop shadow(앞이 아래, XI.10.4).
    pub shadows: Vec<ShadowSpec>,
    /// layer blur 반경(CSS `filter: blur()`, σ = 반경).
    pub blur: f32,
    /// clip(합성 단계에서 적용).
    pub clip: Option<ClipSpec>,
    /// mask coverage 도형(비면 mask 없음). clip의 [`ClipSpec::Mask`]와 함께 쓰지 않습니다.
    pub mask: Vec<CoverageShape>,
}

impl LayerOp {
    /// `content`를 `blend`로 합성하는 layer(불투명도 1, 효과 없음).
    pub fn new(content: Vec<Op>, blend: SceneBlendMode) -> LayerOp {
        LayerOp {
            content,
            blend,
            opacity: 1.0,
            shadows: Vec::new(),
            blur: 0.0,
            clip: None,
            mask: Vec::new(),
        }
    }

    /// 불투명도.
    pub fn with_opacity(mut self, opacity: f32) -> LayerOp {
        self.opacity = opacity;
        self
    }

    /// drop shadow를 덧붙입니다.
    pub fn with_shadow(mut self, offset: [i32; 2], blur: f32, color: OutputColor) -> LayerOp {
        self.shadows.push(ShadowSpec {
            offset,
            blur,
            color,
        });
        self
    }

    /// layer blur 반경.
    pub fn with_blur(mut self, radius: f32) -> LayerOp {
        self.blur = radius;
        self
    }

    /// clip.
    pub fn with_clip(mut self, clip: ClipSpec) -> LayerOp {
        self.clip = Some(clip);
        self
    }

    /// mask 도형을 덧붙입니다.
    pub fn with_mask(mut self, mesh: Mesh, coverage: f32) -> LayerOp {
        self.mask.push(CoverageShape { mesh, coverage });
        self
    }
}

/// drop shadow(CSS `drop-shadow()` 의미: 내용 alpha의 blur).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    /// 정수 pixel offset(harness는 정수 offset만 씁니다).
    pub offset: [i32; 2],
    /// blur 반경(σ = 반경 / 2).
    pub blur: f32,
    /// 그림자 색.
    pub color: OutputColor,
}

/// coverage 도형(mask·clip mask).
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageShape {
    /// 도형(삼각형이 겹치지 않음).
    pub mesh: Mesh,
    /// coverage(`[0, 1]`).
    pub coverage: f32,
}

/// layer clip.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipSpec {
    /// device pixel 사각형 scissor.
    Scissor {
        /// 왼쪽.
        x: u32,
        /// 위.
        y: u32,
        /// 너비.
        width: u32,
        /// 높이.
        height: u32,
    },
    /// path stencil(pixel 중심 판정, 삼각형이 겹치지 않는 mesh).
    Stencil(Mesh),
    /// coverage mask clip(도형마다 coverage, 겹치면 합집합).
    Mask(Vec<CoverageShape>),
}

/// 그리기 하나.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// mesh fill.
    Fill(FillOp),
    /// 이미지.
    Image(ImageOp),
    /// glyph.
    Glyphs(GlyphOp),
    /// offscreen layer.
    Layer(Box<LayerOp>),
}

/// fixture 정의 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum FixtureError {
    /// target format은 8-bit RGBA(`Rgba8UnormSrgb`·`Rgba8Unorm`)만 씁니다.
    UnsupportedFormat(TextureFormat),
    /// 크기가 0이거나 너무 큽니다.
    InvalidSize,
    /// 값이 계약을 벗어남.
    Invalid(&'static str),
    /// tessellation 실패.
    Tessellation(TessellationError),
    /// gradient stop 오류.
    Gradient(GradientError),
}

impl From<TessellationError> for FixtureError {
    fn from(error: TessellationError) -> FixtureError {
        FixtureError::Tessellation(error)
    }
}

/// fixture 크기 상한(한 변 pixel).
pub const MAX_FIXTURE_SIZE: u32 = 512;

/// 적합성 fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    /// 안정 이름(artifact 파일 이름).
    pub name: String,
    /// 분류.
    pub category: Category,
    /// 검사 기능.
    pub features: Vec<Feature>,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// 요청 출력 색공간.
    pub requested: ColorSpace,
    /// device Display P3 surface 지원 여부(capability 모사).
    pub display_p3: bool,
    /// target format.
    pub format: TextureFormat,
    /// clear 색.
    pub clear: OutputColor,
    /// 그리기 목록(painter 순서).
    pub ops: Vec<Op>,
    /// 허용 오차 class 이름(XI.11.2 config).
    pub tolerance: &'static str,
}

impl Fixture {
    /// sRGB surface, `Rgba8UnormSrgb`, 불투명 흰 배경 fixture.
    pub fn new(name: impl Into<String>, category: Category, width: u32, height: u32) -> Fixture {
        Fixture {
            name: name.into(),
            category,
            features: Vec::new(),
            width,
            height,
            requested: ColorSpace::Srgb,
            display_p3: false,
            format: TextureFormat::Rgba8UnormSrgb,
            clear: srgb(1.0, 1.0, 1.0, 1.0),
            ops: Vec::new(),
            tolerance: "default",
        }
    }

    /// 출력 색 변환 규칙(capability 모사로 협상).
    pub fn plan(&self) -> OutputColorPlan {
        OutputColorPlan::negotiate(
            self.requested,
            &DeviceCapabilities {
                backend: BackendKind::Mock,
                max_texture_dimension: crate::render_graph::MAX_TEXTURE_DIMENSION,
                display_p3_surface: self.display_p3,
                timestamp_query: false,
                msaa_x4: false,
                stencil_bits: 8,
            },
        )
    }

    /// clear 색 working 값.
    pub fn clear_working(&self) -> WorkingColor {
        crate::color::to_working(&self.plan(), self.clear)
    }

    /// 크기·format·값 계약을 확인합니다.
    pub fn validate(&self) -> Result<(), FixtureError> {
        if !matches!(
            self.format,
            TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
        ) {
            return Err(FixtureError::UnsupportedFormat(self.format));
        }
        if self.width == 0
            || self.height == 0
            || self.width > MAX_FIXTURE_SIZE
            || self.height > MAX_FIXTURE_SIZE
        {
            return Err(FixtureError::InvalidSize);
        }
        validate_ops(&self.ops, &self.plan())
    }

    /// 그리기를 덧붙입니다.
    pub fn with(mut self, op: Op) -> Fixture {
        self.ops.push(op);
        self
    }

    /// 기능 표시를 덧붙입니다.
    pub fn feature(mut self, feature: Feature) -> Fixture {
        if !self.features.contains(&feature) {
            self.features.push(feature);
        }
        self
    }
}

fn validate_mesh(mesh: &Mesh) -> Result<(), FixtureError> {
    if !mesh.positions.len().is_multiple_of(3) {
        return Err(FixtureError::Invalid("mesh 정점 수가 3의 배수가 아닙니다"));
    }
    if mesh.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(FixtureError::Invalid("mesh 좌표가 유한하지 않습니다"));
    }
    Ok(())
}

fn validate_coverage(shapes: &[CoverageShape]) -> Result<(), FixtureError> {
    for shape in shapes {
        validate_mesh(&shape.mesh)?;
        if !(0.0..=1.0).contains(&shape.coverage) {
            return Err(FixtureError::Invalid("coverage는 [0, 1]입니다"));
        }
    }
    Ok(())
}

fn validate_layer(layer: &LayerOp, plan: &OutputColorPlan) -> Result<(), FixtureError> {
    use super::blur::{layer_blur_sigma, shadow_sigma, valid_sigma};
    if !(0.0..=1.0).contains(&layer.opacity) {
        return Err(FixtureError::Invalid("layer 불투명도는 [0, 1]입니다"));
    }
    if !valid_sigma(layer_blur_sigma(layer.blur)) {
        return Err(FixtureError::Invalid("layer blur 반경이 범위 밖입니다"));
    }
    if layer
        .shadows
        .iter()
        .any(|shadow| !valid_sigma(shadow_sigma(shadow.blur)))
    {
        return Err(FixtureError::Invalid("shadow blur 반경이 범위 밖입니다"));
    }
    match &layer.clip {
        None | Some(ClipSpec::Scissor { .. }) => {}
        Some(ClipSpec::Stencil(mesh)) => validate_mesh(mesh)?,
        Some(ClipSpec::Mask(shapes)) => {
            validate_coverage(shapes)?;
            if !layer.mask.is_empty() {
                return Err(FixtureError::Invalid(
                    "mask clip과 mask를 함께 쓰지 않습니다",
                ));
            }
        }
    }
    validate_coverage(&layer.mask)?;
    validate_ops(&layer.content, plan)
}

fn validate_ops(ops: &[Op], plan: &OutputColorPlan) -> Result<(), FixtureError> {
    for op in ops {
        match op {
            Op::Fill(fill) => {
                validate_mesh(&fill.mesh)?;
                if let Paint::Gradient { stops, .. } = &fill.paint {
                    super::gradient::GradientRamp::resolve(plan, stops)
                        .map_err(FixtureError::Gradient)?;
                }
            }
            Op::Image(image) => {
                let data = &image.image;
                if data.width == 0
                    || data.height == 0
                    || data.texels.len() != (data.width * data.height) as usize
                {
                    return Err(FixtureError::Invalid("이미지 크기와 texel 수가 다릅니다"));
                }
                if !(0.0..=1.0).contains(&image.opacity) {
                    return Err(FixtureError::Invalid("이미지 불투명도는 [0, 1]입니다"));
                }
            }
            Op::Glyphs(glyph) => {
                let mask = &glyph.mask;
                if mask.width == 0
                    || mask.height == 0
                    || mask.data.len() != (mask.width * mask.height) as usize
                {
                    return Err(FixtureError::Invalid(
                        "glyph mask 크기와 data 수가 다릅니다",
                    ));
                }
            }
            Op::Layer(layer) => validate_layer(layer, plan)?,
        }
    }
    Ok(())
}

/// sRGB 색.
pub fn srgb(r: f32, g: f32, b: f32, a: f32) -> OutputColor {
    OutputColor {
        space: ColorSpace::Srgb,
        rgb: [r, g, b],
        alpha: a,
    }
}

/// Display P3 색.
pub fn p3(r: f32, g: f32, b: f32, a: f32) -> OutputColor {
    OutputColor {
        space: ColorSpace::DisplayP3,
        rgb: [r, g, b],
        alpha: a,
    }
}

fn v(x: f32, y: f32) -> Result<Vec2, FixtureError> {
    Vec2::new(x, y).map_err(|_| FixtureError::Invalid("좌표가 유한하지 않습니다"))
}

/// path command 작성 도우미.
#[derive(Debug, Clone, Default)]
pub struct PathBuilder {
    commands: Vec<PathCommand>,
    error: Option<FixtureError>,
}

impl PathBuilder {
    /// 빈 path.
    pub fn new() -> PathBuilder {
        PathBuilder::default()
    }

    fn push(mut self, make: impl FnOnce() -> Result<PathCommand, FixtureError>) -> PathBuilder {
        if self.error.is_none() {
            match make() {
                Ok(command) => self.commands.push(command),
                Err(error) => self.error = Some(error),
            }
        }
        self
    }

    /// subpath 시작.
    pub fn move_to(self, x: f32, y: f32) -> PathBuilder {
        self.push(|| Ok(PathCommand::MoveTo(v(x, y)?)))
    }

    /// 직선.
    pub fn line_to(self, x: f32, y: f32) -> PathBuilder {
        self.push(|| Ok(PathCommand::LineTo(v(x, y)?)))
    }

    /// 3차 Bézier.
    pub fn cubic_to(self, c1: [f32; 2], c2: [f32; 2], to: [f32; 2]) -> PathBuilder {
        self.push(|| {
            Ok(PathCommand::CubicTo {
                c1: v(c1[0], c1[1])?,
                c2: v(c2[0], c2[1])?,
                to: v(to[0], to[1])?,
            })
        })
    }

    /// subpath 닫기.
    pub fn close(self) -> PathBuilder {
        self.push(|| Ok(PathCommand::Close))
    }

    /// 닫힌 다각형.
    pub fn polygon(mut self, points: &[[f32; 2]]) -> PathBuilder {
        for (i, p) in points.iter().enumerate() {
            self = if i == 0 {
                self.move_to(p[0], p[1])
            } else {
                self.line_to(p[0], p[1])
            };
        }
        self.close()
    }

    /// 타원(cubic 4개).
    pub fn ellipse(self, cx: f32, cy: f32, rx: f32, ry: f32) -> PathBuilder {
        const K: f32 = 0.552_284_8;
        let (kx, ky) = (rx * K, ry * K);
        self.move_to(cx + rx, cy)
            .cubic_to([cx + rx, cy + ky], [cx + kx, cy + ry], [cx, cy + ry])
            .cubic_to([cx - kx, cy + ry], [cx - rx, cy + ky], [cx - rx, cy])
            .cubic_to([cx - rx, cy - ky], [cx - kx, cy - ry], [cx, cy - ry])
            .cubic_to([cx + kx, cy - ry], [cx + rx, cy - ky], [cx + rx, cy])
            .close()
    }

    /// 모서리가 둥근 사각형(모서리 반지름 `r`, cubic 근사).
    pub fn rounded_rect(self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> PathBuilder {
        const K: f32 = 0.552_284_8;
        let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let k = r * K;
        self.move_to(x0 + r, y0)
            .line_to(x1 - r, y0)
            .cubic_to([x1 - r + k, y0], [x1, y0 + r - k], [x1, y0 + r])
            .line_to(x1, y1 - r)
            .cubic_to([x1, y1 - r + k], [x1 - r + k, y1], [x1 - r, y1])
            .line_to(x0 + r, y1)
            .cubic_to([x0 + r - k, y1], [x0, y1 - r + k], [x0, y1 - r])
            .line_to(x0, y0 + r)
            .cubic_to([x0, y0 + r - k], [x0 + r - k, y0], [x0 + r, y0])
            .close()
    }

    /// scene path geometry.
    pub fn build(self) -> Result<PathGeometry, FixtureError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        PathGeometry::new(self.commands).map_err(|_| FixtureError::Invalid("잘못된 path"))
    }
}

/// path fill mesh(겹침 없음).
pub fn fill_mesh(path: &PathGeometry, rule: FillRule) -> Result<Mesh, FixtureError> {
    let mesh = SweepFillTessellator.tessellate_fill(
        path,
        &FillStyle {
            fill_rule: rule,
            ..FillStyle::default()
        },
    )?;
    Ok(Mesh::from_fill_mesh(&mesh))
}

/// path stroke mesh(겹칠 수 있음, XI.6.2 기준 tessellator).
pub fn stroke_mesh(path: &PathGeometry, style: &StrokeStyle) -> Result<Mesh, FixtureError> {
    Ok(Mesh::from_fill_mesh(&tessellate_stroke(path, style)?))
}

/// 선형 gradient mesh fill 그리기(stop은 시각 순서).
pub fn linear_gradient(mesh: Mesh, start: [f32; 2], end: [f32; 2], stops: Vec<GradientStop>) -> Op {
    Op::Fill(FillOp {
        mesh,
        paint: Paint::Gradient {
            geometry: GradientGeometry::Linear { start, end },
            stops,
        },
        coverage: Coverage::Disjoint,
    })
}

/// 단색 mesh fill 그리기.
pub fn solid(mesh: Mesh, color: OutputColor, coverage: Coverage) -> Op {
    Op::Fill(FillOp {
        mesh,
        paint: Paint::Solid(color),
        coverage,
    })
}
