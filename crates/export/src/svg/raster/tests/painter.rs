//! 기호 painter 모사로 부분 래스터화 경계의 정확성을 검사합니다 (XX.4.1).
//!
//! pixel 값은 그 pixel에 Normal over로 쌓인 항목 목록입니다. Normal over는 결합 법칙을 만족하므로
//! 목록을 이어 붙이는 것이 합성이고, blend·opacity·blur는 입력 목록 전체를 담은 새 항목 하나를
//! 만듭니다. 그래서 blend가 읽은 backdrop이 하나라도 달라지면 결과 항목이 달라집니다.
//!
//! 원래 합성과, 계획의 Subtrees 경계를 투명 바탕에 따로 그린 image로 바꿔 경계 첫 위치에 Normal로
//! 합성한 결과가 canvas의 모든 pixel에서 같아야 합니다. Content 경계는 같은 내용을 같은 위치에
//! 그리므로, Canvas 경계는 전체를 그대로 그리므로 모사에서 원래 합성과 같습니다.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::*;

/// 모사 영역 `[MIN, MIN + SIDE)²`(canvas 밖 blur 번짐 포함).
const MIN: i64 = -10;
const SIDE: i64 = 50;

type Layer = Vec<Vec<String>>;

fn layer() -> Layer {
    vec![Vec::new(); (SIDE * SIDE) as usize]
}

fn index(x: i64, y: i64) -> Option<usize> {
    let (x, y) = (x - MIN, y - MIN);
    ((0..SIDE).contains(&x) && (0..SIDE).contains(&y)).then(|| (y * SIDE + x) as usize)
}

fn digest(parts: &[&[String]]) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

/// node 사각형(정수 world 좌표, 변환은 이동뿐).
fn rect(node: &SceneNode) -> (i64, i64, i64, i64) {
    let (_, _, _, _, tx, ty) = node.world_transform().components();
    let size = node.layout_rect().size();
    let (x, y) = (tx as i64, ty as i64);
    (x, y, x + size.width() as i64, y + size.height() as i64)
}

fn draw_own(node: &SceneNode, target: &mut Layer) {
    if node.paint().get().layers.is_empty() {
        return;
    }
    let (x0, y0, x1, y1) = rect(node);
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(i) = index(x, y) {
                target[i].push(format!("d{}", node.z().0));
            }
        }
    }
}

/// 반경 `radius`(Chebyshev 거리) 안 값을 모두 섞습니다.
fn blurred(source: &Layer, radius: i64, z: u64) -> Layer {
    let mut out = layer();
    for y in MIN..MIN + SIDE {
        for x in MIN..MIN + SIDE {
            let mut window: Vec<&[String]> = Vec::new();
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if let Some(i) = index(x + dx, y + dy) {
                        window.push(&source[i]);
                    }
                }
            }
            if window.iter().any(|values| !values.is_empty()) {
                out[index(x, y).unwrap()] = vec![format!("u{z}:{:x}", digest(&window))];
            }
        }
    }
    out
}

/// group 결과를 node 합성 효과(blur → opacity → blend 순)로 `target`에 합성합니다.
fn composite(node: &SceneNode, group: Layer, target: &mut Layer) {
    let paint = node.paint().get();
    let z = node.z().0;
    let radius = paint.layer_blur.get() as i64;
    let mut group = if radius > 0 {
        blurred(&group, radius, z)
    } else {
        group
    };
    if paint.opacity < Opacity::OPAQUE {
        for values in group.iter_mut().filter(|values| !values.is_empty()) {
            *values = vec![format!("o{z}:{:x}", digest(&[values]))];
        }
    }
    for (dst, src) in target.iter_mut().zip(group) {
        if src.is_empty() {
            continue;
        }
        match paint.blend {
            SceneBlendMode::Normal => dst.extend(src),
            mode => {
                let value = format!("b{}:{:x}", mode.tag(), digest(&[&src, dst]));
                *dst = vec![value];
            }
        }
    }
}

/// `plan`이 있으면 Subtrees 경계를 따로 그린 image로 바꿉니다.
struct Painter<'a> {
    plan: Option<&'a RasterPlan>,
}

impl Painter<'_> {
    fn paint_list(&self, nodes: &[&SceneNode], target: &mut Layer) {
        let mut i = 0;
        while i < nodes.len() {
            let region = self
                .plan
                .and_then(|plan| plan.boundary_at(nodes[i].z()))
                .map(|boundary| boundary.region().clone());
            if let Some(RasterRegion::Subtrees { first, last, .. }) = region {
                assert_eq!(first.z, nodes[i].z());
                let original = Painter { plan: None };
                let mut image = layer();
                let mut j = i;
                loop {
                    original.paint(nodes[j], &mut image);
                    if nodes[j].z() == last.z {
                        break;
                    }
                    j += 1;
                }
                for (dst, src) in target.iter_mut().zip(image) {
                    dst.extend(src);
                }
                i = j + 1;
                continue;
            }
            self.paint(nodes[i], target);
            i += 1;
        }
    }

    fn paint(&self, node: &SceneNode, target: &mut Layer) {
        if node.hit().visibility == Visibility::Hidden {
            return;
        }
        let children: Vec<&SceneNode> = node.children().map(|child| &**child).collect();
        if is_isolating(node) {
            let mut group = layer();
            draw_own(node, &mut group);
            self.paint_list(&children, &mut group);
            composite(node, group, target);
        } else {
            draw_own(node, target);
            self.paint_list(&children, target);
        }
    }
}

/// canvas `[0, side)²`의 최종 pixel 목록.
fn render(
    roots: &[Arc<SceneNode>],
    plan: Option<&RasterPlan>,
    background: bool,
    side: i64,
) -> Vec<Vec<String>> {
    let mut target = layer();
    if background {
        for y in 0..side {
            for x in 0..side {
                target[index(x, y).unwrap()].push("bg".to_string());
            }
        }
    }
    let refs: Vec<&SceneNode> = roots.iter().map(|root| &**root).collect();
    Painter { plan }.paint_list(&refs, &mut target);
    let mut out = Vec::new();
    for y in 0..side {
        for x in 0..side {
            out.push(target[index(x, y).unwrap()].clone());
        }
    }
    out
}

/// 결정론적 xorshift64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// 무작위 scene(정수 좌표, canvas 24×24 주변과 밖).
fn random_scene(rng: &mut Rng) -> Vec<Arc<SceneNode>> {
    let count = 2 + rng.below(10) as usize;
    let mut parents: Vec<Option<usize>> = Vec::with_capacity(count);
    let mut rects: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(count);
    let mut trees: Vec<Option<T>> = Vec::with_capacity(count);
    for k in 0..count {
        let parent = if k == 0 || rng.chance(35) {
            None
        } else {
            Some(rng.below(k as u64) as usize)
        };
        parents.push(parent);
        let w = 1.0 + rng.below(8) as f32;
        let h = 1.0 + rng.below(8) as f32;
        // 자식은 부모 상자 주변(±3)에 둡니다. 최상위는 canvas 안팎 어디든.
        let (x, y) = match parent {
            Some(parent) => {
                let (px, py, pw, ph) = rects[parent];
                (
                    px - 3.0 + rng.below(pw as u64 + 6) as f32,
                    py - 3.0 + rng.below(ph as u64 + 6) as f32,
                )
            }
            None => (rng.below(26) as f32 - 4.0, rng.below(26) as f32 - 4.0),
        };
        rects.push((x, y, w, h));
        let mut paint = if rng.chance(70) { fill() } else { none() };
        if rng.chance(30) {
            paint.blend = if rng.chance(50) {
                SceneBlendMode::Multiply
            } else {
                SceneBlendMode::Screen
            };
        }
        if rng.chance(20) {
            paint = opacity(paint, 0.5);
        }
        if rng.chance(15) {
            paint = blur(paint, 1.0 + rng.below(3) as f32);
        }
        let mut tree = t(k as u64 + 1, (x, y, w, h), paint);
        if rng.chance(8) {
            tree = tree.hidden();
        }
        trees.push(Some(tree));
    }
    // 뒤에서부터 부모 앞쪽에 붙이면 생성 순서가 형제 paint 순서로 남습니다.
    for k in (0..count).rev() {
        if let Some(parent) = parents[k] {
            let child = trees[k].take().unwrap();
            trees[parent].as_mut().unwrap().children.insert(0, child);
        }
    }
    build(trees.into_iter().flatten().collect())
}

#[test]
fn minimal_regions_preserve_the_composite_on_random_scenes() {
    const CANVAS: i64 = 24;
    let mut rng = Rng(0x5eed_cafe_f00d_1234);
    let (mut scenes_with_regions, mut vector_kept, mut ranges, mut widened) = (0, 0u64, 0, 0);
    for _ in 0..500 {
        let roots = random_scene(&mut rng);
        let background = rng.chance(20);
        let planned = plan(&roots, grid(CANVAS as f32, CANVAS as f32, 1.0), background);
        let expected = render(&roots, None, background, CANVAS);
        let actual = render(&roots, Some(&planned), background, CANVAS);
        assert_eq!(
            expected,
            actual,
            "합성 결과가 달라짐: {:?}",
            planned.boundaries()
        );
        // 보이는 래스터 기능 node는 모두 어떤 경계 안에 있습니다.
        let mut stack: Vec<&SceneNode> = roots.iter().map(|root| &**root).collect();
        while let Some(node) = stack.pop() {
            if node.hit().visibility == Visibility::Hidden {
                continue;
            }
            for feature in node_features(node) {
                if raster_scope(feature).is_some() {
                    assert!(planned.rasterizes(node.z()), "{feature} node가 경계 밖");
                }
            }
            stack.extend(node.children().map(|child| &**child));
        }
        if !planned.is_empty() {
            scenes_with_regions += 1;
            vector_kept += planned.vector_nodes();
            for boundary in planned.boundaries() {
                if let RasterRegion::Subtrees { first, last, .. } = boundary.region() {
                    ranges += 1;
                    if first != last {
                        widened += 1;
                    }
                }
            }
        }
    }
    // 검출력: 경계와 backdrop으로 넓힌 범위가 충분히 나오고, 그 scene에서도 vector가 남습니다.
    assert!(scenes_with_regions > 200, "{scenes_with_regions}");
    assert!(ranges > 200, "{ranges}");
    assert!(widened > 30, "{widened}");
    assert!(vector_kept > 200, "{vector_kept}");
}
