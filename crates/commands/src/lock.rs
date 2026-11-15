//! node 잠금 판정 (D8).
//!
//! 자신이나 조상이 잠긴 node는 바꾸지 않습니다. `locked` 자체는 조상이 잠기지 않았을 때만 바꿀 수 있어, 잠금
//! 해제는 그 node에 대한 명시 명령으로만 합니다. 지금 이 판정을 쓰는 곳은 속성 패널(`inspector`)뿐입니다. 캔버스
//! (`runtime` `editable_ancestry`), Layers(`runtime` sidebar `path_locked`), CLI(`ops` apply `locked_change`)는 아직 각자
//! 판정하고 규칙도 다릅니다(root·흐름 부모·숨김 조상). 한 판정으로 모으는 일은 UX WBS UX.3.11입니다(ADR-0070 결정 5).

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_document::model::DocumentModel;

use crate::change::node_keys;

/// node 하나의 잠금 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeLock {
    /// 자신이 잠김.
    pub own: bool,
    /// 조상 중 하나가 잠김(자신 제외).
    pub ancestor: bool,
}

impl NodeLock {
    /// 문서에 있는 node의 잠금 상태. 문서에 없으면 `None`입니다. 비용은 조상 수에 비례합니다.
    pub fn of(model: &DocumentModel, id: StableId) -> Option<NodeLock> {
        let columns = model.columns();
        let handle = model.runtime_id(id)?;
        let own = columns.get(handle)?.locked;
        let mut ancestor = false;
        let mut parent = columns.parent(handle);
        // 검증한 문서에는 순환이 없지만, 잘못된 입력에서도 끝나도록 node 수로 상한을 둡니다.
        let mut remaining = columns.len();
        while let Some(current) = parent {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            if columns.get(current).is_some_and(|fields| fields.locked) {
                ancestor = true;
                break;
            }
            parent = columns.parent(current);
        }
        Some(NodeLock { own, ancestor })
    }

    /// 자신 또는 조상이 잠김.
    pub fn any(self) -> bool {
        self.own || self.ancestor
    }

    /// `key`를 바꾸는 편집을 막는지. `locked`는 조상 잠금만 막고, 그 밖은 자신 또는 조상 잠금이 막습니다.
    pub fn blocks(self, key: PropertyKey) -> bool {
        if key == node_keys::LOCKED {
            self.ancestor
        } else {
            self.any()
        }
    }
}
