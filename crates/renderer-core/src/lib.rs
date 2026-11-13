//! stellar-renderer-core — 공통 렌더러 추상화. scene 의존, 플랫폼 backend를 역참조하지 않습니다.

/// 결정론적 digest (내부).
mod digest;

/// Shape/Path fill tessellation interface와 cache key (XI.6.1).
pub mod tessellation;

/// thread 역할 표시 (XI.7.1).
pub mod thread_role;

/// worker 전용 이미지 decoder interface와 decoded pixel format (XI.7.1).
pub mod image_decode;

/// missing/decoding asset placeholder (XI.7.4).
pub mod placeholder;

/// content 기반 texture cache key (XI.7.3).
pub mod texture_cache;
