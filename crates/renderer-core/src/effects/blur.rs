//! blur·shadow 수식 (XI.10.4, ADR-0102 결정 5).
//!
//! - **반경 → 표준편차**: CSS 정의를 그대로 씁니다. shadow blur 반경 `b`(`box-shadow`·`drop-shadow()`)는 `σ = b / 2`
//!   ([`shadow_sigma`]), layer blur 반경 `r`(`filter: blur()`)은 `σ = r`([`layer_blur_sigma`])입니다.
//! - **이산화**: 반경 `R = ceil(3σ)`, 가중치 `w_i = exp(−i² / (2σ²))`(`i ∈ [−R, R]`)를 합 1로 정규화합니다([`kernel`]).
//!   σ = 0이면 `R = 0`, 가중치 하나(그대로)입니다.
//! - **적용**: 가로 → 세로 분리 적용, texel 중심끼리 정수 offset으로 읽고 image 밖은 투명입니다. 값은 working premultiplied
//!   입니다. shadow 1단계는 texel alpha를 모든 채널로 퍼뜨려(`source_alpha`) 읽습니다.
//! - **상한**: 한 blur pass의 σ는 [`MAX_SIGMA`] 이하만 받습니다(신뢰되지 않은 입력의 GPU 비용 상한, kernel 탭 수 상한).
//! - **낮은 해상도 효과(ADR-0130 결정 2)**: device σ가 [`MAX_SIGMA`]를 넘으면 효과만 2의 거듭제곱 배수 `d`
//!   ([`effect_downsample`], [`MAX_DOWNSAMPLE`] 이하)로 줄인 texture에서 σ / d로 blur하고, 합성할 때 bilinear로 다시
//!   키웁니다. 줄이기는 한 단계마다 가로·세로 2배 상자 평균(bilinear 표본 하나, 가장자리 clamp)이고 크기는
//!   `ceil(n / 2)`입니다([`downsampled_extent`]). 내용 가장자리 coverage 해상도(supersampling)는 바뀌지 않습니다.
//!   상자 평균·bilinear가 더하는 분산은 `d < σ / 32`라 σ의 상대 오차 0.02% 이하입니다. device σ 상한은
//!   [`MAX_DEVICE_SIGMA`]입니다.

/// 한 blur pass의 σ 상한(pixel).
pub const MAX_SIGMA: f32 = 64.0;

/// 효과 낮은 해상도 배수 상한(2의 거듭제곱).
pub const MAX_DOWNSAMPLE: u32 = 64;

/// device σ 상한(pixel): [`MAX_SIGMA`] × [`MAX_DOWNSAMPLE`].
pub const MAX_DEVICE_SIGMA: f32 = MAX_SIGMA * MAX_DOWNSAMPLE as f32;

/// σ 1당 효과가 번지는 거리(pixel). kernel 반경 3σ에 낮은 해상도 줄이기·키우기가 더하는 거리(`2d < σ / 16`)를 더한
/// 상한입니다. 내보내기 tile 여백이 씁니다.
pub const REACH_PER_SIGMA: f32 = 3.0 + 1.0 / 16.0;

/// device σ를 그릴 낮은 해상도 배수: σ / d ≤ [`MAX_SIGMA`]인 가장 작은 2의 거듭제곱 `d`. [`MAX_DOWNSAMPLE`]로도 모자라면
/// `None`(σ 상한 초과)입니다. σ가 유한하지 않으면 `None`입니다.
pub fn effect_downsample(sigma: f32) -> Option<u32> {
    if !sigma.is_finite() || sigma < 0.0 {
        return None;
    }
    let mut d = 1u32;
    while sigma / d as f32 > MAX_SIGMA {
        if d >= MAX_DOWNSAMPLE {
            return None;
        }
        d *= 2;
    }
    Some(d)
}

/// `d`배(2의 거듭제곱) 줄인 크기: 한 단계마다 `ceil(n / 2)`.
pub fn downsampled_extent(n: u32, d: u32) -> u32 {
    let mut n = n.max(1);
    let mut d = d.max(1);
    while d > 1 {
        n = n.div_ceil(2);
        d /= 2;
    }
    n
}

/// shadow blur 반경의 표준편차(CSS `box-shadow`·`drop-shadow()`).
pub fn shadow_sigma(blur_radius: f32) -> f32 {
    blur_radius / 2.0
}

/// layer blur 반경의 표준편차(CSS `filter: blur()`).
pub fn layer_blur_sigma(radius: f32) -> f32 {
    radius
}

/// kernel 반경 `ceil(3σ)`.
pub fn kernel_radius(sigma: f32) -> u32 {
    if sigma <= 0.0 {
        return 0;
    }
    (3.0 * sigma).ceil() as u32
}

/// 정규화한 가중치(`2R + 1`개, 가운데가 offset 0).
pub fn kernel(sigma: f32) -> Vec<f32> {
    let radius = kernel_radius(sigma) as i64;
    if radius == 0 {
        return vec![1.0];
    }
    let weights: Vec<f32> = (-radius..=radius)
        .map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = weights.iter().sum();
    weights.into_iter().map(|w| w / sum).collect()
}

/// σ가 받는 범위(유한, `[0, MAX_SIGMA]`)면 참.
pub fn valid_sigma(sigma: f32) -> bool {
    sigma.is_finite() && (0.0..=MAX_SIGMA).contains(&sigma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_radius_mapping_and_kernel_shape() {
        assert_eq!(shadow_sigma(8.0), 4.0);
        assert_eq!(layer_blur_sigma(3.0), 3.0);
        assert_eq!(kernel_radius(0.0), 0);
        assert_eq!(kernel_radius(1.0), 3);
        assert_eq!(kernel_radius(1.1), 4);
        assert_eq!(kernel(0.0), vec![1.0]);
        let k = kernel(2.0);
        assert_eq!(k.len(), 13);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        // 대칭이고 가운데가 가장 큽니다.
        for i in 0..6 {
            assert!((k[i] - k[12 - i]).abs() < 1e-9);
            assert!(k[i] < k[i + 1]);
        }
        // σ = 2에서 w_1 / w_0 = exp(−1/8).
        assert!((k[7] / k[6] - (-0.125f32).exp()).abs() < 1e-6);
        assert!(valid_sigma(0.0) && valid_sigma(MAX_SIGMA));
        assert!(!valid_sigma(-1.0) && !valid_sigma(f32::NAN) && !valid_sigma(MAX_SIGMA + 1.0));
    }

    #[test]
    fn large_sigma_picks_the_smallest_power_of_two_downsample() {
        assert_eq!(effect_downsample(0.0), Some(1));
        assert_eq!(effect_downsample(MAX_SIGMA), Some(1));
        assert_eq!(effect_downsample(MAX_SIGMA + 0.5), Some(2));
        assert_eq!(effect_downsample(160.0), Some(4));
        assert_eq!(effect_downsample(MAX_DEVICE_SIGMA), Some(MAX_DOWNSAMPLE));
        assert_eq!(effect_downsample(MAX_DEVICE_SIGMA + 1.0), None);
        assert_eq!(effect_downsample(f32::NAN), None);
        // d ≥ 2이면 d < σ / 32(REACH_PER_SIGMA의 근거).
        for sigma in [64.5f32, 100.0, 127.9, 128.1, 1000.0, 4000.0] {
            let d = effect_downsample(sigma).unwrap() as f32;
            assert!(sigma / d <= MAX_SIGMA && 2.0 * d < sigma / 16.0, "{sigma}");
        }
        assert_eq!(downsampled_extent(10, 1), 10);
        assert_eq!(downsampled_extent(10, 4), 3);
        assert_eq!(downsampled_extent(9, 2), 5);
        assert_eq!(downsampled_extent(1, 64), 1);
    }
}
