//! glyph cache miss 정책 테스트 (XI.6.6).

use super::*;
use std::collections::HashMap;

use stellar_scene::geometry::PathCommand;
use stellar_scene::Vec2;

use crate::glyph::tests::{font, key};
use crate::glyph::{
    AtlasConfig, AxisTag, GlyphBitmap, GlyphBitmapFormat, ShelfGlyphAtlas, VariationCoords,
};

fn atlas(page_size: u32, max_pages: u32) -> ShelfGlyphAtlas {
    ShelfGlyphAtlas::new(AtlasConfig {
        page_size,
        max_pages,
        padding: 1,
    })
    .unwrap()
}

fn coverage_key(glyph: u32, px: f32) -> GlyphKey {
    key(
        font(1, 0),
        glyph,
        px,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    )
}

/// 테스트 rasterizer 결과: 크기에 비례한 bitmap, Outline은 glyph id·크기를 담은 사각형 path.
fn render(key: &GlyphKey) -> GlyphImage {
    if key.glyph == GlyphId(0) {
        return GlyphImage::Empty;
    }
    let side = ((key.size.px() / 2.0) as u32).clamp(1, 20);
    match key.mode.bitmap_format() {
        None => GlyphImage::Outline(outline_for(key)),
        Some(format) => GlyphImage::Bitmap(
            GlyphBitmap::new(
                side,
                side,
                0,
                -(side as i32),
                format,
                vec![key.glyph.0 as u8; (side * side) as usize * format.bytes_per_pixel()],
            )
            .unwrap(),
        ),
    }
}

fn outline_for(key: &GlyphKey) -> PathGeometry {
    let w = key.glyph.0 as f32 + 1.0;
    let h = key.size.px();
    let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, -h)),
        PathCommand::LineTo(p(w, -h)),
        PathCommand::LineTo(p(w, 0.0)),
        PathCommand::Close,
    ])
    .unwrap()
}

/// 요청을 모두 바로 rasterize해 반영합니다.
fn complete_all(resolver: &mut GlyphResolver, atlas: &mut ShelfGlyphAtlas) {
    for key in resolver.take_requests() {
        let image = render(&key);
        resolver.complete(atlas, key, Ok(image));
    }
}

#[test]
fn zoom_draws_previous_valid_size_until_exact_is_ready() {
    let mut atlas = atlas(256, 2);
    let mut resolver = GlyphResolver::new();
    atlas.begin_frame(1).unwrap();
    let k16 = coverage_key(5, 16.0);
    // 처음 보는 glyph: 대체가 없으므로 빈 glyph(placeholder 아님), rasterize 요청 1회.
    assert_eq!(
        resolver.resolve(&mut atlas, &k16),
        GlyphDraw::Empty(EmptyReason::Pending)
    );
    assert_eq!(
        resolver.resolve(&mut atlas, &k16),
        GlyphDraw::Empty(EmptyReason::Pending)
    );
    assert_eq!(resolver.take_requests(), vec![k16.clone()]);
    let CompleteOutcome::Placed(p16) = resolver.complete(&mut atlas, k16.clone(), Ok(render(&k16)))
    else {
        panic!("배치되어야 합니다");
    };

    atlas.begin_frame(2).unwrap();
    assert_eq!(
        resolver.resolve(&mut atlas, &k16),
        GlyphDraw::Exact(GlyphSource::Atlas(p16))
    );
    // zoom: 18 px 요청 → 16 px glyph를 1.125배로.
    let k18 = coverage_key(5, 18.0);
    assert_eq!(
        resolver.resolve(&mut atlas, &k18),
        GlyphDraw::Substitute {
            source: GlyphSource::Atlas(p16),
            from: k16.size,
            scale: 1.125,
        }
    );
    complete_all(&mut resolver, &mut atlas);
    atlas.begin_frame(3).unwrap();
    assert!(matches!(
        resolver.resolve(&mut atlas, &k18),
        GlyphDraw::Exact(GlyphSource::Atlas(_))
    ));

    // 대체 우선순위: 요청 이상 중 가장 작은 크기 → 없으면 요청 미만 중 가장 큰 크기.
    let k24 = coverage_key(5, 24.0);
    resolver.resolve(&mut atlas, &k24);
    complete_all(&mut resolver, &mut atlas);
    let pick = |resolver: &mut GlyphResolver, atlas: &mut ShelfGlyphAtlas, px: f32| match resolver
        .resolve(atlas, &coverage_key(5, px))
    {
        GlyphDraw::Substitute { from, .. } => from.px(),
        other => panic!("대체 기대: {other:?}"),
    };
    assert_eq!(pick(&mut resolver, &mut atlas, 20.0), 24.0);
    assert_eq!(pick(&mut resolver, &mut atlas, 17.0), 18.0);
    assert_eq!(pick(&mut resolver, &mut atlas, 30.0), 24.0);
    assert_eq!(pick(&mut resolver, &mut atlas, 12.0), 16.0);
}

#[test]
fn never_substitutes_a_different_glyph() {
    // 검증 기준(XI.6.6): 잘못된 glyph 표시 없음.
    let mut atlas = atlas(256, 4);
    let mut resolver = GlyphResolver::new();
    atlas.begin_frame(1).unwrap();
    let placed = coverage_key(5, 16.0);
    resolver.resolve(&mut atlas, &placed);
    complete_all(&mut resolver, &mut atlas);
    // 다른 glyph도 cache에 둡니다(같은 크기라도 대체로 쓰이면 안 됨).
    let neighbor = coverage_key(7, 18.0);
    resolver.resolve(&mut atlas, &neighbor);
    complete_all(&mut resolver, &mut atlas);
    atlas.begin_frame(2).unwrap();

    let wght = VariationCoords::new([(AxisTag::parse("wght").unwrap(), 700.0)]).unwrap();
    let others = [
        // 다른 glyph id.
        coverage_key(6, 16.0),
        coverage_key(6, 18.0),
        // 다른 폰트(같은 glyph id라도 모양이 다름).
        key(
            font(2, 0),
            5,
            18.0,
            VariationCoords::none(),
            GlyphRenderMode::Coverage,
        ),
        // 같은 파일의 다른 face.
        key(
            font(1, 1),
            5,
            18.0,
            VariationCoords::none(),
            GlyphRenderMode::Coverage,
        ),
        // 다른 variation instance.
        key(font(1, 0), 5, 18.0, wght, GlyphRenderMode::Coverage),
        // 다른 rendering mode.
        key(
            font(1, 0),
            5,
            18.0,
            VariationCoords::none(),
            GlyphRenderMode::Color,
        ),
    ];
    for other in &others {
        assert_eq!(
            resolver.resolve(&mut atlas, other),
            GlyphDraw::Empty(EmptyReason::Pending),
            "{other:?}"
        );
    }
    assert_eq!(resolver.take_requests().len(), others.len());
}

#[test]
fn failures_no_ink_and_bad_results_draw_nothing() {
    let mut atlas = atlas(64, 1);
    let mut resolver = GlyphResolver::new();
    atlas.begin_frame(1).unwrap();

    // 공백 glyph: 잉크 없음 → 빈 glyph, 다시 요청하지 않음.
    let space = coverage_key(0, 16.0);
    resolver.resolve(&mut atlas, &space);
    complete_all(&mut resolver, &mut atlas);
    assert_eq!(
        resolver.resolve(&mut atlas, &space),
        GlyphDraw::Empty(EmptyReason::NoInk)
    );
    assert!(resolver.take_requests().is_empty());

    // rasterize 실패 → 빈 glyph(Failed), 재요청 없음. forget_failures 뒤 다시 요청.
    let broken = coverage_key(9, 16.0);
    resolver.resolve(&mut atlas, &broken);
    let keys = resolver.take_requests();
    let error = GlyphError::Rasterize { reason: "손상" };
    assert_eq!(
        resolver.complete(&mut atlas, keys[0].clone(), Err(error)),
        CompleteOutcome::Failed(error)
    );
    assert_eq!(
        resolver.resolve(&mut atlas, &broken),
        GlyphDraw::Empty(EmptyReason::Failed)
    );
    assert!(resolver.take_requests().is_empty());
    resolver.forget_failures();
    resolver.resolve(&mut atlas, &broken);
    assert_eq!(resolver.take_requests(), vec![broken.clone()]);

    // mode와 다른 결과(Coverage key에 outline)는 실패로 처리합니다.
    let wrong = coverage_key(10, 16.0);
    resolver.resolve(&mut atlas, &wrong);
    resolver.take_requests();
    assert!(matches!(
        resolver.complete(
            &mut atlas,
            wrong.clone(),
            Ok(GlyphImage::Outline(outline_for(&wrong)))
        ),
        CompleteOutcome::Failed(_)
    ));
    // page보다 큰 bitmap도 실패(Outline mode 필요).
    let huge = coverage_key(11, 16.0);
    let bitmap = GlyphBitmap::new(
        100,
        100,
        0,
        0,
        GlyphBitmapFormat::Coverage8,
        vec![1; 100 * 100],
    )
    .unwrap();
    assert!(matches!(
        resolver.complete(&mut atlas, huge.clone(), Ok(GlyphImage::Bitmap(bitmap))),
        CompleteOutcome::Failed(_)
    ));
    assert_eq!(
        resolver.resolve(&mut atlas, &huge),
        GlyphDraw::Empty(EmptyReason::Failed)
    );
}

#[test]
fn outline_mode_uses_previous_outline_size() {
    let mut atlas = atlas(64, 1);
    let mut resolver = GlyphResolver::new();
    atlas.begin_frame(1).unwrap();
    let outline = |px: f32| {
        key(
            font(3, 0),
            4,
            px,
            VariationCoords::none(),
            GlyphRenderMode::Outline,
        )
    };
    resolver.resolve(&mut atlas, &outline(100.0));
    complete_all(&mut resolver, &mut atlas);
    assert_eq!(
        resolver.resolve(&mut atlas, &outline(100.0)),
        GlyphDraw::Exact(GlyphSource::Outline(outline_for(&outline(100.0))))
    );
    assert_eq!(
        resolver.resolve(&mut atlas, &outline(120.0)),
        GlyphDraw::Substitute {
            source: GlyphSource::Outline(outline_for(&outline(100.0))),
            from: outline(100.0).size,
            scale: 1.2,
        }
    );
    assert_eq!(resolver.outline_count(), 1);
}

#[test]
fn evicted_sizes_are_not_substituted_and_substitutes_are_protected() {
    // 64 px page, 예산 1장. 16 px glyph(8×8 bitmap)를 frame 1에 배치.
    let mut atlas = atlas(64, 1);
    let mut resolver = GlyphResolver::new();
    atlas.begin_frame(1).unwrap();
    let k16 = coverage_key(5, 16.0);
    resolver.resolve(&mut atlas, &k16);
    complete_all(&mut resolver, &mut atlas);

    // frame 2: 18 px 요청이 16 px로 대체되며, 대체 glyph의 page는 현재 frame 사용으로 보호됩니다.
    atlas.begin_frame(2).unwrap();
    atlas.gpu_completed(1);
    assert!(matches!(
        resolver.resolve(&mut atlas, &coverage_key(5, 18.0)),
        GlyphDraw::Substitute { .. }
    ));
    let filler =
        GlyphBitmap::new(62, 62, 0, 0, GlyphBitmapFormat::Coverage8, vec![1; 62 * 62]).unwrap();
    assert_eq!(
        atlas.insert(coverage_key(50, 16.0), &filler),
        Err(AtlasError::Full)
    );

    // frame 4: 아무도 16 px를 쓰지 않은 뒤 page가 비워지면, 그 크기는 대체로 쓰지 않습니다.
    atlas.begin_frame(4).unwrap();
    atlas.gpu_completed(3);
    atlas.insert(coverage_key(50, 16.0), &filler).unwrap();
    resolver.take_requests();
    assert_eq!(
        resolver.resolve(&mut atlas, &coverage_key(5, 18.0)),
        GlyphDraw::Empty(EmptyReason::Pending)
    );
}

/// 결정적 xorshift64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

#[test]
fn random_workload_never_shows_a_wrong_or_stale_glyph() {
    // 400 frame, rasterize 결과는 무작위로 늦게 도착(일부 실패), 작은 atlas라 eviction·Deferred가 잦습니다.
    // 매 resolve 결과가 (1) Exact면 정확히 그 key, (2) Substitute면 같은 모양·다른 크기 key이며 지금 유효,
    // (3) 그 밖은 빈 glyph인지 검사합니다.
    let mut atlas = atlas(64, 3);
    let mut resolver = GlyphResolver::new();
    let mut rng = Rng(0x0006_0006);
    let wght = VariationCoords::new([(AxisTag::parse("wght").unwrap(), 600.0)]).unwrap();
    let mut universe = Vec::new();
    for f in [font(1, 0), font(2, 0)] {
        for glyph in 0..6u32 {
            for px in [12.0f32, 16.0, 20.0, 28.0] {
                for variations in [VariationCoords::none(), wght.clone()] {
                    for mode in [
                        GlyphRenderMode::Coverage,
                        GlyphRenderMode::Color,
                        GlyphRenderMode::Outline,
                    ] {
                        universe.push(key(f, glyph, px, variations.clone(), mode));
                    }
                }
            }
        }
    }
    // 배치 위치 → 그 위치에 올린 key(테스트 쪽 기록).
    let mut owner: HashMap<(u32, u32, u32, u32), GlyphKey> = HashMap::new();
    let mut queue: Vec<GlyphKey> = Vec::new();
    let mut substitutes = 0u32;
    for frame in 1..=400u64 {
        atlas.begin_frame(frame).unwrap();
        if frame > 2 {
            atlas.gpu_completed(frame - 2);
        }
        for _ in 0..1 + rng.below(12) {
            let wanted = &universe[rng.below(universe.len() as u64) as usize];
            match resolver.resolve(&mut atlas, wanted) {
                GlyphDraw::Exact(GlyphSource::Atlas(p)) => {
                    assert!(atlas.is_valid(&p));
                    assert_eq!(&owner[&(p.page.0, p.generation, p.x, p.y)], wanted);
                }
                GlyphDraw::Exact(GlyphSource::Outline(path)) => {
                    assert_eq!(path, outline_for(wanted));
                }
                GlyphDraw::Substitute {
                    source,
                    from,
                    scale,
                } => {
                    substitutes += 1;
                    assert_ne!(from, wanted.size);
                    assert_eq!(scale, wanted.size.px() / from.px());
                    let source_key = match source {
                        GlyphSource::Atlas(p) => {
                            assert!(atlas.is_valid(&p), "무효 배치 대체");
                            owner[&(p.page.0, p.generation, p.x, p.y)].clone()
                        }
                        GlyphSource::Outline(path) => {
                            let k = GlyphKey {
                                size: from,
                                ..wanted.clone()
                            };
                            assert_eq!(path, outline_for(&k));
                            k
                        }
                    };
                    assert!(source_key.same_glyph_shape(wanted), "다른 glyph 대체");
                    assert_eq!(source_key.size, from);
                }
                GlyphDraw::Empty(reason) => {
                    if wanted.glyph == GlyphId(0) && reason == EmptyReason::NoInk {
                        continue;
                    }
                    assert_ne!(reason, EmptyReason::NoInk, "잉크 있는 glyph가 NoInk");
                }
            }
        }
        queue.extend(resolver.take_requests());
        // 대기 요청 일부만 이번 frame에 완료(나머지는 늦게 도착).
        let mut index = 0;
        while index < queue.len() {
            if rng.below(3) == 0 {
                let done = queue.swap_remove(index);
                let result = if rng.below(20) == 0 {
                    Err(GlyphError::Rasterize {
                        reason: "무작위 실패",
                    })
                } else {
                    Ok(render(&done))
                };
                if let CompleteOutcome::Placed(p) =
                    resolver.complete(&mut atlas, done.clone(), result)
                {
                    owner.insert((p.page.0, p.generation, p.x, p.y), done);
                }
            } else {
                index += 1;
            }
        }
        if frame % 50 == 0 {
            resolver.forget_failures();
        }
    }
    assert!(substitutes > 0);
    let stats = resolver.stats();
    assert!(stats.exact > 0 && stats.empty > 0);
    assert!(atlas.stats().evicted_pages > 0);
}
