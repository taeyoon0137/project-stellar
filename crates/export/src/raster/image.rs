//! 이미지 asset pixel 해석과 sampling.
//!
//! - 형식: PNG만 해석합니다([`crate::png::decode_png`]). JPEG·GIF·WebP와 interlace PNG는
//!   [`UnsupportedFeature::ImageFormat`]입니다(근사·생략하지 않음).
//! - 색: image primitive의 색공간 override가 있으면 그것, 없으면 PNG 색 정보([`PngColorInfo::color_space`])를 pixel
//!   색공간으로 보고, renderer 출력 색 변환([`OutputColorPlan::convert`])으로 출력 색공간 선형광 연관 알파 texel로 옮깁니다.
//! - sampling: texel 중심은 `(i + 0.5, j + 0.5)`입니다. Linear는 crop 영역 안에서 이웃 texel 넷을 쌍선형 보간하고(가장자리
//!   texel 반복), Nearest는 점이 속한 texel입니다. 보간은 연관 알파 선형광 값으로 합니다.

use stellar_core_types::color::ColorSpace;
use stellar_renderer_core::output_color::{OutputColor, OutputColorPlan};
use stellar_scene::node::ImageSampling;

use super::UnsupportedFeature;
use crate::capture::{EncodedImage, ImageMedia};
use crate::png::{decode_png, DecodeLimits, PngDecodeError};

/// 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageIssue {
    /// 그릴 수 없는 형식·색 profile.
    Unsupported(UnsupportedFeature),
    /// bytes가 잘못되었습니다.
    Bad(String),
}

/// 출력 색공간 선형광 연관 알파 texel.
#[derive(Debug, Clone)]
pub(crate) struct Texture {
    width: u32,
    height: u32,
    texels: Vec<[f32; 4]>,
}

impl Texture {
    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    pub(crate) const fn height(&self) -> u32 {
        self.height
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        self.texels[(y as usize) * (self.width as usize) + x as usize]
    }

    /// texel 좌표 `(u, v)`를 crop texel 범위 `[x0, x1) × [y0, y1)` 안에서 sampling합니다.
    pub(crate) fn sample(
        &self,
        u: f64,
        v: f64,
        crop: (i64, i64, i64, i64),
        sampling: ImageSampling,
    ) -> [f32; 4] {
        let (x0, y0, x1, y1) = crop;
        let clamp_x = |x: i64| x.clamp(x0, x1 - 1);
        let clamp_y = |y: i64| y.clamp(y0, y1 - 1);
        match sampling {
            ImageSampling::Nearest => {
                self.texel(clamp_x(u.floor() as i64), clamp_y(v.floor() as i64))
            }
            ImageSampling::Linear => {
                let (fx, fy) = (u - 0.5, v - 0.5);
                let (ix, iy) = (fx.floor(), fy.floor());
                let (tx, ty) = ((fx - ix) as f32, (fy - iy) as f32);
                let (ix, iy) = (ix as i64, iy as i64);
                let a = self.texel(clamp_x(ix), clamp_y(iy));
                let b = self.texel(clamp_x(ix + 1), clamp_y(iy));
                let c = self.texel(clamp_x(ix), clamp_y(iy + 1));
                let d = self.texel(clamp_x(ix + 1), clamp_y(iy + 1));
                let mut out = [0.0f32; 4];
                for i in 0..4 {
                    let top = a[i] + (b[i] - a[i]) * tx;
                    let bottom = c[i] + (d[i] - c[i]) * tx;
                    out[i] = top + (bottom - top) * ty;
                }
                out
            }
        }
    }
}

/// encoded image를 texel로 바꿉니다(모듈 문서).
pub(crate) fn decode_texture(
    encoded: &EncodedImage,
    color_override: Option<ColorSpace>,
    plan: &OutputColorPlan,
) -> Result<Texture, ImageIssue> {
    if encoded.media() != ImageMedia::Png {
        return Err(ImageIssue::Unsupported(UnsupportedFeature::ImageFormat));
    }
    let decoded =
        decode_png(encoded.bytes(), DecodeLimits::default()).map_err(|error| match error {
            PngDecodeError::Unsupported { .. } => {
                ImageIssue::Unsupported(UnsupportedFeature::ImageFormat)
            }
            other => ImageIssue::Bad(other.to_string()),
        })?;
    if (decoded.width, decoded.height) != (encoded.pixel_width(), encoded.pixel_height()) {
        return Err(ImageIssue::Bad(format!(
            "PNG 크기 {}×{}가 asset 크기 {}×{}와 다릅니다",
            decoded.width,
            decoded.height,
            encoded.pixel_width(),
            encoded.pixel_height()
        )));
    }
    let space = match color_override {
        Some(space) => space,
        None => decoded
            .color
            .color_space()
            .map_err(|_| ImageIssue::Unsupported(UnsupportedFeature::ImageColorProfile))?,
    };
    let texels = decoded
        .rgba
        .chunks_exact(4)
        .map(|px| {
            let alpha = f32::from(px[3]) / 255.0;
            if alpha <= 0.0 {
                return [0.0; 4];
            }
            let converted = plan.convert(OutputColor {
                space,
                rgb: [
                    f32::from(px[0]) / 255.0,
                    f32::from(px[1]) / 255.0,
                    f32::from(px[2]) / 255.0,
                ],
                alpha,
            });
            let [r, g, b] = converted.linear;
            [r * alpha, g * alpha, b * alpha, alpha]
        })
        .collect();
    Ok(Texture {
        width: decoded.width,
        height: decoded.height,
        texels,
    })
}
