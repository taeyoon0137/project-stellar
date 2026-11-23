//! 속성 패널 값의 CSS 값 문법 parser·formatter (UX.2.2, IX.11.13·IX.11.14).
//!
//! 구현은 registry와 같은 crate의 `stellar_document::css::value`에 있습니다(ADR-0075 결정 5). 속성 패널·선언 블록
//! 복사·붙여넣기·CLI·MCP가 같은 parser·formatter를 쓰도록 여기서는 그대로 다시 내보냅니다.

pub use stellar_document::css::value::*;
