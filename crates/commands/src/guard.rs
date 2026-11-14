//! transaction guard (VII.1.5).
//!
//! [`TransactionGuard`]는 [`EditContext`]를 소유하는 RAII guard입니다. 명시적으로
//! [`TransactionGuard::commit`]하지 않고 끝나면 — 오류로 인한 조기 반환이든 unwind든 —
//! journal을 역순으로 재생해 변경 전 의미 상태로 되돌립니다.
//!
//! 되돌리기는 **전체 문서 복제 없이** 바뀐 부분의 역 primitive만으로 수행합니다
//! ([`EditContext`]의 journal). 비용은 문서 크기가 아니라 실행한 primitive 수에 비례합니다.

use stellar_document::model::DocumentModel;

use crate::change::ChangeRecorder;
use crate::edit::{EditContext, RollbackError};

/// 모델 변경을 원자적으로 묶는 guard.
#[derive(Debug)]
pub struct TransactionGuard<'a> {
    ctx: Option<EditContext<'a>>,
}

impl<'a> TransactionGuard<'a> {
    /// 모델 위에서 guard를 엽니다.
    pub fn begin(model: &'a mut DocumentModel) -> TransactionGuard<'a> {
        TransactionGuard {
            ctx: Some(EditContext::new(model)),
        }
    }

    /// 변경 창구. guard가 끝난 뒤에는 호출할 수 없습니다(소비 메서드만 끝냅니다).
    pub fn ctx(&mut self) -> &mut EditContext<'a> {
        // ctx는 commit/rollback(self 소비)에서만 비워지므로 살아 있는 guard에서는 항상 있습니다.
        self.ctx
            .as_mut()
            .expect("TransactionGuard는 commit/rollback 전까지 context를 보유합니다")
    }

    /// 성공을 확정하고 변경 기록을 반환합니다.
    pub fn commit(mut self) -> ChangeRecorder {
        self.ctx.take().map(EditContext::commit).unwrap_or_default()
    }

    /// 변경 전 상태로 되돌립니다.
    pub fn rollback(mut self) -> Result<(), RollbackError> {
        match self.ctx.take() {
            Some(mut ctx) => ctx.rollback(),
            None => Ok(()),
        }
    }
}

impl Drop for TransactionGuard<'_> {
    fn drop(&mut self) {
        if let Some(mut ctx) = self.ctx.take() {
            // drop 경로에는 오류를 돌려줄 곳이 없습니다. 재생 실패는 내부 불일치 신호이므로
            // 테스트 빌드에서만 즉시 드러냅니다.
            let result = ctx.rollback();
            debug_assert!(result.is_ok(), "guard drop rollback 실패: {result:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::{Command, CommandError, TestProbe};
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::revision::DocumentRevision;
    use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};
    use stellar_document::records::NodePayload;
    use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(model: &DocumentModel) -> String {
        model_snapshot_json(model).expect("snapshot")
    }

    /// 모델의 node id(StableId 순).
    fn node_ids(model: &DocumentModel) -> Vec<StableId> {
        let mut ids: Vec<StableId> = model
            .columns()
            .iter_live()
            .filter_map(|h| model.stable_id(h))
            .collect();
        ids.sort();
        ids
    }

    /// 무작위 primitive 하나를 시도합니다. 모델이 거부한 primitive(예: 자식 있는 node 삭제)는
    /// 실패로 끝나도 journal에 남지 않습니다.
    fn random_primitive(ctx: &mut EditContext<'_>, rng: &mut SplitMix64, step: u64) {
        let ids = node_ids(ctx.model());
        if ids.is_empty() {
            return;
        }
        let target = *rng.pick(&ids);
        match rng.below(5) {
            0 => {
                let _ = ctx.remove_leaf(target);
            }
            1 => {
                let kind_ok = ctx
                    .model()
                    .payload(target)
                    .is_some_and(|p| matches!(p, NodePayload::Frame(_)));
                if kind_ok {
                    let mut record = frame(0, None, 1);
                    record.id = StableId::new_v7(4_000_000_000_000 + step, 1, step);
                    record.parent = Some(target);
                    record.order_key =
                        OrderKey::from_bytes(vec![0xFE, 0x01, step as u8 | 1]).expect("key");
                    let _ = ctx.insert_node(record);
                }
            }
            2 => {
                let _ = ctx.set_display_name(target, format!("renamed {step}"));
            }
            3 => {
                if let Some(payload) = ctx.model().payload(target).cloned() {
                    let _ = ctx.set_payload(target, payload);
                }
            }
            _ => {
                let mut stack = ModifierStack::new(target);
                let id = ModifierId::new(StableId::new_v7(5_000_000_000_000 + step, 2, step));
                if stack.push_back(id, ModifierKind::Opacity).is_ok() {
                    let _ = ctx.set_modifier_stack(stack);
                }
            }
        }
    }

    #[test]
    fn flag_edits_preserve_parent_and_child_handles_through_rollback_and_undo() {
        let mut model = small_model();
        let before = model.record(sid(11)).unwrap();
        let handles = [sid(11), sid(13)].map(|id| model.runtime_id(id).unwrap());
        let mut pending = before.clone();
        pending.visible = false;
        pending.locked = true;
        pending.display_name.push('!');
        let mut guard = TransactionGuard::begin(&mut model);
        guard.ctx().inject_fault_after(1);
        assert!(guard.ctx().replace_record(pending).is_err());
        let partial = guard.ctx().model().record(sid(11)).unwrap();
        assert!(!partial.visible && partial.locked);
        guard.rollback().unwrap();
        assert_eq!(model.record(sid(11)).unwrap(), before);

        let mut state = DocumentState::new(model);
        for command in [
            Command::set_visibility(sid(11), false),
            Command::set_locked(sid(11), true),
        ] {
            let request = CommandRequest::at(&state, command);
            let inverse = state.execute(&request).unwrap().inverse;
            assert_eq!(
                [sid(11), sid(13)].map(|id| state.model().runtime_id(id).unwrap()),
                handles
            );
            let undo = CommandRequest::at(&state, inverse);
            state.execute(&undo).unwrap();
        }
        assert_eq!(state.model().record(sid(11)).unwrap(), before);
        assert_eq!(
            [sid(11), sid(13)].map(|id| state.model().runtime_id(id).unwrap()),
            handles
        );
    }

    #[test]
    fn mid_apply_fault_rolls_back_to_identical_semantic_snapshot() {
        // 검증 기준(VII.1.5): 중간 실패 fault injection 후 semantic snapshot 동일.
        let mut mutated_seeds = 0;
        for seed in 0..60u64 {
            let mut model = random_valid_document(seed);
            let before = snapshot(&model);
            let mut rng = SplitMix64::new(seed ^ 0xFA17);
            let fault_at = 1 + rng.below(12) as usize;
            let mut guard = TransactionGuard::begin(&mut model);
            guard.ctx().inject_fault_after(fault_at);
            for step in 0..24u64 {
                random_primitive(guard.ctx(), &mut rng, step);
            }
            // fault 이후 primitive는 모두 실패하므로 journal은 최대 fault_at개입니다.
            let journal = guard.ctx().journal_len();
            assert!(
                journal <= fault_at,
                "seed {seed}: journal {journal} > {fault_at}"
            );
            if journal > 0 {
                mutated_seeds += 1;
            }
            guard.rollback().expect("rollback");
            assert_eq!(snapshot(&model), before, "seed {seed}");
        }
        assert!(
            mutated_seeds > 30,
            "변경이 실제로 일어난 seed: {mutated_seeds}"
        );
    }

    #[test]
    fn dropped_guard_rolls_back_and_commit_keeps_changes() {
        let mut model = small_model();
        let before = snapshot(&model);
        {
            let mut guard = TransactionGuard::begin(&mut model);
            guard.ctx().remove_leaf(sid(13)).unwrap();
            guard.ctx().remove_leaf(sid(11)).unwrap();
            guard
                .ctx()
                .set_display_name(sid(12), "임시".into())
                .unwrap();
            assert_eq!(guard.ctx().journal_len(), 3, "journal은 primitive 수만큼");
            // commit 없이 scope 종료 → drop rollback.
        }
        assert_eq!(snapshot(&model), before);

        let mut guard = TransactionGuard::begin(&mut model);
        guard.ctx().remove_leaf(sid(13)).unwrap();
        guard.commit();
        assert!(!model.contains(sid(13)));
    }

    #[test]
    fn failing_command_leaves_model_and_revision_unchanged() {
        let mut state = DocumentState::with_revision(small_model(), DocumentRevision(9));
        let before = snapshot(state.model());
        let request = CommandRequest::at(
            &state,
            Command::TestProbe(TestProbe {
                node: sid(11),
                name: "실패 전 변경".into(),
                fail_after_apply: true,
            }),
        );
        assert_eq!(
            state.execute(&request),
            Err(TestProbe::injected_fault(sid(11)))
        );
        assert_eq!(
            snapshot(state.model()),
            before,
            "probe의 이름 변경이 되돌려짐"
        );
        assert_eq!(state.revision(), DocumentRevision(9));
        assert!(matches!(
            state.execute(&CommandRequest::at(
                &state,
                Command::TestProbe(TestProbe {
                    node: sid(99),
                    name: String::new(),
                    fail_after_apply: false,
                }),
            )),
            Err(CommandError::UnknownNode { .. })
        ));
    }
}
