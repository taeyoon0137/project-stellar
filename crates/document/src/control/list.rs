//! List 계약 (IX.6.5).
//!
//! `List = Frame + Collection + Navigation/Selection Contract`(기술 아키텍처 12.5).
//!
//! | field | id | 값 | 필수 |
//! |---|---|---|---|
//! | collection | 1 | node(계약 Frame 자신 또는 자손) — 자식이 item인 container | 예(없으면 진단) |
//! | itemIdentity | 2 | enum [`ITEM_IDENTITY_ENUM`]: NodeId(0)·DataKey(1) | 아니오(없으면 NodeId) |
//! | selectionMode | 3 | enum [`SELECTION_MODE_ENUM`]: None(0)·Single(1)·Multiple(2) | 아니오(없으면 None) |
//! | navigation | 4 | enum [`NAVIGATION_ENUM`]: Vertical(0)·Horizontal(1)·Grid(2) | 아니오(없으면 Vertical) |
//! | selection | 5 | String 상태 변수 — 선택된 item identity(Single) | 아니오 |
//!
//! item identity는 item을 선택·diff할 때 쓰는 안정 key입니다. NodeId 방식은 collection 자식 node의 StableId가
//! identity라 유일성이 구조로 보장됩니다. DataKey 방식은 runtime item 목록이 key를 주며
//! [`validate_item_identities`]가 중복을 거부합니다.
//!
//! 상호작용: focus 가능, 방향 이동을 위해 `Key`, 선택이 있으면 `SelectionChange`를 처리해야 합니다(진단).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_expressions::types::ExpressionType;

use super::{
    ContractField, ContractIssue, ContractKind, ContractProblem, ContractScope, ContractValue,
    ControlContract, FieldChecker, InteractionContract, SemanticContract,
};
use crate::component::event::EventKind;

/// List 계약 종류.
pub const LIST: ContractKind = ContractKind(0x0004);

/// item identity 방식 enum 타입 ID.
pub const ITEM_IDENTITY_ENUM: u32 = 0x000C_0001;
/// 선택 방식 enum 타입 ID.
pub const SELECTION_MODE_ENUM: u32 = 0x000C_0002;
/// 방향 이동 enum 타입 ID.
pub const NAVIGATION_ENUM: u32 = 0x000C_0003;

/// List field.
pub mod fields {
    use super::ContractField;

    /// item container node.
    pub const COLLECTION: ContractField = ContractField(1);
    /// item identity 방식.
    pub const ITEM_IDENTITY: ContractField = ContractField(2);
    /// 선택 방식.
    pub const SELECTION_MODE: ContractField = ContractField(3);
    /// 방향 이동.
    pub const NAVIGATION: ContractField = ContractField(4);
    /// 선택 상태(String 상태 변수).
    pub const SELECTION: ContractField = ContractField(5);

    /// 알려진 field 전체.
    pub const ALL: [ContractField; 5] = [
        COLLECTION,
        ITEM_IDENTITY,
        SELECTION_MODE,
        NAVIGATION,
        SELECTION,
    ];
}

/// item identity 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemIdentity {
    /// collection 자식 node StableId.
    NodeId,
    /// runtime item 목록이 주는 data key.
    DataKey,
}

/// 선택 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionMode {
    /// 선택 없음.
    None,
    /// 한 번에 하나.
    Single,
    /// 여러 개.
    Multiple,
}

/// 방향 이동.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Navigation {
    /// 세로.
    Vertical,
    /// 가로.
    Horizontal,
    /// 격자.
    Grid,
}

impl ItemIdentity {
    /// 계약 상수 값.
    pub fn value(self) -> ContractValue {
        let variant = match self {
            ItemIdentity::NodeId => 0,
            ItemIdentity::DataKey => 1,
        };
        ContractValue::Constant(TypedValue::Enum(EnumValue::new(
            ITEM_IDENTITY_ENUM,
            variant,
        )))
    }
}

impl SelectionMode {
    /// 계약 상수 값.
    pub fn value(self) -> ContractValue {
        let variant = match self {
            SelectionMode::None => 0,
            SelectionMode::Single => 1,
            SelectionMode::Multiple => 2,
        };
        ContractValue::Constant(TypedValue::Enum(EnumValue::new(
            SELECTION_MODE_ENUM,
            variant,
        )))
    }

    fn from_variant(variant: u32) -> SelectionMode {
        match variant {
            1 => SelectionMode::Single,
            2 => SelectionMode::Multiple,
            _ => SelectionMode::None,
        }
    }
}

impl Navigation {
    /// 계약 상수 값.
    pub fn value(self) -> ContractValue {
        let variant = match self {
            Navigation::Vertical => 0,
            Navigation::Horizontal => 1,
            Navigation::Grid => 2,
        };
        ContractValue::Constant(TypedValue::Enum(EnumValue::new(NAVIGATION_ENUM, variant)))
    }
}

/// 계약의 선택 방식(없거나 잘못된 값이면 None).
pub fn selection_mode(contract: &ControlContract) -> SelectionMode {
    match contract.semantic.field(fields::SELECTION_MODE) {
        Some(ContractValue::Constant(TypedValue::Enum(e)))
            if e.enum_type() == SELECTION_MODE_ENUM =>
        {
            SelectionMode::from_variant(e.variant())
        }
        _ => SelectionMode::None,
    }
}

/// 기본 List 계약: collection node, 선택 방식, 세로 이동, focus 가능, Key·SelectionChange 처리.
pub fn list_contract(collection: StableId, mode: SelectionMode) -> ControlContract {
    ControlContract::new(
        SemanticContract::new(LIST)
            .with(fields::COLLECTION, ContractValue::Node(collection))
            .with(fields::SELECTION_MODE, mode.value())
            .with(fields::NAVIGATION, Navigation::Vertical.value()),
        InteractionContract::new(
            true,
            [EventKind::Focus, EventKind::Key, EventKind::SelectionChange],
        ),
    )
}

/// List 계약을 검사합니다.
pub fn validate_list(
    frame: StableId,
    contract: &ControlContract,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut checker = FieldChecker::new(frame, contract, scope);
    if contract.kind() != LIST {
        checker.push(None, ContractProblem::UnknownKind);
        return checker.finish();
    }
    checker.node(fields::COLLECTION, true, true);
    checker.enum_constant(fields::ITEM_IDENTITY, false, ITEM_IDENTITY_ENUM, 2);
    let mode = checker
        .enum_constant(fields::SELECTION_MODE, false, SELECTION_MODE_ENUM, 3)
        .map(SelectionMode::from_variant)
        .unwrap_or(SelectionMode::None);
    checker.enum_constant(fields::NAVIGATION, false, NAVIGATION_ENUM, 3);
    checker.value(fields::SELECTION, false, ExpressionType::String, true);
    checker.requires_focus();
    checker.requires_event(EventKind::Key);
    if mode != SelectionMode::None {
        checker.requires_event(EventKind::SelectionChange);
    }
    checker.unknown_fields(&fields::ALL);
    checker.finish()
}

/// runtime item 목록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListItemError {
    /// 같은 identity의 item이 둘 이상입니다.
    DuplicateItemIdentity {
        /// List Frame.
        frame: StableId,
        /// 중복 identity.
        identity: String,
        /// 처음 위치.
        first: usize,
        /// 중복 위치.
        duplicate: usize,
    },
}

impl fmt::Display for ListItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListItemError::DuplicateItemIdentity {
                frame,
                identity,
                first,
                duplicate,
            } => write!(
                f,
                "List {frame}의 item {first}·{duplicate}가 같은 identity({identity})를 가집니다"
            ),
        }
    }
}

impl std::error::Error for ListItemError {}

/// runtime item identity 목록이 유일한지 검사합니다(첫 중복을 보고).
pub fn validate_item_identities(
    frame: StableId,
    identities: &[String],
) -> Result<(), ListItemError> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, identity) in identities.iter().enumerate() {
        if let Some(first) = seen.insert(identity.as_str(), index) {
            return Err(ListItemError::DuplicateItemIdentity {
                frame,
                identity: identity.clone(),
                first,
                duplicate: index,
            });
        }
    }
    Ok(())
}

/// NodeId 방식의 item identity 목록: collection 자식 node StableId 문자열(구조상 유일).
pub fn node_identities(scope: &dyn ContractScope, collection: StableId) -> Vec<String> {
    scope
        .children(collection)
        .into_iter()
        .map(|id| id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::fixtures::library;

    fn check(contract: &ControlContract) -> Vec<ContractIssue> {
        let library = library();
        validate_list(stable(ROOT), contract, library.get(toggle()).unwrap())
    }

    #[test]
    fn duplicate_item_identity_fixture_is_an_error() {
        // 검증 기준(IX.6.5): duplicate item identity fixture 오류.
        let items: Vec<String> = ["사과", "배", "사과"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            validate_item_identities(stable(ROOT), &items).unwrap_err(),
            ListItemError::DuplicateItemIdentity {
                frame: stable(ROOT),
                identity: "사과".into(),
                first: 0,
                duplicate: 2,
            }
        );
        validate_item_identities(stable(ROOT), &items[..2]).unwrap();
        // NodeId 방식은 collection 자식 StableId라 항상 유일합니다.
        let library = library();
        let entry = library.get(toggle()).unwrap();
        let ids = node_identities(entry, stable(ROOT));
        assert_eq!(ids.len(), 2);
        validate_item_identities(stable(ROOT), &ids).unwrap();
    }

    #[test]
    fn complete_list_has_no_issues() {
        let mut contract = list_contract(stable(ROOT), SelectionMode::Single);
        contract
            .semantic
            .set(fields::ITEM_IDENTITY, ItemIdentity::DataKey.value());
        contract
            .semantic
            .set(fields::SELECTION, ContractValue::State(var(TEXT)));
        assert_eq!(check(&contract), vec![]);
        assert_eq!(selection_mode(&contract), SelectionMode::Single);
    }

    #[test]
    fn invalid_list_fields_are_reported() {
        let mut contract = list_contract(stable(ROOT), SelectionMode::Multiple);
        contract.semantic.remove(fields::COLLECTION);
        // 다른 enum 타입 값, 범위 밖 variant, 쓸 수 없는 selection.
        contract
            .semantic
            .set(fields::NAVIGATION, SelectionMode::Single.value());
        contract.semantic.set(
            fields::ITEM_IDENTITY,
            ContractValue::Constant(TypedValue::Enum(EnumValue::new(ITEM_IDENTITY_ENUM, 9))),
        );
        contract
            .semantic
            .set(fields::SELECTION, ContractValue::Property(prop(CAPTION)));
        contract.interaction = InteractionContract::new(true, [EventKind::Key]);
        let problems: Vec<_> = check(&contract)
            .into_iter()
            .map(|i| (i.field, i.problem))
            .collect();
        let enum_mismatch = |enum_type: u32, found: u32| ContractProblem::TypeMismatch {
            expected: ExpressionType::Enum(stellar_expressions::types::EnumTypeId::new(enum_type)),
            found: Some(stellar_expressions::types::TypeSpec::required(
                ExpressionType::Enum(stellar_expressions::types::EnumTypeId::new(found)),
            )),
        };
        assert_eq!(
            problems,
            vec![
                (Some(fields::COLLECTION), ContractProblem::MissingRequired),
                (
                    Some(fields::ITEM_IDENTITY),
                    enum_mismatch(ITEM_IDENTITY_ENUM, ITEM_IDENTITY_ENUM)
                ),
                (
                    Some(fields::NAVIGATION),
                    enum_mismatch(NAVIGATION_ENUM, SELECTION_MODE_ENUM)
                ),
                (Some(fields::SELECTION), ContractProblem::NotWritable),
                (
                    None,
                    ContractProblem::MissingInteraction {
                        event: EventKind::SelectionChange
                    }
                ),
            ]
        );
    }
}
