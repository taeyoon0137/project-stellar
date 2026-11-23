//! 창별 캔버스 선택 입력 처리 (XII.10.1·XII.10.3\~XII.10.7·XII.10.40, ADR-0104 결정 3).
//!
//! [`CanvasSelectSession`]은 창 하나의 도구 상태, 편집 범위, hover, 진행 중 제스처를 갖고 [`CanvasInput`]을 차례로
//! 처리합니다. 선택 규칙은 `select*` 모듈의 순수 함수를 그대로 부르고, 기록은 [`GestureRecorder`]·`WindowHistory`로
//! 합니다. 커서 아래 hit 목록([`PointStack`])과 node 상자([`CanvasGeometry`])는 호출자가 event마다 넘깁니다(scene hit
//! test는 render 쪽 snapshot에서 계산).
//!
//! Move 도구의 pointer down은 [`plan_press`]로 처리 계획을 정하고, 끌기 시작 거리([`DRAG_THRESHOLD_PT`])를 넘으면
//! [`SessionOutput::DragStarted`]를 냅니다. 끄는 동안의 문서 편집(이동·크기, XII.10.9·XII.10.13)은 호출자가
//! [`CanvasSelectSession::preview`]로 넣고, pointer up이 한 제스처 한 history 규칙으로 기록합니다.

use stellar_commands::change::ChangeSet;
use stellar_commands::selection::{AreaSelectionError, Selection};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;
use stellar_history::entry::{HistoryError, HistorySequence};
use stellar_history::window::WindowHistory;

use super::hit::{CanvasGeometry, PointStack};
use super::select::{escape, select_all, EditingScope, EscapeStep, SelectTree, SelectionChange};
use super::select_area::Marquee;
use super::select_gesture::{GestureRecord, GestureRecorder};
use super::select_hover::HoverState;
use super::select_navigate::{cycle_sibling, double_click, enter, exit_to_parent, Navigation};
use super::select_through::{plan_press, PressPlan};
use super::tool::{CanvasKey, CanvasTool, CanvasToolState, Modifiers, ToolEvent, ToolInput};
use super::tool_units::DRAG_THRESHOLD_PT;

/// 캔버스 입력 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasInput {
    /// 누름. `clicks`는 운영체제 클릭 수(2 이상이면 더블클릭)입니다.
    PointerDown {
        /// view 좌표.
        view: Vec2,
        /// world 좌표.
        world: Vec2,
        /// 수식키.
        modifiers: Modifiers,
        /// 클릭 수.
        clicks: u8,
    },
    /// 이동(누른 채면 제스처, 아니면 hover).
    PointerMove {
        /// view 좌표.
        view: Vec2,
        /// world 좌표.
        world: Vec2,
        /// 수식키.
        modifiers: Modifiers,
    },
    /// 뗌.
    PointerUp {
        /// world 좌표.
        world: Vec2,
    },
    /// 운영체제 pointer cancel.
    PointerCancel,
    /// pointer가 캔버스를 떠남.
    PointerExited,
    /// 키·수식키·초점 입력.
    Key(ToolInput),
}

/// 입력 처리 오류. 오류가 나도 진행 중 제스처 상태는 정리됩니다.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// history 기록·preview 오류.
    History(HistoryError),
    /// 영역 선택 preview 오류.
    Area(AreaSelectionError),
}

impl From<HistoryError> for SessionError {
    fn from(error: HistoryError) -> SessionError {
        SessionError::History(error)
    }
}

impl From<AreaSelectionError> for SessionError {
    fn from(error: AreaSelectionError) -> SessionError {
        SessionError::Area(error)
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::History(error) => write!(f, "history: {error}"),
            SessionError::Area(error) => write!(f, "영역 선택: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 입력 처리 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutput {
    /// 바뀐 것이 없음.
    None,
    /// 실효 도구가 바뀜.
    ToolChanged {
        /// 이전 도구.
        from: CanvasTool,
        /// 새 도구.
        to: CanvasTool,
    },
    /// 제스처 중 Space(생성 드래그 시작점 이동)를 진행 중 제스처에 넘김.
    SpaceInGesture {
        /// 누름이면 참.
        pressed: bool,
    },
    /// hover가 바뀜.
    HoverChanged(Option<StableId>),
    /// 제스처가 진행 중임(누름 직후·preview 변경). 기록은 아직 없습니다.
    Pending,
    /// 선택을 끄는 드래그가 시작됨. 끌 node는 현재 선택입니다.
    DragStarted {
        /// 끄는 선택.
        selection: Vec<StableId>,
    },
    /// 영역 선택 preview가 바뀜.
    MarqueeChanged,
    /// 선택만 바뀐 기록(선택 항목 하나).
    Selected(HistorySequence),
    /// 제스처의 문서 항목 하나(제스처 전·후 선택 포함).
    Edited(HistorySequence),
    /// 범위만 바뀜(기록 없음).
    ScopeChanged,
    /// 제스처를 취소함(기록 없음).
    Cancelled,
    /// Text·Path 내용 편집 요청.
    EditContent(StableId),
    /// 처리하지 않은 입력(호출자의 다음 처리로).
    NotConsumed,
}

#[derive(Debug)]
enum Active {
    /// 누른 채 끌기 시작 거리 안.
    Pressed {
        recorder: GestureRecorder,
        start: Vec2,
        /// 움직이지 않고 놓으면 적용할 선택(선택 유지 계획).
        on_click: Option<SelectionChange>,
    },
    /// 선택을 끄는 중.
    Dragging { recorder: GestureRecorder },
    /// 영역 선택.
    Marquee(Box<Marquee>),
}

/// 창 하나의 캔버스 선택 입력 상태. 저장·history 대상이 아닌 view 상태만 가집니다.
#[derive(Debug, Default)]
pub struct CanvasSelectSession {
    tools: CanvasToolState,
    scope: Option<StableId>,
    hover: HoverState,
    active: Option<Active>,
    next_gesture: u64,
}

/// event마다 넘기는 hit 정보.
#[derive(Debug, Clone, Copy)]
pub struct CanvasHits<'a> {
    /// 커서 아래 node 목록.
    pub stack: &'a PointStack,
    /// node world 상자.
    pub geometry: &'a CanvasGeometry,
}

fn recorded(record: GestureRecord) -> SessionOutput {
    match record {
        GestureRecord::Document(sequence) => SessionOutput::Edited(sequence),
        GestureRecord::Selection(sequence) => SessionOutput::Selected(sequence),
        GestureRecord::Nothing => SessionOutput::None,
    }
}

impl CanvasSelectSession {
    /// Move 도구, 최상위 범위, hover 없음.
    pub fn new() -> CanvasSelectSession {
        CanvasSelectSession::default()
    }

    /// 도구 상태.
    pub fn tools(&self) -> &CanvasToolState {
        &self.tools
    }

    /// 도구를 고릅니다(도구 막대·메뉴).
    pub fn select_tool(&mut self, tool: CanvasTool) -> ToolEvent {
        self.tools.select(tool)
    }

    /// 편집 범위.
    pub fn scope(&self, tree: &SelectTree<'_>) -> EditingScope {
        match self.scope {
            Some(node) => EditingScope::at(tree, node),
            None => EditingScope::top(tree),
        }
    }

    /// hover 상태.
    pub fn hover(&self) -> &HoverState {
        &self.hover
    }

    /// 사이드바 행 hover(캔버스 hover와 같은 상태).
    pub fn hover_sidebar(&mut self, tree: &SelectTree<'_>, row: Option<StableId>) -> bool {
        self.hover.update_sidebar(tree, row)
    }

    /// 사이드바 등 밖에서 선택이 바뀐 뒤 범위를 선택에 맞춥니다(기록 없음).
    pub fn refit_scope(&mut self, tree: &SelectTree<'_>, selection: &Selection) {
        self.scope = Some(self.scope(tree).refit(tree, selection).node(tree));
    }

    /// 제스처가 진행 중인지.
    pub fn gesture_active(&self) -> bool {
        self.active.is_some()
    }

    /// 끄는 중 문서 preview(이동·크기 편집, XII.10.9·XII.10.13이 만듦).
    pub fn preview(
        &mut self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        match &self.active {
            Some(Active::Dragging { recorder }) | Some(Active::Pressed { recorder, .. }) => {
                recorder.preview(history, doc, transaction)
            }
            _ => Err(HistoryError::NoPreview),
        }
    }

    /// 입력 하나를 처리합니다. `root`는 활성 View의 root Frame입니다.
    pub fn handle(
        &mut self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        root: StableId,
        hits: CanvasHits<'_>,
        input: CanvasInput,
    ) -> Result<SessionOutput, SessionError> {
        match input {
            CanvasInput::Key(key) => self.key(history, doc, root, key),
            CanvasInput::PointerDown {
                view,
                world,
                modifiers,
                clicks,
            } => self.down(history, doc, root, hits, (view, world), modifiers, clicks),
            CanvasInput::PointerMove {
                view,
                world,
                modifiers,
            } => self.moved(history, doc, root, hits, (view, world), modifiers),
            CanvasInput::PointerUp { world: _ } => self.up(history, doc, root),
            CanvasInput::PointerCancel => self.cancel(history, doc),
            CanvasInput::PointerExited => Ok(if self.hover.clear() {
                SessionOutput::HoverChanged(None)
            } else {
                SessionOutput::None
            }),
        }
    }

    fn record_change(
        &mut self,
        history: &mut WindowHistory,
        doc: &DocumentState,
        tree: &SelectTree<'_>,
        change: SelectionChange,
    ) -> Result<SessionOutput, SessionError> {
        let scope_before = self.scope(tree);
        self.scope = Some(change.scope.node(tree));
        match change.event(history.view().selection()) {
            Some(event) => Ok(match history.record_selection(doc, &event)? {
                Some(sequence) => SessionOutput::Selected(sequence),
                None => SessionOutput::None,
            }),
            None if change.scope != scope_before => Ok(SessionOutput::ScopeChanged),
            None => Ok(SessionOutput::None),
        }
    }

    fn navigation(
        &mut self,
        history: &mut WindowHistory,
        doc: &DocumentState,
        tree: &SelectTree<'_>,
        navigation: Navigation,
    ) -> Result<SessionOutput, SessionError> {
        match navigation {
            Navigation::Changed(change) => self.record_change(history, doc, tree, change),
            Navigation::EditContent(node) => Ok(SessionOutput::EditContent(node)),
            Navigation::Unchanged => Ok(SessionOutput::None),
        }
    }

    fn key(
        &mut self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        root: StableId,
        input: ToolInput,
    ) -> Result<SessionOutput, SessionError> {
        if input == ToolInput::FocusLost && self.active.is_some() {
            // 초점을 잃으면 pointer up이 오지 않으므로 진행 중 제스처를 취소합니다(ADR-0104 결정 5.1). 제스처가 끝난
            // 뒤 수식키·Space를 지웁니다.
            let output = self.cancel(history, doc)?;
            self.tools.handle(input);
            return Ok(output);
        }
        let event = self.tools.handle(input);
        match event {
            ToolEvent::Changed { from, to } => return Ok(SessionOutput::ToolChanged { from, to }),
            ToolEvent::SpaceInGesture { pressed } => {
                return Ok(SessionOutput::SpaceInGesture { pressed })
            }
            ToolEvent::Unchanged => return Ok(SessionOutput::None),
            ToolEvent::NotConsumed => {}
        }
        let ToolInput::KeyDown { key, modifiers, .. } = input else {
            return Ok(SessionOutput::NotConsumed);
        };
        if key == CanvasKey::Escape && self.active.is_some() {
            return self.cancel(history, doc);
        }
        let base = self.tools.base();
        let tree = SelectTree::new(doc.model(), root);
        let scope = self.scope(&tree);
        let current = history.view().selection().clone();
        match key {
            CanvasKey::Escape => match escape(&tree, scope, &current, &mut self.tools) {
                EscapeStep::CancelGesture => self.cancel(history, doc),
                EscapeStep::ResetTool => Ok(SessionOutput::ToolChanged {
                    from: base,
                    to: CanvasTool::Move,
                }),
                EscapeStep::ScopeUp(change) | EscapeStep::Deselect(change) => {
                    self.record_change(history, doc, &tree, change)
                }
                EscapeStep::Nothing => Ok(SessionOutput::None),
            },
            _ if self.active.is_some() => Ok(SessionOutput::NotConsumed),
            CanvasKey::Return if modifiers.shift => {
                let nav = exit_to_parent(&tree, scope, &current);
                self.navigation(history, doc, &tree, nav)
            }
            CanvasKey::Return => {
                let nav = enter(&tree, scope, &current);
                self.navigation(history, doc, &tree, nav)
            }
            CanvasKey::Tab => {
                let nav = cycle_sibling(&tree, scope, &current, modifiers.shift);
                self.navigation(history, doc, &tree, nav)
            }
            CanvasKey::Character(c) if c.eq_ignore_ascii_case(&'a') && modifiers.command => {
                let change = select_all(&tree, scope);
                self.record_change(history, doc, &tree, change)
            }
            _ => Ok(SessionOutput::NotConsumed),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn down(
        &mut self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        root: StableId,
        hits: CanvasHits<'_>,
        (view, world): (Vec2, Vec2),
        modifiers: Modifiers,
        clicks: u8,
    ) -> Result<SessionOutput, SessionError> {
        if self.active.is_some() || self.tools.current() != CanvasTool::Move {
            return Ok(SessionOutput::NotConsumed);
        }
        let modifiers = Modifiers {
            command: modifiers.command || self.tools.deep_select(),
            ..modifiers
        };
        let tree = SelectTree::new(doc.model(), root);
        let scope = self.scope(&tree);
        let current = history.view().selection().clone();
        if clicks >= 2 {
            let nav = double_click(&tree, scope, &current, hits.stack, modifiers);
            return self.navigation(history, doc, &tree, nav);
        }
        let plan = plan_press(
            &tree,
            scope,
            &current,
            hits.stack,
            hits.geometry,
            world,
            modifiers,
        );
        self.tools.handle(ToolInput::GestureStarted);
        match plan {
            PressPlan::Marquee => {
                let marquee = match Marquee::begin(
                    &tree,
                    scope,
                    hits.geometry,
                    history.view_mut(),
                    world,
                    modifiers,
                ) {
                    Ok(marquee) => marquee,
                    Err(error) => {
                        self.tools.handle(ToolInput::GestureEnded);
                        return Err(error.into());
                    }
                };
                self.active = Some(Active::Marquee(Box::new(marquee)));
            }
            PressPlan::SelectAndDrag(change) => {
                let recorder = self.begin(history)?;
                recorder.select(history, doc, &change.selection);
                self.scope = Some(change.scope.node(&tree));
                self.active = Some(Active::Pressed {
                    recorder,
                    start: view,
                    on_click: None,
                });
            }
            PressPlan::KeepSelection { on_click } => {
                let recorder = self.begin(history)?;
                self.active = Some(Active::Pressed {
                    recorder,
                    start: view,
                    on_click: Some(on_click),
                });
            }
        }
        Ok(SessionOutput::Pending)
    }

    fn begin(&mut self, history: &mut WindowHistory) -> Result<GestureRecorder, HistoryError> {
        self.next_gesture += 1;
        match GestureRecorder::begin(history, self.next_gesture) {
            Ok(recorder) => Ok(recorder),
            Err(error) => {
                self.tools.handle(ToolInput::GestureEnded);
                Err(error)
            }
        }
    }

    fn moved(
        &mut self,
        history: &mut WindowHistory,
        doc: &DocumentState,
        root: StableId,
        hits: CanvasHits<'_>,
        (view, world): (Vec2, Vec2),
        modifiers: Modifiers,
    ) -> Result<SessionOutput, SessionError> {
        match self.active.take() {
            None => {
                if self.tools.current() != CanvasTool::Move {
                    return Ok(SessionOutput::None);
                }
                let tree = SelectTree::new(doc.model(), root);
                let modifiers = Modifiers {
                    command: modifiers.command || self.tools.deep_select(),
                    ..modifiers
                };
                let scope = self.scope(&tree);
                Ok(
                    if self
                        .hover
                        .update_canvas(&tree, scope, hits.stack, modifiers)
                    {
                        SessionOutput::HoverChanged(self.hover.node())
                    } else {
                        SessionOutput::None
                    },
                )
            }
            Some(Active::Pressed {
                recorder,
                start,
                on_click,
            }) => {
                let distance = (view.x() - start.x()).hypot(view.y() - start.y());
                if distance <= DRAG_THRESHOLD_PT {
                    self.active = Some(Active::Pressed {
                        recorder,
                        start,
                        on_click,
                    });
                    return Ok(SessionOutput::Pending);
                }
                let selection = history.view().selection().ids().to_vec();
                self.active = Some(Active::Dragging { recorder });
                Ok(SessionOutput::DragStarted { selection })
            }
            Some(Active::Dragging { recorder }) => {
                self.active = Some(Active::Dragging { recorder });
                Ok(SessionOutput::Pending)
            }
            Some(Active::Marquee(mut marquee)) => {
                let result = marquee.update(history.view_mut(), world);
                self.active = Some(Active::Marquee(marquee));
                result?;
                Ok(SessionOutput::MarqueeChanged)
            }
        }
    }

    fn up(
        &mut self,
        history: &mut WindowHistory,
        doc: &DocumentState,
        root: StableId,
    ) -> Result<SessionOutput, SessionError> {
        let Some(active) = self.active.take() else {
            return Ok(SessionOutput::None);
        };
        self.tools.handle(ToolInput::GestureEnded);
        let tree = SelectTree::new(doc.model(), root);
        match active {
            Active::Pressed {
                recorder, on_click, ..
            } => {
                if let Some(change) = on_click {
                    recorder.select(history, doc, &change.selection);
                    self.scope = Some(change.scope.node(&tree));
                }
                Ok(recorded(recorder.finish(history, doc)?))
            }
            Active::Dragging { recorder } => Ok(recorded(recorder.finish(history, doc)?)),
            Active::Marquee(marquee) => {
                let result = marquee.finish(&tree, history.view_mut())?;
                self.scope = Some(result.scope.node(&tree));
                match result.event {
                    Some(event) => Ok(match history.record_selection(doc, &event)? {
                        Some(sequence) => SessionOutput::Selected(sequence),
                        None => SessionOutput::None,
                    }),
                    None => Ok(SessionOutput::None),
                }
            }
        }
    }

    fn cancel(
        &mut self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
    ) -> Result<SessionOutput, SessionError> {
        let Some(active) = self.active.take() else {
            return Ok(SessionOutput::None);
        };
        self.tools.handle(ToolInput::GestureEnded);
        match active {
            Active::Pressed { recorder, .. } | Active::Dragging { recorder } => {
                recorder.cancel(history, doc)?;
            }
            Active::Marquee(marquee) => {
                marquee.cancel(history.view_mut())?;
            }
        }
        Ok(SessionOutput::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::point_stack;
    use crate::canvas::select_fixtures::{nested, save_bytes, scene_of, sid, view_point, Scene};
    use crate::window_session::test_fixtures::window;
    use stellar_commands::command::Command;
    use stellar_commands::id::{CommandId, CommandOrigin};
    use stellar_commands::property::{PropertyValue, SetProperty};
    use stellar_core_types::value::FiniteF32;
    use stellar_document::frame::keys as frame_keys;
    use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};

    struct Rig {
        session: CanvasSelectSession,
        history: WindowHistory,
        doc: DocumentState,
        root: StableId,
        scene: Scene,
        geometry: CanvasGeometry,
    }

    impl Rig {
        fn new() -> Rig {
            let fixture = nested();
            let scene = scene_of(&fixture.model, fixture.root);
            Rig {
                session: CanvasSelectSession::new(),
                history: WindowHistory::new(window(11)),
                geometry: CanvasGeometry::from_snapshot(&scene.snapshot),
                root: fixture.root,
                doc: DocumentState::new(fixture.model),
                scene,
            }
        }

        fn send(&mut self, x: f32, y: f32, input: CanvasInput) -> SessionOutput {
            let stack = point_stack(
                &self.scene.index,
                &self.scene.viewport,
                view_point(x, y),
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly,
            );
            let hits = CanvasHits {
                stack: &stack,
                geometry: &self.geometry,
            };
            self.session
                .handle(&mut self.history, &mut self.doc, self.root, hits, input)
                .unwrap()
        }

        fn down(&mut self, x: f32, y: f32, modifiers: Modifiers, clicks: u8) -> SessionOutput {
            let p = view_point(x, y);
            self.send(
                x,
                y,
                CanvasInput::PointerDown {
                    view: p,
                    world: p,
                    modifiers,
                    clicks,
                },
            )
        }

        fn mv(&mut self, x: f32, y: f32, modifiers: Modifiers) -> SessionOutput {
            let p = view_point(x, y);
            self.send(
                x,
                y,
                CanvasInput::PointerMove {
                    view: p,
                    world: p,
                    modifiers,
                },
            )
        }

        fn up(&mut self, x: f32, y: f32) -> SessionOutput {
            self.send(
                x,
                y,
                CanvasInput::PointerUp {
                    world: view_point(x, y),
                },
            )
        }

        fn key(&mut self, key: CanvasKey, modifiers: Modifiers) -> SessionOutput {
            self.send(
                0.,
                0.,
                CanvasInput::Key(ToolInput::KeyDown {
                    key,
                    modifiers,
                    repeat: false,
                }),
            )
        }

        fn selected(&self) -> Vec<StableId> {
            self.history.view().selection().ids().to_vec()
        }

        fn scope(&self) -> StableId {
            let tree = SelectTree::new(self.doc.model(), self.root);
            self.session.scope(&tree).node(&tree)
        }
    }

    fn width_tx(doc: &DocumentState, id: u64, width: f32) -> CommandTransaction {
        CommandTransaction::new(CommandId::from_stable_id(sid(id)), CommandOrigin::User)
            .with_command(
                doc.revision(),
                Command::SetProperty(SetProperty {
                    node: sid(11),
                    key: frame_keys::WIDTH,
                    value: PropertyValue::NonNegative(FiniteF32::new(width).unwrap()),
                }),
            )
    }

    #[test]
    fn input_sequence_drives_select_hover_scope_marquee_and_history() {
        let mut rig = Rig::new();
        let none = Modifiers::NONE;
        // hover는 클릭 대상과 같고 기록이 없습니다.
        assert_eq!(
            rig.mv(130., 130., none),
            SessionOutput::HoverChanged(Some(sid(11)))
        );
        // 클릭: 선택 1건.
        assert_eq!(rig.down(130., 130., none, 1), SessionOutput::Pending);
        assert!(matches!(rig.up(130., 130.), SessionOutput::Selected(_)));
        assert_eq!(rig.selected(), vec![sid(11)]);
        // 더블클릭: 11 안의 13.
        assert!(matches!(
            rig.down(130., 130., none, 2),
            SessionOutput::Selected(_)
        ));
        assert_eq!((rig.selected(), rig.scope()), (vec![sid(13)], sid(11)));
        // Tab: 형제 14, Shift-Return: 부모 11, Return: 자식 [13, 14].
        assert!(matches!(
            rig.key(CanvasKey::Tab, none),
            SessionOutput::Selected(_)
        ));
        assert_eq!(rig.selected(), vec![sid(14)]);
        assert!(matches!(
            rig.key(CanvasKey::Return, Modifiers::SHIFT),
            SessionOutput::Selected(_)
        ));
        assert_eq!((rig.selected(), rig.scope()), (vec![sid(11)], sid(10)));
        assert!(matches!(
            rig.key(CanvasKey::Return, none),
            SessionOutput::Selected(_)
        ));
        assert_eq!(
            (rig.selected(), rig.scope()),
            (vec![sid(13), sid(14)], sid(11))
        );
        // ⌘A: 범위 형제 전체(이미 같으면 기록 없음).
        assert_eq!(
            rig.key(CanvasKey::Character('a'), Modifiers::COMMAND),
            SessionOutput::None
        );
        // Escape: 범위 위로(11 선택), 다시 Escape: 해제.
        assert!(matches!(
            rig.key(CanvasKey::Escape, none),
            SessionOutput::Selected(_)
        ));
        assert_eq!((rig.selected(), rig.scope()), (vec![sid(11)], sid(10)));
        assert!(matches!(
            rig.key(CanvasKey::Escape, none),
            SessionOutput::Selected(_)
        ));
        assert!(rig.selected().is_empty());
        let selection_entries = rig.history.history().len();
        assert_eq!(selection_entries, 7);
        // 빈 곳 누름 → 마퀴 → 1건.
        assert_eq!(rig.down(290., 10., none, 1), SessionOutput::Pending);
        assert_eq!(rig.mv(430., 150., none), SessionOutput::MarqueeChanged);
        assert!(matches!(rig.up(430., 150.), SessionOutput::Selected(_)));
        assert_eq!(rig.selected(), vec![sid(11), sid(12)]);
        // 선택한 11을 끌어 편집: 문서 1건, 선택은 그대로.
        let before = save_bytes(&rig.doc);
        assert_eq!(rig.down(130., 130., none, 1), SessionOutput::Pending);
        assert_eq!(
            rig.mv(131., 131., none),
            SessionOutput::Pending,
            "끌기 시작 거리 안"
        );
        assert_eq!(
            rig.mv(160., 160., none),
            SessionOutput::DragStarted {
                selection: vec![sid(11), sid(12)]
            }
        );
        for step in 0..10 {
            let tx = width_tx(&rig.doc, 7_000 + step, 210. + step as f32);
            rig.session
                .preview(&mut rig.history, &mut rig.doc, &tx)
                .unwrap();
        }
        assert!(matches!(rig.up(160., 160.), SessionOutput::Edited(_)));
        assert_ne!(save_bytes(&rig.doc), before);
        assert_eq!(rig.history.history().len(), selection_entries + 2);
        // Escape는 진행 중 제스처를 먼저 취소합니다.
        let len = rig.history.history().len();
        rig.down(310., 30., none, 1);
        rig.mv(350., 60., none);
        let tx = width_tx(&rig.doc, 8_000, 999.);
        rig.session
            .preview(&mut rig.history, &mut rig.doc, &tx)
            .unwrap();
        assert_eq!(rig.key(CanvasKey::Escape, none), SessionOutput::Cancelled);
        assert_eq!(rig.history.history().len(), len);
        assert_eq!(rig.selected(), vec![sid(11), sid(12)]);
        assert!(!rig.session.gesture_active());
        // 초점 상실도 진행 중 제스처(끌기·마퀴)를 취소하고 기록하지 않습니다.
        let before = save_bytes(&rig.doc);
        rig.down(310., 30., none, 1);
        rig.mv(350., 60., none);
        let tx = width_tx(&rig.doc, 8_001, 777.);
        rig.session
            .preview(&mut rig.history, &mut rig.doc, &tx)
            .unwrap();
        assert_eq!(
            rig.send(0., 0., CanvasInput::Key(ToolInput::FocusLost)),
            SessionOutput::Cancelled
        );
        assert!(!rig.session.gesture_active() && !rig.session.tools().gesture_active());
        assert_eq!(save_bytes(&rig.doc), before);
        assert_eq!(rig.history.history().len(), len);
        assert_eq!(rig.selected(), vec![sid(11), sid(12)]);
        rig.down(700., 500., none, 1);
        rig.mv(780., 580., none);
        assert!(rig.history.view().area_selection_active());
        assert_eq!(
            rig.send(0., 0., CanvasInput::Key(ToolInput::FocusLost)),
            SessionOutput::Cancelled
        );
        assert!(!rig.history.view().area_selection_active());
        assert_eq!(rig.history.history().len(), len);
        // 다음 제스처는 정상으로 시작합니다.
        assert_eq!(rig.down(130., 130., none, 1), SessionOutput::Pending);
        rig.up(130., 130.);
    }

    #[test]
    fn tools_and_temporary_hand_route_pointer_input() {
        let mut rig = Rig::new();
        // Frame 도구에서는 선택 입력을 쓰지 않고, Escape가 Move로 돌립니다.
        assert!(matches!(
            rig.key(CanvasKey::Character('f'), Modifiers::NONE),
            SessionOutput::ToolChanged { .. }
        ));
        assert_eq!(
            rig.down(130., 130., Modifiers::NONE, 1),
            SessionOutput::NotConsumed
        );
        assert!(matches!(
            rig.key(CanvasKey::Escape, Modifiers::NONE),
            SessionOutput::ToolChanged { .. }
        ));
        assert_eq!(rig.session.tools().current(), CanvasTool::Move);
        // Space를 누르는 동안 Hand라 누름은 선택이 아닙니다.
        assert!(matches!(
            rig.key(CanvasKey::Space, Modifiers::NONE),
            SessionOutput::ToolChanged { .. }
        ));
        assert_eq!(
            rig.down(130., 130., Modifiers::NONE, 1),
            SessionOutput::NotConsumed
        );
        // ⌘를 누르고 있으면 깊은 선택.
        rig.send(
            0.,
            0.,
            CanvasInput::Key(ToolInput::KeyUp {
                key: CanvasKey::Space,
                modifiers: Modifiers::NONE,
            }),
        );
        rig.send(
            0.,
            0.,
            CanvasInput::Key(ToolInput::ModifiersChanged(Modifiers::COMMAND)),
        );
        assert_eq!(
            rig.mv(220., 220., Modifiers::NONE),
            SessionOutput::HoverChanged(Some(sid(17)))
        );
        rig.down(220., 220., Modifiers::NONE, 1);
        rig.up(220., 220.);
        assert_eq!((rig.selected(), rig.scope()), (vec![sid(17)], sid(14)));
        assert_eq!(
            rig.send(0., 0., CanvasInput::PointerExited),
            SessionOutput::HoverChanged(None)
        );
        assert_eq!(
            rig.history.history().len(),
            1,
            "도구·hover 전환은 기록 없음"
        );
        // 사이드바 선택 뒤 범위 맞춤.
        let tree = SelectTree::new(rig.doc.model(), rig.root);
        rig.session
            .refit_scope(&tree, &Selection::new([sid(19)], None));
        assert_eq!(rig.session.scope(&tree).node(&tree), sid(12));
        assert!(rig.session.hover_sidebar(&tree, Some(sid(18))));
        assert_eq!(rig.session.hover().node(), Some(sid(18)));
    }
}
