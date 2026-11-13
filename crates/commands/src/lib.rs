//! stellar-commands — Project Stellar 편집 command 모델 (Phase VII).
//!
//! 의존 방향은 `history -> commands -> document -> core-types`입니다. 문서 편집은 사용자·
//! 시스템·migration·향후 agent 모두 이 crate의 같은 command 경로를 지납니다(기술 아키텍처
//! §31.4).

/// command 식별자와 출처 (VII.1.1).
pub mod id;

/// command 종류 numeric registry (VII.1.2).
pub mod kind;

/// 직렬화 가능한 closed enum command와 validate·apply·invert 인터페이스 (VII.1.2).
pub mod command;

/// command 적용 context (VII.1.2).
pub mod edit;

/// command canonical 직렬화 (VII.1.2).
pub mod codec;
