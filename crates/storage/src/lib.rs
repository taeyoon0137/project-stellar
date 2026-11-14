//! stellar-storage — 생성 기반 저장소 골격. 의존: core-types.

/// canonical binary 인코딩 (V.1).
pub mod canonical;

/// content hash (V.1.5, ADR-0004).
pub mod hash;

/// chunk 저장소 (V.2).
pub mod chunk;

/// atomic 파일 쓰기 (V.2.5).
pub mod atomic_file;

/// 저장소 파일 접근 guard(보안 감사 STOR, crate 내부 전용).
mod fs_guard;

/// generation 저장 (V.4).
pub mod generation;

/// 저장 트랜잭션 (V.5).
pub mod save;

/// garbage collection (V.7).
pub mod gc;

/// 스키마 버전과 마이그레이션 (V.8).
pub mod migration;
