//! 래스터 경계의 offscreen 렌더 요청과 host rasterizer 연결 지점 (XX.4.2).
//!
//! SVG writer는 raster 경계 planner([`crate::svg::raster`], XX.4.1)가 고른 영역마다 [`RasterRequest`]를 만들어 host
//! [`ExportRasterizer`]에게 PNG를 받고, 그 PNG를 영역 pixel 상자 위치에 `<image>`로 넣습니다. export crate는 crate
//! 의존 정책(ADR-0019)상 renderer에 의존하지 않으므로 렌더는 host(앱·CLI가 renderer offscreen target, XX.2.1로
//! 구현)가 맡습니다. host가 rasterizer를 주지 않으면([`NoRasterizer`]) 사전 진단이 래스터 기능을 형식 차단으로
//! 거부하고, writer도 그 기능에서 typed error로 멈춥니다(ADR-0091 결정 2).
//!
//! ## 렌더 계약
//!
//! - 출력: [`RasterRequest::pixels`] 크기(너비 × 높이 pixel)의 PNG 하나([`ImageMedia::Png`], sRGB, 비연관 알파).
//!   pixel `(i, j)`(0부터)는 내보내기 device pixel `(pixels.x0 + i, pixels.y0 + j)`이고, device pixel과 world의 관계는
//!   [`RasterRequest::grid`]([`PixelGrid`])입니다. 바탕은 투명입니다.
//! - 내용: [`RasterRequest::roots`]를 순서대로(paint 순서) 그립니다. [`RasterRoot::content_only`]면 그 node 자기
//!   내용(shadow, image pixel, text glyph, 칠하기 layer)만 그리고 자식과 그 node의 subtree 합성 효과(opacity·blend·
//!   blur)는 그리지 않습니다. 아니면 subtree 전체를 합성 효과와 함께 그립니다. 숨긴 node는 subtree째 그리지 않습니다.
//!   [`RasterRequest::background`]가 있으면 내용보다 먼저 영역 전체에 칠합니다(canvas 영역).
//! - clip: 그리는 node마다 clip stack(바깥 → 안쪽)의 앞 [`RasterRoot::skip_clip_depth`]개는 SVG 그룹이 이미 걸므로
//!   적용하지 않고, 나머지만 적용합니다. 영역 밖 조상의 합성 효과도 SVG 그룹에 남으므로 그리지 않습니다.
//! - resource: capture가 고정한 font·asset revision만 씁니다. 사용자가 확인해 생략한 missing resource
//!   ([`ConfirmedExport::omits_image`] 등)는 vector 경로와 같이 생략합니다.
//! - 결정론: 같은 요청이면 같은 bytes를 돌려줘야 합니다(같은 snapshot SVG byte-identical).

use std::fmt;

use stellar_core_types::color::Color;
use stellar_scene::node::SceneNode;

use crate::capture::{EncodedImage, ExportCapture, ImageMedia};
use crate::preflight::ConfirmedExport;
use crate::svg::raster::{PixelGrid, PixelRect, RasterBoundary};

/// 영역 안 최상위로 그릴 node 하나.
#[derive(Debug, Clone, Copy)]
pub struct RasterRoot<'a> {
    /// node.
    pub node: &'a SceneNode,
    /// node 자기 내용만 그리는지(모듈 문서 "렌더 계약").
    pub content_only: bool,
    /// 그리는 node마다 적용하지 않을 clip stack 앞부분 개수.
    pub skip_clip_depth: usize,
}

/// 래스터 경계 하나의 렌더 요청.
#[derive(Debug, Clone)]
pub struct RasterRequest<'a> {
    export: &'a ConfirmedExport,
    boundary: &'a RasterBoundary,
    grid: PixelGrid,
    pixels: PixelRect,
    roots: Vec<RasterRoot<'a>>,
    background: Option<Color>,
}

impl<'a> RasterRequest<'a> {
    pub(crate) fn new(
        export: &'a ConfirmedExport,
        boundary: &'a RasterBoundary,
        grid: PixelGrid,
        pixels: PixelRect,
        roots: Vec<RasterRoot<'a>>,
        background: Option<Color>,
    ) -> RasterRequest<'a> {
        RasterRequest {
            export,
            boundary,
            grid,
            pixels,
            roots,
            background,
        }
    }

    /// 진행 허가를 받은 내보내기(확인한 생략 resource 조회).
    pub const fn export(&self) -> &'a ConfirmedExport {
        self.export
    }

    /// 고정 입력.
    pub fn capture(&self) -> &'a ExportCapture {
        self.export.capture()
    }

    /// 렌더할 경계(영역과 이유).
    pub const fn boundary(&self) -> &'a RasterBoundary {
        self.boundary
    }

    /// device pixel grid.
    pub const fn grid(&self) -> &PixelGrid {
        &self.grid
    }

    /// 출력 pixel 상자(canvas로 자름, 넓이 양수).
    pub const fn pixels(&self) -> PixelRect {
        self.pixels
    }

    /// 그릴 최상위 node(paint 순서).
    pub fn roots(&self) -> &[RasterRoot<'a>] {
        &self.roots
    }

    /// 영역 전체에 먼저 칠할 배경(canvas 영역의 불투명·반투명 배경).
    pub const fn background(&self) -> Option<Color> {
        self.background
    }
}

/// rasterizer 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterizeError {
    /// host가 offscreen 렌더를 제공하지 않습니다.
    Unavailable,
    /// 렌더·encoding이 실패했습니다.
    Failed {
        /// host가 준 설명.
        message: String,
    },
}

impl fmt::Display for RasterizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RasterizeError::Unavailable => f.write_str("offscreen 래스터 렌더를 쓸 수 없습니다"),
            RasterizeError::Failed { message } => write!(f, "래스터 렌더 실패: {message}"),
        }
    }
}

impl std::error::Error for RasterizeError {}

/// 래스터 경계 offscreen 렌더. host(renderer offscreen target, XX.2.1)가 구현합니다.
pub trait ExportRasterizer {
    /// 렌더를 제공하면 참입니다. 사전 진단은 거짓이면 래스터 기능을 형식 차단으로 알립니다.
    fn is_available(&self) -> bool;

    /// 요청 영역을 PNG로 렌더합니다(모듈 문서 "렌더 계약").
    fn rasterize(&self, request: &RasterRequest<'_>) -> Result<EncodedImage, RasterizeError>;
}

/// 래스터 렌더를 제공하지 않는 host(renderer offscreen target이 연결되기 전 기본값).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoRasterizer;

impl ExportRasterizer for NoRasterizer {
    fn is_available(&self) -> bool {
        false
    }

    fn rasterize(&self, _: &RasterRequest<'_>) -> Result<EncodedImage, RasterizeError> {
        Err(RasterizeError::Unavailable)
    }
}

/// PNG signature(8 byte).
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// PNG bytes의 첫 chunk가 IHDR이면 그 (너비, 높이)를 돌려줍니다. signature나 IHDR가 없으면 `None`입니다.
fn png_header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    // signature 8 + 길이 4 + "IHDR" 4 + 너비 4 + 높이 4.
    let header = bytes.get(..24)?;
    if header[..8] != PNG_SIGNATURE || &header[12..16] != b"IHDR" {
        return None;
    }
    let read = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Some((read(16), read(20)))
}

/// rasterizer 출력이 계약(PNG, 요청 pixel 크기)을 지키는지 검사합니다. 선언한 형식·크기만 믿지 않고 bytes의 PNG
/// signature와 IHDR 크기도 확인합니다.
pub(crate) fn check_output(image: &EncodedImage, pixels: PixelRect) -> Result<(), RasterizeError> {
    let width_ok = i64::from(image.pixel_width()) == pixels.width();
    let height_ok = i64::from(image.pixel_height()) == pixels.height();
    if image.media() != ImageMedia::Png {
        return Err(RasterizeError::Failed {
            message: format!("PNG가 아닌 출력({})", image.media().mime()),
        });
    }
    let Some((width, height)) = png_header_size(image.bytes()) else {
        return Err(RasterizeError::Failed {
            message: "PNG signature·IHDR가 없는 출력".to_string(),
        });
    };
    if (width, height) != (image.pixel_width(), image.pixel_height()) {
        return Err(RasterizeError::Failed {
            message: format!(
                "PNG IHDR 크기 {width}×{height}가 선언한 크기 {}×{}와 다릅니다",
                image.pixel_width(),
                image.pixel_height()
            ),
        });
    }
    if !(width_ok && height_ok) {
        return Err(RasterizeError::Failed {
            message: format!(
                "출력 크기 {}×{}가 요청 {}×{}와 다릅니다",
                image.pixel_width(),
                image.pixel_height(),
                pixels.width(),
                pixels.height()
            ),
        });
    }
    Ok(())
}
