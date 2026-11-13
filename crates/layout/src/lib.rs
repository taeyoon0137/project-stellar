//! stellar-layout — Project Stellar 결정론적 레이아웃 엔진. core-types만 직접 의존합니다.

/// layout revision newtype (X.1.4).
pub mod revision;

/// node별 layout input hash와 output cache (X.1.4).
pub mod cache;

/// 레이아웃 수치 rounding 정책과 logical subpixel 단위 (X.1.5).
pub mod rounding;
