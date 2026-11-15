//! 창의 메모리 문서에서 활성 View의 직계 자식 한 페이지만 읽습니다.
use super::*;
use crate::document_thread::JobTicket;
use stellar_document::{model::DocumentModel, records::NodePayload};

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
    /// 표시를 바꿀 수 있는지(잠금 규칙 D8: 자신과 조상이 모두 잠기지 않음).
    pub can_hide: bool,
    /// 잠금을 바꿀 수 있는지(D8: 조상이 모두 잠기지 않음).
    pub can_lock: bool,
    /// 직계 자식이 하나 이상인지 여부.
    pub has_children: bool,
    /// 선택한 순번(최초 선택이 0), 이 창에서 선택하지 않았으면 없음.
    pub selection_order: Option<usize>,
}

/// 표시한 페이지·View 이름과 한 부모 바로 아래의 행을 함께 반환합니다.
#[derive(Debug)]
pub struct LayerPage {
    /// 행의 선택 순서와 함께 문서 스레드에서 읽은 이 창의 history 위치.
    pub history_sequence: u64,
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

/// 작업 문서의 Page 하나와 그 View들(정의 순서). 사이드바 머리 줄 Page › View 팝업이 읽습니다.
#[derive(Debug, PartialEq, Eq)]
pub struct OutlinePage {
    /// Page ID.
    pub id: StableId,
    /// Page 표시 이름.
    pub name: String,
    /// View ID와 표시 이름.
    pub views: Vec<(StableId, String)>,
}

/// 레이어 행에서 켜고 끄는 노드 속성(`SetProperty::visibility`·`SetProperty::locked`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerFlag {
    /// `node.visible`.
    Visible,
    /// `node.locked`.
    Locked,
}

impl AppRuntime {
    /// 표시한 레이어 행들의 표시 또는 잠금을 한 번의 Undo 가능한 transaction으로 바꿉니다.
    pub fn set_layer_flag(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        nodes: Vec<StableId>,
        flag: LayerFlag,
        value: bool,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = super::frame_move::canvas_window(ctx, session)?;
            set_flags(window, request, view, revision, &nodes, flag, Some(value))?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 현재 표시한 레이어 이름을 한 번의 Undo 가능한 문서 명령으로 바꿉니다.
    pub fn rename_layer(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        name: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = super::frame_move::canvas_window(ctx, session)?;
            rename(window, request, view, revision, node, name)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 창의 작업 문서에 있는 Page·View 목록 읽기를 접수합니다. UI는 이 ticket을 기다리지 않습니다.
    pub fn view_outline(
        &self,
        window: WindowSessionHandle,
    ) -> Result<JobTicket<Option<Vec<OutlinePage>>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let entry = ctx.app_sessions().windows.get(&window)?;
            Some(outline(entry.session.document()?.model()))
        })
    }

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

fn rename(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    name: String,
) -> Result<(), Failure> {
    use stellar_commands::{command::Command, id::CommandOrigin, transaction::CommandTransaction};
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "뷰·레이어·편집 상태가 변경되었습니다",
        )
    };
    if window.revision() != revision || window.tabs().active_view() != Some(view) {
        return Err(stale());
    }
    let model = window.document().ok_or_else(stale)?.model();
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)
        .ok_or_else(stale)?
        .root_frame;
    let columns = model.columns();
    let original = columns
        .get(model.runtime_id(node).ok_or_else(stale)?)
        .ok_or_else(stale)?;
    let unchanged = original.display_name == name;
    let mut cursor = original;
    loop {
        if cursor.locked {
            return Err(Failure::new(
                RequestFailure::UnknownView,
                "잠긴 레이어나 잠긴 부모 아래의 레이어는 이름을 바꿀 수 없습니다",
            ));
        }
        if cursor.stable_id == root {
            break;
        }
        cursor = columns
            .get(cursor.parent.ok_or_else(stale)?)
            .ok_or_else(stale)?;
    }
    if unchanged {
        return Ok(());
    }
    let tx = CommandTransaction::new(
        super::frame_move::command_id(window.window(), request),
        CommandOrigin::User,
    )
    .with_command(revision, Command::rename_node(node, name));
    window.apply(&tx).map_err(view_failure)?;
    Ok(())
}

/// 표시·잠금 변경을 거부한 이유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlagDenied {
    /// 대상이 없거나 중복이거나 활성 View 트리에 없습니다(오래된 표시).
    Stale,
    /// 잠금 규칙(D8)에 걸립니다.
    Locked,
}

/// `node`부터 View root까지(둘 다 포함) 잠긴 노드가 있는지. `node`가 root 아래에 없으면 `None`입니다.
pub(super) fn path_locked(model: &DocumentModel, node: StableId, root: StableId) -> Option<bool> {
    let columns = model.columns();
    let mut cursor = columns.get(model.runtime_id(node)?)?;
    loop {
        if cursor.locked {
            return Some(true);
        }
        if cursor.stable_id == root {
            return Some(false);
        }
        cursor = columns.get(cursor.parent?)?;
    }
}

/// 표시 또는 잠금을 바꿀 대상과 각 대상의 지금 값(켜져 있으면 true).
///
/// 잠금 규칙(D8): 잠긴 조상 아래의 노드는 바꾸지 않고, 표시 변경은 노드 자신의 잠금도 막습니다.
/// 잠금 해제는 그 노드의 잠금 명령으로만 합니다. 앱 메뉴 활성(선택 능력 mask)과 실행이 이 함수 하나를 씁니다.
pub(super) fn flag_targets(
    model: &DocumentModel,
    root: StableId,
    nodes: &[StableId],
    flag: LayerFlag,
) -> Result<Vec<(StableId, bool)>, FlagDenied> {
    let columns = model.columns();
    let mut seen = std::collections::HashSet::with_capacity(nodes.len());
    let mut targets = Vec::with_capacity(nodes.len());
    for &node in nodes {
        if node == root || !seen.insert(node) {
            return Err(FlagDenied::Stale);
        }
        let fields = model
            .runtime_id(node)
            .and_then(|id| columns.get(id))
            .ok_or(FlagDenied::Stale)?;
        let parent = fields
            .parent
            .and_then(|id| model.stable_id(id))
            .ok_or(FlagDenied::Stale)?;
        let inherited = path_locked(model, parent, root).ok_or(FlagDenied::Stale)?;
        if inherited || (flag == LayerFlag::Visible && fields.locked) {
            return Err(FlagDenied::Locked);
        }
        let on = match flag {
            LayerFlag::Visible => fields.visible,
            LayerFlag::Locked => fields.locked,
        };
        targets.push((node, on));
    }
    if targets.is_empty() {
        return Err(FlagDenied::Stale);
    }
    Ok(targets)
}

/// 대상 노드들의 표시 또는 잠금을 한 transaction으로 바꿉니다. 바뀌는 값이 없으면 기록하지 않습니다.
///
/// `value`가 없으면 대상이 모두 켜져 있을 때 모두 끄고, 아니면 모두 켭니다(선택 메뉴 명령).
pub(super) fn set_flags(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    nodes: &[StableId],
    flag: LayerFlag,
    value: Option<bool>,
) -> Result<(), Failure> {
    use stellar_commands::{command::Command, id::CommandOrigin, transaction::CommandTransaction};
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "뷰·레이어·편집 상태가 변경되었습니다",
        )
    };
    if window.revision() != revision || window.tabs().active_view() != Some(view) {
        return Err(stale());
    }
    let model = window.document().ok_or_else(stale)?.model();
    let root = view_root(model, view).ok_or_else(stale)?;
    let targets = flag_targets(model, root, nodes, flag).map_err(|denied| match denied {
        FlagDenied::Stale => stale(),
        FlagDenied::Locked => Failure::new(
            RequestFailure::UnknownView,
            "잠긴 레이어는 바꿀 수 없습니다",
        ),
    })?;
    let value = value.unwrap_or_else(|| !targets.iter().all(|(_, on)| *on));
    let mut tx = CommandTransaction::new(
        super::frame_move::command_id(window.window(), request),
        CommandOrigin::User,
    );
    let mut changed = false;
    for (node, on) in targets {
        if on == value {
            continue;
        }
        changed = true;
        let command = match flag {
            LayerFlag::Visible => Command::set_visibility(node, value),
            LayerFlag::Locked => Command::set_locked(node, value),
        };
        tx = tx.with_command(revision, command);
    }
    if changed {
        window.apply(&tx).map_err(view_failure)?;
    }
    Ok(())
}

/// 활성 View의 root Frame.
pub(super) fn view_root(model: &DocumentModel, view: StableId) -> Option<StableId> {
    model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)
        .map(|v| v.root_frame)
}

fn outline(model: &DocumentModel) -> Vec<OutlinePage> {
    model
        .definition()
        .pages
        .iter()
        .map(|page| OutlinePage {
            id: page.id,
            name: page.display_name().to_owned(),
            views: page
                .views
                .iter()
                .map(|view| (view.id, view.display_name().to_owned()))
                .collect(),
        })
        .collect()
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
    // 한 페이지의 행은 부모가 같으므로 조상 잠금을 한 번만 봅니다.
    let parent_locked = path_locked(model, parent, view.root_frame)?;
    let mut ids = model.children_front_to_back(parent, after, PAGE_SIZE + 1)?;
    let more = ids.len() > PAGE_SIZE;
    ids.truncate(PAGE_SIZE);
    let selection = window.view().selection();
    // 행마다 선택 목록을 훑지 않도록 선택 순번 표를 한 번만 만듭니다(선택 수 + 페이지 행 수에 선형).
    let selection_order: std::collections::HashMap<StableId, usize> = selection
        .ids()
        .iter()
        .enumerate()
        .map(|(order, id)| (*id, order))
        .collect();
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
                can_hide: !parent_locked && !fields.locked,
                can_lock: !parent_locked,
                has_children: model.columns().child_count(handle)? != 0,
                selection_order: selection_order.get(&id).copied(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(LayerPage {
        history_sequence: window.history_sequence(),
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
    fn rename_checks_revision_ownership_and_changes_one_history_entry() {
        use crate::window_session::test_fixtures::{sample_document, saved_project_with};
        let mut model = sample_document();
        let mut locked = model.record(sid(12)).unwrap();
        locked.locked = true;
        locked.id = sid(30);
        locked.parent = Some(sid(12));
        model.insert_node(locked).unwrap();
        let mut child = model.record(sid(11)).unwrap();
        child.id = sid(31);
        child.parent = Some(sid(30));
        model.insert_node(child).unwrap();
        let dir = saved_project_with("layer-rename", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let rev = session.revision();
        let change = |win: &mut ProjectWindowSession, revision, node, name: &str| {
            rename(win, RequestId(5), sid(3), revision, sid(node), name.into())
        };
        for id in [20, 30, 31, 999] {
            assert!(change(&mut session, rev, id, "거부").is_err());
        }
        assert!(change(&mut session, rev, 12, "n12").is_ok());
        assert_eq!(session.revision(), rev);
        assert!(change(&mut session, rev, 12, "버튼").is_ok());
        assert!(change(&mut session, rev, 12, "이전 요청").is_err());
        assert!(session.undo().unwrap());
        let name = |win: &ProjectWindowSession| {
            win.document()
                .unwrap()
                .model()
                .record(sid(12))
                .unwrap()
                .display_name
        };
        assert_eq!(name(&session), "n12");
        assert!(session.redo().unwrap());
        assert_eq!(name(&session), "버튼");
    }

    #[test]
    fn layer_flags_are_one_history_entry_and_follow_lock_rules() {
        use crate::window_session::test_fixtures::{sample_document, saved_project_with};
        let mut model = sample_document();
        let mut locked = model.record(sid(12)).unwrap();
        locked.locked = true;
        locked.id = sid(30);
        locked.parent = Some(sid(12));
        model.insert_node(locked).unwrap();
        let mut child = model.record(sid(11)).unwrap();
        child.id = sid(31);
        child.parent = Some(sid(30));
        model.insert_node(child).unwrap();
        let dir = saved_project_with("layer-flags", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let set = |win: &mut ProjectWindowSession, nodes: &[u64], flag, value| {
            let revision = win.revision();
            let nodes: Vec<_> = nodes.iter().map(|n| sid(*n)).collect();
            set_flags(win, RequestId(7), sid(3), revision, &nodes, flag, value)
        };
        let flags = |win: &ProjectWindowSession, n| {
            let record = win.document().unwrap().model().record(sid(n)).unwrap();
            (record.visible, record.locked)
        };
        let rev = session.revision();
        // 루트·다른 View·없는 노드·중복·오래된 revision은 거부합니다.
        for nodes in [&[10][..], &[20], &[999], &[11, 11], &[]] {
            assert!(set(&mut session, nodes, LayerFlag::Visible, Some(false)).is_err());
        }
        let next = DocumentRevision(rev.0 + 1);
        assert!(set_flags(
            &mut session,
            RequestId(7),
            sid(3),
            next,
            &[sid(11)],
            LayerFlag::Visible,
            Some(false)
        )
        .is_err());
        // 잠긴 노드의 표시와 잠긴 조상 아래 노드는 바꾸지 않습니다(D8).
        assert!(set(&mut session, &[30], LayerFlag::Visible, Some(false)).is_err());
        assert!(set(&mut session, &[31], LayerFlag::Locked, Some(true)).is_err());
        // 같은 값은 기록하지 않습니다.
        assert!(set(&mut session, &[11, 12], LayerFlag::Visible, Some(true)).is_ok());
        assert_eq!(session.revision(), rev);
        // 두 노드 숨김은 history 한 항목입니다.
        let depth = session.history().history().len();
        assert!(set(&mut session, &[11, 12], LayerFlag::Visible, Some(false)).is_ok());
        assert_eq!(session.history().history().len(), depth + 1);
        assert_eq!(
            (flags(&session, 11), flags(&session, 12)),
            ((false, false), (false, false))
        );
        assert!(session.undo().unwrap());
        assert_eq!(
            (flags(&session, 11), flags(&session, 12)),
            ((true, false), (true, false))
        );
        // 잠금 해제는 그 노드의 잠금 명령으로 합니다. 토글은 하나라도 풀려 있으면 모두 잠급니다.
        assert!(set(&mut session, &[30], LayerFlag::Locked, Some(false)).is_ok());
        assert_eq!(flags(&session, 30), (true, false));
        assert!(set(&mut session, &[11, 30], LayerFlag::Locked, None).is_ok());
        assert!(flags(&session, 11).1 && flags(&session, 30).1);
        assert!(set(&mut session, &[11, 30], LayerFlag::Locked, None).is_ok());
        assert!(!flags(&session, 11).1 && !flags(&session, 30).1);
        assert!(set(&mut session, &[11], LayerFlag::Visible, None).is_ok());
        assert_eq!(flags(&session, 11), (false, false));
    }

    #[test]
    fn outline_lists_every_page_and_view_in_definition_order() {
        let dir = saved_project("view-outline");
        let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        assert_eq!(
            outline(session.document().unwrap().model()),
            [OutlinePage {
                id: sid(2),
                name: "페이지".into(),
                views: vec![(sid(3), "첫 화면".into()), (sid(4), "둘째 화면".into())],
            }]
        );
    }

    #[test]
    fn layer_selection_and_history_are_read_together_across_undo() {
        let dir = saved_project("layer-selection-history");
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let revision = session.revision();
        let page = |session: &ProjectWindowSession| {
            read_page(session, sid(3), revision, None, None).unwrap()
        };
        for id in [11, 12] {
            session
                .select(stellar_commands::selection::Selection::new([sid(id)], None))
                .unwrap();
            let current = page(&session);
            assert_eq!(current.history_sequence, session.history_sequence());
            assert_eq!(current.primary, Some(sid(id)));
            assert_eq!(
                current
                    .rows
                    .iter()
                    .find(|r| r.selection_order == Some(0))
                    .unwrap()
                    .id,
                sid(id)
            );
        }
        let second = page(&session);
        assert!(session.undo().unwrap());
        let first = page(&session);
        assert_ne!(second.history_sequence, first.history_sequence);
        assert_eq!(first.primary, Some(sid(11)));
        assert!(session.redo().unwrap());
        assert_eq!(page(&session).history_sequence, second.history_sequence);
    }

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
