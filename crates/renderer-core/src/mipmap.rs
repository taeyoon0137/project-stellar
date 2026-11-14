//! mipmap 생성과 texture upload task (XI.7.2).
//!
//! decode된 이미지([`DecodedImage`], XI.7.1)를 GPU texture로 올릴 mip chain으로 만들고, backend texture
//! 생성과 level별 upload 요청([`UploadQueue`], XI.5.3)을 준비합니다.
//!
//! ## mip level 수
//!
//! [`MipmapMode::Full`]은 1×1까지 전체 chain이며 level 수는 `floor(log2(max(width, height))) + 1`입니다.
//! level `i` 크기는 각 변 `max(1, 변 >> i)`입니다. [`MipmapMode::None`]은 base level 하나입니다.
//!
//! ## texture pixel 규약
//!
//! 출력 pixel은 `Rgba8UnormSrgb` texture용입니다.
//!
//! - RGB는 **선형광에서 premultiply한 값**을 이미지 색공간의 transfer 곡선(ADR-0002, sRGB·Display P3 공통)으로
//!   인코딩한 8-bit입니다. GPU가 sRGB texture를 sampling하면 선형 premultiplied 값을 얻으므로 bilinear·trilinear
//!   filtering이 선형 공간에서 올바르게 일어납니다. 알파는 선형 8-bit입니다.
//! - 색공간(gamut)은 바꾸지 않습니다. chain은 원본 [`ColorSpace`]를 담고, working 색공간 변환은 색 변환
//!   계층(XI.10)이 합니다.
//!
//! ## 축소 filter
//!
//! 다음 level pixel은 이전 level의 2×2 box 평균입니다. 홀수 변의 마지막 pixel은 남는 행·열까지 3개를 평균해
//! 원본 pixel을 버리지 않습니다. 평균은 선형 premultiplied 16-bit 정수 working buffer에서 하고 level마다 다시
//! 양자화하지 않으므로 오차가 쌓이지 않습니다.
//!
//! ## 결정론
//!
//! 전달 함수는 고정 256-entry 정수 표(decode: 8-bit → 선형 16-bit)와 그 중간값 경계 탐색(encode)으로 계산합니다.
//! 실행 중 `powf`를 쓰지 않으므로 결과 bytes가 플랫폼과 무관하게 같습니다. 표는 IEC 61966-2-1 곡선을 배정밀도로
//! 계산해 반올림한 값이며 테스트가 core-types 전달 함수와 대조합니다.
//!
//! mip 생성은 CPU 비용이 커서 worker 전용입니다([`DecodeWorkerScope`] 인자).

use std::fmt;
use std::sync::Arc;

use stellar_scene::ColorSpace;

use crate::backend::{
    BackendError, DeviceCapabilities, GpuTextureDesc, RendererBackend, TextureHandle,
    TextureRegion, TextureUsage,
};
use crate::frame_resources::{UploadError, UploadQueue, UploadRequest, UploadTarget};
use crate::image_decode::{DecodeWorkerScope, DecodedImage};
use crate::render_graph::{SampleCount, TextureFormat};
use crate::texture_cache::MipmapMode;

/// 8-bit 인코딩 채널 → 선형광 16-bit(0..=65535). IEC 61966-2-1 곡선(ADR-0002).
#[rustfmt::skip]
const DECODE_U16: [u16; 256] = [
    0, 20, 40, 60, 80, 99, 119, 139, 159, 179, 199, 219,
    241, 264, 288, 313, 340, 367, 396, 427, 458, 491, 526, 562,
    599, 637, 677, 718, 761, 805, 851, 898, 947, 997, 1048, 1101,
    1156, 1212, 1270, 1330, 1391, 1453, 1517, 1583, 1651, 1720, 1790, 1863,
    1937, 2013, 2090, 2170, 2250, 2333, 2418, 2504, 2592, 2681, 2773, 2866,
    2961, 3058, 3157, 3258, 3360, 3464, 3570, 3678, 3788, 3900, 4014, 4129,
    4247, 4366, 4488, 4611, 4736, 4864, 4993, 5124, 5257, 5392, 5530, 5669,
    5810, 5953, 6099, 6246, 6395, 6547, 6700, 6856, 7014, 7174, 7335, 7500,
    7666, 7834, 8004, 8177, 8352, 8528, 8708, 8889, 9072, 9258, 9445, 9635,
    9828, 10022, 10219, 10417, 10619, 10822, 11028, 11235, 11446, 11658, 11873, 12090,
    12309, 12530, 12754, 12980, 13209, 13440, 13673, 13909, 14146, 14387, 14629, 14874,
    15122, 15371, 15623, 15878, 16135, 16394, 16656, 16920, 17187, 17456, 17727, 18001,
    18277, 18556, 18837, 19121, 19407, 19696, 19987, 20281, 20577, 20876, 21177, 21481,
    21787, 22096, 22407, 22721, 23038, 23357, 23678, 24002, 24329, 24658, 24990, 25325,
    25662, 26001, 26344, 26688, 27036, 27386, 27739, 28094, 28452, 28813, 29176, 29542,
    29911, 30282, 30656, 31033, 31412, 31794, 32179, 32567, 32957, 33350, 33745, 34143,
    34544, 34948, 35355, 35764, 36176, 36591, 37008, 37429, 37852, 38278, 38706, 39138,
    39572, 40009, 40449, 40891, 41337, 41785, 42236, 42690, 43147, 43606, 44069, 44534,
    45002, 45473, 45947, 46423, 46903, 47385, 47871, 48359, 48850, 49344, 49841, 50341,
    50844, 51349, 51858, 52369, 52884, 53401, 53921, 54445, 54971, 55500, 56032, 56567,
    57105, 57646, 58190, 58737, 59287, 59840, 60396, 60955, 61517, 62082, 62650, 63221,
    63795, 64372, 64952, 65535,
];

/// 선형 16-bit → 가장 가까운 8-bit 인코딩 값(같은 거리면 큰 값).
fn encode_u16(linear: u16) -> u8 {
    // DECODE_U16은 엄격히 증가하므로 이웃 두 값의 중간점을 넘은 개수가 가장 가까운 code입니다.
    let value = u32::from(linear);
    let mut low = 0usize;
    let mut high = 255usize;
    while low < high {
        let mid = (low + high) / 2;
        let threshold = (u32::from(DECODE_U16[mid]) + u32::from(DECODE_U16[mid + 1])).div_ceil(2);
        if value >= threshold {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low as u8
}

/// mip 생성·upload 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipmapError {
    /// worker thread가 아닙니다.
    NotOnWorkerThread,
    /// texture가 device 최대 크기를 넘습니다.
    ExceedsDevice {
        /// 너비.
        width: u32,
        /// 높이.
        height: u32,
        /// device 한 변 최대.
        max: u32,
    },
}

impl fmt::Display for MipmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MipmapError::NotOnWorkerThread => f.write_str("mip 생성은 worker thread 전용입니다"),
            MipmapError::ExceedsDevice { width, height, max } => write!(
                f,
                "texture {width}x{height}가 device 최대 크기 {max}를 넘습니다"
            ),
        }
    }
}

impl std::error::Error for MipmapError {}

/// `width × height`(각 1 이상)의 전체 chain level 수 = `floor(log2(max)) + 1`. 0 크기는 0.
pub const fn mip_level_count(width: u32, height: u32) -> u32 {
    let max = if width > height { width } else { height };
    if max == 0 {
        0
    } else {
        32 - max.leading_zeros()
    }
}

/// level `level`의 크기 `(max(1, width >> level), max(1, height >> level))`.
pub const fn mip_level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    (level_extent(width, level), level_extent(height, level))
}

/// 한 변의 level 크기 `max(1, v >> level)`.
const fn level_extent(v: u32, level: u32) -> u32 {
    if level >= 32 {
        return 1;
    }
    let s = v >> level;
    if s == 0 {
        1
    } else {
        s
    }
}

/// mip level 하나(모듈 문서의 texture pixel 규약, 행 padding 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipLevel {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl MipLevel {
    /// 너비.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// RGBA8 pixel(sRGB 인코딩 premultiplied, 알파 선형).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// `(x, y)` pixel. 범위 밖이면 `None`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        let p = self.pixels.get(offset..offset + 4)?;
        Some([p[0], p[1], p[2], p[3]])
    }
}

/// texture 하나의 mip chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipChain {
    color_space: ColorSpace,
    levels: Vec<MipLevel>,
}

impl MipChain {
    /// RGB 인코딩 색공간(원본 그대로).
    pub const fn color_space(&self) -> ColorSpace {
        self.color_space
    }

    /// level 목록(0 = base).
    pub fn levels(&self) -> &[MipLevel] {
        &self.levels
    }

    /// level 수.
    pub fn level_count(&self) -> u32 {
        self.levels.len() as u32
    }

    /// base 너비.
    pub fn width(&self) -> u32 {
        self.levels.first().map_or(0, MipLevel::width)
    }

    /// base 높이.
    pub fn height(&self) -> u32 {
        self.levels.first().map_or(0, MipLevel::height)
    }

    /// 전체 pixel bytes.
    pub fn total_bytes(&self) -> u64 {
        self.levels
            .iter()
            .map(|level| level.pixels.len() as u64)
            .sum()
    }
}

/// 선형 premultiplied 16-bit working pixel `[r, g, b, a]`(a는 8-bit × 257).
type Working = [u16; 4];

fn to_working(pixel: &[u8]) -> Working {
    let alpha = u32::from(pixel[3]);
    let premul = |c: u8| ((u32::from(DECODE_U16[c as usize]) * alpha + 127) / 255) as u16;
    [
        premul(pixel[0]),
        premul(pixel[1]),
        premul(pixel[2]),
        (alpha * 257) as u16,
    ]
}

fn to_output(pixels: &[Working]) -> Arc<[u8]> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        out.extend_from_slice(&[
            encode_u16(p[0]),
            encode_u16(p[1]),
            encode_u16(p[2]),
            // a16 = a8 × 257의 평균 → 가장 가까운 8-bit.
            ((u32::from(p[3]) + 128) / 257) as u8,
        ]);
    }
    out.into()
}

/// dst index `i`가 평균할 src 범위(시작, 개수).
fn taps(i: u32, src: u32, dst: u32) -> (u32, u32) {
    if src == 1 {
        return (0, 1);
    }
    let start = 2 * i;
    let count = if src % 2 == 1 && i + 1 == dst { 3 } else { 2 };
    (start, count)
}

fn downsample(src: &[Working], sw: u32, sh: u32) -> (Vec<Working>, u32, u32) {
    let (dw, dh) = mip_level_size(sw, sh, 1);
    let mut out = Vec::with_capacity(dw as usize * dh as usize);
    for y in 0..dh {
        let (y0, ny) = taps(y, sh, dh);
        for x in 0..dw {
            let (x0, nx) = taps(x, sw, dw);
            let mut sum = [0u32; 4];
            for yy in y0..y0 + ny {
                for xx in x0..x0 + nx {
                    let p = src[yy as usize * sw as usize + xx as usize];
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u32::from(v);
                    }
                }
            }
            let n = nx * ny;
            out.push(sum.map(|s| ((s + n / 2) / n) as u16));
        }
    }
    (out, dw, dh)
}

/// decode된 이미지로 mip chain을 만듭니다(worker 전용, 모듈 문서 규약).
pub fn generate_mip_chain(
    image: &DecodedImage,
    mode: MipmapMode,
    _scope: &DecodeWorkerScope,
) -> MipChain {
    let (width, height) = (image.width(), image.height());
    let mut working: Vec<Working> = image.pixels().chunks_exact(4).map(to_working).collect();
    let count = match mode {
        MipmapMode::None => 1,
        MipmapMode::Full => mip_level_count(width, height),
    };
    let mut levels = Vec::with_capacity(count as usize);
    levels.push(MipLevel {
        width,
        height,
        pixels: to_output(&working),
    });
    let (mut w, mut h) = (width, height);
    for _ in 1..count {
        let (next, nw, nh) = downsample(&working, w, h);
        levels.push(MipLevel {
            width: nw,
            height: nh,
            pixels: to_output(&next),
        });
        working = next;
        w = nw;
        h = nh;
    }
    MipChain {
        color_space: image.color_space(),
        levels,
    }
}

/// [`generate_mip_chain`]을 현재 thread가 worker일 때만 실행합니다.
pub fn generate_mip_chain_on_worker(
    image: &DecodedImage,
    mode: MipmapMode,
) -> Result<MipChain, MipmapError> {
    let scope = DecodeWorkerScope::current().map_err(|_| MipmapError::NotOnWorkerThread)?;
    Ok(generate_mip_chain(image, mode, &scope))
}

/// mip chain을 GPU texture로 올리는 작업.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureUploadTask {
    desc: GpuTextureDesc,
    color_space: ColorSpace,
    levels: Vec<(TextureRegion, Arc<[u8]>)>,
}

/// upload 요청을 넣은 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadSummary {
    /// 만든 texture.
    pub texture: TextureHandle,
    /// 넣은 요청 수(= level 수).
    pub requests: usize,
    /// 넣은 bytes.
    pub bytes: u64,
}

impl TextureUploadTask {
    /// chain으로 task를 만듭니다. texture는 `Rgba8UnormSrgb`, sampled 전용입니다.
    pub fn new(chain: &MipChain) -> TextureUploadTask {
        let levels = chain
            .levels
            .iter()
            .enumerate()
            .map(|(index, level)| {
                (
                    TextureRegion {
                        mip_level: index as u32,
                        x: 0,
                        y: 0,
                        width: level.width,
                        height: level.height,
                    },
                    Arc::clone(&level.pixels),
                )
            })
            .collect();
        TextureUploadTask {
            desc: GpuTextureDesc {
                width: chain.width(),
                height: chain.height(),
                format: TextureFormat::Rgba8UnormSrgb,
                mip_levels: chain.level_count(),
                samples: SampleCount::X1,
                usage: TextureUsage {
                    sampled: true,
                    render_target: false,
                },
            },
            color_space: chain.color_space,
            levels,
        }
    }

    /// texture descriptor.
    pub const fn desc(&self) -> GpuTextureDesc {
        self.desc
    }

    /// RGB 인코딩 색공간.
    pub const fn color_space(&self) -> ColorSpace {
        self.color_space
    }

    /// level별 upload 영역.
    pub fn regions(&self) -> impl Iterator<Item = TextureRegion> + '_ {
        self.levels.iter().map(|(region, _)| *region)
    }

    /// 전체 bytes.
    pub fn total_bytes(&self) -> u64 {
        self.levels.iter().map(|(_, data)| data.len() as u64).sum()
    }

    /// device가 이 texture를 만들 수 있는지 확인합니다.
    pub fn check_device(&self, caps: &DeviceCapabilities) -> Result<(), MipmapError> {
        let max = caps.max_texture_dimension;
        if self.desc.width > max || self.desc.height > max {
            return Err(MipmapError::ExceedsDevice {
                width: self.desc.width,
                height: self.desc.height,
                max,
            });
        }
        Ok(())
    }

    /// level별 upload 요청을 base level부터 queue에 넣습니다. 하나라도 거부되면 넣지 않습니다.
    pub fn enqueue(
        &self,
        texture: TextureHandle,
        queue: &mut UploadQueue,
    ) -> Result<UploadSummary, UploadError> {
        // 먼저 검증용 사본 queue에 넣어 보고, 모두 통과하면 실제 queue에 넣습니다(부분 upload 방지).
        let mut staged = UploadQueue::new();
        for (region, data) in &self.levels {
            staged.push(UploadRequest {
                target: UploadTarget::Texture {
                    texture,
                    region: *region,
                    format: self.desc.format,
                },
                data: Arc::clone(data),
            })?;
        }
        for (region, data) in &self.levels {
            queue.push(UploadRequest {
                target: UploadTarget::Texture {
                    texture,
                    region: *region,
                    format: self.desc.format,
                },
                data: Arc::clone(data),
            })?;
        }
        Ok(UploadSummary {
            texture,
            requests: self.levels.len(),
            bytes: self.total_bytes(),
        })
    }

    /// texture를 만들고 upload 요청을 queue에 넣습니다(render thread, frame encode 전).
    ///
    /// `caps`는 `backend`의 `init`이 돌려준 capability입니다. 먼저 [`TextureUploadTask::check_device`]로 device
    /// 최대 크기를 확인하고, 넘으면 texture를 만들지 않고 [`TextureUploadError::Device`]를 돌려줍니다(backend
    /// 오류에 기대지 않음). upload 요청이 거부되면 만든 texture를 바로 파괴합니다(아직 어떤 submission도
    /// 참조하지 않음).
    pub fn start(
        &self,
        backend: &mut dyn RendererBackend,
        caps: &DeviceCapabilities,
        queue: &mut UploadQueue,
    ) -> Result<UploadSummary, TextureUploadError> {
        self.check_device(caps)
            .map_err(TextureUploadError::Device)?;
        let texture = backend
            .create_texture(&self.desc)
            .map_err(TextureUploadError::Backend)?;
        match self.enqueue(texture, queue) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                backend
                    .destroy_texture(texture)
                    .map_err(TextureUploadError::Backend)?;
                Err(TextureUploadError::Upload(error))
            }
        }
    }
}

/// [`TextureUploadTask::start`] 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureUploadError {
    /// device가 이 texture를 만들 수 없습니다(texture를 만들지 않음).
    Device(MipmapError),
    /// backend 오류.
    Backend(BackendError),
    /// upload 요청 검증 오류.
    Upload(UploadError),
}

impl fmt::Display for TextureUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextureUploadError::Device(error) => write!(f, "texture 생성 불가: {error}"),
            TextureUploadError::Backend(error) => write!(f, "texture 생성 실패: {error}"),
            TextureUploadError::Upload(error) => write!(f, "texture upload 요청 오류: {error}"),
        }
    }
}

impl std::error::Error for TextureUploadError {}

#[cfg(test)]
mod tests;
