//! 키보드만으로 하는 캔버스 편집 경로 코어 (XII.10.41, ADR-0118 결정 6).
//!
//! - **포커스.** [`CanvasFocus`]는 캔버스 키 초점과 초점 표시(focus ring)를 갖습니다. 키보드(Tab 이동·Full Keyboard
//!   Access)로 초점을 받으면 표시하고, 캔버스 pointer 입력이 오면 숨깁니다. 저장·history 대상이 아닙니다.
//! - **키 배정.** [`keyboard_command`]가 키 하나를 동작 하나로 정합니다. 단위와 수식키는 ADR-0104 결정 2 표
//!   (`tool_units`)입니다: 화살표 이동 1px·Shift 10px, ⌘-화살표 크기 1px·⌘⇧ 10px. Tab·Shift-Tab·Return·Shift-Return·
//!   Escape·⌘A는 선택 엔진(ADR-0104 결정 3.4·3.5)에 그대로 넘깁니다. 문맥 메뉴는 Shift-F10, 문맥 메뉴 키, ⌃Return입니다.
//! - **실행.** 선택 이동은 [`select_key`]가 `CanvasSelectSession`에, 이동·크기는 [`transform_key`]가 기존 키보드
//!   변형 경로(`transform::keyboard`, XII.10.14)에 넘깁니다. 같은 규칙을 다시 구현하지 않습니다.
//! - **삽입.** Insert 메뉴는 화면 가운데에 기본 크기(가로·세로 [`INSERT_SIZE_PX`])로 만듭니다. [`plan_insert_at_center`]는
//!   부모 local 상자·두 점·Text 자리만 정하고, 실제 삽입은 기존 삽입 경로(Frame·Shape 삽입, Text 편집기)가 합니다.
//! - **핸들 대체.** 핸들로만 할 수 있는 동작이 없도록 핸들마다 키 또는 인스펙터 CSS 속성을 [`handle_equivalents`]로
//!   연결합니다.

use stellar_commands::state::DocumentState;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::shape::ShapeKind;
use stellar_history::window::WindowHistory;
use stellar_scene::visibility::Viewport;

use super::handles::CanvasHandle;
use super::hit::{CanvasGeometry, PointStack};
use super::select::SelectTree;
use super::select_session::{
    CanvasHits, CanvasInput, CanvasSelectSession, SessionError, SessionOutput,
};
use super::text::edit::{TextPlacement, TextWidth};
use super::tool::{CanvasKey, Modifiers, ToolInput};
use super::tool_units::{nudge, resize_step};
use super::transform::keyboard::{arrow, resize_key, Arrow, KeyOutcome};
use super::transform::TransformError;
use super::CanvasScene;
use crate::app::RequestId;
use crate::window_session::ProjectWindowSession;

/// Insert 메뉴로 만드는 node의 기본 가로·세로(부모 local px). Line은 가로 길이입니다.
pub const INSERT_SIZE_PX: f32 = 100.0;

/// 선택이 없을 때 화살표 한 번의 camera 이동(화면 pt). Shift는 [`PAN_LARGE_PT`].
pub const PAN_PT: f64 = 40.0;

/// Shift-화살표의 camera 이동(화면 pt).
pub const PAN_LARGE_PT: f64 = 200.0;

/// 캔버스 키 초점과 초점 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CanvasFocus {
    focused: bool,
    ring: bool,
}

impl CanvasFocus {
    /// 초점을 받음. 키보드로 받았으면 초점 표시를 켭니다.
    pub fn focus(&mut self, by_keyboard: bool) {
        self.focused = true;
        self.ring = by_keyboard;
    }

    /// 초점을 잃음.
    pub fn blur(&mut self) {
        *self = CanvasFocus::default();
    }

    /// 캔버스 pointer 입력: 초점은 두고 표시만 끕니다.
    pub fn pointer_input(&mut self) {
        self.ring = false;
    }

    /// 키 입력: 초점이 있으면 표시를 켭니다(키보드 사용 중).
    pub fn key_input(&mut self) {
        if self.focused {
            self.ring = true;
        }
    }

    /// 초점이 있는지.
    pub const fn focused(&self) -> bool {
        self.focused
    }

    /// 초점 표시를 그릴지.
    pub const fn ring_visible(&self) -> bool {
        self.focused && self.ring
    }
}

/// 캔버스가 보는 편집 키.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardKey {
    /// 화살표.
    Arrow(Arrow),
    /// Tab.
    Tab,
    /// Return.
    Return,
    /// Escape.
    Escape,
    /// F10(Shift-F10은 문맥 메뉴).
    F10,
    /// 문맥 메뉴 키.
    ContextMenu,
    /// 글자 키.
    Character(char),
}

/// 키 하나의 동작.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyboardCommand {
    /// 선택 엔진 키(Tab·Shift-Tab 형제 순환, Return 안으로, Shift-Return 부모, Escape, ⌘A).
    Select(CanvasKey, Modifiers),
    /// 선택 이동(부모 공간 px).
    Move(Arrow, f32),
    /// 선택 크기 조절(오른쪽·아래 변, px).
    Resize(Arrow, f32),
    /// 선택(없으면 화면 가운데) 위치에서 문맥 메뉴 열기.
    OpenContextMenu,
}

/// 키 하나를 동작으로 정합니다. 캔버스가 쓰지 않는 키는 `None`입니다.
pub fn keyboard_command(key: KeyboardKey, modifiers: Modifiers) -> Option<KeyboardCommand> {
    let plain = Modifiers {
        shift: false,
        ..modifiers
    } == Modifiers::NONE;
    Some(match key {
        KeyboardKey::Arrow(direction) if plain => {
            KeyboardCommand::Move(direction, nudge(modifiers.shift))
        }
        KeyboardKey::Arrow(direction)
            if modifiers.command && !modifiers.option && !modifiers.control =>
        {
            KeyboardCommand::Resize(direction, resize_step(modifiers.shift))
        }
        KeyboardKey::Arrow(_) => return None,
        KeyboardKey::F10 if modifiers == Modifiers::SHIFT => KeyboardCommand::OpenContextMenu,
        KeyboardKey::ContextMenu => KeyboardCommand::OpenContextMenu,
        KeyboardKey::Return if modifiers.control => KeyboardCommand::OpenContextMenu,
        KeyboardKey::Return => KeyboardCommand::Select(CanvasKey::Return, modifiers),
        KeyboardKey::Tab => KeyboardCommand::Select(CanvasKey::Tab, modifiers),
        KeyboardKey::Escape => KeyboardCommand::Select(CanvasKey::Escape, modifiers),
        KeyboardKey::Character(c) if modifiers.command && c.eq_ignore_ascii_case(&'a') => {
            KeyboardCommand::Select(CanvasKey::Character(c), modifiers)
        }
        KeyboardKey::F10 | KeyboardKey::Character(_) => return None,
    })
}

/// 선택 엔진 키를 창의 선택 session에 넘깁니다(hit 정보가 필요 없는 키 입력).
pub fn select_key(
    session: &mut CanvasSelectSession,
    history: &mut WindowHistory,
    doc: &mut DocumentState,
    root: StableId,
    key: CanvasKey,
    modifiers: Modifiers,
) -> Result<SessionOutput, SessionError> {
    let stack = PointStack::empty();
    let geometry = CanvasGeometry::new();
    session.handle(
        history,
        doc,
        root,
        CanvasHits {
            stack: &stack,
            geometry: &geometry,
        },
        CanvasInput::Key(ToolInput::KeyDown {
            key,
            modifiers,
            repeat: false,
        }),
    )
}

/// 이동·크기 키를 기존 키보드 변형 경로로 실행합니다. 선택이 없으면 화살표는 camera를 움직입니다.
pub fn transform_key<C: MonotonicClock>(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    command: KeyboardCommand,
    large: bool,
    request: RequestId,
    clock: &C,
) -> Result<KeyOutcome, TransformError> {
    match command {
        KeyboardCommand::Move(direction, step) => {
            let pan = if large { PAN_LARGE_PT } else { PAN_PT };
            arrow(window, scene, direction, step, pan, request)
        }
        KeyboardCommand::Resize(direction, step) => {
            Ok(if resize_key(window, scene, direction, step, clock)? {
                KeyOutcome::Moved
            } else {
                KeyOutcome::Unchanged
            })
        }
        _ => Err(TransformError::Disabled("이동·크기 키가 아님")),
    }
}

/// 문맥 메뉴를 열 view 위치: 선택 bounds 가운데, 선택이 없으면 화면 가운데.
pub fn context_menu_anchor(
    tree: &SelectTree<'_>,
    geometry: &CanvasGeometry,
    selection: &[StableId],
    viewport: &Viewport,
) -> Option<Vec2> {
    let bounds = selection
        .iter()
        .filter_map(|node| geometry.bounds(tree, *node))
        .reduce(|a, b| a.union(&b));
    match bounds {
        Some(b) => viewport
            .view()
            .apply(Vec2::new((b.min_x() + b.max_x()) / 2., (b.min_y() + b.max_y()) / 2.).ok()?)
            .ok(),
        None => {
            let size = viewport.size();
            Vec2::new(size.width() / 2., size.height() / 2.).ok()
        }
    }
}

/// Insert 메뉴 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertKind {
    /// Frame.
    Frame,
    /// Shape.
    Shape(ShapeKind),
    /// Text(자동 폭).
    Text,
}

/// 화면 가운데 삽입 계획(부모 local 좌표).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InsertPlan {
    /// Frame 삽입 상자(`insert_canvas_frame`).
    Frame(Rect),
    /// Shape 두 점(`insert_canvas_shape`). Line은 왼쪽에서 오른쪽 수평선입니다.
    Shape(ShapeKind, [Vec2; 2]),
    /// 새 Text 자리(`CanvasTextEditor::begin_new`).
    Text(TextPlacement),
}

/// 화면 가운데(world)를 `parent` local로 옮겨 기본 크기 상자의 가운데가 그 점이 되게 정합니다. `parent_world`는
/// 부모의 local → world 변환(scene 행)입니다.
pub fn plan_insert_at_center(
    kind: InsertKind,
    parent: StableId,
    parent_world: Transform2D,
    viewport: &Viewport,
) -> Option<InsertPlan> {
    let size = viewport.size();
    let center_view = Vec2::new(size.width() / 2., size.height() / 2.).ok()?;
    let center_world = viewport.view().inverse()?.apply(center_view).ok()?;
    let center = parent_world.inverse()?.apply(center_world).ok()?;
    let half = INSERT_SIZE_PX / 2.;
    let origin = Vec2::new(center.x() - half, center.y() - half).ok()?;
    let far = Vec2::new(center.x() + half, center.y() + half).ok()?;
    Some(match kind {
        InsertKind::Frame => InsertPlan::Frame(Rect::new(
            origin,
            Size2::new(INSERT_SIZE_PX, INSERT_SIZE_PX).ok()?,
        )),
        InsertKind::Shape(ShapeKind::Line) => InsertPlan::Shape(
            ShapeKind::Line,
            [
                Vec2::new(origin.x(), center.y()).ok()?,
                Vec2::new(far.x(), center.y()).ok()?,
            ],
        ),
        InsertKind::Shape(shape) => InsertPlan::Shape(shape, [origin, far]),
        InsertKind::Text => InsertPlan::Text(TextPlacement {
            parent,
            origin: center,
            width: TextWidth::Auto,
        }),
    })
}

/// 핸들 동작의 키보드·인스펙터 대체.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Equivalent {
    /// 키 동작.
    Key(KeyboardKey, Modifiers),
    /// 인스펙터의 CSS 속성 입력란(registry 이름 또는 layout longhand 이름).
    InspectorField(&'static str),
}

const MOVE: &[Equivalent] = &[
    Equivalent::Key(KeyboardKey::Arrow(Arrow::Right), Modifiers::NONE),
    Equivalent::Key(KeyboardKey::Arrow(Arrow::Right), Modifiers::SHIFT),
    Equivalent::InspectorField("left"),
    Equivalent::InspectorField("top"),
];

const RESIZE: &[Equivalent] = &[
    Equivalent::Key(KeyboardKey::Arrow(Arrow::Right), Modifiers::COMMAND),
    Equivalent::Key(KeyboardKey::Arrow(Arrow::Down), Modifiers::SHIFT_COMMAND),
    Equivalent::InspectorField("width"),
    Equivalent::InspectorField("height"),
];

const ROTATE: &[Equivalent] = &[Equivalent::InspectorField("transform")];

const RADIUS: &[Equivalent] = &[
    Equivalent::InspectorField("border-top-left-radius"),
    Equivalent::InspectorField("border-top-right-radius"),
    Equivalent::InspectorField("border-bottom-right-radius"),
    Equivalent::InspectorField("border-bottom-left-radius"),
];

/// 핸들 하나로 하는 동작을 키보드만으로 하는 방법.
pub fn handle_equivalents(handle: CanvasHandle) -> &'static [Equivalent] {
    match handle {
        CanvasHandle::Body => MOVE,
        CanvasHandle::ResizeCorner(_) | CanvasHandle::ResizeEdge(_) => RESIZE,
        CanvasHandle::Rotate(_) => ROTATE,
        CanvasHandle::Radius(_) => RADIUS,
    }
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
