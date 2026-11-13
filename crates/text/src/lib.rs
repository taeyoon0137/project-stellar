//! stellar-text — Project Stellar 결정론적 텍스트 엔진. core-types만 직접 의존합니다.

/// 폰트 출처, 파일 hash, face 참조 (X.8.1).
pub mod font;

/// 폰트 face registry와 hash+index dedup (X.8.2).
pub mod registry;
