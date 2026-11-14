//! 창의 메모리 문서에서 활성 View의 직계 자식 한 페이지만 읽습니다.
use super::*;
use crate::document_thread::JobTicket;
use stellar_document::records::NodePayload;

/// 한 번의 disclosure 또는 다음 페이지 요청에서 내보낼 형제 수.
pub const PAGE_SIZE: usize = 64;

/// 사이드바에 표시할 한 노드. 숨김·잠금 상태도 원본대로 읽습니다.
#[derive(Debug)]
pub struct LayerRow {
    /// 노드의 영속 ID.
    pub id: StableId,
    /// 문서의 표시 이름.
    pub name: String,
    /// NodeKind 저장 태그.
    pub kind: u16,
    /// ShapeKind 저장 태그. 다른 종류는 0.
    pub shape: u8,
    /// 노드 자체의 표시 상태.
    pub visible: bool,
    /// 노드 자체의 잠금 상태.
    pub locked: bool,
    /// 직계 자식이 하나 이상인지 여부.
    pub has_children: bool,
    /// 선택한 순번(최초 선택이 0), 이 창에서 선택하지 않았으면 없음.
    pub selection_order: Option<usize>,
}

/// 표시한 페이지·View 이름과 한 부모 바로 아래의 행을 함께 반환합니다.
#[derive(Debug)]
pub struct LayerPage {
    /// 활성 View를 소유한 페이지 ID.
    pub page: StableId,
    /// 활성 페이지의 표시 이름.
    pub page_name: String,
    /// 활성 View의 표시 이름.
    pub view_name: String,
    /// 창의 primary 선택.
    pub primary: Option<StableId>,
    /// 표시한 부모와 primary 사이의 첫 자식. 접힌 선택 경로를 한 부모씩 펼칠 때 씁니다.
    pub reveal_child: Option<StableId>,
    /// 같은 부모 아래에 다음 페이지가 있는지 여부.
    pub more: bool,
    /// 위에 그린 형제부터 나열한 실제 노드.
    pub rows: Vec<LayerRow>,
}

impl AppRuntime {
    /// 문서 스레드의 읽기만 접수합니다. UI는 이 ticket을 기다리지 않습니다.
    pub fn sidebar_page(
        &self,
        window: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: Option<StableId>,
        after: Option<StableId>,
    ) -> Result<JobTicket<Option<LayerPage>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let entry = ctx.app_sessions().windows.get(&window)?;
            read_page(&entry.session, view, revision, parent, after)
        })
    }
}

fn read_page(
    window: &ProjectWindowSession,
    view_id: StableId,
    revision: DocumentRevision,
    parent: Option<StableId>,
    after: Option<StableId>,
) -> Option<LayerPage> {
    if window.revision() != revision || window.tabs().active_view()? != view_id {
        return None;
    }
    let model = window.document()?.model();
    let (page, view) = model.definition().pages.iter().find_map(|page| {
        page.views
            .iter()
            .find(|v| v.id == view_id)
            .map(|v| (page, v))
    })?;
    let parent = parent.unwrap_or(view.root_frame);
    let ancestor = |id| model.stable_id(model.columns().parent(model.runtime_id(id)?)?);
    let mut cursor = parent;
    while cursor != view.root_frame {
        cursor = ancestor(cursor)?;
    }
    let mut ids = model.children_front_to_back(parent, after, PAGE_SIZE + 1)?;
    let more = ids.len() > PAGE_SIZE;
    ids.truncate(PAGE_SIZE);
    let selection = window.view().selection();
    let mut reveal_child = None;
    let mut cursor = selection.primary();
    while let Some(id) = cursor {
        cursor = ancestor(id);
        if cursor == Some(parent) {
            reveal_child = Some(id);
            break;
        }
        if id == view.root_frame {
            break;
        }
    }
    let rows = ids
        .into_iter()
        .map(|id| {
            let handle = model.runtime_id(id)?;
            let fields = model.columns().get(handle)?;
            Some(LayerRow {
                id,
                name: fields.display_name,
                kind: fields.kind.as_u16(),
                shape: match model.payload(id) {
                    Some(NodePayload::Shape(shape)) => shape.kind().tag(),
                    _ => 0,
                },
                visible: fields.visible,
                locked: fields.locked,
                has_children: model.columns().child_count(handle)? != 0,
                selection_order: selection.ids().iter().position(|selected| *selected == id),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(LayerPage {
        page: page.id,
        page_name: page.display_name().to_owned(),
        view_name: view.display_name().to_owned(),
        primary: selection.primary(),
        reveal_child,
        more,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};

    #[test]
    fn a_page_is_direct_front_to_back_and_belongs_to_the_displayed_view() {
        let dir = saved_project("layer-page");
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        session
            .select(stellar_commands::selection::Selection::new(
                [sid(11), sid(12)],
                None,
            ))
            .unwrap();
        let read = |parent, after| read_page(&session, sid(3), session.revision(), parent, after);
        let root = read(None, None).unwrap();
        assert_eq!(
            root.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [sid(12), sid(11)]
        );
        assert_eq!(root.primary, Some(sid(12)));
        assert_eq!(root.rows[0].selection_order, Some(1));
        assert_eq!(root.rows[1].selection_order, Some(0));
        assert_eq!(read(None, Some(sid(12))).unwrap().rows[0].id, sid(11));
        assert!(read(Some(sid(20)), None).is_none());
        assert!(read(None, Some(sid(20))).is_none());
        assert!(read_page(&session, sid(4), session.revision(), None, None).is_none());
        assert!(read_page(
            &session,
            sid(3),
            DocumentRevision(session.revision().0 + 1),
            None,
            None
        )
        .is_none());
    }
}
