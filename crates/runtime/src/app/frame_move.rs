//! 캔버스·속성 패널·Layers 편집 경로가 함께 쓰는 창·잠금·ID 도우미입니다. 이동·크기·반경은
//! `canvas_gesture`(ADR-0071), 속성 값 편집은 `inspector`(ADR-0070)가 합니다.

use super::*;
use stellar_commands::document::{frame::Clipping, records::NodePayload};
use stellar_commands::id::CommandId;
use stellar_document::model::DocumentModel;

/// 캔버스·Inspector·Layers 편집 대상 창을 찾습니다.
///
/// 문서 편집과 저장은 프로젝트 신뢰와 무관하게 허용합니다(사용자 결정 D1, 아키텍처 §24.3). 제한 모드는
/// Git hook·filter, 내장 터미널, MCP·에이전트, 외부 명령처럼 코드를 실행하는 경로에만 적용합니다.
pub(super) fn canvas_window(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
) -> Result<&mut ProjectWindowSession, Failure> {
    window_mut(ctx, session)
}

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
        if !fields.visible
            || fields.locked
            || (!(modifier_target && current == node) && model.modifier_stack(current).is_some())
        {
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

/// 복제·붙여넣기 계획 API는 ID 생성기를 받습니다. seed를 OS 난수로 채워 요청마다 독립 ID를 만듭니다.
pub(super) fn fresh_generator() -> Result<stellar_core_types::ids::StableIdGenerator, Failure> {
    let [seed] = fresh_ids()?;
    let bytes = seed.to_bytes();
    Ok(stellar_core_types::ids::StableIdGenerator::new(
        u64::from_le_bytes(bytes[8..].try_into().expect("eight bytes")),
    ))
}

/// 새 ID 생성기에 넘기는 지금 시각(UUIDv7 timestamp).
pub(super) fn now_clock() -> Result<stellar_core_types::clock::ManualClock, Failure> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_nanos()).ok())
        .ok_or_else(|| {
            Failure::new(RequestFailure::UnknownView, "시스템 시각을 읽지 못했습니다")
        })?;
    Ok(stellar_core_types::clock::ManualClock::new(nanos))
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
