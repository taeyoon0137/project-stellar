//! 새 편집 프로젝트의 첫 저장 세대. 신규 폴더 트랜잭션 안에서만 호출합니다.
use std::io;
use std::path::Path;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Size2;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

use crate::document_io::serialize::serialize_document;
use crate::manifest::CURRENT_SCHEMA_VERSION;

pub(super) fn save(
    root: &Path,
    [project, document, page, view, frame, typography]: [StableId; 6],
) -> io::Result<()> {
    let mut definition = DocumentDefinition::new(document, "Document".into());
    definition.text_styles = stellar_document::text_styles::DocumentTextStyles::initial(typography);
    let mut first_page = PageDefinition::new(page, "Page 1".into());
    first_page
        .add_view(ViewDefinition::new(view, "View 1".into(), frame))
        .map_err(io::Error::other)?;
    definition.add_page(first_page).map_err(io::Error::other)?;
    let mut model = DocumentModel::new(definition);
    model
        .insert_node(NodeRecord {
            id: frame,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: "Frame 1".into(),
            payload: NodePayload::Frame(FramePayload {
                size: Size2::new(800.0, 600.0).expect("positive size"),
                background: Some(
                    Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).expect("opaque white"),
                ),
                ..FramePayload::default()
            }),
        })
        .map_err(io::Error::other)?;
    let serialized = serialize_document(&model).map_err(io::Error::other)?;
    let plan = build_save_plan(
        None,
        project,
        CURRENT_SCHEMA_VERSION,
        &serialized.to_dirty_states(),
        vec![document],
    )
    .map_err(io::Error::other)?;
    run_save(
        root,
        &plan,
        &mut SavedGenerationState::new(None),
        &CancelToken::new(),
    )
    .map_err(io::Error::other)
}
