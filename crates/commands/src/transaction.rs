//! 여러 command를 묶는 `CommandTransaction` (VII.1.6).
//!
//! transaction은 요청([`CommandRequest`]) 목록을 **전부 성공하거나 전부 롤백**하는 단위로
//! 실행합니다. 한 transaction guard(VII.1.5) 안에서 순서대로 검사·검증·역 command 계산·적용을
//! 하므로, 뒤 command는 앞 command가 바꾼 문서를 기준으로 검증되고, 어느 단계에서든 실패하면
//! 앞 command의 효과까지 모두 되돌립니다.
//!
//! - 모든 요청의 base revision은 transaction 시작 시점 revision과 같아야 합니다(VII.1.3).
//! - 성공하면 revision은 transaction당 **한 번** 오릅니다.
//! - 역 transaction은 각 command의 역을 **역순**으로 담고, base revision은 실행 후 revision입니다.
//!   같은 [`CommandId`]·출처를 유지해 history가 원 transaction과 짝지을 수 있습니다.
//!
//! 직렬화: id 16 + origin([`CommandOrigin::encode_into`]) + 요청 수 `u32` + 요청*.

use std::fmt;

use stellar_core_types::revision::DocumentRevision;

use crate::change::{ChangeRecorder, ChangeSet};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::{Command, CommandError};
use crate::guard::TransactionGuard;
use crate::id::{CommandId, CommandOrigin};
use crate::request::CommandRequest;
use crate::state::{rollback_with, DocumentState};

/// 원자 실행 단위.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandTransaction {
    /// 실행 단위 id.
    pub id: CommandId,
    /// 편집 출처.
    pub origin: CommandOrigin,
    /// 순서대로 실행할 요청.
    pub requests: Vec<CommandRequest>,
}

/// transaction 실행 오류: 실패한 요청 위치와 원인. 문서는 실행 전 상태입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionError {
    /// 실패한 요청 index(빈 transaction이면 0).
    pub index: usize,
    /// 원인.
    pub error: CommandError,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "transaction 요청 #{} 실패: {}", self.index, self.error)
    }
}

impl std::error::Error for TransactionError {}

/// 성공한 transaction의 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedTransaction {
    /// 모든 command의 변경을 합친 change set(새 revision 포함).
    pub changes: ChangeSet,
    /// 효과 전체를 되돌리는 transaction.
    pub inverse: CommandTransaction,
}

impl CommandTransaction {
    /// 빈 transaction.
    pub fn new(id: CommandId, origin: CommandOrigin) -> CommandTransaction {
        CommandTransaction {
            id,
            origin,
            requests: Vec::new(),
        }
    }

    /// 요청 하나로 된 transaction.
    pub fn single(
        id: CommandId,
        origin: CommandOrigin,
        request: CommandRequest,
    ) -> CommandTransaction {
        CommandTransaction {
            id,
            origin,
            requests: vec![request],
        }
    }

    /// 요청을 덧붙입니다(빌더).
    pub fn with_request(mut self, request: CommandRequest) -> CommandTransaction {
        self.requests.push(request);
        self
    }

    /// `base`를 기준 revision으로 하는 command를 덧붙입니다(precondition 없음).
    pub fn with_command(self, base: DocumentRevision, command: Command) -> CommandTransaction {
        self.with_request(CommandRequest::new(base, command))
    }

    /// 모든 요청의 base revision을 `revision`으로 바꾼 사본(history 재실행용).
    pub fn rebased(&self, revision: DocumentRevision) -> CommandTransaction {
        let mut out = self.clone();
        for request in &mut out.requests {
            request.base_revision = revision;
        }
        out
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Writer::new();
        self.encode_into(&mut out);
        out.into_bytes()
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.id.stable_id());
        let mut origin = Vec::new();
        self.origin.encode_into(&mut origin);
        out.raw(&origin);
        out.len(self.requests.len());
        for request in &self.requests {
            request.encode_into(out);
        }
    }

    /// [`CommandTransaction::encode`]의 역.
    pub fn decode(bytes: &[u8]) -> Result<CommandTransaction, CommandCodecError> {
        let mut input = Reader::new(bytes);
        let transaction = CommandTransaction::decode_from(&mut input)?;
        input.expect_end()?;
        Ok(transaction)
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<CommandTransaction, CommandCodecError> {
        let id = CommandId::from_stable_id(input.id()?);
        let (origin, used) = CommandOrigin::decode_prefix(input.rest()).map_err(|_| {
            CommandCodecError::Malformed {
                reason: "잘못된 command origin",
            }
        })?;
        input.take(used)?;
        let count = input.len()?;
        let mut requests = Vec::with_capacity(count);
        for _ in 0..count {
            requests.push(CommandRequest::decode_from(input)?);
        }
        Ok(CommandTransaction {
            id,
            origin,
            requests,
        })
    }
}

impl DocumentState {
    /// transaction을 원자적으로 실행합니다.
    ///
    /// 성공하면 revision을 한 번 올리고 합친 change set과 역 transaction을 반환합니다. 어느
    /// 요청에서든 실패하면 문서를 실행 전 의미 상태로 되돌리고 revision을 유지합니다.
    pub fn execute_transaction(
        &mut self,
        transaction: &CommandTransaction,
    ) -> Result<AppliedTransaction, TransactionError> {
        let (changes, inverses) = self.run_requests(&transaction.requests)?;
        let new_revision = changes.new_revision();
        let requests = inverses
            .into_iter()
            .rev()
            .map(|command| CommandRequest::new(new_revision, command))
            .collect();
        Ok(AppliedTransaction {
            changes,
            inverse: CommandTransaction {
                id: transaction.id,
                origin: transaction.origin,
                requests,
            },
        })
    }

    /// 요청들을 한 guard 안에서 실행합니다. 성공 시 (change set, 실행 순서의 역 command).
    pub(crate) fn run_requests(
        &mut self,
        requests: &[CommandRequest],
    ) -> Result<(ChangeSet, Vec<Command>), TransactionError> {
        if requests.is_empty() {
            return Err(TransactionError {
                index: 0,
                error: CommandError::EmptyTransaction,
            });
        }
        let start = self.revision;
        let new_revision = self
            .next_revision()
            .map_err(|error| TransactionError { index: 0, error })?;
        let mut guard = TransactionGuard::begin(&mut self.model);
        let mut inverses = Vec::with_capacity(requests.len());
        for (index, request) in requests.iter().enumerate() {
            let step = (|| {
                request.check(guard.ctx().model(), start)?;
                request.command.validate(guard.ctx().model())?;
                let inverse = request.command.invert(guard.ctx().model())?;
                request.command.apply(guard.ctx())?;
                Ok(inverse)
            })();
            match step {
                Ok(inverse) => inverses.push(inverse),
                Err(error) => {
                    return Err(TransactionError {
                        index,
                        error: rollback_with(guard, error),
                    })
                }
            }
        }
        let recorder: ChangeRecorder = guard.commit();
        self.revision = new_revision;
        Ok((recorder.finish(new_revision), inverses))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{sid, small_model};
    use crate::command::TestProbe;
    use crate::id::AgentProvenance;
    use stellar_document::model::DocumentModel;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(model: &DocumentModel) -> String {
        model_snapshot_json(model).expect("snapshot")
    }

    fn probe(node: u64, name: &str, fail: bool) -> Command {
        Command::TestProbe(TestProbe {
            node: sid(node),
            name: name.into(),
            fail_after_apply: fail,
        })
    }

    fn tx(base: DocumentRevision, commands: Vec<Command>) -> CommandTransaction {
        commands.into_iter().fold(
            CommandTransaction::new(CommandId::from_stable_id(sid(900)), CommandOrigin::User),
            |tx, command| tx.with_command(base, command),
        )
    }

    #[test]
    fn second_command_failure_leaves_no_effect_of_first() {
        // 검증 기준(VII.1.6): 두 번째 command 실패 시 첫 번째 효과 없음.
        let cases = [
            // 검증 실패(대상 없음).
            probe(99, "x", false),
            // 적용 도중 실패(변경 후 오류).
            probe(12, "y", true),
        ];
        for second in cases {
            let mut state = DocumentState::with_revision(small_model(), DocumentRevision(2));
            let before = snapshot(state.model());
            let transaction = tx(
                DocumentRevision(2),
                vec![probe(11, "첫 번째", false), second],
            );
            let error = state.execute_transaction(&transaction).unwrap_err();
            assert_eq!(error.index, 1);
            assert_eq!(snapshot(state.model()), before);
            assert_eq!(state.model().record(sid(11)).unwrap().display_name, "n11");
            assert_eq!(state.revision(), DocumentRevision(2));
        }
    }

    #[test]
    fn stale_base_in_later_request_rolls_back() {
        let mut state = DocumentState::with_revision(small_model(), DocumentRevision(2));
        let before = snapshot(state.model());
        let transaction = tx(DocumentRevision(2), vec![probe(11, "a", false)])
            .with_command(DocumentRevision(1), probe(12, "b", false));
        let error = state.execute_transaction(&transaction).unwrap_err();
        assert_eq!(error.index, 1);
        assert!(matches!(error.error, CommandError::RevisionMismatch { .. }));
        assert_eq!(snapshot(state.model()), before);
        let empty = tx(DocumentRevision(2), Vec::new());
        assert_eq!(
            state.execute_transaction(&empty).unwrap_err().error,
            CommandError::EmptyTransaction
        );
    }

    #[test]
    fn successful_transaction_bumps_revision_once_and_inverts() {
        let mut state = DocumentState::with_revision(small_model(), DocumentRevision(2));
        let before = snapshot(state.model());
        let transaction = tx(
            DocumentRevision(2),
            vec![
                probe(11, "a", false),
                probe(12, "b", false),
                probe(11, "c", false),
            ],
        );
        let applied = state.execute_transaction(&transaction).unwrap();
        assert_eq!(state.revision(), DocumentRevision(3));
        assert_eq!(applied.changes.new_revision(), DocumentRevision(3));
        assert_eq!(applied.changes.changed_nodes(), vec![sid(11), sid(12)]);
        assert_eq!(state.model().record(sid(11)).unwrap().display_name, "c");
        // 역 transaction: 역순, base = 새 revision, 같은 id·출처.
        assert_eq!(applied.inverse.id, transaction.id);
        assert_eq!(applied.inverse.requests.len(), 3);
        assert!(applied
            .inverse
            .requests
            .iter()
            .all(|r| r.base_revision == DocumentRevision(3)));
        assert_eq!(applied.inverse.requests[0].command, probe(11, "a", false));
        assert_eq!(applied.inverse.requests[2].command, probe(11, "n11", false));
        state.execute_transaction(&applied.inverse).unwrap();
        assert_eq!(snapshot(state.model()), before);
        assert_eq!(state.revision(), DocumentRevision(4));
    }

    #[test]
    fn transaction_round_trips_and_rebases() {
        let transaction = CommandTransaction::new(
            CommandId::from_stable_id(sid(901)),
            CommandOrigin::FutureAgent(AgentProvenance::new(sid(77))),
        )
        .with_command(DocumentRevision(5), probe(11, "a", false))
        .with_command(DocumentRevision(5), probe(12, "b", true));
        let bytes = transaction.encode();
        assert_eq!(CommandTransaction::decode(&bytes).unwrap(), transaction);
        for origin in [
            CommandOrigin::User,
            CommandOrigin::System,
            CommandOrigin::Migration,
        ] {
            let mut t = transaction.clone();
            t.origin = origin;
            assert_eq!(CommandTransaction::decode(&t.encode()).unwrap(), t);
        }
        let mut bad = bytes.clone();
        bad[16] = 9;
        assert!(CommandTransaction::decode(&bad).is_err());
        let rebased = transaction.rebased(DocumentRevision(42));
        assert!(rebased
            .requests
            .iter()
            .all(|r| r.base_revision == DocumentRevision(42)));
        assert_eq!(rebased.requests[1].command, transaction.requests[1].command);
    }
}
