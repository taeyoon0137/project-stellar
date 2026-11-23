//! 기존 Path의 벡터 편집 모드 (XII.10.29, ADR-0117 결정 7).
//!
//! - 들어가기: Path node에서 Return·더블클릭(`Navigation::EditContent`). Shape는 들어가지 않습니다(`NotAPath`). Shape를
//!   Path로 바꾸는 일은 명시적 Convert to Path(VII.2.7)뿐이고, 이 모드는 Path로 변환하지 않습니다.
//! - 나오기: Escape(모드 값을 버림). 모드는 창별 상태이고 저장·history 대상이 아닙니다.
//! - point·handle hit: node local → world 변환으로 옮긴 화면 거리(반지름 `RADIUS_HANDLE_PT`).
//! - point·handle drag: update는 순수 계산이고 pointer up에서 `EditPathPoint` 하나를 history 한 항목으로 기록합니다.
//!   취소(확정하지 않음)는 0건입니다. 분기형 vector network는 문서 Path 모델에 없어 지원하지 않습니다.

use stellar_commands::arrange::place::local_transform;
use stellar_commands::command::Command;
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::path_edit::EditPathPoint;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_document::model::DocumentModel;
use stellar_document::path::{IdentifiedPath, PathCommand, PathEdit, PathHandleId, PathPointId};
use stellar_document::records::NodePayload;

use crate::app::canvas_gesture::parent_of;
use crate::canvas::select_navigate::Navigation;
use crate::canvas::tool_units::RADIUS_HANDLE_PT;
use crate::canvas::transform::{commit, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 모드에 들어가지 못한 이유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnterError {
    /// 문서에 없는 node.
    Missing,
    /// Path가 아닙니다(Shape는 Convert to Path 뒤에만).
    NotAPath,
    /// 자신이나 조상이 잠겼습니다.
    Locked,
}

/// 편집 대상 하나.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathTarget {
    /// anchor point.
    Point(PathPointId),
    /// control handle.
    Handle(PathHandleId),
}

/// 편집 가능한 point·handle의 local 위치.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathKnob {
    /// 대상.
    pub target: PathTarget,
    /// node local 좌표.
    pub at: Vec2,
}

/// path의 point·handle 목록(command 순서, handle은 point보다 앞).
pub fn knobs(path: &IdentifiedPath) -> Vec<PathKnob> {
    let mut out = Vec::new();
    for (command, ids) in path.data().commands().iter().zip(path.ids()) {
        match (command, ids.point, ids.handles) {
            (PathCommand::MoveTo(to) | PathCommand::LineTo(to), Some(point), _) => {
                out.push(PathKnob {
                    target: PathTarget::Point(point),
                    at: *to,
                });
            }
            (PathCommand::CubicTo { c1, c2, to }, Some(point), Some((h1, h2))) => {
                out.push(PathKnob {
                    target: PathTarget::Handle(h1),
                    at: *c1,
                });
                out.push(PathKnob {
                    target: PathTarget::Handle(h2),
                    at: *c2,
                });
                out.push(PathKnob {
                    target: PathTarget::Point(point),
                    at: *to,
                });
            }
            _ => {}
        }
    }
    out
}

/// 창별 벡터 편집 모드.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorEditMode {
    node: StableId,
}

fn path_of(model: &DocumentModel, node: StableId) -> Result<&IdentifiedPath, EnterError> {
    match model.payload(node) {
        Some(NodePayload::Path(path)) => Ok(path),
        Some(_) => Err(EnterError::NotAPath),
        None => Err(EnterError::Missing),
    }
}

/// node local → world: 저장 위치 변환(`place::local_transform`)과 가장 가까운 scene 행 조상의 world.
pub fn path_world(
    model: &DocumentModel,
    scene: &CanvasScene,
    node: StableId,
) -> Option<Transform2D> {
    let local = local_transform(model, node)?;
    let mut ancestor = parent_of(model, node);
    while let Some(id) = ancestor {
        if let Some(row) = scene.frames.iter().find(|row| row.id == id) {
            return local.then(&row.world).ok();
        }
        ancestor = parent_of(model, id);
    }
    Some(local)
}

impl VectorEditMode {
    /// Path node에서 들어갑니다.
    pub fn enter(model: &DocumentModel, node: StableId) -> Result<VectorEditMode, EnterError> {
        path_of(model, node)?;
        edit_permission(model, node, EditIntent::Edit).map_err(|_| EnterError::Locked)?;
        Ok(VectorEditMode { node })
    }

    /// 선택 엔진의 더블클릭·Return 결과에서 들어갑니다. 내용 편집 요청이 아니면 `None`, Path가 아니면(Text는
    /// XII.10.33) 오류입니다.
    pub fn from_navigation(
        model: &DocumentModel,
        navigation: &Navigation,
    ) -> Option<Result<VectorEditMode, EnterError>> {
        match navigation {
            Navigation::EditContent(node) => Some(VectorEditMode::enter(model, *node)),
            _ => None,
        }
    }

    /// 편집 중인 Path.
    pub fn node(&self) -> StableId {
        self.node
    }

    /// world 점 아래의 point·handle. 화면 거리 `RADIUS_HANDLE_PT` 안에서 가장 가까운 것(같으면 point 우선)입니다.
    pub fn hit(
        &self,
        model: &DocumentModel,
        world: Transform2D,
        zoom: f64,
        pointer: [f32; 2],
    ) -> Option<PathTarget> {
        let path = path_of(model, self.node).ok()?;
        let reach = f64::from(RADIUS_HANDLE_PT) / zoom;
        let mut best: Option<(f64, bool, PathTarget)> = None;
        for knob in knobs(path) {
            let p = world.apply(knob.at).ok()?;
            let d = f64::from(p.x() - pointer[0]).hypot(f64::from(p.y() - pointer[1]));
            let point = matches!(knob.target, PathTarget::Point(_));
            let better = best.is_none_or(|(bd, bp, _)| d < bd || (d == bd && point && !bp));
            if d <= reach && better {
                best = Some((d, point, knob.target));
            }
        }
        best.map(|b| b.2)
    }

    /// point·handle drag를 시작합니다. `world`는 [`path_world`], `pointer`는 누른 world 점입니다.
    pub fn begin_drag(
        &self,
        model: &DocumentModel,
        world: Transform2D,
        target: PathTarget,
        pointer: [f32; 2],
    ) -> Option<PointDrag> {
        let path = path_of(model, self.node).ok()?;
        let at = knobs(path).into_iter().find(|k| k.target == target)?.at;
        let to_local = world.inverse()?;
        let grab = to_local
            .apply(Vec2::new(pointer[0], pointer[1]).ok()?)
            .ok()?;
        Some(PointDrag {
            node: self.node,
            target,
            start: at,
            grab,
            to_local,
            current: at,
        })
    }
}

/// 진행 중 point·handle drag.
#[derive(Clone, Debug, PartialEq)]
pub struct PointDrag {
    node: StableId,
    target: PathTarget,
    start: Vec2,
    grab: Vec2,
    to_local: Transform2D,
    current: Vec2,
}

impl PointDrag {
    /// pointer(world) update. 문서·history를 바꾸지 않고 preview local 위치를 돌려줍니다.
    pub fn update(&mut self, pointer: [f32; 2]) -> Vec2 {
        if let Some(local) = Vec2::new(pointer[0], pointer[1])
            .ok()
            .and_then(|p| self.to_local.apply(p).ok())
        {
            if let Ok(next) = Vec2::new(
                self.start.x() + local.x() - self.grab.x(),
                self.start.y() + local.y() - self.grab.y(),
            ) {
                self.current = next;
            }
        }
        self.current
    }

    /// 확정할 편집. 움직이지 않았으면 `None`입니다.
    pub fn edit(&self) -> Option<PathEdit> {
        if self.current == self.start {
            return None;
        }
        Some(match self.target {
            PathTarget::Point(point) => PathEdit::MovePoint {
                point,
                to: self.current,
            },
            PathTarget::Handle(handle) => PathEdit::MoveHandle {
                handle,
                to: self.current,
            },
        })
    }

    /// pointer up: `EditPathPoint` 하나를 history 한 항목으로 기록합니다. 기록하면 참입니다.
    pub fn commit(&self, window: &mut ProjectWindowSession) -> Result<bool, TransformError> {
        let Some(edit) = self.edit() else {
            return Ok(false);
        };
        commit(
            window,
            vec![Command::EditPathPoint(EditPathPoint {
                node: self.node,
                edit,
            })],
            history_label::MOVE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::save_bytes;
    use crate::canvas::transform::fixtures::{
        at, canvas_document, history_len, model, open, scene,
    };
    use crate::window_session::test_fixtures::sid;
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableIdGenerator;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
    use stellar_document::path::{PathData, WindingRule};

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    /// 문서 10 아래 (50, 60)에 Path 40: (0,0) → (40,0) → 곡선 (40,40) → close.
    fn path_document() -> DocumentModel {
        let mut model = canvas_document();
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(v(0., 0.)),
                PathCommand::LineTo(v(40., 0.)),
                PathCommand::CubicTo {
                    c1: v(50., 10.),
                    c2: v(50., 30.),
                    to: v(40., 40.),
                },
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut generator = StableIdGenerator::new(41);
        let clock = ManualClock::new(5);
        let path = IdentifiedPath::assign(data, &mut || generator.next_id(&clock));
        let mut record = model.record(sid(12)).unwrap();
        record.id = sid(40);
        record.parent = Some(sid(10));
        record.order_key = model.next_child_order_key(sid(10));
        record.payload = NodePayload::Path(path);
        model.insert_node(record).unwrap();
        let mut stack = ModifierStack::new(sid(40));
        let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
        for (n, payload) in [
            ModifierPayload::Transform(at(50., 60.)),
            ModifierPayload::Stroke {
                color: black,
                width: FiniteF32::new(1.).unwrap(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(sid(4100 + n as u64)),
                    order: OrderKey::from_bytes(vec![0x40 + n as u8]).unwrap(),
                    kind: payload.kind(),
                    enabled: true,
                    payload,
                })
                .unwrap();
        }
        model.set_modifier_stack(stack).unwrap();
        model
    }

    #[test]
    fn enters_only_paths_and_never_converts_shapes() {
        // 검증 기준(XII.10.29): Shape 더블클릭이 Path로 변환하지 않음.
        let (_dir, w) = open("vector-enter", &path_document());
        let m = model(&w);
        let before = save_bytes(w.document().unwrap());
        assert_eq!(VectorEditMode::enter(m, sid(40)).unwrap().node(), sid(40));
        assert_eq!(
            VectorEditMode::enter(m, sid(13)),
            Err(EnterError::NotAPath),
            "Rectangle"
        );
        assert_eq!(
            VectorEditMode::from_navigation(m, &Navigation::EditContent(sid(13))),
            Some(Err(EnterError::NotAPath))
        );
        assert_eq!(
            VectorEditMode::from_navigation(m, &Navigation::Unchanged),
            None
        );
        assert!(matches!(
            model(&w).payload(sid(13)),
            Some(NodePayload::Shape(_))
        ));
        assert_eq!(save_bytes(w.document().unwrap()), before);
        assert_eq!(VectorEditMode::enter(m, sid(999)), Err(EnterError::Missing));
    }

    #[test]
    fn point_drag_records_one_path_command() {
        // 검증 기준(XII.10.29): 기존 Path의 point 드래그가 해당 Path command와 history 1건으로 기록.
        let (_dir, mut w) = open("vector-drag", &path_document());
        let s = scene(&w);
        let mode = VectorEditMode::enter(model(&w), sid(40)).unwrap();
        let world = path_world(model(&w), &s, sid(40)).unwrap();
        // point (40, 0)은 world (90, 60). 배율 2에서 반지름 5pt = world 2.5.
        let hit = mode.hit(model(&w), world, 2.0, [91.5, 61.0]).unwrap();
        let PathTarget::Point(point) = hit else {
            panic!("point")
        };
        assert!(mode.hit(model(&w), world, 2.0, [94.0, 60.0]).is_none());
        // 곡선 handle (50,10)은 world (100, 70).
        assert!(matches!(
            mode.hit(model(&w), world, 1.0, [100.0, 70.0]),
            Some(PathTarget::Handle(_))
        ));
        let before = history_len(&w);
        let revision = w.revision();
        let mut drag = mode
            .begin_drag(model(&w), world, hit, [91.5, 61.0])
            .unwrap();
        for step in 1..=100 {
            drag.update([91.5 + step as f32 * 0.1, 61.0 + step as f32 * 0.2]);
        }
        assert_eq!(
            (w.revision(), history_len(&w)),
            (revision, before),
            "끄는 동안 기록 없음"
        );
        assert_eq!(
            drag.edit(),
            Some(PathEdit::MovePoint {
                point,
                to: drag.clone().update([101.5, 81.0])
            })
        );
        assert!(drag.commit(&mut w).unwrap());
        assert_eq!(history_len(&w), before + 1);
        let Some(NodePayload::Path(path)) = model(&w).payload(sid(40)) else {
            panic!()
        };
        let moved = knobs(path)
            .into_iter()
            .find(|k| k.target == PathTarget::Point(point))
            .unwrap();
        assert!((moved.at.x() - 50.0).abs() < 1e-3 && (moved.at.y() - 20.0).abs() < 1e-3);
        // 점 id는 그대로, undo 한 번이 되돌립니다.
        assert!(w.undo().unwrap());
        let Some(NodePayload::Path(path)) = model(&w).payload(sid(40)) else {
            panic!()
        };
        assert_eq!(
            knobs(path)
                .into_iter()
                .find(|k| k.target == PathTarget::Point(point))
                .unwrap()
                .at,
            v(40., 0.)
        );
        // 움직이지 않은 drag·취소는 기록하지 않습니다.
        let still = mode
            .begin_drag(model(&w), world, hit, [90.0, 60.0])
            .unwrap();
        let after_undo = history_len(&w);
        assert!(!still.commit(&mut w).unwrap());
        assert_eq!(history_len(&w), after_undo);
    }
}
