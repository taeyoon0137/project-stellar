//! 앱 runtime의 cache 예산 (XVI.5.3, ADR-0108 결정 5).
//!
//! 공통 budget manager·priority·제거 순서는 renderer-core([`stellar_renderer_core::cache`])에 있습니다. 이 모듈은 앱
//! 쪽 cache(문서 썸네일)를 둡니다.

/// 문서 썸네일 cache (XVI.5.3).
pub mod thumbnail;
