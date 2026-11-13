//! batching 테스트 (XI.4.5).

use super::*;
use crate::pass_compiler::tests::{id_of, input, random_scene, sim, viewport_with, Rng};
use crate::pass_compiler::{compile_frame, EffectBoundaries};
use crate::test_scene::{opaque, rect, rect_clip, snapshot, ClipFx, ContentFx, NodeFx, IDENTITY};

const SURFACE: TargetInfo = TargetInfo {
    format: TextureFormat::Bgra8Unorm,
    samples: SampleCount::X4,
};

fn translucent(id: u8, x: f32) -> NodeFx {
    rect(id, [x, 0.0, 2.0, 2.0], [0.0, 0.0, 1.0, 0.5])
}

fn rounded(radius: f32) -> ClipFx {
    ClipFx {
        radii: [radius; 4],
        ..rect_clip([0.0, 0.0, 10.0, 10.0])
    }
}

fn image(id: u8, asset: u8) -> NodeFx {
    let mut node =
        rect(id, [0.0, 4.0, 2.0, 2.0], opaque(0.0, 0.0, 0.0)).with_content(ContentFx::Image {
            asset,
            bounds: [0.0, 4.0, 2.0, 2.0],
        });
    node.paint.fills.clear();
    node
}

fn text(id: u8) -> NodeFx {
    rect(id, [0.0, 6.0, 2.0, 2.0], opaque(0.0, 0.0, 0.0)).with_content(ContentFx::Text {
        key: id,
        bounds: [0.0, 6.0, 2.0, 2.0],
    })
}

/// pass별 batch를 node id 묶음으로.
fn grouped(plan: &FramePlan) -> Vec<(PassClass, Vec<Vec<u8>>)> {
    plan.passes_in_order()
        .map(|pass| {
            (
                pass.class,
                batch_pass(pass, SURFACE)
                    .iter()
                    .map(|batch| {
                        pass.draws[batch.draws.clone()]
                            .iter()
                            .map(|draw| id_of(draw.source.node()))
                            .collect()
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn batches_split_on_every_key_component_and_never_reorder() {
    let scene = snapshot(&[
        // 같은 key 연속 → 한 batch.
        translucent(1, 0.0),
        translucent(2, 1.0),
        translucent(3, 2.0),
        // clip(scissor) 변경 → 새 batch, 같은 scissor는 이어짐.
        translucent(4, 0.0).with_clip(rect_clip([0.0, 0.0, 8.0, 8.0])),
        translucent(5, 1.0).with_clip(rect_clip([0.0, 0.0, 8.0, 8.0])),
        // pipeline(glyph mask)·atlas 변경.
        text(6),
        text(7),
        // texture 변경: asset 1 → asset 2.
        image(8, 1),
        image(9, 2),
        image(10, 2),
        // Normal이 아닌 blend는 같은 key여도 묶지 않습니다.
        translucent(11, 0.0).with_blend(1),
        translucent(12, 1.0).with_blend(1),
        // stencil clip: 같은 clip 모양은 이어지고, scissor·reference가 같아도 모양이 다르면 끊깁니다.
        translucent(13, 0.0).with_clip(rounded(2.0)),
        translucent(14, 1.0).with_clip(rounded(2.0)),
        translucent(15, 2.0).with_clip(rounded(3.0)),
        // 앞 batch와 key가 같아도 떨어져 있으면 묶지 않습니다(z-order 보존).
        translucent(16, 3.0),
        // opaque pass.
        rect(17, [0.0, 10.0, 2.0, 2.0], opaque(1.0, 0.0, 0.0)),
        rect(18, [4.0, 10.0, 2.0, 2.0], opaque(0.0, 1.0, 0.0)),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    assert_eq!(
        grouped(&plan),
        vec![
            (PassClass::Opaque, vec![vec![18, 17]]),
            (
                PassClass::Transparent,
                vec![
                    vec![1, 2, 3],
                    vec![4, 5],
                    vec![6, 7],
                    vec![8],
                    vec![9, 10],
                    vec![11],
                    vec![12],
                    vec![13, 14],
                    vec![15],
                    vec![16],
                ]
            ),
        ]
    );
    // key 구성 요소 확인.
    let transparent = plan.passes_in_order().nth(1).unwrap();
    let keys: Vec<BatchKey> = batch_pass(transparent, SURFACE)
        .iter()
        .map(|batch| batch.key)
        .collect();
    assert_eq!(keys[0].pipeline.program, ShaderProgram::SolidFill);
    assert_eq!(keys[0].pipeline.blend, BlendState::PremultipliedOver);
    assert_eq!(keys[0].pipeline.depth, DepthMode::TestOnly);
    assert_eq!(keys[0].pipeline.color_format, TextureFormat::Bgra8Unorm);
    assert_eq!(keys[0].clip, ClipKey::None);
    assert!(matches!(keys[1].clip, ClipKey::Scissor(_)));
    assert_eq!(keys[2].pipeline.program, ShaderProgram::GlyphMask);
    assert_eq!(keys[2].texture, TextureKey::GlyphAtlas);
    assert_eq!(keys[3].pipeline.program, ShaderProgram::Texture);
    assert!(matches!(keys[3].texture, TextureKey::Resource(_)));
    assert_ne!(keys[3].texture, keys[4].texture);
    assert_eq!(keys[5].blend, SceneBlendMode::Multiply);
    assert_eq!(keys[5], keys[6]);
    assert_eq!(keys[7].pipeline.stencil, StencilMode::TestEqual);
    assert_eq!(keys[7], keys[8]);
    assert_eq!(keys[0], keys[9]);
    let opaque_pass = plan.passes_in_order().next().unwrap();
    let opaque_key = batch_pass(opaque_pass, SURFACE)[0].key;
    assert_eq!(opaque_key.pipeline.blend, BlendState::Replace);
    assert_eq!(opaque_key.pipeline.depth, DepthMode::TestAndWrite);
}

#[test]
fn layer_passes_use_layer_target_and_composites_use_layer_textures() {
    let scene = snapshot(&[
        crate::test_scene::group(
            1,
            [0.0, 0.0, 8.0, 8.0],
            vec![
                rect(2, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)),
                rect(3, [1.0, 1.0, 4.0, 4.0], opaque(0.0, 1.0, 0.0)),
            ],
        )
        .with_opacity(0.5),
        crate::test_scene::group(
            4,
            [0.0, 0.0, 8.0, 8.0],
            vec![
                rect(5, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)),
                rect(6, [1.0, 1.0, 4.0, 4.0], opaque(0.0, 1.0, 0.0)),
            ],
        )
        .with_opacity(0.5)
        .with_blend(2),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
    let batches = batch_frame(&plan, SURFACE);
    let effect = plan
        .passes_in_order()
        .find(|pass| pass.class == PassClass::Effect)
        .unwrap();
    let key = batch_pass(effect, SURFACE)[0].key;
    assert_eq!(key.pipeline.color_format, LAYER_FORMAT);
    assert_eq!(key.pipeline.samples, SampleCount::X1);
    assert_eq!(key.pipeline.depth, DepthMode::Disabled);
    // 합성 draw: 서로 다른 layer texture라 따로이고, screen 경계는 자기 blend를 key에 담습니다.
    let main = batches.last().unwrap();
    assert_eq!(main.batches.len(), 2);
    assert!(matches!(main.batches[0].key.texture, TextureKey::Layer(_)));
    assert_eq!(
        main.batches[0].key.pipeline.program,
        ShaderProgram::LayerComposite
    );
    assert_eq!(main.batches[1].key.blend, SceneBlendMode::Screen);
    // layer pass마다 같은 key 사각형 두 개가 batch 하나, main opaque pass는 비어 있음, 합성 2개.
    assert_eq!(batch_count(&batches), 1 + 1 + 2);
    assert_eq!(plan.draw_count(), 2 + 2 + 2);
}

#[test]
fn batching_preserves_render_order_on_random_scenes() {
    // 검증 기준(XI.4.5): batch 후 render order 의미 동일.
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let mut rng = Rng(0xba7c_4ed0_0000_0042);
    let mut draws = 0usize;
    let mut batches_total = 0usize;
    for _ in 0..400 {
        let scene = snapshot(&random_scene(&mut rng));
        let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Minimal)).unwrap();
        let frame = batch_frame(&plan, SURFACE);
        // batch를 펼친 순서로 plan을 다시 만들어 실행해도 painter 기준과 같습니다.
        let mut rebuilt = plan.clone();
        for pass_batches in &frame {
            let pass = rebuilt
                .passes
                .iter_mut()
                .find(|pass| pass.pass == pass_batches.pass)
                .unwrap();
            let original = pass.draws.clone();
            let mut next_index = 0;
            let mut flattened = Vec::new();
            for batch in &pass_batches.batches {
                // batch는 연속 구간이고 빈틈·겹침 없이 순서대로 이어집니다.
                assert_eq!(batch.draws.start, next_index);
                assert!(batch.draws.end > batch.draws.start);
                next_index = batch.draws.end;
                let class = pass.class;
                for draw in &original[batch.draws.clone()] {
                    assert_eq!(
                        batch_key(draw, class, target_for(class, SURFACE)),
                        batch.key
                    );
                    flattened.push(draw.clone());
                }
            }
            assert_eq!(next_index, original.len());
            let order = |draws: &[PlannedDraw]| -> Vec<(u64, u32)> {
                draws.iter().map(|draw| (draw.z.0, draw.depth)).collect()
            };
            assert_eq!(order(&flattened), order(&original));
            pass.draws = flattened;
            draws += original.len();
            batches_total += pass_batches.batches.len();
        }
        assert_eq!(sim::execute(&rebuilt), sim::reference(&scene));
    }
    assert!(batches_total < draws, "{batches_total} / {draws}");
}
