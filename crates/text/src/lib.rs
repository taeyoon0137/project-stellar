//! stellar-text — Project Stellar 결정론적 텍스트 엔진.
//!
//! 내부 crate 중에서는 core-types만 직접 의존합니다. 외부 의존성은 ADR-0016과
//! `config/dependencies.yaml`의 승인 목록(순수 Rust shaping·Unicode 알고리즘 crate)만 씁니다.
//! raw pointer·FFI가 없도록 `unsafe` 코드를 금지합니다.

#![forbid(unsafe_code)]

/// 폰트 출처, 파일 hash, face 참조 (X.8.1).
pub mod font;

/// 폰트 face registry와 hash+index dedup (X.8.2).
pub mod registry;

/// 명시적 font fallback chain (X.8.3).
pub mod fallback;

/// missing font 진단 (X.8.4).
pub mod missing_font;

/// 공통 shaping 엔진 wrapper (X.9.1).
pub mod shaping;

#[cfg(test)]
pub(crate) mod test_fonts;
