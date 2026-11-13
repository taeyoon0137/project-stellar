//! Scene snapshot canonical serialization (XI.1.1).
//!
//! snapshot round-trip 테스트·golden fixture·semantic hash가 쓰는 **canonical binary
//! 표현**입니다. 프로젝트 파일 포맷이 아니며(scene은 저장하지 않음) 디스크에 기록하지
//! 않습니다. 같은 snapshot 값은 `Arc` 공유 구조와 무관하게 항상 같은 bytes가 됩니다.
//!
//! 규칙은 저장 canonical primitive(V.1)와 같습니다: 정수·float은 little-endian, float은
//! finite만 허용하고 `-0.0`은 `0.0`으로 정규화된 값을 씁니다.
//!
//! ```text
//! snapshot   : magic "STSN" | version u8(=1) | document_revision u64 | scene_revision u64
//!              | root_count u32 | node*            (node는 pre-order)
//! node       : source [u8;16] | content | world_transform | layout_rect rect | clip_stack
//!              | paint_stack | z u64 | visibility u8 | pointer_events u8 | child_count u32
//! content    : tag u8 + payload
//!   1 rect   : rect | radii
//!   2 path   : fill_rule u8 | geometry(PathGeometry::canonical_bytes)
//!   3 text   : layout key [u8;16] | bounds rect
//!   4 image  : asset [u8;16] | bounds rect | fit u8 | crop(x, y, w, h f32) | sampling u8
//!              | color_space u8(0 = 없음, 그 외 ColorSpace tag)
//! clip_stack : depth u32 | clip*                   (바깥쪽 → 안쪽)
//! clip       : tag u8 + payload
//!   1 rect   : rect | radii | transform
//!   2 path   : fill_rule u8 | geometry | transform
//! paint_stack: layer_count u32 | layer* | shadow_count u32 | Shadow::to_bytes(29)*
//!              | opacity f32 | blend u8 | layer_blur f32
//! layer      : 1 fill: paint | 2 stroke: paint | width f32 | align u8 | cap u8
//! paint      : 1 solid: Color::to_bytes(17)
//! rect       : x f32 | y f32 | width f32 | height f32
//! radii      : top_left f32 | top_right f32 | bottom_right f32 | bottom_left f32
//! transform  : a f32 | b f32 | c f32 | d f32 | tx f32 | ty f32
//! ```
//!
//! decode는 신뢰하지 않는 입력을 가정합니다. 잘린 입력·잉여 bytes·미지 tag·비유한 값·
//! 음수 크기·잘못된 crop·subpath 규칙 위반·z-order 불변식 위반을 구조화 오류로 거부하고
//! panic하지 않으며, 깊은 tree도 재귀 없이 처리합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::{Color, ColorError, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::shadow::{BlurError, BlurRadius, Shadow, ShadowError};
use stellar_core_types::value::{Rect, Size2, SizeError, Transform2D, Vec2};

use crate::geometry::{
    FillRule, PathCommand, PathGeometry, PathGeometryError, TAG_CLOSE, TAG_CUBIC, TAG_LINE,
    TAG_MOVE,
};
use crate::node::{
    CornerRadii, HitTestMeta, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PathPrimitive,
    PointerEvents, RectPrimitive, SceneContent, SceneNode, SceneNodeSpec, SceneTreeError,
    SceneValueError, TextLayoutKey, TextPrimitive, Visibility, ZOrder,
};
use crate::snapshot::SceneSnapshot;
use crate::stack::{
    ClipShape, ClipStackRef, Opacity, Paint, PaintLayer, PaintStack, PaintStackRef, SceneBlendMode,
    StrokeAlign, StrokeCap, StrokeLayer,
};

/// canonical bytes 시작 magic.
pub const SCENE_CODEC_MAGIC: [u8; 4] = *b"STSN";

/// canonical 표현 버전.
pub const SCENE_CODEC_VERSION: u8 = 1;

const CONTENT_RECT: u8 = 1;
const CONTENT_PATH: u8 = 2;
const CONTENT_TEXT: u8 = 3;
const CONTENT_IMAGE: u8 = 4;

const CLIP_RECT: u8 = 1;
const CLIP_PATH: u8 = 2;

const LAYER_FILL: u8 = 1;
const LAYER_STROKE: u8 = 2;

const PAINT_SOLID: u8 = 1;

const COLOR_SPACE_NONE: u8 = 0;

/// canonical bytes를 받는 출력(byte buffer 또는 hash).
pub(crate) trait ByteSink {
    /// bytes를 순서대로 덧붙입니다.
    fn put(&mut self, bytes: &[u8]);
}

impl ByteSink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

/// decode 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneCodecError {
    /// magic이 다릅니다.
    BadMagic,
    /// 지원하지 않는 버전입니다.
    UnsupportedVersion {
        /// 발견된 버전.
        version: u8,
    },
    /// 입력이 도중에 끊겼습니다.
    Truncated,
    /// snapshot을 다 읽은 뒤 bytes가 남았습니다.
    TrailingBytes {
        /// 남은 byte 수.
        extra: usize,
    },
    /// 알 수 없는 enum tag입니다.
    UnknownTag {
        /// field 이름.
        field: &'static str,
        /// 발견된 tag.
        tag: u8,
    },
    /// 값 규칙 위반(비유한·음수·범위 밖).
    InvalidValue(SceneValueError),
    /// 색상 규칙 위반.
    InvalidColor(ColorError),
    /// 그림자 규칙 위반.
    InvalidShadow(ShadowError),
    /// path subpath 규칙 위반.
    InvalidPath(PathGeometryError),
    /// z-order 불변식 위반.
    InvalidTree(SceneTreeError),
}

impl fmt::Display for SceneCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneCodecError::BadMagic => f.write_str("scene snapshot magic이 다릅니다"),
            SceneCodecError::UnsupportedVersion { version } => {
                write!(f, "지원하지 않는 scene snapshot 버전: {version}")
            }
            SceneCodecError::Truncated => f.write_str("scene snapshot 입력이 잘렸습니다"),
            SceneCodecError::TrailingBytes { extra } => {
                write!(f, "scene snapshot 뒤에 {extra} bytes가 남았습니다")
            }
            SceneCodecError::UnknownTag { field, tag } => {
                write!(f, "{field}의 알 수 없는 tag: {tag}")
            }
            SceneCodecError::InvalidValue(error) => write!(f, "잘못된 scene 값: {error}"),
            SceneCodecError::InvalidColor(error) => write!(f, "잘못된 색상: {error}"),
            SceneCodecError::InvalidShadow(error) => write!(f, "잘못된 그림자: {error}"),
            SceneCodecError::InvalidPath(error) => write!(f, "잘못된 path: {error}"),
            SceneCodecError::InvalidTree(error) => write!(f, "잘못된 scene tree: {error}"),
        }
    }
}

impl std::error::Error for SceneCodecError {}

impl From<SceneValueError> for SceneCodecError {
    fn from(error: SceneValueError) -> SceneCodecError {
        SceneCodecError::InvalidValue(error)
    }
}

impl From<SceneTreeError> for SceneCodecError {
    fn from(error: SceneTreeError) -> SceneCodecError {
        SceneCodecError::InvalidTree(error)
    }
}

// --- encode ---

/// snapshot을 canonical bytes로 인코딩합니다.
pub fn encode(snapshot: &SceneSnapshot) -> Vec<u8> {
    let mut buf = Vec::new();
    write_snapshot(snapshot, &mut buf);
    buf
}

/// snapshot canonical bytes 전체를 `out`에 씁니다.
pub(crate) fn write_snapshot<S: ByteSink + ?Sized>(snapshot: &SceneSnapshot, out: &mut S) {
    out.put(&SCENE_CODEC_MAGIC);
    out.put(&[SCENE_CODEC_VERSION]);
    out.put(&snapshot.document_revision().0.to_le_bytes());
    out.put(&snapshot.scene_revision().0.to_le_bytes());
    write_body(snapshot, out);
}

/// revision을 뺀 본문(root 수 + node pre-order)을 씁니다.
pub(crate) fn write_body<S: ByteSink + ?Sized>(snapshot: &SceneSnapshot, out: &mut S) {
    out.put(&(snapshot.root_count() as u32).to_le_bytes());
    for node in snapshot.iter_preorder() {
        write_node(node, out);
    }
}

/// node 하나(자식 제외, 자식 수 포함)를 씁니다.
pub(crate) fn write_node<S: ByteSink + ?Sized>(node: &SceneNode, out: &mut S) {
    out.put(&node.source().to_bytes());
    write_content(node.content(), out);
    put_transform(out, node.world_transform());
    put_rect(out, node.layout_rect());
    write_clip_stack(node.clip(), out);
    write_paint_stack(node.paint().get(), out);
    out.put(&node.z().0.to_le_bytes());
    out.put(&[node.hit().visibility.tag(), node.hit().pointer_events.tag()]);
    out.put(&(node.child_count() as u32).to_le_bytes());
}

fn write_content<S: ByteSink + ?Sized>(content: &SceneContent, out: &mut S) {
    match content {
        SceneContent::Rect(rect) => {
            out.put(&[CONTENT_RECT]);
            put_rect(out, rect.rect);
            put_radii(out, rect.radii);
        }
        SceneContent::Path(path) => {
            out.put(&[CONTENT_PATH]);
            out.put(&[path.fill_rule.tag()]);
            path.geometry.write_canonical(out);
        }
        SceneContent::Text(text) => {
            out.put(&[CONTENT_TEXT]);
            out.put(&text.layout.to_bytes());
            put_rect(out, text.bounds);
        }
        SceneContent::Image(image) => {
            out.put(&[CONTENT_IMAGE]);
            out.put(&image.asset.id().to_bytes());
            put_rect(out, image.bounds);
            out.put(&[image.fit.tag()]);
            let (x, y, w, h) = image.crop.components();
            for v in [x, y, w, h] {
                put_f32(out, v);
            }
            out.put(&[image.sampling.tag()]);
            out.put(&[image.color_space.map_or(COLOR_SPACE_NONE, ColorSpace::tag)]);
        }
    }
}

fn write_clip_stack<S: ByteSink + ?Sized>(clip: &ClipStackRef, out: &mut S) {
    out.put(&(clip.depth() as u32).to_le_bytes());
    for shape in clip.to_outer_first() {
        match shape {
            ClipShape::Rect {
                rect,
                radii,
                transform,
            } => {
                out.put(&[CLIP_RECT]);
                put_rect(out, *rect);
                put_radii(out, *radii);
                put_transform(out, *transform);
            }
            ClipShape::Path {
                geometry,
                fill_rule,
                transform,
            } => {
                out.put(&[CLIP_PATH]);
                out.put(&[fill_rule.tag()]);
                geometry.write_canonical(out);
                put_transform(out, *transform);
            }
        }
    }
}

fn write_paint_stack<S: ByteSink + ?Sized>(paint: &PaintStack, out: &mut S) {
    out.put(&(paint.layers.len() as u32).to_le_bytes());
    for layer in &paint.layers {
        match layer {
            PaintLayer::Fill(fill) => {
                out.put(&[LAYER_FILL]);
                put_paint(out, *fill);
            }
            PaintLayer::Stroke(stroke) => {
                out.put(&[LAYER_STROKE]);
                put_paint(out, stroke.paint());
                put_f32(out, stroke.width());
                out.put(&[stroke.align().tag(), stroke.cap().tag()]);
            }
        }
    }
    out.put(&(paint.shadows.len() as u32).to_le_bytes());
    for shadow in &paint.shadows {
        out.put(&shadow.to_bytes());
    }
    put_f32(out, paint.opacity.get());
    out.put(&[paint.blend.tag()]);
    put_f32(out, paint.layer_blur.get());
}

fn put_paint<S: ByteSink + ?Sized>(out: &mut S, paint: Paint) {
    match paint {
        Paint::Solid(color) => {
            out.put(&[PAINT_SOLID]);
            out.put(&color.to_bytes());
        }
    }
}

pub(crate) fn put_f32<S: ByteSink + ?Sized>(out: &mut S, value: f32) {
    out.put(&value.to_le_bytes());
}

pub(crate) fn put_rect<S: ByteSink + ?Sized>(out: &mut S, rect: Rect) {
    put_f32(out, rect.origin().x());
    put_f32(out, rect.origin().y());
    put_f32(out, rect.size().width());
    put_f32(out, rect.size().height());
}

fn put_radii<S: ByteSink + ?Sized>(out: &mut S, radii: CornerRadii) {
    let (tl, tr, br, bl) = radii.components();
    for v in [tl, tr, br, bl] {
        put_f32(out, v);
    }
}

pub(crate) fn put_transform<S: ByteSink + ?Sized>(out: &mut S, transform: Transform2D) {
    let (a, b, c, d, tx, ty) = transform.components();
    for v in [a, b, c, d, tx, ty] {
        put_f32(out, v);
    }
}

// --- decode ---

/// 신뢰하지 않는 입력용 byte reader.
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    pub(crate) fn take<const N: usize>(&mut self) -> Result<[u8; N], SceneCodecError> {
        let end = self.pos.checked_add(N).ok_or(SceneCodecError::Truncated)?;
        let slice = self
            .input
            .get(self.pos..end)
            .ok_or(SceneCodecError::Truncated)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, SceneCodecError> {
        Ok(self.take::<1>()?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32, SceneCodecError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, SceneCodecError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    pub(crate) fn f32(&mut self, field: &'static str) -> Result<f32, SceneCodecError> {
        let value = f32::from_le_bytes(self.take()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(SceneValueError::NonFinite { field }.into())
        }
    }

    pub(crate) fn point(&mut self, field: &'static str) -> Result<Vec2, SceneCodecError> {
        let x = self.f32(field)?;
        let y = self.f32(field)?;
        Vec2::new(x, y).map_err(|_| SceneValueError::NonFinite { field }.into())
    }

    pub(crate) fn rect(&mut self, field: &'static str) -> Result<Rect, SceneCodecError> {
        let origin = self.point(field)?;
        let width = self.f32(field)?;
        let height = self.f32(field)?;
        let size = Size2::new(width, height).map_err(|error| match error {
            SizeError::NonFinite => SceneValueError::NonFinite { field },
            SizeError::Negative => SceneValueError::Negative { field },
        })?;
        Ok(Rect::new(origin, size))
    }

    pub(crate) fn radii(&mut self) -> Result<CornerRadii, SceneCodecError> {
        let tl = self.f32("corner radius")?;
        let tr = self.f32("corner radius")?;
        let br = self.f32("corner radius")?;
        let bl = self.f32("corner radius")?;
        Ok(CornerRadii::new(tl, tr, br, bl)?)
    }

    pub(crate) fn transform(
        &mut self,
        field: &'static str,
    ) -> Result<Transform2D, SceneCodecError> {
        let mut v = [0f32; 6];
        for slot in &mut v {
            *slot = self.f32(field)?;
        }
        Transform2D::new(v[0], v[1], v[2], v[3], v[4], v[5])
            .map_err(|_| SceneValueError::NonFinite { field }.into())
    }

    pub(crate) fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }
}

/// decode 중인 node(자식을 모으는 중).
struct Pending {
    spec: SceneNodeSpec,
    remaining: u32,
    children: Vec<Arc<SceneNode>>,
}

/// canonical bytes에서 snapshot을 복원합니다.
pub fn decode(bytes: &[u8]) -> Result<SceneSnapshot, SceneCodecError> {
    let mut reader = Reader::new(bytes);
    if reader.take::<4>()? != SCENE_CODEC_MAGIC {
        return Err(SceneCodecError::BadMagic);
    }
    let version = reader.u8()?;
    if version != SCENE_CODEC_VERSION {
        return Err(SceneCodecError::UnsupportedVersion { version });
    }
    let document_revision = DocumentRevision(reader.u64()?);
    let scene_revision = SceneRevision(reader.u64()?);
    let root_count = reader.u32()?;
    let mut roots = Vec::new();
    for _ in 0..root_count {
        roots.push(read_subtree(&mut reader)?);
    }
    if reader.remaining() != 0 {
        return Err(SceneCodecError::TrailingBytes {
            extra: reader.remaining(),
        });
    }
    Ok(SceneSnapshot::new(
        document_revision,
        scene_revision,
        roots,
    )?)
}

/// subtree 하나를 명시적 stack으로 읽습니다(재귀 없음).
fn read_subtree(reader: &mut Reader<'_>) -> Result<Arc<SceneNode>, SceneCodecError> {
    let mut stack = vec![read_pending(reader)?];
    loop {
        let Some(top) = stack.last_mut() else {
            return Err(SceneCodecError::Truncated);
        };
        if top.remaining > 0 {
            top.remaining -= 1;
            let child = read_pending(reader)?;
            stack.push(child);
            continue;
        }
        let Some(done) = stack.pop() else {
            return Err(SceneCodecError::Truncated);
        };
        let node = Arc::new(SceneNode::new(done.spec, done.children)?);
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => return Ok(node),
        }
    }
}

fn read_pending(reader: &mut Reader<'_>) -> Result<Pending, SceneCodecError> {
    let source = StableId::from_bytes(reader.take()?);
    let content = read_content(reader)?;
    let world_transform = reader.transform("world transform")?;
    let layout_rect = reader.rect("layout rect")?;
    let clip = read_clip_stack(reader)?;
    let paint = read_paint_stack(reader)?;
    let z = ZOrder(reader.u64()?);
    let visibility_tag = reader.u8()?;
    let visibility = Visibility::from_tag(visibility_tag).ok_or(SceneCodecError::UnknownTag {
        field: "visibility",
        tag: visibility_tag,
    })?;
    let pointer_tag = reader.u8()?;
    let pointer_events =
        PointerEvents::from_tag(pointer_tag).ok_or(SceneCodecError::UnknownTag {
            field: "pointer events",
            tag: pointer_tag,
        })?;
    let remaining = reader.u32()?;
    Ok(Pending {
        spec: SceneNodeSpec {
            source,
            content,
            world_transform,
            layout_rect,
            clip,
            paint: PaintStackRef::new(paint),
            z,
            hit: HitTestMeta {
                visibility,
                pointer_events,
            },
        },
        remaining,
        children: Vec::new(),
    })
}

fn read_content(reader: &mut Reader<'_>) -> Result<SceneContent, SceneCodecError> {
    match reader.u8()? {
        CONTENT_RECT => {
            let rect = reader.rect("rect")?;
            let radii = reader.radii()?;
            Ok(SceneContent::Rect(RectPrimitive { rect, radii }))
        }
        CONTENT_PATH => {
            let fill_rule = read_fill_rule(reader)?;
            let geometry = read_geometry(reader)?;
            Ok(SceneContent::Path(PathPrimitive {
                geometry,
                fill_rule,
            }))
        }
        CONTENT_TEXT => {
            let layout = TextLayoutKey::from_bytes(reader.take()?);
            let bounds = reader.rect("text bounds")?;
            Ok(SceneContent::Text(TextPrimitive { layout, bounds }))
        }
        CONTENT_IMAGE => {
            let asset = AssetRef::new(StableId::from_bytes(reader.take()?));
            let bounds = reader.rect("image bounds")?;
            let fit_tag = reader.u8()?;
            let fit = ImageFit::from_tag(fit_tag).ok_or(SceneCodecError::UnknownTag {
                field: "image fit",
                tag: fit_tag,
            })?;
            let x = reader.f32("image crop")?;
            let y = reader.f32("image crop")?;
            let w = reader.f32("image crop")?;
            let h = reader.f32("image crop")?;
            let crop = ImageCrop::new(x, y, w, h)?;
            let sampling_tag = reader.u8()?;
            let sampling =
                ImageSampling::from_tag(sampling_tag).ok_or(SceneCodecError::UnknownTag {
                    field: "image sampling",
                    tag: sampling_tag,
                })?;
            let space_tag = reader.u8()?;
            let color_space = if space_tag == COLOR_SPACE_NONE {
                None
            } else {
                Some(
                    ColorSpace::from_tag(space_tag).ok_or(SceneCodecError::UnknownTag {
                        field: "color space",
                        tag: space_tag,
                    })?,
                )
            };
            Ok(SceneContent::Image(ImagePrimitive {
                asset,
                bounds,
                fit,
                crop,
                sampling,
                color_space,
            }))
        }
        tag => Err(SceneCodecError::UnknownTag {
            field: "scene content",
            tag,
        }),
    }
}

fn read_fill_rule(reader: &mut Reader<'_>) -> Result<FillRule, SceneCodecError> {
    let tag = reader.u8()?;
    FillRule::from_tag(tag).ok_or(SceneCodecError::UnknownTag {
        field: "fill rule",
        tag,
    })
}

fn read_clip_stack(reader: &mut Reader<'_>) -> Result<ClipStackRef, SceneCodecError> {
    let depth = reader.u32()?;
    let mut clip = ClipStackRef::none();
    for _ in 0..depth {
        let shape = match reader.u8()? {
            CLIP_RECT => ClipShape::Rect {
                rect: reader.rect("clip rect")?,
                radii: reader.radii()?,
                transform: reader.transform("clip transform")?,
            },
            CLIP_PATH => ClipShape::Path {
                fill_rule: read_fill_rule(reader)?,
                geometry: read_geometry(reader)?,
                transform: reader.transform("clip transform")?,
            },
            tag => {
                return Err(SceneCodecError::UnknownTag {
                    field: "clip shape",
                    tag,
                })
            }
        };
        clip = clip.push(shape);
    }
    Ok(clip)
}

fn read_paint_stack(reader: &mut Reader<'_>) -> Result<PaintStack, SceneCodecError> {
    let layer_count = reader.u32()?;
    let mut layers = Vec::new();
    for _ in 0..layer_count {
        let layer = match reader.u8()? {
            LAYER_FILL => PaintLayer::Fill(read_paint(reader)?),
            LAYER_STROKE => {
                let paint = read_paint(reader)?;
                let width = reader.f32("stroke width")?;
                let align_tag = reader.u8()?;
                let align =
                    StrokeAlign::from_tag(align_tag).ok_or(SceneCodecError::UnknownTag {
                        field: "stroke align",
                        tag: align_tag,
                    })?;
                let cap_tag = reader.u8()?;
                let cap = StrokeCap::from_tag(cap_tag).ok_or(SceneCodecError::UnknownTag {
                    field: "stroke cap",
                    tag: cap_tag,
                })?;
                PaintLayer::Stroke(StrokeLayer::new(paint, width, align, cap)?)
            }
            tag => {
                return Err(SceneCodecError::UnknownTag {
                    field: "paint layer",
                    tag,
                })
            }
        };
        layers.push(layer);
    }
    let shadow_count = reader.u32()?;
    let mut shadows = Vec::new();
    for _ in 0..shadow_count {
        shadows.push(Shadow::from_bytes(reader.take()?).map_err(SceneCodecError::InvalidShadow)?);
    }
    let opacity = Opacity::new(reader.f32("opacity")?)?;
    let blend_tag = reader.u8()?;
    let blend = SceneBlendMode::from_tag(blend_tag).ok_or(SceneCodecError::UnknownTag {
        field: "blend mode",
        tag: blend_tag,
    })?;
    let layer_blur = BlurRadius::new(reader.f32("layer blur")?).map_err(|error| match error {
        BlurError::NonFinite => SceneValueError::NonFinite {
            field: "layer blur",
        },
        BlurError::Negative => SceneValueError::Negative {
            field: "layer blur",
        },
    })?;
    Ok(PaintStack {
        layers,
        shadows,
        opacity,
        blend,
        layer_blur,
    })
}

fn read_paint(reader: &mut Reader<'_>) -> Result<Paint, SceneCodecError> {
    match reader.u8()? {
        PAINT_SOLID => Ok(Paint::Solid(
            Color::from_bytes(reader.take()?).map_err(SceneCodecError::InvalidColor)?,
        )),
        tag => Err(SceneCodecError::UnknownTag {
            field: "paint",
            tag,
        }),
    }
}

/// [`PathGeometry::canonical_bytes`] 형식을 읽습니다.
pub(crate) fn read_geometry(reader: &mut Reader<'_>) -> Result<PathGeometry, SceneCodecError> {
    let count = reader.u64()?;
    // 신뢰하지 않는 개수로 미리 할당하지 않습니다(command 하나는 최소 1 byte).
    let mut commands = Vec::with_capacity((count as usize).min(reader.remaining()));
    for _ in 0..count {
        let command = match reader.u8()? {
            TAG_MOVE => PathCommand::MoveTo(reader.point("path point")?),
            TAG_LINE => PathCommand::LineTo(reader.point("path point")?),
            TAG_CUBIC => PathCommand::CubicTo {
                c1: reader.point("path point")?,
                c2: reader.point("path point")?,
                to: reader.point("path point")?,
            },
            TAG_CLOSE => PathCommand::Close,
            tag => {
                return Err(SceneCodecError::UnknownTag {
                    field: "path command",
                    tag,
                })
            }
        };
        commands.push(command);
    }
    PathGeometry::new(commands).map_err(SceneCodecError::InvalidPath)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, id, p, rect, solid, spec, FixtureConfig};

    /// 모든 content·clip·paint 종류와 중첩을 포함한 snapshot.
    fn sample_snapshot() -> SceneSnapshot {
        fixtures::small()
    }

    /// 단일 leaf snapshot(기본 spec).
    fn one(content: SceneContent) -> Vec<u8> {
        encode(
            &SceneSnapshot::new(
                DocumentRevision(1),
                SceneRevision(1),
                vec![Arc::new(SceneNode::leaf(spec(1, content, 1)))],
            )
            .unwrap(),
        )
    }

    // node content는 header(25) + source(16) 다음 offset 41에서 시작합니다.
    const CONTENT: usize = 25 + 16;
    // rect content 길이: tag 1 + rect 16 + radii 16.
    const RECT_CONTENT_LEN: usize = 1 + 16 + 16;

    #[test]
    fn snapshot_round_trips_through_canonical_bytes() {
        // 검증 기준(XI.1.1): core snapshot round-trip serialization.
        let snapshot = sample_snapshot();
        let bytes = encode(&snapshot);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, snapshot);
        // canonical: 다시 인코딩하면 같은 bytes.
        assert_eq!(encode(&decoded), bytes);
        assert_eq!(decoded.node_count(), snapshot.node_count());
        assert_eq!(decoded.resource_refs(), snapshot.resource_refs());
    }

    #[test]
    fn every_content_kind_round_trips() {
        let path = SceneContent::Path(PathPrimitive {
            geometry: PathGeometry::new(vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(10.0, -0.0)),
                PathCommand::CubicTo {
                    c1: p(10.0, 5.0),
                    c2: p(5.0, 10.0),
                    to: p(0.0, 10.0),
                },
                PathCommand::Close,
                PathCommand::MoveTo(p(2.0, 2.0)),
                PathCommand::LineTo(p(3.0, 3.0)),
            ])
            .unwrap(),
            fill_rule: FillRule::EvenOdd,
        });
        let text = SceneContent::Text(TextPrimitive {
            layout: TextLayoutKey::from_bytes([0xAB; 16]),
            bounds: rect(1.0, 2.0, 30.0, 12.0),
        });
        let image = SceneContent::Image(ImagePrimitive {
            asset: AssetRef::new(id(0x44)),
            bounds: rect(0.0, 0.0, 64.0, 32.0),
            fit: ImageFit::Cover,
            crop: ImageCrop::new(0.25, 0.0, 0.5, 1.0).unwrap(),
            sampling: ImageSampling::Nearest,
            color_space: Some(ColorSpace::DisplayP3),
        });
        for content in [fixtures::rect_content(3.0, 4.0), path, text, image] {
            let bytes = one(content.clone());
            let decoded = decode(&bytes).unwrap();
            assert_eq!(decoded.root(0).unwrap().content(), &content);
            assert_eq!(encode(&decoded), bytes);
        }
    }

    #[test]
    fn header_bytes_are_hand_checked() {
        let empty =
            SceneSnapshot::new(DocumentRevision(0x0102), SceneRevision(3), Vec::new()).unwrap();
        let bytes = encode(&empty);
        let mut expected = b"STSN".to_vec();
        expected.push(1);
        expected.extend_from_slice(&[0x02, 0x01, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(bytes, expected);
        assert_eq!(decode(&bytes).unwrap(), empty);

        // 기본 spec leaf rect 하나:
        // source 16 | content 33 | transform 24 | layout 16 | clip depth 4
        // | paint(layer 4 + shadow 4 + opacity 4 + blend 1 + blur 4) | z 8 | hit 2 | child 4.
        let bytes = one(fixtures::rect_content(8.0, 2.0));
        let node = &bytes[25..];
        assert_eq!(node.len(), 16 + 33 + 24 + 16 + 4 + 17 + 8 + 2 + 4);
        assert_eq!(&node[..16], &id(1).to_bytes());
        assert_eq!(node[16], CONTENT_RECT);
        assert_eq!(&node[17 + 8..17 + 12], &8.0f32.to_le_bytes());
        // 항등 transform.
        let t = &node[49..73];
        assert_eq!(&t[..4], &1.0f32.to_le_bytes());
        assert_eq!(&t[12..16], &1.0f32.to_le_bytes());
        // opacity 1.0, blend Normal(0), blur 0.
        let paint = &node[93..110];
        assert_eq!(&paint[8..12], &1.0f32.to_le_bytes());
        assert_eq!(paint[12], 0);
        // z = 1, visible·auto, 자식 0.
        assert_eq!(&node[110..118], &1u64.to_le_bytes());
        assert_eq!(&node[118..120], &[0, 0]);
        assert_eq!(&node[120..], &[0, 0, 0, 0]);
    }

    #[test]
    fn deep_tree_round_trips_without_recursion() {
        let mut node = Arc::new(SceneNode::leaf(spec(
            0,
            fixtures::rect_content(1.0, 1.0),
            1_001,
        )));
        for z in (1..=1_000u64).rev() {
            node = Arc::new(
                SceneNode::new(spec(0, fixtures::rect_content(1.0, 1.0), z), vec![node]).unwrap(),
            );
        }
        let snapshot =
            SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), vec![node]).unwrap();
        let bytes = encode(&snapshot);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.node_count(), 1_001);
        assert_eq!(encode(&decoded), bytes);
    }

    #[test]
    fn every_truncation_is_a_structured_error() {
        let bytes = encode(&sample_snapshot());
        for len in 0..bytes.len() {
            let result = decode(&bytes[..len]);
            assert!(result.is_err(), "길이 {len}에서 오류가 아님");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(
            decode(&extra),
            Err(SceneCodecError::TrailingBytes { extra: 1 })
        );
    }

    #[test]
    fn malformed_values_are_rejected() {
        let mut bad_magic = one(fixtures::rect_content(4.0, 4.0));
        bad_magic[0] = b'X';
        assert_eq!(decode(&bad_magic), Err(SceneCodecError::BadMagic));

        let mut bad_version = one(fixtures::rect_content(4.0, 4.0));
        bad_version[4] = 2;
        assert_eq!(
            decode(&bad_version),
            Err(SceneCodecError::UnsupportedVersion { version: 2 })
        );

        let mut bad_tag = one(fixtures::rect_content(4.0, 4.0));
        bad_tag[CONTENT] = 9;
        assert_eq!(
            decode(&bad_tag),
            Err(SceneCodecError::UnknownTag {
                field: "scene content",
                tag: 9
            })
        );

        let mut nan = one(fixtures::rect_content(4.0, 4.0));
        nan[CONTENT + 1..CONTENT + 5].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            decode(&nan),
            Err(SceneCodecError::InvalidValue(SceneValueError::NonFinite {
                field: "rect"
            }))
        );

        let mut negative = one(fixtures::rect_content(4.0, 4.0));
        negative[CONTENT + 9..CONTENT + 13].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            decode(&negative),
            Err(SceneCodecError::InvalidValue(SceneValueError::Negative {
                field: "rect"
            }))
        );

        let mut negative_radius = one(fixtures::rect_content(4.0, 4.0));
        negative_radius[CONTENT + 17..CONTENT + 21].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            decode(&negative_radius),
            Err(SceneCodecError::InvalidValue(SceneValueError::Negative {
                field: "corner radius"
            }))
        );

        // content 다음 fields: transform 24 | layout 16 | clip depth 4 | paint 17 | z 8 | hit 2.
        let after = CONTENT + RECT_CONTENT_LEN;
        let mut nan_transform = one(fixtures::rect_content(4.0, 4.0));
        nan_transform[after..after + 4].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(
            decode(&nan_transform),
            Err(SceneCodecError::InvalidValue(SceneValueError::NonFinite {
                field: "world transform"
            }))
        );
        let clip_at = after + 24 + 16;
        let mut bad_clip = one(fixtures::rect_content(4.0, 4.0));
        bad_clip[clip_at..clip_at + 4].copy_from_slice(&1u32.to_le_bytes());
        bad_clip.insert(clip_at + 4, 7);
        assert_eq!(
            decode(&bad_clip),
            Err(SceneCodecError::UnknownTag {
                field: "clip shape",
                tag: 7
            })
        );
        let paint_at = clip_at + 4;
        let mut bad_opacity = one(fixtures::rect_content(4.0, 4.0));
        bad_opacity[paint_at + 8..paint_at + 12].copy_from_slice(&2.0f32.to_le_bytes());
        assert_eq!(
            decode(&bad_opacity),
            Err(SceneCodecError::InvalidValue(SceneValueError::OutOfRange {
                field: "opacity"
            }))
        );
        let mut bad_blend = one(fixtures::rect_content(4.0, 4.0));
        bad_blend[paint_at + 12] = 6;
        assert_eq!(
            decode(&bad_blend),
            Err(SceneCodecError::UnknownTag {
                field: "blend mode",
                tag: 6
            })
        );
        let mut negative_blur = one(fixtures::rect_content(4.0, 4.0));
        negative_blur[paint_at + 13..paint_at + 17].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            decode(&negative_blur),
            Err(SceneCodecError::InvalidValue(SceneValueError::Negative {
                field: "layer blur"
            }))
        );
        let hit_at = paint_at + 17 + 8;
        let mut bad_visibility = one(fixtures::rect_content(4.0, 4.0));
        bad_visibility[hit_at] = 2;
        assert_eq!(
            decode(&bad_visibility),
            Err(SceneCodecError::UnknownTag {
                field: "visibility",
                tag: 2
            })
        );
        let mut bad_pointer = one(fixtures::rect_content(4.0, 4.0));
        bad_pointer[hit_at + 1] = 2;
        assert_eq!(
            decode(&bad_pointer),
            Err(SceneCodecError::UnknownTag {
                field: "pointer events",
                tag: 2
            })
        );

        // stroke·fill layer와 색상 검증.
        let stroked = {
            let mut s = spec(1, fixtures::rect_content(4.0, 4.0), 1);
            s.paint = PaintStackRef::new(PaintStack {
                layers: vec![
                    PaintLayer::Fill(solid(1.0, 0.0, 0.0, 1.0)),
                    PaintLayer::Stroke(
                        StrokeLayer::new(
                            solid(0.0, 0.0, 1.0, 0.5),
                            2.0,
                            StrokeAlign::Inside,
                            StrokeCap::Round,
                        )
                        .unwrap(),
                    ),
                ],
                ..PaintStack::empty()
            });
            encode(
                &SceneSnapshot::new(
                    DocumentRevision(1),
                    SceneRevision(1),
                    vec![Arc::new(SceneNode::leaf(s))],
                )
                .unwrap(),
            )
        };
        assert!(decode(&stroked).is_ok());
        // 첫 layer: tag(1) | paint tag(1) | color(17): color 알파를 2.0으로.
        let color_at = paint_at + 4 + 2;
        let mut bad_alpha = stroked.clone();
        bad_alpha[color_at + 13..color_at + 17].copy_from_slice(&2.0f32.to_le_bytes());
        assert_eq!(
            decode(&bad_alpha),
            Err(SceneCodecError::InvalidColor(ColorError::AlphaOutOfRange))
        );
        let mut bad_layer = stroked.clone();
        bad_layer[paint_at + 4] = 9;
        assert_eq!(
            decode(&bad_layer),
            Err(SceneCodecError::UnknownTag {
                field: "paint layer",
                tag: 9
            })
        );
        // 둘째 layer(stroke): tag | paint 18 | width 4 | align | cap.
        let stroke_at = paint_at + 4 + 1 + 18;
        let mut bad_cap = stroked.clone();
        bad_cap[stroke_at + 1 + 18 + 4 + 1] = 3;
        assert_eq!(
            decode(&bad_cap),
            Err(SceneCodecError::UnknownTag {
                field: "stroke cap",
                tag: 3
            })
        );

        let path = |fill: u8, commands: &[u8]| {
            let mut bytes = one(fixtures::rect_content(1.0, 1.0));
            let tail = bytes.split_off(CONTENT + RECT_CONTENT_LEN);
            bytes.truncate(CONTENT);
            bytes.push(CONTENT_PATH);
            bytes.push(fill);
            bytes.extend_from_slice(commands);
            bytes.extend_from_slice(&tail);
            bytes
        };
        let mut line_first = 1u64.to_le_bytes().to_vec();
        line_first.push(TAG_LINE);
        line_first.extend_from_slice(&1.0f32.to_le_bytes());
        line_first.extend_from_slice(&1.0f32.to_le_bytes());
        assert_eq!(
            decode(&path(0, &line_first)),
            Err(SceneCodecError::InvalidPath(
                PathGeometryError::MissingMoveTo { index: 0 }
            ))
        );
        assert_eq!(
            decode(&path(2, &0u64.to_le_bytes())),
            Err(SceneCodecError::UnknownTag {
                field: "fill rule",
                tag: 2
            })
        );
        let mut bad_command = 1u64.to_le_bytes().to_vec();
        bad_command.push(7);
        assert_eq!(
            decode(&path(0, &bad_command)),
            Err(SceneCodecError::UnknownTag {
                field: "path command",
                tag: 7
            })
        );
        // 거대한 command 개수는 미리 할당하지 않고 오류가 됩니다.
        let huge = path(0, &u64::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());

        let image = || {
            one(SceneContent::Image(ImagePrimitive {
                asset: AssetRef::new(id(1)),
                bounds: rect(0.0, 0.0, 1.0, 1.0),
                fit: ImageFit::Contain,
                crop: ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space: None,
            }))
        };
        // image: tag 1 | asset 16 | bounds 16 | fit 1 | crop 16 | sampling 1 | space 1
        let fit_at = CONTENT + 1 + 16 + 16;
        let mut bad_fit = image();
        bad_fit[fit_at] = 4;
        assert_eq!(
            decode(&bad_fit),
            Err(SceneCodecError::UnknownTag {
                field: "image fit",
                tag: 4
            })
        );
        let mut bad_crop = image();
        bad_crop[fit_at + 1..fit_at + 5].copy_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(
            decode(&bad_crop),
            Err(SceneCodecError::InvalidValue(SceneValueError::OutOfRange {
                field: "image crop"
            }))
        );
        let mut bad_sampling = image();
        bad_sampling[fit_at + 17] = 2;
        assert_eq!(
            decode(&bad_sampling),
            Err(SceneCodecError::UnknownTag {
                field: "image sampling",
                tag: 2
            })
        );
        let mut bad_space = image();
        bad_space[fit_at + 18] = 9;
        assert_eq!(
            decode(&bad_space),
            Err(SceneCodecError::UnknownTag {
                field: "color space",
                tag: 9
            })
        );
    }

    #[test]
    fn z_order_violation_is_rejected_on_decode() {
        // 부모 z 5, 자식 z 3인 bytes를 직접 만들어 decode가 거부하는지 확인합니다.
        let parent = SceneNode::new(
            spec(1, fixtures::rect_content(1.0, 1.0), 5),
            vec![fixtures::leaf(2, 6, 1.0, 1.0)],
        )
        .unwrap();
        let snapshot = SceneSnapshot::new(
            DocumentRevision(1),
            SceneRevision(1),
            vec![Arc::new(parent)],
        )
        .unwrap();
        let mut bytes = encode(&snapshot);
        let node_len = (bytes.len() - 25) / 2;
        let child_z_at = 25 + node_len + node_len - 4 - 2 - 8;
        bytes[child_z_at..child_z_at + 8].copy_from_slice(&3u64.to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(SceneCodecError::InvalidTree(
                SceneTreeError::ZOrderNotIncreasing {
                    previous: ZOrder(5),
                    next: ZOrder(3)
                }
            ))
        );
    }

    #[test]
    fn negative_zero_is_canonical() {
        let make = |x: f32, radius: f32| {
            let content = SceneContent::Rect(RectPrimitive {
                rect: rect(x, 0.0, 1.0, 1.0),
                radii: CornerRadii::new(radius, 0.0, 0.0, 0.0).unwrap(),
            });
            one(content)
        };
        assert_eq!(make(-0.0, -0.0), make(0.0, 0.0));
    }

    #[test]
    fn larger_fixture_round_trips() {
        let snapshot = fixtures::generate(FixtureConfig {
            seed: 77,
            roots: 3,
            children: 4,
            depth: 3,
        });
        let bytes = encode(&snapshot);
        assert_eq!(decode(&bytes).unwrap(), snapshot);
    }
}
