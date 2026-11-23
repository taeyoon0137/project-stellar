//! `position: absolute` 자식의 inset 이동 (XII.10.24, ADR-0125).
//!
//! 흐름에서 빠진 자식(layout record에 `position: absolute`를 명시한 node)은 위치 원본이 inset이므로, 이동은 layout record의
//! inset longhand를 바꾸는 `SetLayoutProperty`입니다(저장 transform은 바꾸지 않음). 화면 위치는 layout 결과
//! ([`CanvasLayout`])로 읽고 끄는 동안의 preview도 같은 상자에서 만듭니다.
//!
//! # anchor
//!
//! 축마다 현재 anchor의 inset을 씁니다(속성 모델 4.5절).
//!
//! - 시작(`left`·`top`)만 정해져 있으면 시작을, 끝(`right`·`bottom`)만 정해져 있으면 끝을 바꿉니다. 끝은 반대 방향입니다.
//! - 둘 다 정해져 있으면(늘이기) 두 값을 같이 옮겨 크기를 지킵니다.
//! - 둘 다 `auto`(static position)면 시작 inset에 지금 layout 위치 + 이동량을 px로 둡니다.
//! - 백분율 inset은 containing block(부모 padding box) 크기로 바꾼 백분율로 둡니다.

use stellar_commands::command::Command;
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::css::style_defaults::resolve;
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_scene::document::layout::PlacementRule;
use stellar_scene::incremental::ScenePreview;
use stellar_scene::source::{SceneKey, SourceKind};

use crate::canvas::scene_layout::CanvasLayout;
use crate::canvas::transform::{selection_roots, TransformError};
use crate::window_session::{history_label, ProjectWindowSession};

/// 한 축의 inset 상태.
#[derive(Debug, Clone, PartialEq)]
struct Axis {
    start: LayoutLonghand,
    end: LayoutLonghand,
    start_value: LayoutValue,
    end_value: LayoutValue,
    /// 지금 layout 위치(부모 border box 좌표).
    position: f32,
    /// containing block 길이.
    block: f32,
}

fn is_auto(value: &LayoutValue) -> bool {
    matches!(value, LayoutValue::Keyword("auto"))
}

impl Axis {
    /// 이동량 `delta`(부모 local px)의 inset 쓰기.
    fn writes(&self, delta: f32) -> Vec<(LayoutLonghand, LayoutValue)> {
        if delta == 0.0 {
            return Vec::new();
        }
        let shifted = |value: &LayoutValue, by: f32| -> LayoutValue {
            match value {
                LayoutValue::Length(v) => LayoutValue::Length(v + by + 0.0),
                LayoutValue::Percentage(p) if self.block > 0.0 => {
                    LayoutValue::Percentage(p + by / self.block * 100.0 + 0.0)
                }
                _ => LayoutValue::Length(self.position + by + 0.0),
            }
        };
        match (is_auto(&self.start_value), is_auto(&self.end_value)) {
            (false, false) => vec![
                (self.start, shifted(&self.start_value, delta)),
                (self.end, shifted(&self.end_value, -delta)),
            ],
            (true, false) => vec![(self.end, shifted(&self.end_value, -delta))],
            // 시작만 정해졌거나 둘 다 auto(static position): 시작 inset.
            (_, true) => vec![(self.start, shifted(&self.start_value, delta))],
        }
    }
}

/// `position: absolute` 자식 하나의 inset 이동.
#[derive(Debug, Clone)]
pub struct InsetDrag {
    node: StableId,
    /// world 이동량 → 부모 local 이동량(선형 부분 역행렬).
    to_local: Transform2D,
    horizontal: Axis,
    vertical: Axis,
}

impl InsetDrag {
    /// 지금 선택(흐름에서 빠진 자식 하나)으로 시작합니다.
    pub fn begin(
        window: &ProjectWindowSession,
        layout: &CanvasLayout,
    ) -> Result<InsetDrag, TransformError> {
        let (model, _root, roots) = selection_roots(window)?;
        let [node] = roots.as_slice() else {
            return Err(TransformError::Disabled("single-absolute-child"));
        };
        let node = *node;
        let document_layout = layout.layout();
        let set = document_layout
            .style_set()
            .ok_or(TransformError::Disabled("layout-unavailable"))?;
        let payload = model.payload(node).ok_or(TransformError::Stale)?;
        let resolved =
            resolve(set, payload, model.layout_record(node)).ok_or(TransformError::Stale)?;
        if document_layout.rule(model, node) != PlacementRule::Layout
            || resolved.get(LayoutLonghand::Position).value != LayoutValue::Keyword("absolute")
        {
            return Err(TransformError::Disabled("not-absolute"));
        }
        edit_permission(model, node, EditIntent::Edit)
            .map_err(|_| TransformError::Disabled("locked"))?;
        let own = document_layout
            .layout_box(node)
            .ok_or(TransformError::Disabled("layout-unavailable"))?;
        let parent = document_layout
            .layout_parent(node)
            .ok_or(TransformError::Stale)?;
        let block = document_layout
            .layout_box(parent)
            .ok_or(TransformError::Stale)?
            .local_rect();
        let world = layout
            .source()
            .world_transform(SceneKey::node(parent), None)
            .ok_or(TransformError::Stale)?;
        let (a, b, c, d, _, _) = world.components();
        let to_local = Transform2D::new(a, b, c, d, 0.0, 0.0)
            .ok()
            .and_then(|linear| linear.inverse())
            .ok_or(TransformError::Disabled("singular-parent"))?;
        let rect = own.local_rect();
        let value = |longhand| resolved.get(longhand).value.clone();
        Ok(InsetDrag {
            node,
            to_local,
            horizontal: Axis {
                start: LayoutLonghand::Left,
                end: LayoutLonghand::Right,
                start_value: value(LayoutLonghand::Left),
                end_value: value(LayoutLonghand::Right),
                position: rect.x().to_px_f32().get(),
                block: block.width().to_px_f32().get(),
            },
            vertical: Axis {
                start: LayoutLonghand::Top,
                end: LayoutLonghand::Bottom,
                start_value: value(LayoutLonghand::Top),
                end_value: value(LayoutLonghand::Bottom),
                position: rect.y().to_px_f32().get(),
                block: block.height().to_px_f32().get(),
            },
        })
    }

    /// 끄는 node.
    pub fn node(&self) -> StableId {
        self.node
    }

    /// world 이동량의 부모 local 이동량.
    fn local(&self, delta: [f32; 2]) -> [f32; 2] {
        let (a, b, c, d, _, _) = self.to_local.components();
        [
            a * delta[0] + c * delta[1] + 0.0,
            b * delta[0] + d * delta[1] + 0.0,
        ]
    }

    /// world 이동량 `delta`의 command(축마다 anchor inset). 이동이 없으면 빈 목록입니다.
    pub fn commands(&self, delta: [f32; 2]) -> Vec<Command> {
        let [dx, dy] = self.local(delta);
        self.horizontal
            .writes(dx)
            .into_iter()
            .chain(self.vertical.writes(dy))
            .map(|(longhand, value)| {
                Command::SetLayoutProperty(SetLayoutProperty {
                    node: self.node,
                    longhand,
                    value,
                })
            })
            .collect()
    }

    /// 끄는 동안의 scene 대체 입력: node를 부모 local 이동량만큼 옮긴 입력(문서·history 불변).
    pub fn preview(&self, layout: &CanvasLayout, delta: [f32; 2]) -> ScenePreview {
        let mut preview = ScenePreview::new();
        let [dx, dy] = self.local(delta);
        let key = SceneKey::node(self.node);
        let Some(node) = layout.source().source.get(key) else {
            return preview;
        };
        let mut kind = node.kind.clone();
        let input = match &mut kind {
            SourceKind::Frame(input) => &mut input.node,
            SourceKind::Shape(input) => &mut input.node,
            SourceKind::Path(input) => &mut input.node,
            SourceKind::Text(input) => &mut input.node,
            SourceKind::Image(input) => &mut input.node,
            SourceKind::Fragment(_) => return preview,
        };
        let shift = Transform2D::new(1.0, 0.0, 0.0, 1.0, dx, dy).ok();
        if let Some(moved) = shift.and_then(|s| input.transform.then(&s).ok()) {
            input.transform = moved;
            preview.set(key, kind);
        }
        preview
    }

    /// pointer up: inset 쓰기를 history 한 항목으로 기록합니다. 기록하면 참입니다.
    pub fn commit(
        &self,
        window: &mut ProjectWindowSession,
        delta: [f32; 2],
    ) -> Result<bool, TransformError> {
        crate::canvas::transform::commit(window, self.commands(delta), history_label::MOVE)
    }
}

#[cfg(test)]
mod tests;
