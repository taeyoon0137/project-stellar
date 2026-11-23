//! MotionSpec 생성·변경·삭제 command (XXX.1.6, ADR-0094 결정 4).
//!
//! [`MotionCommand`]는 document 계층 closed enum command입니다(Component Property command와 같은 방식). 적용 전에
//! 전부 검증하고(실패하면 모음 불변), 적용하면 정확히 되돌리는 inverse command를 돌려줍니다.
//!
//! - `Create`: 새 spec을 추가합니다. 대상이 있고 보간할 수 있는지, 시작 조건 변수가 있는지 검사합니다. inverse는 `Delete`.
//! - `Replace`: 같은 id의 spec을 바꿉니다(소유자는 바꿀 수 없음). 새 spec을 `Create`와 같이 검사합니다. inverse는 이전
//!   spec의 `Restore`.
//! - `Delete`: spec을 지웁니다. inverse는 `Restore`.
//! - `Restore`: inverse 전용입니다. 대상 검사 없이 spec을 되살리거나(없을 때) 이전 값으로 되돌립니다(있을 때). 대상이
//!   이미 사라진 spec도 그대로 되살아나 orphan 진단으로 남습니다.
//!
//! 대상 속성·node가 삭제되어도 이 command 계층은 MotionSpec을 지우지 않습니다. 남은 spec은 [`super::world::diagnose`]가
//! `motion-orphan-target`으로 보고합니다.

use std::fmt;

use super::set::{MotionSet, MotionSetError};
use super::spec::{MotionSpec, MotionSpecId};
use super::world::{check_targets, diagnose, MotionCheckError, MotionDiagnostic, MotionWorld};

/// MotionSpec command.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionCommand {
    /// 추가합니다.
    Create(MotionSpec),
    /// 같은 id의 spec을 바꿉니다.
    Replace(MotionSpec),
    /// 지웁니다.
    Delete(MotionSpecId),
    /// inverse 전용: 검사 없이 되살리거나 되돌립니다.
    Restore(MotionSpec),
}

/// command 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionCommandError {
    /// 모음 오류(id 중복·없음).
    Set(MotionSetError),
    /// 대상·조건 검사 실패.
    Check(MotionCheckError),
    /// `Replace`가 소유자를 바꾸려 했습니다.
    OwnerChanged(MotionSpecId),
}

impl fmt::Display for MotionCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MotionCommandError::Set(e) => e.fmt(f),
            MotionCommandError::Check(e) => e.fmt(f),
            MotionCommandError::OwnerChanged(id) => {
                write!(f, "MotionSpec 소유자는 바꿀 수 없습니다: {id}")
            }
        }
    }
}

impl std::error::Error for MotionCommandError {}

impl From<MotionSetError> for MotionCommandError {
    fn from(e: MotionSetError) -> MotionCommandError {
        MotionCommandError::Set(e)
    }
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionCommandOutcome {
    /// 정확히 되돌리는 command.
    pub inverse: MotionCommand,
    /// 적용 뒤 바뀐 spec의 진단(삭제면 빈 목록).
    pub diagnostics: Vec<MotionDiagnostic>,
}

impl MotionCommand {
    /// 대상 spec id.
    pub fn id(&self) -> MotionSpecId {
        match self {
            MotionCommand::Create(spec)
            | MotionCommand::Replace(spec)
            | MotionCommand::Restore(spec) => spec.id(),
            MotionCommand::Delete(id) => *id,
        }
    }

    /// 모음을 바꾸지 않고 검증합니다.
    pub fn validate(
        &self,
        set: &MotionSet,
        world: &dyn MotionWorld,
    ) -> Result<(), MotionCommandError> {
        match self {
            MotionCommand::Create(spec) => {
                if set.get(spec.id()).is_some() {
                    return Err(MotionSetError::DuplicateId(spec.id()).into());
                }
                check_targets(spec, world).map_err(MotionCommandError::Check)
            }
            MotionCommand::Replace(spec) => {
                let current = set
                    .get(spec.id())
                    .ok_or(MotionSetError::UnknownId(spec.id()))?;
                if current.owner() != spec.owner() {
                    return Err(MotionCommandError::OwnerChanged(spec.id()));
                }
                check_targets(spec, world).map_err(MotionCommandError::Check)
            }
            MotionCommand::Delete(id) => {
                set.get(*id).ok_or(MotionSetError::UnknownId(*id))?;
                Ok(())
            }
            MotionCommand::Restore(_) => Ok(()),
        }
    }

    /// 검증 후 적용하고 inverse를 돌려줍니다. 실패하면 모음은 바뀌지 않습니다.
    pub fn apply(
        &self,
        set: &mut MotionSet,
        world: &dyn MotionWorld,
    ) -> Result<MotionCommandOutcome, MotionCommandError> {
        self.validate(set, world)?;
        let (inverse, changed) = match self {
            MotionCommand::Create(spec) => {
                set.insert(spec.clone())?;
                (MotionCommand::Delete(spec.id()), Some(spec))
            }
            MotionCommand::Replace(spec) => {
                let previous = set.replace(spec.clone())?;
                (MotionCommand::Restore(previous), Some(spec))
            }
            MotionCommand::Delete(id) => {
                let removed = set.remove(*id)?;
                (MotionCommand::Restore(removed), None)
            }
            MotionCommand::Restore(spec) => {
                let inverse = match set.get(spec.id()) {
                    Some(_) => MotionCommand::Restore(set.replace(spec.clone())?),
                    None => {
                        set.insert(spec.clone())?;
                        MotionCommand::Delete(spec.id())
                    }
                };
                (inverse, Some(spec))
            }
        };
        Ok(MotionCommandOutcome {
            inverse,
            diagnostics: changed.map(|s| diagnose(s, world)).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::ComponentRef;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use crate::component::state_machine::StateVariableId;
    use crate::motion::spec::{
        MotionOwner, MotionSpecInput, ReducedMotion, StartCondition, TimingFunction,
    };
    use crate::motion::world::OrphanCause;
    use crate::node_kind::NodeKind;
    use crate::prototype::socket::TargetKind;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[derive(Default)]
    struct FakeWorld {
        nodes: BTreeMap<StableId, NodeKind>,
        variables: BTreeMap<StateVariableId, TypeSpec>,
    }

    impl MotionWorld for FakeWorld {
        fn target_kind(&self, _owner: MotionOwner, root: StableId) -> Option<TargetKind> {
            self.nodes.get(&root).map(|k| TargetKind::Node(*k))
        }

        fn variable_type(
            &self,
            _owner: MotionOwner,
            variable: StateVariableId,
        ) -> Option<TypeSpec> {
            self.variables.get(&variable).copied()
        }
    }

    fn world() -> FakeWorld {
        let mut w = FakeWorld::default();
        w.nodes.insert(sid(10), NodeKind::Frame);
        w.variables.insert(
            StateVariableId::new(sid(20)),
            TypeSpec::required(ExpressionType::Boolean),
        );
        w
    }

    fn spec(id: u64, key: u32, duration: f32) -> MotionSpec {
        MotionSpec::new(MotionSpecInput {
            id: crate::motion::spec::MotionSpecId::new(sid(id)),
            owner: MotionOwner::Component(ComponentRef::new(sid(1))),
            targets: vec![PropertyPath::new(sid(10)).push(PropertyKey(key))],
            start: StartCondition::Changes {
                variable: StateVariableId::new(sid(20)),
            },
            duration_ms: duration,
            delay_ms: 0.0,
            timing: TimingFunction::Linear,
            reduced_motion: ReducedMotion::Instant,
        })
        .expect("유효")
    }

    #[test]
    fn apply_and_inverse_round_trip() {
        // 검증 기준(XXX.1.6): apply/inverse 통과.
        let w = world();
        let mut set = MotionSet::new();
        let empty = set.encode();
        let created = MotionCommand::Create(spec(5, 0x0104, 200.0))
            .apply(&mut set, &w)
            .expect("생성");
        assert!(created.diagnostics.is_empty());
        let after_create = set.encode();
        let replaced = MotionCommand::Replace(spec(5, 0x0104, 300.0))
            .apply(&mut set, &w)
            .expect("변경");
        assert_eq!(
            set.get(spec(5, 0x0104, 0.0).id()).map(|s| s.duration_ms()),
            Some(300.0)
        );
        replaced.inverse.apply(&mut set, &w).expect("변경 되돌리기");
        assert_eq!(set.encode(), after_create);
        let deleted = MotionCommand::Delete(spec(5, 0x0104, 0.0).id())
            .apply(&mut set, &w)
            .expect("삭제");
        assert!(set.is_empty());
        deleted.inverse.apply(&mut set, &w).expect("삭제 되돌리기");
        assert_eq!(set.encode(), after_create);
        created.inverse.apply(&mut set, &w).expect("생성 되돌리기");
        assert_eq!(set.encode(), empty);
    }

    #[test]
    fn invalid_commands_leave_set_unchanged() {
        let w = world();
        let mut set = MotionSet::new();
        MotionCommand::Create(spec(5, 0x0104, 200.0))
            .apply(&mut set, &w)
            .expect("생성");
        let before = set.encode();
        // 보간할 수 없는 대상(visible: boolean), 없는 node, 중복 id, 없는 id.
        let bad = [
            MotionCommand::Create(spec(6, 0x0001, 200.0)),
            MotionCommand::Create(
                MotionSpec::new(MotionSpecInput {
                    targets: vec![PropertyPath::new(sid(99)).push(PropertyKey(0x0104))],
                    ..spec(7, 0x0104, 1.0).to_input()
                })
                .expect("유효"),
            ),
            MotionCommand::Create(spec(5, 0x0104, 1.0)),
            MotionCommand::Delete(spec(8, 0x0104, 1.0).id()),
        ];
        for command in bad {
            assert!(command.apply(&mut set, &w).is_err(), "{command:?}");
            assert_eq!(set.encode(), before);
        }
    }

    #[test]
    fn deleted_target_leaves_orphan_motion() {
        // 검증 기준(XXX.1.6): 대상 삭제 orphan fixture 정확.
        let mut w = world();
        let mut set = MotionSet::new();
        MotionCommand::Create(spec(5, 0x0104, 200.0))
            .apply(&mut set, &w)
            .expect("생성");
        w.nodes.remove(&sid(10));
        assert_eq!(set.len(), 1, "대상 삭제가 MotionSpec을 지우지 않습니다");
        let stored = set.iter().next().expect("spec");
        assert_eq!(
            diagnose(stored, &w),
            vec![MotionDiagnostic::OrphanTarget {
                motion: stored.id(),
                target: stored.targets()[0].clone(),
                cause: OrphanCause::MissingNode,
            }]
        );
        // orphan spec을 삭제하고 되돌려도 그대로 되살아납니다.
        let deleted = MotionCommand::Delete(stored.id())
            .apply(&mut set, &w)
            .expect("삭제");
        let restored = deleted.inverse.apply(&mut set, &w).expect("되돌리기");
        assert_eq!(set.len(), 1);
        assert_eq!(restored.diagnostics.len(), 1);
        assert_eq!(restored.diagnostics[0].code(), "motion-orphan-target");
    }
}
