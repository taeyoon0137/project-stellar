//! 이미지 decoder interface (XI.7.1).
//!
//! ## worker 전용
//!
//! decode는 [`ThreadRole::Worker`]로 bind된 thread에서만 실행합니다. [`ImageDecoder`]의 모든
//! 메서드는 [`DecodeWorkerScope`]를 인자로 받습니다. scope는 worker thread에서만
//! [`DecodeWorkerScope::current`]로 얻을 수 있고 `!Send`라 다른 thread로 옮길 수 없으므로,
//! UI·Document·Render thread에는 decoder를 호출할 수단이 없습니다. 실제 decoder(platform
//! codec 등)는 이 trait를 구현하고, worker pool은 [`decode_on_worker`]로 실행합니다.
//!
//! ## decoded pixel format
//!
//! 출력은 [`DecodedPixelFormat::Rgba8Unorm`] 하나로 고정합니다.
//!
//! - 채널 순서 R, G, B, A. 채널당 8-bit unsigned normalized(`0..=255` → `0.0..=1.0`).
//! - RGB는 이미지 색공간([`ColorSpace`]: sRGB 또는 Display P3)의 **비선형 전달 함수로 인코딩된**
//!   값입니다. 선형화는 GPU sampling(sRGB texture format) 또는 upload 단계가 담당합니다.
//! - 알파는 선형이며 **비연관(straight) 알파**입니다(core-types `Color`와 같은 규약).
//!   premultiply는 mip 생성·upload(XI.7.2)에서 선형 공간으로 수행합니다.
//! - row-major, 원점은 좌상단, 행 사이 padding 없음(stride = width × 4 bytes).
//! - 그 밖의 색공간(embedded ICC 등)은 decoder가 sRGB 또는 Display P3로 변환해 반환합니다.
//!   색공간 정보가 없는 이미지는 sRGB로 간주합니다.
//! - HDR·채널당 8-bit 초과 정밀도는 초기 범위에서 제외합니다(아키텍처 §10.3).
//!
//! ## 손상 입력
//!
//! decoder 입력은 신뢰되지 않은 bytes입니다(아키텍처 §37.2).
//!
//! - 구현은 어떤 입력에도 panic하지 않고 [`ImageDecodeError`]를 반환해야 합니다. release
//!   profile은 `panic = "abort"`라 `catch_unwind`로 복구할 수 없습니다.
//! - [`decode_on_worker`]는 입력 크기 상한 → header probe → 크기 상한([`DecodeLimits`]) →
//!   decode → header와 출력 일치 검증 순서로 실행해, 거대한 header는 pixel buffer 할당 전에
//!   거부됩니다.
//! - [`DecodedImage`]는 생성 시 buffer 길이를 검증하므로 잘못된 출력이 upload·mip 단계로
//!   넘어가지 않습니다.

use std::fmt;
use std::marker::PhantomData;

use stellar_scene::ColorSpace;

use crate::thread_role::{current_thread_role, ThreadRole};

/// decoded pixel format. 모듈 문서의 규약을 따릅니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodedPixelFormat {
    /// RGBA, 채널당 8-bit unorm, RGB 비선형 인코딩, straight alpha, 좌상단 원점, padding 없음.
    Rgba8Unorm,
}

impl DecodedPixelFormat {
    /// pixel당 byte 수.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            DecodedPixelFormat::Rgba8Unorm => 4,
        }
    }
}

/// 이미지 decode 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDecodeError {
    /// worker thread가 아닌 곳에서 decode를 시도했습니다.
    NotOnWorkerThread {
        /// 현재 thread 역할(bind되지 않았으면 `None`).
        role: Option<ThreadRole>,
    },
    /// 입력이 크기 상한을 넘습니다.
    InputTooLarge {
        /// 입력 길이.
        len: usize,
        /// 상한.
        limit: usize,
    },
    /// decoder가 지원하지 않는 형식입니다.
    UnsupportedFormat,
    /// 입력이 손상되었습니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 너비 또는 높이가 0입니다.
    ZeroDimension,
    /// 크기가 decode 상한을 넘습니다.
    DimensionLimit {
        /// 너비.
        width: u32,
        /// 높이.
        height: u32,
    },
    /// pixel buffer가 크기·format과 맞지 않습니다.
    InvalidBuffer {
        /// 원인.
        reason: &'static str,
    },
    /// decoder 출력이 probe header와 다릅니다.
    OutputMismatch {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageDecodeError::NotOnWorkerThread { role } => {
                write!(
                    f,
                    "이미지 decode는 worker thread 전용입니다(현재: {role:?})"
                )
            }
            ImageDecodeError::InputTooLarge { len, limit } => {
                write!(f, "이미지 입력이 너무 큽니다: {len} bytes (상한 {limit})")
            }
            ImageDecodeError::UnsupportedFormat => f.write_str("지원하지 않는 이미지 형식입니다"),
            ImageDecodeError::Malformed { reason } => write!(f, "손상된 이미지: {reason}"),
            ImageDecodeError::ZeroDimension => f.write_str("이미지 너비·높이는 0일 수 없습니다"),
            ImageDecodeError::DimensionLimit { width, height } => {
                write!(f, "이미지 크기가 decode 상한을 넘습니다: {width}x{height}")
            }
            ImageDecodeError::InvalidBuffer { reason } => {
                write!(f, "decoded pixel buffer 오류: {reason}")
            }
            ImageDecodeError::OutputMismatch { reason } => {
                write!(f, "decoder 출력이 header와 다릅니다: {reason}")
            }
        }
    }
}

impl std::error::Error for ImageDecodeError {}

/// decode된 이미지. pixel은 [`DecodedPixelFormat::Rgba8Unorm`]입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    color_space: ColorSpace,
    pixels: Vec<u8>,
}

/// `width × height × bytes_per_pixel`. 넘침이면 `None`.
fn buffer_len(width: u32, height: u32) -> Option<usize> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    width
        .checked_mul(height)?
        .checked_mul(DecodedPixelFormat::Rgba8Unorm.bytes_per_pixel())
}

impl DecodedImage {
    /// buffer를 검증해 만듭니다. 크기는 0이 아니어야 하고 길이는 정확히 `width × height × 4`
    /// 여야 합니다.
    pub fn new(
        width: u32,
        height: u32,
        color_space: ColorSpace,
        pixels: Vec<u8>,
    ) -> Result<DecodedImage, ImageDecodeError> {
        if width == 0 || height == 0 {
            return Err(ImageDecodeError::ZeroDimension);
        }
        let expected = buffer_len(width, height).ok_or(ImageDecodeError::InvalidBuffer {
            reason: "buffer 크기 계산 넘침",
        })?;
        if pixels.len() != expected {
            return Err(ImageDecodeError::InvalidBuffer {
                reason: "buffer 길이가 width × height × 4와 다릅니다",
            });
        }
        Ok(DecodedImage {
            width,
            height,
            color_space,
            pixels,
        })
    }

    /// pixel format.
    pub fn format(&self) -> DecodedPixelFormat {
        DecodedPixelFormat::Rgba8Unorm
    }

    /// 너비(pixel).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 높이(pixel).
    pub fn height(&self) -> u32 {
        self.height
    }

    /// RGB 인코딩 색공간.
    pub fn color_space(&self) -> ColorSpace {
        self.color_space
    }

    /// 행 간격(bytes) = width × 4.
    pub fn stride(&self) -> usize {
        self.pixels.len() / self.height as usize
    }

    /// 전체 pixel bytes(row-major, 좌상단 원점).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// pixel buffer를 소유권과 함께 꺼냅니다.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    /// `(x, y)` pixel의 RGBA. 범위 밖이면 `None`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = y as usize * self.stride() + x as usize * 4;
        let bytes = self.pixels.get(offset..offset + 4)?;
        Some([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

/// decode 전 header 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
    /// 너비(pixel).
    pub width: u32,
    /// 높이(pixel).
    pub height: u32,
    /// decode 결과 RGB 인코딩 색공간.
    pub color_space: ColorSpace,
}

/// decode 자원 상한. 초기값이며 실제 값은 메모리 벤치마크(XVI) 후 조정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// 입력 bytes 상한.
    pub max_input_bytes: usize,
    /// 너비·높이 각각의 상한(양 backend 공통 최대 2D texture 크기).
    pub max_dimension: u32,
    /// 전체 pixel 수 상한.
    pub max_pixels: u64,
}

impl DecodeLimits {
    /// 기본 상한: 입력 256 MiB, 변 16384 px, 64 Mi pixel(RGBA8 256 MiB).
    pub const DEFAULT: DecodeLimits = DecodeLimits {
        max_input_bytes: 256 * 1024 * 1024,
        max_dimension: 16_384,
        max_pixels: 64 * 1024 * 1024,
    };

    /// 입력 길이를 검사합니다.
    pub fn check_input(&self, len: usize) -> Result<(), ImageDecodeError> {
        if len > self.max_input_bytes {
            return Err(ImageDecodeError::InputTooLarge {
                len,
                limit: self.max_input_bytes,
            });
        }
        Ok(())
    }

    /// header 크기를 검사합니다(0 크기·변 상한·pixel 수 상한).
    pub fn check_header(&self, header: &ImageHeader) -> Result<(), ImageDecodeError> {
        if header.width == 0 || header.height == 0 {
            return Err(ImageDecodeError::ZeroDimension);
        }
        let pixels = u64::from(header.width) * u64::from(header.height);
        if header.width > self.max_dimension
            || header.height > self.max_dimension
            || pixels > self.max_pixels
            || buffer_len(header.width, header.height).is_none()
        {
            return Err(ImageDecodeError::DimensionLimit {
                width: header.width,
                height: header.height,
            });
        }
        Ok(())
    }
}

impl Default for DecodeLimits {
    fn default() -> DecodeLimits {
        DecodeLimits::DEFAULT
    }
}

/// worker thread에서만 얻을 수 있는 decode 권한 표식. `!Send`·`!Sync`입니다.
///
/// ```compile_fail
/// use stellar_renderer_core::image_decode::DecodeWorkerScope;
/// fn assert_send<T: Send>() {}
/// assert_send::<DecodeWorkerScope>();
/// ```
#[derive(Debug)]
pub struct DecodeWorkerScope {
    _not_send: PhantomData<*const ()>,
}

impl DecodeWorkerScope {
    /// 현재 thread가 [`ThreadRole::Worker`]면 scope를 반환합니다.
    pub fn current() -> Result<DecodeWorkerScope, ImageDecodeError> {
        match current_thread_role() {
            Some(ThreadRole::Worker) => Ok(DecodeWorkerScope {
                _not_send: PhantomData,
            }),
            role => Err(ImageDecodeError::NotOnWorkerThread { role }),
        }
    }
}

/// 이미지 decoder interface(worker 전용).
///
/// 구현은 손상 입력에 panic하지 않고 오류를 반환해야 하며, [`ImageDecoder::decode`]는
/// [`ImageDecoder::probe`]가 반환한 header와 같은 크기·색공간의 이미지를 반환해야 합니다.
pub trait ImageDecoder: Send + Sync {
    /// pixel을 decode하지 않고 header만 읽습니다.
    fn probe(
        &self,
        encoded: &[u8],
        scope: &DecodeWorkerScope,
    ) -> Result<ImageHeader, ImageDecodeError>;

    /// header 검사를 통과한 입력을 decode합니다.
    fn decode(
        &self,
        encoded: &[u8],
        header: &ImageHeader,
        scope: &DecodeWorkerScope,
    ) -> Result<DecodedImage, ImageDecodeError>;
}

/// worker thread에서 상한 검사와 출력 검증을 포함해 decode합니다.
///
/// 순서: worker 확인 → 입력 크기 → probe → header 상한 → decode → header와 출력 일치.
pub fn decode_on_worker(
    decoder: &dyn ImageDecoder,
    encoded: &[u8],
    limits: &DecodeLimits,
) -> Result<DecodedImage, ImageDecodeError> {
    let scope = DecodeWorkerScope::current()?;
    limits.check_input(encoded.len())?;
    let header = decoder.probe(encoded, &scope)?;
    limits.check_header(&header)?;
    let image = decoder.decode(encoded, &header, &scope)?;
    if image.width() != header.width || image.height() != header.height {
        return Err(ImageDecodeError::OutputMismatch {
            reason: "크기가 header와 다릅니다",
        });
    }
    if image.color_space() != header.color_space {
        return Err(ImageDecodeError::OutputMismatch {
            reason: "색공간이 header와 다릅니다",
        });
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_role::bind_thread_role;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    /// 테스트 전용 참조 decoder: binary PPM(`P6`, maxval 255) → RGBA8(알파 255, sRGB).
    struct PpmDecoder;

    fn is_space(byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
    }

    /// header를 읽어 (header, pixel data 시작 위치)를 반환합니다.
    fn parse_ppm_header(bytes: &[u8]) -> Result<(ImageHeader, usize), ImageDecodeError> {
        if !bytes.starts_with(b"P6") {
            return Err(ImageDecodeError::UnsupportedFormat);
        }
        let mut pos = 2;
        let mut fields = [0u32; 3];
        for field in fields.iter_mut() {
            // 각 field 앞에는 공백(또는 주석)이 최소 하나 필요합니다.
            let start = pos;
            loop {
                match bytes.get(pos) {
                    Some(&b) if is_space(b) => pos += 1,
                    Some(b'#') => {
                        while let Some(&b) = bytes.get(pos) {
                            pos += 1;
                            if b == b'\n' {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
            }
            if pos == start {
                return Err(ImageDecodeError::Malformed {
                    reason: "field 구분 공백 없음",
                });
            }
            let digits_start = pos;
            let mut value: u32 = 0;
            while let Some(&b) = bytes.get(pos) {
                if !b.is_ascii_digit() {
                    break;
                }
                value = value
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(u32::from(b - b'0')))
                    .ok_or(ImageDecodeError::Malformed {
                        reason: "숫자 넘침",
                    })?;
                pos += 1;
            }
            if pos == digits_start {
                return Err(ImageDecodeError::Malformed {
                    reason: "숫자 없음",
                });
            }
            *field = value;
        }
        // maxval 뒤 정확히 한 개의 공백.
        match bytes.get(pos) {
            Some(&b) if is_space(b) => pos += 1,
            _ => {
                return Err(ImageDecodeError::Malformed {
                    reason: "header 종료 공백 없음",
                })
            }
        }
        let [width, height, maxval] = fields;
        if maxval == 0 || maxval > 65_535 {
            return Err(ImageDecodeError::Malformed {
                reason: "maxval 범위 밖",
            });
        }
        if maxval != 255 {
            return Err(ImageDecodeError::UnsupportedFormat);
        }
        Ok((
            ImageHeader {
                width,
                height,
                color_space: ColorSpace::Srgb,
            },
            pos,
        ))
    }

    impl ImageDecoder for PpmDecoder {
        fn probe(
            &self,
            encoded: &[u8],
            _scope: &DecodeWorkerScope,
        ) -> Result<ImageHeader, ImageDecodeError> {
            parse_ppm_header(encoded).map(|(header, _)| header)
        }

        fn decode(
            &self,
            encoded: &[u8],
            header: &ImageHeader,
            _scope: &DecodeWorkerScope,
        ) -> Result<DecodedImage, ImageDecodeError> {
            let (parsed, offset) = parse_ppm_header(encoded)?;
            if parsed != *header {
                return Err(ImageDecodeError::Malformed {
                    reason: "header 불일치",
                });
            }
            let data = encoded.get(offset..).unwrap_or_default();
            let pixel_count = (header.width as usize)
                .checked_mul(header.height as usize)
                .ok_or(ImageDecodeError::Malformed {
                    reason: "크기 넘침",
                })?;
            if Some(data.len()) != pixel_count.checked_mul(3) {
                return Err(ImageDecodeError::Malformed {
                    reason: "pixel data 길이 불일치",
                });
            }
            let mut pixels = Vec::with_capacity(pixel_count * 4);
            for rgb in data.chunks_exact(3) {
                pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            DecodedImage::new(header.width, header.height, header.color_space, pixels)
        }
    }

    /// worker 역할로 bind된 새 thread에서 `f`를 실행합니다.
    fn on_worker<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        thread::spawn(move || {
            let _guard = bind_thread_role(ThreadRole::Worker).unwrap();
            f()
        })
        .join()
        .unwrap()
    }

    fn ppm(width: u32, height: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = format!("P6\n# 테스트\n{width} {height}\n255\n").into_bytes();
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn decoded_pixel_format_contract() {
        let image = on_worker(|| {
            decode_on_worker(
                &PpmDecoder,
                &ppm(2, 2, &[255, 0, 0, 0, 128, 255, 1, 2, 3, 4, 5, 6]),
                &DecodeLimits::DEFAULT,
            )
        })
        .unwrap();
        assert_eq!(image.format(), DecodedPixelFormat::Rgba8Unorm);
        assert_eq!(image.format().bytes_per_pixel(), 4);
        assert_eq!((image.width(), image.height()), (2, 2));
        assert_eq!(image.stride(), 8);
        assert_eq!(image.color_space(), ColorSpace::Srgb);
        // row-major, 좌상단 원점, straight alpha(불투명 255).
        assert_eq!(
            image.pixels(),
            &[255, 0, 0, 255, 0, 128, 255, 255, 1, 2, 3, 255, 4, 5, 6, 255]
        );
        assert_eq!(image.pixel(1, 0), Some([0, 128, 255, 255]));
        assert_eq!(image.pixel(0, 1), Some([1, 2, 3, 255]));
        assert_eq!(image.pixel(2, 0), None);
        assert_eq!(image.pixel(0, 2), None);
    }

    #[test]
    fn decode_is_worker_only() {
        let input = ppm(1, 1, &[1, 2, 3]);
        // 역할 없는 thread.
        let unbound = {
            let input = input.clone();
            thread::spawn(move || decode_on_worker(&PpmDecoder, &input, &DecodeLimits::DEFAULT))
                .join()
                .unwrap()
        };
        assert_eq!(
            unbound.unwrap_err(),
            ImageDecodeError::NotOnWorkerThread { role: None }
        );
        // UI·Document·Render thread.
        for role in [ThreadRole::Ui, ThreadRole::Document, ThreadRole::Render] {
            let input = input.clone();
            let result = thread::spawn(move || {
                let _guard = bind_thread_role(role).unwrap();
                assert!(DecodeWorkerScope::current().is_err());
                decode_on_worker(&PpmDecoder, &input, &DecodeLimits::DEFAULT)
            })
            .join()
            .unwrap();
            assert_eq!(
                result.unwrap_err(),
                ImageDecodeError::NotOnWorkerThread { role: Some(role) }
            );
        }
        // worker thread.
        let ok = on_worker(move || decode_on_worker(&PpmDecoder, &input, &DecodeLimits::DEFAULT));
        assert_eq!(ok.unwrap().pixel(0, 0), Some([1, 2, 3, 255]));
    }

    /// 결정론적 LCG.
    struct Lcg(u64);

    impl Lcg {
        fn next_u8(&mut self) -> u8 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 56) as u8
        }
    }

    /// 손상 이미지 corpus: 손으로 만든 사례 + 유효 파일의 모든 절단·byte 변형 + 무작위 bytes.
    fn malformed_corpus() -> Vec<Vec<u8>> {
        let mut corpus: Vec<Vec<u8>> = [
            &b""[..],
            b"P",
            b"P6",
            b"P6\n",
            b"P3\n1 1\n255\n\x00\x00\x00",
            b"PNG\r\n\x1a\n",
            b"P6\n1 1\n255",
            b"P6\n1 1\n255\n",
            b"P6\n0 0\n255\n",
            b"P6\n0 5\n255\n",
            b"P6\n-1 1\n255\n\x00\x00\x00",
            b"P6\n4294967296 1\n255\n",
            b"P6\n99999999999999999999 1\n255\n",
            b"P6\n4000000000 4000000000\n255\n",
            b"P6\n16385 1\n255\n",
            b"P6\n1 1\n0\n\x00\x00\x00",
            b"P6\n1 1\n65536\n\x00\x00\x00",
            b"P6\n1 1\n65535\n\x00\x00\x00\x00\x00\x00",
            b"P6 # \xff\xfe unterminated comment",
            b"P6#\n1#\n1#\n255\n\x01\x02\x03",
            b"P61 1 255\n\x00\x00\x00",
            b"P6\n1 1\n255\n\x00\x00",
            b"P6\n1 1\n255\n\x00\x00\x00\x00",
            b"P6\n2 2\n255\n\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        ]
        .iter()
        .map(|b| b.to_vec())
        .collect();

        let valid = ppm(
            3,
            2,
            &[
                10, 20, 30, 40, 50, 60, 70, 80, 90, 1, 2, 3, 4, 5, 6, 7, 8, 9,
            ],
        );
        for end in 0..valid.len() {
            corpus.push(valid[..end].to_vec());
        }
        for index in 0..valid.len() {
            for replacement in [0x00, 0xFF, b' ', b'#', b'9', b'P', b'\n', b'-'] {
                let mut mutated = valid.clone();
                mutated[index] = replacement;
                corpus.push(mutated);
            }
        }
        let mut rng = Lcg(0x5EED);
        for sample in 0..512usize {
            let len = sample % 97;
            let mut bytes: Vec<u8> = if sample % 2 == 0 {
                b"P6 ".to_vec()
            } else {
                Vec::new()
            };
            bytes.extend((0..len).map(|_| rng.next_u8()));
            corpus.push(bytes);
        }
        corpus
    }

    #[test]
    fn malformed_corpus_does_not_panic() {
        // 검증 기준(XI.7.1): malformed image corpus panic 없음.
        let corpus = malformed_corpus();
        let total = corpus.len();
        let (ok, err) = on_worker(move || {
            let mut ok = 0usize;
            let mut err = 0usize;
            for input in &corpus {
                match decode_on_worker(&PpmDecoder, input, &DecodeLimits::DEFAULT) {
                    Ok(image) => {
                        // 성공한 출력은 항상 format 계약을 지킵니다.
                        assert_eq!(
                            image.pixels().len(),
                            image.stride() * image.height() as usize
                        );
                        assert_eq!(image.stride(), image.width() as usize * 4);
                        ok += 1;
                    }
                    Err(_) => err += 1,
                }
            }
            (ok, err)
        });
        // 모든 입력이 panic 없이 결과를 냈습니다.
        assert_eq!(ok + err, total);
        assert!(err > 0);
    }

    #[test]
    fn specific_malformed_inputs_map_to_errors() {
        let cases: Vec<(Vec<u8>, ImageDecodeError)> = vec![
            (b"".to_vec(), ImageDecodeError::UnsupportedFormat),
            (b"PNG".to_vec(), ImageDecodeError::UnsupportedFormat),
            (b"P6\n0 5\n255\n".to_vec(), ImageDecodeError::ZeroDimension),
            (
                b"P6\n4294967296 1\n255\n".to_vec(),
                ImageDecodeError::Malformed {
                    reason: "숫자 넘침",
                },
            ),
            // 거대한 header는 pixel buffer 할당 전에 거부됩니다.
            (
                b"P6\n4000000000 4000000000\n255\n".to_vec(),
                ImageDecodeError::DimensionLimit {
                    width: 4_000_000_000,
                    height: 4_000_000_000,
                },
            ),
            (
                b"P6\n16385 1\n255\n".to_vec(),
                ImageDecodeError::DimensionLimit {
                    width: 16_385,
                    height: 1,
                },
            ),
            (
                b"P6\n1 1\n255\n\x00\x00".to_vec(),
                ImageDecodeError::Malformed {
                    reason: "pixel data 길이 불일치",
                },
            ),
            (
                b"P6\n1 1\n65535\n\x00\x00\x00\x00\x00\x00".to_vec(),
                ImageDecodeError::UnsupportedFormat,
            ),
        ];
        let results = on_worker(move || {
            cases
                .into_iter()
                .map(|(input, expected)| {
                    (
                        decode_on_worker(&PpmDecoder, &input, &DecodeLimits::DEFAULT),
                        expected,
                    )
                })
                .collect::<Vec<_>>()
        });
        for (index, (result, expected)) in results.into_iter().enumerate() {
            assert_eq!(result.unwrap_err(), expected, "case {index}");
        }
    }

    /// probe·decode 결과를 지정하는 테스트 decoder(계약 위반 구현 모사).
    struct ScriptedDecoder {
        header: ImageHeader,
        output: fn() -> Result<DecodedImage, ImageDecodeError>,
        decode_calls: AtomicUsize,
    }

    impl ImageDecoder for ScriptedDecoder {
        fn probe(
            &self,
            _encoded: &[u8],
            _scope: &DecodeWorkerScope,
        ) -> Result<ImageHeader, ImageDecodeError> {
            Ok(self.header)
        }

        fn decode(
            &self,
            _encoded: &[u8],
            _header: &ImageHeader,
            _scope: &DecodeWorkerScope,
        ) -> Result<DecodedImage, ImageDecodeError> {
            self.decode_calls.fetch_add(1, Ordering::SeqCst);
            (self.output)()
        }
    }

    fn one_by_one() -> Result<DecodedImage, ImageDecodeError> {
        DecodedImage::new(1, 1, ColorSpace::Srgb, vec![0, 0, 0, 255])
    }

    fn header(width: u32, height: u32, color_space: ColorSpace) -> ImageHeader {
        ImageHeader {
            width,
            height,
            color_space,
        }
    }

    #[test]
    fn contract_violating_decoder_output_is_rejected() {
        let cases = vec![
            // 출력 크기가 header와 다름.
            (
                header(2, 1, ColorSpace::Srgb),
                ImageDecodeError::OutputMismatch {
                    reason: "크기가 header와 다릅니다",
                },
                1,
            ),
            // 출력 색공간이 header와 다름.
            (
                header(1, 1, ColorSpace::DisplayP3),
                ImageDecodeError::OutputMismatch {
                    reason: "색공간이 header와 다릅니다",
                },
                1,
            ),
            // header가 0 크기·상한 초과면 decode를 호출하지 않습니다.
            (
                header(0, 1, ColorSpace::Srgb),
                ImageDecodeError::ZeroDimension,
                0,
            ),
            (
                header(16_384, 16_384, ColorSpace::Srgb),
                ImageDecodeError::DimensionLimit {
                    width: 16_384,
                    height: 16_384,
                },
                0,
            ),
        ];
        on_worker(move || {
            for (index, (header, expected, calls)) in cases.into_iter().enumerate() {
                let decoder = ScriptedDecoder {
                    header,
                    output: one_by_one,
                    decode_calls: AtomicUsize::new(0),
                };
                let result = decode_on_worker(&decoder, b"x", &DecodeLimits::DEFAULT);
                assert_eq!(result.unwrap_err(), expected, "case {index}");
                assert_eq!(
                    decoder.decode_calls.load(Ordering::SeqCst),
                    calls,
                    "case {index}"
                );
            }
        });
    }

    #[test]
    fn input_size_limit_is_checked_before_probe() {
        let limits = DecodeLimits {
            max_input_bytes: 4,
            ..DecodeLimits::DEFAULT
        };
        let result = on_worker(move || decode_on_worker(&PpmDecoder, b"P6\n1 1\n255\n", &limits));
        assert_eq!(
            result.unwrap_err(),
            ImageDecodeError::InputTooLarge { len: 11, limit: 4 }
        );
    }

    #[test]
    fn decoded_image_rejects_inconsistent_buffers() {
        assert_eq!(
            DecodedImage::new(0, 1, ColorSpace::Srgb, Vec::new()),
            Err(ImageDecodeError::ZeroDimension)
        );
        assert!(matches!(
            DecodedImage::new(2, 1, ColorSpace::Srgb, vec![0; 7]),
            Err(ImageDecodeError::InvalidBuffer { .. })
        ));
        assert!(matches!(
            DecodedImage::new(2, 1, ColorSpace::Srgb, vec![0; 9]),
            Err(ImageDecodeError::InvalidBuffer { .. })
        ));
        // 크기 계산 넘침도 panic 없이 거부.
        assert!(matches!(
            DecodedImage::new(u32::MAX, u32::MAX, ColorSpace::Srgb, Vec::new()),
            Err(ImageDecodeError::InvalidBuffer { .. })
        ));
        assert!(DecodedImage::new(2, 1, ColorSpace::DisplayP3, vec![0; 8]).is_ok());
    }
}
