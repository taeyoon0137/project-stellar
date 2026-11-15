//! 실제 문서 레코드에서 서로 다른 CSS longhand를 편집한 두 branch의 병합과 역투영.
use stellar_document::{
    definitions::DocumentDefinition, records::DocumentRecords, text_styles::DocumentTextStyles,
};
use stellar_semantic_diff::core_types::{
    ids::StableId,
    typed_value::TypographyValue,
    typography::{FontSize, LetterSpacing, Typography},
    value::FiniteF32,
};
use stellar_semantic_diff::{build::SnapshotBuilder, records::document_records};
use stellar_semantic_merge::{merge::merge, model::MergeInput};

#[test]
fn typography_size_and_spacing_merge_and_save_together() {
    let doc = StableId::new_v7(1, 0, 1);
    let font = StableId::new_v7(2, 0, 2);
    let snapshot = |size, spacing| {
        let mut definition = DocumentDefinition::new(doc, "Design".into());
        let mut styles = DocumentTextStyles::initial(font);
        let value = TypographyValue::from_record(&Typography {
            size,
            letter_spacing: spacing,
            ..Default::default()
        })
        .unwrap();
        styles.typography.insert(font, value);
        definition.text_styles = styles;
        let mut builder = SnapshotBuilder::new();
        builder
            .add_document(&DocumentRecords::new(definition))
            .unwrap();
        builder.finish()
    };
    let size = FontSize::Subpixels(1536);
    let spacing = LetterSpacing::Px(FiniteF32::new(0.75).unwrap());
    let input = MergeInput {
        base: snapshot(FontSize::Medium, LetterSpacing::Normal),
        ours: snapshot(size, LetterSpacing::Normal),
        theirs: snapshot(FontSize::Medium, spacing),
    };
    let merged = merge(&input).unwrap();
    assert!(merged.is_clean());
    assert_eq!(merged.merged, snapshot(size, spacing));
    let model = document_records(&merged.merged, doc).unwrap().records;
    let persisted = DocumentDefinition::decode(&model.definition.encode()).unwrap();
    let restored = persisted
        .text_styles
        .resolve(0, None)
        .unwrap()
        .typography
        .record()
        .unwrap();
    assert_eq!(restored.size, size);
    assert_eq!(restored.letter_spacing, spacing);
}

#[test]
fn paragraph_and_run_style_fields_merge_separately() {
    use stellar_document::text::{LineHeight, TextAlignment, TextStyle};
    use stellar_semantic_diff::core_types::color::{Color, ColorSpace};
    let doc = StableId::new_v7(1, 0, 1);
    let font = StableId::new_v7(2, 0, 2);
    let red = Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap();
    let spacing = LetterSpacing::Px(FiniteF32::new(1.5).unwrap());
    let fixed = LineHeight::Fixed(FiniteF32::new(24.).unwrap());
    let snapshot = |alignment, line_height, fill, letter_spacing| {
        let mut definition = DocumentDefinition::new(doc, "Design".into());
        let mut styles = DocumentTextStyles::initial(font);
        let paragraph = styles.paragraphs.get_mut(&0).unwrap();
        paragraph.alignment = alignment;
        paragraph.line_height = line_height;
        let mut run = TextStyle::new(None, fill, vec![], vec![]);
        run.letter_spacing = letter_spacing;
        styles.runs.insert(0, run);
        definition.text_styles = styles;
        let mut builder = SnapshotBuilder::new();
        builder
            .add_document(&DocumentRecords::new(definition))
            .unwrap();
        builder.finish()
    };
    // 같은 문단·run entry에서 ours는 정렬과 색을, theirs는 줄 높이와 자간을 바꿉니다.
    let input = MergeInput {
        base: snapshot(TextAlignment::Start, LineHeight::Auto, None, None),
        ours: snapshot(TextAlignment::Center, LineHeight::Auto, Some(red), None),
        theirs: snapshot(TextAlignment::Start, fixed, None, Some(spacing)),
    };
    let merged = merge(&input).unwrap();
    assert!(merged.is_clean());
    assert_eq!(
        merged.merged,
        snapshot(TextAlignment::Center, fixed, Some(red), Some(spacing))
    );
    let restored = document_records(&merged.merged, doc)
        .unwrap()
        .records
        .definition
        .text_styles;
    assert_eq!(restored.paragraphs[&0].alignment, TextAlignment::Center);
    assert_eq!(restored.paragraphs[&0].line_height, fixed);
    assert_eq!(restored.runs[&0].fill, Some(red));
    assert_eq!(restored.runs[&0].letter_spacing, Some(spacing));
    // 같은 필드를 양쪽에서 다르게 바꾸면 충돌로 남깁니다.
    let conflict = MergeInput {
        base: snapshot(TextAlignment::Start, LineHeight::Auto, None, None),
        ours: snapshot(TextAlignment::Center, LineHeight::Auto, None, None),
        theirs: snapshot(TextAlignment::End, LineHeight::Auto, None, None),
    };
    assert!(!merge(&conflict).unwrap().is_clean());
}
