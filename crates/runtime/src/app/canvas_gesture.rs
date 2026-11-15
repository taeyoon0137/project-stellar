//! 캔버스 직접 조작(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(포인터 아래 행, world delta, local 상자)만 보내고, 카메라·선택·편집의 의미와 command
//! 구성은 여기서 정합니다. 카메라 조작은 문서·history를 바꾸지 않습니다.

use super::frame_move::editable_ancestry;
use super::*;
use crate::canvas::CanvasScene;
use stellar_commands::selection::Selection;
use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_document::frame::CornerRadius;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierId, ModifierPayload};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;
use stellar_scene::bounds::Aabb;

/// 셸이 그린 행 위에서 고른 선택 의도(UX 명세 5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasPick {
    /// 포인터 아래 가장 위 행(`leaf`). `None`은 모든 Frame 밖입니다.
    Point {
        /// 셸이 그린 가장 위 행의 node.
        leaf: Option<StableId>,
        /// Shift: 선택에 넣고 빼기.
        toggle: bool,
        /// ⌘: 편집 범위와 관계없이 그 행을 바로 고릅니다.
        deep: bool,
        /// 더블클릭: 선택한 node 안으로 한 단계 들어갑니다.
        enter: bool,
    },
    /// world 영역. root의 직계 자식 가운데 겹치는(contain이면 완전히 포함된) node를 고릅니다.
    Area {
        /// 영역 한 꼭짓점(world).
        from: Vec2,
        /// 맞은편 꼭짓점(world).
        to: Vec2,
        /// Option: 완전히 포함된 것만.
        contain: bool,
        /// Shift: 지금 선택에 더합니다.
        add: bool,
    },
}

/// 캔버스가 표시하는 선택 노드 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasNodeKind {
    /// 자유 배치 Frame.
    Frame,
    /// Rectangle Shape.
    Rectangle,
    /// Ellipse Shape.
    Ellipse,
    /// Line Shape.
    Line,
    /// Polygon Shape.
    Polygon,
    /// Text.
    Text,
    /// Image.
    Image,
    /// 배치한 ComponentInstance.
    Instance,
}

/// 반경 핸들 방식.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadiusHandles {
    /// `border-*-radius` 네 longhand(Option으로 한 모서리).
    PerCorner,
    /// 값 하나(Polygon).
    Uniform,
}

/// 선택 표시와 핸들에 쓰는 노드 하나의 상자(코어가 정한 편집 가능 범위 포함).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasSelectionItem {
    /// 문서 node.
    pub id: StableId,
    /// 노드 종류.
    pub kind: CanvasNodeKind,
    /// local → world.
    pub world: Transform2D,
    /// local 상자(Frame·Rectangle·Text·Image는 원점 0, Ellipse·Polygon은 bounds, Line은 두 끝점 상자).
    pub rect: Rect,
    /// 저장된 모서리 반경(Frame·Rectangle은 네 모서리, Polygon은 같은 값 넷).
    pub radii: CornerRadius,
    /// 이동할 수 있는지(잠금·숨김·흐름 부모가 없음).
    pub movable: bool,
    /// 크기 핸들로 `width`·`height`를 바꿀 수 있는지.
    pub resizable: bool,
    /// 반경 핸들이 있는지와 그 방식.
    pub radius: Option<RadiusHandles>,
    /// 가로·세로가 고정 크기인지(아니면 크기 라벨에 `fit-content`).
    pub fixed: [bool; 2],
}

impl AppRuntime {
    /// 셸이 그린 행에서 고른 선택을 코어 규칙(직계 자식, Shift 넣고 빼기, ⌘ 깊은 선택, 영역)으로 정해 기록합니다.
    pub fn canvas_pick(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        pick: CanvasPick,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let sessions = ctx.app_sessions_mut();
            let entry = sessions
                .windows
                .get_mut(&session)
                .ok_or_else(|| unknown_window(session))?;
            let preview = entry
                .text
                .as_ref()
                .and_then(|c| c.preview_at(entry.session.revision()));
            let scene = entry
                .canvas
                .current(&entry.session, preview, &mut sessions.shared_scenes);
            let window = &mut entry.session;
            let selection = resolve_pick(window, scene.as_deref(), view, pick)?;
            window.select(selection).map_err(view_failure)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 화면 중심에서 `anchor` point인 점을 고정하고 배율을 `factor`배 합니다(핀치, ⌘ 스크롤).
    ///
    /// 배율 상하한은 viewport가 정합니다. 문서·history·저장 상태는 바뀌지 않습니다.
    pub fn zoom_viewport_by(
        &self,
        session: WindowSessionHandle,
        anchor: [f64; 2],
        factor: f64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            change_view_on_thread(ctx, request, session, |window| {
                window.zoom(factor, anchor[0], anchor[1]).map(|_| ())
            })
        })
    }
}

fn stale_view() -> Failure {
    Failure::new(
        RequestFailure::UnknownView,
        "캔버스의 View·선택·편집 상태가 변경되었습니다",
    )
}

pub(super) fn view_root(model: &DocumentModel, view: StableId) -> Option<StableId> {
    model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|v| v.id == view)
        .map(|v| v.root_frame)
}

pub(super) fn parent_of(model: &DocumentModel, node: StableId) -> Option<StableId> {
    let fields = model
        .runtime_id(node)
        .and_then(|id| model.columns().get(id))?;
    fields.parent.and_then(|id| model.stable_id(id))
}

/// `[node, 부모, …, root]`. root 아래가 아니면 `None`입니다.
fn ancestry(model: &DocumentModel, node: StableId, root: StableId) -> Option<Vec<StableId>> {
    let mut chain = vec![node];
    let mut current = node;
    while current != root {
        current = parent_of(model, current)?;
        chain.push(current);
    }
    Some(chain)
}

/// 클릭이 고를 node. 지금 선택과 같은 깊이(선택의 부모가 조상이면 그 자식)를 먼저, 없으면 root의 직계 자식을 고릅니다.
fn point_target(
    model: &DocumentModel,
    root: StableId,
    current: &[StableId],
    leaf: StableId,
    deep: bool,
    enter: bool,
) -> Option<StableId> {
    let chain = ancestry(model, leaf, root)?;
    if deep {
        return Some(leaf);
    }
    for i in 1..chain.len() {
        let scope = chain[i];
        let entered = enter && current.contains(&scope);
        let sibling = current
            .iter()
            .any(|&s| s != scope && parent_of(model, s) == Some(scope));
        if entered || sibling {
            return Some(chain[i - 1]);
        }
    }
    Some(match chain.len() {
        1 => root,
        n => chain[n - 2],
    })
}

fn resolve_pick(
    window: &ProjectWindowSession,
    scene: Option<&CanvasScene>,
    view: StableId,
    pick: CanvasPick,
) -> Result<Selection, Failure> {
    if window.tabs().active_view() != Some(view) {
        return Err(stale_view());
    }
    let model = window.document().ok_or_else(stale_view)?.model();
    let root = view_root(model, view).ok_or_else(stale_view)?;
    let current = window.view().selection().clone();
    let ids = current.ids();
    match pick {
        CanvasPick::Point {
            leaf,
            toggle,
            deep,
            enter,
        } => {
            let target = leaf.and_then(|leaf| point_target(model, root, ids, leaf, deep, enter));
            Ok(match (target, toggle) {
                (None, false) => Selection::empty(),
                (None, true) => current,
                (Some(target), false) => Selection::new([target], Some(target)),
                (Some(target), true) if current.contains(target) => {
                    let rest: Vec<StableId> =
                        ids.iter().copied().filter(|&id| id != target).collect();
                    let primary = current
                        .primary()
                        .filter(|&p| p != target)
                        .or(rest.last().copied());
                    Selection::new(rest, primary)
                }
                (Some(target), true) => {
                    Selection::new(ids.iter().copied().chain([target]), Some(target))
                }
            })
        }
        CanvasPick::Area {
            from,
            to,
            contain,
            add,
        } => {
            let area = Aabb::new(
                from.x().min(to.x()),
                from.y().min(to.y()),
                from.x().max(to.x()),
                from.y().max(to.y()),
            )
            .ok_or_else(stale_view)?;
            let bounds = scene.map(world_bounds).unwrap_or_default();
            let hits: Vec<StableId> = model
                .children_in_order(root)
                .into_iter()
                .filter(|child| {
                    let open = model
                        .runtime_id(*child)
                        .and_then(|id| model.columns().get(id))
                        .is_some_and(|f| f.visible && !f.locked);
                    open && bounds.get(child).is_some_and(|b| {
                        if contain {
                            b.min_x() >= area.min_x()
                                && b.min_y() >= area.min_y()
                                && b.max_x() <= area.max_x()
                                && b.max_y() <= area.max_y()
                        } else {
                            b.intersects(&area)
                        }
                    })
                })
                .collect();
            Ok(if add {
                let primary = hits.last().copied().or(current.primary());
                Selection::new(ids.iter().copied().chain(hits), primary)
            } else {
                let primary = hits.last().copied();
                Selection::new(hits, primary)
            })
        }
    }
}

/// 문서 node별 world 경계. 같은 node의 행(인스턴스 투영)은 합칩니다.
fn world_bounds(scene: &CanvasScene) -> std::collections::BTreeMap<StableId, Aabb> {
    let mut bounds = std::collections::BTreeMap::new();
    for row in &scene.frames {
        let Some(b) = Aabb::from_rect(row.geometry.rect).transformed(&row.world) else {
            continue;
        };
        bounds
            .entry(row.id)
            .and_modify(|a: &mut Aabb| *a = a.union(&b))
            .or_insert(b);
    }
    bounds
}

/// 표시한 선택의 상자와 편집 가능 범위. 셸은 이 값으로 외곽선·핸들·크기 라벨만 그립니다.
pub(crate) fn selection_items(
    session: &ProjectWindowSession,
    scene: Option<&CanvasScene>,
) -> std::sync::Arc<[CanvasSelectionItem]> {
    let (Some(scene), Some(document), Some(view)) =
        (scene, session.document(), session.tabs().active_view())
    else {
        return std::sync::Arc::from([]);
    };
    let model = document.model();
    let ids = session.view().selection().ids();
    let mut items = Vec::with_capacity(ids.len());
    for &id in ids {
        let (Some(row), Some(payload)) = (
            scene.frames.iter().find(|row| row.id == id),
            model.payload(id),
        ) else {
            continue;
        };
        let (kind, radii, radius) = match payload {
            NodePayload::Frame(frame) => (
                CanvasNodeKind::Frame,
                frame.corner_radius,
                Some(RadiusHandles::PerCorner),
            ),
            NodePayload::Shape(ShapeGeometry::Rectangle(rect)) => (
                CanvasNodeKind::Rectangle,
                rect.corner_radius,
                Some(RadiusHandles::PerCorner),
            ),
            NodePayload::Shape(ShapeGeometry::Polygon(polygon)) => {
                let r = polygon.corner_radius();
                (
                    CanvasNodeKind::Polygon,
                    CornerRadius::new(r, r, r, r).unwrap_or(CornerRadius::ZERO),
                    Some(RadiusHandles::Uniform),
                )
            }
            NodePayload::Shape(ShapeGeometry::Ellipse(_)) => {
                (CanvasNodeKind::Ellipse, CornerRadius::ZERO, None)
            }
            NodePayload::Shape(ShapeGeometry::Line(_)) => {
                (CanvasNodeKind::Line, CornerRadius::ZERO, None)
            }
            NodePayload::Text(_) => (CanvasNodeKind::Text, CornerRadius::ZERO, None),
            NodePayload::Image(_) => (CanvasNodeKind::Image, CornerRadius::ZERO, None),
            NodePayload::ComponentInstance(_) => {
                (CanvasNodeKind::Instance, CornerRadius::ZERO, None)
            }
            _ => continue,
        };
        let frame = kind == CanvasNodeKind::Frame;
        let movable = editable_ancestry(model, view, id, !frame).is_some();
        let resizable = movable
            && !matches!(kind, CanvasNodeKind::Text | CanvasNodeKind::Instance)
            && resize_transform(model, id, frame).is_some();
        let fixed = kind != CanvasNodeKind::Text;
        items.push(CanvasSelectionItem {
            id,
            kind,
            world: row.world,
            rect: row.geometry.rect,
            radii,
            movable,
            resizable,
            radius: radius.filter(|_| resizable),
            fixed: [fixed, fixed],
        });
    }
    items.into()
}

/// 크기를 바꿀 때 원점 이동을 기록할 변환. Frame은 자기 transform, 그 밖은 하나뿐인 Transform modifier(없으면 항등)입니다.
pub(super) fn resize_transform(
    model: &DocumentModel,
    node: StableId,
    frame: bool,
) -> Option<(Option<ModifierId>, Transform2D)> {
    if frame {
        let Some(NodePayload::Frame(payload)) = model.payload(node) else {
            return None;
        };
        return Some((None, payload.transform));
    }
    let stack = model.modifier_stack(node);
    let style = crate::canvas::shape::shape_style(stack)?;
    let transforms = stack.map_or(0, |stack| {
        stack
            .entries()
            .iter()
            .filter(|e| e.enabled && matches!(e.payload, ModifierPayload::Transform(_)))
            .count()
    });
    match (transforms, style.last_transform) {
        (0, None) => Some((None, Transform2D::identity())),
        (1, Some((id, transform))) => Some((Some(id), transform)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::canvas::CanvasCache;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_core_types::value::Size2;
    use stellar_document::frame::FramePayload;
    use stellar_document::records::NodeRecord;

    fn placed(w: f32, h: f32, x: f32, y: f32) -> NodePayload {
        NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            ..FramePayload::default()
        })
    }

    /// View 3의 root 10(400×300) 아래 11(10,10 100×100)과 그 자식 13(5,5 20×20), 12(200,50 50×50).
    pub(super) fn nested_document() -> DocumentModel {
        let mut model = sample_document();
        model
            .set_payload(sid(10), placed(400., 300., 0., 0.))
            .unwrap();
        model
            .set_payload(sid(11), placed(100., 100., 10., 10.))
            .unwrap();
        model
            .set_payload(sid(12), placed(50., 50., 200., 50.))
            .unwrap();
        let mut child = model.record(sid(11)).unwrap();
        child.id = sid(13);
        child.parent = Some(sid(11));
        child.payload = placed(20., 20., 5., 5.);
        model
            .insert_node(NodeRecord {
                display_name: "n13".into(),
                ..child
            })
            .unwrap();
        model
    }

    fn pick(
        window: &mut ProjectWindowSession,
        view: u64,
        pick: CanvasPick,
    ) -> Option<Vec<StableId>> {
        let scene = CanvasCache::default().current(window, None, &mut Default::default());
        let selection = resolve_pick(window, scene.as_deref(), sid(view), pick).ok()?;
        window.select(selection).unwrap();
        Some(window.view().selection().ids().to_vec())
    }

    fn point(leaf: Option<u64>, toggle: bool, deep: bool, enter: bool) -> CanvasPick {
        CanvasPick::Point {
            leaf: leaf.map(sid),
            toggle,
            deep,
            enter,
        }
    }

    fn area(from: (f32, f32), to: (f32, f32), contain: bool, add: bool) -> CanvasPick {
        CanvasPick::Area {
            from: Vec2::new(from.0, from.1).unwrap(),
            to: Vec2::new(to.0, to.1).unwrap(),
            contain,
            add,
        }
    }

    #[test]
    fn click_selects_at_selection_depth_with_toggle_deep_enter_and_area() {
        let dir = saved_project_with("canvas-pick", &[&nested_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let revision = session.revision();
        let ids = |v: &[u64]| v.iter().copied().map(sid).collect::<Vec<_>>();
        // 깊은 행을 눌러도 root의 직계 자식을 고르고, 더블클릭은 한 단계 들어갑니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, false, false)).unwrap(),
            ids(&[11])
        );
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, false, true)).unwrap(),
            ids(&[13])
        );
        // 선택과 같은 깊이가 없으면 다시 root의 직계 자식입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(12), false, false, false)).unwrap(),
            ids(&[12])
        );
        // ⌘는 깊은 선택, Shift는 넣고 빼기입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, true, false)).unwrap(),
            ids(&[13])
        );
        assert_eq!(
            pick(&mut session, 3, point(Some(12), true, false, false)).unwrap(),
            ids(&[13, 12])
        );
        assert_eq!(session.view().selection().primary(), Some(sid(12)));
        assert_eq!(
            pick(&mut session, 3, point(Some(12), true, false, false)).unwrap(),
            ids(&[13])
        );
        // root 배경 클릭은 root, 모든 Frame 밖 클릭은 비움, Shift 빈 곳은 그대로입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(10), true, false, false)).unwrap(),
            ids(&[13, 10])
        );
        assert_eq!(
            pick(&mut session, 3, point(None, true, false, false)).unwrap(),
            ids(&[13, 10])
        );
        assert_eq!(
            pick(&mut session, 3, point(None, false, false, false)).unwrap(),
            ids(&[])
        );
        // 영역: 겹치는 root 자식, Option은 완전히 포함된 것만, Shift는 더하기.
        assert_eq!(
            pick(&mut session, 3, area((150., 150.), (0., 0.), false, false)).unwrap(),
            ids(&[11])
        );
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (105., 105.), true, false)).unwrap(),
            ids(&[])
        );
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (300., 120.), true, false)).unwrap(),
            ids(&[11, 12])
        );
        pick(&mut session, 3, point(Some(12), false, false, false)).unwrap();
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (50., 50.), false, true)).unwrap(),
            ids(&[12, 11])
        );
        // 다른 View로 보낸 요청은 거부하고 선택을 바꾸지 않습니다.
        let before = session.view().selection().clone();
        assert!(pick(&mut session, 4, point(Some(11), false, false, false)).is_none());
        assert_eq!(session.view().selection(), &before);
        assert_eq!(session.revision(), revision);
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn area_skips_locked_and_hidden_children() {
        let mut model = nested_document();
        model.replace_flags(sid(12), true, true).unwrap();
        model.replace_flags(sid(11), false, false).unwrap();
        let dir = saved_project_with("canvas-pick-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        assert!(
            pick(&mut session, 3, area((0., 0.), (400., 300.), false, false))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn selection_items_report_boxes_and_handles_from_the_core() {
        let dir = saved_project_with("canvas-selection-items", &[&nested_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(13), sid(10)], Some(sid(13))))
            .unwrap();
        let scene = CanvasCache::default().current(&session, None, &mut Default::default());
        let items = selection_items(&session, scene.as_deref());
        assert_eq!(items.len(), 2);
        let child = &items[0];
        assert_eq!(child.id, sid(13));
        assert_eq!(child.kind, CanvasNodeKind::Frame);
        assert_eq!(child.world.components(), (1., 0., 0., 1., 15., 15.));
        assert_eq!(child.rect.size(), Size2::new(20., 20.).unwrap());
        assert!(child.movable && child.resizable);
        assert_eq!(child.radius, Some(RadiusHandles::PerCorner));
        assert_eq!(child.fixed, [true, true]);
        assert_eq!(items[1].id, sid(10));
    }

    #[test]
    fn continuous_zoom_keeps_anchor_and_leaves_document_and_history_alone() {
        let dir = saved_project_with("canvas-zoom-by", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        let before = app.state().window(session).unwrap().clone();
        let camera = before.viewport.unwrap();
        app.zoom_viewport_by(session, [40.0, -20.0], 1.25).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let after = app.state().window(session).unwrap().clone();
        let zoomed = after.viewport.unwrap();
        assert_eq!(zoomed.zoom(), camera.zoom() * 1.25);
        // anchor 화면 점 아래의 문서 점이 그대로입니다.
        let world = |c: CanvasViewport, x: f64| c.center_x() + x / c.zoom();
        assert!((world(camera, 40.0) - world(zoomed, 40.0)).abs() < 1e-9);
        assert_eq!(
            (after.revision, after.undo_depth, after.has_unsaved_changes),
            (
                before.revision,
                before.undo_depth,
                before.has_unsaved_changes
            )
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 1e9).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert_eq!(
            app.state()
                .window(session)
                .unwrap()
                .viewport
                .unwrap()
                .zoom(),
            stellar_commands::view_tabs::MAX_ZOOM
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 0.0).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { .. }
        ));
    }
}
