//! 상태 머신·MotionSpec 입력의 참조 무결성 검사와 IR 정렬 (XXII.1.5).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;

use super::behavior::{
    IrAction, IrExpr, IrMotion, IrOwner, IrStateMachine, IrTrigger, MAX_EXPR_DEPTH,
};
use super::build::Scope;
use super::IrError;

fn owner_id(owner: IrOwner) -> StableId {
    match owner {
        IrOwner::Component(id) | IrOwner::View(id) => id,
    }
}

/// 표현식 하나를 검사합니다(깊이, 상태·토큰 선언).
pub(crate) fn check_expr(
    expr: &IrExpr,
    scope: &Scope<'_>,
    state_ok: &dyn Fn(StableId) -> bool,
    owner: StableId,
) -> Result<(), IrError> {
    if expr.depth() > MAX_EXPR_DEPTH {
        return Err(IrError::ExpressionTooDeep);
    }
    if let Some(state) = expr.states().into_iter().find(|s| !state_ok(*s)) {
        return Err(IrError::MachineUnknownState { owner, state });
    }
    if let Some(token) = expr.tokens().into_iter().find(|t| !scope.has_token(*t)) {
        return Err(IrError::ExpressionUnknownToken(token));
    }
    Ok(())
}

/// 상태 머신을 검사하고 소유자 오름차순으로 정렬합니다.
pub(crate) fn machines(
    input: &[IrStateMachine],
    scope: &Scope<'_>,
) -> Result<Vec<IrStateMachine>, IrError> {
    let mut owners = BTreeSet::new();
    let mut transitions = BTreeSet::new();
    let mut out = Vec::with_capacity(input.len());
    for machine in input {
        let owner = owner_id(machine.owner);
        if !owners.insert(machine.owner) {
            return Err(IrError::DuplicateMachine(owner));
        }
        let mut variables = machine.variables.clone();
        variables.sort();
        variables.dedup();
        if let Some(state) = variables.iter().find(|v| !scope.has_state(**v)) {
            return Err(IrError::MachineUnknownState {
                owner,
                state: *state,
            });
        }
        let mut actions = machine.actions.clone();
        actions.sort_by_key(|a| a.id);
        actions.dedup_by_key(|a| a.id);
        let declared_action = |id: StableId| actions.binary_search_by(|a| a.id.cmp(&id)).is_ok();
        // 머신 안 식은 그 머신 변수와 IR 상태 변수 선언만 읽습니다.
        let state_ok = |id: StableId| variables.binary_search(&id).is_ok() || scope.has_state(id);
        let mut kept = Vec::new();
        for transition in &machine.transitions {
            if !transitions.insert(transition.id) {
                return Err(IrError::DuplicateTransition(transition.id));
            }
            let source = match transition.trigger {
                IrTrigger::Event { source, .. } => source,
                IrTrigger::NestedAction { instance, .. } => instance,
            };
            let present = scope
                .node(source)
                .ok_or(IrError::TransitionOutsideView(source))?;
            if let Some(condition) = &transition.condition {
                check_expr(condition, scope, &state_ok, owner)?;
            }
            for action in &transition.actions {
                match action {
                    IrAction::SetState { variable, value } => {
                        if variables.binary_search(variable).is_err() {
                            return Err(IrError::MachineUnknownState {
                                owner,
                                state: *variable,
                            });
                        }
                        check_expr(value, scope, &state_ok, owner)?;
                    }
                    IrAction::Emit { action } => {
                        if !declared_action(*action) {
                            return Err(IrError::MachineUnknownAction {
                                transition: transition.id,
                                action: *action,
                            });
                        }
                    }
                }
            }
            // profile에서 제외된 source의 transition은 출력에 이벤트가 없으므로 버립니다.
            if present {
                kept.push(transition.clone());
            }
        }
        out.push(IrStateMachine {
            owner: machine.owner,
            variables,
            actions,
            transitions: kept,
        });
    }
    out.sort_by_key(|m| m.owner);
    Ok(out)
}

/// MotionSpec을 검사하고 id 오름차순으로 정렬합니다.
pub(crate) fn motions(input: &[IrMotion], scope: &Scope<'_>) -> Result<Vec<IrMotion>, IrError> {
    let mut ids = BTreeSet::new();
    let mut out = Vec::with_capacity(input.len());
    for motion in input {
        if !ids.insert(motion.id) {
            return Err(IrError::DuplicateMotion(motion.id));
        }
        let state = motion.start.state();
        if !scope.has_state(state) {
            return Err(IrError::MotionUnknownState {
                motion: motion.id,
                state,
            });
        }
        let mut targets = Vec::with_capacity(motion.targets.len());
        for target in &motion.targets {
            match scope.node(target.root()) {
                Some(true) => targets.push(target.clone()),
                Some(false) => {}
                None => return Err(IrError::MotionTargetOutsideView(target.root())),
            }
        }
        if targets.is_empty() {
            continue;
        }
        let mut motion = motion.clone();
        motion.targets = targets;
        out.push(motion);
    }
    out.sort_by_key(|m| m.id);
    Ok(out)
}
