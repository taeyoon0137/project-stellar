//! 눈금자 눈금과 가이드 제스처 (XII.10.19, ADR-0117 결정 3).
//!
//! - **C1**: 눈금자는 셸이 화면 고정 띠로 그립니다. 눈금 값과 화면 위치는 이 module이 scene camera와 같은
//!   [`CanvasViewport`]에서 계산하므로 camera와 어긋나지 않습니다([`ruler_ticks`]).
//! - 눈금 원점은 primary 선택의 최상위 Frame local 원점, 없으면 View root 원점입니다([`ruler_origin`]).
//! - 가이드 제스처([`GuideDrag`])는 눈금자에서 끌어 만들고, 선을 잡아 옮기고, 눈금자로 끌어 지웁니다. 결과는
//!   문서 가이드 편집 값([`GuideEdit`])이며 제스처 하나에 편집 하나입니다.
//! - 가이드 선은 스냅 엔진의 가이드 후보로 들어갑니다([`guide_snap_lines`]).
//!
//! 좌표: world는 캔버스 world(View root 좌표), 화면은 캔버스 왼쪽 위 원점 pt입니다. 가이드 위치는 소유 Frame local
//! px입니다. 소유 Frame은 평행 이동만 있는 Frame만 받습니다(회전·확대된 최상위 Frame은 축이 맞지 않음).

use stellar_commands::view_tabs::CanvasViewport;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::guides::{GuideAxis, GuideEdit, NodeGuides, RulerGuide};
use stellar_document::model::DocumentModel;

use crate::app::canvas_gesture::parent_of;
use crate::canvas::CanvasScene;

/// 큰 눈금의 최소 화면 간격(pt).
pub const MAJOR_MIN_SPACING_PT: f64 = 64.0;
/// 가이드 선을 잡는 화면 거리(pt).
pub const GUIDE_HIT_PT: f64 = 3.0;

/// 눈금 하나.
#[derive(Clone, Debug, PartialEq)]
pub struct RulerTick {
    /// 눈금자 축 방향 화면 좌표(pt).
    pub screen: f64,
    /// 원점 기준 CSS px 값.
    pub value: f64,
    /// 큰 눈금이면 라벨이 있습니다.
    pub label: Option<String>,
}

/// 큰 눈금 간격(CSS px): 1·2·5×10ⁿ 중 화면 간격이 [`MAJOR_MIN_SPACING_PT`] 이상인 가장 작은 값과 작은 눈금 등분 수.
pub fn tick_spacing(zoom: f64) -> Option<(f64, u32)> {
    if !(zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    let mut exponent = (MAJOR_MIN_SPACING_PT / zoom).log10().floor() as i32 - 1;
    loop {
        for (lead, divisions) in [(1.0, 5), (2.0, 4), (5.0, 5)] {
            let spacing = lead * 10f64.powi(exponent);
            if spacing * zoom >= MAJOR_MIN_SPACING_PT - 1e-9 {
                return Some((spacing, divisions));
            }
        }
        exponent += 1;
    }
}

/// 눈금 라벨: CSS px 값(간격 정밀도로 반올림, 지수 없음).
fn label(value: f64) -> String {
    stellar_document::css::value::format_number(value as f32)
}

/// `axis` 눈금자의 눈금. `axis`가 x면 위쪽 가로 눈금자(x 값), y면 왼쪽 세로 눈금자(y 값)입니다. `view_size`는
/// 캔버스 화면 크기(pt), `origin`은 눈금 0의 world 좌표입니다.
pub fn ruler_ticks(
    viewport: CanvasViewport,
    view_size: [f64; 2],
    origin: [f64; 2],
    axis: GuideAxis,
) -> Vec<RulerTick> {
    let i = axis.index();
    let (zoom, size) = (viewport.zoom(), view_size[i]);
    let center = [viewport.center_x(), viewport.center_y()][i];
    let Some((major, divisions)) = tick_spacing(zoom) else {
        return Vec::new();
    };
    if !(size.is_finite() && size > 0.0 && origin[i].is_finite()) {
        return Vec::new();
    }
    let minor = major / f64::from(divisions);
    // 화면 0..size가 보는 world 범위를 원점 기준 값으로.
    let low = center - size / 2.0 / zoom - origin[i];
    let high = center + size / 2.0 / zoom - origin[i];
    let first = (low / minor).floor() as i64;
    let last = (high / minor).ceil() as i64;
    (first..=last)
        .filter_map(|k| {
            let value = k as f64 * minor;
            let screen = (value + origin[i] - center) * zoom + size / 2.0;
            if !(0.0..=size).contains(&screen) {
                return None;
            }
            let is_major = k.rem_euclid(i64::from(divisions)) == 0;
            Some(RulerTick {
                screen,
                value: value + 0.0,
                label: is_major.then(|| label(value)),
            })
        })
        .collect()
}

/// 평행 이동만 있는 변환이면 그 이동량.
fn translation_only(world: &Transform2D) -> Option<[f32; 2]> {
    let (a, b, c, d, e, f) = world.components();
    (a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0).then_some([e, f])
}

/// `node`의 최상위 Frame(View root의 직계 Frame 조상, 자신 포함).
pub fn top_level_frame(model: &DocumentModel, root: StableId, node: StableId) -> Option<StableId> {
    let mut current = node;
    loop {
        let parent = parent_of(model, current)?;
        if parent == root {
            return matches!(
                model.payload(current),
                Some(stellar_document::records::NodePayload::Frame(_))
            )
            .then_some(current);
        }
        current = parent;
    }
}

fn world_of(scene: &CanvasScene, node: StableId) -> Option<Transform2D> {
    scene
        .frames
        .iter()
        .find(|row| row.id == node)
        .map(|row| row.world)
}

/// 눈금 원점(world)과 그 원점의 소유 Frame. primary 선택(없으면 첫 선택)의 최상위 Frame이 평행 이동만 있으면 그
/// local 원점, 아니면 View root 원점입니다.
pub fn ruler_origin(
    model: &DocumentModel,
    scene: &CanvasScene,
    root: StableId,
    primary: Option<StableId>,
) -> (StableId, [f32; 2]) {
    let root_origin = world_of(scene, root)
        .and_then(|w| translation_only(&w))
        .unwrap_or([0.0, 0.0]);
    primary
        .and_then(|node| top_level_frame(model, root, node))
        .and_then(|frame| Some((frame, translation_only(&world_of(scene, frame)?)?)))
        .unwrap_or((root, root_origin))
}

/// 새 가이드의 소유자: pointer(world) 아래 가장 위의 평행 이동만 있는 최상위 Frame, 없으면 View root.
pub fn guide_owner(
    model: &DocumentModel,
    scene: &CanvasScene,
    root: StableId,
    pointer: [f32; 2],
) -> (StableId, [f32; 2]) {
    for row in scene.frames.iter().rev() {
        if row.id == root || parent_of(model, row.id) != Some(root) || !row.hit_target {
            continue;
        }
        if !matches!(
            model.payload(row.id),
            Some(stellar_document::records::NodePayload::Frame(_))
        ) {
            continue;
        }
        let Some([x, y]) = translation_only(&row.world) else {
            continue;
        };
        let r = row.geometry.rect;
        let local = [pointer[0] - x, pointer[1] - y];
        if local[0] >= r.origin().x()
            && local[1] >= r.origin().y()
            && local[0] <= r.origin().x() + r.size().width()
            && local[1] <= r.origin().y() + r.size().height()
        {
            return (row.id, [x, y]);
        }
    }
    let origin = world_of(scene, root)
        .and_then(|w| translation_only(&w))
        .unwrap_or([0.0, 0.0]);
    (root, origin)
}

/// 진행 중 가이드 제스처의 종류.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideDragKind {
    /// 눈금자에서 끌어 새 가이드.
    Create,
    /// 기존 가이드 이동(시작 위치).
    Move {
        /// 대상 가이드.
        id: StableId,
        /// 제스처 전 위치(소유자 local px).
        before: f32,
    },
}

/// 가이드 제스처. update는 순수 계산이고 [`GuideDrag::finish`]가 편집 하나를 돌려줍니다.
#[derive(Clone, Debug, PartialEq)]
pub struct GuideDrag {
    owner: StableId,
    origin: [f32; 2],
    axis: GuideAxis,
    kind: GuideDragKind,
    position: f32,
}

/// 가이드 제스처가 끝난 결과.
#[derive(Clone, Debug, PartialEq)]
pub enum GuideDragEnd {
    /// 문서에 기록할 편집 하나(소유자와 함께).
    Edit {
        /// 소유 Frame.
        owner: StableId,
        /// 편집.
        edit: GuideEdit,
    },
    /// 기록할 것이 없음(눈금자로 돌아간 새 가이드, 움직이지 않은 이동, 취소).
    Nothing,
}

impl GuideDrag {
    /// 눈금자에서 새 가이드를 끌기 시작합니다. `owner`·`origin`은 [`guide_owner`] 결과입니다.
    pub fn from_ruler(owner: StableId, origin: [f32; 2], axis: GuideAxis) -> GuideDrag {
        GuideDrag {
            owner,
            origin,
            axis,
            kind: GuideDragKind::Create,
            position: 0.0,
        }
    }

    /// 화면 점 `pointer`가 가이드 선 위(화면 [`GUIDE_HIT_PT`] 안)이면 그 가이드 이동을 시작합니다. 가까운 선이
    /// 이기고 같으면 작은 id입니다.
    pub fn on_guide(
        record: &NodeGuides,
        origin: [f32; 2],
        viewport: CanvasViewport,
        view_size: [f64; 2],
        pointer: [f64; 2],
    ) -> Option<GuideDrag> {
        let zoom = viewport.zoom();
        let center = [viewport.center_x(), viewport.center_y()];
        let mut best: Option<(f64, &RulerGuide)> = None;
        for guide in record.guides() {
            let i = guide.axis.index();
            let world = f64::from(origin[i]) + f64::from(guide.position);
            let screen = (world - center[i]) * zoom + view_size[i] / 2.0;
            let distance = (screen - pointer[i]).abs();
            if distance <= GUIDE_HIT_PT && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, guide));
            }
        }
        let (_, guide) = best?;
        Some(GuideDrag {
            owner: record.owner(),
            origin,
            axis: guide.axis,
            kind: GuideDragKind::Move {
                id: guide.id,
                before: guide.position,
            },
            position: guide.position,
        })
    }

    /// 축.
    pub fn axis(&self) -> GuideAxis {
        self.axis
    }

    /// 소유 Frame.
    pub fn owner(&self) -> StableId {
        self.owner
    }

    /// 지금 preview 위치(소유자 local px).
    pub fn position(&self) -> f32 {
        self.position
    }

    /// pointer(world) update. `pixel_snap`이면 정수 소유자 px로 맞춥니다(호출자가 토글과 Control 해제를 반영해
    /// 넘김). 비유한 pointer는 무시합니다.
    pub fn update(&mut self, pointer: [f32; 2], pixel_snap: bool) -> f32 {
        let i = self.axis.index();
        let local = pointer[i] - self.origin[i];
        if local.is_finite() {
            self.position = if pixel_snap { local.round() } else { local } + 0.0;
        }
        self.position
    }

    /// 제스처를 끝냅니다. `over_ruler`는 pointer를 눈금자 위에서 놓았는지, `new_id`는 새 가이드 id 원천입니다.
    pub fn finish(self, over_ruler: bool, new_id: &mut dyn FnMut() -> StableId) -> GuideDragEnd {
        let edit = match (self.kind, over_ruler) {
            (GuideDragKind::Create, true) => return GuideDragEnd::Nothing,
            (GuideDragKind::Create, false) => {
                match RulerGuide::new(new_id(), self.axis, self.position) {
                    Ok(guide) => GuideEdit::Add(guide),
                    Err(_) => return GuideDragEnd::Nothing,
                }
            }
            (GuideDragKind::Move { id, .. }, true) => GuideEdit::Remove { id },
            (GuideDragKind::Move { before, .. }, false) if before == self.position => {
                return GuideDragEnd::Nothing
            }
            (GuideDragKind::Move { id, .. }, false) => GuideEdit::Move {
                id,
                position: self.position,
            },
        };
        GuideDragEnd::Edit {
            owner: self.owner,
            edit,
        }
    }
}

/// 가이드 기록들을 대상 부모 local 좌표의 스냅 가이드 선으로 바꿉니다. `records`는 (기록, 소유자 world 원점),
/// `parent_world`는 끄는 node 부모의 local → world입니다. 부모가 회전·기울어져 있으면 축이 맞지 않아 선을 내지
/// 않습니다.
pub fn guide_snap_lines(
    records: &[(&NodeGuides, [f32; 2])],
    parent_world: Transform2D,
) -> [Vec<f32>; 2] {
    let (a, b, c, d, e, f) = parent_world.components();
    if b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 {
        return [Vec::new(), Vec::new()];
    }
    let mut lines: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    for (record, origin) in records {
        for axis in [GuideAxis::X, GuideAxis::Y] {
            let i = axis.index();
            let (scale, shift) = if i == 0 { (a, e) } else { (d, f) };
            lines[i].extend(
                record
                    .positions(axis)
                    .into_iter()
                    .map(|p| (p + origin[i] - shift) / scale)
                    .filter(|v| v.is_finite()),
            );
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::sid;
    use crate::canvas::snap::{snap, SnapCandidates, SnapLines, SnapMode, SnapSettings};
    use crate::canvas::transform::fixtures::{canvas_document, model, open, scene};

    fn viewport(x: f64, y: f64, zoom: f64) -> CanvasViewport {
        CanvasViewport::new(x, y, zoom).unwrap()
    }

    #[test]
    fn spacing_follows_one_two_five_steps() {
        assert_eq!(tick_spacing(1.0), Some((100.0, 5)));
        assert_eq!(tick_spacing(0.5), Some((200.0, 4)));
        assert_eq!(tick_spacing(2.0), Some((50.0, 5)));
        assert_eq!(tick_spacing(64.0), Some((1.0, 5)));
        assert_eq!(tick_spacing(256.0), Some((0.5, 5)));
        assert_eq!(tick_spacing(0.0), None);
        for zoom in [0.02, 0.37, 1.0, 13.0, 256.0] {
            let (major, _) = tick_spacing(zoom).unwrap();
            assert!(major * zoom >= MAJOR_MIN_SPACING_PT - 1e-9);
        }
    }

    #[test]
    fn ticks_follow_the_camera_after_pan_and_zoom() {
        // 검증 기준(XII.10.19): camera 이동 후 눈금 값 정확, 눈금자 표현(C1)의 camera 동기.
        let size = [800.0, 600.0];
        for (vp, origin) in [
            (viewport(0.0, 0.0, 1.0), [0.0, 0.0]),
            (viewport(1234.5, -77.0, 2.5), [100.0, 40.0]),
            (viewport(-5000.0, 300.0, 0.05), [0.0, 0.0]),
        ] {
            let camera = vp.world_to_view(800.0, 600.0).unwrap();
            for axis in [GuideAxis::X, GuideAxis::Y] {
                let ticks = ruler_ticks(vp, size, origin, axis);
                assert!(!ticks.is_empty());
                assert!(ticks.iter().any(|t| t.label.is_some()));
                for tick in &ticks {
                    // 같은 world 점을 scene camera로 옮긴 화면 좌표와 같습니다.
                    let world = tick.value + origin[axis.index()];
                    let p = camera
                        .apply(
                            stellar_core_types::value::Vec2::new(world as f32, world as f32)
                                .unwrap(),
                        )
                        .unwrap();
                    let screen = [p.x(), p.y()][axis.index()];
                    assert!(
                        (f64::from(screen) - tick.screen).abs() < 0.05,
                        "{tick:?} vs {screen}"
                    );
                }
            }
        }
        // 원점 정렬: 100% 원점 중심이면 화면 가운데(400pt)가 0이고 라벨은 "0".
        let ticks = ruler_ticks(viewport(0.0, 0.0, 1.0), size, [0.0, 0.0], GuideAxis::X);
        let zero = ticks.iter().find(|t| t.value == 0.0).unwrap();
        assert_eq!(zero.screen, 400.0);
        assert_eq!(zero.label.as_deref(), Some("0"));
        assert!(ticks
            .iter()
            .any(|t| t.value == 300.0 && t.label.as_deref() == Some("300")));
        assert!(ticks.iter().any(|t| t.value == 20.0 && t.label.is_none()));
    }

    #[test]
    fn origin_is_the_selected_top_level_frame() {
        let (_dir, w) = open("ruler-origin", &canvas_document());
        let scene = scene(&w);
        let model = model(&w);
        assert_eq!(
            ruler_origin(model, &scene, sid(10), Some(sid(11))),
            (sid(11), [10.0, 20.0])
        );
        assert_eq!(
            ruler_origin(model, &scene, sid(10), None),
            (sid(10), [0.0, 0.0])
        );
        // Rectangle 13은 Frame이 아니라 최상위 Frame이 없습니다 → View root.
        assert_eq!(
            ruler_origin(model, &scene, sid(10), Some(sid(13))),
            (sid(10), [0.0, 0.0])
        );
        assert_eq!(
            guide_owner(model, &scene, sid(10), [50.0, 30.0]),
            (sid(11), [10.0, 20.0])
        );
        assert_eq!(
            guide_owner(model, &scene, sid(10), [390.0, 290.0]),
            (sid(10), [0.0, 0.0])
        );
    }

    #[test]
    fn create_move_delete_and_snap_to_guides() {
        // 검증 기준(XII.10.19): 가이드 생성·이동·삭제·스냅 fixture.
        let mut record = NodeGuides::new(sid(11));
        let mut next = 500u64;
        let mut ids = || {
            next += 1;
            sid(next)
        };
        // 1) 눈금자에서 끌어 만들기(픽셀 스냅), Frame 11 원점 (10, 20).
        let mut drag = GuideDrag::from_ruler(sid(11), [10.0, 20.0], GuideAxis::X);
        drag.update([42.4, 999.0], true);
        assert_eq!(drag.position(), 32.0);
        let GuideDragEnd::Edit { owner, edit } = drag.finish(false, &mut ids) else {
            panic!("생성")
        };
        assert_eq!(owner, sid(11));
        let undo_add = record.apply(&edit).unwrap();
        let id = record.guides().next().unwrap().id;
        assert_eq!(record.positions(GuideAxis::X), vec![32.0]);
        // 눈금자로 되돌아간 새 가이드는 편집이 없습니다.
        let back = GuideDrag::from_ruler(sid(11), [10.0, 20.0], GuideAxis::Y);
        assert_eq!(back.finish(true, &mut ids), GuideDragEnd::Nothing);

        // 2) 선을 잡아 옮기기: world x = 42 → 화면(100%, 중심 0, 폭 800) 442pt.
        let vp = viewport(0.0, 0.0, 1.0);
        let mut moving =
            GuideDrag::on_guide(&record, [10.0, 20.0], vp, [800.0, 600.0], [443.5, 10.0]).unwrap();
        assert!(
            GuideDrag::on_guide(&record, [10.0, 20.0], vp, [800.0, 600.0], [446.0, 10.0]).is_none()
        );
        moving.update([70.6, 0.0], false);
        let GuideDragEnd::Edit { edit, .. } = moving.clone().finish(false, &mut ids) else {
            panic!("이동")
        };
        let undo_move = record.apply(&edit).unwrap();
        assert!((record.guide(id).unwrap().position - 60.6).abs() < 1e-4);

        // 3) 스냅: Frame 11 안의 자식(부모 world = 평행 이동 (10,20))이 가이드 선에 맞습니다.
        let lines = guide_snap_lines(
            &[(&record, [10.0, 20.0])],
            Transform2D::new(1.0, 0.0, 0.0, 1.0, 10.0, 20.0).unwrap(),
        );
        assert_eq!(lines[0].len(), 1);
        let candidates = SnapCandidates::with_lines(
            Vec::new(),
            SnapLines {
                guides: lines,
                grid: [Vec::new(), Vec::new()],
            },
            [0.0, 0.0, 1.0, 1.0],
        );
        let settings = SnapSettings {
            objects: true,
            pixels: false,
        };
        let snapped = snap(
            [62.0, 5.0, 10.0, 10.0],
            SnapMode::Move,
            5.0,
            settings,
            &candidates,
        )
        .unwrap();
        assert!((snapped.rect[0] - 60.6).abs() < 1e-4);
        // 회전한 부모에는 가이드 선을 내지 않습니다.
        let rotated = Transform2D::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0).unwrap();
        assert!(guide_snap_lines(&[(&record, [0.0, 0.0])], rotated)[0].is_empty());

        // 4) 눈금자로 끌어 지우기.
        let deleting =
            GuideDrag::on_guide(&record, [10.0, 20.0], vp, [800.0, 600.0], [470.6, 0.0]).unwrap();
        let GuideDragEnd::Edit { edit, .. } = deleting.finish(true, &mut ids) else {
            panic!("삭제")
        };
        assert_eq!(edit, GuideEdit::Remove { id });
        let undo_remove = record.apply(&edit).unwrap();
        assert!(record.is_empty());
        // 역 편집을 거꾸로 적용하면 처음으로 돌아갑니다(history 한 항목 = 편집 하나).
        for undo in [undo_remove, undo_move, undo_add] {
            record.apply(&undo).unwrap();
        }
        assert!(record.is_empty());
        // 움직이지 않은 이동은 편집이 없습니다.
        record
            .apply(&GuideEdit::Add(
                RulerGuide::new(sid(9), GuideAxis::Y, 5.0).unwrap(),
            ))
            .unwrap();
        let still =
            GuideDrag::on_guide(&record, [10.0, 20.0], vp, [800.0, 600.0], [0.0, 325.0]).unwrap();
        assert_eq!(still.finish(false, &mut ids), GuideDragEnd::Nothing);
    }
}
