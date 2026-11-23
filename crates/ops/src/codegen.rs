//! 문서 값 → 코드 생성 IR 입력 (XXII.1.2, XXII.1.4, ADR-0106 결정 1).
//!
//! codegen은 문서 crate를 읽지 못하므로(crate 그래프) 이 module이 문서 registry를 codegen 지역 입력으로 옮깁니다.
//! 새 판정·formatter를 두지 않고 문서 registry와 layout record 이름을 그대로 씁니다.
//!
//! - CSS 기반 속성 사용: [`css_property_use`], [`layout_property_use`], [`all_css_subjects`] (XXII.1.2·XXII.1.4).
//! - `MotionSpec`: [`motion`] (XXII.1.5). 문서 값을 그대로 옮깁니다(f32 bit 보존).
//! - 상태 머신의 조건식·값 식(`CheckedExpression`)을 [`stellar_codegen::ir::IrExpr`]로 낮추는 일은 expressions
//!   타입을 이름으로 써야 하므로 ops에서 하지 않습니다(crate 그래프, ADR-0106 Consequences).

use stellar_codegen::ir::{
    CssPropertyUse, IrMotion, IrMotionStart, IrOwner, IrReducedMotion, IrStepPosition, IrTiming,
};
use stellar_document::css::mapping::MappingStatus;
use stellar_document::css::registry::{self, RegistryEntry};
use stellar_document::layout_record::{LayoutLonghand, LayoutRole};
use stellar_document::motion::spec::{
    MotionOwner, MotionSpec, ReducedMotion, StartCondition, StepPosition, TimingFunction,
};
use stellar_target_profile::css_matrix::MappingInput;

use crate::core_types::property::PropertyPath;

/// 문서 CSS 매핑 상태 → codegen 입력 상태.
pub fn mapping_input(status: MappingStatus) -> MappingInput {
    match status {
        MappingStatus::Exact => MappingInput::Exact,
        MappingStatus::Converted => MappingInput::Converted,
        MappingStatus::Approximated => MappingInput::Approximated,
        MappingStatus::StellarOnly => MappingInput::StellarOnly,
    }
}

/// registry 항목 하나의 사용(값 경로가 있으면 함께).
pub fn css_property_use(entry: &RegistryEntry, path: Option<PropertyPath>) -> CssPropertyUse {
    CssPropertyUse {
        scope: entry.scope.name(),
        name: entry.name,
        status: mapping_input(entry.css.status),
        path,
    }
}

/// layout longhand 하나의 사용. item longhand는 `node`, container longhand는 `frame` 범위입니다(IX.11.19 matrix와
/// 같은 대응). layout longhand는 CSS longhand 그대로이므로 매핑 상태는 `exact`입니다. registry 항목과 이름이 같으면
/// registry 상태를 씁니다.
pub fn layout_property_use(longhand: LayoutLonghand) -> CssPropertyUse {
    let scope = match longhand.role() {
        LayoutRole::Item => "node",
        LayoutRole::Container => "frame",
    };
    let status = registry::entries()
        .iter()
        .find(|e| e.scope.name() == scope && e.name == longhand.name())
        .map_or(MappingInput::Exact, |e| mapping_input(e.css.status));
    CssPropertyUse {
        scope,
        name: longhand.name(),
        status,
        path: None,
    }
}

/// registry 항목 전체와 layout longhand 전체(중복 없이, registry 순서 다음 longhand 순서).
pub fn all_css_subjects() -> Vec<CssPropertyUse> {
    let mut out: Vec<CssPropertyUse> = registry::entries()
        .iter()
        .map(|entry| css_property_use(entry, None))
        .collect();
    for longhand in LayoutLonghand::ALL {
        let item = layout_property_use(longhand);
        if !out
            .iter()
            .any(|u| u.scope == item.scope && u.name == item.name)
        {
            out.push(item);
        }
    }
    out
}

/// 문서 `MotionSpec` → IR motion. 값은 그대로 옮깁니다.
pub fn motion(spec: &MotionSpec) -> IrMotion {
    let owner = match spec.owner() {
        MotionOwner::Component(component) => IrOwner::Component(component.id()),
        MotionOwner::View(view) => IrOwner::View(view),
    };
    let start = match spec.start() {
        StartCondition::Becomes { variable, value } => IrMotionStart::Becomes {
            state: variable.stable_id(),
            value: value.clone(),
        },
        StartCondition::Changes { variable } => IrMotionStart::Changes {
            state: variable.stable_id(),
        },
    };
    let timing = match spec.timing() {
        TimingFunction::Linear => IrTiming::Linear,
        TimingFunction::CubicBezier { x1, y1, x2, y2 } => IrTiming::CubicBezier { x1, y1, x2, y2 },
        TimingFunction::Steps { count, position } => IrTiming::Steps {
            count,
            position: match position {
                StepPosition::JumpStart => IrStepPosition::JumpStart,
                StepPosition::JumpEnd => IrStepPosition::JumpEnd,
                StepPosition::JumpNone => IrStepPosition::JumpNone,
                StepPosition::JumpBoth => IrStepPosition::JumpBoth,
            },
        },
        TimingFunction::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        } => IrTiming::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        },
    };
    IrMotion {
        id: spec.id().stable_id(),
        owner,
        targets: spec.targets().to_vec(),
        start,
        duration_ms: spec.duration_ms(),
        delay_ms: spec.delay_ms(),
        timing,
        reduced_motion: match spec.reduced_motion() {
            ReducedMotion::Instant => IrReducedMotion::Instant,
            ReducedMotion::Play => IrReducedMotion::Play,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_codegen::matrix::css_entry;
    use stellar_target_profile::css_matrix::{web_capability, RegistryInput};
    use stellar_target_profile::profile::TargetProfileKind;

    fn input(css: &CssPropertyUse) -> RegistryInput<'static> {
        RegistryInput {
            scope: css.scope,
            name: css.name,
            status: css.status,
        }
    }

    #[test]
    fn every_registry_item_has_apple_and_android_status_and_rule() {
        // 검증 기준(XXII.1.4): Apple·Android profile의 모든 registry 항목에 상태·규칙이 있는지 완전성 검사.
        let subjects = all_css_subjects();
        assert!(subjects.len() >= 110, "{}", subjects.len());
        let mut missing = Vec::new();
        for css in &subjects {
            for kind in [TargetProfileKind::Apple, TargetProfileKind::Android] {
                let entry = css_entry(input(css), kind);
                if entry.status.is_none() {
                    missing.push(format!("{}/{} {}", css.scope, css.name, kind.name()));
                }
            }
        }
        assert!(missing.is_empty(), "규칙 없는 항목: {missing:?}");
    }

    #[test]
    fn web_status_equals_the_ix_11_19_matrix_for_every_registry_item() {
        // 검증 기준(XXII.1.4): Web 항목 상태가 IX.11.19 matrix와 같음.
        for css in all_css_subjects() {
            assert_eq!(
                css_entry(input(&css), TargetProfileKind::Web).status,
                Some(web_capability(input(&css)).status),
                "{}/{}",
                css.scope,
                css.name
            );
        }
    }

    #[test]
    fn motion_spec_values_are_copied_bit_for_bit() {
        use crate::core_types::ids::StableId;
        use crate::core_types::typed_value::TypedValue;
        use stellar_document::component::state_machine::StateVariableId;
        use stellar_document::motion::spec::{MotionSpecId, MotionSpecInput};

        let id =
            |n: u8| StableId::from_bytes([0, 0, 0, 0, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, n]);
        let target = PropertyPath::new(id(2)).push(crate::core_types::property::PropertyKey(4));
        let cases = [
            (
                TimingFunction::Spring {
                    mass: 1.25,
                    stiffness: 170.0,
                    damping: 26.0,
                    initial_velocity: 0.1,
                },
                ReducedMotion::Play,
                StartCondition::Changes {
                    variable: StateVariableId::new(id(3)),
                },
            ),
            (
                TimingFunction::Steps {
                    count: 4,
                    position: StepPosition::JumpBoth,
                },
                ReducedMotion::Instant,
                StartCondition::Becomes {
                    variable: StateVariableId::new(id(3)),
                    value: TypedValue::Boolean(true),
                },
            ),
        ];
        for (timing, reduced_motion, start) in cases {
            let spec = MotionSpec::new(MotionSpecInput {
                id: MotionSpecId::new(id(1)),
                owner: MotionOwner::View(id(9)),
                targets: vec![target.clone()],
                start,
                duration_ms: 333.3,
                delay_ms: 0.5,
                timing,
                reduced_motion,
            })
            .expect("유효한 MotionSpec");
            let ir = motion(&spec);
            assert_eq!(ir.id, id(1));
            assert_eq!(ir.owner, IrOwner::View(id(9)));
            assert_eq!(ir.targets, vec![target.clone()]);
            assert_eq!(ir.duration_ms.to_bits(), 333.3f32.to_bits());
            assert_eq!(ir.delay_ms.to_bits(), 0.5f32.to_bits());
            match (timing, ir.timing) {
                (
                    TimingFunction::Spring {
                        mass,
                        stiffness,
                        damping,
                        initial_velocity,
                    },
                    IrTiming::Spring {
                        mass: m,
                        stiffness: k,
                        damping: c,
                        initial_velocity: v,
                    },
                ) => {
                    assert_eq!(
                        [mass, stiffness, damping, initial_velocity].map(f32::to_bits),
                        [m, k, c, v].map(f32::to_bits)
                    );
                    assert_eq!(ir.reduced_motion, IrReducedMotion::Play);
                }
                (TimingFunction::Steps { count, .. }, IrTiming::Steps { count: c, position }) => {
                    assert_eq!(count, c);
                    assert_eq!(position, IrStepPosition::JumpBoth);
                    assert_eq!(ir.reduced_motion, IrReducedMotion::Instant);
                    assert!(matches!(ir.start, IrMotionStart::Becomes { .. }));
                }
                other => panic!("timing이 바뀌었습니다: {other:?}"),
            }
        }
    }

    #[test]
    fn rule_table_names_only_existing_registry_items() {
        let subjects = all_css_subjects();
        for row in stellar_codegen::matrix::css::CSS_LOWERINGS {
            assert!(
                subjects
                    .iter()
                    .any(|u| u.scope == row.scope && u.name == row.name),
                "{}/{}는 registry에 없습니다",
                row.scope,
                row.name
            );
        }
    }
}
