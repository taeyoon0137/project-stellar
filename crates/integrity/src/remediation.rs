//! remediation action inventory와 실행 계획 (XXV.3.3).
//!
//! remediation은 두 형태만 있습니다(ADR-0095 결정 5).
//!
//! - **명시 navigation**([`NavigationTarget`]): 문서의 node·속성 또는 프로젝트 설정 절로 이동합니다. 문서를 바꾸지 않습니다.
//! - **일반 CommandTransaction**: closed enum command(`stellar_commands::command::Command`)로 만든 transaction입니다.
//!   실행은 다른 편집과 같은 경로(`DocumentState::execute_transaction`, headless는 `stellar apply`)를 지나므로 잠금·
//!   검증·undo·저장 규칙이 그대로 적용됩니다.
//!
//! 임의 script·외부 명령·plugin 실행 형태는 없습니다. [`RemediationPlan`]은 위 두 variant만 가진 closed enum이고,
//! 진단이 제공하는 action은 [`ACTION_INVENTORY`]에 있는 [`RemediationActionId`]만 될 수 있습니다(숫자 값은 저장 schema).
//!
//! | action | 형태 | 대상 진단 | 계획 |
//! |---|---|---|---|
//! | `reveal-source` | navigation | source가 있는 진단 | `Node { document, source }` |
//! | `reveal-property` | navigation | property path가 있는 진단 | `Property { document, path }` |
//! | `open-dependencies` | navigation | missing library | 프로젝트 의존성 절 |
//! | `open-fonts` | navigation | missing font | 프로젝트 font 절 |
//! | `open-target-profiles` | navigation | target unsupported | 프로젝트 Target Profile 절 |
//! | `open-migration` | navigation | migration warning | 프로젝트 schema migration 절 |
//! | `remove-dangling-modifier` | command | mask·clip source가 없는 modifier(missing node) | `RemoveModifier { owner, modifier }` 하나 |
//!
//! Boolean operand 하나가 없을 때 modifier 전체를 지우면 남은 operand의 결과까지 잃으므로 command action을 주지 않고
//! navigation만 줍니다. OrphanedBinding 재연결·삭제(VIII.8.4)는 아직 CommandTransaction이 아닌 binding 전용
//! command라 이 inventory에 넣지 않습니다(결정 5).

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_document::modifier_metadata::keys as modifier_keys;

use crate::diagnostic::DiagnosticKind;

/// remediation action 식별자. 숫자 값은 저장 schema([`crate::codec`])의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum RemediationActionId {
    /// source node로 이동.
    RevealSource = 1,
    /// 원인 속성으로 이동.
    RevealProperty = 2,
    /// 프로젝트 의존성 절로 이동.
    OpenDependencies = 3,
    /// 프로젝트 font 절로 이동.
    OpenFonts = 4,
    /// 프로젝트 Target Profile 절로 이동.
    OpenTargetProfiles = 5,
    /// 프로젝트 schema migration 절로 이동.
    OpenMigration = 6,
    /// source가 없는 mask·clip modifier를 지우는 CommandTransaction.
    RemoveDanglingModifier = 7,
}

/// action 형태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionForm {
    /// 명시 navigation.
    Navigation,
    /// 일반 CommandTransaction.
    Command,
}

/// inventory 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDescriptor {
    /// action.
    pub id: RemediationActionId,
    /// 형태.
    pub form: ActionForm,
    /// 사람이 읽는 설명.
    pub summary: &'static str,
}

/// 전체 remediation action inventory(숫자 순). 새 action은 여기와 [`plan`]에 함께 추가해야 합니다.
pub const ACTION_INVENTORY: [ActionDescriptor; 7] = [
    ActionDescriptor {
        id: RemediationActionId::RevealSource,
        form: ActionForm::Navigation,
        summary: "원인 객체로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::RevealProperty,
        form: ActionForm::Navigation,
        summary: "원인 속성으로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::OpenDependencies,
        form: ActionForm::Navigation,
        summary: "프로젝트 의존성 설정으로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::OpenFonts,
        form: ActionForm::Navigation,
        summary: "프로젝트 font 설정으로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::OpenTargetProfiles,
        form: ActionForm::Navigation,
        summary: "Target Profile 설정으로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::OpenMigration,
        form: ActionForm::Navigation,
        summary: "schema migration 안내로 이동합니다",
    },
    ActionDescriptor {
        id: RemediationActionId::RemoveDanglingModifier,
        form: ActionForm::Command,
        summary: "source가 없는 mask·clip modifier를 지웁니다(일반 CommandTransaction, undo 가능)",
    },
];

impl RemediationActionId {
    /// 모든 action(숫자 순).
    pub const ALL: [RemediationActionId; 7] = [
        RemediationActionId::RevealSource,
        RemediationActionId::RevealProperty,
        RemediationActionId::OpenDependencies,
        RemediationActionId::OpenFonts,
        RemediationActionId::OpenTargetProfiles,
        RemediationActionId::OpenMigration,
        RemediationActionId::RemoveDanglingModifier,
    ];

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            RemediationActionId::RevealSource => "reveal-source",
            RemediationActionId::RevealProperty => "reveal-property",
            RemediationActionId::OpenDependencies => "open-dependencies",
            RemediationActionId::OpenFonts => "open-fonts",
            RemediationActionId::OpenTargetProfiles => "open-target-profiles",
            RemediationActionId::OpenMigration => "open-migration",
            RemediationActionId::RemoveDanglingModifier => "remove-dangling-modifier",
        }
    }

    /// 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<RemediationActionId> {
        RemediationActionId::ALL
            .into_iter()
            .find(|a| a.name() == name)
    }

    /// 저장 schema 숫자 값.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// 숫자 값에서 찾습니다.
    pub fn from_code(code: u8) -> Option<RemediationActionId> {
        RemediationActionId::ALL
            .into_iter()
            .find(|a| a.code() == code)
    }

    /// inventory 항목.
    pub fn descriptor(self) -> &'static ActionDescriptor {
        ACTION_INVENTORY
            .iter()
            .find(|d| d.id == self)
            .expect("모든 action은 inventory에 있음")
    }
}

impl fmt::Display for RemediationActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 진단 종류와 위치에서 제공할 action을 정합니다(생산자 변환이 부릅니다, inventory 순·중복 없음).
pub fn actions_for(
    kind: DiagnosticKind,
    document: Option<StableId>,
    source: Option<StableId>,
    property: Option<&PropertyPath>,
) -> Vec<RemediationActionId> {
    use RemediationActionId as A;
    let mut actions = Vec::new();
    if document.is_some() && source.is_some() {
        actions.push(A::RevealSource);
    }
    if document.is_some() && property.is_some() {
        actions.push(A::RevealProperty);
    }
    match kind {
        DiagnosticKind::MissingLibrary => actions.push(A::OpenDependencies),
        DiagnosticKind::MissingFont => actions.push(A::OpenFonts),
        DiagnosticKind::TargetUnsupported => actions.push(A::OpenTargetProfiles),
        DiagnosticKind::MigrationWarning => actions.push(A::OpenMigration),
        DiagnosticKind::MissingNode
            if source.is_some() && property.is_some_and(is_dangling_modifier_source) =>
        {
            actions.push(A::RemoveDanglingModifier)
        }
        _ => {}
    }
    actions.sort();
    actions.dedup();
    actions
}

/// 경로가 mask·clip modifier의 source 속성(`(modifier, [MASK_SOURCE|CLIP_SOURCE])`)인지.
fn is_dangling_modifier_source(path: &PropertyPath) -> bool {
    path.tail().is_none()
        && matches!(path.keys(), [key] if *key == modifier_keys::MASK_SOURCE || *key == modifier_keys::CLIP_SOURCE)
}

