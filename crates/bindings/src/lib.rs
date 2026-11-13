//! stellar-bindings — Project Stellar 토큰과 통합 Binding Graph. core-types와 expressions만 직접 의존합니다.

/// 토큰 정의·이름·타입·TokenSet (VIII.5.1).
pub mod token;

/// 프로젝트·라이브러리·문서 scope 토큰 registry (VIII.5.2).
pub mod registry;
