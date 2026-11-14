//! 정의의 Frame을 원본 인스턴스 한 개의 painter 행으로 펼칩니다. 문서에는 복제하지 않습니다.
use super::*;
use stellar_document::component::definition::ComponentEntry;
use stellar_document::modifier::{ModifierPayload, ModifierStack};
use stellar_document::record_codec::decode_payload;
use stellar_scene::builder::SceneScope;

#[allow(clippy::too_many_arguments)]
pub(super) fn project(
    scene: &mut CanvasScene,
    builder: &mut SceneBuilder,
    scope: &SceneScope,
    parent: Option<u32>,
    instance: StableId,
    locked: bool,
    stack: Option<&ModifierStack>,
    entry: &ComponentEntry,
) -> Option<()> {
    let mut position = Transform2D::identity();
    for modifier in stack
        .into_iter()
        .flat_map(ModifierStack::entries)
        .filter(|m| m.enabled)
    {
        let ModifierPayload::Transform(next) = modifier.payload else {
            return None;
        };
        position = position.then(&next).ok()?;
    }
    let tree = entry.tree();
    let root = tree.tree_root()?;
    let mut pending = vec![(root, scope.clone(), parent)];
    while let Some((id, parent_scope, parent)) = pending.pop() {
        let node = tree.node(id)?;
        if !node.visible {
            continue;
        }
        let NodePayload::Frame(mut frame) = decode_payload(node.kind, &node.payload).ok()? else {
            return None;
        };
        if frame.layout_mode.is_some() || matches!(frame.clipping, Clipping::Path(_)) {
            return None;
        }
        if id == root {
            let (a, b, c, d, _, _) = frame.transform.components();
            frame.transform = Transform2D::new(a, b, c, d, 0., 0.)
                .ok()?
                .then(&position)
                .ok()?;
        }
        let (scope, index) =
            scene.push_frame(builder, &parent_scope, parent, instance, &frame, locked)?;
        scene.frames[index as usize].instance_frame = true;
        pending.extend(
            tree.children(id)
                .iter()
                .rev()
                .map(|child| (*child, scope.clone(), Some(index))),
        );
    }
    Some(())
}
