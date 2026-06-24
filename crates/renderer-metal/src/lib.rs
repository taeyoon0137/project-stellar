//! stellar-renderer-metal — macOS Metal backend 골격.
//!
//! `cfg(target_os = "macos")`로 게이트되어 비대상 플랫폼에서는 내용이 제외됩니다(feature 미사용).
#![cfg(target_os = "macos")]

/// backend 식별자.
pub const BACKEND: &str = "metal";
