//! Web adapter (XXII.2). IR만 읽어 HTML·CSS를 만듭니다.

/// 생성 HTML 구조 모델과 결정론 writer.
pub mod html;

/// 접근성 tree → HTML/ARIA 매핑 (XXII.2.4).
pub mod aria;

/// 생성 HTML 정적 접근성 검사 (XXII.2.4).
pub mod check;

#[cfg(test)]
mod tests;
