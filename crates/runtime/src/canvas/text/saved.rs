//! 직전 활성 View의 텍스트만 보관합니다. 동일한 본문/스타일의 게시 결과는 snapshot 간에 공유합니다.
use super::{document::FontCache, DocumentText};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use stellar_core_types::{
    ids::StableId,
    value::{Size2, Transform2D},
};
use stellar_document::{
    model::DocumentModel, modifier::ModifierStack, text::TextPayload,
    text_styles::DocumentTextStyles,
};
use stellar_text::incremental::IncrementalTextLayout;

type Projection = (Arc<DocumentText>, Size2, Transform2D);

#[derive(Debug)]
struct Entry {
    payload: TextPayload,
    modifiers: Option<ModifierStack>,
    engine: Option<IncrementalTextLayout>,
    projection: Projection,
    visited: bool,
}

#[derive(Debug, Default)]
pub(crate) struct SavedTextLayouts {
    styles: DocumentTextStyles,
    entries: BTreeMap<StableId, Entry>,
}

impl SavedTextLayouts {
    pub(crate) fn begin_view(&mut self, styles: &DocumentTextStyles) {
        if self.styles != *styles {
            self.styles.clone_from(styles);
            self.entries.clear();
        }
        for entry in self.entries.values_mut() {
            entry.visited = false;
        }
    }

    pub(crate) fn end_view(&mut self) {
        self.entries.retain(|_, entry| entry.visited);
    }

    pub(crate) fn layout(
        &mut self,
        fonts: &mut FontCache,
        root: &Path,
        model: &DocumentModel,
        id: StableId,
        payload: &TextPayload,
    ) -> Option<Projection> {
        let modifiers = model.modifier_stack(id);
        if let Some(entry) = self.entries.get_mut(&id) {
            if &entry.payload == payload && entry.modifiers.as_ref() == modifiers {
                entry.visited = true;
                return Some(entry.projection.clone());
            }
        }
        // 편집되지 않은 node는 완성한 projection만 보유합니다. 첫 변경부터 문단 cache를 유지합니다.
        let (prepared, engine) = match self.entries.remove(&id) {
            Some(entry) => {
                let mut engine = entry.engine.unwrap_or_default();
                let prepared = fonts.layout_with(model, root, id, payload, &mut engine)?;
                (prepared, Some(engine))
            }
            None => (fonts.layout(model, root, id, payload)?, None),
        };
        let (text, size, transform) = prepared;
        let projection = (Arc::new(text), size, transform);
        self.entries.insert(
            id,
            Entry {
                payload: payload.clone(),
                modifiers: modifiers.cloned(),
                engine,
                projection: projection.clone(),
                visited: true,
            },
        );
        Some(projection)
    }
}
