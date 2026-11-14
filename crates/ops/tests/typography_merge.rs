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
