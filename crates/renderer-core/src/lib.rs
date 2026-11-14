//! stellar-renderer-core — 공통 렌더러 추상화. scene 의존, 플랫폼 backend를 역참조하지 않습니다.

/// 결정론적 digest (내부).
mod digest;

/// Shape/Path fill tessellation interface와 cache key (XI.6.1).
pub mod tessellation;

/// stroke tessellation: 폭·join·cap·dash (XI.6.2).
pub mod stroke;

/// tessellation worker pool과 stale revision 결과 폐기 (XI.6.3).
pub mod tessellation_worker;

/// glyph outline/bitmap atlas interface와 key (XI.6.4).
pub mod glyph;

/// thread 역할 표시 (XI.7.1).
pub mod thread_role;

/// worker 전용 이미지 decoder interface와 decoded pixel format (XI.7.1).
pub mod image_decode;

/// missing/decoding asset placeholder (XI.7.4).
pub mod placeholder;

/// content 기반 texture cache key (XI.7.3).
pub mod texture_cache;

/// backend 독립 Render Graph (XI.4.1).
pub mod render_graph;

/// GPU backend interface (XI.5.1).
pub mod backend;

/// generational GPU handle (XI.5.2).
pub mod gpu_handle;

/// resource upload queue와 frame-local allocator (XI.5.3).
pub mod frame_resources;

/// present queue 깊이 ≤ 1 frame scheduler (XI.5.5).
pub mod frame_scheduler;

/// scene primitive → opaque/transparent/effect pass compiler (XI.4.2).
pub mod pass_compiler;

/// clip stack → scissor/stencil/offscreen mask 계획 (XI.4.3).
pub mod clip_plan;

/// blend/opacity/filter 의존 분석과 최소 offscreen 경계 (XI.4.4).
pub mod offscreen;

/// draw item batching key와 순서 보존 batching (XI.4.5).
pub mod batching;

/// RenderGraph compile trace (XI.4.6).
pub mod graph_trace;

/// 테스트 전용 SHA-256 (asset 계층 모사).
#[cfg(test)]
mod test_sha256;

/// 테스트 전용 scene fixture.
#[cfg(test)]
pub(crate) mod test_scene;
