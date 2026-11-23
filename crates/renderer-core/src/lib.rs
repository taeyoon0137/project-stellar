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

/// mipmap 생성과 texture upload task (XI.7.2).
pub mod mipmap;

/// 공통 cache budget manager·priority·증분 제거·GPU OOM 재시도 (XVI.5.2·XVI.5.4·XVI.5.6).
pub mod cache;

/// glyph atlas·tessellation·이미지 mipmap cache의 budget manager 등록 (XVI.5.3).
pub mod cache_adapters;

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

/// benchmark 결과의 present queue 깊이·dropped stale frame 지표 (XI.11.5).
pub mod present_metrics;

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

/// surface 출력 색공간 협상과 gamut 경고 (XI.10.5), HDR 미지원 상태 (XI.10.6).
pub mod output_color;

/// 선형 working space와 출력 target encode 공통 수식 (XI.10.1).
pub mod color;

/// alpha 합성과 blend mode 수식 (XI.10.2).
pub mod blend;

/// 렌더 효과 수식, 공통 shader ABI, 적합성 fixture·harness (XI.10.2~XI.10.4, XI.11.1~XI.11.3).
pub mod effects;

/// 테스트 전용 지원 코드: 다른 crate가 따로 보관하는 상수의 대조용 재노출.
///
/// 이 crate의 unit test와, 기본값이 꺼진 `test-support` feature를 `[dev-dependencies]`에서 켠 crate의
/// 테스트에서만 빌드됩니다.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// 테스트 전용 SHA-256 (asset 계층 모사).
#[cfg(test)]
mod test_sha256;

/// 테스트 전용 scene fixture.
#[cfg(test)]
pub(crate) mod test_scene;
