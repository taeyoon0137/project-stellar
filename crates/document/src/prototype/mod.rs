//! 노드 기반 프로토타이핑의 문서 쪽 정의 (XXX.1.2, XXX.1.3, XXX.1.5, ADR-0025 결정 2, ADR-0094).
//!
//! 프로토타입 그래프는 새 원본이 아니라 binding·expression·상태 머신·MotionSpec 위의 projection입니다. 이 module은
//! projection과 편집이 함께 쓰는 닫힌 어휘(socket 종류, 속성 포트 타입, node 종류 registry)와, 그래프 고유의 저장
//! 원본인 희소 배치 hint와 프로토타입 텍스트 원본을 정의합니다. projection·편집 plan·프리뷰 평가는
//! `stellar-prototype` crate가 맡습니다.

/// socket 종류와 속성 입력 포트 타입.
pub mod socket;

/// 프로토타입 node 종류 registry(닫힌 집합).
pub mod node_kind;

/// 희소 배치 hint.
pub mod layout_hint;

/// 프로토타입 텍스트 원본 v1(MotionSpec·배치 hint).
pub mod source;
