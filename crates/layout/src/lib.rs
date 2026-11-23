//! stellar-layout — Project Stellar 결정론적 레이아웃 엔진. core-types만 직접 의존합니다.
//!
//! 엔진 입력은 CSS longhand와 CSS 값입니다([`style`], ADR-0074 결정 6, ADR-0076). 배치 의미는 CSS를 따릅니다:
//! `display: block`([`block`]), `display: flex`([`flex`]), `position: absolute`·`relative`([`position`]).

/// layout revision newtype (X.1.4).
pub mod revision;

/// node별 layout input hash와 output cache (X.1.4).
pub mod cache;

/// 레이아웃 수치 rounding 정책과 logical subpixel 단위 (X.1.5).
pub mod rounding;

/// 레이아웃 좌표·크기 (X.1.1).
pub mod geometry;

/// 레이아웃 입력 CSS 값 (ADR-0076).
pub mod style;

/// 레이아웃 입력 속성 식별자와 dirty class registry (X.1.1, X.1.3).
pub mod property;

/// command invalidation → layout dirty set (X.7.1).
pub mod dirty;

/// 레이아웃 오류 (X.1.1).
pub mod error;

/// 레이아웃 입력 tree (X.1.1).
pub mod input;

/// 레이아웃 출력 box·결과 (X.1.1).
pub mod output;

/// content intrinsic 측정 hook (X.6.4).
pub mod intrinsic;

/// subtree input hash와 계산 간 배치 cache (X.7.2).
pub mod measure;

/// 레이아웃 엔진 진입점 (X.2.1).
pub mod engine;

/// `display: block` 배치.
pub mod block;

/// `display: flex` 배치 (IX.11.4).
pub mod flex;

/// `position: absolute`·`relative` 배치.
pub mod position;

/// local transform 적용 정책 (X.2.4).
pub mod transform;

/// layout worker result guard (X.7.5).
pub mod guard;

/// 영향 범위만 다시 배치하는 증분 레이아웃 (X.7.3).
pub mod incremental;

/// 증분 레이아웃 단일 leaf 변경 benchmark (X.7.4).
pub mod bench;

/// 문서 원천을 Fragment flatten한 layout tree view (X.1.2).
pub mod view;

/// 레이아웃 적합성 fixture corpus (X.11.1).
pub mod corpus;

/// canonical bytes writer/reader (crate 내부).
mod codec;

/// style canonical bytes (crate 내부).
mod style_codec;

/// 계산 중 memo·cache map hasher (crate 내부).
mod fast_hash;

/// 공간 분배와 잔여 subpixel 처리 (crate 내부).
mod distribute;

#[cfg(test)]
mod test_support;
