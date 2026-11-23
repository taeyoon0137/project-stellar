//! 상태 머신·MotionSpec 입력의 참조 무결성 검사와 IR 정렬 (XXII.1.5).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;

use super::behavior::{
    IrAction, IrExpr, IrMotion, IrOwner, IrStateMachine, IrTrigger, MAX_EXPR_DEPTH,
};
use super::build::Scope;
use super::context::{IrContextSource, IrContexts};
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
    state_error: &dyn Fn(StableId) -> IrError,
) -> Result<(), IrError> {
    if expr.depth() > MAX_EXPR_DEPTH {
        return Err(IrError::ExpressionTooDeep);
    }
    if let Some(state) = expr.states().into_iter().find(|s| !state_ok(*s)) {
        return Err(state_error(state));
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
        let state_error = |state: StableId| IrError::MachineUnknownState { owner, state };
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
                check_expr(condition, scope, &state_ok, &state_error)?;
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
                        check_expr(value, scope, &state_ok, &state_error)?;
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

/// Context 해석 결과를 검사하고 정렬합니다(XXII.1.6). 제외 node의 provision·소비·slot host는 버립니다.
pub(crate) fn contexts(input: &IrContexts, scope: &Scope<'_>) -> Result<IrContexts, IrError> {
    let mut keys = input.keys.clone();
    keys.sort_by_key(|k| k.id);
    if let Some(pair) = keys.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(IrError::DuplicateContextKey(pair[0].id));
    }
    let declared = |id: StableId| keys.binary_search_by(|k| k.id.cmp(&id)).is_ok();
    let state_ok = |id: StableId| scope.has_state(id);
    let state_error = IrError::ExpressionUnknownState;
    let check = |expr: &IrExpr| -> Result<(), IrError> {
        check_expr(expr, scope, &state_ok, &state_error)?;
        if let Some(key) = expr.contexts().into_iter().find(|k| !declared(*k)) {
            return Err(IrError::UnknownContextKey(key));
        }
        Ok(())
    };
    for key in &keys {
        if key.name.trim().is_empty() {
            return Err(IrError::EmptyContextKeyName(key.id));
        }
        check(&key.default)?;
    }
    let present = |id: StableId| scope.node(id).ok_or(IrError::ContextNodeOutsideView(id));

    let mut provisions = Vec::with_capacity(input.provisions.len());
    for provision in &input.provisions {
        if !declared(provision.key) {
            return Err(IrError::UnknownContextKey(provision.key));
        }
        check(&provision.value)?;
        let mut depths = provision.depths.clone();
        depths.sort_unstable();
        depths.dedup();
        if depths.is_empty() || depths[0] == 0 {
            return Err(IrError::InvalidProvisionDepth {
                node: provision.node,
                key: provision.key,
            });
        }
        if present(provision.node)? {
            let mut provision = provision.clone();
            provision.depths = depths;
            provisions.push(provision);
        }
    }
    provisions.sort_by_key(|p| (p.node, p.key));
    if let Some(pair) = provisions
        .windows(2)
        .find(|w| (w[0].node, w[0].key) == (w[1].node, w[1].key))
    {
        return Err(IrError::DuplicateProvision {
            node: pair[0].node,
            key: pair[0].key,
        });
    }
    let provided = |node: StableId, key: StableId| {
        provisions
            .binary_search_by(|p| (p.node, p.key).cmp(&(node, key)))
            .is_ok()
    };

    let mut consumptions = Vec::with_capacity(input.consumptions.len());
    for consumption in &input.consumptions {
        if !declared(consumption.key) {
            return Err(IrError::UnknownContextKey(consumption.key));
        }
        if consumption.path.root() != consumption.node {
            return Err(IrError::ContextNodeOutsideView(consumption.path.root()));
        }
        for crossing in &consumption.slots {
            present(crossing.host)?;
            present(crossing.instance)?;
        }
        if !present(consumption.node)? {
            continue;
        }
        if let IrContextSource::Provision(node) = consumption.source {
            if !provided(node, consumption.key) {
                return Err(IrError::ContextSourceMissing {
                    node: consumption.node,
                    key: consumption.key,
                });
            }
        }
        consumptions.push(consumption.clone());
    }
    consumptions.sort_by(|a, b| (a.node, &a.path, a.key).cmp(&(b.node, &b.path, b.key)));
    if let Some(pair) = consumptions
        .windows(2)
        .find(|w| (w[0].node, &w[0].path) == (w[1].node, &w[1].path))
    {
        return Err(IrError::DuplicateConsumption(pair[0].node));
    }

    let mut slots = Vec::with_capacity(input.slots.len());
    for slot in &input.slots {
        let host = present(slot.host)?;
        let instance = present(slot.instance)?;
        let mut slot = slot.clone();
        slot.public_keys.sort();
        slot.public_keys.dedup();
        if let Some(key) = slot.public_keys.iter().find(|k| !declared(**k)) {
            return Err(IrError::UnknownContextKey(*key));
        }
        if host && instance {
            slots.push(slot);
        }
    }
    slots.sort_by_key(|s| s.host);
    Ok(IrContexts {
        keys,
        provisions,
        consumptions,
        slots,
    })
}
