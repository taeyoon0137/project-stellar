//! stellar-layout — Project Stellar 결정론적 레이아웃 엔진. core-types만 직접 의존합니다.

/// layout revision newtype (X.1.4).
pub mod revision;

/// node별 layout input hash와 output cache (X.1.4).
pub mod cache;

/// 레이아웃 수치 rounding 정책과 logical subpixel 단위 (X.1.5).
pub mod rounding;

/// 레이아웃 좌표·크기 (X.1.1).
pub mod geometry;

/// 레이아웃 입력 속성 식별자와 dirty class registry (X.1.1, X.1.3).
pub mod property;

/// 레이아웃 오류 (X.1.1).
pub mod error;

/// 레이아웃 입력 tree (X.1.1).
pub mod input;

/// 레이아웃 출력 box·결과 (X.1.1).
pub mod output;

/// 축별 크기 결정 방식 (X.6.1).
pub mod sizing;

/// content intrinsic 측정 hook (X.6.4).
pub mod intrinsic;

/// 측정 cache key와 계산 간 측정 cache (X.7.2).
pub mod measure;

/// 레이아웃 엔진 진입점 (X.2.1).
pub mod engine;

/// 자유 배치 (X.2.1~X.2.3).
pub mod free;

/// Stack(Flex 계열) 배치 (X.3).
pub mod stack;

/// local transform 적용 정책 (X.2.4).
pub mod transform;

/// Stack 자식 grow·shrink·basis (X.3.3).
pub mod flex;

/// Grid 배치 (X.4).
pub mod grid;

/// Grid track sizing (X.4.2, crate 내부).
mod grid_tracks;

/// Overlay 배치 (X.5.1).
pub mod overlay;

/// Anchor·Constraint 배치 (X.5).
pub mod constraint;

/// layout worker result guard (X.7.5).
pub mod guard;

/// 문서 원천을 Fragment flatten한 layout tree view (X.1.2).
pub mod view;

/// 레이아웃 적합성 fixture corpus (X.11.1).
pub mod corpus;

/// canonical bytes writer/reader (crate 내부).
mod codec;

/// 공간 분배와 잔여 subpixel 처리 (crate 내부).
mod distribute;

#[cfg(test)]
mod test_support;
