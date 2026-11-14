//! glyph atlas 할당·eviction 테스트 (XI.6.5).

use super::*;
use crate::frame_resources::UploadQueue;
use crate::glyph::tests::{font, key};
use crate::glyph::{GlyphRenderMode, VariationCoords};
use crate::gpu_handle::{HandleTable, TextureKind};

fn coverage_key(glyph: u32) -> GlyphKey {
    key(
        font(1, 0),
        glyph,
        16.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    )
}

fn color_key(glyph: u32) -> GlyphKey {
    key(
        font(1, 0),
        glyph,
        16.0,
        VariationCoords::none(),
        GlyphRenderMode::Color,
    )
}

fn coverage(w: u32, h: u32) -> GlyphBitmap {
    GlyphBitmap::new(
        w,
        h,
        0,
        -(h as i32),
        GlyphBitmapFormat::Coverage8,
        vec![0xFF; (w * h) as usize],
    )
    .unwrap()
}

fn color(w: u32, h: u32) -> GlyphBitmap {
    GlyphBitmap::new(
        w,
        h,
        0,
        0,
        GlyphBitmapFormat::Rgba8Premultiplied,
        vec![0x80; (w * h * 4) as usize],
    )
    .unwrap()
}

fn atlas(page_size: u32, max_pages: u32) -> ShelfGlyphAtlas {
    ShelfGlyphAtlas::new(AtlasConfig {
        page_size,
        max_pages,
        padding: 1,
    })
    .unwrap()
}

/// 같은 page의 cell(gutter 포함)이 겹치지 않고 page 안에 있는지 검사합니다.
fn assert_disjoint(placements: &[AtlasPlacement], page_size: u32, pad: u32) {
    for (i, a) in placements.iter().enumerate() {
        assert!(a.x >= pad && a.y >= pad);
        assert!(a.x + a.width + pad <= page_size && a.y + a.height + pad <= page_size);
        for b in &placements[i + 1..] {
            if a.page != b.page || a.generation != b.generation {
                continue;
            }
            let separated = a.x + a.width + pad <= b.x - pad
                || b.x + b.width + pad <= a.x - pad
                || a.y + a.height + pad <= b.y - pad
                || b.y + b.height + pad <= a.y - pad;
            assert!(separated, "겹침: {a:?} {b:?}");
        }
    }
}

#[test]
fn shelf_allocation_is_disjoint_and_deterministic() {
    let run = || {
        let mut atlas = atlas(64, 4);
        atlas.begin_frame(1).unwrap();
        let mut placements = Vec::new();
        for glyph in 0..40u32 {
            let (w, h) = (4 + glyph % 7, 5 + glyph % 9);
            placements.push(atlas.insert(coverage_key(glyph), &coverage(w, h)).unwrap());
        }
        (atlas, placements)
    };
    let (atlas, placements) = run();
    assert_disjoint(&placements, 64, 1);
    // 같은 입력 순서 → 같은 배치.
    assert_eq!(run().1, placements);
    // 첫 glyph는 gutter 1 px 안쪽 (1, 1).
    assert_eq!((placements[0].x, placements[0].y), (1, 1));
    assert!(
        atlas.page_count() >= 2,
        "64 px page 하나에 다 들어가지 않습니다"
    );
    assert_eq!(atlas.stats().glyphs, 40);
    // 이미 있는 key는 기존 배치를 돌려줍니다.
    let mut atlas = atlas;
    assert_eq!(
        atlas.insert(coverage_key(0), &coverage(4, 5)).unwrap(),
        placements[0]
    );
    assert_eq!(atlas.stats().inserted, 40);
}

#[test]
fn formats_use_separate_pages_and_validate() {
    let mut atlas = atlas(64, 4);
    atlas.begin_frame(1).unwrap();
    let a = atlas.insert(coverage_key(1), &coverage(8, 8)).unwrap();
    let b = atlas.insert(color_key(1), &color(8, 8)).unwrap();
    assert_ne!(a.page, b.page);
    assert_eq!(
        atlas.page_format(a.page),
        Some(GlyphBitmapFormat::Coverage8)
    );
    assert_eq!(
        atlas.page_format(b.page),
        Some(GlyphBitmapFormat::Rgba8Premultiplied)
    );
    assert_eq!(atlas.memory_bytes(), 64 * 64 + 64 * 64 * 4);
    // mode와 다른 format bitmap, page보다 큰 bitmap, 잘못된 설정은 오류.
    assert_eq!(
        atlas.insert(coverage_key(2), &color(4, 4)),
        Err(AtlasError::FormatMismatch)
    );
    assert_eq!(
        atlas.insert(coverage_key(3), &coverage(63, 10)),
        Err(AtlasError::TooLarge {
            width: 63,
            height: 10
        })
    );
    for bad in [
        AtlasConfig {
            page_size: 32,
            ..AtlasConfig::DEFAULT
        },
        AtlasConfig {
            max_pages: 0,
            ..AtlasConfig::DEFAULT
        },
        AtlasConfig {
            padding: 9,
            ..AtlasConfig::DEFAULT
        },
    ] {
        assert!(ShelfGlyphAtlas::new(bad).is_err());
    }
    // frame 번호는 엄격히 증가.
    assert_eq!(atlas.begin_frame(1), Err(AtlasError::FrameOrder));
}

/// page 하나를 가득 채우는 glyph 크기(64 px page, gutter 1 → cell 62 + 2 = 64).
fn page_filler() -> GlyphBitmap {
    coverage(62, 62)
}

#[test]
fn lru_page_is_evicted_and_placements_invalidated() {
    let mut atlas = atlas(64, 2);
    atlas.begin_frame(1).unwrap();
    let a = atlas.insert(coverage_key(1), &page_filler()).unwrap();
    atlas.begin_frame(2).unwrap();
    let b = atlas.insert(coverage_key(2), &page_filler()).unwrap();
    atlas.begin_frame(3).unwrap();
    atlas.gpu_completed(2);
    // 예산 2장이 찼으므로 가장 오래 쓰지 않은 page(a, frame 1)를 비웁니다.
    let c = atlas.insert(coverage_key(3), &page_filler()).unwrap();
    assert_eq!(c.page, a.page);
    assert!(!atlas.is_valid(&a));
    assert!(atlas.is_valid(&b) && atlas.is_valid(&c));
    assert!(atlas.lookup(&coverage_key(1)).is_none());
    assert_eq!(atlas.lookup(&coverage_key(2)), Some(b));
    let stats = atlas.stats();
    assert_eq!((stats.evicted_pages, stats.evicted_glyphs), (1, 1));
    assert_eq!(stats.pages, 2);
}

#[test]
fn reused_slot_bumps_generation_and_can_change_format() {
    // 리뷰 반영(XI.6.5): 비운 slot이 다른 format page로 다시 쓰이면 세대가 바뀌므로, renderer는
    // `(page, generation)`으로 page texture를 관리해 다시 만들 시점을 알 수 있습니다(모듈 문서).
    let mut atlas = atlas(64, 1);
    atlas.begin_frame(1).unwrap();
    let old = atlas.insert(coverage_key(1), &page_filler()).unwrap();
    assert_eq!(
        atlas.page_texture_desc(old.format).format,
        TextureFormat::R8Unorm
    );
    atlas.begin_frame(2).unwrap();
    atlas.gpu_completed(1);
    let new = atlas.insert(color_key(1), &color(8, 8)).unwrap();
    assert_eq!(new.page, old.page, "예산 1장이라 같은 slot을 다시 씁니다");
    assert_ne!(new.generation, old.generation);
    assert_eq!(new.format, GlyphBitmapFormat::Rgba8Premultiplied);
    assert_eq!(
        atlas.page_format(new.page),
        Some(GlyphBitmapFormat::Rgba8Premultiplied)
    );
    assert!(!atlas.is_valid(&old) && atlas.is_valid(&new));
    // color page texture는 mip chain과 같은 sRGB texture입니다(pixel 규약 공유).
    assert_eq!(
        atlas.page_texture_desc(new.format).format,
        TextureFormat::Rgba8UnormSrgb
    );
}

#[test]
fn current_frame_and_in_flight_pages_are_never_evicted() {
    let mut atlas = atlas(64, 2);
    atlas.begin_frame(1).unwrap();
    let a = atlas.insert(coverage_key(1), &page_filler()).unwrap();
    let b = atlas.insert(coverage_key(2), &page_filler()).unwrap();
    // 두 page 모두 현재 frame에서 썼으므로 비울 수 없습니다.
    assert_eq!(
        atlas.insert(coverage_key(3), &page_filler()),
        Err(AtlasError::Full)
    );
    assert!(atlas.is_valid(&a) && atlas.is_valid(&b));

    // frame 2: a만 다시 씀. b는 frame 1에서 썼고 GPU가 frame 1을 아직 끝내지 않았습니다.
    atlas.begin_frame(2).unwrap();
    assert_eq!(atlas.lookup(&coverage_key(1)), Some(a));
    assert_eq!(
        atlas.insert(coverage_key(3), &page_filler()),
        Err(AtlasError::Full)
    );
    // GPU가 frame 1을 끝내면 b를 비울 수 있습니다(a는 현재 frame 사용이라 보호).
    atlas.gpu_completed(1);
    let c = atlas.insert(coverage_key(3), &page_filler()).unwrap();
    assert_eq!(c.page, b.page);
    assert!(atlas.is_valid(&a));
    assert!(!atlas.is_valid(&b));
    assert_eq!(atlas.stats().full, 2);
}

#[test]
fn memory_pressure_keeps_protected_pages_then_converges() {
    let mut atlas = atlas(64, 4);
    atlas.begin_frame(1).unwrap();
    let placements: Vec<_> = (1..=4)
        .map(|glyph| atlas.insert(coverage_key(glyph), &page_filler()).unwrap())
        .collect();
    atlas.begin_frame(2).unwrap();
    atlas.gpu_completed(1);
    // 현재 frame이 glyph 2·3을 씁니다.
    atlas.lookup(&coverage_key(2)).unwrap();
    atlas.lookup(&coverage_key(3)).unwrap();
    // 압박: 예산 1장. 보호되지 않은 page 1·4만 없애고 2·3은 남깁니다.
    let outcome = atlas.apply_memory_pressure(1);
    assert_eq!(
        outcome,
        TrimOutcome {
            evicted_pages: 2,
            remaining_pages: 2,
            protected_over_budget: 1,
        }
    );
    assert!(atlas.is_valid(&placements[1]) && atlas.is_valid(&placements[2]));
    assert!(!atlas.is_valid(&placements[0]) && !atlas.is_valid(&placements[3]));
    // 다음 frame에서 3만 쓰고 GPU가 frame 2를 끝내면 예산 1로 수렴합니다.
    atlas.begin_frame(3).unwrap();
    atlas.gpu_completed(2);
    atlas.lookup(&coverage_key(3)).unwrap();
    let outcome = atlas.apply_memory_pressure(1);
    assert_eq!(outcome.remaining_pages, 1);
    assert_eq!(outcome.protected_over_budget, 0);
    assert!(atlas.is_valid(&placements[2]));
    // 예산을 풀면 다시 page를 늘립니다.
    atlas.apply_memory_pressure(4);
    assert!(atlas.insert(coverage_key(9), &page_filler()).is_ok());
    assert_eq!(atlas.page_count(), 2);
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
fn memory_pressure_simulation_never_evicts_current_frame_glyphs() {
    // 검증 기준(XI.6.5): memory pressure simulation. 400 frame, GPU는 2 frame 늦게 끝내고, 예산이 수시로
    // 줄었다 늘어납니다. 매 frame이 받은 placement는 frame 끝까지 모두 유효해야 합니다.
    let mut atlas = atlas(128, 6);
    let mut rng = Rng(0x0006_0005);
    let mut full = 0u32;
    for frame in 1..=400u64 {
        atlas.begin_frame(frame).unwrap();
        if frame > 2 {
            atlas.gpu_completed(frame - 2);
        }
        let mut used: Vec<AtlasPlacement> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1 + rng.below(24) {
            // 작은 working set(자주 hit) + 가끔 새 glyph.
            let glyph = if rng.below(4) == 0 {
                1000 + rng.below(400) as u32
            } else {
                rng.below(40) as u32
            };
            let k = coverage_key(glyph);
            let placement = match atlas.lookup(&k) {
                Some(placement) => Some(placement),
                None => {
                    let side = 6 + (glyph % 23);
                    match atlas.insert(k, &coverage(side, side + glyph % 5)) {
                        Ok(placement) => Some(placement),
                        Err(AtlasError::Full) => {
                            full += 1;
                            None
                        }
                        Err(other) => panic!("예상하지 못한 오류: {other}"),
                    }
                }
            };
            if let Some(placement) = placement {
                // 같은 glyph의 반복 조회는 한 번만 기록합니다(다른 glyph의 같은 위치는 겹침으로 잡힘).
                if seen.insert(glyph) {
                    used.push(placement);
                } else {
                    assert!(used.contains(&placement));
                }
            }
            if rng.below(10) == 0 {
                let budget = 1 + rng.below(6) as u32;
                let outcome = atlas.apply_memory_pressure(budget);
                assert!(outcome.remaining_pages <= budget + outcome.protected_over_budget);
            }
            // 현재 frame이 받은 placement는 eviction·압박에도 유효합니다.
            for placement in &used {
                assert!(
                    atlas.is_valid(placement),
                    "frame {frame}: {placement:?} 무효"
                );
            }
        }
        assert_disjoint(&used, 128, 1);
    }
    let stats = atlas.stats();
    assert!(stats.evicted_pages > 0);
    assert!(stats.hits > 0 && stats.inserted > 0);
    let _ = full;
}

#[test]
fn upload_request_fills_gutter_with_zeros() {
    let mut atlas = atlas(64, 1);
    atlas.begin_frame(1).unwrap();
    let bitmap = coverage(3, 2);
    let placement = atlas.insert(coverage_key(1), &bitmap).unwrap();
    let mut textures: HandleTable<TextureKind, ()> = HandleTable::new(0);
    let texture = textures.insert(()).unwrap();
    let request = atlas.upload_request(texture, &placement, &bitmap).unwrap();
    let UploadTarget::Texture { region, format, .. } = request.target else {
        panic!("texture upload");
    };
    assert_eq!(format, TextureFormat::R8Unorm);
    assert_eq!(
        (region.x, region.y, region.width, region.height),
        (0, 0, 5, 4)
    );
    // 5×4 cell: 가운데 3×2만 bitmap, 나머지는 0.
    #[rustfmt::skip]
    let expected: Vec<u8> = vec![
        0, 0,    0,    0,    0,
        0, 0xFF, 0xFF, 0xFF, 0,
        0, 0xFF, 0xFF, 0xFF, 0,
        0, 0,    0,    0,    0,
    ];
    assert_eq!(&request.data[..], &expected[..]);
    // upload queue 검증(길이 = 영역 × bpp)을 통과합니다.
    let mut queue = UploadQueue::new();
    queue.push(request).unwrap();
    // 다른 bitmap으로 만든 요청은 거부합니다.
    assert_eq!(
        atlas.upload_request(texture, &placement, &coverage(2, 2)),
        Err(AtlasError::FormatMismatch)
    );
    let desc = atlas.page_texture_desc(GlyphBitmapFormat::Rgba8Premultiplied);
    assert_eq!(desc.format, TextureFormat::Rgba8UnormSrgb);
    assert_eq!((desc.width, desc.height, desc.mip_levels), (64, 64, 1));
}
