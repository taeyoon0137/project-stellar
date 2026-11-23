//! 렌더 효과 수식, 공통 shader ABI, 적합성 fixture (ADR-0102).
//!
//! - [`abi`]: backend shader가 따르는 정점·uniform byte 배치.
//! - [`fill`]: fixture용 fill tessellator(가로 band 사다리꼴 분해).
//! - [`fixture`]: backend 중립 적합성 fixture 모델.
//! - [`raster`]: CPU 기준 rasterizer.
//! - [`lower`]: fixture → render graph + draw command(`RendererBackend` generic).
//! - [`artifact`]: reference image, raw RGBA·PNG 기록.
//! - [`compare`]: 채널·structural 허용 오차 비교와 diff artifact.
//! - [`inventory`]: 기능별 fixture 목록.

/// 공통 shader ABI byte 배치.
pub mod abi;

/// reference image와 raw RGBA·PNG artifact (XI.11.1).
pub mod artifact;

/// image 비교·허용 오차 config·diff artifact (XI.11.2).
pub mod compare;

/// fixture용 fill tessellator.
pub mod fill;

/// 적합성 fixture 모델 (XI.11.1).
pub mod fixture;

/// 적합성 fixture 목록.
pub mod inventory;

/// fixture lowering (XI.11.1).
pub mod lower;

/// CPU 기준 rasterizer (XI.11.1).
pub mod raster;
