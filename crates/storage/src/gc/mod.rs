//! garbage collection (V.7).
//!
//! 보존 정책, reachable mark scanner, sweep, scheduler interface를 담습니다.

/// 보존 generation 정책 (V.7.1).
pub mod retention;

/// reachable mark scanner (V.7.2).
pub mod mark;
