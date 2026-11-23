//! 캔버스 변형·정렬 엔진 (XII.10.9~XII.10.14, XII.10.21, XII.10.23, XII.10.12, XII.10.44, ADR-0105).
//!
//! 셸 없이 Rust 호출만으로 같은 결과를 내는 캔버스 편집 의미입니다. 모양은 다음과 같습니다.
//!
//! - **gesture 상태와 순수 update**: 제스처 시작 때 게시된 캔버스 scene에서 대상의 기하([`NodeGeometry`])를 한 번 읽고,
//!   pointer update마다 순수 함수로 미리보기 값을 계산합니다. update는 문서·history를 바꾸지 않습니다. 그래서 drag 500
//!   update 뒤 pointer up에서 확정하면 history 항목은 하나이고, Escape로 버리면(확정하지 않으면) 0입니다.
//! - **확정**: pointer up(또는 메뉴 명령 하나)에서 command를 만들어 [`commit`]으로 CommandTransaction 하나를 창 history에
//!   기록합니다(Undo 한 번).
//! - **쓰기 한 곳**: 위치·선형부·크기 쓰기는 모두 `stellar_commands::arrange::place`가 저장 위치 규칙에 맞게 만듭니다.
//!   정렬·분배·순서·묶음·비례 확대·뒤집기의 문서 쪽 계획은 `stellar_commands::{arrange, scale, flip}`에 있어 이후
//!   CLI·MCP도 같은 계획을 씁니다. 이 module은 scene에서 화면 상자(Text layout 결과 포함)를 읽어 그 계획에 넘깁니다.
//!
//! 좌표: world는 캔버스 world, "부모 공간"은 node 부모(가장 가까운 Frame, Fragment는 변환이 없음)의 local 좌표,
//! "local"은 node 자신의 좌표입니다. 크기 핸들은 node local 좌표입니다(XII.10.8).

use stellar_commands::arrange::group::GroupError;
use stellar_commands::arrange::order::OrderError;
use stellar_commands::arrange::place::PlaceError;
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_document::model::DocumentModel;
use stellar_scene::bounds::Aabb;

use crate::canvas::CanvasScene;
use crate::window_session::ProjectWindowSession;

pub mod radius;
pub mod resize;
pub mod rotate;

#[cfg(test)]
pub(crate) mod fixtures;

/// 변형 요청을 받을 수 없는 이유.
#[derive(Clone, Debug, PartialEq)]
pub enum TransformError {
    /// 활성 View·선택·scene이 요청과 맞지 않습니다(대상이 scene에 없음, 다른 View의 node 등).
    Stale,
    /// 이 조합은 비활성입니다. 코드는 도움말에 씁니다(`flow-child`, `mixed-parents`, `justify-content` 등).
    Disabled(&'static str),
    /// 배치 쓰기를 만들 수 없습니다(잠금·표현 불가).
    Place(PlaceError),
    /// 묶음 명령 오류.
    Group(GroupError),
    /// 순서 명령 오류.
    Order(OrderError),
    /// 새 식별자를 만들지 못했습니다.
    Ids,
    /// 창 history 기록 실패.
    Edit(String),
}

impl From<PlaceError> for TransformError {
    fn from(error: PlaceError) -> TransformError {
        TransformError::Place(error)
    }
}

impl From<GroupError> for TransformError {
    fn from(error: GroupError) -> TransformError {
        TransformError::Group(error)
    }
}

impl From<OrderError> for TransformError {
    fn from(error: OrderError) -> TransformError {
        TransformError::Order(error)
    }
}

/// node 하나의 화면 기하(게시된 캔버스 scene 기준).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeGeometry {
    /// 문서 node.
    pub id: StableId,
    /// 부모 local → world(부모가 없으면 항등).
    pub parent_world: Transform2D,
    /// node local → 부모 공간.
    pub local: Transform2D,
    /// local 상자(x, y, width, height).
    pub rect: [f32; 4],
    /// world 경계 상자(회전 반영, 같은 node의 인스턴스 투영 행은 합침).
    pub world_bounds: [f32; 4],
}

impl NodeGeometry {
    /// 부모 공간 경계 상자(회전 반영).
    pub fn parent_bounds(&self) -> Option<[f32; 4]> {
        bounds_of(&self.local, self.rect)
    }

    /// local 상자 중심의 world 좌표.
    pub fn world_center(&self) -> Option<[f32; 2]> {
        let world = self.local.then(&self.parent_world).ok()?;
        let [x, y, w, h] = self.rect;
        let p = world
            .apply(Vec2::new(x + w / 2.0, y + h / 2.0).ok()?)
            .ok()?;
        Some([p.x(), p.y()])
    }
}

/// 변환한 상자의 축 정렬 경계.
pub fn bounds_of(transform: &Transform2D, rect: [f32; 4]) -> Option<[f32; 4]> {
    let [x, y, w, h] = rect;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
        let p = transform.apply(Vec2::new(px, py).ok()?).ok()?;
        min = [min[0].min(p.x()), min[1].min(p.y())];
        max = [max[0].max(p.x()), max[1].max(p.y())];
    }
    Some([min[0], min[1], max[0] - min[0], max[1] - min[1]])
}

/// 상자들의 경계.
pub fn union(boxes: impl IntoIterator<Item = [f32; 4]>) -> Option<[f32; 4]> {
    let mut out: Option<[f32; 4]> = None;
    for b in boxes {
        out = Some(match out {
            None => b,
            Some(a) => {
                let x0 = a[0].min(b[0]);
                let y0 = a[1].min(b[1]);
                let x1 = (a[0] + a[2]).max(b[0] + b[2]);
                let y1 = (a[1] + a[3]).max(b[1] + b[3]);
                [x0, y0, x1 - x0, y1 - y0]
            }
        });
    }
    out
}

/// scene에서 `node`의 기하를 읽습니다. 행이 없으면 `None`입니다.
pub fn node_geometry(scene: &CanvasScene, node: StableId) -> Option<NodeGeometry> {
    let rows = &scene.frames;
    let first = rows.iter().find(|row| row.id == node)?;
    let parent_world = first
        .parent
        .and_then(|p| rows.get(p as usize))
        .map_or(Transform2D::identity(), |p| p.world);
    let local = first.world.then(&parent_world.inverse()?).ok()?;
    let r = first.geometry.rect;
    let rect = [
        r.origin().x(),
        r.origin().y(),
        r.size().width(),
        r.size().height(),
    ];
    let mut world: Option<Aabb> = None;
    for row in rows.iter().filter(|row| row.id == node) {
        let Some(b) = Aabb::from_rect(row.geometry.rect).transformed(&row.world) else {
            continue;
        };
        world = Some(world.map_or(b, |w| w.union(&b)));
    }
    let world = world?;
    Some(NodeGeometry {
        id: node,
        parent_world,
        local,
        rect,
        world_bounds: [world.min_x(), world.min_y(), world.width(), world.height()],
    })
}

/// 여러 node의 기하. 하나라도 scene에 없으면 [`TransformError::Stale`]입니다.
pub fn geometries(
    scene: &CanvasScene,
    nodes: &[StableId],
) -> Result<Vec<NodeGeometry>, TransformError> {
    nodes
        .iter()
        .map(|&n| node_geometry(scene, n).ok_or(TransformError::Stale))
        .collect()
}

/// 활성 View의 문서 모델과 root, 그리고 지금 선택에서 다른 선택의 자손을 뺀 node(선택 순서).
pub(crate) fn selection_roots(
    window: &ProjectWindowSession,
) -> Result<(&DocumentModel, StableId, Vec<StableId>), TransformError> {
    let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
    let model = window.document().ok_or(TransformError::Stale)?.model();
    let root = crate::app::canvas_gesture::view_root(model, view).ok_or(TransformError::Stale)?;
    let selected = window.view().selection().ids();
    let roots = crate::app::canvas_gesture::selection_roots(model, root, selected)
        .ok_or(TransformError::Stale)?;
    Ok((model, root, roots))
}

/// 제품 경로의 새 영속 ID 원천(UUIDv7, OS 난수). 실패하면 NIL을 돌려주고 [`FreshIds::finish`]에서 오류로 바꿉니다.
#[derive(Default)]
pub struct FreshIds {
    failed: bool,
}

impl FreshIds {
    /// 새 ID 하나.
    pub fn fresh(&mut self) -> StableId {
        match stellar_project::new_ids::new_ids::<1>() {
            Ok([id]) => id,
            Err(_) => {
                self.failed = true;
                StableId::NIL
            }
        }
    }

    /// 실패가 있었으면 [`TransformError::Ids`].
    pub fn finish(self) -> Result<(), TransformError> {
        if self.failed {
            Err(TransformError::Ids)
        } else {
            Ok(())
        }
    }
}

/// command들을 CommandTransaction 하나로 묶습니다. command ID는 새 UUIDv7입니다.
pub fn transaction(
    window: &ProjectWindowSession,
    commands: Vec<Command>,
) -> Result<CommandTransaction, TransformError> {
    let mut ids = FreshIds::default();
    let id = CommandId::from_bytes(ids.fresh().to_bytes());
    ids.finish()?;
    let revision = window.revision();
    Ok(commands.into_iter().fold(
        CommandTransaction::new(id, CommandOrigin::User),
        |tx, command| tx.with_command(revision, command),
    ))
}

/// command들을 transaction 하나로 창 history에 기록합니다(Undo 한 번). command가 없으면 기록하지 않고 `false`입니다.
pub fn commit(
    window: &mut ProjectWindowSession,
    commands: Vec<Command>,
    label: u8,
) -> Result<bool, TransformError> {
    if commands.is_empty() {
        return Ok(false);
    }
    let tx = transaction(window, commands)?;
    window
        .apply(&tx, label)
        .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
    Ok(true)
}
