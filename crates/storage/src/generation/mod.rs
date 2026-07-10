//! generation 저장 (V.4).
//!
//! generation manifest와 current pointer, loader, fallback scanner를 담습니다.

/// generation manifest (V.4.1).
pub mod manifest;

/// current generation pointer 파일 (V.4.3).
pub mod current;

/// 프로젝트 열기 loader (V.4.4).
pub mod loader;

/// 정상 세대 fallback scanner (V.4.5).
pub mod scanner;

/// 프로젝트 저장소 open과 읽기 전용 recovery (V.4.6).
pub mod open;

/// 지연 로딩 loader (V.6.6).
pub mod lazy;
