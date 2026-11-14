//! `SemanticChange` 모델 (XIV.3.2).
//!
//! 두 [`SemanticSnapshot`](crate::snapshot::SemanticSnapshot)의 의미 차이 한 단위입니다.
//! 변경 종류([`SemanticChangeKind`])는 안정 numeric code를 가집니다.
//!
//! | code | 종류 | 대상 |
//! |---|---|---|
//! | 1 | AddObject | token 외 객체 추가(객체 전체 포함) |
//! | 2 | RemoveObject | token 외 객체 삭제(삭제 전 객체 전체 포함) |
//! | 3 | MoveObject | parent가 바뀐 위치 변경 |
//! | 4 | ReorderObject | 같은 parent 안 OrderKey 변경 |
//! | 5 | SetProperty | 일반 속성 변경 |
//! | 6 | AddToken | token 객체 추가 |
//! | 7 | RemoveToken | token 객체 삭제 |
//! | 8 | ChangeToken | token 속성 변경 |
//! | 9 | ComponentInterfaceChange | 컴포넌트 Property·Slot 목록, Slot record 변경 |
//! | 10 | AssetRefChange | asset 참조 속성 변경 |
//!
//! 모든 변경은 이전·이후 상태를 함께 담습니다(추가·삭제는 객체 전체, 위치는 from/to, 속성은
//! old/new). 그래서 적용(XIV.3.9) 전 base 상태를 검증할 수 있고 역변경(XIV.3.10)은 정보
//! 손실 없이 만들 수 있습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyPath;

use crate::keys;
use crate::snapshot::{ObjectKind, SemanticObject};
use crate::value::SemanticValue;

/// 변경 종류(안정 numeric code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SemanticChangeKind {
    /// 객체 추가.
    AddObject = 1,
    /// 객체 삭제.
    RemoveObject = 2,
    /// 다른 parent로 이동.
    MoveObject = 3,
    /// 같은 parent 안 재정렬.
    ReorderObject = 4,
    /// 속성 변경.
    SetProperty = 5,
    /// token 추가.
    AddToken = 6,
    /// token 삭제.
    RemoveToken = 7,
    /// token 속성 변경.
    ChangeToken = 8,
    /// 컴포넌트 interface 변경.
    ComponentInterfaceChange = 9,
    /// asset 참조 변경.
    AssetRefChange = 10,
}

impl SemanticChangeKind {
    /// 정의된 모든 종류(code 오름차순).
    pub const ALL: [SemanticChangeKind; 10] = [
        SemanticChangeKind::AddObject,
        SemanticChangeKind::RemoveObject,
        SemanticChangeKind::MoveObject,
        SemanticChangeKind::ReorderObject,
        SemanticChangeKind::SetProperty,
        SemanticChangeKind::AddToken,
        SemanticChangeKind::RemoveToken,
        SemanticChangeKind::ChangeToken,
        SemanticChangeKind::ComponentInterfaceChange,
        SemanticChangeKind::AssetRefChange,
    ];

    /// 안정 numeric code.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// code에서 복원합니다.
    pub fn from_code(code: u8) -> Option<SemanticChangeKind> {
        SemanticChangeKind::ALL
            .into_iter()
            .find(|k| k.code() == code)
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            SemanticChangeKind::AddObject => "add-object",
            SemanticChangeKind::RemoveObject => "remove-object",
            SemanticChangeKind::MoveObject => "move-object",
            SemanticChangeKind::ReorderObject => "reorder-object",
            SemanticChangeKind::SetProperty => "set-property",
            SemanticChangeKind::AddToken => "add-token",
            SemanticChangeKind::RemoveToken => "remove-token",
            SemanticChangeKind::ChangeToken => "change-token",
            SemanticChangeKind::ComponentInterfaceChange => "component-interface-change",
            SemanticChangeKind::AssetRefChange => "asset-ref-change",
        }
    }

    /// 객체 단위(추가·삭제·위치) 변경인지. 아니면 속성 단위입니다.
    pub fn is_structural(self) -> bool {
        matches!(
            self,
            SemanticChangeKind::AddObject
                | SemanticChangeKind::RemoveObject
                | SemanticChangeKind::MoveObject
                | SemanticChangeKind::ReorderObject
                | SemanticChangeKind::AddToken
                | SemanticChangeKind::RemoveToken
        )
    }

    /// 객체 종류에 맞는 추가 변경 종류.
    pub fn add_for(kind: ObjectKind) -> SemanticChangeKind {
        if kind == ObjectKind::Token {
            SemanticChangeKind::AddToken
        } else {
            SemanticChangeKind::AddObject
        }
    }

    /// 객체 종류에 맞는 삭제 변경 종류.
    pub fn remove_for(kind: ObjectKind) -> SemanticChangeKind {
        if kind == ObjectKind::Token {
            SemanticChangeKind::RemoveToken
        } else {
            SemanticChangeKind::RemoveObject
        }
    }

    /// 객체 종류와 속성 경로에 맞는 속성 변경 종류.
    ///
    /// token 객체 → ChangeToken, 컴포넌트 Property·Slot 목록과 Slot record →
    /// ComponentInterfaceChange, asset 참조 key → AssetRefChange, 나머지 → SetProperty.
    pub fn property_for(kind: ObjectKind, path: &PropertyPath) -> SemanticChangeKind {
        let first = path.keys().first().copied();
        let interface = match kind {
            ObjectKind::ComponentSlot => true,
            ObjectKind::Component => {
                first == Some(keys::COMPONENT_PROPERTIES) || first == Some(keys::COMPONENT_SLOTS)
            }
            _ => false,
        };
        if kind == ObjectKind::Token {
            SemanticChangeKind::ChangeToken
        } else if interface {
            SemanticChangeKind::ComponentInterfaceChange
        } else if first.is_some_and(keys::is_asset_reference) {
            SemanticChangeKind::AssetRefChange
        } else {
            SemanticChangeKind::SetProperty
        }
    }
}

/// 변경이 어디서 왔는지(XIV.3.6). 같은 node 변경이라도 컴포넌트 정의 변경과 인스턴스
/// override 변경을 구분합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provenance {
    /// 프로젝트 범위(metadata, 프로젝트 token, asset).
    Project,
    /// 문서 scene·정의.
    Document {
        /// 문서.
        document: StableId,
    },
    /// 컴포넌트 정의(정의 resource·정의 tree·Slot).
    ComponentDefinition {
        /// 문서.
        document: StableId,
        /// 컴포넌트 정의.
        component: StableId,
    },
    /// 인스턴스 override(Property override·Slot override).
    InstanceOverride {
        /// 문서.
        document: StableId,
        /// 인스턴스 node.
        instance: StableId,
        /// 인스턴스가 참조하는 컴포넌트 정의.
        component: StableId,
    },
    /// 문서에 속하지 않는 프로젝트 공용 컴포넌트 정의.
    ProjectComponentDefinition(StableId),
}

impl Provenance {
    /// 안정 numeric code.
    pub fn code(&self) -> u8 {
        match self {
            Provenance::Project => 1,
            Provenance::Document { .. } => 2,
            Provenance::ComponentDefinition { .. } => 3,
            Provenance::InstanceOverride { .. } => 4,
            Provenance::ProjectComponentDefinition(_) => 5,
        }
    }

    /// 변경이 정의 또는 그 tree에 속하면 소유 컴포넌트.
    pub fn definition_component(&self) -> Option<StableId> {
        match self {
            Self::ProjectComponentDefinition(component)
            | Self::ComponentDefinition { component, .. } => Some(*component),
            _ => None,
        }
    }

    /// 소유 문서(프로젝트 범위면 `None`).
    pub fn document(&self) -> Option<StableId> {
        match self {
            Provenance::Project | Provenance::ProjectComponentDefinition(_) => None,
            Provenance::Document { document }
            | Provenance::ComponentDefinition { document, .. }
            | Provenance::InstanceOverride { document, .. } => Some(*document),
        }
    }
}

/// 객체 위치: parent와 형제 순서 키.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// 부모 객체.
    pub parent: Option<StableId>,
    /// 형제 순서 키.
    pub order_key: Option<OrderKey>,
}

impl Position {
    /// 객체의 현재 위치.
    pub fn of(object: &SemanticObject) -> Position {
        Position {
            parent: object.parent(),
            order_key: object.order_key().cloned(),
        }
    }
}

/// 객체 추가·삭제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectChange {
    /// 출처.
    pub provenance: Provenance,
    /// 추가된 객체 또는 삭제 전 객체 전체.
    pub object: SemanticObject,
}

/// 위치 변경(Move·Reorder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionChange {
    /// 출처.
    pub provenance: Provenance,
    /// 대상 객체.
    pub object: StableId,
    /// 이전 위치.
    pub from: Position,
    /// 이후 위치.
    pub to: Position,
}

/// 속성 변경. `None`은 속성이 없음을 뜻합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyChange {
    /// 출처.
    pub provenance: Provenance,
    /// 속성 경로(root = 대상 객체).
    pub path: PropertyPath,
    /// 이전 값.
    pub old: Option<SemanticValue>,
    /// 이후 값.
    pub new: Option<SemanticValue>,
}

/// 의미 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticChange {
    /// 객체 추가.
    AddObject(ObjectChange),
    /// 객체 삭제.
    RemoveObject(ObjectChange),
    /// 다른 parent로 이동.
    MoveObject(PositionChange),
    /// 같은 parent 안 재정렬.
    ReorderObject(PositionChange),
    /// 속성 변경.
    SetProperty(PropertyChange),
    /// token 추가.
    AddToken(ObjectChange),
    /// token 삭제.
    RemoveToken(ObjectChange),
    /// token 속성 변경.
    ChangeToken(PropertyChange),
    /// 컴포넌트 interface 변경.
    ComponentInterfaceChange(PropertyChange),
    /// asset 참조 변경.
    AssetRefChange(PropertyChange),
}

impl SemanticChange {
    /// 객체 종류에 맞는 추가 변경(token이면 AddToken).
    pub fn added(provenance: Provenance, object: SemanticObject) -> SemanticChange {
        let change = ObjectChange { provenance, object };
        match SemanticChangeKind::add_for(change.object.kind()) {
            SemanticChangeKind::AddToken => SemanticChange::AddToken(change),
            _ => SemanticChange::AddObject(change),
        }
    }

    /// 객체 종류에 맞는 삭제 변경(token이면 RemoveToken).
    pub fn removed(provenance: Provenance, object: SemanticObject) -> SemanticChange {
        let change = ObjectChange { provenance, object };
        match SemanticChangeKind::remove_for(change.object.kind()) {
            SemanticChangeKind::RemoveToken => SemanticChange::RemoveToken(change),
            _ => SemanticChange::RemoveObject(change),
        }
    }

    /// 객체 종류·경로에 맞는 속성 변경([`SemanticChangeKind::property_for`]).
    pub fn property(kind: ObjectKind, change: PropertyChange) -> SemanticChange {
        match SemanticChangeKind::property_for(kind, &change.path) {
            SemanticChangeKind::ChangeToken => SemanticChange::ChangeToken(change),
            SemanticChangeKind::ComponentInterfaceChange => {
                SemanticChange::ComponentInterfaceChange(change)
            }
            SemanticChangeKind::AssetRefChange => SemanticChange::AssetRefChange(change),
            _ => SemanticChange::SetProperty(change),
        }
    }

    /// 종류.
    pub fn kind(&self) -> SemanticChangeKind {
        match self {
            SemanticChange::AddObject(_) => SemanticChangeKind::AddObject,
            SemanticChange::RemoveObject(_) => SemanticChangeKind::RemoveObject,
            SemanticChange::MoveObject(_) => SemanticChangeKind::MoveObject,
            SemanticChange::ReorderObject(_) => SemanticChangeKind::ReorderObject,
            SemanticChange::SetProperty(_) => SemanticChangeKind::SetProperty,
            SemanticChange::AddToken(_) => SemanticChangeKind::AddToken,
            SemanticChange::RemoveToken(_) => SemanticChangeKind::RemoveToken,
            SemanticChange::ChangeToken(_) => SemanticChangeKind::ChangeToken,
            SemanticChange::ComponentInterfaceChange(_) => {
                SemanticChangeKind::ComponentInterfaceChange
            }
            SemanticChange::AssetRefChange(_) => SemanticChangeKind::AssetRefChange,
        }
    }

    /// 출처.
    pub fn provenance(&self) -> &Provenance {
        match self {
            SemanticChange::AddObject(c)
            | SemanticChange::RemoveObject(c)
            | SemanticChange::AddToken(c)
            | SemanticChange::RemoveToken(c) => &c.provenance,
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => &c.provenance,
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => &c.provenance,
        }
    }

    /// 소유 문서(프로젝트 범위면 `None`).
    pub fn document(&self) -> Option<StableId> {
        self.provenance().document()
    }

    /// 대상 객체 StableId.
    pub fn object_id(&self) -> StableId {
        match self {
            SemanticChange::AddObject(c)
            | SemanticChange::RemoveObject(c)
            | SemanticChange::AddToken(c)
            | SemanticChange::RemoveToken(c) => c.object.id(),
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => c.object,
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => c.path.root(),
        }
    }

    /// 속성 변경이면 속성 경로.
    pub fn property_path(&self) -> Option<&PropertyPath> {
        match self {
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => Some(&c.path),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{NodeKind, Origin};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn kind_codes_and_names_golden() {
        let golden: [(u8, &str); 10] = [
            (1, "add-object"),
            (2, "remove-object"),
            (3, "move-object"),
            (4, "reorder-object"),
            (5, "set-property"),
            (6, "add-token"),
            (7, "remove-token"),
            (8, "change-token"),
            (9, "component-interface-change"),
            (10, "asset-ref-change"),
        ];
        assert_eq!(SemanticChangeKind::ALL.len(), golden.len());
        for (kind, (code, name)) in SemanticChangeKind::ALL.into_iter().zip(golden) {
            assert_eq!(kind.code(), code);
            assert_eq!(kind.name(), name);
            assert_eq!(SemanticChangeKind::from_code(code), Some(kind));
        }
        assert_eq!(SemanticChangeKind::from_code(0), None);
        assert_eq!(SemanticChangeKind::from_code(11), None);
    }

    #[test]
    fn provenance_codes_golden() {
        let document = sid(1);
        let golden = [
            (Provenance::Project, 1, None),
            (Provenance::ProjectComponentDefinition(sid(2)), 5, None),
            (Provenance::Document { document }, 2, Some(document)),
            (
                Provenance::ComponentDefinition {
                    document,
                    component: sid(2),
                },
                3,
                Some(document),
            ),
            (
                Provenance::InstanceOverride {
                    document,
                    instance: sid(3),
                    component: sid(2),
                },
                4,
                Some(document),
            ),
        ];
        for (provenance, code, doc) in golden {
            assert_eq!(provenance.code(), code);
            assert_eq!(provenance.document(), doc);
        }
    }

    #[test]
    fn classification_table() {
        let path = |key| PropertyPath::with_keys(sid(9), vec![key]);
        let node = ObjectKind::Node(NodeKind::Image);
        let cases = [
            (
                ObjectKind::Token,
                path(keys::TOKEN_VALUE),
                SemanticChangeKind::ChangeToken,
            ),
            (
                ObjectKind::Token,
                path(keys::TOKEN_NAME),
                SemanticChangeKind::ChangeToken,
            ),
            (
                ObjectKind::Component,
                path(keys::COMPONENT_PROPERTIES),
                SemanticChangeKind::ComponentInterfaceChange,
            ),
            (
                ObjectKind::Component,
                path(keys::COMPONENT_SLOTS),
                SemanticChangeKind::ComponentInterfaceChange,
            ),
            (
                ObjectKind::Component,
                path(keys::DISPLAY_NAME),
                SemanticChangeKind::SetProperty,
            ),
            (
                ObjectKind::ComponentSlot,
                path(keys::SLOT_MAX),
                SemanticChangeKind::ComponentInterfaceChange,
            ),
            (
                node,
                path(keys::IMAGE_ASSET),
                SemanticChangeKind::AssetRefChange,
            ),
            (
                node,
                path(keys::IMAGE_SIZE),
                SemanticChangeKind::SetProperty,
            ),
        ];
        for (kind, path, expected) in cases {
            assert_eq!(
                SemanticChangeKind::property_for(kind, &path),
                expected,
                "{kind:?} {path:?}"
            );
            let change = SemanticChange::property(
                kind,
                PropertyChange {
                    provenance: Provenance::Project,
                    path: path.clone(),
                    old: None,
                    new: Some(SemanticValue::Bool(true)),
                },
            );
            assert_eq!(change.kind(), expected);
            assert_eq!(change.property_path(), Some(&path));
            assert_eq!(change.object_id(), sid(9));
            assert!(!expected.is_structural());
        }
    }

    #[test]
    fn token_objects_use_token_kinds() {
        let token = SemanticObject::new(sid(5), ObjectKind::Token, Origin::Project);
        let frame = SemanticObject::new(
            sid(6),
            ObjectKind::Node(NodeKind::Frame),
            Origin::Document(sid(1)),
        );
        let added = SemanticChange::added(Provenance::Project, token.clone());
        let removed = SemanticChange::removed(Provenance::Project, token);
        assert_eq!(added.kind(), SemanticChangeKind::AddToken);
        assert_eq!(removed.kind(), SemanticChangeKind::RemoveToken);
        let document = Provenance::Document { document: sid(1) };
        assert_eq!(
            SemanticChange::added(document, frame.clone()).kind(),
            SemanticChangeKind::AddObject
        );
        let removed = SemanticChange::removed(document, frame);
        assert_eq!(removed.kind(), SemanticChangeKind::RemoveObject);
        assert_eq!(removed.document(), Some(sid(1)));
        assert_eq!(removed.object_id(), sid(6));
        assert!(removed.kind().is_structural());
        assert_eq!(removed.property_path(), None);
    }
}
