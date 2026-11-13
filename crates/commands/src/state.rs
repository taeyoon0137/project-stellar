//! command 대상 문서 상태 (VII.1.3).
//!
//! 한 창의 인메모리 문서(기술 아키텍처 §22.2: 창마다 독립 문서 상태)는 런타임 모델과 현재
//! [`DocumentRevision`]의 묶음입니다. revision은 완료된 transaction마다 하나씩 증가하는 단조
//! 카운터이며, command의 base revision(VII.1.3)과 비교해 낡은 요청을 거부합니다.
//!
//! 모델을 바꾸는 공개 경로는 command 실행뿐입니다(직접 `&mut DocumentModel`을 내주지
//! 않음). 사용자·시스템·agent 편집이 모두 같은 command 경로를 지나게 하는 장치입니다.

use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;

/// 한 창의 인메모리 문서 상태.
#[derive(Debug)]
pub struct DocumentState {
    pub(crate) model: DocumentModel,
    pub(crate) revision: DocumentRevision,
}

impl DocumentState {
    /// 로드한 모델로 revision 0 상태를 만듭니다.
    pub fn new(model: DocumentModel) -> DocumentState {
        DocumentState::with_revision(model, DocumentRevision(0))
    }

    /// 지정 revision으로 만듭니다(예: 재로드 후 이어지는 revision).
    pub fn with_revision(model: DocumentModel, revision: DocumentRevision) -> DocumentState {
        DocumentState { model, revision }
    }

    /// 현재 모델(읽기 전용).
    pub fn model(&self) -> &DocumentModel {
        &self.model
    }

    /// 현재 revision.
    pub fn revision(&self) -> DocumentRevision {
        self.revision
    }

    /// 모델을 꺼냅니다(창 종료·저장 경로).
    pub fn into_model(self) -> DocumentModel {
        self.model
    }
}
