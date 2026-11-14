//! 라이브러리에서 고른 정의 참조를 활성 View에 삽입합니다.
use super::*;
use crate::document_thread::DocumentSender;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::{
    order_key::OrderKey,
    reference::ComponentRef,
    value::{Transform2D, Vec2},
};
use stellar_document::{
    component::instance::ComponentInstancePayload,
    modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack},
    records::{NodePayload, NodeRecord},
};
use stellar_ops::revision::ProjectRevision;
use stellar_storage::hash::ContentHash;

/// 이름 입력 대신 라이브러리에 표시된 원본·View·revision을 고정합니다.
pub struct InstancePlacement {
    /// 선택한 정의 ID.
    pub component: StableId,
    /// 표시한 원본의 실제 hash.
    pub source_hash: ContentHash,
    /// 배치할 View.
    pub view: StableId,
    /// 시트를 열 때의 문서 revision.
    pub revision: DocumentRevision,
    /// View root 기준 논리 좌표.
    pub position: Vec2,
}

impl AppRuntime {
    /// 저장은 기존 Save Now 경로를 사용합니다. 이 요청은 하나의 Undo 가능한 삽입을 만듭니다.
    pub fn place_component_instance(
        &self,
        session: WindowSessionHandle,
        placement: InstancePlacement,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        if self.shared.creating_component.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let sender = self.sender.clone();
        let shared = self.shared.clone();
        let worker = std::thread::Builder::new()
            .name("stellar-instance-place".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| {
                    place(&shared, &sender, session, request, placement)
                })
                .unwrap_or_else(|_| {
                    Err(Failure::new(
                        RequestFailure::SaveFailed,
                        "컴포넌트 배치 작업이 중단되었습니다",
                    ))
                });
                shared.creating_component.store(false, Ordering::Release);
                shared.outbox().push_back(match result {
                    Ok(()) => RuntimeEvent::WindowViewChanged { request, session },
                    Err(e) => RuntimeEvent::RequestFailed {
                        request,
                        failure: e.failure,
                        message: e.message,
                    },
                });
            });
        if worker.is_err() {
            self.shared
                .creating_component
                .store(false, Ordering::Release);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

fn place(
    shared: &Arc<AppShared>,
    sender: &DocumentSender,
    session: WindowSessionHandle,
    request: RequestId,
    input: InstancePlacement,
) -> Result<(), Failure> {
    let state = shared.current_state();
    let owner = state
        .window(session)
        .and_then(|w| state.project(w.project))
        .ok_or_else(|| unknown_window(session))?;
    let invalid = |message: String| Failure::new(RequestFailure::SaveFailed, message);
    let library = owner.component_library().map_err(invalid)?;
    let definition = library
        .entries
        .into_iter()
        .find(|e| e.id == input.component && e.hash == input.source_hash)
        .ok_or_else(|| {
            invalid("표시한 컴포넌트 원본이 바뀌었습니다. 라이브러리를 새로 고치세요".into())
        })?;
    let ids = stellar_project::new_ids::new_ids().map_err(|e| invalid(e.to_string()))?;
    let shared = shared.clone();
    let ticket = sender
        .submit(move |ctx| {
            let window = super::frame_move::trusted_canvas_window(ctx, session)?;
            insert(
                window,
                request,
                &input,
                library.revision,
                definition.name,
                ids,
            )?;
            shared.publish_state(build_state(ctx));
            Ok(())
        })
        .map_err(|e| invalid(e.to_string()))?;
    ticket.wait().map_err(|e| invalid(e.to_string()))?
}

fn insert(
    window: &mut ProjectWindowSession,
    request: RequestId,
    input: &InstancePlacement,
    base: ProjectRevision,
    name: String,
    [node, transform_id]: [StableId; 2],
) -> Result<(), Failure> {
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "배치할 View나 문서가 변경되었습니다",
        )
    };
    if window.tabs().active_view() != Some(input.view)
        || window.revision() != input.revision
        || window.loaded_generation().map(|g| g.manifest_hash) != base.manifest_hash()
    {
        return Err(stale());
    }
    let model = window.document().ok_or_else(stale)?.model();
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == input.view)
        .ok_or_else(stale)?
        .root_frame;
    super::frame_move::editable_frame(model, input.view, root).ok_or_else(stale)?;
    let component = ComponentInstancePayload::new(ComponentRef::new(input.component));
    let mut command = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(root),
        order_key: model.next_child_order_key(root),
        visible: true,
        locked: false,
        display_name: name,
        payload: NodePayload::ComponentInstance(component),
    });
    let transform = Transform2D::new(1., 0., 0., 1., input.position.x(), input.position.y())
        .map_err(|_| stale())?;
    let mut stack = ModifierStack::new(node);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(transform_id),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(transform),
        })
        .map_err(|_| stale())?;
    command.modifier_stacks.push(stack);
    let id = super::frame_move::command_id(window.window(), request);
    let tx = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(input.revision, Command::InsertNode(command));
    window.insert_and_select(&tx, node).map_err(view_failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};

    #[test]
    fn place_records_real_reference_and_exact_offsets_with_one_undo_and_save() {
        let dir = saved_project("place-instance");
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let loaded = session.loaded_generation().unwrap();
        let base = ProjectRevision::Saved {
            generation: loaded.manifest.generation,
            manifest_hash: loaded.manifest_hash,
        };
        let mut input = InstancePlacement {
            component: sid(301),
            source_hash: ContentHash::compute(b"source"),
            view: sid(3),
            revision: session.revision(),
            position: Vec2::new(230.5, -10.).unwrap(),
        };
        let old = session.document().unwrap().model().to_records().unwrap();
        let before = session.history().history().len();
        assert!(insert(
            &mut session,
            RequestId(1),
            &input,
            ProjectRevision::Unsaved,
            "Card".into(),
            [sid(400), sid(401)]
        )
        .is_err());
        assert!(insert(
            &mut session,
            RequestId(2),
            &input,
            base,
            "Card".into(),
            [sid(11), sid(401)]
        )
        .is_err());
        assert_eq!(
            session.document().unwrap().model().to_records().unwrap(),
            old
        );
        insert(
            &mut session,
            RequestId(3),
            &input,
            base,
            "Card".into(),
            [sid(400), sid(401)],
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(insert(
            &mut session,
            RequestId(4),
            &input,
            base,
            "Stale".into(),
            [sid(402), sid(403)]
        )
        .is_err());
        assert_eq!(session.history().history().len(), before + 1);
        assert_eq!(session.view().selection().ids(), [sid(400)]);
        assert!(session.undo().unwrap());
        assert_eq!(
            session.document().unwrap().model().to_records().unwrap(),
            old
        );
        assert!(session.redo().unwrap());
        input.revision = session.revision();
        input.position = Vec2::new(30., 20.).unwrap();
        insert(
            &mut session,
            RequestId(5),
            &input,
            base,
            "Card".into(),
            [sid(402), sid(403)],
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        for (node, x, y) in [(400, 230.5, -10.), (402, 30., 20.)] {
            assert_eq!(
                model.payload(sid(node)),
                Some(&NodePayload::ComponentInstance(
                    ComponentInstancePayload::new(ComponentRef::new(sid(301)))
                ))
            );
            let [entry] = model.modifier_stack(sid(node)).unwrap().entries() else {
                panic!("single placement transform")
            };
            assert_eq!(
                entry.payload,
                ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., x, y).unwrap())
            );
        }
        assert_eq!(model.references().inbound(sid(301)), [sid(400), sid(402)]);
    }
}
