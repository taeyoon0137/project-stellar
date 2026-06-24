//! stellar-renderer-d3d12 — Windows Direct3D 12 backend 골격.
//!
//! `cfg(target_os = "windows")`로 게이트되어 비대상 플랫폼에서는 내용이 제외됩니다(feature 미사용).
#![cfg(target_os = "windows")]

/// backend 식별자.
pub const BACKEND: &str = "d3d12";
