//! 테스트 전용 지원 코드(`cfg(test)` 또는 기본값이 꺼진 `test-support` feature).
//!
//! crate 의존 정책 때문에 같은 상수를 다른 crate가 따로 보관하는 경우, 정책 밖 통합 테스트
//! (`tests/integration/tests/constant_copies.rs`)가 이 재노출 값으로 두 사본을 대조합니다. 프로덕션 코드는
//! 이 module을 쓰지 않습니다.

/// surface 색 변환(XI.10.5)의 선형 Display P3 → 선형 sRGB 행렬(행 우선, D65). expressions 색 함수와 같은 값이어야
/// 합니다.
pub const P3_TO_SRGB: [[f32; 3]; 3] = crate::output_color::P3_TO_SRGB;

/// surface 색 변환(XI.10.5)의 선형 sRGB → 선형 Display P3 행렬(행 우선, D65). expressions 색 함수와 같은 값이어야
/// 합니다.
pub const SRGB_TO_P3: [[f32; 3]; 3] = crate::output_color::SRGB_TO_P3;
