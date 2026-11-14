use super::*;
use stellar_assets::{
    catalog::AssetCatalog, id::AssetId, policy::AssetImportPolicy, store::AssetStore,
};
use stellar_core_types::text::UserText;
use stellar_document::text::{Paragraph, TextPayload};
use stellar_project::fonts::{
    license::FontLicenseInfo,
    project_font::{import_project_font, ProjectFaceError, ProjectFontRecord},
};
use stellar_renderer_core::glyph::{
    AtlasConfig, EmptyReason, GlyphAtlas, GlyphDraw, GlyphResolver, GlyphSource, ShelfGlyphAtlas,
};
use stellar_text::{
    bidi::BaseDirection,
    cache::TargetProfileKey,
    caret::hit_test,
    fallback::FontFallbackChain,
    font::{FontFaceRef, FontSource},
    font_runs::{LoadedFaces, ShapingStyle},
    incremental::{IncrementalTextLayout, LayoutRequest, ParagraphSpec},
    itemize::StyleSpan,
    line_break::LineBreakOptions,
    paragraph::TextStyleSpec,
    registry::InMemoryFontRegistry,
    shaping::LanguageTag,
    text_layout::TextBox,
};

#[test]
fn project_font_content_to_positioned_outlines_and_line_hits() {
    // 고정 fixture가 저장한 본문과 font record에서 시작합니다. style의 모든 값은 명시 입력입니다.
    let dir = std::env::temp_dir().join(format!(
        "stellar-text-outlines-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("test-font.ttf");
    std::fs::write(&file, stellar_text::test_support::outline_test_font()).unwrap();
    let policy =
        AssetImportPolicy::from_config_str(include_str!("../../../../../config/asset-import.yaml"))
            .unwrap();
    let store = AssetStore::new(&dir);
    let imported = import_project_font(
        &store,
        &policy,
        AssetId::from_bytes([1; 16]),
        &file,
        FontLicenseInfo::default(),
    )
    .unwrap();
    let object = store.content_path(&imported.asset.object_path);
    std::fs::remove_file(&file).unwrap();
    let font = ProjectFontRecord::decode(&imported.font.encode().unwrap()).unwrap();
    let reference = font.face_ref(0).unwrap();
    let mut catalog = AssetCatalog::new();
    catalog
        .insert(font.asset_id(), imported.asset.record.clone())
        .unwrap();
    let face = font
        .load_face(&reference, &catalog, &store, &policy)
        .unwrap();
    let mut registry = InMemoryFontRegistry::new();
    let handle = registry
        .register(FontSource::Project, FontFaceKey::from(&reference))
        .unwrap();
    let mut faces = LoadedFaces::new();
    faces.insert(handle, face);
    let chain = FontFallbackChain::new(vec![reference.clone()]).unwrap();
    let payload = TextPayload::new(
        UserText::new("A A\nA"),
        vec![Paragraph { style: 0 }; 2],
        vec![],
    )
    .unwrap();
    let payload = TextPayload::decode(&payload.encode()).unwrap();
    let lines: Vec<_> = payload.content().split('\n').collect();
    let spans: Vec<_> = lines
        .iter()
        .map(|line| StyleSpan {
            range: 0..line.len(),
            style: 0,
            language: None,
        })
        .collect();
    let paragraphs: Vec<_> = lines
        .iter()
        .zip(&spans)
        .map(|(line, span)| ParagraphSpec {
            text: line,
            spans: std::slice::from_ref(span),
            base_direction: BaseDirection::Auto,
            style: Default::default(),
        })
        .collect();
    let styles = [TextStyleSpec {
        chain: chain.resolve(&registry),
        shaping: ShapingStyle::default(),
        font_size: FontSize::from_px(20).unwrap(),
    }];
    let request = LayoutRequest {
        paragraphs: &paragraphs,
        styles: &styles,
        line_break: &LineBreakOptions::new(LanguageTag::parse("en").unwrap()),
        target_profile: TargetProfileKey(1),
        max_width: None,
        paragraph_spacing: TextUnit::from_px(3).unwrap(),
        text_box: TextBox::default(),
    };
    let mut engine = IncrementalTextLayout::new();
    let (layout, _) = engine.layout(&request, &faces).unwrap();
    let layout_key = TextLayoutKey::from_bytes([4; 16]);
    let geometry = CanvasTextGeometry::from_layout(layout_key, &layout).unwrap();
    let glyphs = &geometry.glyphs;
    assert_eq!(glyphs.len(), 4);
    assert_eq!(glyphs[0].key, glyphs[2].key);
    assert_eq!(
        glyphs[0].key.font.file_sha256(),
        reference.file_hash().sha256_digest()
    );
    assert_eq!(glyphs[0].origin, Vec2::new(0., 16.).unwrap());
    assert_eq!(glyphs[2].origin, Vec2::new(17., 16.).unwrap());
    assert_eq!(glyphs[3].origin, Vec2::new(0., 39.).unwrap());
    let rects = geometry.line_rects(layout_key).unwrap();
    assert_eq!(rects[0].size().width(), 29.);
    assert_eq!(rects[1].origin().y(), 23.);
    assert!(geometry
        .line_rects(TextLayoutKey::from_bytes([2; 16]))
        .is_none());
    let mut builder = stellar_scene::builder::SceneBuilder::new();
    let node = builder
        .build_text(
            &mut builder.root_scope(),
            &stellar_scene::builder::TextInput {
                node: stellar_scene::builder::NodeInput::new(
                    stellar_core_types::ids::StableId::new_v7(1, 0, 1),
                    stellar_core_types::value::Transform2D::identity(),
                ),
                layout: layout_key,
                size: Size2::new(px(layout.width), px(layout.height)).unwrap(),
            },
        )
        .unwrap();
    let scene_hit = |x, y| {
        stellar_scene::narrow_phase::content_contains(
            &node,
            Vec2::new(x, y).unwrap(),
            &stellar_scene::narrow_phase::NarrowPhaseOptions::EXACT,
            &geometry,
        )
    };
    assert!(scene_hit(18., 10.));
    assert!(!scene_hit(18., 21.), "두 줄 사이");
    assert!(!scene_hit(18., 30.), "두 번째 짧은 줄의 바깥");
    let hit = hit_test(
        &layout,
        TextUnit::from_px(18).unwrap(),
        TextUnit::from_px(10).unwrap(),
    )
    .unwrap();
    assert_eq!(hit.cluster, Some(2..3));
    assert!(hit.inside);
    let (_, report) = engine.layout(&request, &faces).unwrap();
    assert!(report.reshaped.is_empty());

    let rasterizer = TextOutlineRasterizer {
        registry: &registry,
        faces: &faces,
    };
    let mut atlas = ShelfGlyphAtlas::new(AtlasConfig::default()).unwrap();
    atlas.begin_frame(1).unwrap();
    let mut resolver = GlyphResolver::new();
    for g in glyphs {
        resolver.resolve(&mut atlas, &g.key);
    }
    let requests = resolver.take_requests();
    assert_eq!(requests.len(), 2, "A의 outline은 한 번만 요청");
    for key in requests {
        let image = rasterizer.rasterize(&key);
        resolver.complete(&mut atlas, key, image);
    }
    let GlyphDraw::Exact(GlyphSource::Outline(path)) = resolver.resolve(&mut atlas, &glyphs[0].key)
    else {
        panic!("A의 실제 outline 필요")
    };
    assert!(matches!(path.commands().first(), Some(PathCommand::MoveTo(p)) if *p == Vec2::ZERO));
    assert!(path.commands().iter().any(|cmd| matches!(cmd,PathCommand::CubicTo{c1,c2,to} if c1.y() < -6.6 && c2.y() < -6.6 && to.x() == 10.)));
    assert_eq!(
        resolver.resolve(&mut atlas, &glyphs[1].key),
        GlyphDraw::Empty(EmptyReason::NoInk)
    );
    let mut unknown = glyphs[0].key.clone();
    unknown.glyph = GlyphId(900);
    assert!(rasterizer.rasterize(&unknown).is_err());
    unknown.font = FontHash::new([0; 32], 0);
    assert!(rasterizer.rasterize(&unknown).is_err());

    let foreign = FontFaceRef::new(FontSource::System, reference.file_hash(), 0, vec![]);
    assert!(matches!(
        font.load_face(&foreign, &catalog, &store, &policy),
        Err(ProjectFaceError::ReferenceMismatch)
    ));
    let size = imported.asset.record.size;
    assert!(store
        .read_content(&imported.asset.record.content_hash, size, size - 1)
        .is_err());
    std::fs::write(object, vec![0u8; size as usize]).unwrap();
    assert!(matches!(
        font.load_face(&reference, &catalog, &store, &policy),
        Err(ProjectFaceError::MissingAsset)
    ));
    std::fs::remove_dir_all(dir).unwrap();
}
