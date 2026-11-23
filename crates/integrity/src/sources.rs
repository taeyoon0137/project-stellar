//! 생산자 결과 → 공통 진단 변환과 내장 checker (XXV.2.1).
//!
//! 판정은 하지 않고 생산자의 공개 함수·결과를 그대로 옮깁니다(이중 판정 금지).
//!
//! | 종류 | 생산자(판정 함수) | 변환 | scope |
//! |---|---|---|---|
//! | missing node·token·component | `stellar_document::validate::validate_document`의 미해결 참조 | [`ReferenceChecker`](내장) | 문서 |
//! | missing asset | `ImagePayload::resolve_asset`(VI.8.2) | [`AssetChecker`](내장, asset store 조회 필요) | 문서 |
//! | OrphanedBinding | `stellar_bindings::orphan::BindingStore`(VIII.8.1) | [`bindings`] | 문서 |
//! | missing token(외부 참조) | 호출자가 모은 `UnresolvedRef`(token registry 등) | [`unresolved_refs`] | 문서 |
//! | missing font | `stellar_text::missing_font::collect_missing_fonts`(X.8.4) | [`fonts`] | 문서 |
//! | invalid accessibility | `stellar_accessibility::diagnostics::diagnose`(IX.8.5) | [`accessibility`] | 문서 |
//! | missing library | `stellar_dependencies::library_load`(XV.5.5) | [`libraries`]·[`library_refs`] | 프로젝트·문서 |
//! | target unsupported | `stellar_target_profile::validate::validate_profiles`(IX.10.8) | [`target_profiles`] | 문서 |
//! | migration warning | project schema 판정(IX.11.21)의 메모리 migration | [`migration`] | 프로젝트 |

use std::collections::BTreeSet;

use stellar_accessibility::diagnostics::{AccessibilityIssue, Severity as AccessibilitySeverity};
use stellar_accessibility::tree::AccessibilityTree;
use stellar_bindings::orphan::{BindingStore, OrphanCause};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{ExpectedRefType, NodeRef, UnresolvedRef};
use stellar_dependencies::library_load::{
    LibraryUnavailableDiagnostic, UnavailableReason, UnresolvedExternalRef,
};
use stellar_document::frame::{keys as frame_keys, Clipping};
use stellar_document::image::{ImageResolution, IMAGE_PROP_ASSET};
use stellar_document::modifier::ModifierPayload;
use stellar_document::modifier_metadata::keys as modifier_keys;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::validate::validate_document;
use stellar_expressions::ast::ReferenceTarget;
use stellar_target_profile::validate::{
    CapabilityDiagnosticCode, MultiProfileReport, ProfileOutcome,
};
use stellar_text::missing_font::MissingFontDiagnostic;
use stellar_text::registry::FontFaceKey;

use crate::diagnostic::{Diagnostic, DiagnosticKey, DiagnosticKind, Severity};
use crate::incremental::{
    origins, CheckContext, Contribution, DocumentChecker, Footprint, KeyInterest, SourceId,
};
use crate::remediation::actions_for;

/// 진단 하나를 만듭니다(remediation은 종류·위치 규칙으로).
#[allow(clippy::too_many_arguments)]
fn make(
    origin: SourceId,
    document: Option<StableId>,
    kind: DiagnosticKind,
    severity: Severity,
    source: Option<StableId>,
    property: Option<PropertyPath>,
    code: impl Into<String>,
    subject: impl Into<String>,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        remediation: actions_for(kind, document, source, property.as_ref()),
        key: DiagnosticKey {
            document,
            kind,
            source,
            property,
            code: code.into(),
            subject: subject.into(),
            origin: origin.to_string(),
        },
        severity,
        message: message.into(),
    }
}

/// 미해결 참조 종류 → 진단 종류.
pub fn kind_for_reference(expected: ExpectedRefType) -> DiagnosticKind {
    match expected {
        ExpectedRefType::Node => DiagnosticKind::MissingNode,
        ExpectedRefType::Token => DiagnosticKind::MissingToken,
        ExpectedRefType::Component => DiagnosticKind::MissingComponent,
        ExpectedRefType::Asset => DiagnosticKind::MissingAsset,
    }
}

fn reference_name(expected: ExpectedRefType) -> &'static str {
    match expected {
        ExpectedRefType::Node => "node",
        ExpectedRefType::Token => "token",
        ExpectedRefType::Component => "컴포넌트",
        ExpectedRefType::Asset => "asset",
    }
}

fn from_unresolved(
    origin: SourceId,
    document: StableId,
    unresolved: &UnresolvedRef,
    source: StableId,
    code: &str,
) -> Diagnostic {
    let expected = unresolved.expected();
    make(
        origin,
        Some(document),
        kind_for_reference(expected),
        Severity::Warning,
        Some(source),
        Some(unresolved.location().clone()),
        code,
        unresolved.original_id().to_string(),
        format!(
            "{} 참조 {}의 대상이 없습니다",
            reference_name(expected),
            unresolved.original_id()
        ),
    )
}

/// 호출자가 모은 미해결 참조(token registry alias, 외부 표현식 등)를 진단으로 바꿉니다. source는 참조 위치의 root입니다.
/// footprint는 위치 root와 원래 대상 ID입니다.
pub fn unresolved_refs(document: StableId, refs: &[UnresolvedRef]) -> Contribution {
    let mut footprint = Footprint::default();
    let diagnostics = refs
        .iter()
        .map(|r| {
            footprint.nodes.insert(r.location().root());
            footprint.nodes.insert(r.original_id());
            from_unresolved(
                origins::TOKENS,
                document,
                r,
                r.location().root(),
                "unresolved-reference",
            )
        })
        .collect();
    Contribution {
        diagnostics,
        footprint,
    }
}

// --- 내장 checker: 미해결 node 참조 ---

/// 문서 레코드에서 참조를 담을 수 있는 속성 key. `validate_document`의 참조 검사(VI.10.1 5단계)가 읽는 위치와 같아야
/// 합니다(`증분_결과는_전체_재검사와_같다` test가 어긋남을 잡습니다).
pub fn reference_keys() -> BTreeSet<PropertyKey> {
    [
        frame_keys::CLIPPING,
        modifier_keys::BOOLEAN_OPERANDS,
        modifier_keys::MASK_SOURCE,
        modifier_keys::CLIP_SOURCE,
    ]
    .into()
}

/// 참조 위치 하나: (소유 node, 위치 root(node 또는 modifier), 대상).
fn reference_sites(records: &DocumentRecords) -> Vec<(StableId, StableId, NodeRef)> {
    let mut sites = Vec::new();
    for node in &records.nodes {
        if let NodePayload::Frame(frame) = &node.payload {
            if let Clipping::Path(target) = frame.clipping {
                sites.push((node.id, node.id, target));
            }
        }
    }
    for stack in &records.modifier_stacks {
        for entry in stack.entries() {
            let modifier = entry.id.stable_id();
            match &entry.payload {
                ModifierPayload::Boolean { operands, .. } => {
                    sites.extend(operands.iter().map(|o| (stack.node, modifier, *o)))
                }
                ModifierPayload::Mask { source } | ModifierPayload::Clip { source } => {
                    sites.push((stack.node, modifier, *source))
                }
                _ => {}
            }
        }
    }
    sites
}

/// 문서 validator의 미해결 참조를 missing node·token·component 진단으로 옮기는 내장 checker.
///
/// 문서 validator가 위반(구조 오류)을 내면 미해결 참조 목록을 얻을 수 없으므로 진단을 내지 않습니다. 구조 오류는
/// `stellar validate`(`document-invalid`)의 몫입니다.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReferenceChecker;

impl DocumentChecker for ReferenceChecker {
    fn origin(&self) -> SourceId {
        origins::REFERENCES
    }

    fn check(
        &self,
        document: StableId,
        records: &DocumentRecords,
        _context: &CheckContext<'_>,
    ) -> Option<Contribution> {
        let sites = reference_sites(records);
        let mut footprint = Footprint {
            nodes: BTreeSet::new(),
            keys: KeyInterest::Keys(reference_keys()),
            inserts: true,
        };
        for (owner, root, target) in &sites {
            footprint.nodes.extend([*owner, *root, target.id()]);
        }
        let diagnostics = match validate_document(records) {
            Ok(result) => result
                .unresolved
                .iter()
                .map(|unresolved| {
                    let root = unresolved.location().root();
                    // 위치 root가 modifier면 소유 node를 source로 씁니다(remediation이 그 node의 stack을 고침).
                    let owner = sites
                        .iter()
                        .find(|(_, r, _)| *r == root)
                        .map_or(root, |(owner, _, _)| *owner);
                    footprint.nodes.insert(unresolved.original_id());
                    from_unresolved(
                        origins::REFERENCES,
                        document,
                        unresolved,
                        owner,
                        "unresolved-reference",
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        Some(Contribution {
            diagnostics,
            footprint,
        })
    }
}

// --- 내장 checker: Image asset ---

/// Image node의 asset 참조를 asset store 조회로 해석하는 내장 checker(`ImagePayload::resolve_asset`).
/// [`CheckContext::asset_exists`]가 없으면 실행하지 않습니다.
#[derive(Debug, Clone, Copy, Default)]
pub struct AssetChecker;

impl DocumentChecker for AssetChecker {
    fn origin(&self) -> SourceId {
        origins::ASSETS
    }

    fn check(
        &self,
        document: StableId,
        records: &DocumentRecords,
        context: &CheckContext<'_>,
    ) -> Option<Contribution> {
        let exists = context.asset_exists?;
        let mut footprint = Footprint {
            nodes: BTreeSet::new(),
            keys: KeyInterest::Keys([IMAGE_PROP_ASSET].into()),
            inserts: true,
        };
        let mut diagnostics = Vec::new();
        for node in &records.nodes {
            let NodePayload::Image(image) = &node.payload else {
                continue;
            };
            footprint.nodes.insert(node.id);
            if let ImageResolution::MissingResource(unresolved) =
                image.resolve_asset(node.id, exists)
            {
                diagnostics.push(from_unresolved(
                    origins::ASSETS,
                    document,
                    &unresolved,
                    node.id,
                    "missing-asset-resource",
                ));
            }
        }
        Some(Contribution {
            diagnostics,
            footprint,
        })
    }
}

/// 내장 checker 목록.
pub fn built_in_checkers() -> Vec<Box<dyn DocumentChecker>> {
    vec![Box::new(ReferenceChecker), Box::new(AssetChecker)]
}

// --- 외부 생산자 변환 ---

fn orphan_code(cause: &OrphanCause) -> &'static str {
    match cause {
        OrphanCause::TargetMissing => "orphaned-binding/target-missing",
        OrphanCause::SourceMissing { .. } => "orphaned-binding/source-missing",
        OrphanCause::SourceTypeChanged { .. } => "orphaned-binding/source-type-changed",
        OrphanCause::TargetTypeChanged { .. } => "orphaned-binding/target-type-changed",
    }
}

/// binding 저장소의 OrphanedBinding을 진단으로 바꿉니다. footprint는 live·orphan binding 전체의 target node와 속성
/// 참조 대상입니다(그 node가 사라지거나 타입이 바뀌면 orphan 판정이 바뀔 수 있음).
pub fn bindings(document: StableId, store: &BindingStore) -> Contribution {
    let mut footprint = Footprint::default();
    let mut watch = |target: &PropertyPath, references: BTreeSet<ReferenceTarget>| {
        footprint.nodes.insert(target.root());
        for reference in references {
            if let ReferenceTarget::Property(path) = reference {
                footprint.nodes.insert(path.root());
            }
        }
    };
    for binding in store.live().iter() {
        watch(binding.target(), binding.references());
    }
    let mut diagnostics = Vec::new();
    for orphan in store.orphans() {
        watch(orphan.target(), orphan.binding().references());
        diagnostics.push(make(
            origins::BINDINGS,
            Some(document),
            DiagnosticKind::OrphanedBinding,
            Severity::Warning,
            Some(orphan.target().root()),
            Some(orphan.target().clone()),
            orphan_code(orphan.cause()),
            orphan.id().to_string(),
            format!("binding {}이 끊어졌습니다: {}", orphan.id(), orphan.cause()),
        ));
    }
    Contribution {
        diagnostics,
        footprint,
    }
}

/// missing font 진단(face별)을 영향받는 Text마다 하나씩 바꿉니다. footprint는 영향받는 Text와 새 node 삽입입니다
/// (새 Text의 fallback chain은 호출자가 다시 모아야 함).
pub fn fonts(document: StableId, missing: &[MissingFontDiagnostic]) -> Contribution {
    let mut footprint = Footprint {
        inserts: true,
        ..Footprint::default()
    };
    let mut diagnostics = Vec::new();
    for diagnostic in missing {
        let face = FontFaceKey::from(diagnostic.requested());
        let subject = format!("{face} {}", diagnostic.source());
        for text in diagnostic.affected_text_ids() {
            footprint.nodes.insert(*text);
            diagnostics.push(make(
                origins::FONTS,
                Some(document),
                DiagnosticKind::MissingFont,
                Severity::Warning,
                Some(*text),
                None,
                MissingFontDiagnostic::CODE,
                subject.clone(),
                format!(
                    "Text {text}의 font face {face}(출처 {})가 등록되어 있지 않습니다",
                    diagnostic.source()
                ),
            ));
        }
    }
    Contribution {
        diagnostics,
        footprint,
    }
}

/// 접근성 진단을 바꿉니다. footprint는 접근성 tree의 모든 source node와 node 삽입입니다(접근성 규칙은 조상·형제
/// 문맥을 보므로 tree 안 어느 node가 바뀌어도 다시 계산).
pub fn accessibility(
    document: StableId,
    tree: &AccessibilityTree,
    issues: &[AccessibilityIssue],
) -> Contribution {
    let footprint = Footprint {
        nodes: tree.nodes().map(|n| n.source).collect(),
        keys: KeyInterest::Nothing,
        inserts: true,
    };
    let diagnostics = issues
        .iter()
        .map(|issue| {
            let severity = match issue.severity() {
                AccessibilitySeverity::Error => Severity::Error,
                AccessibilitySeverity::Warning => Severity::Warning,
            };
            let message = issue.to_string();
            make(
                origins::ACCESSIBILITY,
                Some(document),
                DiagnosticKind::InvalidAccessibility,
                severity,
                Some(issue.node),
                None,
                issue.code(),
                format!("{:?}", issue.problem),
                message,
            )
        })
        .collect();
    Contribution {
        diagnostics,
        footprint,
    }
}

fn unavailable_code(reason: &UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::MissingCacheEntry => "library-unavailable/missing-cache-entry",
        UnavailableReason::ContentHashMismatch => "library-unavailable/content-hash-mismatch",
        UnavailableReason::InconsistentLockEntries => {
            "library-unavailable/inconsistent-lock-entries"
        }
        UnavailableReason::CacheFailure => "library-unavailable/cache-failure",
        UnavailableReason::ExportUnreadable(_) => "library-unavailable/export-unreadable",
        UnavailableReason::InvalidExports(_) => "library-unavailable/invalid-exports",
    }
}

/// 사용할 수 없는 library(프로젝트 단위)를 바꿉니다. 문서 변경과 무관합니다. 설명은 생산자 Display(repository URL 없음,
/// XV.1.4)입니다.
pub fn libraries(unavailable: &[LibraryUnavailableDiagnostic]) -> Contribution {
    let diagnostics = unavailable
        .iter()
        .map(|d| {
            make(
                origins::LIBRARIES,
                None,
                DiagnosticKind::MissingLibrary,
                Severity::Warning,
                None,
                None,
                unavailable_code(&d.reason),
                d.node.to_string(),
                d.to_string(),
            )
        })
        .collect();
    Contribution {
        diagnostics,
        footprint: Footprint::independent(),
    }
}

/// 사용할 수 없는 library의 resource를 가리키는 문서 참조를 바꿉니다. footprint는 참조 위치 root입니다.
pub fn library_refs(document: StableId, refs: &[UnresolvedExternalRef]) -> Contribution {
    let mut footprint = Footprint::default();
    let diagnostics = refs
        .iter()
        .map(|r| {
            let root = r.location().root();
            footprint.nodes.insert(root);
            make(
                origins::LIBRARY_REFS,
                Some(document),
                DiagnosticKind::MissingLibrary,
                Severity::Warning,
                Some(root),
                Some(r.location().clone()),
                "library-resource-unavailable",
                r.reference().to_string(),
                format!(
                    "사용할 수 없는 library의 resource를 가리킵니다: {}",
                    r.reference()
                ),
            )
        })
        .collect();
    Contribution {
        diagnostics,
        footprint,
    }
}

/// Target Profile 검사 결과에서 지원하지 않음·미등록(오류)과 폐기(경고)를 바꿉니다. Native·Approximated는 문제가 아니라
/// 바꾸지 않습니다. pin한 registry를 앱이 몰라 검사하지 못한 profile은 경고 하나입니다. footprint는 어느 속성이든 새
/// 사용이 될 수 있으므로 편집 전용 필드를 뺀 모든 key와 node 삽입입니다.
pub fn target_profiles(document: StableId, report: &MultiProfileReport) -> Contribution {
    let mut diagnostics = Vec::new();
    let mut footprint = Footprint {
        nodes: BTreeSet::new(),
        keys: KeyInterest::Any,
        inserts: true,
    };
    for profile in &report.profiles {
        match &profile.outcome {
            ProfileOutcome::Validated { findings, .. } => {
                for finding in findings {
                    footprint.nodes.insert(finding.node);
                    let (severity, code) = match finding.code {
                        CapabilityDiagnosticCode::Unsupported => (Severity::Error, "target-unsupported"),
                        CapabilityDiagnosticCode::NotRegistered => (Severity::Error, "target-not-registered"),
                        CapabilityDiagnosticCode::Deprecated => (Severity::Warning, "target-deprecated"),
                        CapabilityDiagnosticCode::Native | CapabilityDiagnosticCode::Approximated => continue,
                    };
                    let replacement = finding
                        .replacement
                        .as_ref()
                        .map(|r| format!(" (대체: {r})"))
                        .unwrap_or_default();
                    diagnostics.push(make(
                        origins::TARGET_PROFILES,
                        Some(document),
                        DiagnosticKind::TargetUnsupported,
                        severity,
                        Some(finding.node),
                        finding.path.clone(),
                        code,
                        format!("{} {}", profile.profile, finding.subject),
                        format!(
                            "Target Profile {}에서 {}: {}{replacement}",
                            profile.profile, code, finding.subject
                        ),
                    ));
                }
            }
            ProfileOutcome::RegistryUnavailable { pinned, latest } => diagnostics.push(make(
                origins::TARGET_PROFILES,
                Some(document),
                DiagnosticKind::TargetUnsupported,
                Severity::Warning,
                None,
                None,
                "target-registry-unavailable",
                profile.profile.to_string(),
                format!(
                    "Target Profile {}가 pin한 registry {pinned:?}를 이 앱(최신 {latest:?})이 몰라 검사하지 않았습니다",
                    profile.profile
                ),
            )),
        }
    }
    Contribution {
        diagnostics,
        footprint,
    }
}

/// 낮은 schema generation(`from`)을 대상 schema(`to`)로 메모리에서 migration해 읽기 전용으로 연 상태를 바꿉니다.
pub fn migration(from: u32, to: u32) -> Contribution {
    Contribution {
        diagnostics: vec![make(
            origins::MIGRATION,
            None,
            DiagnosticKind::MigrationWarning,
            Severity::Warning,
            None,
            None,
            "schema-migrated-in-memory",
            format!("{from}->{to}"),
            format!(
                "schema {from} generation을 {to}로 메모리에서 migration해 읽기 전용으로 열었습니다(`stellar migrate`로 기록해야 편집할 수 있음)"
            ),
        )],
        footprint: Footprint::independent(),
    }
}
