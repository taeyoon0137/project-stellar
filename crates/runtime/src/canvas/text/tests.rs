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

#[test]
fn document_selection_projects_multiline_scalar_offsets_and_verifies_changed_registration() {
    use crate::window_session::test_fixtures::{sample_document, sid, TempDir};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_project::fonts::document::{import_default_font, DocumentFontSelection};
    let project = TempDir::new("document-font");
    let data = TempDir::new("document-font-origin");
    let file = data.path().join("chosen.ttf");
    std::fs::write(&file, stellar_text::test_support::outline_test_font()).unwrap();
    let mut model = sample_document();
    let fill = Color::new(ColorSpace::DisplayP3, 0.25, 0.5, 1., 0.5).unwrap();
    let mut input = DocumentFontSelection {
        file,
        index: 1,
        size: 20 * 64,
        fill,
    };
    let ids = [sid(401), sid(402)];
    let current = &model.definition().text_styles;
    assert!(import_default_font(project.path(), data.path(), current, ids, &input).is_err());
    assert!(!project.path().join("assets").exists());
    input.index = 0;
    let imported = import_default_font(project.path(), data.path(), current, ids, &input).unwrap();
    assert_eq!(imported.styles.fonts.len(), 1);
    let hash = imported.reference.file_hash();
    model.replace_text_styles(imported.styles);
    let payload = TextPayload::new(
        UserText::new("A A\nA"),
        vec![Paragraph { style: 0 }; 2],
        vec![],
    )
    .unwrap();
    let mut cache = document::FontCache::default();
    cache.sync_registrations(&model.definition().text_styles);
    let project_text = |cache: &mut document::FontCache, model: &_| {
        cache.layout(model, project.path(), sid(502), &payload)
    };
    let (text, _, _) = project_text(&mut cache, &model).unwrap();
    assert_eq!(text.fill, fill);
    assert_eq!(text.glyphs.len(), 3);
    assert_eq!(text.lines.len(), 2);
    assert_eq!(text.hit(1., 22.).unwrap().0, 4);
    assert_eq!(text.caret_rect(4, false).unwrap().origin().y(), 20.);
    assert!(text.caret_rect(6, false).is_none());
    assert_eq!(text.selection_rects(1, 5).unwrap().len(), 2);
    let empty = TextPayload::new(UserText::new(""), vec![Paragraph { style: 0 }], vec![]).unwrap();
    let (empty_text, _, _) = cache
        .layout(&model, project.path(), sid(503), &empty)
        .unwrap();
    assert!(empty_text.glyphs.is_empty());
    assert!(empty_text.caret_rect(0, false).unwrap().size().height() > 0.);
    // 뒤의 run이 같은 범위를 완전히 덮습니다. 가려진 미해결 style은 결과에 영향을 주지 않습니다.
    use stellar_document::text::{StyleRun, TextRange};
    let overlaid = TextPayload::new(
        UserText::new("AA"),
        vec![Paragraph { style: 0 }],
        vec![
            StyleRun {
                range: TextRange::new(0, 2).unwrap(),
                style: 23,
            },
            StyleRun {
                range: TextRange::new(0, 2).unwrap(),
                style: 0,
            },
        ],
    )
    .unwrap();
    assert_eq!(
        cache
            .layout(&model, project.path(), sid(502), &overlaid)
            .unwrap()
            .0
            .glyphs
            .len(),
        2
    );
    let unicode = TextPayload::new(
        UserText::new("é\nA"),
        vec![Paragraph { style: 0 }; 2],
        vec![],
    )
    .unwrap();
    let unicode = cache
        .layout(&model, project.path(), sid(502), &unicode)
        .unwrap()
        .0;
    assert_eq!(unicode.hit(0., 21.).unwrap().0, 2);
    assert!(unicode.caret_rect(3, false).is_some());
    let mut changed = model.definition().text_styles.clone();
    changed.fonts.get_mut(&hash).unwrap().asset_record.clear();
    model.replace_text_styles(changed);
    cache.sync_registrations(&model.definition().text_styles);
    assert!(project_text(&mut cache, &model).is_none());
}

#[test]
fn saved_layout_reuses_only_matching_content_modifiers_styles_and_visible_nodes() {
    use crate::window_session::test_fixtures::{sample_document, sid, TempDir};
    use std::sync::Arc;
    use stellar_core_types::{
        color::{Color, ColorSpace},
        order_key::OrderKey,
        value::Transform2D,
    };
    use stellar_document::{
        modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack},
        records::{NodePayload, NodeRecord},
    };
    use stellar_project::fonts::document::{import_default_font, DocumentFontSelection};
    let project = TempDir::new("saved-text-cache");
    let data = TempDir::new("saved-text-cache-origin");
    let file = data.path().join("chosen.ttf");
    std::fs::write(&file, stellar_text::test_support::outline_test_font()).unwrap();
    let mut model = sample_document();
    let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
    let red = Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap();
    let default = import_default_font(
        project.path(),
        data.path(),
        &model.definition().text_styles,
        [sid(801), sid(802)],
        &DocumentFontSelection {
            file,
            index: 0,
            size: 20 * 64,
            fill: black,
        },
    )
    .unwrap();
    model.replace_text_styles(default.styles);
    let a = TextPayload::new(
        UserText::new("A\nA"),
        vec![Paragraph { style: 0 }; 2],
        vec![],
    )
    .unwrap();
    let b = TextPayload::new(
        UserText::new("A A\nA"),
        vec![Paragraph { style: 0 }; 2],
        vec![],
    )
    .unwrap();
    let id = sid(501);
    model
        .insert_node(NodeRecord {
            id,
            parent: Some(sid(10)),
            order_key: model.next_child_order_key(sid(10)),
            visible: true,
            locked: false,
            display_name: "Test".into(),
            payload: NodePayload::Text(a.clone()),
        })
        .unwrap();
    let mut fonts = document::FontCache::default();
    let mut saved = saved::SavedTextLayouts::default();
    let mut project_text =
        |id, model: &stellar_document::model::DocumentModel, payload: &TextPayload| {
            let table = &model.definition().text_styles;
            fonts.sync_registrations(table);
            saved.begin_view(table);
            let result = saved
                .layout(&mut fonts, project.path(), model, id, payload)
                .unwrap();
            saved.end_view();
            result
        };
    let original = project_text(id, &model, &a).0;
    let unchanged = project_text(id, &model, &a).0;
    assert!(Arc::ptr_eq(&original, &unchanged));
    let changed = project_text(id, &model, &b).0;
    assert!(!Arc::ptr_eq(&original, &changed));
    assert_eq!(changed.glyphs.len(), 3);
    let restored = project_text(id, &model, &a).0;
    assert_eq!(original, restored);
    let mut stack = ModifierStack::new(id);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(sid(502)),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 22., 0.).unwrap()),
        })
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
    let (moved, _, transform) = project_text(id, &model, &a);
    assert_eq!(
        transform,
        Transform2D::new(1., 0., 0., 1., 22., 0.).unwrap()
    );
    assert!(!Arc::ptr_eq(&restored, &moved));
    let mut styles = model.definition().text_styles.clone();
    styles.default_fill = Some(red);
    model.replace_text_styles(styles);
    let recolored = project_text(id, &model, &a).0;
    assert_eq!(recolored.fill, red);
    assert_eq!(original.fill, black);
    project_text(sid(600), &model, &a);
    let returned = project_text(id, &model, &a).0;
    assert!(!Arc::ptr_eq(&returned, &recolored));
    assert_eq!(returned, recolored);
}
