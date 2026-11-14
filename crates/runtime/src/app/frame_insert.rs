//! 표시한 자유 배치 Frame 아래에 새 Frame 하나를 삽입합니다.

use super::frame_move::{command_id, editable_frame, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::{Rect, Transform2D};
use stellar_document::frame::FramePayload;
use stellar_document::records::{NodePayload, NodeRecord};

impl AppRuntime {
    /// 부모의 local 공간에서 그린 영역에 Frame을 삽입하고 선택합니다.
    pub fn insert_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        bounds: Rect,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, session)?;
            insert_frame(window, request, view, revision, parent, bounds)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

fn insert_frame(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    parent: StableId,
    bounds: Rect,
) -> Result<StableId, Failure> {
    let invalid = || {
        Failure::new(
            RequestFailure::UnknownView,
            "Frame을 추가할 뷰·부모·영역이 유효하지 않습니다",
        )
    };
    if window.revision() != revision
        || window.tabs().active_view() != Some(view)
        || bounds.is_empty()
    {
        return Err(invalid());
    }
    let model = window.document().ok_or_else(invalid)?.model();
    editable_frame(model, view, parent).ok_or_else(invalid)?;
    let id = command_id(window.window(), request);
    // 새 node 식별자는 창 ID와 요청 순번으로부터 얻고 UUIDv7의 variant·version을 유지합니다.
    let bytes = id.to_bytes();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?;
    let ms = u64::try_from(now.as_millis()).map_err(|_| invalid())?;
    let node = StableId::new_v7(
        ms,
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u64::from_le_bytes(bytes[2..10].try_into().expect("eight bytes")),
    );
    let record = NodeRecord {
        id: node,
        parent: Some(parent),
        order_key: model.next_child_order_key(parent),
        visible: true,
        locked: false,
        display_name: "Frame".into(),
        payload: NodePayload::Frame(FramePayload {
            size: bounds.size(),
            transform: Transform2D::new(
                1.0,
                0.0,
                0.0,
                1.0,
                bounds.origin().x(),
                bounds.origin().y(),
            )
            .map_err(|_| invalid())?,
            background: Some(
                Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).expect("opaque white"),
            ),
            ..FramePayload::default()
        }),
    };
    let transaction = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(revision, Command::InsertNode(InsertNode::single(record)));
    window
        .insert_and_select(&transaction, node)
        .map_err(view_failure)?;
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};
    use stellar_commands::selection::Selection;
    use stellar_core_types::value::{Size2, Vec2};

    #[test]
    fn insert_and_selection_share_an_entry_and_save_real_geometry() {
        let dir = saved_project("canvas-insert");
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        let bounds = Rect::new(
            Vec2::new(12.5, 45.25).unwrap(),
            Size2::new(101.0, 68.0).unwrap(),
        );
        assert!(insert_frame(
            &mut session,
            RequestId(1),
            sid(4),
            revision,
            sid(10),
            bounds
        )
        .is_err());
        assert!(insert_frame(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            sid(20),
            bounds
        )
        .is_err());
        let created = insert_frame(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(10),
            bounds,
        )
        .ok()
        .unwrap();
        assert_eq!(session.view().selection().ids(), [created]);
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .children_in_order(sid(10))
                .last(),
            Some(&created)
        );
        assert!(insert_frame(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            sid(10),
            bounds
        )
        .is_err());
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), [sid(12)]);
        assert!(!session.document().unwrap().model().contains(created));
        assert!(session.redo().unwrap());
        assert_eq!(session.view().selection().ids(), [created]);
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        let Some(NodePayload::Frame(frame)) = model.payload(created) else {
            panic!("저장된 Frame이 없음");
        };
        assert_eq!(frame.size, bounds.size());
        assert_eq!(
            frame.transform.components(),
            (1.0, 0.0, 0.0, 1.0, 12.5, 45.25)
        );
    }
}
