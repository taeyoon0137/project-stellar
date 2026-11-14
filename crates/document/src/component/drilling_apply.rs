//! 승인된 DrillingPlan 적용 (IX.4.5).
//!
//! [`apply_drilling_plan`]은 승인된 계획(IX.4.3·IX.4.4)을 **하나의 transaction**([`ComponentTransaction`])으로
//! 적용합니다. transaction은 컴포넌트 계층 closed enum command([`ComponentCommand`]: Property command(IX.1.3)와
//! drilling 모음 command([`DrillingCommand`]))의 목록이며 전부 성공하거나 전부 롤백합니다 — 중간 command가 실패하면
//! 이미 적용한 command의 inverse를 역순으로 적용해 library·drilling 모음을 적용 전 bytes로 되돌립니다. 새 property는
//! 하나도 남지 않습니다. 전체 상태를 복제하지 않습니다(inverse 기반 롤백, VII.1.5와 같은 정책).
//!
//! 적용 순서: 새 property를 모두 만든 뒤(바깥 → 안쪽) link를 추가합니다. link 추가는 IX.4.1 의미 검증을 통과해야
//! 합니다. 적용 직전에 같은 요청으로 계획을 다시 계산해 승인한 계획과 다르면(그 사이 문서가 바뀜) 아무것도 하지 않고
//! [`DrillingApplyError::StalePlan`]으로 거부합니다.
//!
//! 새 StableId(property·default 표현식 node·link)는 호출자가 주는 id 원천에서 발급합니다(앱은
//! `StableIdGenerator`, 테스트는 결정론적 원천). 결과의 `undo` transaction이 적용을 정확히 되돌립니다.
//!
//! `stellar-commands`의 `CommandTransaction`(VII.1.6)은 `DocumentModel`(scene) 대상이고 이 lane의 허용 경로가 아니라,
//! 같은 all-or-nothing 계약을 컴포넌트 계층에서 구현했습니다. command 계층 등록은 후속입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::{AstError, ExprNode, ExprNodeId, ExpressionAst};

use super::definition::{ComponentLibrary, ComponentPropertyId};
use super::drilling::{
    DrillingBinding, DrillingBindingId, DrillingError, DrillingOrphan, DrillingSet,
};
use super::drilling_plan::{DrillingPlan, DrillingPlanError, PlannedProperty};
use super::property::{ComponentProperty, ComponentPropertyError, PropertyVisibility};
use super::property_commands::{PropertyCommand, PropertyCommandError};

/// drilling 모음 command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillingCommand {
    /// binding을 추가합니다(IX.4.1 의미 검증 통과 필수).
    Add(DrillingBinding),
    /// binding을 지웁니다(orphan 정보 포함).
    Remove {
        /// binding.
        binding: DrillingBindingId,
    },
    /// `Remove`의 inverse 전용: 지운 binding과 orphan 정보를 검사 없이 되살립니다.
    Restore {
        /// binding.
        binding: DrillingBinding,
        /// 지울 때의 orphan 정보.
        orphan: Option<DrillingOrphan>,
    },
}

impl DrillingCommand {
    /// 검증 후 적용하고 inverse를 반환합니다. 실패 시 모음은 바뀌지 않습니다.
    pub fn apply(
        &self,
        library: &ComponentLibrary,
        drilling: &mut DrillingSet,
    ) -> Result<DrillingCommand, DrillingError> {
        match self {
            DrillingCommand::Add(binding) => {
                binding.validate(library, drilling)?;
                drilling.insert(binding.clone())?;
                Ok(DrillingCommand::Remove {
                    binding: binding.id(),
                })
            }
            DrillingCommand::Remove { binding } => {
                let orphan = drilling.orphan(*binding).cloned();
                let removed = drilling.remove(*binding)?;
                Ok(DrillingCommand::Restore {
                    binding: removed,
                    orphan,
                })
            }
            DrillingCommand::Restore { binding, orphan } => {
                drilling.insert(binding.clone())?;
                if let Some(orphan) = orphan {
                    drilling.mark_orphan(binding.id(), orphan.clone())?;
                }
                Ok(DrillingCommand::Remove {
                    binding: binding.id(),
                })
            }
        }
    }
}

/// 컴포넌트 계층 command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentCommand {
    /// Property command(IX.1.3).
    Property(PropertyCommand),
    /// drilling 모음 command.
    Drilling(DrillingCommand),
}

/// 컴포넌트 계층 command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentCommandError {
    /// Property command 오류.
    Property(PropertyCommandError),
    /// drilling command 오류.
    Drilling(DrillingError),
    /// 테스트 fault injection(테스트 전용 probe가 주입).
    Injected {
        /// 실패시킨 command index.
        index: usize,
    },
}

impl fmt::Display for ComponentCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentCommandError::Property(err) => write!(f, "{err}"),
            ComponentCommandError::Drilling(err) => write!(f, "{err}"),
            ComponentCommandError::Injected { index } => {
                write!(f, "command {index}에 주입된 실패")
            }
        }
    }
}

impl std::error::Error for ComponentCommandError {}

impl ComponentCommand {
    /// 검증 후 적용하고 inverse를 반환합니다. 실패 시 상태는 바뀌지 않습니다.
    pub fn apply(
        &self,
        library: &mut ComponentLibrary,
        drilling: &mut DrillingSet,
    ) -> Result<ComponentCommand, ComponentCommandError> {
        match self {
            ComponentCommand::Property(command) => command
                .apply(library, drilling)
                .map(|outcome| ComponentCommand::Property(outcome.inverse))
                .map_err(ComponentCommandError::Property),
            ComponentCommand::Drilling(command) => command
                .apply(library, drilling)
                .map(ComponentCommand::Drilling)
                .map_err(ComponentCommandError::Drilling),
        }
    }
}

/// transaction 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionError {
    /// 실패한 command index.
    pub index: usize,
    /// 원인.
    pub error: Box<ComponentCommandError>,
    /// 롤백 중 실패(정상이면 `None`). `Some`이면 상태가 적용 전과 다를 수 있습니다.
    pub rollback: Option<Box<ComponentCommandError>>,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "transaction command {} 실패: {}", self.index, self.error)?;
        if let Some(rollback) = &self.rollback {
            write!(f, " (롤백 실패: {rollback})")?;
        }
        Ok(())
    }
}

impl std::error::Error for TransactionError {}

/// 전부 성공하거나 전부 롤백하는 컴포넌트 command 묶음.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentTransaction {
    commands: Vec<ComponentCommand>,
}

impl ComponentTransaction {
    /// command 목록으로 만듭니다.
    pub fn new(commands: Vec<ComponentCommand>) -> ComponentTransaction {
        ComponentTransaction { commands }
    }

    /// command 목록.
    pub fn commands(&self) -> &[ComponentCommand] {
        &self.commands
    }

    /// 전부 적용하고 되돌리는 transaction(inverse 역순)을 반환합니다. 하나라도 실패하면 앞서 적용한 command를
    /// 역순 inverse로 되돌린 뒤 오류를 반환합니다.
    pub fn apply(
        &self,
        library: &mut ComponentLibrary,
        drilling: &mut DrillingSet,
    ) -> Result<ComponentTransaction, TransactionError> {
        self.apply_with_probe(library, drilling, &mut |_| Ok(()))
    }

    /// [`ComponentTransaction::apply`] 본체. `probe(index)`는 command `index` 적용 직후 호출되며 오류를 돌려주면 그
    /// command가 실패한 것으로 취급합니다(fault injection 테스트용, 프로덕션 호출자는 항상 `Ok`).
    pub(crate) fn apply_with_probe(
        &self,
        library: &mut ComponentLibrary,
        drilling: &mut DrillingSet,
        probe: &mut dyn FnMut(usize) -> Result<(), ComponentCommandError>,
    ) -> Result<ComponentTransaction, TransactionError> {
        let mut inverses: Vec<ComponentCommand> = Vec::with_capacity(self.commands.len());
        for (index, command) in self.commands.iter().enumerate() {
            let step = command.apply(library, drilling).and_then(|inverse| {
                inverses.push(inverse);
                probe(index)
            });
            if let Err(error) = step {
                let rollback = rollback(library, drilling, &mut inverses);
                return Err(TransactionError {
                    index,
                    error: Box::new(error),
                    rollback: rollback.map(Box::new),
                });
            }
        }
        inverses.reverse();
        Ok(ComponentTransaction { commands: inverses })
    }
}

fn rollback(
    library: &mut ComponentLibrary,
    drilling: &mut DrillingSet,
    inverses: &mut Vec<ComponentCommand>,
) -> Option<ComponentCommandError> {
    let mut first_error = None;
    while let Some(inverse) = inverses.pop() {
        if let Err(error) = inverse.apply(library, drilling) {
            first_error.get_or_insert(error);
        }
    }
    first_error
}

/// 계획 적용 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillingApplyError {
    /// 같은 요청으로 다시 계산한 계획이 승인한 계획과 다릅니다(문서가 바뀜).
    StalePlan,
    /// 다시 계산할 수 없습니다.
    Plan(DrillingPlanError),
    /// 새 property record를 만들 수 없습니다.
    Property(ComponentPropertyError),
    /// default 표현식 AST를 만들 수 없습니다.
    Ast(AstError),
    /// transaction 실패(롤백 완료 여부 포함).
    Transaction(TransactionError),
}

impl fmt::Display for DrillingApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrillingApplyError::StalePlan => {
                f.write_str("승인한 drilling 계획이 현재 문서와 맞지 않습니다")
            }
            DrillingApplyError::Plan(err) => write!(f, "{err}"),
            DrillingApplyError::Property(err) => write!(f, "{err}"),
            DrillingApplyError::Ast(err) => write!(f, "default 표현식 생성 실패: {err:?}"),
            DrillingApplyError::Transaction(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for DrillingApplyError {}

/// 계획 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedDrilling {
    /// 노출 후 top에서 쓰는 property.
    pub exposed: ComponentPropertyId,
    /// 만든 property(바깥 → 안쪽).
    pub created_properties: Vec<(ComponentRef, ComponentPropertyId)>,
    /// 만든 link(바깥 → 안쪽).
    pub created_links: Vec<DrillingBindingId>,
    /// 적용을 정확히 되돌리는 transaction.
    pub undo: ComponentTransaction,
}

/// 승인된 계획을 한 transaction으로 적용합니다. 실패하면 library·drilling 모음은 적용 전과 같습니다.
pub fn apply_drilling_plan(
    library: &mut ComponentLibrary,
    drilling: &mut DrillingSet,
    plan: &DrillingPlan,
    ids: &mut dyn FnMut() -> StableId,
) -> Result<AppliedDrilling, DrillingApplyError> {
    apply_drilling_plan_with_probe(library, drilling, plan, ids, &mut |_| Ok(()))
}

/// [`apply_drilling_plan`] 본체(transaction probe 주입 가능, fault injection 테스트용).
pub(crate) fn apply_drilling_plan_with_probe(
    library: &mut ComponentLibrary,
    drilling: &mut DrillingSet,
    plan: &DrillingPlan,
    ids: &mut dyn FnMut() -> StableId,
    probe: &mut dyn FnMut(usize) -> Result<(), ComponentCommandError>,
) -> Result<AppliedDrilling, DrillingApplyError> {
    let current = DrillingPlan::compute(library, drilling, plan.request().clone())
        .map_err(DrillingApplyError::Plan)?;
    if current != *plan {
        return Err(DrillingApplyError::StalePlan);
    }
    let mut commands = Vec::new();
    let mut created_properties = Vec::new();
    let mut new_ids = Vec::new();
    for property in plan.new_properties() {
        let id = ComponentPropertyId::new(ids());
        let node = ExprNodeId::new(ids());
        let default = match &property.default {
            Some(value) => ExprNode::literal(node, value.clone()),
            None => ExprNode::null(node, property.value_type.value_type()),
        };
        let ast = ExpressionAst::new(node, vec![default]).map_err(DrillingApplyError::Ast)?;
        let record = ComponentProperty::new(
            id,
            property.name.clone(),
            property.value_type,
            ast,
            PropertyVisibility::Public,
        )
        .map_err(DrillingApplyError::Property)?;
        commands.push(ComponentCommand::Property(PropertyCommand::Create {
            component: property.component,
            property: record,
            index: None,
        }));
        created_properties.push((property.component, id));
        new_ids.push((property.boundary, id));
    }
    let resolve = |planned: PlannedProperty| -> ComponentPropertyId {
        match planned {
            PlannedProperty::Existing(id) => id,
            PlannedProperty::New { boundary } => new_ids
                .iter()
                .find(|(b, _)| *b == boundary)
                .map(|(_, id)| *id)
                // 계획 불변식: 모든 New 참조는 new_properties에 있습니다. 없으면 존재하지 않는 id로 두어 link
                // 검증(UnknownProperty)이 transaction을 실패·롤백시킵니다.
                .unwrap_or(ComponentPropertyId::new(StableId::NIL)),
        }
    };
    let mut created_links = Vec::new();
    for link in plan.links() {
        let id = DrillingBindingId::new(ids());
        commands.push(ComponentCommand::Drilling(DrillingCommand::Add(
            DrillingBinding::link(
                id,
                link.owner,
                resolve(link.source),
                link.instance,
                resolve(link.target),
            ),
        )));
        created_links.push(id);
    }
    let undo = ComponentTransaction::new(commands)
        .apply_with_probe(library, drilling, probe)
        .map_err(DrillingApplyError::Transaction)?;
    Ok(AppliedDrilling {
        exposed: resolve(plan.exposed()),
        created_properties,
        created_links,
        undo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::drilling::fixtures::*;
    use crate::component::drilling_plan::DrillingRequest;
    use stellar_core_types::typed_value::TypedValue;

    fn request(depth: usize) -> DrillingRequest {
        DrillingRequest {
            top: component(0),
            path: (0..depth).map(instance).collect(),
            property: pid(LABEL),
        }
    }

    /// 결정론적 id 원천(60_000부터).
    fn id_source() -> impl FnMut() -> StableId {
        let mut next = 60_000u64;
        move || {
            next += 1;
            stable(next)
        }
    }

    fn snapshot(library: &ComponentLibrary, set: &DrillingSet) -> (Vec<u8>, Vec<u8>) {
        (library.encode(), set.encode())
    }

    #[test]
    fn approved_plan_applies_as_one_transaction_and_undoes_exactly() {
        for depth in [1, 2, 10] {
            let mut library = nested_library(depth);
            let mut set = DrillingSet::new();
            let before = snapshot(&library, &set);
            let plan = DrillingPlan::compute(&library, &set, request(depth)).unwrap();
            let applied =
                apply_drilling_plan(&mut library, &mut set, &plan, &mut id_source()).unwrap();
            assert_eq!(applied.created_properties.len(), depth);
            assert_eq!(applied.created_links.len(), depth);
            assert_eq!(applied.exposed, applied.created_properties[0].1);
            // 모든 link가 IX.4.1 검증을 통과하고, library도 저장 검증을 통과합니다.
            set.validate(&library).unwrap();
            ComponentLibrary::decode(&library.encode()).unwrap();
            let top = library.get(component(0)).unwrap();
            let exposed = top.property(applied.exposed).unwrap();
            assert_eq!(exposed.name(), "label");
            assert!(exposed.is_public());
            assert_eq!(
                exposed.default_value(),
                Some(&TypedValue::String("확인".into()))
            );
            // 이제 노출이 끝났으므로 같은 요청은 no-op 계획입니다.
            let again = DrillingPlan::compute(&library, &set, request(depth)).unwrap();
            assert!(again.is_noop(), "depth {depth}");
            // 승인 후 문서가 바뀐 계획(원래 계획)은 stale.
            assert_eq!(
                apply_drilling_plan(&mut library, &mut set, &plan, &mut id_source()).unwrap_err(),
                DrillingApplyError::StalePlan
            );
            // undo는 한 번에 정확히 되돌립니다.
            let redo = applied.undo.apply(&mut library, &mut set).unwrap();
            assert_eq!(snapshot(&library, &set), before, "depth {depth}");
            redo.apply(&mut library, &mut set).unwrap();
            set.validate(&library).unwrap();
        }
    }

    #[test]
    fn fault_injection_at_every_step_rolls_back_everything() {
        // 검증 기준(IX.4.5): fault injection rollback — 중간 실패 시 property 하나도 남기지 않습니다.
        let depth = 3;
        let library0 = nested_library(depth);
        let set0 = DrillingSet::new();
        let before = snapshot(&library0, &set0);
        let plan = DrillingPlan::compute(&library0, &set0, request(depth)).unwrap();
        let steps = plan.new_properties().len() + plan.links().len();
        assert_eq!(steps, 2 * depth);
        for fail_at in 0..steps {
            let mut library = library0.clone();
            let mut set = set0.clone();
            let err = apply_drilling_plan_with_probe(
                &mut library,
                &mut set,
                &plan,
                &mut id_source(),
                &mut |index| {
                    if index == fail_at {
                        Err(ComponentCommandError::Injected { index })
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(
                err,
                DrillingApplyError::Transaction(TransactionError {
                    index: fail_at,
                    error: Box::new(ComponentCommandError::Injected { index: fail_at }),
                    rollback: None,
                })
            );
            assert_eq!(snapshot(&library, &set), before, "fail at {fail_at}");
            for k in 0..=depth {
                assert_eq!(
                    library
                        .get(component(k))
                        .unwrap()
                        .definition()
                        .properties()
                        .len(),
                    if k == depth { 2 } else { 0 },
                    "fail at {fail_at}: 컴포넌트 {k}에 property가 남지 않음"
                );
            }
        }
    }

    #[test]
    fn real_mid_transaction_failure_also_rolls_back() {
        // id 원천이 두 번째 property id로 C1에 이미 있는 property id를 주면 두 번째 Create가 실패합니다.
        let mut library = nested_library(2);
        declare(&mut library, 1, string_property(60_003, "other"));
        let mut set = DrillingSet::new();
        let before = snapshot(&library, &set);
        let plan = DrillingPlan::compute(&library, &set, request(2)).unwrap();
        let err = apply_drilling_plan(&mut library, &mut set, &plan, &mut id_source()).unwrap_err();
        let DrillingApplyError::Transaction(err) = err else {
            panic!("transaction 실패여야 함: {err:?}");
        };
        assert_eq!(err.index, 1);
        assert!(matches!(
            *err.error,
            ComponentCommandError::Property(PropertyCommandError::DuplicateProperty { .. })
        ));
        assert_eq!(err.rollback, None);
        assert_eq!(snapshot(&library, &set), before);
    }

    #[test]
    fn drilling_commands_apply_and_invert() {
        let mut library = nested_library(1);
        declare(&mut library, 0, string_property(41_000, "label"));
        let mut set = DrillingSet::new();
        let link =
            DrillingBinding::link(bid(1), component(0), pid(41_000), instance(0), pid(LABEL));
        let undo_add = DrillingCommand::Add(link.clone())
            .apply(&library, &mut set)
            .unwrap();
        assert_eq!(undo_add, DrillingCommand::Remove { binding: bid(1) });
        // 검증 실패 binding은 추가되지 않습니다.
        assert!(DrillingCommand::Add(DrillingBinding::link(
            bid(2),
            component(0),
            pid(49_999),
            instance(0),
            pid(LABEL)
        ))
        .apply(&library, &mut set)
        .is_err());
        let with_link = set.encode();
        let undo_remove = undo_add.apply(&library, &mut set).unwrap();
        assert!(set.is_empty());
        undo_remove.apply(&library, &mut set).unwrap();
        assert_eq!(set.encode(), with_link);
    }
}
