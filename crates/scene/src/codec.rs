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
//! snapshot : magic "STSN" | version u8(=1) | document_revision u64 | scene_revision u64
//!            | root_count u32 | node*            (node는 pre-order)
//! node     : source [u8;16] | content | child_count u32
//! content  : tag u8 + payload
//!   1 rect : rect | radii(tl, tr, br, bl f32)
//!   2 path : fill_rule u8 | geometry(PathGeometry::canonical_bytes)
//!   3 text : layout key [u8;16] | bounds rect
//!   4 image: asset [u8;16] | bounds rect | fit u8 | crop(x, y, w, h f32) | sampling u8
//!            | color_space u8(0 = 없음, 그 외 ColorSpace tag)
//! rect     : x f32 | y f32 | width f32 | height f32
//! ```
//!
//! decode는 신뢰하지 않는 입력을 가정합니다. 잘린 입력·잉여 bytes·미지 tag·비유한 값·
//! 음수 크기·잘못된 crop·subpath 규칙 위반을 구조화 오류로 거부하고 panic하지 않으며,
//! 깊은 tree도 재귀 없이 처리합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::value::{Rect, Size2, SizeError, Vec2};

use crate::geometry::{
    FillRule, PathCommand, PathGeometry, PathGeometryError, TAG_CLOSE, TAG_CUBIC, TAG_LINE,
    TAG_MOVE,
};
use crate::node::{
    CornerRadii, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PathPrimitive, RectPrimitive,
    SceneContent, SceneNode, SceneNodeSpec, SceneValueError, TextLayoutKey, TextPrimitive,
};
use crate::snapshot::SceneSnapshot;

/// canonical bytes 시작 magic.
pub const SCENE_CODEC_MAGIC: [u8; 4] = *b"STSN";

/// canonical 표현 버전.
pub const SCENE_CODEC_VERSION: u8 = 1;

const CONTENT_RECT: u8 = 1;
const CONTENT_PATH: u8 = 2;
const CONTENT_TEXT: u8 = 3;
const CONTENT_IMAGE: u8 = 4;

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
    /// path subpath 규칙 위반.
    InvalidPath(PathGeometryError),
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
            SceneCodecError::InvalidPath(error) => write!(f, "잘못된 path: {error}"),
        }
    }
}

impl std::error::Error for SceneCodecError {}

impl From<SceneValueError> for SceneCodecError {
    fn from(error: SceneValueError) -> SceneCodecError {
        SceneCodecError::InvalidValue(error)
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

fn write_node<S: ByteSink + ?Sized>(node: &SceneNode, out: &mut S) {
    out.put(&node.source().to_bytes());
    write_content(node.content(), out);
    out.put(&(node.child_count() as u32).to_le_bytes());
}

fn write_content<S: ByteSink + ?Sized>(content: &SceneContent, out: &mut S) {
    match content {
        SceneContent::Rect(rect) => {
            out.put(&[CONTENT_RECT]);
            put_rect(out, rect.rect);
            let (tl, tr, br, bl) = rect.radii.components();
            for v in [tl, tr, br, bl] {
                put_f32(out, v);
            }
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

pub(crate) fn put_f32<S: ByteSink + ?Sized>(out: &mut S, value: f32) {
    out.put(&value.to_le_bytes());
}

pub(crate) fn put_rect<S: ByteSink + ?Sized>(out: &mut S, rect: Rect) {
    put_f32(out, rect.origin().x());
    put_f32(out, rect.origin().y());
    put_f32(out, rect.size().width());
    put_f32(out, rect.size().height());
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
    Ok(SceneSnapshot::new(document_revision, scene_revision, roots))
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
        let node = Arc::new(SceneNode::new(done.spec, done.children));
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => return Ok(node),
        }
    }
}

fn read_pending(reader: &mut Reader<'_>) -> Result<Pending, SceneCodecError> {
    let source = StableId::from_bytes(reader.take()?);
    let content = read_content(reader)?;
    let remaining = reader.u32()?;
    Ok(Pending {
        spec: SceneNodeSpec { source, content },
        remaining,
        children: Vec::new(),
    })
}

fn read_content(reader: &mut Reader<'_>) -> Result<SceneContent, SceneCodecError> {
    match reader.u8()? {
        CONTENT_RECT => {
            let rect = reader.rect("rect")?;
            let tl = reader.f32("corner radius")?;
            let tr = reader.f32("corner radius")?;
            let br = reader.f32("corner radius")?;
            let bl = reader.f32("corner radius")?;
            Ok(SceneContent::Rect(RectPrimitive {
                rect,
                radii: CornerRadii::new(tl, tr, br, bl)?,
            }))
        }
        CONTENT_PATH => {
            let tag = reader.u8()?;
            let fill_rule = FillRule::from_tag(tag).ok_or(SceneCodecError::UnknownTag {
                field: "fill rule",
                tag,
            })?;
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

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
    }

    fn p(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    fn id(n: u8) -> StableId {
        StableId::from_bytes([n; 16])
    }

    fn spec(n: u8, content: SceneContent) -> SceneNodeSpec {
        SceneNodeSpec {
            source: id(n),
            content,
        }
    }

    fn rect_content(w: f32) -> SceneContent {
        SceneContent::Rect(RectPrimitive {
            rect: rect(0.0, 0.0, w, w / 2.0),
            radii: CornerRadii::new(1.0, 2.0, 3.0, 4.0).unwrap(),
        })
    }

    /// 모든 content 종류와 중첩을 포함한 snapshot.
    fn sample_snapshot() -> SceneSnapshot {
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
        let image_no_space = SceneContent::Image(ImagePrimitive {
            asset: AssetRef::new(id(0x45)),
            bounds: rect(0.0, 0.0, 8.0, 8.0),
            fit: ImageFit::None,
            crop: ImageCrop::FULL,
            sampling: ImageSampling::Linear,
            color_space: None,
        });
        let inner = SceneNode::new(
            spec(3, rect_content(20.0)),
            vec![
                Arc::new(SceneNode::leaf(spec(4, path))),
                Arc::new(SceneNode::leaf(spec(5, text))),
            ],
        );
        let root = SceneNode::new(
            spec(1, rect_content(100.0)),
            vec![Arc::new(SceneNode::leaf(spec(2, image))), Arc::new(inner)],
        );
        SceneSnapshot::new(
            DocumentRevision(42),
            SceneRevision(7),
            vec![
                Arc::new(root),
                Arc::new(SceneNode::leaf(spec(6, image_no_space))),
            ],
        )
    }

    #[test]
    fn snapshot_round_trips_through_canonical_bytes() {
        // 검증 기준(XI.1.1): core snapshot round-trip serialization.
        let snapshot = sample_snapshot();
        let bytes = encode(&snapshot);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, snapshot);
        // canonical: 다시 인코딩하면 같은 bytes.
        assert_eq!(encode(&decoded), bytes);
        assert_eq!(decoded.node_count(), 6);
        assert_eq!(decoded.resource_refs(), snapshot.resource_refs());
    }

    #[test]
    fn header_bytes_are_hand_checked() {
        let empty = SceneSnapshot::new(DocumentRevision(0x0102), SceneRevision(3), Vec::new());
        let bytes = encode(&empty);
        let mut expected = b"STSN".to_vec();
        expected.push(1);
        expected.extend_from_slice(&[0x02, 0x01, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(bytes, expected);
        assert_eq!(decode(&bytes).unwrap(), empty);

        // leaf rect node 하나: source 16 + tag 1 + rect 16 + radii 16 + child count 4.
        let one = SceneSnapshot::new(
            DocumentRevision(1),
            SceneRevision(1),
            vec![Arc::new(SceneNode::leaf(spec(9, rect_content(8.0))))],
        );
        let bytes = encode(&one);
        let node = &bytes[25..];
        assert_eq!(node.len(), 16 + 1 + 16 + 16 + 4);
        assert_eq!(&node[..16], &[9; 16]);
        assert_eq!(node[16], CONTENT_RECT);
        assert_eq!(&node[17 + 8..17 + 12], &8.0f32.to_le_bytes());
        assert_eq!(&node[node.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn deep_tree_round_trips_without_recursion() {
        let mut node = Arc::new(SceneNode::leaf(spec(0, rect_content(1.0))));
        for _ in 0..1_000 {
            node = Arc::new(SceneNode::new(spec(0, rect_content(1.0)), vec![node]));
        }
        let snapshot = SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), vec![node]);
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
        let one = |content: SceneContent| {
            encode(&SceneSnapshot::new(
                DocumentRevision(1),
                SceneRevision(1),
                vec![Arc::new(SceneNode::leaf(spec(1, content)))],
            ))
        };
        // node content는 header(25) + source(16) 다음 offset 41에서 시작합니다.
        const CONTENT: usize = 25 + 16;

        let mut bad_magic = one(rect_content(4.0));
        bad_magic[0] = b'X';
        assert_eq!(decode(&bad_magic), Err(SceneCodecError::BadMagic));

        let mut bad_version = one(rect_content(4.0));
        bad_version[4] = 2;
        assert_eq!(
            decode(&bad_version),
            Err(SceneCodecError::UnsupportedVersion { version: 2 })
        );

        let mut bad_tag = one(rect_content(4.0));
        bad_tag[CONTENT] = 9;
        assert_eq!(
            decode(&bad_tag),
            Err(SceneCodecError::UnknownTag {
                field: "scene content",
                tag: 9
            })
        );

        let mut nan = one(rect_content(4.0));
        nan[CONTENT + 1..CONTENT + 5].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            decode(&nan),
            Err(SceneCodecError::InvalidValue(SceneValueError::NonFinite {
                field: "rect"
            }))
        );

        let mut negative = one(rect_content(4.0));
        negative[CONTENT + 9..CONTENT + 13].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            decode(&negative),
            Err(SceneCodecError::InvalidValue(SceneValueError::Negative {
                field: "rect"
            }))
        );

        let mut negative_radius = one(rect_content(4.0));
        negative_radius[CONTENT + 17..CONTENT + 21].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            decode(&negative_radius),
            Err(SceneCodecError::InvalidValue(SceneValueError::Negative {
                field: "corner radius"
            }))
        );

        let path = |fill: u8, commands: &[u8]| {
            let mut bytes = one(rect_content(1.0));
            bytes.truncate(CONTENT);
            bytes.push(CONTENT_PATH);
            bytes.push(fill);
            bytes.extend_from_slice(commands);
            bytes.extend_from_slice(&0u32.to_le_bytes());
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
        // 거대한 command 개수는 미리 할당하지 않고 잘림 오류가 됩니다.
        let mut huge = path(0, &u64::MAX.to_le_bytes());
        huge.truncate(huge.len() - 4);
        assert_eq!(decode(&huge), Err(SceneCodecError::Truncated));

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
    fn negative_zero_is_canonical() {
        let a = SceneSnapshot::new(
            DocumentRevision(1),
            SceneRevision(1),
            vec![Arc::new(SceneNode::leaf(spec(
                1,
                SceneContent::Rect(RectPrimitive {
                    rect: rect(-0.0, 0.0, 1.0, 1.0),
                    radii: CornerRadii::new(-0.0, 0.0, 0.0, 0.0).unwrap(),
                }),
            )))],
        );
        let b = SceneSnapshot::new(
            DocumentRevision(1),
            SceneRevision(1),
            vec![Arc::new(SceneNode::leaf(spec(
                1,
                SceneContent::Rect(RectPrimitive {
                    rect: rect(0.0, 0.0, 1.0, 1.0),
                    radii: CornerRadii::ZERO,
                }),
            )))],
        );
        assert_eq!(encode(&a), encode(&b));
    }
}
