//! 효과 pass 테스트 (ADR-0116 결정 3).

use super::*;
use crate::pass_compiler::tests::{id_of, input, viewport_with};
use crate::pass_compiler::{compile_frame, EffectBoundaries, FramePlan, PassClass};
use crate::render_graph::SampleCount;
use crate::test_scene::{group, opaque, rect, snapshot, ClipFx, ShadowFx, IDENTITY};

fn reads(plan: &FramePlan, pass: PassId) -> Vec<ResourceVersion> {
    plan.graph.pass(pass).unwrap().reads.clone()
}

fn transparent(plan: &FramePlan) -> &crate::pass_compiler::PassPlan {
    plan.passes
        .iter()
        .find(|pass| pass.class == PassClass::Transparent)
        .unwrap()
}

#[test]
fn layer_blur_adds_two_blur_passes_and_parent_reads_the_result() {
    let scene = snapshot(&[group(
        1,
        [0.0, 0.0, 8.0, 8.0],
        vec![rect(2, [1.0, 1.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))],
    )
    .with_blur(2.0)]);
    // 배율 2: device σ = 반경 × 2.
    let viewport = viewport_with(32.0, 32.0, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    assert_eq!(plan.layers.len(), 1);
    let effects = plan.layer_effects(LayerId(0)).unwrap();
    let blur = effects.blur.unwrap();
    assert_eq!((blur.sigma_x, blur.sigma_y, blur.exact), (4.0, 4.0, true));
    assert_eq!(plan.effect_passes.len(), 2);
    let [horizontal, vertical] = &plan.effect_passes[..] else {
        unreachable!()
    };
    match (&horizontal.step, &vertical.step) {
        (
            EffectStep::Blur {
                source,
                direction: [1.0, 0.0],
                alpha_to_all: false,
                shadow: None,
                ..
            },
            EffectStep::Blur {
                source: second,
                direction: [0.0, 1.0],
                ..
            },
        ) => {
            assert_eq!(*source, plan.layers[0].output);
            assert_eq!(*second, horizontal.output);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(effects.blurred, Some(vertical.output));
    // 부모 transparent pass는 blur 결과만 읽습니다(blur 전 layer 출력이 아님).
    let parent = transparent(&plan);
    assert_eq!(reads(&plan, parent.pass), vec![vertical.output]);
    let composite = plan.draw_effects(parent.pass, 1).unwrap();
    assert_eq!(composite.composite.program, ShaderProgram::LayerComposite);
    assert!(plan.effect_diagnostics.is_empty());
}

#[test]
fn blend_composites_resolve_program_and_state() {
    let scene = snapshot(&[
        rect(1, [0.0, 0.0, 8.0, 8.0], opaque(0.0, 0.0, 1.0)),
        // 겹치는 두 자식: 접지 못해 layer 경계.
        group(
            2,
            [0.0, 0.0, 8.0, 8.0],
            vec![
                rect(3, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)),
                rect(4, [2.0, 2.0, 4.0, 4.0], opaque(0.0, 1.0, 0.0)),
            ],
        )
        .with_blend(SceneBlendMode::Multiply.tag())
        .with_opacity(0.5),
        // 자식 하나: blend를 내용 draw에 접습니다.
        group(
            5,
            [0.0, 0.0, 8.0, 8.0],
            vec![rect(6, [5.0, 5.0, 2.0, 2.0], opaque(1.0, 1.0, 0.0))],
        )
        .with_blend(SceneBlendMode::Screen.tag()),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    let pass = transparent(&plan);
    let by_node = |id: u8| {
        let draw = pass
            .draws
            .iter()
            .find(|draw| id_of(draw.source.node()) == id)
            .unwrap();
        plan.draw_effects(pass.pass, draw.depth).unwrap()
    };
    let layer = by_node(2);
    assert_eq!(layer.composite.mode, SceneBlendMode::Multiply);
    assert_eq!(
        layer.composite.program,
        ShaderProgram::BlendComposite(SceneBlendMode::Multiply)
    );
    assert_eq!(layer.composite.state, BlendState::Replace);
    assert_eq!(layer.node_opacity.get(), 0.5);
    let folded = by_node(6);
    assert_eq!(folded.composite.mode, SceneBlendMode::Screen);
    assert_eq!(folded.node_opacity, Opacity::OPAQUE);
    assert!(plan.blend_diagnostics.is_empty());
    // 지원 목록 안 mode는 진단 없이 같은 mode로 풉니다.
    let mut diagnostics = Vec::new();
    for mode in SceneBlendMode::ALL {
        assert_eq!(Composite::resolve(mode, &mut diagnostics).mode, mode);
    }
    assert!(diagnostics.is_empty());
}

#[test]
fn shadows_get_source_and_blur_passes_in_device_units() {
    let scene = snapshot(&[rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 1.0, 1.0))
        .with_shadow(ShadowFx {
            offset: [1.0, 2.0],
            blur: 2.0,
            color: [0.0, 0.0, 0.0, 0.5],
        })
        .with_shadow(ShadowFx {
            offset: [-1.0, 0.0],
            blur: 0.0,
            color: [1.0, 0.0, 0.0, 1.0],
        })]);
    // 축별 배율 (2, 3): σx = σ·2, σy = σ·3, offset = (2·x, 3·y).
    let viewport = viewport_with(32.0, 32.0, [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    // 원본 1 + shadow마다 blur 2.
    assert_eq!(plan.effect_passes.len(), 5);
    assert!(matches!(
        plan.effect_passes[0].step,
        EffectStep::ShadowSource { .. }
    ));
    let pass = transparent(&plan);
    let effects = plan.draw_effects(pass.pass, 1).unwrap();
    assert_eq!(effects.shadows.len(), 2);
    let [first, second] = &effects.shadows[..] else {
        unreachable!()
    };
    assert_eq!((first.index, first.offset), (0, [2.0, 6.0]));
    // σ = blur / 2 = 1.
    assert_eq!((first.blur.sigma_x, first.blur.sigma_y), (2.0, 3.0));
    assert_eq!((second.index, second.offset), (1, [-2.0, 0.0]));
    assert_eq!((second.blur.sigma_x, second.blur.sigma_y), (0.0, 0.0));
    // shadow 1단계는 alpha를 퍼뜨리고 2단계가 shadow 색을 곱합니다.
    match (&plan.effect_passes[1].step, &plan.effect_passes[2].step) {
        (
            EffectStep::Blur {
                alpha_to_all: true,
                shadow: None,
                ..
            },
            EffectStep::Blur {
                alpha_to_all: false,
                shadow: Some((_, 0)),
                ..
            },
        ) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(
        reads(&plan, pass.pass),
        vec![first.output, second.output],
        "target pass는 shadow 결과를 읽습니다"
    );
}

#[test]
fn skewed_or_huge_blur_is_reported() {
    let skewed = snapshot(&[
        rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 1.0, 1.0)).with_shadow(ShadowFx {
            offset: [0.0, 0.0],
            blur: 2.0,
            color: [0.0, 0.0, 0.0, 1.0],
        }),
    ]);
    let viewport = viewport_with(32.0, 32.0, [1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
    let plan = compile_frame(&input(&skewed, &viewport, EffectBoundaries::Minimal)).unwrap();
    assert!(plan
        .effect_diagnostics
        .iter()
        .any(|d| d.code() == "render.effect.anisotropic_blur"));
    // 회전 + 균일 배율은 정확합니다.
    let rotated = viewport_with(32.0, 32.0, [0.0, 2.0, -2.0, 0.0, 16.0, 0.0]);
    let plan = compile_frame(&input(&skewed, &rotated, EffectBoundaries::Minimal)).unwrap();
    assert!(plan.effect_diagnostics.is_empty());
    let huge = snapshot(&[group(
        1,
        [0.0, 0.0, 8.0, 8.0],
        vec![rect(2, [1.0, 1.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))],
    )
    .with_blur(100.0)]);
    let identity = viewport_with(32.0, 32.0, IDENTITY);
    let plan = compile_frame(&input(&huge, &identity, EffectBoundaries::Minimal)).unwrap();
    let blur = plan.layer_effects(LayerId(0)).unwrap().blur.unwrap();
    assert_eq!(blur.sigma_x, MAX_SIGMA);
    assert!(plan
        .effect_diagnostics
        .iter()
        .any(|d| matches!(d, EffectDiagnostic::BlurTooLarge { sigma, .. } if *sigma == 100.0)));
}

#[test]
fn mask_clip_draws_get_a_mask_pass() {
    let rounded = ClipFx {
        radii: [2.0; 4],
        ..crate::test_scene::rect_clip([0.0, 0.0, 6.0, 6.0])
    };
    let scene =
        snapshot(&[rect(1, [0.0, 0.0, 8.0, 8.0], opaque(1.0, 0.0, 0.0)).with_clip(rounded)]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let mut no_msaa = input(&scene, &viewport, EffectBoundaries::Minimal);
    no_msaa.clip_caps.samples = SampleCount::X1;
    let plan = compile_frame(&no_msaa).unwrap();
    let pass = transparent(&plan);
    let draw = &pass.draws[0];
    assert!(matches!(draw.clip, ClipPlan::Mask { .. }));
    let effects = plan.draw_effects(pass.pass, draw.depth).unwrap();
    let mask = effects.mask.unwrap();
    let effect = plan
        .effect_passes
        .iter()
        .find(|p| p.output == mask)
        .unwrap();
    assert!(matches!(&effect.step, EffectStep::Mask { clips, .. } if clips.len() == 1));
    let desc = plan.graph.resource(mask.resource).unwrap().desc;
    assert_eq!(desc.format, MASK_FORMAT);
    assert_eq!(reads(&plan, pass.pass), vec![mask]);
}
