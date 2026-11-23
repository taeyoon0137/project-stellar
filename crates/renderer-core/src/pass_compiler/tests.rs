//! pass compiler 테스트 (XI.4.2).
//!
//! `sim`은 painter 의미를 검사하는 coarse raster 모사입니다. scene tree를 그대로 painter 순서로 칠한
//! 기준 결과와, compile된 plan을 pass 순서·depth test 규칙대로 실행한 결과를 cell 단위로 비교합니다.
//! XI.4.4(경계 최소화)와 XI.4.5(batching)도 같은 모사로 의미 보존을 확인합니다.

use std::collections::HashMap;

use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::visibility::visible_candidates;
use stellar_scene::Size2;

use super::*;
use crate::test_scene::{
    group, nodes, opaque, rect, snapshot, ContentFx, NodeFx, ShadowFx, IDENTITY,
};

/// `view` 변환과 크기로 viewport를 만듭니다(Transform2D는 fixture node에서 얻음).
pub(crate) fn viewport_with(width: f32, height: f32, view: [f32; 6]) -> Viewport {
    let carrier =
        snapshot(&[rect(250, [0.0, 0.0, 1.0, 1.0], opaque(0.0, 0.0, 0.0)).with_transform(view)]);
    let transform = nodes(&carrier)[0].world_transform();
    Viewport::new(Size2::new(width, height).unwrap(), transform).unwrap()
}

pub(crate) fn input<'a>(
    snapshot: &'a SceneSnapshot,
    viewport: &'a Viewport,
    boundaries: EffectBoundaries,
) -> CompileInput<'a> {
    CompileInput {
        snapshot,
        viewport,
        visible: None,
        surface_format: TextureFormat::Bgra8Unorm,
        clear: ClearColor::TRANSPARENT,
        boundaries,
        clip_caps: MSAA_CLIPS,
    }
}

/// 4x MSAA + 8-bit stencil.
pub(crate) const MSAA_CLIPS: ClipCapabilities = ClipCapabilities {
    stencil_bits: 8,
    samples: crate::render_graph::SampleCount::X4,
};

pub(crate) fn id_of(node: &SceneNode) -> u8 {
    node.source().to_bytes()[0]
}

/// coarse raster 모사.
pub(crate) mod sim {
    use super::*;

    /// cell 값: (내용 hash, 누적 합성 불투명도). 같은 값이면 같은 그림입니다.
    pub(crate) type Cell = Option<(u64, u32)>;

    pub(crate) const W: usize = 16;
    pub(crate) const H: usize = 16;

    fn alpha_bits(alpha: f32) -> u32 {
        alpha.to_bits()
    }

    fn hash(parts: &[u64]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for part in parts {
            for byte in part.to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    /// 합성. blur는 원본 내용을 바꾼 뒤(opacity·blend보다 먼저) 적용됩니다. 빈 backdrop 위에서는
    /// blend와 무관하게 불투명도만 곱하고(빈 backdrop 위 blend 결과는 원본), 그 외는 순서 의존
    /// hash입니다.
    pub(crate) fn composite(
        dst: Cell,
        src: (u64, u32),
        opacity: f32,
        blend: u8,
        blur: f32,
    ) -> Cell {
        let content = if blur > 0.0 {
            hash(&[src.0, u64::from(blur.to_bits())])
        } else {
            src.0
        };
        let alpha = f32::from_bits(src.1) * opacity;
        match dst {
            None => Some((content, alpha_bits(alpha))),
            Some(dst) => Some((
                hash(&[
                    dst.0,
                    u64::from(dst.1),
                    content,
                    u64::from(alpha_bits(alpha)),
                    u64::from(blend),
                ]),
                alpha_bits(1.0),
            )),
        }
    }

    /// node 자기 내용이 완전 불투명 덮어쓰기인지(모사 의미).
    pub(crate) fn replaces(node: &SceneNode) -> bool {
        let paint = node.paint().get();
        matches!(node.content(), SceneContent::Rect(_))
            && matches!(
                paint.layers.last(),
                Some(PaintLayer::Fill(Paint::Solid(color))) if color.alpha() >= 1.0
            )
            && paint
                .layers
                .iter()
                .all(|layer| matches!(layer, PaintLayer::Fill(_)))
            && paint.shadows.is_empty()
    }

    /// node 내용이 덮는 cell index(이동 변환만 가정).
    pub(crate) fn coverage(node: &SceneNode) -> Vec<usize> {
        let Some(bounds) = node.local_content_bounds() else {
            return Vec::new();
        };
        let (_, _, _, _, tx, ty) = node.world_transform().components();
        let x0 = (bounds.min_x() + tx).floor().max(0.0) as usize;
        let y0 = (bounds.min_y() + ty).floor().max(0.0) as usize;
        let x1 = ((bounds.max_x() + tx).ceil().max(0.0) as usize).min(W);
        let y1 = ((bounds.max_y() + ty).ceil().max(0.0) as usize).min(H);
        let mut cells = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                cells.push(y * W + x);
            }
        }
        cells
    }

    fn content_symbol(node: &SceneNode) -> u64 {
        hash(&[u64::from(id_of(node)), 0x5eed])
    }

    /// 내용 하나를 칠합니다(`opacity`·`blend`는 접어 넣은 합성 효과).
    pub(crate) fn paint_content(
        grid: &mut [Cell],
        node: &SceneNode,
        opacity: f32,
        blend: u8,
        pass: impl Fn(usize) -> bool,
    ) {
        let symbol = content_symbol(node);
        let replace = replaces(node) && opacity >= 1.0 && blend == 0;
        for cell in coverage(node) {
            if !pass(cell) {
                continue;
            }
            grid[cell] = if replace {
                Some((symbol, alpha_bits(1.0)))
            } else {
                composite(grid[cell], (symbol, alpha_bits(1.0)), opacity, blend, 0.0)
            };
        }
    }

    fn effects(node: &SceneNode) -> (f32, u8, f32) {
        let paint = node.paint().get();
        (
            paint.opacity.get(),
            paint.blend.tag(),
            paint.layer_blur.get(),
        )
    }

    /// layer를 합성합니다.
    /// layer를 합성합니다. 경계 node 자기 blend가 Normal이면 접힌 `folded_blend`를 씁니다.
    pub(crate) fn composite_layer(
        grid: &mut [Cell],
        layer: &[Cell],
        node: &SceneNode,
        opacity: f32,
        folded_blend: u8,
        pass: impl Fn(usize) -> bool,
    ) {
        let (own_opacity, own_blend, blur) = effects(node);
        let blend = if own_blend == 0 {
            folded_blend
        } else {
            own_blend
        };
        for (cell, value) in layer.iter().enumerate() {
            if let Some(value) = value {
                if pass(cell) {
                    grid[cell] = composite(grid[cell], *value, own_opacity * opacity, blend, blur);
                }
            }
        }
    }

    /// 기준: scene tree를 painter 순서로 칠합니다(합성 효과가 있으면 subtree를 따로 칠해 합성).
    pub(crate) fn reference(snapshot: &SceneSnapshot) -> Vec<Cell> {
        fn eval(node: &SceneNode, grid: &mut Vec<Cell>) {
            let shown = node.hit().visibility == Visibility::Visible;
            if shown && has_compositing_effect(node) {
                let mut layer = vec![None; W * H];
                if draws_content(node) {
                    paint_content(&mut layer, node, 1.0, 0, |_| true);
                }
                for child in node.children() {
                    eval(child, &mut layer);
                }
                composite_layer(grid, &layer, node, 1.0, 0, |_| true);
            } else {
                if shown && draws_content(node) {
                    paint_content(grid, node, 1.0, 0, |_| true);
                }
                for child in node.children() {
                    eval(child, grid);
                }
            }
        }
        let mut grid = vec![None; W * H];
        for root in snapshot.roots() {
            eval(root, &mut grid);
        }
        grid
    }

    /// plan을 pass 실행 순서와 depth 규칙대로 실행합니다.
    pub(crate) fn execute(plan: &FramePlan) -> Vec<Cell> {
        let layer_nodes: HashMap<ZOrder, LayerId> = plan
            .layers
            .iter()
            .map(|layer| (layer.node.z(), layer.id))
            .collect();
        let mut layers: HashMap<LayerId, Vec<Cell>> = HashMap::new();
        let mut main = vec![None; W * H];
        let mut depth = vec![0u32; W * H];
        for pass in plan.passes_in_order() {
            match pass.class {
                PassClass::Effect => {
                    let mut grid = vec![None; W * H];
                    for draw in &pass.draws {
                        apply(&mut grid, draw, &layers, &layer_nodes, |_| true);
                    }
                    layers.insert(pass.layer.unwrap(), grid);
                }
                PassClass::Opaque => {
                    // 앞 → 뒤, depth test + write.
                    let mut last = u32::MAX;
                    for draw in &pass.draws {
                        assert!(draw.depth < last, "opaque pass는 앞 → 뒤 순서여야 합니다");
                        last = draw.depth;
                        assert_eq!(draw.class, PrimitiveClass::Opaque);
                        assert_eq!(draw.opacity, Opacity::OPAQUE);
                        assert_eq!(draw.blend, SceneBlendMode::Normal);
                        let DrawSource::Content(node) = &draw.source else {
                            panic!("opaque pass에 layer 합성");
                        };
                        assert!(replaces(node), "덮어쓰기가 아닌 내용이 opaque pass에 있음");
                        for cell in coverage(node) {
                            if draw.depth > depth[cell] {
                                depth[cell] = draw.depth;
                                main[cell] = Some((content_symbol(node), alpha_bits(1.0)));
                            }
                        }
                    }
                }
                PassClass::Transparent => {
                    let mut last = 0;
                    for draw in &pass.draws {
                        assert!(
                            draw.depth > last,
                            "transparent pass는 뒤 → 앞 순서여야 합니다"
                        );
                        last = draw.depth;
                        let stored = depth.clone();
                        apply(&mut main, draw, &layers, &layer_nodes, |cell| {
                            draw.depth > stored[cell]
                        });
                    }
                }
            }
        }
        main
    }

    fn apply(
        grid: &mut [Cell],
        draw: &PlannedDraw,
        layers: &HashMap<LayerId, Vec<Cell>>,
        layer_nodes: &HashMap<ZOrder, LayerId>,
        pass: impl Fn(usize) -> bool,
    ) {
        match &draw.source {
            DrawSource::Content(node) => {
                // layer 경계 node의 효과는 합성에서, 접힌 효과는 draw opacity·blend로 적용됩니다.
                if layer_nodes.contains_key(&node.z()) {
                    assert_eq!(draw.opacity, Opacity::OPAQUE);
                }
                paint_content(grid, node, draw.opacity.get(), draw.blend.tag(), pass);
            }
            DrawSource::Layer { layer, node } => {
                let content = layers
                    .get(layer)
                    .expect("layer는 합성 전에 그려져야 합니다");
                composite_layer(
                    grid,
                    content,
                    node,
                    draw.opacity.get(),
                    draw.blend.tag(),
                    pass,
                );
            }
        }
    }
}

/// 결정론적 xorshift.
pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// 무작위 scene(합성 효과·중첩·반투명·모서리·소수 위치 포함).
pub(crate) fn random_scene(rng: &mut Rng) -> Vec<NodeFx> {
    fn node(rng: &mut Rng, next_id: &mut u8, depth: u32) -> NodeFx {
        *next_id += 1;
        let id = *next_id;
        let x = rng.below(12) as f32;
        let y = rng.below(12) as f32;
        let w = (rng.below(6) + 1) as f32;
        let h = (rng.below(6) + 1) as f32;
        let alpha = if rng.below(3) == 0 { 0.5 } else { 1.0 };
        let mut fx = rect(id, [x, y, w, h], [0.2, 0.4, 0.6, alpha]);
        match rng.below(10) {
            0 => fx = fx.with_radii([1.0; 4]),
            1 => fx = fx.with_stroke(opaque(0.0, 0.0, 0.0), 1.0),
            2 => {
                fx = fx.with_content(ContentFx::Text {
                    key: id,
                    bounds: [x, y, w, h],
                })
            }
            3 => {
                fx = fx.with_transform([1.0, 0.0, 0.0, 1.0, 0.5, 0.0]);
            }
            4 => {
                fx = fx.with_shadow(ShadowFx {
                    offset: [1.0, 1.0],
                    blur: 0.0,
                    color: [0.0, 0.0, 0.0, 0.5],
                })
            }
            5 => fx.paint.fills.clear(),
            _ => {}
        }
        match rng.below(8) {
            0 => fx = fx.with_opacity(0.5),
            1 => fx = fx.with_opacity(0.25),
            2 => fx = fx.with_blend(1 + rng.below(5) as u8),
            3 if rng.below(2) == 0 => fx = fx.with_blur(2.0),
            _ => {}
        }
        if depth < 3 && rng.below(3) == 0 {
            let children = rng.below(4) + 1;
            for _ in 0..children {
                if *next_id < 200 {
                    let child = node(rng, next_id, depth + 1);
                    fx.children.push(child);
                }
            }
        } else if rng.below(12) == 0 {
            fx = fx.hidden();
        }
        fx
    }
    let mut next_id = 0u8;
    let roots = rng.below(6) + 1;
    (0..roots).map(|_| node(rng, &mut next_id, 0)).collect()
}

#[test]
fn ordering_fixture_classifies_and_orders_passes() {
    // 검증 기준(XI.4.2): ordering fixture 통과.
    let scene = snapshot(&[
        rect(1, [0.0, 0.0, 8.0, 8.0], opaque(1.0, 0.0, 0.0)),
        rect(2, [2.0, 2.0, 4.0, 4.0], [0.0, 1.0, 0.0, 0.5]),
        rect(3, [4.0, 4.0, 8.0, 8.0], opaque(0.0, 0.0, 1.0)),
        group(
            4,
            [0.0, 0.0, 16.0, 16.0],
            vec![
                rect(5, [1.0, 1.0, 2.0, 2.0], opaque(1.0, 1.0, 0.0)),
                rect(6, [2.0, 1.0, 2.0, 2.0], [1.0, 0.0, 1.0, 0.5]),
            ],
        )
        .with_opacity(0.5),
        rect(7, [0.0, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0)).with_radii([1.0; 4]),
        rect(8, [0.0, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0)).with_content(ContentFx::Text {
            key: 8,
            bounds: [0.0, 0.0, 3.0, 3.0],
        }),
        rect(9, [0.5, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0)),
        rect(10, [0.0, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0)).hidden(),
        rect(11, [0.0, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0))
            .with_stroke(opaque(0.0, 0.0, 0.0), 1.0),
        rect(12, [0.0, 0.0, 3.0, 3.0], opaque(1.0, 1.0, 1.0)).with_shadow(ShadowFx {
            offset: [1.0, 1.0],
            blur: 1.0,
            color: [0.0, 0.0, 0.0, 0.5],
        }),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();

    type DrawSummary = Vec<(u8, u32, PrimitiveClass)>;
    let summary: Vec<(PassClass, DrawSummary)> = plan
        .passes_in_order()
        .map(|pass| {
            (
                pass.class,
                pass.draws
                    .iter()
                    .map(|draw| (id_of(draw.source.node()), draw.depth, draw.class))
                    .collect(),
            )
        })
        .collect();
    use PrimitiveClass::{Opaque as O, Transparent as T};
    assert_eq!(
        summary,
        vec![
            // effect layer(4): painter 순서, 내부 opaque도 layer 안에서는 순서대로.
            (PassClass::Effect, vec![(5, 1, O), (6, 2, T)]),
            // opaque: 앞 → 뒤.
            (PassClass::Opaque, vec![(3, 3, O), (1, 1, O)]),
            // transparent: 뒤 → 앞(layer 합성은 node 4의 위치).
            (
                PassClass::Transparent,
                vec![
                    (2, 2, T),
                    (4, 4, T),
                    (7, 5, T),
                    (8, 6, T),
                    (9, 7, T),
                    (11, 8, T),
                    (12, 9, T),
                ]
            ),
        ]
    );
    assert!(matches!(
        plan.passes_in_order().nth(1).unwrap().draws[0].source,
        DrawSource::Content(_)
    ));
    let composite = &plan.passes_in_order().nth(2).unwrap().draws[1];
    assert!(matches!(
        composite.source,
        DrawSource::Layer {
            layer: LayerId(0),
            ..
        }
    ));
    assert_eq!(plan.layers.len(), 1);
    assert_eq!(id_of(&plan.layers[0].node), 4);
    assert_eq!(plan.layers[0].parent, None);
    assert_eq!(plan.draw_count(), 11);
    // graph: layer → opaque → transparent와 node 12 shadow pass(원본 1 + blur 가로·세로 2, ADR-0116 결정 3),
    // output은 surface 최신 version.
    assert_eq!(plan.graph.order().len(), 6);
    assert_eq!(plan.effect_passes.len(), 3);
    assert_eq!(plan.graph.output().map(|v| v.version), Some(2));
    assert_eq!(sim::execute(&plan), sim::reference(&scene));
}

#[test]
fn opaque_requires_pixel_aligned_device_rect() {
    let aligned = rect(1, [2.0, 2.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
    let half = rect(2, [2.5, 2.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
    let rotated = rect(3, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))
        .with_transform([0.0, 1.0, -1.0, 0.0, 8.0, 0.0]);
    let clipped = rect(4, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))
        .with_clip(crate::test_scene::rect_clip([0.0, 0.0, 2.0, 2.0]));
    let half_clip = rect(5, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))
        .with_clip(crate::test_scene::rect_clip([0.0, 0.0, 2.5, 2.0]));
    let translucent = rect(6, [0.0, 0.0, 4.0, 4.0], [1.0, 0.0, 0.0, 0.99]);
    let tilted = rect(7, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))
        .with_transform([0.8, 0.6, -0.6, 0.8, 8.0, 0.0]);
    let rounded_clip =
        rect(8, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)).with_clip(crate::test_scene::ClipFx {
            radii: [1.0; 4],
            ..crate::test_scene::rect_clip([0.0, 0.0, 2.0, 2.0])
        });
    let scene = snapshot(&[
        aligned,
        half,
        rotated,
        clipped,
        half_clip,
        translucent,
        tilted,
        rounded_clip,
    ]);
    let all = nodes(&scene);
    let identity = viewport_with(32.0, 32.0, IDENTITY);
    let classes: Vec<PrimitiveClass> = all
        .iter()
        .map(|node| classify_content(node, &identity, 0))
        .collect();
    use PrimitiveClass::{Opaque as O, Transparent as T};
    // 90° 회전은 축을 보존하고 pixel 경계에 놓여 opaque, 임의 회전·둥근 clip은 transparent.
    assert_eq!(classes, vec![O, T, O, O, T, T, T, T]);
    // layer 안쪽(clip을 건너뛴 위치)에서는 바깥 둥근 clip을 보지 않습니다.
    assert_eq!(classify_content(&all[7], &identity, 1), O);
    // zoom 2배면 2.5는 5로 정렬되고, zoom 1.5배면 2는 3, 6은 9로 정렬되지만 4.5는 아닙니다.
    let zoom2 = viewport_with(64.0, 64.0, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    assert_eq!(classify_content(&all[1], &zoom2, 0), O);
    let zoom15 = viewport_with(64.0, 64.0, [1.5, 0.0, 0.0, 1.5, 0.0, 0.0]);
    assert_eq!(classify_content(&all[0], &zoom15, 0), O);
    assert_eq!(classify_content(&all[1], &zoom15, 0), T);
    // pan이 소수 pixel이면 정렬이 깨집니다.
    let pan = viewport_with(64.0, 64.0, [1.0, 0.0, 0.0, 1.0, 0.25, 0.0]);
    assert_eq!(classify_content(&all[0], &pan, 0), T);
}

#[test]
fn compiled_plans_preserve_painter_order_on_random_scenes() {
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let mut rng = Rng(0x0dd5_eed5_0f00_0001);
    let mut with_layers = 0;
    let mut with_opaque = 0;
    for _ in 0..400 {
        let roots = random_scene(&mut rng);
        let scene = snapshot(&roots);
        let plan =
            compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();
        if !plan.layers.is_empty() {
            with_layers += 1;
        }
        if plan
            .passes
            .iter()
            .any(|pass| pass.class == PassClass::Opaque && !pass.draws.is_empty())
        {
            with_opaque += 1;
        }
        assert_eq!(sim::execute(&plan), sim::reference(&scene));
    }
    // 무작위 scene이 두 경로를 충분히 다룹니다.
    assert!(with_layers > 100, "{with_layers}");
    assert!(with_opaque > 100, "{with_opaque}");
}

#[test]
fn visibility_list_filters_draws_but_keeps_layer_boundaries() {
    // group(effect)은 화면 밖에 있지만 자식 하나는 화면 안입니다.
    let scene = snapshot(&[
        rect(1, [100.0, 100.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)),
        NodeFx {
            paint: crate::test_scene::PaintFx {
                fills: vec![opaque(0.0, 0.0, 0.0)],
                ..Default::default()
            },
            ..group(
                2,
                [50.0, 50.0, 4.0, 4.0],
                vec![
                    rect(3, [1.0, 1.0, 2.0, 2.0], opaque(0.0, 1.0, 0.0)),
                    rect(4, [70.0, 70.0, 2.0, 2.0], opaque(0.0, 0.0, 1.0)),
                ],
            )
        }
        .with_opacity(0.5),
        rect(5, [2.0, 2.0, 2.0, 2.0], opaque(1.0, 1.0, 1.0)),
    ]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let index = SceneSpatialIndex::build(&scene);
    let visible = visible_candidates(&index, &viewport);
    let mut compile = input(&scene, &viewport, EffectBoundaries::Conservative);
    compile.visible = Some(&visible);
    let plan = compile_frame(&compile).unwrap();
    let drawn: Vec<u8> = plan
        .passes_in_order()
        .flat_map(|pass| pass.draws.iter().map(|draw| id_of(draw.source.node())))
        .collect();
    // 화면 밖 1·2·4는 그리지 않지만 node 2의 layer 경계는 남아 3을 합성합니다.
    assert_eq!(drawn, vec![3, 5, 2]);
    assert_eq!(plan.layers.len(), 1);

    // 화면 안 자식이 없는 effect group은 layer를 만들지 않습니다.
    let offscreen = snapshot(&[group(
        1,
        [50.0, 50.0, 4.0, 4.0],
        vec![rect(2, [60.0, 60.0, 2.0, 2.0], opaque(0.0, 0.0, 1.0))],
    )
    .with_opacity(0.5)]);
    let index = SceneSpatialIndex::build(&offscreen);
    let visible = visible_candidates(&index, &viewport);
    let mut compile = input(&offscreen, &viewport, EffectBoundaries::Conservative);
    compile.visible = Some(&visible);
    let plan = compile_frame(&compile).unwrap();
    assert!(plan.layers.is_empty());
    assert_eq!(plan.draw_count(), 0);
    // 빈 frame도 surface clear pass 하나로 유효한 graph입니다.
    assert_eq!(plan.graph.order().len(), 1);
}

#[test]
fn deep_nested_effects_compile_without_recursion() {
    // 600단 중첩(짝수 단은 opacity 효과): scope stack과 순회가 모두 반복문입니다.
    let mut leaf = rect(200, [1.0, 1.0, 2.0, 2.0], opaque(1.0, 0.0, 0.0));
    for level in 0..600u32 {
        let mut parent = group(1 + (level % 199) as u8, [0.0, 0.0, 8.0, 8.0], vec![leaf]);
        if level % 2 == 0 {
            parent = parent.with_opacity(0.5);
        }
        leaf = parent;
    }
    let scene = snapshot(&[leaf]);
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let plan = compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();
    assert_eq!(plan.layers.len(), 300);
    // 각 layer는 자식 layer를 합성하는 draw 하나(가장 안쪽은 leaf)만 갖고, main transparent pass가
    // 가장 바깥 layer를 합성합니다.
    assert_eq!(plan.draw_count(), 301);
    let order: Vec<PassId> = plan.graph.order().to_vec();
    // layer pass 300 + opaque(clear) + transparent.
    assert_eq!(order.len(), 302);
    // 안쪽 layer pass가 바깥 layer pass보다 먼저 실행됩니다.
    let position = |layer: LayerId| {
        let pass = plan
            .passes
            .iter()
            .find(|pass| pass.layer == Some(layer))
            .unwrap()
            .pass;
        order.iter().position(|p| *p == pass).unwrap()
    };
    for layer in &plan.layers {
        if let Some(parent) = layer.parent {
            assert!(position(layer.id) < position(parent));
        }
    }
    assert_eq!(plan.layer(LayerId(0)).map(|layer| layer.parent), Some(None));
    assert!(plan.layer(LayerId(9_999)).is_none());
}
