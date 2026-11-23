//! blur·shadow 수식 (XI.10.4, ADR-0102 결정 5).
//!
//! - **반경 → 표준편차**: CSS 정의를 그대로 씁니다. shadow blur 반경 `b`(`box-shadow`·`drop-shadow()`)는 `σ = b / 2`
//!   ([`shadow_sigma`]), layer blur 반경 `r`(`filter: blur()`)은 `σ = r`([`layer_blur_sigma`])입니다.
//! - **이산화**: 반경 `R = ceil(3σ)`, 가중치 `w_i = exp(−i² / (2σ²))`(`i ∈ [−R, R]`)를 합 1로 정규화합니다([`kernel`]).
//!   σ = 0이면 `R = 0`, 가중치 하나(그대로)입니다.
//! - **적용**: 가로 → 세로 분리 적용, texel 중심끼리 정수 offset으로 읽고 image 밖은 투명입니다. 값은 working premultiplied
//!   입니다. shadow 1단계는 texel alpha를 모든 채널로 퍼뜨려(`source_alpha`) 읽습니다.
//! - **상한**: σ는 [`MAX_SIGMA`] 이하만 받습니다(신뢰되지 않은 입력의 GPU 비용 상한).

/// σ 상한(pixel).
pub const MAX_SIGMA: f32 = 64.0;

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
}
