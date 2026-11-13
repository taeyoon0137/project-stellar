//! fixture 생성 오류.
//!
//! 생성기는 구성상 유효한 입력만 만들지만, 실제 문서 모델·validator·serializer를 거치므로 그
//! 단계의 오류를 그대로 전달합니다. 오류가 나면 fixture를 쓰지 않습니다(부분 결과 없음).

use std::fmt;

use stellar_document::definitions::DefinitionError;
use stellar_document::model::{LoadModelError, ModelError};
use stellar_document::record_codec::PartitionError;
use stellar_project::document_io::serialize::SerializeError;
use stellar_storage::chunk::split::{SplitError, SplitTreeError};

/// fixture 생성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    /// 명세가 허용 범위를 벗어납니다(생성 전 거부).
    InvalidSpec {
        /// 이유.
        reason: &'static str,
    },
    /// 문서·페이지·View 정의 구성 실패.
    Definition(DefinitionError),
    /// 레코드 → 런타임 모델 로드 실패(validator 위반 포함).
    Load(LoadModelError),
    /// 런타임 모델 내부 불일치.
    Model(ModelError),
    /// 생성 결과에 미해결 참조가 있습니다(벤치마크 fixture는 모든 참조가 해결돼야 합니다).
    Unresolved {
        /// 미해결 참조 수.
        count: usize,
    },
    /// chunk model 직렬화 실패.
    Serialize(SerializeError),
    /// View subtree 분할 실패.
    Partition(PartitionError),
    /// split planner 입력 구성 실패.
    SplitTree(SplitTreeError),
    /// split planner 실패.
    Split(SplitError),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FixtureError::InvalidSpec { reason } => write!(f, "fixture 명세 오류: {reason}"),
            FixtureError::Definition(e) => write!(f, "문서 정의: {e}"),
            FixtureError::Load(e) => write!(f, "문서 로드: {e}"),
            FixtureError::Model(e) => write!(f, "문서 모델: {e}"),
            FixtureError::Unresolved { count } => {
                write!(f, "생성된 fixture에 미해결 참조 {count}건")
            }
            FixtureError::Serialize(e) => write!(f, "직렬화: {e}"),
            FixtureError::Partition(e) => write!(f, "subtree 분할: {e}"),
            FixtureError::SplitTree(e) => write!(f, "split 입력: {e}"),
            FixtureError::Split(e) => write!(f, "split 계획: {e}"),
        }
    }
}

impl std::error::Error for FixtureError {}
