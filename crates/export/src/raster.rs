//! 내보내기 offscreen 렌더 target (XX.2.1, ADR-0103).
//!
//! PNG 내보내기는 고정 입력([`ConfirmedExport`])을 [`OffscreenTarget`]에 넘겨 내보내기 영역 전체의 RGBA image
//! ([`RgbaImage`])를 받고, 그 image를 PNG로 encode합니다([`crate::png`]).
//!
//! ## 렌더 계약
//!
//! - **크기**: [`OffscreenRequest::pixels`]는 내보내기 device pixel grid([`PixelGrid`], SVG 래스터 경계와 같은
//!   grid)의 canvas 전체입니다. 크기는 `⌈영역 너비 × scale⌉ × ⌈영역 높이 × scale⌉`(각 변 최소 1)이고, pixel `(i, j)`는
//!   device pixel `(i, j)`, world 점 `p`는 device `(p − 영역 원점) × scale`입니다. 한 변이
//!   [`MAX_DIMENSION`]을 넘거나 pixel 수가 [`MAX_PIXELS`]를 넘으면 [`OffscreenError::TooLarge`]입니다.
//! - **pixel**: 행 우선, pixel마다 R·G·B·A 8-bit. RGB는 [`OutputProfile`] 색공간의 transfer 인코딩 값이고 알파는
//!   비연관(straight)입니다. 투명 pixel(알파 0)의 RGB는 0입니다.
//! - **색**: 합성은 출력 색공간의 선형광, 연관(premultiplied) 알파로 합니다(아키텍처 §10.3). scene 색과 이미지
//!   pixel은 renderer 출력 색 변환([`OutputColorPlan`], XI.10.5)으로 출력 색공간 선형광에 옮기며, 범위 밖은 그
//!   변환 규칙대로 자릅니다(gamut 경고는 PNG 결과 report에 남김).
//! - **내용**: capture root(paint 순서)의 보이는 node를 painter 순서로 그립니다. 배경
//!   ([`OffscreenRequest::background`])이 있으면 먼저 canvas 전체에 칠합니다. 합성 불투명도가 1보다 작은 node는
//!   subtree를 따로 그린 layer를 그 불투명도로 합성합니다. layer 안 draw는 경계 node 안쪽 clip만, layer 합성은
//!   경계 node 바깥 clip을 적용합니다(renderer pass compiler XI.4.2~XI.4.4와 같은 규칙). selection root는
//!   [`ExportRoot::base_clip_depth`](crate::capture::ExportRoot) 앞의 조상 clip과 조상 합성 효과를 그리지 않습니다.
//! - **missing resource**: 사용자가 확인한 missing image·text layout·font만 생략합니다([`ConfirmedExport`]).
//! - **결정론**: 같은 요청이면 같은 pixel입니다.
//!
//! ## target
//!
//! - [`software::SoftwareTarget`]: CPU 구현입니다. headless 도구(CLI·MCP)는 crate 의존 정책상 GPU backend에 닿지
//!   않으므로(ADR-0005 §8) 이 target을 씁니다. 렌더러가 아직 공식을 고정하지 않은 효과(shadow, layer blur,
//!   Normal이 아닌 blend)는 근사하지 않고 [`OffscreenError::Unsupported`]입니다(ADR-0091 결정 2와 같은 원칙).
//! - GPU(RenderGraph offscreen) target은 같은 trait을 구현해 붙입니다. 출력은 [`check_image`]로 계약을 검사합니다.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_renderer_core::backend::{BackendKind, DeviceCapabilities};
use stellar_renderer_core::output_color::OutputColorPlan;
use stellar_renderer_core::tessellation::TessellationError;
use stellar_scene::node::TextLayoutKey;

use crate::preflight::ConfirmedExport;
use crate::request::ExportBackground;
use crate::svg::raster::{PixelGrid, PixelRect};
use crate::svg::text::TextOutlineError;

/// CPU offscreen target.
pub mod software;

/// coverage 래스터화(polygon → 부분 coverage mask).
mod coverage;

/// path 평탄화·device 변환.
mod path;

/// 이미지 pixel 해석과 sampling.
mod image;

/// 출력 한 변 최대 pixel.
pub const MAX_DIMENSION: u32 = 16_384;

/// 출력 최대 pixel 수(4096²). 합성 buffer는 pixel마다 16 byte입니다.
pub const MAX_PIXELS: u64 = 16_777_216;

/// 출력 색 profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputProfile {
    /// sRGB(기본).
    #[default]
    Srgb,
    /// Display P3(D65, sRGB transfer 곡선).
    DisplayP3,
}

/// profile 이름 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProfile {
    /// 입력 이름(최대 64 byte로 자름).
    pub name: String,
}

impl fmt::Display for UnknownProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "출력 색 profile '{}'을 지원하지 않습니다(srgb, display-p3)",
            self.name
        )
    }
}

impl std::error::Error for UnknownProfile {}

impl OutputProfile {
    /// 모든 profile.
    pub const ALL: [OutputProfile; 2] = [OutputProfile::Srgb, OutputProfile::DisplayP3];

    /// 안정 이름(`srgb`, `display-p3`).
    pub const fn name(self) -> &'static str {
        match self {
            OutputProfile::Srgb => "srgb",
            OutputProfile::DisplayP3 => "display-p3",
        }
    }

    /// 이름에서 profile을 찾습니다(대소문자 구분).
    pub fn from_name(name: &str) -> Result<OutputProfile, UnknownProfile> {
        OutputProfile::ALL
            .into_iter()
            .find(|profile| profile.name() == name)
            .ok_or_else(|| UnknownProfile {
                name: truncate(name, 64),
            })
    }

    /// 출력 색공간.
    pub const fn color_space(self) -> ColorSpace {
        match self {
            OutputProfile::Srgb => ColorSpace::Srgb,
            OutputProfile::DisplayP3 => ColorSpace::DisplayP3,
        }
    }

    /// renderer 출력 색 변환(XI.10.5). 파일 출력은 두 색공간을 모두 표현하므로 요청 색공간 그대로입니다.
    pub fn color_plan(self) -> OutputColorPlan {
        let caps = DeviceCapabilities {
            backend: BackendKind::Mock,
            max_texture_dimension: MAX_DIMENSION,
            display_p3_surface: true,
            timestamp_query: false,
            msaa_x4: false,
            stencil_bits: 0,
        };
        OutputColorPlan::negotiate(self.color_space(), &caps)
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// 취소 신호와 진행 정도. 복제하면 같은 신호를 공유합니다.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    inner: Arc<TokenState>,
}

#[derive(Debug, Default)]
struct TokenState {
    cancelled: AtomicBool,
    progress: AtomicU64,
}

impl CancelToken {
    /// 취소되지 않은 새 신호.
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// 취소를 요청합니다.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
    }

    /// 취소를 요청받았으면 참입니다.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// 진행 단위(렌더한 draw·encode한 행)를 더합니다.
    pub fn advance(&self, amount: u64) {
        self.inner.progress.fetch_add(amount, Ordering::Relaxed);
    }

    /// 지금까지의 진행 단위.
    pub fn progress(&self) -> u64 {
        self.inner.progress.load(Ordering::Relaxed)
    }
}

/// 내보내기 영역 전체 렌더 요청.
#[derive(Debug, Clone)]
pub struct OffscreenRequest<'a> {
    export: &'a ConfirmedExport,
    grid: PixelGrid,
    profile: OutputProfile,
}

impl<'a> OffscreenRequest<'a> {
    /// 진행 허가를 받은 내보내기 전체를 `profile`로 렌더하는 요청입니다. grid는 capture 영역과 요청 배율입니다.
    pub fn new(export: &'a ConfirmedExport, profile: OutputProfile) -> OffscreenRequest<'a> {
        OffscreenRequest {
            export,
            grid: PixelGrid::of_capture(export.capture()),
            profile,
        }
    }

    /// 진행 허가를 받은 내보내기.
    pub const fn export(&self) -> &'a ConfirmedExport {
        self.export
    }

    /// device pixel grid.
    pub const fn grid(&self) -> &PixelGrid {
        &self.grid
    }

    /// 출력 pixel 상자(canvas 전체, 원점 0).
    pub const fn pixels(&self) -> PixelRect {
        self.grid.canvas()
    }

    /// 출력 색 profile.
    pub const fn profile(&self) -> OutputProfile {
        self.profile
    }

    /// 먼저 칠할 배경(요청 배경이 단색일 때).
    pub fn background(&self) -> Option<Color> {
        match self.export.capture().request().background() {
            ExportBackground::Transparent => None,
            ExportBackground::Solid(color) => Some(color),
        }
    }

    /// 출력 크기(너비, 높이). 상한을 넘으면 [`OffscreenError::TooLarge`]입니다.
    pub fn size(&self) -> Result<(u32, u32), OffscreenError> {
        let pixels = self.pixels();
        let (width, height) = (pixels.width(), pixels.height());
        let fits = |v: i64| u32::try_from(v).ok().filter(|v| *v <= MAX_DIMENSION);
        match (fits(width), fits(height)) {
            (Some(w), Some(h)) if u64::from(w) * u64::from(h) <= MAX_PIXELS => Ok((w, h)),
            _ => Err(OffscreenError::TooLarge {
                width: width.max(0) as u64,
                height: height.max(0) as u64,
            }),
        }
    }
}

/// 렌더 결과 image(모듈 문서 "pixel").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    profile: OutputProfile,
    pixels: Vec<u8>,
}

impl RgbaImage {
    /// 크기와 pixel로 만듭니다. pixel 길이가 `너비 × 높이 × 4`가 아니거나 크기가 0이면 `None`입니다.
    pub fn new(
        width: u32,
        height: u32,
        profile: OutputProfile,
        pixels: Vec<u8>,
    ) -> Option<RgbaImage> {
        let expected = u64::from(width) * u64::from(height) * 4;
        (width > 0 && height > 0 && pixels.len() as u64 == expected).then_some(RgbaImage {
            width,
            height,
            profile,
            pixels,
        })
    }

    /// 너비.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 색 profile.
    pub const fn profile(&self) -> OutputProfile {
        self.profile
    }

    /// RGBA8 pixel(행 우선).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// pixel `(x, y)`의 RGBA.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = ((y as usize) * (self.width as usize) + x as usize) * 4;
        self.pixels.get(at..at + 4)?.try_into().ok()
    }
}

/// 그릴 수 없는 기능.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsupportedFeature {
    /// drop shadow(렌더러 blur 반경 수식 미고정, XI.10.4).
    Shadow,
    /// layer blur(같은 이유).
    LayerBlur,
    /// Normal이 아닌 blend(렌더러 blend 수식 미고정, XI.10.2).
    BlendMode,
    /// 해석할 수 없는 이미지 형식(PNG 외, 또는 지원하지 않는 PNG 변형).
    ImageFormat,
    /// 이미지 내장 색 profile이 sRGB·Display P3가 아닙니다.
    ImageColorProfile,
    /// outline이 없는 glyph(bitmap emoji 등).
    GlyphWithoutOutline,
}

impl UnsupportedFeature {
    /// 안정 code.
    pub const fn code(self) -> &'static str {
        match self {
            UnsupportedFeature::Shadow => "shadow",
            UnsupportedFeature::LayerBlur => "layer-blur",
            UnsupportedFeature::BlendMode => "blend-mode",
            UnsupportedFeature::ImageFormat => "image-format",
            UnsupportedFeature::ImageColorProfile => "image-color-profile",
            UnsupportedFeature::GlyphWithoutOutline => "glyph-without-outline",
        }
    }
}

/// offscreen 렌더 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum OffscreenError {
    /// 취소되었습니다.
    Cancelled,
    /// 출력 크기가 상한을 넘습니다.
    TooLarge {
        /// 요청 너비.
        width: u64,
        /// 요청 높이.
        height: u64,
    },
    /// target이 그릴 수 없는 기능입니다.
    Unsupported {
        /// node.
        node: StableId,
        /// 기능.
        feature: UnsupportedFeature,
    },
    /// 확인하지 않은 missing image.
    MissingImage {
        /// node.
        node: StableId,
        /// asset.
        asset: AssetRef,
    },
    /// 이미지 bytes를 해석하지 못했습니다.
    BadImage {
        /// node.
        node: StableId,
        /// asset.
        asset: AssetRef,
        /// 원인.
        reason: String,
    },
    /// 확인하지 않은 missing text layout.
    MissingTextLayout {
        /// node.
        node: StableId,
        /// layout.
        layout: TextLayoutKey,
    },
    /// glyph outline 변환 실패.
    Text {
        /// node.
        node: StableId,
        /// 원인.
        error: TextOutlineError,
    },
    /// stroke tessellation 실패.
    Stroke {
        /// node.
        node: StableId,
        /// 원인.
        error: TessellationError,
    },
    /// target 출력이 계약(크기·profile·길이)을 어겼습니다.
    Contract {
        /// 설명.
        message: String,
    },
    /// host target 실패.
    Failed {
        /// host가 준 설명.
        message: String,
    },
}

impl fmt::Display for OffscreenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OffscreenError::Cancelled => f.write_str("렌더가 취소되었습니다"),
            OffscreenError::TooLarge { width, height } => write!(
                f,
                "출력 {width}×{height} pixel이 상한(한 변 {MAX_DIMENSION}, 전체 {MAX_PIXELS})을 넘습니다"
            ),
            OffscreenError::Unsupported { node, feature } => {
                write!(f, "node {node}: 그릴 수 없는 기능({})", feature.code())
            }
            OffscreenError::MissingImage { node, asset } => {
                write!(f, "node {node}: 이미지 {}가 없습니다", asset.id())
            }
            OffscreenError::BadImage {
                node,
                asset,
                reason,
            } => write!(f, "node {node}: 이미지 {}를 읽지 못했습니다({reason})", asset.id()),
            OffscreenError::MissingTextLayout { node, .. } => {
                write!(f, "node {node}: text layout이 없습니다")
            }
            OffscreenError::Text { node, error } => write!(f, "node {node}: {error}"),
            OffscreenError::Stroke { node, error } => write!(f, "node {node}: stroke 실패({error})"),
            OffscreenError::Contract { message } => write!(f, "렌더 출력 계약 위반: {message}"),
            OffscreenError::Failed { message } => write!(f, "렌더 실패: {message}"),
        }
    }
}

impl std::error::Error for OffscreenError {}

/// 내보내기 영역 전체 offscreen 렌더(모듈 문서 "렌더 계약").
pub trait OffscreenTarget {
    /// 요청을 렌더합니다. `cancel`이 취소되면 가능한 빨리 [`OffscreenError::Cancelled`]로 멈춥니다.
    fn render(
        &self,
        request: &OffscreenRequest<'_>,
        cancel: &CancelToken,
    ) -> Result<RgbaImage, OffscreenError>;
}

/// target 출력이 요청 크기·profile인지 검사합니다.
pub fn check_image(
    image: &RgbaImage,
    request: &OffscreenRequest<'_>,
) -> Result<(), OffscreenError> {
    let (width, height) = request.size()?;
    if (image.width(), image.height()) != (width, height) {
        return Err(OffscreenError::Contract {
            message: format!(
                "출력 {}×{}가 요청 {width}×{height}와 다릅니다",
                image.width(),
                image.height()
            ),
        });
    }
    if image.profile() != request.profile() {
        return Err(OffscreenError::Contract {
            message: format!(
                "출력 profile {}가 요청 {}와 다릅니다",
                image.profile().name(),
                request.profile().name()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
