//! Web adapter (XXII.2). IR만 읽어 HTML·CSS를 만듭니다.

/// 생성 HTML 구조 모델과 결정론 writer.
pub mod html;

/// IR node → HTML semantic 구조 (XXII.2.1).
pub mod structure;

/// 토큰 Web 출력 계획 (XXII.2.3).
pub mod tokens;

/// IrExpr → CSS·JavaScript 식 (XXII.2.5, XXII.2.6).
pub mod expr;

/// Context → Web 출력 (XXII.2.5).
pub mod context;

/// 생성 JavaScript 값·식별자 표기 (XXII.2.3, XXII.5.1).
pub mod js;

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
