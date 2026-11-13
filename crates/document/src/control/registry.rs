//! contract 유효성 registry와 target export hint (IX.6.7).
//!
//! 알려진 계약 종류마다 descriptor 하나를 둡니다: 이름, 의미 field 표(필수 여부), 검사기, target export hint.
//! hint는 target profile(Web·Apple·Android)에서 이 계약이 대응하는 플랫폼 구성 요소의 **이름**일 뿐이며 코드를
//! 생성하지 않습니다(기술 아키텍처 17.3 — 코드 생성은 로드맵).
//!
//! [`check_registry`]는 registry가 알려진 종류([`KNOWN_KINDS`])를 빠짐없이, 중복 없이 담고 field 표가 종류 모듈의
//! field 목록과 일치하는지 검사합니다. [`validate_contracts`]는 표의 모든 계약을 registry 검사기로 검사하고,
//! registry에 없는 종류는 보존하되 `UnknownKind` 진단을 냅니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;

use super::{
    button, list, modal, selection_group, text_input, ContractField, ContractIssue, ContractKind,
    ContractProblem, ContractScope, ControlContract, ControlContractTable,
};

/// 계약 검사기 서명.
pub type ContractValidator =
    fn(StableId, &ControlContract, &ControlContractTable, &dyn ContractScope) -> Vec<ContractIssue>;

/// 의미 field 설명.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// field.
    pub field: ContractField,
    /// 안정 이름.
    pub name: &'static str,
    /// 필수 여부(없으면 MissingRequired 진단).
    pub required: bool,
}

/// export 대상 target profile 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportTarget {
    /// Web(HTML·ARIA).
    Web,
    /// Apple(SwiftUI).
    Apple,
    /// Android(Jetpack Compose).
    Android,
}

impl ExportTarget {
    /// 모든 target.
    pub const ALL: [ExportTarget; 3] = [
        ExportTarget::Web,
        ExportTarget::Apple,
        ExportTarget::Android,
    ];
}

/// target별 대응 구성 요소 이름(코드 생성 없음).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportHints {
    /// Web.
    pub web: &'static str,
    /// Apple.
    pub apple: &'static str,
    /// Android.
    pub android: &'static str,
}

impl ExportHints {
    /// target의 hint.
    pub fn hint(&self, target: ExportTarget) -> &'static str {
        match target {
            ExportTarget::Web => self.web,
            ExportTarget::Apple => self.apple,
            ExportTarget::Android => self.android,
        }
    }
}

/// 계약 종류 descriptor.
#[derive(Clone, Copy)]
pub struct ContractDescriptor {
    /// 종류.
    pub kind: ContractKind,
    /// 안정 이름.
    pub name: &'static str,
    /// 의미 field 표(field id 오름차순).
    pub fields: &'static [FieldDescriptor],
    /// 검사기.
    pub validate: ContractValidator,
    /// target export hint.
    pub export: ExportHints,
}

impl fmt::Debug for ContractDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContractDescriptor")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("fields", &self.fields)
            .field("export", &self.export)
            .finish()
    }
}

/// 알려진 계약 종류(기술 아키텍처 12.5 초기 4종 + SingleSelectionGroup).
pub const KNOWN_KINDS: [ContractKind; 5] = [
    button::BUTTON,
    text_input::TEXT_INPUT,
    modal::MODAL,
    list::LIST,
    selection_group::SINGLE_SELECTION_GROUP,
];

const fn field(field: ContractField, name: &'static str, required: bool) -> FieldDescriptor {
    FieldDescriptor {
        field,
        name,
        required,
    }
}

const BUTTON_FIELDS: [FieldDescriptor; 4] = [
    field(button::fields::ACTIVATE, "activate", true),
    field(button::fields::FOCUSED, "focused", false),
    field(button::fields::DISABLED, "disabled", false),
    field(button::fields::PRESSED, "pressed", false),
];
const TEXT_INPUT_FIELDS: [FieldDescriptor; 4] = [
    field(text_input::fields::VALUE, "value", true),
    field(text_input::fields::PLACEHOLDER, "placeholder", false),
    field(text_input::fields::FOCUSED, "focused", false),
    field(text_input::fields::IME_INPUT, "imeInput", false),
];
const MODAL_FIELDS: [FieldDescriptor; 4] = [
    field(modal::fields::OVERLAY, "overlay", false),
    field(modal::fields::FOCUS_SCOPE_ROOT, "focusScopeRoot", true),
    field(modal::fields::DISMISS, "dismiss", false),
    field(modal::fields::OPEN, "open", false),
];
const LIST_FIELDS: [FieldDescriptor; 5] = [
    field(list::fields::COLLECTION, "collection", true),
    field(list::fields::ITEM_IDENTITY, "itemIdentity", false),
    field(list::fields::SELECTION_MODE, "selectionMode", false),
    field(list::fields::NAVIGATION, "navigation", false),
    field(list::fields::SELECTION, "selection", false),
];
const GROUP_FIELDS: [FieldDescriptor; 2] = [
    field(selection_group::fields::LIST, "list", true),
    field(selection_group::fields::SELECTED, "selected", false),
];

fn without_table(
    validate: fn(StableId, &ControlContract, &dyn ContractScope) -> Vec<ContractIssue>,
) -> impl Fn(StableId, &ControlContract, &ControlContractTable, &dyn ContractScope) -> Vec<ContractIssue>
{
    move |frame, contract, _table, scope| validate(frame, contract, scope)
}

fn validate_button(
    frame: StableId,
    contract: &ControlContract,
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    without_table(button::validate_button)(frame, contract, table, scope)
}

fn validate_text_input(
    frame: StableId,
    contract: &ControlContract,
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    without_table(text_input::validate_text_input)(frame, contract, table, scope)
}

fn validate_modal(
    frame: StableId,
    contract: &ControlContract,
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    without_table(modal::validate_modal)(frame, contract, table, scope)
}

fn validate_list(
    frame: StableId,
    contract: &ControlContract,
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    without_table(list::validate_list)(frame, contract, table, scope)
}

/// 계약 registry(종류 오름차순).
pub const REGISTRY: [ContractDescriptor; 5] = [
    ContractDescriptor {
        kind: button::BUTTON,
        name: "button",
        fields: &BUTTON_FIELDS,
        validate: validate_button,
        export: ExportHints {
            web: "button",
            apple: "Button",
            android: "Button",
        },
    },
    ContractDescriptor {
        kind: text_input::TEXT_INPUT,
        name: "textInput",
        fields: &TEXT_INPUT_FIELDS,
        validate: validate_text_input,
        export: ExportHints {
            web: "input[type=text]",
            apple: "TextField",
            android: "TextField",
        },
    },
    ContractDescriptor {
        kind: modal::MODAL,
        name: "modal",
        fields: &MODAL_FIELDS,
        validate: validate_modal,
        export: ExportHints {
            web: "dialog",
            apple: "sheet",
            android: "Dialog",
        },
    },
    ContractDescriptor {
        kind: list::LIST,
        name: "list",
        fields: &LIST_FIELDS,
        validate: validate_list,
        export: ExportHints {
            web: "ul[role=listbox]",
            apple: "List",
            android: "LazyColumn",
        },
    },
    ContractDescriptor {
        kind: selection_group::SINGLE_SELECTION_GROUP,
        name: "singleSelectionGroup",
        fields: &GROUP_FIELDS,
        validate: selection_group::validate_single_selection_group,
        export: ExportHints {
            web: "div[role=radiogroup]",
            apple: "Picker(.radioGroup)",
            android: "Modifier.selectableGroup",
        },
    },
];

/// 종류의 descriptor.
pub fn descriptor(kind: ContractKind) -> Option<&'static ContractDescriptor> {
    REGISTRY.iter().find(|d| d.kind == kind)
}

/// registry 검사 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// 알려진 종류의 descriptor가 없습니다.
    MissingKind {
        /// 종류.
        kind: ContractKind,
    },
    /// 같은 종류·이름의 descriptor가 둘 이상입니다.
    DuplicateKind {
        /// 종류.
        kind: ContractKind,
    },
    /// field 표가 종류 모듈의 field 목록과 다릅니다(누락·추가·순서).
    FieldTableMismatch {
        /// 종류.
        kind: ContractKind,
    },
    /// target hint가 비었습니다.
    EmptyExportHint {
        /// 종류.
        kind: ContractKind,
        /// target.
        target: ExportTarget,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::MissingKind { kind } => {
                write!(f, "계약 종류 0x{:04X}의 registry 항목이 없습니다", kind.0)
            }
            RegistryError::DuplicateKind { kind } => {
                write!(f, "계약 종류 0x{:04X} registry 항목이 중복됩니다", kind.0)
            }
            RegistryError::FieldTableMismatch { kind } => write!(
                f,
                "계약 종류 0x{:04X}의 field 표가 종류 정의와 다릅니다",
                kind.0
            ),
            RegistryError::EmptyExportHint { kind, target } => write!(
                f,
                "계약 종류 0x{:04X}의 {target:?} export hint가 비었습니다",
                kind.0
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// 종류 모듈이 정의한 field 목록.
fn module_fields(kind: ContractKind) -> Option<&'static [ContractField]> {
    match kind {
        k if k == button::BUTTON => Some(&button::fields::ALL),
        k if k == text_input::TEXT_INPUT => Some(&text_input::fields::ALL),
        k if k == modal::MODAL => Some(&modal::fields::ALL),
        k if k == list::LIST => Some(&list::fields::ALL),
        k if k == selection_group::SINGLE_SELECTION_GROUP => Some(&selection_group::fields::ALL),
        _ => None,
    }
}

/// `entries`가 [`KNOWN_KINDS`]를 빠짐없이·중복 없이 담고 field 표·hint가 올바른지 검사합니다.
pub fn check_registry_entries(entries: &[ContractDescriptor]) -> Result<(), RegistryError> {
    let mut kinds = BTreeSet::new();
    let mut names = BTreeSet::new();
    for entry in entries {
        if !kinds.insert(entry.kind) || !names.insert(entry.name) {
            return Err(RegistryError::DuplicateKind { kind: entry.kind });
        }
        let table: Vec<ContractField> = entry.fields.iter().map(|f| f.field).collect();
        if module_fields(entry.kind) != Some(table.as_slice()) {
            return Err(RegistryError::FieldTableMismatch { kind: entry.kind });
        }
        for target in ExportTarget::ALL {
            if entry.export.hint(target).is_empty() {
                return Err(RegistryError::EmptyExportHint {
                    kind: entry.kind,
                    target,
                });
            }
        }
    }
    if let Some(kind) = KNOWN_KINDS.iter().find(|k| !kinds.contains(k)) {
        return Err(RegistryError::MissingKind { kind: *kind });
    }
    Ok(())
}

/// 내장 [`REGISTRY`]를 검사합니다.
pub fn check_registry() -> Result<(), RegistryError> {
    check_registry_entries(&REGISTRY)
}

/// 표의 모든 계약을 registry 검사기로 검사합니다(frame 오름차순). registry에 없는 종류는 `UnknownKind` 진단.
pub fn validate_contracts(
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut issues = Vec::new();
    for (frame, contract) in table.iter() {
        match descriptor(contract.kind()) {
            Some(descriptor) => issues.extend((descriptor.validate)(frame, contract, table, scope)),
            None => issues.push(ContractIssue {
                frame,
                field: None,
                problem: ContractProblem::UnknownKind,
            }),
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::button::button_contract;
    use crate::control::fixtures::library;
    use crate::control::{InteractionContract, SemanticContract};

    #[test]
    fn registry_has_no_missing_entries() {
        // 검증 기준(IX.6.7): registry 누락 검사 통과.
        check_registry().unwrap();
        for kind in KNOWN_KINDS {
            let descriptor = descriptor(kind).unwrap();
            assert_eq!(
                descriptor.fields.iter().filter(|f| f.required).count(),
                1,
                "{} 필수 field 1개",
                descriptor.name
            );
        }
        // 항목 하나를 빼면 누락으로 실패합니다.
        assert_eq!(
            check_registry_entries(&REGISTRY[..4]).unwrap_err(),
            RegistryError::MissingKind {
                kind: selection_group::SINGLE_SELECTION_GROUP
            }
        );
        // 중복·field 표 불일치·빈 hint도 실패합니다.
        let mut duplicated = REGISTRY.to_vec();
        duplicated.push(REGISTRY[0]);
        assert!(matches!(
            check_registry_entries(&duplicated),
            Err(RegistryError::DuplicateKind { .. })
        ));
        let mut fields = REGISTRY;
        fields[0].fields = &BUTTON_FIELDS[..3];
        assert!(matches!(
            check_registry_entries(&fields),
            Err(RegistryError::FieldTableMismatch { .. })
        ));
        let mut hint = REGISTRY;
        hint[2].export.android = "";
        assert_eq!(
            check_registry_entries(&hint).unwrap_err(),
            RegistryError::EmptyExportHint {
                kind: modal::MODAL,
                target: ExportTarget::Android
            }
        );
    }

    #[test]
    fn export_hints_are_names_only() {
        assert_eq!(
            descriptor(button::BUTTON)
                .unwrap()
                .export
                .hint(ExportTarget::Web),
            "button"
        );
        assert_eq!(
            descriptor(selection_group::SINGLE_SELECTION_GROUP)
                .unwrap()
                .export
                .hint(ExportTarget::Web),
            "div[role=radiogroup]"
        );
        assert!(descriptor(ContractKind(0x7777)).is_none());
    }

    #[test]
    fn table_validation_dispatches_by_kind_and_flags_unknown_kinds() {
        let library = library();
        let entry = library.get(toggle()).unwrap();
        let kind_of = |id| ContractScope::node_kind(entry, id);
        let mut table = ControlContractTable::new();
        let mut incomplete = button_contract(action_id(TOGGLED));
        incomplete.semantic.remove(button::fields::ACTIVATE);
        table
            .attach(stable(KNOB), kind_of, incomplete.clone())
            .unwrap();
        table
            .attach(
                stable(ROOT),
                kind_of,
                ControlContract::new(
                    SemanticContract::new(ContractKind(0x7777)),
                    InteractionContract::default(),
                ),
            )
            .unwrap();
        let issues = validate_contracts(&table, entry);
        // root(300) < knob(301) 순.
        assert_eq!(
            issues,
            vec![
                ContractIssue {
                    frame: stable(ROOT),
                    field: None,
                    problem: ContractProblem::UnknownKind,
                },
                ContractIssue {
                    frame: stable(KNOB),
                    field: Some(button::fields::ACTIVATE),
                    problem: ContractProblem::MissingRequired,
                },
            ]
        );
        assert_eq!(
            issues[1..],
            button::validate_button(stable(KNOB), &incomplete, entry)[..]
        );
    }
}
