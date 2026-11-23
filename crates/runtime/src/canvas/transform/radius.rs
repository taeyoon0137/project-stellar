//! 모서리 반경 핸들 (XII.10.11, ADR-0105 결정 2).
//!
//! 반경 쓰기는 기존 캔버스 편집 경로 하나(`CanvasEdit::Radius`, ADR-0071 UX.2.10)를 그대로 씁니다. Frame·Rectangle은
//! 바뀐 `border-*-radius` longhand만, Polygon은 stellar-only 반경 key 하나를 바꾸며 묶음 key 0x0106을 쓰지 않습니다.
//! 이 module은 그 앞의 두 가지를 더합니다.
//!
//! - [`RadiusHandles`]: 핸들이 보일 값. 핸들 수치는 **저장값**이고, 상자보다 큰 값은 평가 규칙(VI.5.2,
//!   `RectangleParams::effective_corner_radius`)으로 줄인 **표시 반경**을 따로 둡니다. 저장값은 입력값 그대로입니다.
//! - [`RadiusHandles::edit`]: pointer와 수식키를 `gesture_math::preview`로 해석해 확정 요청 하나를 만듭니다. Option은
//!   Frame·Rectangle에서 잡은 모서리만 바꾸고, Polygon은 값이 하나라 Option에서도 같은 결과입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::FiniteF32;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_document::shape::{RectangleParams, ShapeGeometry};

use super::TransformError;
use crate::app::canvas_gesture::{apply_edit, CanvasEdit};
use crate::app::gesture_math::{preview, Corner, Gesture, GestureOutput, Modifiers};
use crate::app::RequestId;
use crate::canvas::CanvasScene;
use crate::window_session::ProjectWindowSession;

/// 반경 핸들 값.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusHandles {
    /// 대상.
    pub node: StableId,
    /// local 상자(x, y, width, height).
    pub rect: [f32; 4],
    /// 저장값(tl, tr, br, bl). Polygon은 같은 값 넷입니다. 핸들 수치는 이 값입니다.
    pub stored: [f32; 4],
    /// 평가한 표시 반경(상자보다 큰 값은 줄어듦).
    pub effective: [f32; 4],
    /// 값 하나(Polygon)인지.
    pub uniform: bool,
}

fn effective(
    size: stellar_core_types::value::Size2,
    corner_radius: stellar_document::frame::CornerRadius,
) -> [f32; 4] {
    let (a, b, c, d) = RectangleParams {
        size,
        corner_radius,
    }
    .effective_corner_radius();
    [a, b, c, d]
}

impl RadiusHandles {
    /// node의 반경 핸들 값. 반경이 없는 종류는 `None`입니다.
    pub fn of(model: &DocumentModel, node: StableId) -> Option<RadiusHandles> {
        let (rect, stored, effective, uniform) = match model.payload(node)? {
            NodePayload::Frame(frame) => {
                let (a, b, c, d) = frame.corner_radius.components();
                (
                    [0.0, 0.0, frame.size.width(), frame.size.height()],
                    [a, b, c, d],
                    effective(frame.size, frame.corner_radius),
                    false,
                )
            }
            NodePayload::Shape(ShapeGeometry::Rectangle(r)) => {
                let (a, b, c, d) = r.corner_radius.components();
                let (ea, eb, ec, ed) = r.effective_corner_radius();
                (
                    [0.0, 0.0, r.size.width(), r.size.height()],
                    [a, b, c, d],
                    [ea, eb, ec, ed],
                    false,
                )
            }
            NodePayload::Shape(ShapeGeometry::Polygon(p)) => {
                let b = p.bounds();
                let r = p.corner_radius();
                // Polygon 반경의 표시는 path 평가(VI.5.6)가 정합니다. 핸들은 저장값을 보입니다.
                (
                    [
                        b.origin().x(),
                        b.origin().y(),
                        b.size().width(),
                        b.size().height(),
                    ],
                    [r; 4],
                    [r; 4],
                    true,
                )
            }
            _ => return None,
        };
        Some(RadiusHandles {
            node,
            rect,
            stored,
            effective,
            uniform,
        })
    }

    /// pointer(local)와 수식키를 확정 요청으로 바꿉니다. 비유한 입력은 `None`입니다.
    pub fn edit(
        &self,
        corner: Corner,
        pointer: [f32; 2],
        modifiers: Modifiers,
    ) -> Option<CanvasEdit> {
        let corner = (!self.uniform).then_some(corner);
        let GestureOutput::Radii(next) = preview(
            Gesture::Radius {
                corner,
                rect: self.rect,
                radii: self.stored,
            },
            modifiers,
            pointer,
        )?
        else {
            return None;
        };
        let index = corner.map_or(0, |c| match c {
            Corner::TopLeft => 0,
            Corner::TopRight => 1,
            Corner::BottomRight => 2,
            Corner::BottomLeft => 3,
        });
        Some(CanvasEdit::Radius {
            node: self.node,
            corner: (!self.uniform && modifiers.option).then_some(index),
            radius: FiniteF32::new(next[index]).ok()?,
        })
    }
}

/// 확정 요청을 기존 캔버스 편집 경로로 기록합니다(Undo 한 번). `request`는 command ID를 만드는 요청 번호입니다.
pub fn commit(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    view: StableId,
    request: RequestId,
    edit: CanvasEdit,
) -> Result<(), TransformError> {
    apply_edit(window, scene, request, view, edit).map_err(|_| TransformError::Stale)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;

    fn stored(window: &ProjectWindowSession, node: u64) -> [f32; 4] {
        RadiusHandles::of(model(window), sid(node)).unwrap().stored
    }

    #[test]
    fn frame_and_rectangle_all_corners_single_corner_and_oversize() {
        for node in [11u64, 13] {
            let (_dir, mut w) = open("radius-corners", &canvas_document());
            select(&mut w, &[node]);
            let handles = RadiusHandles::of(model(&w), sid(node)).unwrap();
            // 왼쪽 위 핸들을 안쪽 대각선으로 16(반경 8): 네 모서리 동시.
            let edit = handles
                .edit(Corner::TopLeft, [8.0, 8.0], Modifiers::default())
                .unwrap();
            let before = history_len(&w);
            let s = scene(&w);
            commit(&mut w, &s, sid(3), RequestId(1), edit).unwrap();
            assert_eq!(history_len(&w), before + 1);
            assert_eq!(stored(&w, node), [8.0; 4]);
            // Option: 오른쪽 아래만.
            let handles = RadiusHandles::of(model(&w), sid(node)).unwrap();
            let r = handles.rect;
            let option = Modifiers {
                shift: false,
                option: true,
            };
            let edit = handles
                .edit(Corner::BottomRight, [r[2] - 4.0, r[3] - 4.0], option)
                .unwrap();
            let s = scene(&w);
            commit(&mut w, &s, sid(3), RequestId(2), edit).unwrap();
            assert_eq!(stored(&w, node), [8.0, 8.0, 4.0, 8.0]);
            // 상자보다 큰 저장값: 입력값 그대로 저장하고 표시만 줄입니다.
            let big = CanvasEdit::Radius {
                node: sid(node),
                corner: None,
                radius: FiniteF32::new(500.0).unwrap(),
            };
            let s = scene(&w);
            commit(&mut w, &s, sid(3), RequestId(3), big).unwrap();
            let handles = RadiusHandles::of(model(&w), sid(node)).unwrap();
            assert_eq!(handles.stored, [500.0; 4]);
            let short = handles.rect[2].min(handles.rect[3]) / 2.0;
            assert!(
                handles.effective.iter().all(|v| (v - short).abs() < 1e-3),
                "{:?}",
                handles.effective
            );
            // 새 command가 묶음 key 0x0106을 쓰지 않습니다(바뀐 key는 border-*-radius longhand).
            let entries = w.history().history().entries();
            let last = entries.last().unwrap();
            assert!(!format!("{last:?}").contains("PropertyKey(262)"));
        }
    }

    #[test]
    fn polygon_writes_one_key_and_option_gives_the_same_result() {
        let mut results = Vec::new();
        for option in [false, true] {
            let (_dir, mut w) = open("radius-polygon", &canvas_document());
            select(&mut w, &[16]);
            let handles = RadiusHandles::of(model(&w), sid(16)).unwrap();
            assert!(handles.uniform);
            assert_eq!(handles.stored, [3.0; 4]);
            let edit = handles
                .edit(
                    Corner::BottomRight,
                    [0.0, 6.0],
                    Modifiers {
                        shift: false,
                        option,
                    },
                )
                .unwrap();
            let s = scene(&w);
            commit(&mut w, &s, sid(3), RequestId(1), edit).unwrap();
            results.push(stored(&w, 16));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], [6.0; 4]);
        assert!(RadiusHandles::of(&canvas_document(), sid(14)).is_none());
    }
}
