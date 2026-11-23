//! Context validator (XXX.7.5, ADR-0025 결정 3.7, ADR-0099 결정 5).
//!
//! | 진단 | 수준 |
//! |---|---|
//! | 정의 안(그 정의가 작성한 slot 콘텐츠 포함)의 선언 없는 소비 | 오류 |
//! | 타입 불일치(소비 binding, provision 값) | 오류 |
//! | provision의 owner 범위 밖 provider, provision 값 식 오류, provision 읽기 cycle | 오류 |
//! | 존재하지 않는 key의 provision·Slot 공개 목록·requirement·소비 | orphan |
//! | 쓰지 않는 requirement | 경고 |
//! | default로 해석됨, shadowing | 정보 |
//!
//! View 최상위 트리(scene node)의 소비는 선언이 필요 없습니다. 판정은 소비 node를 소유한 정의(`ComponentLibrary::owner_of`)로
//! 합니다. 정의 안에서 작성한 slot 콘텐츠도 그 정의가 소유하므로 그 정의의 requirement가 필요합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::bytecode::Program;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;

use super::provision::{check_provision_scope, context_reads, ContextOwner};
use super::resolve::{
    ContextEvalError, ContextEvaluator, ContextInputs, ContextOrigin, ContextResolver, ContextSite,
};
use super::set::ContextKeys;
use crate::component::definition::ComponentLibrary;
use crate::component::expansion::{ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads};
use crate::model::DocumentModel;
use crate::record_codec::encode_payload;
use crate::records::NodePayload;

/// 진단 수준.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextSeverity {
    /// 오류(저장·적용 전 거부 대상).
    Error,
    /// 참조 대상이 없는 항목.
    Orphan,
    /// 경고.
    Warning,
    /// 정보.
    Info,
}

/// 진단 종류.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextDiagnosticKind {
    /// 정의 안에서 requirement 없이 key를 읽습니다(binding 또는 provision 값 식).
    UndeclaredConsumption {
        /// 소비 node를 소유한 정의.
        component: StableId,
        /// key.
        key: StableId,
        /// 읽은 binding 또는 provision.
        reader: StableId,
    },
    /// 소비 binding의 기대 타입과 해석 타입이 맞지 않거나 provision 값 타입이 key 타입과 다릅니다.
    TypeMismatch {
        /// binding 또는 provision.
        reader: StableId,
        /// 기대 타입.
        expected: TypeSpec,
        /// 실제 타입(추론 실패면 `None`).
        found: Option<TypeSpec>,
    },
    /// provision이 owner 범위 밖 node를 가리킵니다.
    ProvisionOutsideOwner {
        /// provision.
        provision: StableId,
        /// 원인.
        error: String,
    },
    /// provision 값 식끼리의 읽기 cycle(서로 다른 key가 서로를 읽음).
    ProvisionCycle {
        /// cycle의 provision(순서대로).
        provisions: Vec<StableId>,
    },
    /// 존재하지 않는 key의 provision.
    OrphanProvision {
        /// provision.
        provision: StableId,
        /// 없는 key.
        key: StableId,
    },
    /// 존재하지 않는 key·Slot의 공개 목록 항목.
    OrphanSlotKey {
        /// Slot.
        slot: StableId,
        /// key.
        key: StableId,
    },
    /// 존재하지 않는 key·컴포넌트의 requirement.
    OrphanRequirement {
        /// 컴포넌트.
        component: StableId,
        /// key.
        key: StableId,
    },
    /// 존재하지 않는 key를 읽는 소비 binding(binding 쪽에서는 OrphanedBinding이 됩니다).
    OrphanConsumption {
        /// binding.
        binding: StableId,
        /// key.
        key: StableId,
    },
    /// 쓰지 않는 requirement.
    UnusedRequirement {
        /// 컴포넌트.
        component: StableId,
        /// key.
        key: StableId,
    },
    /// 소비가 key default로 해석됩니다.
    DefaultResolved {
        /// binding.
        binding: StableId,
        /// key.
        key: StableId,
    },
    /// 소비를 해석한 provision이 바깥 provision을 가립니다.
    Shadowing {
        /// binding.
        binding: StableId,
        /// key.
        key: StableId,
        /// 해석한 provision.
        provision: StableId,
        /// 가려진 provision(가까운 순).
        shadowed: Vec<StableId>,
    },
}

impl ContextDiagnosticKind {
    /// 수준.
    pub fn severity(&self) -> ContextSeverity {
        use ContextDiagnosticKind::*;
        match self {
            UndeclaredConsumption { .. }
            | TypeMismatch { .. }
            | ProvisionOutsideOwner { .. }
            | ProvisionCycle { .. } => ContextSeverity::Error,
            OrphanProvision { .. }
            | OrphanSlotKey { .. }
            | OrphanRequirement { .. }
            | OrphanConsumption { .. } => ContextSeverity::Orphan,
            UnusedRequirement { .. } => ContextSeverity::Warning,
            DefaultResolved { .. } | Shadowing { .. } => ContextSeverity::Info,
        }
    }

    /// 안정 코드(진단 이름).
    pub fn code(&self) -> &'static str {
        use ContextDiagnosticKind::*;
        match self {
            UndeclaredConsumption { .. } => "context.undeclared-consumption",
            TypeMismatch { .. } => "context.type-mismatch",
            ProvisionOutsideOwner { .. } => "context.provision-outside-owner",
            ProvisionCycle { .. } => "context.provision-cycle",
            OrphanProvision { .. } => "context.orphan-provision",
            OrphanSlotKey { .. } => "context.orphan-slot-key",
            OrphanRequirement { .. } => "context.orphan-requirement",
            OrphanConsumption { .. } => "context.orphan-consumption",
            UnusedRequirement { .. } => "context.unused-requirement",
            DefaultResolved { .. } => "context.default-resolved",
            Shadowing { .. } => "context.shadowing",
        }
    }
}

impl fmt::Display for ContextDiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ContextDiagnosticKind::*;
        match self {
            UndeclaredConsumption {
                component,
                key,
                reader,
            } => write!(
                f,
                "컴포넌트 {component} 안의 {reader}가 선언하지 않은 Context key {key}를 읽습니다"
            ),
            TypeMismatch {
                reader,
                expected,
                found,
            } => match found {
                Some(found) => write!(
                    f,
                    "{reader}: Context 값 타입 {found}는 {expected}에 맞지 않습니다"
                ),
                None => write!(
                    f,
                    "{reader}: Context 값 타입을 정할 수 없습니다(기대 {expected})"
                ),
            },
            ProvisionOutsideOwner { provision, error } => {
                write!(f, "provision {provision}: {error}")
            }
            ProvisionCycle { provisions } => write!(f, "provision 읽기 cycle: {provisions:?}"),
            OrphanProvision { provision, key } => {
                write!(f, "provision {provision}의 key {key}가 없습니다")
            }
            OrphanSlotKey { slot, key } => write!(f, "Slot {slot} 공개 목록의 {key}가 없습니다"),
            OrphanRequirement { component, key } => {
                write!(f, "컴포넌트 {component} requirement {key}가 없습니다")
            }
            OrphanConsumption { binding, key } => {
                write!(f, "binding {binding}이 없는 Context key {key}를 읽습니다")
            }
            UnusedRequirement { component, key } => {
                write!(f, "컴포넌트 {component}가 선언한 key {key}를 쓰지 않습니다")
            }
            DefaultResolved { binding, key } => {
                write!(f, "binding {binding}의 key {key}는 default로 해석됩니다")
            }
            Shadowing {
                binding,
                key,
                provision,
                shadowed,
            } => write!(
                f,
                "binding {binding}의 key {key}: provision {provision}이 {shadowed:?}를 가립니다"
            ),
        }
    }
}

/// 진단 하나.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextDiagnostic {
    /// 종류.
    pub kind: ContextDiagnosticKind,
}

impl ContextDiagnostic {
    /// 수준.
    pub fn severity(&self) -> ContextSeverity {
        self.kind.severity()
    }
}

/// 소비 binding 하나(XXX.7.5). binding은 문서 crate 밖 resource라 호출자가 넘깁니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextConsumer {
    /// binding StableId.
    pub binding: StableId,
    /// 소비 지점(target 속성 node, 정의 node면 인스턴스 chain 포함).
    pub site: ContextSite,
    /// target 속성 기대 타입.
    pub expected: TypeSpec,
    /// binding source.
    pub source: ExpressionAst,
}

impl ContextConsumer {
    /// source가 읽는 key.
    pub fn reads(&self) -> BTreeSet<StableId> {
        context_reads(&self.source)
    }
}

/// 문서의 모든 소비 가능 site: scene node 전체와 scene 인스턴스를 펼친 가상 node 전체(StableId·chain 순).
pub fn all_sites(model: &DocumentModel, library: &ComponentLibrary) -> Vec<ContextSite> {
    let mut payloads = ScenePayloads::new();
    let mut scene = Vec::new();
    for handle in model.columns().iter_live() {
        if let Some(id) = model.stable_id(handle) {
            if let Some(payload) = model.payload(id) {
                payloads.insert(id, encode_payload(payload));
            }
            scene.push(id);
        }
    }
    let index = SceneIndex::build(model.columns(), &payloads);
    let mut out: BTreeSet<ContextSite> = scene.iter().map(|id| ContextSite::scene(*id)).collect();
    for id in &scene {
        if !matches!(model.payload(*id), Some(NodePayload::ComponentInstance(_))) {
            continue;
        }
        let Ok(expansion) = InstanceExpansion::new(library, &index, *id) else {
            continue;
        };
        let Ok(walk) = expansion.walk() else {
            continue;
        };
        for node in walk {
            if let ExpandedNodeId::Virtual { .. } = node {
                out.insert(ContextSite::from(&node));
            }
        }
    }
    out.into_iter().collect()
}

struct TypesAt<'v> {
    keys: &'v ContextKeys<'v>,
    inputs: &'v dyn ContextInputs,
    site: &'v ContextSite,
}

impl ReferenceTypes for TypesAt<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Context(key) => self.keys.key(*key).map(|k| k.value_type()),
            other => self.inputs.input_type(other, self.site),
        }
    }
}

fn check_type(
    out: &mut Vec<ContextDiagnostic>,
    reader: StableId,
    ast: &ExpressionAst,
    expected: TypeSpec,
    types: &TypesAt<'_>,
) {
    let found = Program::compile(ast, types).ok().map(|p| p.result_type());
    let ok = found.is_some_and(|found| crate::component::typed_expr::assignable(found, expected));
    if !ok {
        out.push(ContextDiagnostic {
            kind: ContextDiagnosticKind::TypeMismatch {
                reader,
                expected,
                found,
            },
        });
    }
}

/// provision을 소유한 정의(View owner면 `None`).
fn provision_component(owner: ContextOwner) -> Option<StableId> {
    match owner {
        ContextOwner::Component(id) => Some(id),
        ContextOwner::View(_) => None,
    }
}

/// Context 진단을 모두 계산합니다(종류·대상 순으로 정렬, 결정적).
pub fn validate_context(
    model: &DocumentModel,
    library: &ComponentLibrary,
    keys: &ContextKeys<'_>,
    consumers: &[ContextConsumer],
    inputs: &dyn ContextInputs,
) -> Vec<ContextDiagnostic> {
    let context = model.context();
    let resolver = ContextResolver::new(model, library);
    let evaluator = ContextEvaluator::new(&resolver, keys, inputs);
    let mut out = Vec::new();
    let mut used: BTreeSet<(StableId, StableId)> = BTreeSet::new();
    let mut note_read = |out: &mut Vec<ContextDiagnostic>,
                         owner: Option<StableId>,
                         key: StableId,
                         reader: StableId| {
        if let Some(component) = owner {
            used.insert((component, key));
            if !context.requirements(component).contains(&key) {
                out.push(ContextDiagnostic {
                    kind: ContextDiagnosticKind::UndeclaredConsumption {
                        component,
                        key,
                        reader,
                    },
                });
            }
        }
    };

    // provision: owner 범위, orphan key, 값 타입, 정의 안 읽기 선언.
    for provision in context.provisions() {
        if let Err(error) = check_provision_scope(provision, model, library) {
            out.push(ContextDiagnostic {
                kind: ContextDiagnosticKind::ProvisionOutsideOwner {
                    provision: provision.id(),
                    error: error.to_string(),
                },
            });
        }
        let Some(key) = keys.key(provision.key()) else {
            out.push(ContextDiagnostic {
                kind: ContextDiagnosticKind::OrphanProvision {
                    provision: provision.id(),
                    key: provision.key(),
                },
            });
            continue;
        };
        let site = ContextSite::scene(provision.provider());
        check_type(
            &mut out,
            provision.id(),
            provision.value(),
            key.value_type(),
            &TypesAt {
                keys,
                inputs,
                site: &site,
            },
        );
        for read in provision.reads() {
            note_read(
                &mut out,
                provision_component(provision.owner()),
                read,
                provision.id(),
            );
        }
    }

    // Slot 공개 목록·requirement orphan.
    let slots: BTreeSet<StableId> = library
        .iter()
        .flat_map(|entry| entry.slots().map(|slot| slot.id.stable_id()))
        .collect();
    for (slot, list) in context.all_slot_keys() {
        for key in list {
            if !slots.contains(&slot) || keys.key(*key).is_none() {
                out.push(ContextDiagnostic {
                    kind: ContextDiagnosticKind::OrphanSlotKey { slot, key: *key },
                });
            }
        }
    }
    for (component, list) in context.all_requirements() {
        for key in list {
            if library.get(ComponentRef::new(component)).is_none() || keys.key(*key).is_none() {
                out.push(ContextDiagnostic {
                    kind: ContextDiagnosticKind::OrphanRequirement {
                        component,
                        key: *key,
                    },
                });
            }
        }
    }

    // 소비 binding.
    for consumer in consumers {
        let owner = library.owner_of(consumer.site.node).map(|c| c.id());
        let mut orphan = false;
        for key in consumer.reads() {
            if keys.key(key).is_none() {
                orphan = true;
                out.push(ContextDiagnostic {
                    kind: ContextDiagnosticKind::OrphanConsumption {
                        binding: consumer.binding,
                        key,
                    },
                });
                continue;
            }
            note_read(&mut out, owner, key, consumer.binding);
            if let Ok(resolution) = resolver.resolve(key, &consumer.site) {
                match &resolution.origin {
                    ContextOrigin::Default => out.push(ContextDiagnostic {
                        kind: ContextDiagnosticKind::DefaultResolved {
                            binding: consumer.binding,
                            key,
                        },
                    }),
                    ContextOrigin::Provision { provision, .. }
                        if !resolution.shadowed.is_empty() =>
                    {
                        out.push(ContextDiagnostic {
                            kind: ContextDiagnosticKind::Shadowing {
                                binding: consumer.binding,
                                key,
                                provision: *provision,
                                shadowed: resolution.shadowed.clone(),
                            },
                        })
                    }
                    ContextOrigin::Provision { .. } => {}
                }
            }
        }
        if !orphan {
            check_type(
                &mut out,
                consumer.binding,
                &consumer.source,
                consumer.expected,
                &TypesAt {
                    keys,
                    inputs,
                    site: &consumer.site,
                },
            );
        }
    }

    // 쓰지 않는 requirement.
    for (component, list) in context.all_requirements() {
        if library.get(ComponentRef::new(component)).is_none() {
            continue;
        }
        for key in list {
            if keys.key(*key).is_some() && !used.contains(&(component, *key)) {
                out.push(ContextDiagnostic {
                    kind: ContextDiagnosticKind::UnusedRequirement {
                        component,
                        key: *key,
                    },
                });
            }
        }
    }

    // provision 읽기 cycle: 모든 site의 provision 값을 평가해 순환을 찾습니다.
    let mut cycles: BTreeSet<Vec<StableId>> = BTreeSet::new();
    for site in all_sites(model, library) {
        for provision in context.provisions_at(site.node) {
            if keys.key(provision.key()).is_none() {
                continue;
            }
            if let Err(ContextEvalError::Cycle(path)) =
                evaluator.provision_value(provision, &site.instances)
            {
                cycles.insert(canonical_cycle(path));
            }
        }
    }
    for provisions in cycles {
        out.push(ContextDiagnostic {
            kind: ContextDiagnosticKind::ProvisionCycle { provisions },
        });
    }

    out.sort();
    out.dedup();
    out
}

/// cycle을 가장 작은 id부터 시작하는 회전으로 맞춥니다(마지막 원소는 첫 원소의 반복이라 뺍니다).
fn canonical_cycle(mut path: Vec<StableId>) -> Vec<StableId> {
    if let Some(last) = path.last().copied() {
        if let Some(start) = path.iter().position(|id| *id == last) {
            path = path[start..path.len() - 1].to_vec();
        }
    }
    if let Some((min, _)) = path.iter().enumerate().min_by_key(|(_, id)| **id) {
        path.rotate_left(min);
    }
    path
}

/// 오류 수준 진단만 모은 map(종류 코드 → 수). 명령 적용 전 검사에 씁니다.
pub fn error_counts(diagnostics: &[ContextDiagnostic]) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for diagnostic in diagnostics {
        if diagnostic.severity() == ContextSeverity::Error {
            *out.entry(diagnostic.kind.code()).or_default() += 1;
        }
    }
    out
}
