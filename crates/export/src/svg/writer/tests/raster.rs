//! 부분 래스터화 writer 테스트 (XX.4.2).
//!
//! 테스트 rasterizer는 요청을 기록하고, 요청 pixel 크기의 단색 PNG(영역 첫 root 첫 칠하기 색과 알파)를 돌려줍니다.
//!
//! - 위치·scale: `<image>` 상자가 경계 pixel 상자의 world 상자이고, PNG pixel 크기가 world 크기 × 배율입니다.
//! - alpha: PNG는 그대로(투명 바탕, 비연관 알파) 들어가고, 영역 밖 조상 group opacity는 vector 그룹에 남습니다.
//!   canvas 영역만 배경을 rasterizer가 칠하고 vector 배경 사각형을 쓰지 않습니다.

use std::cell::RefCell;

use super::*;
use crate::svg::raster::{plan_raster, PixelRect, RasterRegion};
use crate::svg::rasterize::{ExportRasterizer, RasterRequest, RasterizeError};

/// 기록한 요청 하나: pixel 상자, (root, content_only, skip_clip_depth), 배경.
type Call = (PixelRect, Vec<(StableId, bool, usize)>, Option<Color>);

/// 테스트 rasterizer.
#[derive(Default)]
pub(super) struct FakeRaster {
    pub(super) calls: RefCell<Vec<Call>>,
    /// 출력 조작(계약 위반 검사).
    tamper: Option<Tamper>,
}

#[derive(Clone, Copy)]
enum Tamper {
    WrongSize,
    Jpeg,
    Fail,
    /// 선언 크기는 맞지만 bytes IHDR 크기가 다릅니다.
    LyingHeader,
    /// PNG라고 선언했지만 PNG bytes가 아닙니다.
    NotPngBytes,
}

/// 영역 첫 root의 첫 칠하기 색(없으면 투명)을 RGBA8로.
fn first_fill(request: &RasterRequest<'_>) -> [u8; 4] {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    request
        .roots()
        .first()
        .and_then(|root| {
            root.node
                .paint()
                .get()
                .layers
                .iter()
                .find_map(|layer| match layer {
                    PaintLayer::Fill(Paint::Solid(color)) => Some(*color),
                    PaintLayer::Stroke(_) => None,
                })
        })
        .map_or([0, 0, 0, 0], |c| {
            [to8(c.r()), to8(c.g()), to8(c.b()), to8(c.alpha())]
        })
}

impl ExportRasterizer for FakeRaster {
    fn is_available(&self) -> bool {
        true
    }

    fn rasterize(&self, request: &RasterRequest<'_>) -> Result<EncodedImage, RasterizeError> {
        let pixels = request.pixels();
        self.calls.borrow_mut().push((
            pixels,
            request
                .roots()
                .iter()
                .map(|root| (root.node.source(), root.content_only, root.skip_clip_depth))
                .collect(),
            request.background(),
        ));
        let (mut width, height) = (pixels.width() as u32, pixels.height() as u32);
        let mut media = ImageMedia::Png;
        match self.tamper {
            None => {}
            Some(Tamper::WrongSize) => width += 1,
            Some(Tamper::Jpeg) => media = ImageMedia::Jpeg,
            Some(Tamper::Fail) => {
                return Err(RasterizeError::Failed {
                    message: "장치 없음".to_string(),
                })
            }
            Some(Tamper::LyingHeader) => {
                let png = png::solid(width + 1, height, first_fill(request));
                return Ok(EncodedImage::new(media, Arc::from(png), width, height).unwrap());
            }
            Some(Tamper::NotPngBytes) => {
                let bytes = b"\xFF\xD8\xFF not a png".to_vec();
                return Ok(EncodedImage::new(media, Arc::from(bytes), width, height).unwrap());
            }
        }
        let png = png::solid(width, height, first_fill(request));
        Ok(EncodedImage::new(media, Arc::from(png), width, height).unwrap())
    }
}

/// 테스트 PNG encoder(무압축 deflate)와 decoder 일부.
pub(super) mod png {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(body);
        out.extend_from_slice(&crc_input);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    /// 단색 RGBA8 PNG.
    pub(in super::super) fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut row = vec![0u8];
        for _ in 0..width {
            row.extend_from_slice(&rgba);
        }
        let raw = row.repeat(height as usize);
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (index, block) in blocks.iter().enumerate() {
            zlib.push(u8::from(index + 1 == blocks.len()));
            let len = block.len() as u16;
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// PNG IHDR 크기.
    pub(in super::super) fn size(png: &[u8]) -> (u32, u32) {
        let read = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
        (read(16), read(20))
    }
}

/// 표준 base64 decode(테스트 검증용).
fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("base64 문자 아님: {c}"),
        }
    };
    let mut out = Vec::new();
    for group in text.as_bytes().chunks(4) {
        let pad = group.iter().filter(|&&c| c == b'=').count();
        let mut n = 0u32;
        for &c in group {
            n = (n << 6) | if c == b'=' { 0 } else { value(c) };
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    out
}

/// SVG의 래스터 `<image>` 하나: (x, y, width, height, PNG bytes).
pub(super) fn raster_images(svg: &str) -> Vec<([f32; 4], Vec<u8>)> {
    svg.split("<image x=\"")
        .skip(1)
        .map(|rest| {
            let tag = &rest[..rest.find("/>").unwrap()];
            let number = |name: &str| -> f32 {
                let start = if name == "x" {
                    0
                } else {
                    tag.find(&format!(" {name}=\"")).unwrap() + name.len() + 3
                };
                let end = start + tag[start..].find('"').unwrap();
                tag[start..end].parse().unwrap()
            };
            let data = &tag[tag.find("base64,").unwrap() + 7..tag.rfind('"').unwrap()];
            (
                [number("x"), number("y"), number("width"), number("height")],
                base64_decode(data),
            )
        })
        .collect()
}

fn shadow() -> Shadow {
    Shadow::new(
        fx::p(1.0, 1.0),
        BlurRadius::new(2.0).unwrap(),
        fx::srgb(0.0, 0.0, 0.0, 0.5),
    )
}

/// `rect_spec`에 paint 수정을 더합니다.
pub(super) fn painted(n: u64, z: u64, x: f32, edit: impl FnOnce(&mut PaintStack)) -> SceneNodeSpec {
    let mut spec = rect_spec(n, z, x);
    let mut paint = spec.paint.get().clone();
    edit(&mut paint);
    spec.paint = PaintStackRef::new(paint);
    spec
}

fn request_with(scale: f32, background: ExportBackground) -> ExportRequest {
    ExportRequest::new(
        ExportSource::view(fx::view_id()).unwrap(),
        fx::profile(),
        ExportScale::new(scale).unwrap(),
        background,
        OutputPath::new("/tmp/stellar-export/out.svg").unwrap(),
        ExportFormat::Svg,
    )
    .unwrap()
}

pub(super) fn write_raster(
    capture: &ExportCapture,
    raster: &FakeRaster,
) -> Result<SvgDocument, SvgExportError> {
    write_svg_with(
        &confirmed(capture),
        &assets(),
        &NoText,
        raster,
        &SvgWriteOptions::default(),
    )
}

/// 세 사각형(x 0·20·40) 중 가운데(2)에 drop shadow.
fn shadow_row() -> SceneSnapshot {
    snapshot_of(vec![
        SceneNode::leaf(rect_spec(1, 1, 0.0)),
        SceneNode::leaf(painted(2, 2, 20.0, |p| p.shadows.push(shadow()))),
        SceneNode::leaf(rect_spec(3, 3, 40.0)),
    ])
}

#[test]
fn content_regions_replace_only_the_node_content() {
    for scale in [1.0_f32, 2.0] {
        let capture = capture(
            shadow_row(),
            request_with(scale, ExportBackground::Transparent),
        );
        let raster = FakeRaster::default();
        let document = write_raster(&capture, &raster).unwrap();
        validate(document.as_str());
        let plan = plan_raster(&capture);
        let boundary = &plan.boundaries()[0];
        assert_eq!(plan.boundaries().len(), 1);
        let pixels = boundary.pixels().unwrap();
        // 요청: node 2 자기 내용만, clip 없음.
        let calls = raster.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (pixels, vec![(fx::node_id(2), true, 0)], None));
        // 위치: image 상자 = 경계 pixel 상자의 world 상자. scale: PNG pixel = world 크기 × 배율.
        let images = raster_images(document.as_str());
        assert_eq!(images.len(), 1);
        let world = plan.grid().world_rect(&pixels).map(|v| v as f32);
        assert_eq!(images[0].0, world);
        // 사각형(20..30)과 shadow(오프셋 1, blur 2: 19..33)를 덮는 상자: world (19, -1)부터 14×14.
        assert_eq!(world, [19.0, -1.0, 14.0, 14.0]);
        let (pw, ph) = png::size(&images[0].1);
        assert_eq!((pw as f32, ph as f32), (world[2] * scale, world[3] * scale));
        // alpha: PNG bytes가 그대로 들어갑니다.
        assert_eq!(
            images[0].1,
            png::solid(pw, ph, [128, 128, 128, 255]),
            "rasterizer 출력 그대로"
        );
        // node 2 그룹은 남고 그 안에 vector 사각형 대신 image가 있습니다. 1·3은 vector입니다.
        let svg = document.as_str();
        let group = format!("<g id=\"n-{}\">\n<image ", fx::node_id(2));
        assert!(svg.contains(&group), "{svg}");
        assert_eq!(document.groups(), 3);
        assert_eq!(document.raster_images(), 1);
        assert_eq!(svg.matches("<rect ").count(), 2);
    }
}

#[test]
fn subtree_regions_keep_ancestor_effects_vector() {
    // group(10, opacity 0.5, frame clip) 아래 blur 자식(11)과 vector 자식(12).
    let frame_clip = ClipStackRef::none().push(ClipShape::Rect {
        rect: fx::rect(0.0, 0.0, 100.0, 100.0),
        radii: CornerRadii::ZERO,
        transform: Transform2D::identity(),
    });
    let parent = painted(10, 1, 0.0, |p| p.opacity = Opacity::new(0.5).unwrap());
    let mut blurred = painted(11, 2, 30.0, |p| {
        p.layer_blur = BlurRadius::new(3.0).unwrap();
        p.layers = vec![PaintLayer::Fill(Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 0.5)))];
    });
    blurred.clip = frame_clip.clone();
    let mut grandchild = rect_spec(13, 3, 32.0);
    grandchild.clip = frame_clip.clone();
    let mut vector = rect_spec(12, 4, 60.0);
    vector.clip = frame_clip;
    let blurred = SceneNode::new(blurred, vec![Arc::new(SceneNode::leaf(grandchild))]).unwrap();
    let tree = SceneNode::new(
        parent,
        vec![Arc::new(blurred), Arc::new(SceneNode::leaf(vector))],
    )
    .unwrap();
    let capture = capture(snapshot_of(vec![tree]), svg_request());
    let raster = FakeRaster::default();
    let document = write_raster(&capture, &raster).unwrap();
    validate(document.as_str());
    let svg = document.as_str();
    // blur subtree(11, 13)만 image 하나로, 부모 opacity 그룹과 형제 12는 vector입니다.
    let calls = raster.calls.borrow();
    assert_eq!(calls.len(), 1);
    // 영역 root 11은 부모가 건 clip 뒤 자기 frame clip까지 SVG 그룹이 걸어 rasterizer는 clip을 건너뜁니다.
    assert_eq!(calls[0].1, vec![(fx::node_id(11), false, 1)]);
    assert!(svg.contains(&format!("<g id=\"n-{}\" opacity=\"0.5\">", fx::node_id(10))));
    let region_group = format!("<g id=\"n-{}\" clip-path=\"url(#", fx::node_id(11));
    assert!(svg.contains(&region_group), "{svg}");
    assert!(!svg.contains(&format!("n-{}", fx::node_id(13))));
    assert!(svg.contains(&format!("<g id=\"n-{}\" clip-path=", fx::node_id(12))));
    assert_eq!(document.raster_images(), 1);
    // 그룹: 10, 영역 11, 12.
    assert_eq!(document.groups(), 3);
    // alpha: 반투명 빨강 PNG가 그대로 들어가고 위 opacity 0.5 그룹이 합성합니다.
    let images = raster_images(svg);
    let (pw, ph) = png::size(&images[0].1);
    assert_eq!(images[0].1, png::solid(pw, ph, [255, 0, 0, 128]));
    let opacity_group = svg
        .find(&format!("n-{}\" opacity", fx::node_id(10)))
        .unwrap();
    assert!(svg.find("<image ").unwrap() > opacity_group);
}

#[test]
fn blend_backdrops_and_canvas_backgrounds_are_rasterized_together() {
    // 앞 사각형(1, x 0)과 겹치는 multiply(2, x 5), 떨어진 사각형(3, x 40).
    let scene = || {
        snapshot_of(vec![
            SceneNode::leaf(rect_spec(1, 1, 0.0)),
            SceneNode::leaf(painted(2, 2, 5.0, |p| p.blend = SceneBlendMode::Multiply)),
            SceneNode::leaf(rect_spec(3, 3, 40.0)),
        ])
    };
    let capture_t = capture(scene(), svg_request());
    let raster = FakeRaster::default();
    let document = write_raster(&capture_t, &raster).unwrap();
    validate(document.as_str());
    // 투명 배경: 1·2 연속 범위 하나, 3은 vector.
    assert_eq!(
        raster.calls.borrow()[0].1,
        vec![(fx::node_id(1), false, 0), (fx::node_id(2), false, 0)]
    );
    assert_eq!(raster.calls.borrow()[0].2, None);
    assert_eq!(document.groups(), 2);
    assert!(document
        .as_str()
        .contains(&format!("<g id=\"n-{}\">\n<image ", fx::node_id(1))));
    assert!(document
        .as_str()
        .contains(&format!("<g id=\"n-{}\">\n<rect ", fx::node_id(3))));
    // 불투명 배경: canvas 전체가 image 하나이고 배경은 rasterizer가 칠합니다.
    let white = fx::srgb(1.0, 1.0, 1.0, 1.0);
    let capture_b = capture(scene(), request_with(1.0, ExportBackground::Solid(white)));
    let raster = FakeRaster::default();
    let document = write_raster(&capture_b, &raster).unwrap();
    validate(document.as_str());
    let plan = plan_raster(&capture_b);
    assert_eq!(*plan.boundaries()[0].region(), RasterRegion::Canvas);
    let calls = raster.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, plan.grid().canvas());
    assert_eq!(calls[0].2, Some(white));
    assert_eq!(calls[0].1.len(), 3);
    assert_eq!(document.raster_images(), 1);
    assert_eq!(document.groups(), 1);
    // vector 배경 사각형이 없습니다.
    assert!(!document.as_str().contains("<rect "));
    let images = raster_images(document.as_str());
    let bounds = capture_b.bounds();
    assert_eq!(
        images[0].0,
        [
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            bounds.height()
        ]
    );
}

#[test]
fn raster_contract_violations_are_errors() {
    let capture = capture(shadow_row(), svg_request());
    for (tamper, expected) in [
        (Tamper::WrongSize, "크기"),
        (Tamper::Jpeg, "PNG"),
        (Tamper::Fail, "장치 없음"),
        (Tamper::LyingHeader, "IHDR 크기"),
        (Tamper::NotPngBytes, "signature"),
    ] {
        let raster = FakeRaster {
            tamper: Some(tamper),
            ..FakeRaster::default()
        };
        let error = write_raster(&capture, &raster).unwrap_err();
        match &error {
            SvgExportError::Raster { node, .. } => assert_eq!(*node, fx::node_id(2)),
            other => panic!("{other:?}"),
        }
        assert!(error.to_string().contains(expected), "{error}");
    }
    // rasterizer가 없으면 래스터 기능에서 typed error입니다.
    assert_eq!(
        write(&capture).unwrap_err(),
        SvgExportError::Unsupported {
            node: fx::node_id(2),
            feature: SvgFeature::DropShadow,
            support: SvgFeature::DropShadow.support(),
        }
    );
    assert!(!RasterizeError::Unavailable.to_string().is_empty());
}

#[test]
fn raster_output_is_byte_identical() {
    let capture = capture(shadow_row(), svg_request());
    let first = write_raster(&capture, &FakeRaster::default()).unwrap();
    let second = write_raster(&capture, &FakeRaster::default()).unwrap();
    assert_eq!(first.as_bytes(), second.as_bytes());
}

pub(super) fn capture_of_snapshot(snapshot: SceneSnapshot) -> ExportCapture {
    capture(snapshot, svg_request())
}
