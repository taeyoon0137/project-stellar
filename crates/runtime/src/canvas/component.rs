//! 정의의 Frame을 원본 인스턴스 한 개의 painter 행으로 펼칩니다. 문서에는 복제하지 않습니다.
use super::*;
use stellar_document::component::definition::ComponentEntry;
use stellar_document::component::instance::{frame_instance_transform, placed_free_frame};
use stellar_document::modifier::ModifierStack;
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
    let position = frame_instance_transform(stack)?;
    let tree = entry.tree();
    let root = tree.tree_root()?;
    let mut pending = vec![(root, scope.clone(), parent)];
    while let Some((id, parent_scope, parent)) = pending.pop() {
        let node = tree.node(id)?;
        if !node.visible {
            continue;
        }
        let frame = placed_free_frame(node, (id == root).then_some(position))?;
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
