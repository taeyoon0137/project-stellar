//! stellar-prototype — 노드 기반 프로토타이핑 엔진 (Phase XXX, ADR-0025 결정 2, ADR-0094).
//!
//! 프로토타입 그래프는 새 원본이 아니라 binding·expression·상태 머신·MotionSpec 위의 projection입니다. 이 crate는
//! projection과 결정론적 자동 배치, 그래프 편집을 기존 command로 낮춘 plan, 포트 공개, 진단 매핑, 그리고 프리뷰의 논리
//! 시계·timing evaluator·tick 처리를 맡습니다. 시스템 시각·파일·네트워크에 접근하지 않습니다(XXX.6.3 inventory). 문서를
//! 바꾸는 길은 [`edit::PrototypeTransaction`] 하나입니다.

/// 프리뷰 논리 시계 (XXX.3.1).
pub mod clock;
