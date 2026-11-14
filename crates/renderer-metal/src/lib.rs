//! stellar-renderer-metal — macOS Metal backend.
//!
//! [`MetalBackend`]는 `stellar-renderer-core`의
//! [`RendererBackend`](stellar_renderer_core::backend::RendererBackend)를 Metal로 구현합니다
//! (아키텍처 §10.1). 공통 계층은 handle·descriptor·graph만 넘기고, Metal 객체는 이 crate 밖으로 나가지
//! 않습니다.
//!
//! - **binding**: 외부 crate 없이 Objective-C runtime(`objc_msgSend`)과 Metal·QuartzCore·CoreGraphics
//!   C 함수를 직접 선언합니다([`objc`] 모듈 문서). 대상은 Apple Silicon macOS 26 이상입니다(ADR-0006).
//! - **surface**: platform 셸의 CAMetalLayer를 [`MetalBackend::with_layer`]로 넘겨받습니다. surface 없는
//!   [`MetalBackend::headless`]는 external texture에만 그립니다(offscreen 적합성 harness).
//!
//! `cfg(target_os = "macos")`로 게이트되어 비대상 플랫폼에서는 내용이 제외됩니다(feature 미사용).
#![cfg(target_os = "macos")]

/// 최소 Objective-C runtime FFI.
mod objc;

/// Metal·QuartzCore·CoreGraphics C 선언과 상수.
mod sys;

/// texture format 대응.
mod format;

/// `MTLDevice`·command queue (XI.8.1).
mod device;

/// CAMetalLayer 연결·drawable (XI.8.1), resize·frame skip (XI.8.2).
mod surface;

/// texture 생성 (XI.8.1).
mod texture;

/// RenderGraph pass encoding.
mod encode;

/// `RendererBackend` 구현.
mod backend;

pub use backend::MetalBackend;
pub use surface::{LayerOptions, SurfaceStats};

/// backend 식별자.
pub const BACKEND: &str = "metal";

#[cfg(test)]
mod tests;
