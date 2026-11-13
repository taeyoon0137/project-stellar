//! 테스트 전용 scene fixture 생성기 (XI.1.2).
//!
//! 고정 seed의 결정론적 난수(SplitMix64)로 world transform·layout rect·clip stack·paint
//! stack·z-order·hit-test metadata를 모두 채운 scene을 만듭니다. 같은 설정은 항상 같은
//! snapshot을 만듭니다.

use std::sync::Arc;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};

use crate::geometry::{FillRule, PathCommand, PathGeometry};
use crate::node::{
    CornerRadii, HitTestMeta, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PathPrimitive,
    PointerEvents, RectPrimitive, SceneContent, SceneNode, SceneNodeSpec, TextLayoutKey,
    TextPrimitive, Visibility, ZOrder,
};
use crate::snapshot::SceneSnapshot;
use crate::stack::{
    ClipShape, ClipStackRef, Opacity, Paint, PaintLayer, PaintStack, PaintStackRef, SceneBlendMode,
    StrokeAlign, StrokeCap, StrokeLayer,
};

/// 결정론적 난수(SplitMix64).
pub(crate) struct SplitMix64(u64);

impl SplitMix64 {
    pub(crate) fn new(seed: u64) -> SplitMix64 {
        SplitMix64(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `[0, n)` 정수.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// `[lo, hi)` 실수(1/1024 단위로 양자화해 IEEE 기본 연산만으로 재현 가능).
    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let steps = (((hi - lo) * 1024.0) as u64).max(1);
        lo + self.below(steps) as f32 / 1024.0
    }
}

pub(crate) fn id(n: u64) -> StableId {
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&n.to_be_bytes());
    StableId::from_bytes(bytes)
}

pub(crate) fn p(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).expect("finite point")
}

pub(crate) fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(p(x, y), Size2::new(w, h).expect("non-negative size"))
}

pub(crate) fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).expect("finite transform")
}

pub(crate) fn solid(r: f32, g: f32, b: f32, a: f32) -> Paint {
    Paint::Solid(Color::new(ColorSpace::Srgb, r, g, b, a).expect("valid color"))
}

pub(crate) fn rect_content(w: f32, h: f32) -> SceneContent {
    SceneContent::Rect(RectPrimitive {
        rect: rect(0.0, 0.0, w, h),
        radii: CornerRadii::ZERO,
    })
}

/// 항등 변환·content 상자 layout·clip 없음·빈 paint·interactive 기본 spec.
pub(crate) fn spec(source: u64, content: SceneContent, z: u64) -> SceneNodeSpec {
    let layout_rect = match &content {
        SceneContent::Rect(r) => r.rect,
        SceneContent::Text(t) => t.bounds,
        SceneContent::Image(i) => i.bounds,
        SceneContent::Path(_) => rect(0.0, 0.0, 0.0, 0.0),
    };
    SceneNodeSpec {
        source: id(source),
        content,
        world_transform: Transform2D::identity(),
        layout_rect,
        clip: ClipStackRef::none(),
        paint: PaintStackRef::empty(),
        z: ZOrder(z),
        hit: HitTestMeta::INTERACTIVE,
    }
}

/// 크기 `w×h` rect leaf.
pub(crate) fn leaf(source: u64, z: u64, w: f32, h: f32) -> Arc<SceneNode> {
    Arc::new(SceneNode::leaf(spec(source, rect_content(w, h), z)))
}

/// fixture 설정.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixtureConfig {
    pub(crate) seed: u64,
    /// 최상위 frame 수.
    pub(crate) roots: usize,
    /// frame마다 자식 수.
    pub(crate) children: usize,
    /// frame 중첩 단계(1이면 최상위 frame 아래가 바로 leaf).
    pub(crate) depth: usize,
}

struct Generator {
    rng: SplitMix64,
    next_z: u64,
    next_source: u64,
}

impl Generator {
    fn z(&mut self) -> ZOrder {
        // 간격을 두고 증가(연속일 필요 없음).
        self.next_z += 1 + self.rng.below(3);
        ZOrder(self.next_z)
    }

    fn source(&mut self) -> u64 {
        self.next_source += 1;
        self.next_source
    }

    fn color(&mut self) -> Color {
        let space = if self.rng.below(4) == 0 {
            ColorSpace::DisplayP3
        } else {
            ColorSpace::Srgb
        };
        Color::new(
            space,
            self.rng.range(0.0, 1.0),
            self.rng.range(0.0, 1.0),
            self.rng.range(0.0, 1.0),
            self.rng.range(0.0, 1.0),
        )
        .expect("valid color")
    }

    fn hit(&mut self) -> HitTestMeta {
        HitTestMeta {
            visibility: if self.rng.below(8) == 0 {
                Visibility::Hidden
            } else {
                Visibility::Visible
            },
            pointer_events: if self.rng.below(8) == 0 {
                PointerEvents::None
            } else {
                PointerEvents::Auto
            },
        }
    }

    fn paint(&mut self, stroke_align: StrokeAlign) -> PaintStackRef {
        let mut layers = Vec::new();
        if self.rng.below(3) != 0 {
            layers.push(PaintLayer::Fill(Paint::Solid(self.color())));
        }
        if self.rng.below(2) == 0 {
            let cap = StrokeCap::from_tag(self.rng.below(3) as u8).expect("cap tag");
            layers.push(PaintLayer::Stroke(
                StrokeLayer::new(
                    Paint::Solid(self.color()),
                    self.rng.range(0.0, 4.0),
                    stroke_align,
                    cap,
                )
                .expect("stroke"),
            ));
        }
        let shadows = if self.rng.below(4) == 0 {
            vec![Shadow::new(
                p(self.rng.range(-4.0, 4.0), self.rng.range(-4.0, 4.0)),
                BlurRadius::new(self.rng.range(0.0, 8.0)).expect("blur"),
                self.color(),
            )]
        } else {
            Vec::new()
        };
        PaintStackRef::new(PaintStack {
            layers,
            shadows,
            opacity: Opacity::new(self.rng.range(0.5, 1.0)).expect("opacity"),
            blend: SceneBlendMode::from_tag(self.rng.below(6) as u8).expect("blend tag"),
            layer_blur: BlurRadius::new(if self.rng.below(6) == 0 { 2.0 } else { 0.0 })
                .expect("blur"),
        })
    }

    fn layout_key(&mut self) -> TextLayoutKey {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.rng.next_u64().to_le_bytes());
        bytes[8..].copy_from_slice(&self.rng.next_u64().to_le_bytes());
        TextLayoutKey::from_bytes(bytes)
    }

    fn leaf_content(&mut self, w: f32, h: f32) -> (SceneContent, Rect) {
        let bounds = rect(0.0, 0.0, w, h);
        let content = match self.rng.below(4) {
            0 => SceneContent::Rect(RectPrimitive {
                rect: bounds,
                radii: CornerRadii::new(1.0, 1.0, 1.0, 1.0).expect("radii"),
            }),
            1 => SceneContent::Path(PathPrimitive {
                geometry: PathGeometry::new(vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::LineTo(p(w, 0.0)),
                    PathCommand::CubicTo {
                        c1: p(w, h / 2.0),
                        c2: p(w / 2.0, h),
                        to: p(0.0, h),
                    },
                    PathCommand::Close,
                ])
                .expect("path"),
                fill_rule: if self.rng.below(2) == 0 {
                    FillRule::NonZero
                } else {
                    FillRule::EvenOdd
                },
            }),
            2 => SceneContent::Text(TextPrimitive {
                layout: self.layout_key(),
                bounds,
            }),
            _ => SceneContent::Image(ImagePrimitive {
                asset: AssetRef::new(id(1_000_000 + self.rng.below(16))),
                bounds,
                fit: ImageFit::from_tag(self.rng.below(4) as u8).expect("fit tag"),
                crop: ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space: None,
            }),
        };
        (content, bounds)
    }

    fn frame(
        &mut self,
        depth: usize,
        children: usize,
        parent_world: Transform2D,
        parent_clip: &ClipStackRef,
    ) -> Arc<SceneNode> {
        let w = self.rng.range(50.0, 400.0);
        let h = self.rng.range(50.0, 400.0);
        let local = translate(self.rng.range(-200.0, 600.0), self.rng.range(-200.0, 600.0));
        let world = local.then(&parent_world).expect("finite world transform");
        let frame_rect = rect(0.0, 0.0, w, h);
        let radii = CornerRadii::new(2.0, 2.0, 2.0, 2.0).expect("radii");
        let spec = SceneNodeSpec {
            source: id(self.source()),
            content: SceneContent::Rect(RectPrimitive {
                rect: frame_rect,
                radii,
            }),
            world_transform: world,
            layout_rect: frame_rect,
            clip: parent_clip.clone(),
            paint: self.paint(StrokeAlign::Inside),
            z: self.z(),
            hit: self.hit(),
        };
        // 대부분의 frame은 bounds clip, 일부는 path clip, 일부는 clip 없음.
        let child_clip = match self.rng.below(5) {
            0 => parent_clip.clone(),
            1 => parent_clip.push(ClipShape::Path {
                geometry: PathGeometry::new(vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::LineTo(p(w, 0.0)),
                    PathCommand::LineTo(p(0.0, h)),
                    PathCommand::Close,
                ])
                .expect("clip path"),
                fill_rule: FillRule::NonZero,
                transform: world,
            }),
            _ => parent_clip.push(ClipShape::Rect {
                rect: frame_rect,
                radii,
                transform: world,
            }),
        };
        let mut nodes = Vec::with_capacity(children);
        for _ in 0..children {
            if depth > 1 {
                nodes.push(self.frame(depth - 1, children, world, &child_clip));
            } else {
                let cw = self.rng.range(4.0, 120.0);
                let ch = self.rng.range(4.0, 120.0);
                let (content, bounds) = self.leaf_content(cw, ch);
                let local = translate(self.rng.range(0.0, w), self.rng.range(0.0, h));
                let leaf = SceneNodeSpec {
                    source: id(self.source()),
                    content,
                    world_transform: local.then(&world).expect("finite world transform"),
                    layout_rect: bounds,
                    clip: child_clip.clone(),
                    paint: self.paint(StrokeAlign::Center),
                    z: self.z(),
                    hit: self.hit(),
                };
                nodes.push(Arc::new(SceneNode::leaf(leaf)));
            }
        }
        Arc::new(SceneNode::new(spec, nodes).expect("fixture z-order"))
    }
}

/// 설정대로 결정론적 snapshot을 만듭니다.
pub(crate) fn generate(config: FixtureConfig) -> SceneSnapshot {
    let mut generator = Generator {
        rng: SplitMix64::new(config.seed),
        next_z: 0,
        next_source: 0,
    };
    let roots = (0..config.roots)
        .map(|_| {
            generator.frame(
                config.depth,
                config.children,
                Transform2D::identity(),
                &ClipStackRef::none(),
            )
        })
        .collect();
    SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), roots).expect("fixture z-order")
}

/// 작은 기본 fixture: 최상위 2개 × 자식 3개 × 2단계.
pub(crate) fn small() -> SceneSnapshot {
    generate(FixtureConfig {
        seed: 0x5EED,
        roots: 2,
        children: 3,
        depth: 2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec;

    #[test]
    fn fixture_generation_fills_every_node_field() {
        // 검증 기준(XI.1.2): world transform·layout rect·clip·paint·z-order·hit-test
        // metadata를 모두 채운 fixture가 생성됩니다.
        let config = FixtureConfig {
            seed: 0xF17E,
            roots: 4,
            children: 4,
            depth: 3,
        };
        let snapshot = generate(config);
        assert_eq!(snapshot.node_count(), 4 * (1 + 4 + 16 + 64));

        let mut kinds = [false; 4];
        let (mut moved, mut clipped, mut path_clipped) = (0, 0, 0);
        let (mut layered, mut shadowed, mut blended) = (0, 0, 0);
        let (mut hidden, mut pass_through) = (0, 0);
        let mut previous: Option<ZOrder> = None;
        for node in snapshot.iter_preorder() {
            kinds[match node.content() {
                SceneContent::Rect(_) => 0,
                SceneContent::Path(_) => 1,
                SceneContent::Text(_) => 2,
                SceneContent::Image(_) => 3,
            }] = true;
            if node.world_transform() != Transform2D::identity() {
                moved += 1;
            }
            assert!(!node.layout_rect().is_empty(), "layout rect 비어 있음");
            if !node.clip().is_empty() {
                clipped += 1;
            }
            if node
                .clip()
                .iter_inner_first()
                .any(|clip| matches!(clip, ClipShape::Path { .. }))
            {
                path_clipped += 1;
            }
            let paint = node.paint().get();
            if !paint.layers.is_empty() {
                layered += 1;
            }
            if !paint.shadows.is_empty() {
                shadowed += 1;
            }
            if paint.blend != SceneBlendMode::Normal {
                blended += 1;
            }
            if node.hit().visibility == Visibility::Hidden {
                hidden += 1;
            }
            if node.hit().pointer_events == PointerEvents::None {
                pass_through += 1;
            }
            // pre-order z-order 엄격 증가.
            if let Some(previous) = previous {
                assert!(node.z() > previous, "z-order 불변식 위반");
            }
            previous = Some(node.z());
        }
        assert_eq!(kinds, [true; 4], "모든 content 종류");
        assert!(moved > 0 && clipped > 0 && path_clipped > 0);
        assert!(layered > 0 && shadowed > 0 && blended > 0);
        assert!(hidden > 0 && pass_through > 0);

        // 중첩 frame의 자식 clip stack은 조상 clip을 공유하며 더 깊거나 같습니다.
        let root = snapshot.root(0).expect("root");
        let child = root.child(0).expect("child");
        let grandchild = child.child(0).expect("grandchild");
        assert!(grandchild.clip().depth() >= child.clip().depth());

        // 결정론: 같은 설정은 같은 bytes, 다른 seed는 다른 bytes.
        let bytes = codec::encode(&snapshot);
        assert_eq!(codec::encode(&generate(config)), bytes);
        assert_ne!(
            codec::encode(&generate(FixtureConfig { seed: 1, ..config })),
            bytes
        );
        // fixture는 canonical round-trip도 통과합니다.
        assert_eq!(codec::decode(&bytes).expect("decode"), snapshot);
    }
}
