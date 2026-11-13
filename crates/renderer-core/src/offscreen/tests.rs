//! 최소 offscreen 경계 테스트 (XI.4.4).

use stellar_scene::stack::{Opacity, SceneBlendMode};

use super::*;
use crate::pass_compiler::tests::{id_of, input, random_scene, sim, viewport_with, Rng};
use crate::pass_compiler::{compile_frame, DrawSource, EffectBoundaries, PassClass};
use crate::test_scene::{group, nodes, opaque, rect, snapshot, ContentFx, IDENTITY};

/// pass 순서대로 (분류, [(node id, 합성이면 참, opacity, blend tag)]).
type PassSummary = Vec<(PassClass, Vec<(u8, bool, f32, u8)>)>;

fn summarize(plan: &crate::pass_compiler::FramePlan) -> PassSummary {
    plan.passes_in_order()
        .map(|pass| {
            (
                pass.class,
                pass.draws
                    .iter()
                    .map(|draw| {
                        (
                            id_of(draw.source.node()),
                            matches!(draw.source, DrawSource::Layer { .. }),
                            draw.opacity.get(),
                            draw.blend.tag(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

const MULTIPLY: u8 = 1;
const SCREEN: u8 = 2;

fn red() -> [f32; 4] {
    opaque(1.0, 0.0, 0.0)
}

#[test]
fn nested_opacity_fixture_expected_pass_graph() {
    // 검증 기준(XI.4.4): nested opacity fixture expected pass graph.
    let scene = snapshot(&[
        // 1(0.5) ⊃ 2(0.5) ⊃ 3: 모두 접혀 3을 0.25로 그림.
        group(
            1,
            [0.0, 0.0, 4.0, 4.0],
            vec![group(
                2,
                [0.0, 0.0, 4.0, 4.0],
                vec![rect(3, [0.0, 0.0, 2.0, 2.0], red())],
            )
            .with_opacity(0.5)],
        )
        .with_opacity(0.5),
        // 4(0.5) ⊃ 겹치는 5·6: 경계(layer).
        group(
            4,
            [0.0, 0.0, 8.0, 8.0],
            vec![
                rect(5, [4.0, 4.0, 3.0, 3.0], red()),
                rect(6, [5.0, 5.0, 3.0, 3.0], red()),
            ],
        )
        .with_opacity(0.5),
        // 7(0.5) ⊃ 겹치지 않는 8·9: 접혀 각각 0.5.
        group(
            7,
            [0.0, 0.0, 16.0, 16.0],
            vec![
                rect(8, [10.0, 0.0, 2.0, 2.0], red()),
                rect(9, [13.0, 0.0, 2.0, 2.0], red()),
            ],
        )
        .with_opacity(0.5),
        // 10(0.5) ⊃ 11(0.5) ⊃ 겹치는 12·13: 11은 경계, 10은 11 합성 하나라 접힘.
        group(
            10,
            [0.0, 0.0, 16.0, 16.0],
            vec![group(
                11,
                [0.0, 0.0, 16.0, 16.0],
                vec![
                    rect(12, [0.0, 10.0, 3.0, 3.0], red()),
                    rect(13, [1.0, 11.0, 3.0, 3.0], red()),
                ],
            )
            .with_opacity(0.5)],
        )
        .with_opacity(0.5),
        // 14: blur leaf는 항상 경계.
        rect(14, [10.0, 10.0, 2.0, 2.0], red()).with_blur(1.0),
        // 15(0.5) ⊃ 16(multiply leaf): 16은 접혀 multiply로 그림 → 15는 NestedBlend 경계.
        group(
            15,
            [0.0, 0.0, 16.0, 16.0],
            vec![rect(16, [6.0, 12.0, 2.0, 2.0], red()).with_blend(MULTIPLY)],
        )
        .with_opacity(0.5),
        // 17(screen) ⊃ 겹치지 않는 18·19: 접혀 각각 screen.
        group(
            17,
            [0.0, 0.0, 16.0, 16.0],
            vec![
                rect(18, [12.0, 4.0, 1.0, 1.0], red()),
                rect(19, [14.0, 4.0, 1.0, 1.0], red()),
            ],
        )
        .with_blend(SCREEN),
        // 20(0.5) fill + stroke leaf: 여러 번 칠하므로 경계.
        rect(20, [12.0, 7.0, 2.0, 2.0], red())
            .with_stroke(opaque(0.0, 0.0, 0.0), 1.0)
            .with_opacity(0.5),
        // 21(0.5) text leaf: glyph 겹침 가능성 때문에 경계.
        rect(21, [0.0, 14.0, 2.0, 2.0], red())
            .with_content(ContentFx::Text {
                key: 21,
                bounds: [0.0, 14.0, 2.0, 2.0],
            })
            .with_opacity(0.5),
        // 22(0.5): 그릴 것이 없는 group.
        group(22, [0.0, 0.0, 1.0, 1.0], Vec::new()).with_opacity(0.5),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);

    let all = nodes(&scene);
    let z_of = |id: u8| all.iter().find(|node| id_of(node) == id).unwrap().z();
    let analysis = plan_boundaries(&scene, &viewport, |_| true);
    let expected = [
        (1, EffectDecision::Folded),
        (2, EffectDecision::Folded),
        (
            4,
            EffectDecision::Boundary(BoundaryReason::OverlappingDraws),
        ),
        (7, EffectDecision::Folded),
        (10, EffectDecision::Folded),
        (
            11,
            EffectDecision::Boundary(BoundaryReason::OverlappingDraws),
        ),
        (14, EffectDecision::Boundary(BoundaryReason::Blur)),
        (15, EffectDecision::Boundary(BoundaryReason::NestedBlend)),
        (16, EffectDecision::Folded),
        (17, EffectDecision::Folded),
        (20, EffectDecision::Boundary(BoundaryReason::CompoundDraw)),
        (21, EffectDecision::Boundary(BoundaryReason::CompoundDraw)),
        (22, EffectDecision::NothingToDraw),
    ];
    for (id, decision) in expected {
        assert_eq!(analysis.decision(z_of(id)), Some(decision), "node {id}");
    }
    assert_eq!(analysis.decision(z_of(3)), None);
    assert_eq!(analysis.boundary_count(), 6);

    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    assert_eq!(
        summarize(&plan),
        vec![
            // layer는 pre-order 역순(안쪽·뒤쪽 먼저)으로 실행합니다.
            (PassClass::Effect, vec![(21, false, 1.0, 0)]),
            (PassClass::Effect, vec![(20, false, 1.0, 0)]),
            // 15 layer 안: 16이 multiply로 접힘.
            (PassClass::Effect, vec![(16, false, 1.0, MULTIPLY)]),
            (PassClass::Effect, vec![(14, false, 1.0, 0)]),
            (
                PassClass::Effect,
                vec![(12, false, 1.0, 0), (13, false, 1.0, 0)]
            ),
            (
                PassClass::Effect,
                vec![(5, false, 1.0, 0), (6, false, 1.0, 0)]
            ),
            // 접힌 효과가 걸린 draw는 opaque pass에 들어가지 않습니다.
            (PassClass::Opaque, vec![]),
            (
                PassClass::Transparent,
                vec![
                    (3, false, 0.25, 0),
                    (4, true, 1.0, 0),
                    (8, false, 0.5, 0),
                    (9, false, 0.5, 0),
                    // 11 합성에 10의 0.5가 접힘(11 자기 0.5는 합성에서 적용).
                    (11, true, 0.5, 0),
                    (14, true, 1.0, 0),
                    (15, true, 1.0, 0),
                    (18, false, 1.0, SCREEN),
                    (19, false, 1.0, SCREEN),
                    (20, true, 1.0, 0),
                    (21, true, 1.0, 0),
                ]
            ),
        ]
    );
    assert_eq!(plan.layers.len(), 6);
    // 같은 scene을 보수적 경계로 compile하면 효과 node마다 layer가 생깁니다(22는 그릴 것 없음).
    let conservative =
        compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();
    assert_eq!(conservative.layers.len(), 12);
    // 두 plan 모두 painter 기준과 같은 그림입니다.
    let reference = sim::reference(&scene);
    assert_eq!(sim::execute(&plan), reference);
    assert_eq!(sim::execute(&conservative), reference);
}

#[test]
fn minimal_boundaries_preserve_painter_order_on_random_scenes() {
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let mut rng = Rng(0x00ff_5c4e_e5b0_0dd1);
    let mut conservative_layers = 0usize;
    let mut minimal_layers = 0usize;
    let mut folded_draws = 0usize;
    for _ in 0..400 {
        let scene = snapshot(&random_scene(&mut rng));
        let minimal = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
        let conservative =
            compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();
        assert!(minimal.layers.len() <= conservative.layers.len());
        conservative_layers += conservative.layers.len();
        minimal_layers += minimal.layers.len();
        folded_draws += minimal
            .passes
            .iter()
            .flat_map(|pass| &pass.draws)
            .filter(|draw| draw.opacity < Opacity::OPAQUE || draw.blend != SceneBlendMode::Normal)
            .count();
        assert_eq!(sim::execute(&minimal), sim::reference(&scene));
    }
    // 접기가 실제로 layer를 줄입니다(무작위 scene에서 25% 이상, 실측 약 42%).
    assert!(
        minimal_layers * 4 < conservative_layers * 3,
        "{minimal_layers} / {conservative_layers}"
    );
    assert!(folded_draws > 100, "{folded_draws}");
}

#[test]
fn fractional_neighbours_overlap_at_pixel_level() {
    // 소수 위치로 같은 pixel을 나눠 덮는 두 사각형은 겹치는 것으로 봅니다(AA 가장자리 공유).
    let touching = |second_x: f32| {
        snapshot(&[group(
            1,
            [0.0, 0.0, 16.0, 16.0],
            vec![
                rect(2, [0.0, 0.0, 4.5, 2.0], red()),
                rect(3, [second_x, 0.0, 3.0, 2.0], red()),
            ],
        )
        .with_opacity(0.5)])
    };
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let shared_pixel = touching(4.6);
    let z = nodes(&shared_pixel)[0].z();
    assert_eq!(
        plan_boundaries(&shared_pixel, &viewport, |_| true).decision(z),
        Some(EffectDecision::Boundary(BoundaryReason::OverlappingDraws))
    );
    // 정수 경계에서 맞닿기만 하면 겹치지 않습니다.
    let mut aligned = touching(5.0);
    let _ = &mut aligned;
    let z = nodes(&aligned)[0].z();
    assert_eq!(
        plan_boundaries(&aligned, &viewport, |_| true).decision(z),
        Some(EffectDecision::Folded)
    );
    // zoom하면 pixel 판정도 device 기준으로 바뀝니다(4.5 × 2 = 9, 5 × 2 = 10 → 겹치지 않음).
    let zoomed = viewport_with(32.0, 32.0, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let z = nodes(&touching(5.0))[0].z();
    assert_eq!(
        plan_boundaries(&touching(5.0), &zoomed, |_| true).decision(z),
        Some(EffectDecision::Folded)
    );
}

#[test]
fn distribution_limit_and_visibility() {
    // 겹치지 않는 사각형이 상한보다 많으면 경계입니다.
    let many = |count: u8| {
        snapshot(&[group(
            1,
            [0.0, 0.0, 64.0, 64.0],
            (0..count)
                .map(|i| rect(2 + i, [f32::from(i) * 2.0, 0.0, 1.0, 1.0], red()))
                .collect(),
        )
        .with_opacity(0.5)])
    };
    let viewport = viewport_with(64.0, 64.0, IDENTITY);
    let at_limit = many(MAX_DISTRIBUTED_DRAWS as u8);
    let z = nodes(&at_limit)[0].z();
    assert_eq!(
        plan_boundaries(&at_limit, &viewport, |_| true).decision(z),
        Some(EffectDecision::Folded)
    );
    let over = many(MAX_DISTRIBUTED_DRAWS as u8 + 1);
    assert_eq!(
        plan_boundaries(&over, &viewport, |_| true).decision(z),
        Some(EffectDecision::Boundary(BoundaryReason::TooManyDraws))
    );
    // 그리지 않는 node는 unit이 아닙니다: 겹치는 둘 중 하나가 안 보이면 접힙니다.
    let overlapping = snapshot(&[group(
        1,
        [0.0, 0.0, 8.0, 8.0],
        vec![
            rect(2, [0.0, 0.0, 4.0, 4.0], red()),
            rect(3, [1.0, 1.0, 4.0, 4.0], red()),
        ],
    )
    .with_opacity(0.5)]);
    let z = nodes(&overlapping)[0].z();
    let z3 = nodes(&overlapping)[2].z();
    assert_eq!(
        plan_boundaries(&overlapping, &viewport, |_| true).decision(z),
        Some(EffectDecision::Boundary(BoundaryReason::OverlappingDraws))
    );
    assert_eq!(
        plan_boundaries(&overlapping, &viewport, |node| node.z() != z3).decision(z),
        Some(EffectDecision::Folded)
    );
    // 숨김 효과 node는 결정하지 않습니다(효과가 적용되지 않음).
    let hidden = snapshot(&[group(1, [0.0, 0.0, 4.0, 4.0], Vec::new())
        .with_opacity(0.5)
        .hidden()]);
    assert_eq!(
        plan_boundaries(&hidden, &viewport, |_| true).decision(nodes(&hidden)[0].z()),
        None
    );
    // image는 단일 합성, text는 아님.
    let image = snapshot(&[
        rect(1, [0.0, 0.0, 2.0, 2.0], red()).with_content(ContentFx::Image {
            asset: 1,
            bounds: [0.0, 0.0, 2.0, 2.0],
        }),
    ]);
    let image_node = &nodes(&image)[0];
    // fixture fill layer가 있으므로 image + layer 1개는 여러 번 칠함.
    assert!(!draws_as_single_composite(image_node));
}
