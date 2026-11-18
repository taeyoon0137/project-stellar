//! 캔버스·속성 패널·Layers 편집 경로가 함께 쓰는 잠금·ID 도우미입니다. 편집 대상 창은 `window_mut`로 찾습니다.
//! 문서 편집과 저장은 프로젝트 신뢰와 무관합니다(사용자 결정 D1). 제한 모드는 코드를 실행하는 경로에만 적용합니다. 이동·크기·반경은
//! `canvas_gesture`(ADR-0071), 속성 값 편집은 `inspector`(ADR-0070)가 합니다.

use super::*;
use stellar_commands::document::{frame::Clipping, records::NodePayload};
use stellar_commands::id::CommandId;
use stellar_document::model::DocumentModel;

pub(super) fn editable_frame(
    model: &stellar_document::model::DocumentModel,
    view: StableId,
    node: StableId,
) -> Option<&stellar_document::frame::FramePayload> {
    let Some(NodePayload::Frame(target)) = model.payload(node) else {
        return None;
    };
    editable_ancestry(model, view, node, false)?;
    Some(target)
}

pub(super) fn editable_ancestry(
    model: &DocumentModel,
    view: StableId,
    node: StableId,
    modifier_target: bool,
) -> Option<()> {
    stellar_commands::lock::edit_permission(
        model,
        node,
        stellar_commands::lock::EditIntent::Modify(
            stellar_commands::change::node_keys::DISPLAY_NAME,
        ),
    )
    .ok()?;
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|v| v.id == view)?
        .root_frame;
    let mut current = node;
    loop {
        let fields = model
            .runtime_id(current)
            .and_then(|id| model.columns().get(id))?;
        if !(modifier_target && current == node) && model.modifier_stack(current).is_some() {
            return None;
        }
        match model.payload(current) {
            Some(NodePayload::Frame(frame))
                if frame.layout_mode.is_none() && !matches!(frame.clipping, Clipping::Path(_)) => {}
            Some(NodePayload::Fragment(_)) => {}
            Some(
                NodePayload::Shape(_)
                | NodePayload::ComponentInstance(_)
                | NodePayload::Text(_)
                | NodePayload::Image(_),
            ) if modifier_target && current == node => {}
            _ => return None,
        }
        if current == root {
            break;
        }
        current = fields.parent.and_then(|id| model.stable_id(id))?;
    }
    Some(())
}

/// 새 node·modifier의 영속 ID(UUIDv7, OS 난수)를 만듭니다. 요청 순번이나 command ID에서 파생하지 않습니다.
pub(super) fn fresh_ids<const N: usize>() -> Result<[StableId; N], Failure> {
    stellar_project::new_ids::new_ids().map_err(|error| {
        Failure::new(
            RequestFailure::UnknownView,
            format!("새 식별자를 만들지 못했습니다: {error}"),
        )
    })
}

/// 복제·붙여넣기·Option 드래그 복제·이동·속성 탭이 계획에 넘기는 새 ID 원천입니다. ID마다 [`fresh_ids`](UUIDv7, OS 난수)를 부르고, 실패하면
/// NIL을 돌려준 뒤 [`FreshIds::finish`]에서 실패로 바꿉니다(NIL ID는 command validator도 거부합니다).
/// 테스트용 결정적 생성기(`StableIdGenerator`)는 제품 경로에서 쓰지 않습니다.
#[derive(Default)]
pub(super) struct FreshIds {
    failure: Option<Failure>,
}

impl FreshIds {
    pub(super) fn next(&mut self) -> StableId {
        match fresh_ids() {
            Ok([id]) => id,
            Err(failure) => {
                self.failure.get_or_insert(failure);
                StableId::NIL
            }
        }
    }

    pub(super) fn finish(self) -> Result<(), Failure> {
        self.failure.map_or(Ok(()), Err)
    }
}

/// 요청 순번은 runtime 안에서만 유일하므로 창의 영속 ID와 함께 식별합니다.
pub(super) fn command_id(window: WindowSessionId, request: RequestId) -> CommandId {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&window.stable_id().to_bytes());
    key[16..].copy_from_slice(&request.0.to_le_bytes());
    let digest = stellar_storage::hash::ContentHash::compute(&key);
    let mut raw = [0; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    CommandId::from_bytes(raw)
}
