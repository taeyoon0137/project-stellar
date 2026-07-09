//! canonical binary 인코딩 (V.1).
//!
//! 규칙 문서 `docs/architecture/storage-canonical-primitives.md`(R1~R6)와
//! golden fixture `docs/fixtures/storage/canonical-primitives.yaml`을 1차 기준으로 따릅니다.

/// canonical encoder (V.1.2).
pub mod encode;

/// canonical decoder (V.1.3).
pub mod decode;

/// map entry: (key bytes, canonical 인코딩된 value bytes).
pub type MapEntry = (Vec<u8>, Vec<u8>);
