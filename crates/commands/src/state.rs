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

use crate::change::ChangeSet;
use crate::command::{Command, CommandError};
use crate::guard::TransactionGuard;
use crate::request::CommandRequest;

/// command 실행 결과: 변경 집합(VII.1.4)과 undo용 역 command.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOutcome {
    /// 바뀐 node·속성·무효화 class·새 revision.
    pub changes: ChangeSet,
    /// 적용 결과를 되돌리는 command.
    pub inverse: Command,
}

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

    /// 다음 revision(오버플로는 오류).
    pub(crate) fn next_revision(&self) -> Result<DocumentRevision, CommandError> {
        self.revision
            .0
            .checked_add(1)
            .map(DocumentRevision)
            .ok_or(CommandError::RevisionOverflow)
    }

    /// 요청 하나를 실행합니다: 검사(VII.1.3) → validate → invert → apply.
    ///
    /// 성공하면 revision을 하나 올리고 변경 집합과 역 command를 반환합니다. apply가 도중에
    /// 실패하면 transaction guard(VII.1.5)가 변경 전 상태로 되돌리고 revision을 유지합니다.
    pub fn execute(&mut self, request: &CommandRequest) -> Result<CommandOutcome, CommandError> {
        request.check_state(self)?;
        let new_revision = self.next_revision()?;
        request.command.validate(&self.model)?;
        let inverse = request.command.invert(&self.model)?;
        let mut guard = TransactionGuard::begin(&mut self.model);
        if let Err(error) = request.command.apply(guard.ctx()) {
            return Err(rollback_with(guard, error));
        }
        let changes = guard.commit().finish(new_revision);
        self.revision = new_revision;
        Ok(CommandOutcome { changes, inverse })
    }
}

/// guard를 되돌리고 원래 오류를 반환합니다. 되돌리기도 실패하면 둘 다 보고합니다.
pub(crate) fn rollback_with(guard: TransactionGuard<'_>, error: CommandError) -> CommandError {
    match guard.rollback() {
        Ok(()) => error,
        Err(rollback) => CommandError::RollbackFailed {
            original: Box::new(error),
            rollback: Box::new(rollback),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::node_keys;
    use crate::command::test_fixtures::{sid, small_model};
    use crate::command::TestProbe;
    use stellar_core_types::property::PropertyPath;
    use stellar_document::modifier_metadata::InvalidationSet;

    #[test]
    fn single_property_command_reports_expected_change_set() {
        // 검증 기준(VII.1.4): 단일 property command expected change set 일치.
        let mut state = DocumentState::with_revision(small_model(), DocumentRevision(3));
        let request = CommandRequest::at(
            &state,
            Command::TestProbe(TestProbe {
                node: sid(12),
                name: "이름".into(),
                fail_after_apply: false,
            }),
        );
        let outcome = state.execute(&request).unwrap();
        assert_eq!(outcome.changes.changed_nodes(), vec![sid(12)]);
        assert_eq!(
            outcome.changes.changed_properties(),
            vec![PropertyPath::with_keys(
                sid(12),
                vec![node_keys::DISPLAY_NAME]
            )]
        );
        assert_eq!(
            outcome.changes.node_invalidation(sid(12)),
            Some(InvalidationSet::EMPTY)
        );
        assert_eq!(outcome.changes.invalidation(), InvalidationSet::EMPTY);
        assert_eq!(outcome.changes.new_revision(), DocumentRevision(4));
        assert_eq!(state.revision(), DocumentRevision(4));
        assert_eq!(
            outcome.inverse,
            Command::TestProbe(TestProbe {
                node: sid(12),
                name: "n12".into(),
                fail_after_apply: false,
            })
        );
    }

    #[test]
    fn rejected_request_keeps_revision() {
        let mut state = DocumentState::with_revision(small_model(), DocumentRevision(3));
        let stale = CommandRequest::new(
            DocumentRevision(2),
            Command::TestProbe(TestProbe {
                node: sid(12),
                name: "x".into(),
                fail_after_apply: false,
            }),
        );
        assert!(matches!(
            state.execute(&stale),
            Err(CommandError::RevisionMismatch { .. })
        ));
        assert_eq!(state.revision(), DocumentRevision(3));
        let mut max = DocumentState::with_revision(small_model(), DocumentRevision(u64::MAX));
        let request = CommandRequest::at(
            &max,
            Command::TestProbe(TestProbe {
                node: sid(12),
                name: "x".into(),
                fail_after_apply: false,
            }),
        );
        assert_eq!(max.execute(&request), Err(CommandError::RevisionOverflow));
        assert_eq!(max.model().record(sid(12)).unwrap().display_name, "n12");
    }
}
