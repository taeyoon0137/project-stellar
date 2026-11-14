//! `SemanticSnapshot`: 프로젝트 의미 상태의 StableId index (XIV.3.1).
//!
//! Git HEAD·Index·Working Tree 등 한 시점의 프로젝트를 **영속 의미 단위 객체**의 평평한
//! StableId map으로 펼칩니다(아키텍처 §26). 객체는 프로젝트 metadata, 문서·페이지·View,
//! scene node, modifier, 컴포넌트 정의·정의 node·Slot, token, asset 등이며, 각 객체는
//! 위치(parent·OrderKey)와 `PropertyPath → SemanticValue` 속성 map을 가집니다.
//!
//! - **runtime cache 제외**: RuntimeEntityId·평가 결과·layout·render cache는 표현할 필드가
//!   없습니다. 같은 영속 레코드는 runtime 이력과 무관하게 같은 snapshot이 됩니다.
//! - **저장 배치 무관**: 객체 순서는 StableId 순이고 chunk·파일 배치를 기록하지 않습니다.
//! - **식별**: 객체는 StableId로만 찾습니다. 이름·geometry 유사도는 쓰지 않습니다(XIV.3.3).
//! - 모든 속성 경로의 root는 소유 객체 StableId입니다([`SemanticObject::set_property`]가 강제).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PropertyKey, PropertyPath};

pub use stellar_document::modifier::ModifierKind;
pub use stellar_document::node_kind::NodeKind;

use crate::keys;
use crate::value::SemanticValue;

/// semantic snapshot 객체 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    /// 프로젝트 metadata.
    Project,
    /// 문서 정의.
    Document,
    /// 페이지 정의.
    Page,
    /// View 정의.
    View,
    /// node(scene 또는 컴포넌트 정의 tree). 소속은 [`Origin`]이 구분합니다.
    Node(NodeKind),
    /// modifier stack 항목.
    Modifier(ModifierKind),
    /// 컴포넌트 정의 resource.
    Component,
    /// 컴포넌트 Slot.
    ComponentSlot,
    /// token.
    Token,
    /// asset record.
    Asset,
    /// expression AST node.
    ExpressionNode,
    /// View Variant override 묶음.
    Variant,
}

impl ObjectKind {
    /// 안정 numeric code. Node·Modifier는 하위 바이트에 종류 값을 담습니다.
    pub fn code(self) -> u16 {
        match self {
            ObjectKind::Project => 0x0001,
            ObjectKind::Document => 0x0002,
            ObjectKind::Page => 0x0003,
            ObjectKind::View => 0x0004,
            ObjectKind::Node(kind) => 0x0100 | kind.as_u16(),
            ObjectKind::Modifier(kind) => 0x0200 | kind.as_u16(),
            ObjectKind::Component => 0x0301,
            ObjectKind::ComponentSlot => 0x0302,
            ObjectKind::Token => 0x0401,
            ObjectKind::Asset => 0x0501,
            ObjectKind::ExpressionNode => 0x0601,
            ObjectKind::Variant => 0x0701,
        }
    }

    /// code에서 복원합니다. 알 수 없는 code는 `None`.
    pub fn from_code(code: u16) -> Option<ObjectKind> {
        let low = code & 0x00FF;
        match code & 0xFF00 {
            0x0100 => NodeKind::from_u16(low).map(ObjectKind::Node),
            0x0200 => ModifierKind::from_u16(low).map(ObjectKind::Modifier),
            _ => match code {
                0x0001 => Some(ObjectKind::Project),
                0x0002 => Some(ObjectKind::Document),
                0x0003 => Some(ObjectKind::Page),
                0x0004 => Some(ObjectKind::View),
                0x0301 => Some(ObjectKind::Component),
                0x0302 => Some(ObjectKind::ComponentSlot),
                0x0401 => Some(ObjectKind::Token),
                0x0501 => Some(ObjectKind::Asset),
                0x0601 => Some(ObjectKind::ExpressionNode),
                0x0701 => Some(ObjectKind::Variant),
                _ => None,
            },
        }
    }
}

/// 객체가 속한 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// 프로젝트 범위(metadata, 프로젝트 token, asset).
    Project,
    /// 문서 scene·정의 범위.
    Document(StableId),
    /// 문서의 컴포넌트 정의 범위(정의 resource, 정의 tree node, Slot).
    Component {
        /// 소유 문서.
        document: StableId,
        /// 소유 컴포넌트 정의.
        component: StableId,
    },
    /// 프로젝트 공유 라이브러리의 정의·tree·Slot.
    ProjectComponent(StableId),
}

impl Origin {
    /// 정의 객체나 그 안의 tree·Slot이 속하는 컴포넌트.
    pub fn component(self) -> Option<StableId> {
        match self {
            Self::Component { component, .. } | Self::ProjectComponent(component) => {
                Some(component)
            }
            _ => None,
        }
    }

    /// 소유 문서. 프로젝트 범위면 `None`.
    pub fn document(&self) -> Option<StableId> {
        match self {
            Origin::Project | Origin::ProjectComponent(_) => None,
            Origin::Document(document) | Origin::Component { document, .. } => Some(*document),
        }
    }
}

/// snapshot 구성·변경 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 같은 StableId 객체가 이미 있습니다.
    DuplicateObject {
        /// 중복 id.
        id: StableId,
    },
    /// 속성 경로 root가 소유 객체가 아닙니다.
    PropertyRootMismatch {
        /// 소유 객체.
        object: StableId,
        /// 경로 root.
        root: StableId,
    },
    /// 같은 속성 경로가 두 번 정의되었습니다.
    DuplicateProperty {
        /// 경로.
        path: PropertyPath,
    },
    /// 유한하지 않은 float(저장된 레코드에서는 발생하지 않아야 함).
    NonFiniteValue {
        /// 소유 객체.
        object: StableId,
        /// 속성 key.
        key: PropertyKey,
    },
    /// 컴포넌트 정의 node payload를 해석할 수 없습니다.
    DefinitionPayload {
        /// 정의 node.
        node: StableId,
        /// 종류.
        kind: NodeKind,
        /// 원인.
        reason: String,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::DuplicateObject { id } => write!(f, "중복 객체 StableId: {id}"),
            SnapshotError::PropertyRootMismatch { object, root } => {
                write!(f, "속성 경로 root {root}가 소유 객체 {object}와 다릅니다")
            }
            SnapshotError::DuplicateProperty { path } => {
                write!(f, "중복 속성 경로: {path:?}")
            }
            SnapshotError::NonFiniteValue { object, key } => {
                write!(f, "유한하지 않은 값: 객체 {object}, key 0x{:04X}", key.0)
            }
            SnapshotError::DefinitionPayload { node, kind, reason } => write!(
                f,
                "컴포넌트 정의 node {node}({}) payload 해석 실패: {reason}",
                kind.name()
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// snapshot 객체 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticObject {
    id: StableId,
    kind: ObjectKind,
    origin: Origin,
    parent: Option<StableId>,
    order_key: Option<OrderKey>,
    properties: BTreeMap<PropertyPath, SemanticValue>,
}

impl SemanticObject {
    /// 위치·속성이 없는 객체를 만듭니다.
    pub fn new(id: StableId, kind: ObjectKind, origin: Origin) -> SemanticObject {
        SemanticObject {
            id,
            kind,
            origin,
            parent: None,
            order_key: None,
            properties: BTreeMap::new(),
        }
    }

    /// 위치를 지정합니다(빌더).
    pub fn with_position(
        mut self,
        parent: Option<StableId>,
        order_key: Option<OrderKey>,
    ) -> SemanticObject {
        self.parent = parent;
        self.order_key = order_key;
        self
    }

    /// 안정 식별자.
    pub fn id(&self) -> StableId {
        self.id
    }

    /// 종류.
    pub fn kind(&self) -> ObjectKind {
        self.kind
    }

    /// 범위.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// 소유 문서(프로젝트 범위면 `None`).
    pub fn document(&self) -> Option<StableId> {
        self.origin.document()
    }

    /// 부모 객체.
    pub fn parent(&self) -> Option<StableId> {
        self.parent
    }

    /// 형제 순서 키.
    pub fn order_key(&self) -> Option<&OrderKey> {
        self.order_key.as_ref()
    }

    /// 위치를 바꿉니다.
    pub fn set_position(&mut self, parent: Option<StableId>, order_key: Option<OrderKey>) {
        self.parent = parent;
        self.order_key = order_key;
    }

    /// 속성 map(경로 오름차순).
    pub fn properties(&self) -> &BTreeMap<PropertyPath, SemanticValue> {
        &self.properties
    }

    /// 속성 값.
    pub fn property(&self, path: &PropertyPath) -> Option<&SemanticValue> {
        self.properties.get(path)
    }

    /// 이 객체를 root로 하는 key 경로.
    pub fn path(&self, keys: &[PropertyKey]) -> PropertyPath {
        PropertyPath::with_keys(self.id, keys.to_vec())
    }

    /// 속성을 설정하고 이전 값을 반환합니다. 경로 root가 이 객체가 아니면 거부합니다.
    pub fn set_property(
        &mut self,
        path: PropertyPath,
        value: SemanticValue,
    ) -> Result<Option<SemanticValue>, SnapshotError> {
        self.check_root(&path)?;
        Ok(self.properties.insert(path, value))
    }

    /// 새 속성을 추가합니다. 이미 있으면 거부합니다.
    pub fn insert_property(
        &mut self,
        path: PropertyPath,
        value: SemanticValue,
    ) -> Result<(), SnapshotError> {
        self.check_root(&path)?;
        if self.properties.contains_key(&path) {
            return Err(SnapshotError::DuplicateProperty { path });
        }
        self.properties.insert(path, value);
        Ok(())
    }

    /// 속성을 지우고 이전 값을 반환합니다.
    pub fn remove_property(&mut self, path: &PropertyPath) -> Option<SemanticValue> {
        self.properties.remove(path)
    }

    /// id·종류·범위가 같은지(속성 단위 비교가 가능한 같은 객체인지).
    pub fn same_identity(&self, other: &SemanticObject) -> bool {
        self.id == other.id && self.kind == other.kind && self.origin == other.origin
    }

    fn check_root(&self, path: &PropertyPath) -> Result<(), SnapshotError> {
        if path.root() == self.id {
            Ok(())
        } else {
            Err(SnapshotError::PropertyRootMismatch {
                object: self.id,
                root: path.root(),
            })
        }
    }
}

/// 프로젝트 의미 상태의 StableId index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticSnapshot {
    objects: BTreeMap<StableId, SemanticObject>,
}

impl SemanticSnapshot {
    /// 빈 snapshot.
    pub fn new() -> SemanticSnapshot {
        SemanticSnapshot::default()
    }

    /// 객체를 추가합니다. 같은 StableId가 있으면 거부합니다.
    pub fn insert(&mut self, object: SemanticObject) -> Result<(), SnapshotError> {
        if self.objects.contains_key(&object.id) {
            return Err(SnapshotError::DuplicateObject { id: object.id });
        }
        self.objects.insert(object.id, object);
        Ok(())
    }

    /// 객체를 지우고 반환합니다.
    pub fn remove(&mut self, id: StableId) -> Option<SemanticObject> {
        self.objects.remove(&id)
    }

    /// 객체.
    pub fn get(&self, id: StableId) -> Option<&SemanticObject> {
        self.objects.get(&id)
    }

    /// 객체(가변). 변경 적용(XIV.3.9) 내부에서만 씁니다.
    pub(crate) fn get_mut(&mut self, id: StableId) -> Option<&mut SemanticObject> {
        self.objects.get_mut(&id)
    }

    /// 객체가 있는지.
    pub fn contains(&self, id: StableId) -> bool {
        self.objects.contains_key(&id)
    }

    /// 객체 수.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// StableId 오름차순 객체 순회.
    pub fn objects(&self) -> impl Iterator<Item = &SemanticObject> + '_ {
        self.objects.values()
    }

    /// 부모의 자식 id를 `(OrderKey, StableId)` 오름차순으로 반환합니다.
    pub fn children(&self, parent: StableId) -> Vec<StableId> {
        let mut children: Vec<&SemanticObject> = self
            .objects
            .values()
            .filter(|o| o.parent == Some(parent))
            .collect();
        children.sort_by(|a, b| (&a.order_key, a.id).cmp(&(&b.order_key, b.id)));
        children.into_iter().map(|o| o.id).collect()
    }

    /// asset 참조 index: asset StableId → 그 asset을 참조하는 속성 경로(오름차순).
    pub fn asset_references(&self) -> BTreeMap<StableId, BTreeSet<PropertyPath>> {
        let mut index: BTreeMap<StableId, BTreeSet<PropertyPath>> = BTreeMap::new();
        for object in self.objects.values() {
            for (path, value) in &object.properties {
                let is_reference = path
                    .keys()
                    .first()
                    .copied()
                    .is_some_and(keys::is_asset_reference);
                if let (true, SemanticValue::Id(asset)) = (is_reference, value) {
                    index.entry(*asset).or_default().insert(path.clone());
                }
            }
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn object_kind_codes_round_trip() {
        let mut kinds = vec![
            ObjectKind::Project,
            ObjectKind::Document,
            ObjectKind::Page,
            ObjectKind::View,
            ObjectKind::Component,
            ObjectKind::ComponentSlot,
            ObjectKind::Token,
            ObjectKind::Asset,
            ObjectKind::ExpressionNode,
            ObjectKind::Variant,
        ];
        kinds.extend(NodeKind::ALL.into_iter().map(ObjectKind::Node));
        kinds.extend(ModifierKind::ALL.into_iter().map(ObjectKind::Modifier));
        let mut codes = BTreeSet::new();
        for kind in kinds {
            assert!(codes.insert(kind.code()), "중복 code {kind:?}");
            assert_eq!(ObjectKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(ObjectKind::from_code(0x0000), None);
        assert_eq!(ObjectKind::from_code(0x0100), None);
        assert_eq!(ObjectKind::from_code(0x01FF), None);
        assert_eq!(ObjectKind::from_code(0xFFFF), None);
    }

    #[test]
    fn property_root_must_be_owner() {
        let mut object = SemanticObject::new(sid(1), ObjectKind::Token, Origin::Project);
        let foreign = PropertyPath::with_keys(sid(2), vec![keys::TOKEN_NAME]);
        assert_eq!(
            object.set_property(foreign, SemanticValue::Text("x".into())),
            Err(SnapshotError::PropertyRootMismatch {
                object: sid(1),
                root: sid(2)
            })
        );
        let own = object.path(&[keys::TOKEN_NAME]);
        object
            .insert_property(own.clone(), SemanticValue::Text("x".into()))
            .unwrap();
        assert_eq!(
            object.insert_property(own.clone(), SemanticValue::Text("y".into())),
            Err(SnapshotError::DuplicateProperty { path: own })
        );
    }

    #[test]
    fn duplicate_object_is_rejected_and_children_are_ordered() {
        let mut snapshot = SemanticSnapshot::new();
        let parent = SemanticObject::new(
            sid(1),
            ObjectKind::Node(NodeKind::Frame),
            Origin::Document(sid(9)),
        );
        snapshot.insert(parent.clone()).unwrap();
        assert_eq!(
            snapshot.insert(parent),
            Err(SnapshotError::DuplicateObject { id: sid(1) })
        );
        let key = |b: u8| Some(OrderKey::from_bytes(vec![b]).unwrap());
        for (n, k) in [(4, 0x80), (2, 0x40), (3, 0x80)] {
            snapshot
                .insert(
                    SemanticObject::new(
                        sid(n),
                        ObjectKind::Node(NodeKind::Fragment),
                        Origin::Document(sid(9)),
                    )
                    .with_position(Some(sid(1)), key(k)),
                )
                .unwrap();
        }
        // (key, id) 순: 0x40(2), 0x80(3), 0x80(4).
        assert_eq!(snapshot.children(sid(1)), vec![sid(2), sid(3), sid(4)]);
    }
}
