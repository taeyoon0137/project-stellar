//! 생산자 전체 fixture(XXV.2.1), 증분 재검사(XXV.2.3), remediation 실행(XXV.3.3) test.

#![cfg(test)]

use std::collections::BTreeMap;

use stellar_accessibility::diagnostics::diagnose;
use stellar_accessibility::node::{AccessibilityNode, AccessibilityRole};
use stellar_accessibility::tree::AccessibilityTree;
use stellar_bindings::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use stellar_bindings::orphan::{BindingStore, BindingWorld};
use stellar_commands::change::node_keys;
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::test_support::{random_valid_request, FreshIds};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::font::{FontFaceRef, FontFileHash, FontSource};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{AssetRef, NodeRef, TokenRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Length, Size2};
use stellar_dependencies::declaration::{CommitId, LibraryId};
use stellar_dependencies::graph::DependencyNodeKey;
use stellar_dependencies::library_load::{LibraryUnavailableDiagnostic, UnavailableReason};
use stellar_document::frame::{keys as frame_keys, Clipping, CornerRadius, FramePayload};
use stellar_document::image::ImagePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::ops::BinaryOp;
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_target_profile::capability::{
    CapabilityPropertyId, CapabilityStatus, CapabilitySubject, ElementKind,
};
use stellar_target_profile::profile::TargetProfileId;
use stellar_target_profile::registry::ProfileRegistryVersion;
use stellar_target_profile::validate::{
    CapabilityDiagnosticCode, CapabilityFinding, MultiProfileReport, ProfileOutcome, ProfileReport,
};
use stellar_text::fallback::FontFallbackChain;
use stellar_text::missing_font::{collect_missing_fonts, TextFontUsage};
use stellar_text::registry::InMemoryFontRegistry;

use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity};
use crate::incremental::{origins, CheckContext, Contribution, IntegrityEngine};
use crate::index::{DiagnosticFilter, DiagnosticIndex};
use crate::remediation::{plan, RemediationActionId, RemediationPlan};
use crate::sources;

fn sid(n: u64) -> StableId {
    StableId::new_v7(1_900_000_000_000 + n, 0, n)
}

const CLIP_FRAME: u64 = 1;
const MASKED: u64 = 2;
const MASK_MODIFIER: u64 = 3;
const IMAGE: u64 = 4;
const PLAIN: u64 = 5;
const UNRELATED: u64 = 6;
const MISSING_PATH: u64 = 100;
const MISSING_MASK_SOURCE: u64 = 101;
const MISSING_ASSET: u64 = 102;

fn asset_exists(id: StableId) -> bool {
    id != sid(MISSING_ASSET)
}

/// 무작위 유효 문서에 깨진 참조 3개(clip path, mask source, Image asset)와 무관한 Rectangle 하나를 더합니다.
fn fixture() -> DocumentModel {
    let mut records = random_valid_document(11).to_records().expect("records");
    let root = records
        .nodes
        .iter()
        .find(|n| n.parent.is_none() && matches!(n.payload, NodePayload::Frame(_)))
        .map(|n| n.id)
        .expect("root frame");
    let mut last = records
        .nodes
        .iter()
        .filter(|n| n.parent == Some(root))
        .map(|n| n.order_key.clone())
        .max();
    let mut add = |id: u64, payload: NodePayload, records: &mut DocumentRecords| {
        let order_key = OrderKey::between(last.as_ref(), None);
        last = Some(order_key.clone());
        records.nodes.push(NodeRecord {
            id: sid(id),
            parent: Some(root),
            order_key,
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload,
        });
    };
    let rect = || {
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(10.0, 10.0).unwrap(),
            corner_radius: CornerRadius::ZERO,
        }))
    };
    add(
        CLIP_FRAME,
        NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(MISSING_PATH))),
            ..FramePayload::default()
        }),
        &mut records,
    );
    add(MASKED, rect(), &mut records);
    add(
        IMAGE,
        NodePayload::Image(ImagePayload::new(
            AssetRef::new(sid(MISSING_ASSET)),
            Size2::new(4.0, 4.0).unwrap(),
        )),
        &mut records,
    );
    add(PLAIN, rect(), &mut records);
    add(UNRELATED, rect(), &mut records);
    let mut stack = ModifierStack::new(sid(MASKED));
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(sid(MASK_MODIFIER)),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Mask,
            enabled: true,
            payload: ModifierPayload::Mask {
                source: NodeRef::new(sid(MISSING_MASK_SOURCE)),
            },
        })
        .expect("mask");
    records.modifier_stacks.push(stack);
    let (model, _) = DocumentModel::from_records(records).expect("유효 문서");
    model
}

fn records_of(model: &DocumentModel) -> DocumentRecords {
    let mut records = model.to_records().expect("records");
    records.sort_canonical();
    records
}

const DOC: u64 = 900;

fn context() -> CheckContext<'static> {
    CheckContext {
        asset_exists: Some(&asset_exists),
    }
}

/// 내장 checker 결과를 scope 단위로 index에 넣습니다.
fn index_built_ins(index: &mut DiagnosticIndex, model: &DocumentModel) {
    let records = records_of(model);
    for checker in sources::built_in_checkers() {
        let contribution = checker
            .check(sid(DOC), &records, &context())
            .expect("문맥 있음");
        index.replace_scope(Some(sid(DOC)), checker.origin(), contribution.diagnostics);
    }
}

/// 문서 변경 없이 같은 결과를 내는 baseline(무작위 문서 자체의 진단 수).
fn baseline() -> BTreeMap<DiagnosticKind, usize> {
    let mut index = DiagnosticIndex::new();
    index_built_ins(&mut index, &random_valid_document(11));
    DiagnosticKind::ALL
        .into_iter()
        .map(|k| (k, index.summary().kind(k)))
        .collect()
}

// --- 외부 생산자 fixture ---

fn length() -> TypeSpec {
    TypeSpec::required(ExpressionType::Length)
}

fn token_ref() -> ReferenceTarget {
    ReferenceTarget::Token(TokenRef::new(sid(1_001)))
}

fn plus_one(base: u64, target: ReferenceTarget) -> ExpressionAst {
    let x = |n: u64| ExprNodeId::new(sid(n));
    ExpressionAst::new(
        x(base),
        vec![
            ExprNode::binary(x(base), BinaryOp::Add, x(base + 1), x(base + 2)),
            ExprNode::reference(x(base + 1), target),
            ExprNode::literal(
                x(base + 2),
                TypedValue::Length(Length::new(1.0).expect("유한")),
            ),
        ],
    )
    .expect("유효")
}

struct World {
    targets: BTreeMap<PropertyPath, TypeSpec>,
}

impl BindingWorld for World {
    fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
        self.targets.get(target).copied()
    }
    fn reference_type(&self, _target: &ReferenceTarget) -> Option<TypeSpec> {
        None // token이 모두 사라진 상태
    }
}

/// binding 3개: PLAIN·IMAGE target은 token이 없어 orphan, 없는 node target은 target-missing orphan.
fn binding_store() -> BindingStore {
    let target = |node: u64| PropertyPath::new(sid(node)).push(PropertyKey(0x0101));
    let mut live = BindingSet::new();
    for (n, node) in [(1u64, PLAIN), (2, IMAGE), (3, 7_777)] {
        live.insert(ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            target(node),
            plus_one(10 * n + 2_000, token_ref()),
            length(),
            ScopeOwner::Node(NodeRef::new(sid(node))),
        ))
        .expect("binding");
    }
    let mut store = BindingStore::new(live);
    let world = World {
        targets: [(target(PLAIN), length()), (target(IMAGE), length())].into(),
    };
    assert_eq!(store.orphan_broken(&world).len(), 3);
    store
}

fn missing_fonts() -> Vec<stellar_text::missing_font::MissingFontDiagnostic> {
    let face = FontFaceRef::new(
        FontSource::Project,
        FontFileHash::from_sha256_digest([7; 32]),
        0,
        vec![],
    );
    let chain = FontFallbackChain::new(vec![face]).expect("chain");
    let usages = [
        TextFontUsage {
            text_id: sid(PLAIN),
            chain: &chain,
        },
        TextFontUsage {
            text_id: sid(MASKED),
            chain: &chain,
        },
    ];
    collect_missing_fonts(&usages, &InMemoryFontRegistry::new())
}

fn accessibility_tree() -> AccessibilityTree {
    let mut tree = AccessibilityTree::new();
    tree.insert(AccessibilityNode::new(sid(IMAGE), AccessibilityRole::Image))
        .expect("node");
    tree.push_root(sid(IMAGE));
    tree
}

fn target_report() -> MultiProfileReport {
    let finding = |node: u64, code: CapabilityDiagnosticCode, status: Option<CapabilityStatus>| {
        CapabilityFinding {
            node: sid(node),
            subject: CapabilitySubject::Property(CapabilityPropertyId(0x0101)),
            path: Some(PropertyPath::with_keys(
                sid(node),
                vec![PropertyKey(0x0101)],
            )),
            status,
            code,
            replacement: None,
        }
    };
    MultiProfileReport {
        profiles: vec![ProfileReport {
            profile: TargetProfileId::new("web").expect("id"),
            outcome: ProfileOutcome::Validated {
                registry_version: ProfileRegistryVersion::new(1).expect("version"),
                findings: vec![
                    finding(
                        IMAGE,
                        CapabilityDiagnosticCode::Unsupported,
                        Some(CapabilityStatus::Unsupported),
                    ),
                    finding(
                        IMAGE,
                        CapabilityDiagnosticCode::Native,
                        Some(CapabilityStatus::Native),
                    ),
                    CapabilityFinding {
                        subject: CapabilitySubject::Element(ElementKind(6)),
                        ..finding(
                            IMAGE,
                            CapabilityDiagnosticCode::Deprecated,
                            Some(CapabilityStatus::Deprecated),
                        )
                    },
                ],
            },
        }],
        unknown_override_profiles: vec![],
    }
}

fn unavailable_library() -> LibraryUnavailableDiagnostic {
    LibraryUnavailableDiagnostic {
        node: DependencyNodeKey::new(
            LibraryId::from_stable_id(sid(8_000)),
            "0123456789abcdef0123456789abcdef01234567"
                .parse::<CommitId>()
                .expect("commit"),
        ),
        paths: vec![],
        reason: UnavailableReason::MissingCacheEntry,
    }
}

/// 외부 생산자 결과: (문서 또는 프로젝트, 생산자, 결과).
fn external_contributions() -> Vec<(Option<StableId>, &'static str, Contribution)> {
    let document = sid(DOC);
    let tree = accessibility_tree();
    vec![
        (
            Some(document),
            origins::BINDINGS,
            sources::bindings(document, &binding_store()),
        ),
        (
            Some(document),
            origins::FONTS,
            sources::fonts(document, &missing_fonts()),
        ),
        (
            Some(document),
            origins::ACCESSIBILITY,
            sources::accessibility(document, &tree, &diagnose(&tree)),
        ),
        (
            None,
            origins::LIBRARIES,
            sources::libraries(&[unavailable_library()]),
        ),
        (
            Some(document),
            origins::TARGET_PROFILES,
            sources::target_profiles(document, &target_report()),
        ),
        (None, origins::MIGRATION, sources::migration(3, 4)),
        (
            Some(document),
            origins::TOKENS,
            sources::unresolved_refs(
                document,
                &[
                    TokenRef::new(sid(1_002)).unresolved_at(PropertyPath::with_keys(
                        sid(PLAIN),
                        vec![PropertyKey(0x0101)],
                    )),
                ],
            ),
        ),
    ]
}

/// 모든 생산자를 넣은 공통 index.
fn full_index(model: &DocumentModel) -> DiagnosticIndex {
    let mut index = DiagnosticIndex::new();
    index_built_ins(&mut index, model);
    for (document, origin, contribution) in external_contributions() {
        index.replace_scope(document, origin, contribution.diagnostics);
    }
    index
}

#[test]
fn fixture_counts_are_exact_per_kind_and_severity() {
    // 검증 기준(XXV.2.1): fixture count 정확.
    let base = baseline();
    let index = full_index(&fixture());
    let summary = index.summary();
    let expected = [
        (
            DiagnosticKind::MissingNode,
            base[&DiagnosticKind::MissingNode] + 2,
        ),
        (
            DiagnosticKind::MissingAsset,
            base[&DiagnosticKind::MissingAsset] + 1,
        ),
        (DiagnosticKind::OrphanedBinding, 3),
        (DiagnosticKind::MissingFont, 2),
        (DiagnosticKind::InvalidAccessibility, 1),
        (DiagnosticKind::MissingLibrary, 1),
        (DiagnosticKind::TargetUnsupported, 2),
        (DiagnosticKind::MigrationWarning, 1),
        (
            DiagnosticKind::MissingToken,
            base[&DiagnosticKind::MissingToken] + 1,
        ),
        (
            DiagnosticKind::MissingComponent,
            base[&DiagnosticKind::MissingComponent],
        ),
    ];
    for (kind, count) in expected {
        assert_eq!(summary.kind(kind), count, "{kind}");
    }
    assert_eq!(summary.total(), index.len());
    let errors = DiagnosticFilter {
        min_severity: Some(Severity::Error),
        ..Default::default()
    };
    // 오류: 접근성 label 누락 1 + target unsupported 1.
    assert_eq!(index.count(&errors), 2);
    // 모든 진단은 XXV.2.2 필드를 갖고 schema round-trip합니다.
    let all: Vec<Diagnostic> = index.iter().collect();
    assert_eq!(
        crate::codec::decode(&crate::codec::encode(&all)).unwrap(),
        all
    );
    for d in &all {
        assert!(!d.code().is_empty() && !d.key.origin.is_empty());
        if d.document().is_some() {
            assert!(
                d.source().is_some() || d.kind() == DiagnosticKind::TargetUnsupported,
                "{d:?}"
            );
        }
    }
    let mask = all
        .iter()
        .find(|d| d.source() == Some(sid(MASKED)) && d.kind() == DiagnosticKind::MissingNode)
        .expect("mask 진단");
    assert_eq!(
        mask.remediation,
        vec![
            RemediationActionId::RevealSource,
            RemediationActionId::RevealProperty,
            RemediationActionId::RemoveDanglingModifier
        ]
    );
    let clip = all
        .iter()
        .find(|d| d.source() == Some(sid(CLIP_FRAME)))
        .expect("clip 진단");
    assert_eq!(
        clip.property(),
        Some(&PropertyPath::with_keys(
            sid(CLIP_FRAME),
            vec![frame_keys::CLIPPING]
        ))
    );
    assert!(!clip
        .remediation
        .contains(&RemediationActionId::RemoveDanglingModifier));
}

/// 모든 생산자를 넣은 증분 엔진.
fn full_engine(model: &DocumentModel) -> IntegrityEngine {
    let mut engine = IntegrityEngine::new();
    engine.check_document(sid(DOC), &records_of(model), &context());
    for (document, origin, contribution) in external_contributions() {
        engine.submit(document, origin, contribution);
    }
    engine
}

fn execute(
    state: &mut DocumentState,
    command: Command,
    n: u64,
) -> stellar_commands::change::ChangeSet {
    let transaction = CommandTransaction::single(
        CommandId::from_stable_id(sid(50_000 + n)),
        CommandOrigin::User,
        CommandRequest::at(state, command),
    );
    state
        .execute_transaction(&transaction)
        .expect("실행")
        .changes
}

#[test]
fn unrelated_edit_recomputes_nothing_and_related_edit_rechecks_only_its_scope() {
    // 검증 기준(XXV.2.3): unrelated edit recompute 0.
    let model = fixture();
    let mut engine = full_engine(&model);
    // 엔진 전체 검사 결과는 생산자 결과를 index에 직접 넣은 것과 같습니다.
    assert_eq!(
        engine.index().iter().collect::<Vec<_>>(),
        full_index(&model).iter().collect::<Vec<_>>()
    );
    let mut state = DocumentState::new(model);
    let document = sid(DOC);
    let before_runs = engine.check_runs();
    let before: Vec<Diagnostic> = engine.index().iter().collect();
    let revision = engine.index().revision();
    // 편집 전용 필드(표시 이름·잠금)는 어느 scope에도 걸리지 않습니다.
    let unrelated = sid(UNRELATED);
    for (i, (key, value)) in [
        (
            node_keys::DISPLAY_NAME,
            PropertyValue::Text("바뀐 이름".into()),
        ),
        (node_keys::LOCKED, PropertyValue::Bool(true)),
    ]
    .into_iter()
    .enumerate()
    {
        let changes = execute(
            &mut state,
            Command::SetProperty(SetProperty {
                node: unrelated,
                key,
                value,
            }),
            i as u64,
        );
        let report = engine
            .apply_changes(document, &changes, state.model(), &context())
            .unwrap();
        assert!(
            report.rechecked.is_empty() && report.stale.is_empty(),
            "{report:?}"
        );
    }
    assert_eq!(engine.check_runs(), before_runs, "recompute 0");
    assert_eq!(engine.index().revision(), revision);
    assert_eq!(engine.index().iter().collect::<Vec<_>>(), before);

    // 표시 여부는 Target Profile(모든 key 관심)만 stale로 만들고 내장 checker는 실행하지 않습니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(PLAIN),
            key: node_keys::VISIBLE,
            value: PropertyValue::Bool(false),
        }),
        10,
    );
    let report = engine
        .apply_changes(document, &changes, state.model(), &context())
        .unwrap();
    assert!(report.rechecked.is_empty());
    let stale: Vec<String> = report.stale.iter().map(|(_, o)| o.clone()).collect();
    assert!(stale.contains(&origins::TARGET_PROFILES.to_string()));
    assert!(
        stale.contains(&origins::BINDINGS.to_string()),
        "PLAIN은 binding target"
    );
    assert!(
        !stale.contains(&origins::ACCESSIBILITY.to_string()),
        "PLAIN은 접근성 tree 밖"
    );
    assert_eq!(engine.check_runs(), before_runs);

    // clip을 풀면 참조 checker만 다시 계산하고 clip 진단 하나가 사라집니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(CLIP_FRAME),
            key: frame_keys::CLIPPING,
            value: PropertyValue::Clipping(Clipping::FrameBounds),
        }),
        11,
    );
    let missing_nodes = engine.index().summary().kind(DiagnosticKind::MissingNode);
    let report = engine
        .apply_changes(document, &changes, state.model(), &context())
        .unwrap();
    assert_eq!(report.rechecked, vec![origins::REFERENCES]);
    assert_eq!(engine.check_runs(), before_runs + 1);
    assert_eq!(
        engine.index().summary().kind(DiagnosticKind::MissingNode),
        missing_nodes - 1
    );
}

#[test]
fn remediation_runs_as_ordinary_transaction_and_undo_restores_diagnostic() {
    // 검증 기준(XXV.3.3): command remediation은 일반 CommandTransaction, navigation은 문서를 바꾸지 않음.
    let model = fixture();
    let mut engine = full_engine(&model);
    let mut state = DocumentState::new(model);
    let document = sid(DOC);
    let mask = engine
        .index()
        .iter()
        .find(|d| d.source() == Some(sid(MASKED)) && d.kind() == DiagnosticKind::MissingNode)
        .unwrap();
    let RemediationPlan::Navigate(_) = plan(
        &mask,
        RemediationActionId::RevealProperty,
        state.revision(),
        CommandId::from_stable_id(sid(1)),
    )
    .unwrap() else {
        panic!("navigation");
    };
    let RemediationPlan::Transaction(transaction) = plan(
        &mask,
        RemediationActionId::RemoveDanglingModifier,
        state.revision(),
        CommandId::from_stable_id(sid(60_000)),
    )
    .unwrap() else {
        panic!("transaction");
    };
    let applied = state
        .execute_transaction(&transaction)
        .expect("일반 경로 실행");
    assert!(state.model().modifier_stack(sid(MASKED)).is_none());
    engine
        .apply_changes(document, &applied.changes, state.model(), &context())
        .unwrap();
    assert!(
        engine.index().get(&mask.key).is_none(),
        "고친 진단은 사라짐"
    );
    let undo = state.execute_transaction(&applied.inverse).expect("undo");
    engine
        .apply_changes(document, &undo.changes, state.model(), &context())
        .unwrap();
    assert_eq!(engine.index().get(&mask.key), Some(mask));
}

#[test]
fn incremental_result_equals_full_recheck_on_random_command_sequences() {
    // footprint가 결과를 바꾸는 변경을 놓치지 않는지: 무작위 유효 command 열에서 증분 결과 = 전체 재검사.
    let odd_missing = |id: StableId| !id.to_bytes()[15].is_multiple_of(3);
    let ctx = CheckContext {
        asset_exists: Some(&odd_missing),
    };
    let document = sid(DOC);
    let mut skipped = 0u64;
    let mut steps = 0u64;
    for seed in 0..12u64 {
        let mut state = DocumentState::new(random_valid_document(seed));
        let mut engine = IntegrityEngine::new();
        engine.check_document(document, &records_of(state.model()), &ctx);
        let mut rng = SplitMix64::new(seed);
        let mut ids = FreshIds::new(seed);
        for step in 0..40u64 {
            let Some(request) = random_valid_request(&state, &mut rng, &mut ids, 64) else {
                break;
            };
            let transaction = CommandTransaction::single(
                CommandId::from_stable_id(sid(70_000 + seed * 100 + step)),
                CommandOrigin::User,
                request,
            );
            let Ok(applied) = state.execute_transaction(&transaction) else {
                continue;
            };
            let runs = engine.check_runs();
            engine
                .apply_changes(document, &applied.changes, state.model(), &ctx)
                .unwrap();
            if engine.check_runs() == runs {
                skipped += 1;
            }
            steps += 1;
            let mut fresh = IntegrityEngine::new();
            fresh.check_document(document, &records_of(state.model()), &ctx);
            assert_eq!(
                engine.index().iter().collect::<Vec<_>>(),
                fresh.index().iter().collect::<Vec<_>>(),
                "seed {seed} step {step}: {:?}",
                transaction.requests[0].command
            );
        }
    }
    assert!(steps > 100, "충분한 command 실행 {steps}");
    assert!(
        skipped > 0,
        "일부 변경은 재검사 없이 지나감 {skipped}/{steps}"
    );
}
