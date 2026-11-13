//! stellar-commands — Project Stellar 편집 command 모델 (Phase VII).
//!
//! 의존 방향은 `history -> commands -> document -> core-types`입니다. 문서 편집은 사용자·
//! 시스템·migration·향후 agent 모두 이 crate의 같은 command 경로를 지납니다(기술 아키텍처
//! §31.4).

/// command 식별자와 출처 (VII.1.1).
pub mod id;
