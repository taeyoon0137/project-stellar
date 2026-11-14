//! 캔버스 도구 상호작용 상태 기계 (XII.3.6).
//!
//! pointer gesture 하나(누름 → 이동\* → 뗌 또는 취소)를 도구별 닫힌 enum 상태 기계로 해석합니다.
//! 상호작용은 Pan·Zoom·Select·Drag·Resize·Rotate 여섯 가지이고([`InteractionKind`]), 한 번에 하나만
//! 진행합니다([`InteractionState`]). trait object나 열린 확장점은 없습니다.
//!
//! 상태 기계는 문서·scene snapshot·history를 바꾸지 않는 값 계산입니다. 진행 중에는 transient preview
//! ([`InteractionPreview`])만 내고, 뗌에서 결과([`InteractionOutcome`])를 한 번 냅니다. 취소는 결과
//! 없이 시작 전으로 돌아갑니다([`Cancellation`]). 결과를 camera·선택·`CommandTransaction`에 반영하는
//! 일은 호출자가 합니다(preview 주입과 pointer up 확정은 XII.3.7, camera 연결은 XII.4.3).
//!
//! # 시작 규칙
//!
//! 누름 시점의 도구와 hit 대상([`PressContext`])으로 상호작용을 정합니다.
//!
//! | 도구 | 누른 대상 | 상호작용 |
//! |---|---|---|
//! | [`InteractionTool::Hand`] | 무관 | Pan |
//! | [`InteractionTool::Zoom`] | 무관 | Zoom |
//! | [`InteractionTool::Move`] | 크기 조절 모서리·변 handle | Resize |
//! | [`InteractionTool::Move`] | 회전 영역 handle | Rotate |
//! | [`InteractionTool::Move`] | 선택된 node | Drag |
//! | [`InteractionTool::Move`] | 선택되지 않은 node, 빈 곳 | Select |
//! | [`InteractionTool::Move`] | modifier control point | 시작하지 않음(이 기계 밖) |
//!
//! # 진행 규칙
//!
//! - 모든 상호작용은 **Pressed**(누른 채 [`InteractionConfig::drag_threshold`] view px 안)에서 시작해,
//!   누른 점에서 그보다 멀어지면 **진행** 단계로 바뀝니다. 진행 단계의 값은 항상 누른 점 기준 전체
//!   이동량으로 계산합니다.
//! - Select에서 node를 누르고 threshold를 넘기면 그 node의 Drag로, 빈 곳이면 영역 선택(marquee)으로
//!   바뀝니다.
//! - 뗌은 뗀 위치로 마지막 이동을 반영한 뒤 결과를 냅니다. Pressed에서 떼면 클릭입니다. Select·Drag는
//!   [`InteractionOutcome::Click`], Zoom은 한 단계 확대, Pan·Resize·Rotate는
//!   [`InteractionOutcome::Unchanged`]입니다. 뗌은 언제나 gesture를 끝냅니다. 뗀 위치나 결과를 계산할 수
//!   없으면 결과 없이 취소로 끝내고, 오류는 [`Cancellation::failure`]에 담습니다.
//! - 취소([`InteractionController::cancel`]: 운영체제 pointer cancel, Escape, focus 상실 등)는 결과를 내지
//!   않습니다. Pan·Zoom은 되돌릴 시작 camera를 알려 줍니다.
//!
//! # 계산
//!
//! - 좌표: 입력은 캔버스 view 좌표(XII.3.1 좌표 규칙), camera는 world → view 변환입니다(viewport의
//!   view). world 값은 누름 시점 camera의 역으로 구합니다. gesture 중 camera는 바뀌지 않는다고 봅니다.
//! - Pan: 시작 camera 뒤에 view 이동량만큼 평행 이동합니다.
//! - Zoom: 누른 점 기준 배율입니다. 끌기는 오른쪽 [`InteractionConfig::zoom_drag_per_doubling`] view px마다
//!   2배(왼쪽은 축소), 클릭은 [`InteractionConfig::click_zoom_factor`]배입니다. 결과 배율
//!   (`sqrt(|det|)`)은 [`InteractionConfig::min_zoom`] 이상 [`InteractionConfig::max_zoom`] 이하로
//!   고정합니다.
//! - Drag: world 이동량만큼 평행 이동합니다.
//! - Resize: 선택 상자(world 꼭짓점 `[top_left, top_right, bottom_right, bottom_left]`)의 자기 축
//!   좌표계에서 잡은 handle 반대편 모서리·변을 고정하고 크기를 바꿉니다. 회전한 상자도 같습니다. 반대편을
//!   넘어가면 뒤집힙니다(음의 배율). 길이가 0인 축(수평·수직 선의 상자)은 배율을 1로 고정하고 다른 축만
//!   바꿉니다. 잡은 handle이 바꾸는 축이 모두 길이 0이면 시작하지 않습니다
//!   ([`InteractionError::DegenerateSelectionBox`]). 선택 상자 handle layer(XI.3.5)는 그런 handle을 만들지
//!   않습니다.
//! - Rotate: 상자 중심을 기준으로 누른 점과 현재 점의 각도 차입니다. 이동마다 ±π 안에서 이어 붙이므로
//!   한 gesture에서 반 바퀴를 넘게 돌릴 수 있습니다. view가 y 아래 방향이므로 양의 각은 화면에서 시계
//!   방향입니다.
//!
//! 이 모듈은 XII.3.6의 상태 전이 기반입니다. 수식키·일시 도구 전환은 XII.10.1·XII.10.2,
//! 캔버스 명세의 영역 드래그 줌은 XII.10.37에서 연결합니다. 현재 수평 드래그 줌은 잠정 계산이며
//! 정상 앱의 최종 편집 UX를 구현한 것이 아닙니다.
//! 값은 IEEE `f32` 기본 연산과 `hypot`·`atan2`·`sin`·`cos`·`exp2`·`sqrt`만 씁니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Transform2D, Vec2};

use crate::bounds::Aabb;
use crate::node::{non_negative, SceneValueError};
use crate::transient::{CanvasHit, Corner, Edge, HandleKind, ResizeAxes};

// ---------------------------------------------------------------------------
// 입력·설정
// ---------------------------------------------------------------------------

/// pointer gesture를 해석하는 캔버스 도구(도구 막대의 선택·탐색 도구). 만들기 도구는 이 기계 밖입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionTool {
    /// 선택·이동·크기 조절·회전(Move, V).
    Move,
    /// 화면 이동(Hand, H).
    Hand,
    /// 확대·축소(Zoom, Z).
    Zoom,
}

/// 상호작용 종류(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionKind {
    /// 화면 이동.
    Pan,
    /// 확대·축소.
    Zoom,
    /// 클릭 선택·영역 선택.
    Select,
    /// 선택 node 끌기.
    Drag,
    /// 선택 상자 크기 조절.
    Resize,
    /// 선택 상자 회전.
    Rotate,
}

/// 누른 지점의 hit 대상. [`CanvasHit`](XI.3.5)에서 상호작용에 필요한 값만 옮깁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PressTarget {
    /// 아무것도 맞지 않음.
    Empty,
    /// scene node(원본 document node).
    Node(StableId),
    /// transient handle.
    Handle(HandleKind),
}

impl PressTarget {
    /// canvas hit 결과를 누름 대상으로 바꿉니다.
    pub fn from_hit(hit: &CanvasHit) -> PressTarget {
        match hit {
            CanvasHit::Handle(hit) => PressTarget::Handle(hit.handle.kind),
            CanvasHit::Node(node) => PressTarget::Node(node.source),
            CanvasHit::Empty => PressTarget::Empty,
        }
    }
}

/// 누름 시점의 창 상태.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressContext<'a> {
    /// 현재 도구.
    pub tool: InteractionTool,
    /// world → view camera(viewport의 view 변환).
    pub camera: Transform2D,
    /// 누른 지점의 hit 대상.
    pub target: PressTarget,
    /// 현재 선택(선택 순서).
    pub selection: &'a [StableId],
    /// 현재 선택 상자의 world 꼭짓점 `[top_left, top_right, bottom_right, bottom_left]`(handle을 만든
    /// 상자, [`TransientLayer::push_selection_box`](crate::transient::TransientLayer::push_selection_box)).
    pub selection_box: Option<[Vec2; 4]>,
}

/// 상호작용 수치. 기본값은 사용성 측정 전 잠정값입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InteractionConfig {
    drag_threshold: FiniteF32,
    zoom_drag_per_doubling: FiniteF32,
    click_zoom_factor: FiniteF32,
    min_zoom: FiniteF32,
    max_zoom: FiniteF32,
}

impl InteractionConfig {
    /// 기본 끌기 threshold(view px).
    pub const DEFAULT_DRAG_THRESHOLD: f32 = 3.0;
    /// 기본 zoom 끌기 거리(2배당 view px).
    pub const DEFAULT_ZOOM_DRAG_PER_DOUBLING: f32 = 200.0;
    /// 기본 zoom 클릭 배율.
    pub const DEFAULT_CLICK_ZOOM_FACTOR: f32 = 2.0;
    /// 기본 최소 배율(2%).
    pub const DEFAULT_MIN_ZOOM: f32 = 0.02;
    /// 기본 최대 배율(25600%).
    pub const DEFAULT_MAX_ZOOM: f32 = 256.0;

    /// 수치를 검사해 만듭니다. threshold는 0 이상, 나머지는 0보다 커야 하고 `min_zoom ≤ max_zoom`입니다.
    pub fn new(
        drag_threshold: f32,
        zoom_drag_per_doubling: f32,
        click_zoom_factor: f32,
        min_zoom: f32,
        max_zoom: f32,
    ) -> Result<InteractionConfig, SceneValueError> {
        let positive = |value: f32, field: &'static str| {
            let value = non_negative(value, field)?;
            if value.get() == 0.0 {
                return Err(SceneValueError::OutOfRange { field });
            }
            Ok(value)
        };
        let config = InteractionConfig {
            drag_threshold: non_negative(drag_threshold, "drag threshold")?,
            zoom_drag_per_doubling: positive(zoom_drag_per_doubling, "zoom drag distance")?,
            click_zoom_factor: positive(click_zoom_factor, "click zoom factor")?,
            min_zoom: positive(min_zoom, "min zoom")?,
            max_zoom: positive(max_zoom, "max zoom")?,
        };
        if config.min_zoom.get() > config.max_zoom.get() {
            return Err(SceneValueError::OutOfRange {
                field: "zoom range",
            });
        }
        Ok(config)
    }

    /// 끌기로 바뀌는 누른 점과의 view 거리(px). 이 거리를 넘어야 진행 단계가 됩니다.
    pub const fn drag_threshold(&self) -> f32 {
        self.drag_threshold.get()
    }

    /// zoom 끌기에서 배율 2배에 해당하는 오른쪽 view 이동(px).
    pub const fn zoom_drag_per_doubling(&self) -> f32 {
        self.zoom_drag_per_doubling.get()
    }

    /// zoom 클릭 배율.
    pub const fn click_zoom_factor(&self) -> f32 {
        self.click_zoom_factor.get()
    }

    /// 최소 camera 배율.
    pub const fn min_zoom(&self) -> f32 {
        self.min_zoom.get()
    }

    /// 최대 camera 배율.
    pub const fn max_zoom(&self) -> f32 {
        self.max_zoom.get()
    }
}

impl Default for InteractionConfig {
    fn default() -> InteractionConfig {
        InteractionConfig::new(
            InteractionConfig::DEFAULT_DRAG_THRESHOLD,
            InteractionConfig::DEFAULT_ZOOM_DRAG_PER_DOUBLING,
            InteractionConfig::DEFAULT_CLICK_ZOOM_FACTOR,
            InteractionConfig::DEFAULT_MIN_ZOOM,
            InteractionConfig::DEFAULT_MAX_ZOOM,
        )
        .expect("기본 상호작용 수치는 유효")
    }
}

/// 상호작용 입력 오류. 오류가 나면 상태는 바뀌지 않습니다. 뗌은 오류를 돌려주지 않고 취소로 끝냅니다
/// ([`InteractionController::pointer_up`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionError {
    /// 이미 진행 중인 상호작용이 있습니다(뗌 없이 다시 누름).
    AlreadyActive,
    /// camera를 되돌릴 수 없습니다(행렬식 0).
    CameraNotInvertible,
    /// 크기 조절·회전 handle을 눌렀지만 선택 상자가 없습니다.
    MissingSelectionBox,
    /// 잡은 handle이 바꾸는 선택 상자 축의 길이가 0이라(수평 선의 위·아래 변, 점 상자 등) 크기를 조절할
    /// 수 없습니다.
    DegenerateSelectionBox,
    /// 계산 결과가 유한하지 않습니다(극단 좌표).
    NonFinite,
}

impl std::fmt::Display for InteractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InteractionError::AlreadyActive => f.write_str("이미 진행 중인 상호작용이 있습니다"),
            InteractionError::CameraNotInvertible => f.write_str("camera를 되돌릴 수 없습니다"),
            InteractionError::MissingSelectionBox => f.write_str("선택 상자가 없습니다"),
            InteractionError::DegenerateSelectionBox => {
                f.write_str("선택 상자의 이 축은 길이가 0이라 크기를 조절할 수 없습니다")
            }
            InteractionError::NonFinite => f.write_str("계산 결과가 유한하지 않습니다"),
        }
    }
}

impl std::error::Error for InteractionError {}

// ---------------------------------------------------------------------------
// 출력
// ---------------------------------------------------------------------------

/// 크기 조절 handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    /// 모서리(두 축).
    Corner(Corner),
    /// 변(한 축).
    Edge(Edge),
}

/// 진행 중 transient preview. scene·문서에 쓰지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InteractionPreview {
    /// 누른 채 threshold 안: 아직 바뀐 것이 없습니다.
    Pending {
        /// 진행 중인 상호작용.
        kind: InteractionKind,
    },
    /// pan·zoom 중 camera(world → view).
    Camera {
        /// [`InteractionKind::Pan`] 또는 [`InteractionKind::Zoom`].
        kind: InteractionKind,
        /// 현재 camera.
        camera: Transform2D,
    },
    /// 영역 선택 사각형(world).
    Marquee {
        /// 누른 점과 현재 점의 bounds.
        rect: Aabb,
    },
    /// 대상([`InteractionController::targets`])에 적용할 world 변환(gesture 시작 상태 기준).
    Transform {
        /// [`InteractionKind::Drag`]·[`InteractionKind::Resize`]·[`InteractionKind::Rotate`].
        kind: InteractionKind,
        /// 시작 상태 → 현재 상태 world 변환.
        transform: Transform2D,
    },
}

/// 뗌 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionOutcome {
    /// 눌렀다 뗐지만 바뀐 것이 없습니다(Pan·Resize·Rotate 클릭). 기록할 것이 없습니다.
    Unchanged {
        /// 상호작용 종류.
        kind: InteractionKind,
    },
    /// pan 끝 camera.
    Pan {
        /// 끝 camera.
        camera: Transform2D,
    },
    /// zoom 끝 camera(클릭은 한 단계 확대).
    Zoom {
        /// 끝 camera.
        camera: Transform2D,
    },
    /// 클릭 선택: node 또는 빈 곳(`None`, 선택 해제).
    Click {
        /// 누른 node.
        target: Option<StableId>,
    },
    /// 영역 선택 끝 사각형(world).
    Marquee {
        /// 사각형.
        rect: Aabb,
    },
    /// 끌기 끝.
    Drag {
        /// 대상(선택 순서).
        targets: Vec<StableId>,
        /// world 이동량.
        delta: Vec2,
        /// 이동 변환.
        transform: Transform2D,
    },
    /// 크기 조절 끝.
    Resize {
        /// 대상(선택 순서).
        targets: Vec<StableId>,
        /// 잡은 handle.
        handle: ResizeHandle,
        /// 시작 상자 → 끝 상자 world 변환.
        transform: Transform2D,
        /// 끝 상자 world 꼭짓점.
        quad: [Vec2; 4],
    },
    /// 회전 끝.
    Rotate {
        /// 대상(선택 순서).
        targets: Vec<StableId>,
        /// 회전 중심(world).
        center: Vec2,
        /// 각도(radian, 화면에서 시계 방향 양수).
        angle: f32,
        /// 회전 변환.
        transform: Transform2D,
    },
}

/// 취소 결과. 기록할 것이 없습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cancellation {
    /// 취소한 상호작용.
    pub kind: InteractionKind,
    /// pan·zoom이면 되돌릴 시작 camera.
    pub restore_camera: Option<Transform2D>,
    /// 뗌 계산이 실패해 취소로 끝났으면 그 오류. 호출자가 취소했으면 `None`.
    pub failure: Option<InteractionError>,
}

/// 입력 하나의 처리 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionStep {
    /// 상호작용을 시작하거나 진행하지 않는 입력입니다.
    Ignored,
    /// 상호작용을 시작했습니다(누름).
    Started(InteractionKind),
    /// preview가 갱신됐습니다(이동).
    Updated(InteractionPreview),
    /// 끝났습니다(뗌).
    Completed(InteractionOutcome),
    /// 취소했습니다.
    Cancelled(Cancellation),
}

// ---------------------------------------------------------------------------
// 상태
// ---------------------------------------------------------------------------

/// 누른 점. 상태 기계 밖에서는 만들 수 없습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Press {
    view: Vec2,
    world: Vec2,
    camera: Transform2D,
    inverse: Transform2D,
}

impl Press {
    fn new(view: Vec2, camera: Transform2D) -> Result<Press, InteractionError> {
        let inverse = camera
            .inverse()
            .ok_or(InteractionError::CameraNotInvertible)?;
        let world = inverse
            .apply(view)
            .map_err(|_| InteractionError::NonFinite)?;
        Ok(Press {
            view,
            world,
            camera,
            inverse,
        })
    }

    /// 누른 view 점.
    pub const fn view(&self) -> Vec2 {
        self.view
    }

    /// 누른 world 점.
    pub const fn world(&self) -> Vec2 {
        self.world
    }

    /// 누름 시점 camera.
    pub const fn camera(&self) -> Transform2D {
        self.camera
    }

    fn world_of(&self, view: Vec2) -> Result<Vec2, InteractionError> {
        self.inverse
            .apply(view)
            .map_err(|_| InteractionError::NonFinite)
    }

    fn beyond(&self, view: Vec2, config: &InteractionConfig) -> bool {
        let distance = (view.x() - self.view.x()).hypot(view.y() - self.view.y());
        distance > config.drag_threshold()
    }
}

/// 크기 조절 기준이 되는 선택 상자 좌표계. 상태 기계 밖에서는 만들 수 없습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionFrame {
    quad: [Vec2; 4],
    axes: ResizeAxes,
    to_world: Transform2D,
    to_local: Transform2D,
}

impl SelectionFrame {
    /// 상자 꼭짓점에서 축 좌표계(top_left 원점, top_right·bottom_left 방향 단위 1)를 만듭니다. 길이가 0인
    /// 축은 다른 축에 수직인 world 단위 벡터로 채웁니다. 그 축의 배율은 1로 고정되므로 채운 길이는 결과에
    /// 영향이 없습니다. 두 축 모두 쓸 수 없으면 오류입니다.
    fn new(quad: [Vec2; 4]) -> Result<SelectionFrame, InteractionError> {
        let axes = ResizeAxes::of(&quad);
        let [tl, tr, _, bl] = quad;
        let u = (tr.x() - tl.x(), tr.y() - tl.y());
        let v = (bl.x() - tl.x(), bl.y() - tl.y());
        let ((ux, uy), (vx, vy)) = match (axes.horizontal(), axes.vertical()) {
            (true, true) => (u, v),
            (true, false) => {
                let length = u.0.hypot(u.1);
                (u, (-u.1 / length, u.0 / length))
            }
            (false, true) => {
                let length = v.0.hypot(v.1);
                ((v.1 / length, -v.0 / length), v)
            }
            (false, false) => return Err(InteractionError::DegenerateSelectionBox),
        };
        let to_world = Transform2D::new(ux, uy, vx, vy, tl.x(), tl.y())
            .map_err(|_| InteractionError::NonFinite)?;
        let to_local = to_world
            .inverse()
            .ok_or(InteractionError::DegenerateSelectionBox)?;
        Ok(SelectionFrame {
            quad,
            axes,
            to_world,
            to_local,
        })
    }

    /// 시작 상자 world 꼭짓점.
    pub const fn quad(&self) -> [Vec2; 4] {
        self.quad
    }

    /// 크기를 조절할 수 있는 축.
    pub const fn axes(&self) -> ResizeAxes {
        self.axes
    }
}

/// Pan 상태.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanState {
    /// 누른 채 threshold 안.
    Pressed {
        /// 누른 점.
        press: Press,
    },
    /// 화면 이동 중.
    Panning {
        /// 누른 점.
        press: Press,
        /// 현재 camera.
        camera: Transform2D,
    },
}

/// Zoom 상태.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomState {
    /// 누른 채 threshold 안(떼면 클릭 확대).
    Pressed {
        /// 누른 점.
        press: Press,
    },
    /// 끌어서 배율 조절 중.
    Scrubbing {
        /// 누른 점.
        press: Press,
        /// 현재 camera.
        camera: Transform2D,
    },
}

/// Select 상태.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectState {
    /// node 또는 빈 곳을 누른 채 threshold 안(떼면 클릭).
    Pressed {
        /// 누른 점.
        press: Press,
        /// 누른 node(빈 곳이면 `None`).
        target: Option<StableId>,
    },
    /// 빈 곳에서 끌어 영역 선택 중.
    Marquee {
        /// 누른 점.
        press: Press,
        /// 현재 사각형(world).
        rect: Aabb,
    },
}

/// Drag 상태.
#[derive(Debug, Clone, PartialEq)]
pub enum DragState {
    /// 선택된 node를 누른 채 threshold 안(떼면 그 node 클릭).
    Pressed {
        /// 누른 점.
        press: Press,
        /// 누른 node.
        clicked: StableId,
        /// 끌 대상(현재 선택).
        targets: Vec<StableId>,
    },
    /// 끌기 중.
    Dragging {
        /// 누른 점.
        press: Press,
        /// 끌 대상.
        targets: Vec<StableId>,
        /// 현재 world 이동량.
        delta: Vec2,
    },
}

/// Resize 상태.
#[derive(Debug, Clone, PartialEq)]
pub enum ResizeState {
    /// handle을 누른 채 threshold 안.
    Pressed {
        /// 누른 점.
        press: Press,
        /// 잡은 handle.
        handle: ResizeHandle,
        /// 시작 상자.
        frame: SelectionFrame,
        /// 대상(현재 선택).
        targets: Vec<StableId>,
    },
    /// 크기 조절 중.
    Resizing {
        /// 누른 점.
        press: Press,
        /// 잡은 handle.
        handle: ResizeHandle,
        /// 시작 상자.
        frame: SelectionFrame,
        /// 대상.
        targets: Vec<StableId>,
        /// 시작 상자 → 현재 상자 world 변환.
        transform: Transform2D,
    },
}

/// Rotate 상태.
#[derive(Debug, Clone, PartialEq)]
pub enum RotateState {
    /// 회전 영역을 누른 채 threshold 안.
    Pressed {
        /// 누른 점.
        press: Press,
        /// 상자 중심(world).
        center: Vec2,
        /// 대상(현재 선택).
        targets: Vec<StableId>,
    },
    /// 회전 중.
    Rotating {
        /// 누른 점.
        press: Press,
        /// 상자 중심(world).
        center: Vec2,
        /// 대상.
        targets: Vec<StableId>,
        /// 누적 각도(radian).
        angle: f32,
    },
}

/// 진행 중인 상호작용(닫힌 집합). 한 번에 하나만 진행합니다.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum InteractionState {
    /// 진행 중인 상호작용 없음.
    #[default]
    Idle,
    /// 화면 이동.
    Pan(PanState),
    /// 확대·축소.
    Zoom(ZoomState),
    /// 클릭·영역 선택.
    Select(SelectState),
    /// 끌기.
    Drag(DragState),
    /// 크기 조절.
    Resize(ResizeState),
    /// 회전.
    Rotate(RotateState),
}

impl InteractionState {
    /// 진행 중인 상호작용 종류. `Idle`이면 `None`.
    pub fn kind(&self) -> Option<InteractionKind> {
        match self {
            InteractionState::Idle => None,
            InteractionState::Pan(_) => Some(InteractionKind::Pan),
            InteractionState::Zoom(_) => Some(InteractionKind::Zoom),
            InteractionState::Select(_) => Some(InteractionKind::Select),
            InteractionState::Drag(_) => Some(InteractionKind::Drag),
            InteractionState::Resize(_) => Some(InteractionKind::Resize),
            InteractionState::Rotate(_) => Some(InteractionKind::Rotate),
        }
    }

    fn press(&self) -> Option<&Press> {
        match self {
            InteractionState::Idle => None,
            InteractionState::Pan(
                PanState::Pressed { press } | PanState::Panning { press, .. },
            )
            | InteractionState::Zoom(
                ZoomState::Pressed { press } | ZoomState::Scrubbing { press, .. },
            )
            | InteractionState::Select(
                SelectState::Pressed { press, .. } | SelectState::Marquee { press, .. },
            )
            | InteractionState::Drag(
                DragState::Pressed { press, .. } | DragState::Dragging { press, .. },
            )
            | InteractionState::Resize(
                ResizeState::Pressed { press, .. } | ResizeState::Resizing { press, .. },
            )
            | InteractionState::Rotate(
                RotateState::Pressed { press, .. } | RotateState::Rotating { press, .. },
            ) => Some(press),
        }
    }

    /// 현재 transient preview. `Idle`이면 `None`.
    pub fn preview(&self) -> Option<InteractionPreview> {
        let kind = self.kind()?;
        let pending = InteractionPreview::Pending { kind };
        Some(match self {
            InteractionState::Idle => return None,
            InteractionState::Pan(PanState::Panning { camera, .. })
            | InteractionState::Zoom(ZoomState::Scrubbing { camera, .. }) => {
                InteractionPreview::Camera {
                    kind,
                    camera: *camera,
                }
            }
            InteractionState::Select(SelectState::Marquee { rect, .. }) => {
                InteractionPreview::Marquee { rect: *rect }
            }
            InteractionState::Drag(DragState::Dragging { delta, .. }) => {
                InteractionPreview::Transform {
                    kind,
                    transform: translation(*delta).ok()?,
                }
            }
            InteractionState::Resize(ResizeState::Resizing { transform, .. }) => {
                InteractionPreview::Transform {
                    kind,
                    transform: *transform,
                }
            }
            InteractionState::Rotate(RotateState::Rotating { center, angle, .. }) => {
                InteractionPreview::Transform {
                    kind,
                    transform: rotation(*center, *angle).ok()?,
                }
            }
            InteractionState::Pan(PanState::Pressed { .. })
            | InteractionState::Zoom(ZoomState::Pressed { .. })
            | InteractionState::Select(SelectState::Pressed { .. })
            | InteractionState::Drag(DragState::Pressed { .. })
            | InteractionState::Resize(ResizeState::Pressed { .. })
            | InteractionState::Rotate(RotateState::Pressed { .. }) => pending,
        })
    }

    /// Drag·Resize·Rotate 대상(선택 순서). 그 밖이면 빈 slice.
    pub fn targets(&self) -> &[StableId] {
        match self {
            InteractionState::Drag(
                DragState::Pressed { targets, .. } | DragState::Dragging { targets, .. },
            )
            | InteractionState::Resize(
                ResizeState::Pressed { targets, .. } | ResizeState::Resizing { targets, .. },
            )
            | InteractionState::Rotate(
                RotateState::Pressed { targets, .. } | RotateState::Rotating { targets, .. },
            ) => targets,
            _ => &[],
        }
    }
}

// ---------------------------------------------------------------------------
// 상태 기계
// ---------------------------------------------------------------------------

/// 창 하나의 캔버스 상호작용 상태 기계.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InteractionController {
    config: InteractionConfig,
    state: InteractionState,
}

impl InteractionController {
    /// 수치를 지정해 만듭니다. 처음 상태는 `Idle`입니다.
    pub fn new(config: InteractionConfig) -> InteractionController {
        InteractionController {
            config,
            state: InteractionState::Idle,
        }
    }

    /// 수치.
    pub fn config(&self) -> &InteractionConfig {
        &self.config
    }

    /// 현재 상태.
    pub fn state(&self) -> &InteractionState {
        &self.state
    }

    /// 진행 중인 상호작용 종류.
    pub fn kind(&self) -> Option<InteractionKind> {
        self.state.kind()
    }

    /// 진행 중인 상호작용이 있으면 참입니다.
    pub fn is_active(&self) -> bool {
        self.state != InteractionState::Idle
    }

    /// 현재 transient preview.
    pub fn preview(&self) -> Option<InteractionPreview> {
        self.state.preview()
    }

    /// Drag·Resize·Rotate 대상.
    pub fn targets(&self) -> &[StableId] {
        self.state.targets()
    }

    /// pointer 누름(주 버튼). 시작 규칙(모듈 문서)으로 상호작용을 시작합니다.
    pub fn pointer_down(
        &mut self,
        view: Vec2,
        context: &PressContext<'_>,
    ) -> Result<InteractionStep, InteractionError> {
        if self.is_active() {
            return Err(InteractionError::AlreadyActive);
        }
        let press = Press::new(view, context.camera)?;
        let state = match (context.tool, context.target) {
            (InteractionTool::Hand, _) => InteractionState::Pan(PanState::Pressed { press }),
            (InteractionTool::Zoom, _) => InteractionState::Zoom(ZoomState::Pressed { press }),
            (InteractionTool::Move, PressTarget::Handle(kind)) => match kind {
                HandleKind::ResizeCorner(corner) => {
                    resize_start(press, ResizeHandle::Corner(corner), context)?
                }
                HandleKind::ResizeEdge(edge) => {
                    resize_start(press, ResizeHandle::Edge(edge), context)?
                }
                HandleKind::Rotate(_) => {
                    let quad = context
                        .selection_box
                        .ok_or(InteractionError::MissingSelectionBox)?;
                    InteractionState::Rotate(RotateState::Pressed {
                        press,
                        center: quad_center(quad)?,
                        targets: context.selection.to_vec(),
                    })
                }
                HandleKind::ModifierControl(_) => return Ok(InteractionStep::Ignored),
            },
            (InteractionTool::Move, PressTarget::Node(node))
                if context.selection.contains(&node) =>
            {
                InteractionState::Drag(DragState::Pressed {
                    press,
                    clicked: node,
                    targets: context.selection.to_vec(),
                })
            }
            (InteractionTool::Move, PressTarget::Node(node)) => {
                InteractionState::Select(SelectState::Pressed {
                    press,
                    target: Some(node),
                })
            }
            (InteractionTool::Move, PressTarget::Empty) => {
                InteractionState::Select(SelectState::Pressed {
                    press,
                    target: None,
                })
            }
        };
        let step = state
            .kind()
            .map_or(InteractionStep::Ignored, InteractionStep::Started);
        self.state = state;
        Ok(step)
    }

    /// pointer 이동. 진행 중이 아니면 무시합니다.
    pub fn pointer_move(&mut self, view: Vec2) -> Result<InteractionStep, InteractionError> {
        if !self.is_active() {
            return Ok(InteractionStep::Ignored);
        }
        let next = advance(&self.state, view, &self.config)?;
        let preview = next.preview().ok_or(InteractionError::NonFinite)?;
        self.state = next;
        Ok(InteractionStep::Updated(preview))
    }

    /// pointer 뗌. 뗀 위치를 마지막 이동으로 반영하고 결과를 냅니다. 진행 중이 아니면 무시합니다.
    ///
    /// 운영체제 pointer는 이미 올라갔으므로 뗌은 항상 gesture를 끝냅니다(그 뒤 `Idle`). 뗀 위치나 결과를
    /// 계산할 수 없으면(유한하지 않은 값 등) 결과 없이 [`InteractionStep::Cancelled`]로 끝내고 오류를
    /// [`Cancellation::failure`]에 담습니다. 호출자는 취소와 똑같이 preview를 버리고, Pan·Zoom이면
    /// [`Cancellation::restore_camera`]로 camera를 되돌립니다. 호출자가 따로 [`Self::cancel`]을 부를 필요가
    /// 없습니다.
    pub fn pointer_up(&mut self, view: Vec2) -> InteractionStep {
        if !self.is_active() {
            return InteractionStep::Ignored;
        }
        let outcome =
            advance(&self.state, view, &self.config).and_then(|last| complete(last, &self.config));
        match outcome {
            Ok(Some(outcome)) => {
                self.state = InteractionState::Idle;
                InteractionStep::Completed(outcome)
            }
            // 진행 중이면 advance가 Idle을 내지 않으므로 오지 않는 경우입니다. 그래도 gesture는 끝냅니다.
            Ok(None) => self.finish_cancelled(None),
            Err(error) => self.finish_cancelled(Some(error)),
        }
    }

    /// 진행 중인 상호작용을 결과 없이 취소합니다. 진행 중이 아니면 무시합니다.
    pub fn cancel(&mut self) -> InteractionStep {
        self.finish_cancelled(None)
    }

    /// 상태를 `Idle`로 비우고 취소 결과를 냅니다. 진행 중이 아니었으면 `Ignored`.
    fn finish_cancelled(&mut self, failure: Option<InteractionError>) -> InteractionStep {
        let state = std::mem::take(&mut self.state);
        let (Some(kind), Some(press)) = (state.kind(), state.press()) else {
            return InteractionStep::Ignored;
        };
        let restore_camera =
            matches!(kind, InteractionKind::Pan | InteractionKind::Zoom).then_some(press.camera);
        InteractionStep::Cancelled(Cancellation {
            kind,
            restore_camera,
            failure,
        })
    }
}

fn resize_start(
    press: Press,
    handle: ResizeHandle,
    context: &PressContext<'_>,
) -> Result<InteractionState, InteractionError> {
    let quad = context
        .selection_box
        .ok_or(InteractionError::MissingSelectionBox)?;
    let frame = SelectionFrame::new(quad)?;
    let (move_s, move_t) = moving_axes(handle, frame.axes);
    if !(move_s || move_t) {
        return Err(InteractionError::DegenerateSelectionBox);
    }
    Ok(InteractionState::Resize(ResizeState::Pressed {
        press,
        handle,
        frame,
        targets: context.selection.to_vec(),
    }))
}

/// 현재 상태에서 view 점 `view`로 이동한 다음 상태. 실패하면 상태를 바꾸지 않도록 새 값만 만듭니다.
fn advance(
    state: &InteractionState,
    view: Vec2,
    config: &InteractionConfig,
) -> Result<InteractionState, InteractionError> {
    Ok(match state {
        InteractionState::Idle => InteractionState::Idle,
        InteractionState::Pan(pan) => InteractionState::Pan(match *pan {
            PanState::Pressed { press } if !press.beyond(view, config) => *pan,
            PanState::Pressed { press } | PanState::Panning { press, .. } => PanState::Panning {
                press,
                camera: pan_camera(&press, view)?,
            },
        }),
        InteractionState::Zoom(zoom) => InteractionState::Zoom(match *zoom {
            ZoomState::Pressed { press } if !press.beyond(view, config) => *zoom,
            ZoomState::Pressed { press } | ZoomState::Scrubbing { press, .. } => {
                let factor = (view.x() - press.view.x()) / config.zoom_drag_per_doubling();
                ZoomState::Scrubbing {
                    press,
                    camera: zoom_camera(&press, factor.exp2(), config)?,
                }
            }
        }),
        InteractionState::Select(select) => match *select {
            SelectState::Pressed { press, .. } if !press.beyond(view, config) => {
                InteractionState::Select(*select)
            }
            SelectState::Pressed {
                press,
                target: Some(node),
            } => InteractionState::Drag(DragState::Dragging {
                press,
                targets: vec![node],
                delta: drag_delta(&press, view)?,
            }),
            SelectState::Pressed {
                press,
                target: None,
            }
            | SelectState::Marquee { press, .. } => {
                InteractionState::Select(SelectState::Marquee {
                    press,
                    rect: marquee(&press, view)?,
                })
            }
        },
        InteractionState::Drag(drag) => InteractionState::Drag(match drag {
            DragState::Pressed { press, .. } if !press.beyond(view, config) => drag.clone(),
            DragState::Pressed { press, targets, .. }
            | DragState::Dragging { press, targets, .. } => DragState::Dragging {
                press: *press,
                targets: targets.clone(),
                delta: drag_delta(press, view)?,
            },
        }),
        InteractionState::Resize(resize) => InteractionState::Resize(match resize {
            ResizeState::Pressed { press, .. } if !press.beyond(view, config) => resize.clone(),
            ResizeState::Pressed {
                press,
                handle,
                frame,
                targets,
            }
            | ResizeState::Resizing {
                press,
                handle,
                frame,
                targets,
                ..
            } => ResizeState::Resizing {
                press: *press,
                handle: *handle,
                frame: *frame,
                targets: targets.clone(),
                transform: resize_transform(press, *handle, frame, view)?,
            },
        }),
        InteractionState::Rotate(rotate) => InteractionState::Rotate(match rotate {
            RotateState::Pressed { press, .. } if !press.beyond(view, config) => rotate.clone(),
            RotateState::Pressed {
                press,
                center,
                targets,
            } => RotateState::Rotating {
                press: *press,
                center: *center,
                targets: targets.clone(),
                angle: rotate_angle(press, *center, view, 0.0)?,
            },
            RotateState::Rotating {
                press,
                center,
                targets,
                angle,
            } => RotateState::Rotating {
                press: *press,
                center: *center,
                targets: targets.clone(),
                angle: rotate_angle(press, *center, view, *angle)?,
            },
        }),
    })
}

/// 마지막 상태의 결과. `Idle`이면 결과가 없습니다.
fn complete(
    state: InteractionState,
    config: &InteractionConfig,
) -> Result<Option<InteractionOutcome>, InteractionError> {
    Ok(Some(match state {
        InteractionState::Idle => return Ok(None),
        InteractionState::Pan(PanState::Pressed { .. }) => InteractionOutcome::Unchanged {
            kind: InteractionKind::Pan,
        },
        InteractionState::Pan(PanState::Panning { camera, .. }) => {
            InteractionOutcome::Pan { camera }
        }
        InteractionState::Zoom(ZoomState::Pressed { press }) => InteractionOutcome::Zoom {
            camera: zoom_camera(&press, config.click_zoom_factor(), config)?,
        },
        InteractionState::Zoom(ZoomState::Scrubbing { camera, .. }) => {
            InteractionOutcome::Zoom { camera }
        }
        InteractionState::Select(SelectState::Pressed { target, .. }) => {
            InteractionOutcome::Click { target }
        }
        InteractionState::Select(SelectState::Marquee { rect, .. }) => {
            InteractionOutcome::Marquee { rect }
        }
        InteractionState::Drag(DragState::Pressed { clicked, .. }) => InteractionOutcome::Click {
            target: Some(clicked),
        },
        InteractionState::Drag(DragState::Dragging { targets, delta, .. }) => {
            InteractionOutcome::Drag {
                targets,
                delta,
                transform: translation(delta)?,
            }
        }
        InteractionState::Resize(ResizeState::Pressed { .. }) => InteractionOutcome::Unchanged {
            kind: InteractionKind::Resize,
        },
        InteractionState::Resize(ResizeState::Resizing {
            handle,
            frame,
            targets,
            transform,
            ..
        }) => InteractionOutcome::Resize {
            targets,
            handle,
            transform,
            quad: transform_quad(&transform, frame.quad)?,
        },
        InteractionState::Rotate(RotateState::Pressed { .. }) => InteractionOutcome::Unchanged {
            kind: InteractionKind::Rotate,
        },
        InteractionState::Rotate(RotateState::Rotating {
            center,
            targets,
            angle,
            ..
        }) => InteractionOutcome::Rotate {
            targets,
            center,
            angle,
            transform: rotation(center, angle)?,
        },
    }))
}

// ---------------------------------------------------------------------------
// 계산
// ---------------------------------------------------------------------------

fn vec2(x: f32, y: f32) -> Result<Vec2, InteractionError> {
    Vec2::new(x, y).map_err(|_| InteractionError::NonFinite)
}

fn transform(
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
) -> Result<Transform2D, InteractionError> {
    Transform2D::new(a, b, c, d, tx, ty).map_err(|_| InteractionError::NonFinite)
}

fn then(first: &Transform2D, next: &Transform2D) -> Result<Transform2D, InteractionError> {
    first.then(next).map_err(|_| InteractionError::NonFinite)
}

fn translation(delta: Vec2) -> Result<Transform2D, InteractionError> {
    transform(1.0, 0.0, 0.0, 1.0, delta.x(), delta.y())
}

/// `center` 기준 `angle` 회전(view y 아래 방향에서 양수 = 시계 방향).
fn rotation(center: Vec2, angle: f32) -> Result<Transform2D, InteractionError> {
    let (sin, cos) = angle.sin_cos();
    let (cx, cy) = (center.x(), center.y());
    transform(
        cos,
        sin,
        -sin,
        cos,
        cx - cos * cx + sin * cy,
        cy - sin * cx - cos * cy,
    )
}

fn pan_camera(press: &Press, view: Vec2) -> Result<Transform2D, InteractionError> {
    let delta = vec2(view.x() - press.view.x(), view.y() - press.view.y())?;
    then(&press.camera, &translation(delta)?)
}

/// 시작 camera에 누른 view 점 기준 `factor`배를 더합니다. 결과 배율은 설정 범위로 고정합니다.
fn zoom_camera(
    press: &Press,
    factor: f32,
    config: &InteractionConfig,
) -> Result<Transform2D, InteractionError> {
    let start = press.camera.determinant().abs().sqrt();
    if !(start.is_finite() && start > 0.0) {
        return Err(InteractionError::CameraNotInvertible);
    }
    let target = (start * factor).clamp(config.min_zoom(), config.max_zoom());
    let f = target / start;
    let (px, py) = (press.view.x(), press.view.y());
    then(
        &press.camera,
        &transform(f, 0.0, 0.0, f, px - f * px, py - f * py)?,
    )
}

fn drag_delta(press: &Press, view: Vec2) -> Result<Vec2, InteractionError> {
    let world = press.world_of(view)?;
    vec2(world.x() - press.world.x(), world.y() - press.world.y())
}

fn marquee(press: &Press, view: Vec2) -> Result<Aabb, InteractionError> {
    let world = press.world_of(view)?;
    Aabb::from_points([press.world, world]).ok_or(InteractionError::NonFinite)
}

fn quad_center(quad: [Vec2; 4]) -> Result<Vec2, InteractionError> {
    let x = quad.iter().map(Vec2::x).sum::<f32>() / 4.0;
    let y = quad.iter().map(Vec2::y).sum::<f32>() / 4.0;
    vec2(x, y)
}

fn transform_quad(transform: &Transform2D, quad: [Vec2; 4]) -> Result<[Vec2; 4], InteractionError> {
    let mut out = [Vec2::ZERO; 4];
    for (slot, point) in out.iter_mut().zip(quad) {
        *slot = transform
            .apply(point)
            .map_err(|_| InteractionError::NonFinite)?;
    }
    Ok(out)
}

/// handle의 상자 축 좌표 `(s, t)`와 고정점, handle이 바꾸는 축.
fn handle_axes(handle: ResizeHandle) -> ((f32, f32), (f32, f32), (bool, bool)) {
    match handle {
        ResizeHandle::Corner(Corner::TopLeft) => ((0.0, 0.0), (1.0, 1.0), (true, true)),
        ResizeHandle::Corner(Corner::TopRight) => ((1.0, 0.0), (0.0, 1.0), (true, true)),
        ResizeHandle::Corner(Corner::BottomRight) => ((1.0, 1.0), (0.0, 0.0), (true, true)),
        ResizeHandle::Corner(Corner::BottomLeft) => ((0.0, 1.0), (1.0, 0.0), (true, true)),
        ResizeHandle::Edge(Edge::Top) => ((0.0, 0.0), (0.0, 1.0), (false, true)),
        ResizeHandle::Edge(Edge::Right) => ((1.0, 0.0), (0.0, 0.0), (true, false)),
        ResizeHandle::Edge(Edge::Bottom) => ((0.0, 1.0), (0.0, 0.0), (false, true)),
        ResizeHandle::Edge(Edge::Left) => ((0.0, 0.0), (1.0, 0.0), (true, false)),
    }
}

/// handle이 바꾸는 축 중 길이가 있어 실제로 움직이는 축 `(s, t)`.
fn moving_axes(handle: ResizeHandle, axes: ResizeAxes) -> (bool, bool) {
    let (_, _, (move_s, move_t)) = handle_axes(handle);
    (move_s && axes.horizontal(), move_t && axes.vertical())
}

/// 시작 상자 → 현재 상자 world 변환: 상자 축 좌표에서 고정점 기준으로 움직이는 축만 늘립니다.
fn resize_transform(
    press: &Press,
    handle: ResizeHandle,
    frame: &SelectionFrame,
    view: Vec2,
) -> Result<Transform2D, InteractionError> {
    let world = press.world_of(view)?;
    let local = |point: Vec2| {
        frame
            .to_local
            .apply(point)
            .map_err(|_| InteractionError::NonFinite)
    };
    let (from, to) = (local(press.world)?, local(world)?);
    let (ds, dt) = (to.x() - from.x(), to.y() - from.y());
    let ((hs, ht), (anchor_s, anchor_t), _) = handle_axes(handle);
    let (move_s, move_t) = moving_axes(handle, frame.axes);
    // 고정점과 handle은 축마다 거리 1이므로 배율은 (handle + 이동 − 고정점) / (handle − 고정점)입니다.
    let ks = if move_s {
        (hs + ds - anchor_s) / (hs - anchor_s)
    } else {
        1.0
    };
    let kt = if move_t {
        (ht + dt - anchor_t) / (ht - anchor_t)
    } else {
        1.0
    };
    let scale = transform(
        ks,
        0.0,
        0.0,
        kt,
        anchor_s * (1.0 - ks),
        anchor_t * (1.0 - kt),
    )?;
    then(&then(&frame.to_local, &scale)?, &frame.to_world)
}

/// 이전 누적 각도 `previous`에 이어 붙인 누른 점 → `view`의 회전 각도.
fn rotate_angle(
    press: &Press,
    center: Vec2,
    view: Vec2,
    previous: f32,
) -> Result<f32, InteractionError> {
    use std::f32::consts::{PI, TAU};
    let world = press.world_of(view)?;
    let start = (press.world.y() - center.y()).atan2(press.world.x() - center.x());
    let now = (world.y() - center.y()).atan2(world.x() - center.x());
    // 이전 각도와의 차이를 (−π, π]로 접어 이어 붙입니다.
    let mut step = (now - start - previous) % TAU;
    if step > PI {
        step -= TAU;
    } else if step <= -PI {
        step += TAU;
    }
    let angle = previous + step;
    if angle.is_finite() {
        Ok(angle)
    } else {
        Err(InteractionError::NonFinite)
    }
}

#[cfg(test)]
mod tests;
