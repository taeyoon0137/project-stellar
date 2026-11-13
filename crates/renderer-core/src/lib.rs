//! stellar-renderer-core — 공통 렌더러 추상화. scene 의존, 플랫폼 backend를 역참조하지 않습니다.

/// 결정론적 digest (내부).
mod digest;

/// Shape/Path fill tessellation interface와 cache key (XI.6.1).
pub mod tessellation;
