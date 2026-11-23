//! Web adapter (XXII.2). IR만 읽어 HTML·CSS를 만듭니다.

/// 생성 HTML 구조 모델과 결정론 writer.
pub mod html;

/// 접근성 tree → HTML/ARIA 매핑 (XXII.2.4).
pub mod aria;

/// 생성 HTML 정적 접근성 검사 (XXII.2.4).
pub mod check;

/// 고정 Free Frame의 CSS 출력 값과 formatter.
pub mod frame;

/// 기본값 세트 차이의 특이도 0 기반 규칙과 layout 선언 블록 (IX.11.30, ADR-0089).
pub mod layout;

#[cfg(test)]
mod tests;
