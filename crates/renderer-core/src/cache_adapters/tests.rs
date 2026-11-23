//! glyph atlas·tessellation·이미지 mipmap cache 등록 시험 (XVI.5.3).

use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::{ColorSpace, Vec2};

use crate::cache::*;
use crate::cache_adapters::{
    mesh_bytes, BudgetedGlyphAtlas, BudgetedMipmapCache, BudgetedTessellationCache,
};
use crate::glyph::atlas::{AtlasConfig, ShelfGlyphAtlas};
use crate::glyph::{
    FontHash, GlyphAtlas, GlyphBitmap, GlyphBitmapFormat, GlyphId, GlyphKey, GlyphRenderMode,
    GlyphSize, VariationCoords,
};
use crate::image_decode::{DecodeWorkerScope, DecodedImage};
use crate::mipmap::{generate_mip_chain, MipChain};
use crate::tessellation::{
    FillMesh, FillStyle, FillTessellator, TessellationError, TessellationKey,
};
use crate::texture_cache::{
    AssetContentHash, ColorTransform, DecodeSettings, MipmapMode, SourceColorSpace, TextureCacheKey,
};
use crate::thread_role::{bind_thread_role, ThreadRole};

/// subpath anchor를 fan 삼각형으로 나누는 시험 tessellator.
struct Fan;

impl FillTessellator for Fan {
    fn tessellate_fill(
        &self,
        path: &PathGeometry,
        _style: &FillStyle,
    ) -> Result<FillMesh, TessellationError> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for command in path.commands() {
            if let PathCommand::MoveTo(v) | PathCommand::LineTo(v) = *command {
                vertices.push(v);
                let last = vertices.len() as u32 - 1;
                if last >= 2 {
                    indices.extend_from_slice(&[0, last - 1, last]);
                }
            }
        }
        FillMesh::new(vertices, indices)
    }
}

fn p(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

/// 꼭짓점 `n`개 다각형.
fn polygon(n: u32, offset: f32) -> PathGeometry {
    let mut commands = vec![PathCommand::MoveTo(p(offset, 0.0))];
    for i in 1..n {
        commands.push(PathCommand::LineTo(p(offset + i as f32, (i * i) as f32)));
    }
    commands.push(PathCommand::Close);
    PathGeometry::new(commands).unwrap()
}

/// worker thread에서 단색 이미지 mip chain을 만듭니다.
fn mip_chain(width: u32, height: u32) -> MipChain {
    std::thread::spawn(move || {
        let _role = bind_thread_role(ThreadRole::Worker).unwrap();
        let scope = DecodeWorkerScope::current().unwrap();
        let pixels = [10u8, 20, 30, 255].repeat((width * height) as usize);
        let image = DecodedImage::new(width, height, ColorSpace::Srgb, pixels).unwrap();
        generate_mip_chain(&image, MipmapMode::Full, &scope)
    })
    .join()
    .unwrap()
}

fn texture_key(seed: u8) -> TextureCacheKey {
    TextureCacheKey::new(
        AssetContentHash::sha256([seed; 32]),
        DecodeSettings::DEFAULT,
        ColorTransform::new(SourceColorSpace::from_override(None), ColorSpace::Srgb),
    )
}

fn glyph_key(glyph: u32) -> GlyphKey {
    GlyphKey {
        font: FontHash::new([7; 32], 0),
        glyph: GlyphId(glyph),
        size: GlyphSize::from_px(16.0).unwrap(),
        variations: VariationCoords::none(),
        mode: GlyphRenderMode::Coverage,
    }
}

/// page 하나를 가득 채우는 coverage glyph.
fn page_filling_glyph() -> GlyphBitmap {
    GlyphBitmap::new(
        60,
        60,
        0,
        -60,
        GlyphBitmapFormat::Coverage8,
        vec![0xFF; 60 * 60],
    )
    .unwrap()
}

/// 64 px page `pages`장을 쓰는 atlas. 모든 page는 지난 frame에 썼고 GPU가 끝냈습니다(지울 수 있음).
fn atlas_with_pages(pages: u32) -> BudgetedGlyphAtlas {
    let mut atlas = ShelfGlyphAtlas::new(AtlasConfig {
        page_size: 64,
        max_pages: 8,
        padding: 1,
    })
    .unwrap();
    for frame in 1..=u64::from(pages) {
        atlas.begin_frame(frame).unwrap();
        atlas
            .insert(glyph_key(frame as u32), &page_filling_glyph())
            .unwrap();
    }
    atlas.gpu_completed(u64::from(pages));
    atlas.begin_frame(u64::from(pages) + 1).unwrap();
    assert_eq!(atlas.page_count(), pages);
    BudgetedGlyphAtlas::new(atlas)
}

/// 시험용 썸네일 cache: 같은 크기 entry를 LRU로 지웁니다.
struct Thumbnails {
    entries: Vec<u64>,
}

impl BudgetedCache for Thumbnails {
    fn kind(&self) -> CacheKind {
        CacheKind::Thumbnail
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage::of(
            CachePriority::BackgroundThumbnail,
            self.entries.iter().sum(),
        )
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        if priority != CachePriority::BackgroundThumbnail || self.entries.is_empty() {
            return 0;
        }
        self.entries.remove(0)
    }
}

/// 문서·scene, 썸네일, mipmap(보이는 것·가까운 것), tessellation(보이는 것·가까운 것), glyph atlas를 등록한 manager.
pub(crate) fn fixture() -> (CacheBudgetManager, [CacheId; 6]) {
    let mut manager = CacheBudgetManager::new();
    let document = manager.register(Box::new(ReportedUsage::new(
        CacheKind::DocumentModel,
        CachePriority::DocumentScene,
        40_000,
    )));
    let scene = manager.register(Box::new(ReportedUsage::new(
        CacheKind::SceneSnapshot,
        CachePriority::DocumentScene,
        20_000,
    )));
    // glyph atlas를 tessellation보다 먼저 등록해도 같은 priority 안에서는 종류 순서(§10.4)가 앞섭니다.
    let glyphs = manager.register(Box::new(atlas_with_pages(3)));
    let mut tessellation = BudgetedTessellationCache::new();
    for (i, priority) in [
        CachePriority::VisibleGlyphGeometry,
        CachePriority::NearViewport,
        CachePriority::VisibleGlyphGeometry,
        CachePriority::NearViewport,
    ]
    .into_iter()
    .enumerate()
    {
        tessellation
            .fill(
                &polygon(6 + i as u32, i as f32 * 100.0),
                FillStyle::default(),
                &Fan,
                priority,
            )
            .unwrap();
    }
    let tessellation = manager.register(Box::new(tessellation));
    let mut mipmaps = BudgetedMipmapCache::new();
    for (i, priority) in [
        CachePriority::NearViewport,
        CachePriority::VisibleGlyphGeometry,
    ]
    .into_iter()
    .enumerate()
    {
        mipmaps
            .get_or_create(texture_key(i as u8), priority, |_| {
                Ok::<_, ()>(mip_chain(8 + i as u32, 8))
            })
            .unwrap();
    }
    let mipmaps = manager.register(Box::new(mipmaps));
    let thumbnails = manager.register(Box::new(Thumbnails {
        entries: vec![1_000, 1_000, 1_000],
    }));
    (
        manager,
        [document, scene, glyphs, tessellation, mipmaps, thumbnails],
    )
}

#[test]
fn registered_caches_follow_the_eviction_order() {
    // 실제 cache를 등록해도 XVI.5.2 순서(낮은 priority, §10.4 종류 순서)로 지우고 Document/CurrentScene은 남깁니다.
    let (mut manager, [document, scene, ..]) = fixture();
    let mut pass = EvictionPass::new(0);
    let outcome = manager.evict_chunk(&mut pass, &mut Unbounded);
    let order: Vec<(CachePriority, CacheKind)> = outcome
        .evicted
        .iter()
        .map(|eviction| (eviction.priority, eviction.kind))
        .collect();
    use CacheKind as K;
    use CachePriority as P;
    assert_eq!(
        order,
        vec![
            (P::BackgroundThumbnail, K::Thumbnail),
            (P::BackgroundThumbnail, K::Thumbnail),
            (P::BackgroundThumbnail, K::Thumbnail),
            (P::NearViewport, K::ImageMipmap),
            (P::NearViewport, K::Tessellation),
            (P::NearViewport, K::Tessellation),
            (P::VisibleGlyphGeometry, K::ImageMipmap),
            (P::VisibleGlyphGeometry, K::Tessellation),
            (P::VisibleGlyphGeometry, K::Tessellation),
            // atlas는 마지막 page 하나를 남깁니다(XI.6.5 page 예산 하한 1).
            (P::VisibleGlyphGeometry, K::GlyphAtlas),
            (P::VisibleGlyphGeometry, K::GlyphAtlas),
        ]
    );
    assert!(outcome.finished && outcome.over_target);
    let usage = manager.usage();
    assert_eq!(
        usage.get(P::DocumentScene),
        60_000,
        "Document/CurrentScene 비축출"
    );
    assert_eq!(usage.get(P::BackgroundThumbnail), 0);
    assert_eq!(usage.get(P::NearViewport), 0);
    assert_eq!(
        usage.get(P::VisibleGlyphGeometry),
        64 * 64,
        "atlas page 하나"
    );
    assert_eq!(
        manager.usage_of(document),
        Some(CacheUsage::of(P::DocumentScene, 40_000))
    );
    assert_eq!(
        manager.usage_of(scene),
        Some(CacheUsage::of(P::DocumentScene, 20_000))
    );
}

#[test]
fn eviction_stops_at_target_and_keeps_lru_order_within_a_cache() {
    let mut manager = CacheBudgetManager::new();
    let mut tessellation = BudgetedTessellationCache::new();
    let paths: Vec<PathGeometry> = (0..4).map(|i| polygon(8, i as f32 * 50.0)).collect();
    for path in &paths {
        tessellation
            .fill(
                path,
                FillStyle::default(),
                &Fan,
                CachePriority::VisibleGlyphGeometry,
            )
            .unwrap();
    }
    // 첫 path를 다시 써서 가장 최근 사용으로 만듭니다.
    let hit = tessellation
        .fill(
            &paths[0],
            FillStyle::default(),
            &Fan,
            CachePriority::VisibleGlyphGeometry,
        )
        .unwrap();
    let one = mesh_bytes(&hit.mesh);
    let id = manager.register(Box::new(tessellation));
    assert_eq!(manager.usage().total(), one * 4);
    let mut pass = EvictionPass::new(one * 2);
    let outcome = manager.evict_chunk(&mut pass, &mut Unbounded);
    assert_eq!(outcome.evicted.len(), 2);
    assert!(outcome.finished && !outcome.over_target);
    assert_eq!(manager.usage().total(), one * 2);
    let cache = manager.get_mut::<BudgetedTessellationCache>(id).unwrap();
    // 남은 것은 최근에 쓴 path 0과 마지막에 만든 path 3입니다.
    for (i, kept) in [(0, true), (1, false), (2, false), (3, true)] {
        let key = TessellationKey::fill(&paths[i], FillStyle::default());
        assert_eq!(cache.remove(&key).is_some(), kept, "path {i}");
    }
}

#[test]
fn total_usage_matches_registered_caches() {
    // 검증 기준(XVI.5.3): glyph·tessellation·이미지 mipmap·썸네일 cache가 등록한 usage 합계가 각 cache가 실제로 든
    // resource byte와 일치합니다.
    let (mut manager, [_, _, glyphs, tessellation, mipmaps, thumbnails]) = fixture();
    let glyph_bytes = manager
        .get::<BudgetedGlyphAtlas>(glyphs)
        .unwrap()
        .atlas()
        .memory_bytes();
    assert_eq!(glyph_bytes, 3 * 64 * 64);
    let mesh_total: u64 = (0..4u32)
        .map(|i| {
            let mesh = Fan
                .tessellate_fill(&polygon(6 + i, i as f32 * 100.0), &FillStyle::default())
                .unwrap();
            mesh_bytes(&mesh)
        })
        .sum();
    let mip_total = mip_chain(8, 8).total_bytes() + mip_chain(9, 8).total_bytes();
    let expected = 60_000 + glyph_bytes + mesh_total + mip_total + 3_000;
    assert_eq!(manager.usage().total(), expected);
    let per_cache: u64 = [glyphs, tessellation, mipmaps, thumbnails]
        .iter()
        .map(|id| manager.usage_of(*id).unwrap().total())
        .sum::<u64>()
        + 60_000;
    assert_eq!(per_cache, expected);
    // 사용량은 cache 변경을 그대로 따라갑니다: priority 이동, 삭제, 새 entry.
    let mip = manager.get_mut::<BudgetedMipmapCache>(mipmaps).unwrap();
    assert!(mip.set_priority(&texture_key(0), CachePriority::VisibleGlyphGeometry));
    mip.remove(&texture_key(1)).unwrap();
    let usage = manager.usage_of(mipmaps).unwrap();
    assert_eq!(usage.get(CachePriority::NearViewport), 0);
    assert_eq!(
        usage.get(CachePriority::VisibleGlyphGeometry),
        mip_chain(8, 8).total_bytes()
    );
    assert_eq!(
        manager.usage().total(),
        expected - mip_chain(9, 8).total_bytes()
    );
}
