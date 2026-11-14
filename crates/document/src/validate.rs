//! 전체 document validator (VI.10.1).
//!
//! [`DocumentRecords`](영속 레코드)가 문서 불변식을 지키는지 저장 전·로드 후에 검증합니다.
//! 위반은 하나도 버리지 않고 **정확한 위치**([`ViolationLocation`])와 함께 모두
//! 보고합니다. 검사 항목:
//!
//! 1. **ID uniqueness**: 문서·페이지·View·node·modifier·path point/handle id가 문서
//!    안에서 전부 유일하고 NIL이 아님.
//! 2. **root count**: 각 View의 root는 존재하는 부모 없는 Frame이고, View끼리 공유하지
//!    않으며, 부모 없는 node는 반드시 어떤 View의 root임(root는 View당 정확히 하나).
//! 3. **parent consistency**: 부모가 존재하고, 자식을 가질 수 있는 종류(Frame·Fragment,
//!    §12.3·§12.4)이며, 부모 chain에 cycle이 없음.
//! 4. **OrderKey uniqueness**: 같은 부모의 형제끼리 OrderKey가 다름.
//! 5. **ref type**: 참조가 문서 안의 다른 종류 객체를 가리키지 않음(Frame clip은 Path
//!    node, modifier 참조는 node). 대상이 없는 참조는 위반이 아니라 미해결 진단(III.2.4)
//!    입니다.
//! 6. **modifier**: stack 소유 node 존재·유일, 빈 stack 레코드 금지, Fragment modifier
//!    금지(VI.9.6), 참조 cycle 금지(VI.9.5). 런타임 모델은 빈 stack을 보관하지 않으므로
//!    (stack 없음과 같음) 저장된 빈 stack 레코드는 비canonical입니다 — 받아들이면 load→save가
//!    byte 안정적이지 않고 residency dirty 판정이 영구히 dirty가 됩니다.
//! 7. **depth ≤ 64**: View root depth 0 기준 [`MAX_NODE_DEPTH`]까지 허용.
//!
//! 결정론: 검사는 레코드 순서와 StableId 순서로만 진행하므로 같은 입력은 같은 위반
//! 목록을 같은 순서로 냅니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{NodeRef, UnresolvedRef};

use crate::frame::{keys as frame_keys, Clipping};
use crate::modifier::{find_modifier_cycle, ModifierId, ModifierPayload};
use crate::modifier_metadata::keys as modifier_keys;
use crate::node_kind::NodeKind;
use crate::records::{DocumentRecords, NodePayload};

/// 허용 최대 node depth(View root = 0). 이 값을 넘는 node는 위반입니다.
pub const MAX_NODE_DEPTH: u32 = 64;

/// 위반이 발견된 정확한 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationLocation {
    /// 문서 정의.
    Document(StableId),
    /// Typography resource.
    Typography(StableId),
    /// 페이지 정의.
    Page(StableId),
    /// View 정의.
    View(StableId),
    /// node 레코드: `DocumentRecords::nodes` index와 StableId. id가 중복·NIL이어도
    /// index로 정확히 짚을 수 있습니다.
    Node {
        /// 레코드 index.
        index: usize,
        /// 레코드 StableId.
        id: StableId,
    },
    /// modifier stack 레코드: `DocumentRecords::modifier_stacks` index와 소유 node.
    ModifierStack {
        /// stack index.
        index: usize,
        /// 소유 node StableId.
        owner: StableId,
    },
    /// modifier 항목.
    Modifier {
        /// 소유 node.
        node: StableId,
        /// modifier id.
        modifier: ModifierId,
    },
    /// path point·handle.
    PathElement {
        /// Path node.
        node: StableId,
        /// point·handle id.
        element: StableId,
    },
    /// 속성 위치. `path` root는 node payload 속성이면 node, modifier 속성이면 modifier id입니다.
    Property {
        /// 속성을 소유한 node.
        node: StableId,
        /// 속성 경로.
        path: PropertyPath,
    },
}

impl fmt::Display for ViolationLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViolationLocation::Document(id) => write!(f, "문서 {id}"),
            ViolationLocation::Typography(id) => write!(f, "Typography {id}"),
            ViolationLocation::Page(id) => write!(f, "페이지 {id}"),
            ViolationLocation::View(id) => write!(f, "View {id}"),
            ViolationLocation::Node { index, id } => write!(f, "node {id}(#{index})"),
            ViolationLocation::ModifierStack { index, owner } => {
                write!(f, "node {owner}의 modifier stack(#{index})")
            }
            ViolationLocation::Modifier { node, modifier } => {
                write!(f, "node {node}의 modifier {}", modifier.stable_id())
            }
            ViolationLocation::PathElement { node, element } => {
                write!(f, "path node {node}의 point/handle {element}")
            }
            ViolationLocation::Property { node, path } => {
                let keys: Vec<String> = path
                    .keys()
                    .iter()
                    .map(|k| format!("{:#06x}", k.0))
                    .collect();
                write!(f, "node {node}의 속성 {}:{}", path.root(), keys.join("/"))
            }
        }
    }
}

/// 참조가 기대하는 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefExpectation {
    /// 종류를 가리지 않는 node.
    AnyNode,
    /// 특정 종류의 node.
    NodeOfKind(NodeKind),
}

/// 참조가 실제로 가리킨 문서 안 객체 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefTarget {
    /// node(종류 포함).
    Node(NodeKind),
    /// 문서 정의.
    Document,
    /// Typography resource.
    Typography,
    /// 페이지 정의.
    Page,
    /// View 정의.
    View,
    /// modifier 항목.
    Modifier,
    /// path point·handle.
    PathElement,
}

impl fmt::Display for RefTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefTarget::Node(kind) => write!(f, "{} node", kind.name()),
            RefTarget::Document => f.write_str("문서"),
            RefTarget::Typography => f.write_str("Typography"),
            RefTarget::Page => f.write_str("페이지"),
            RefTarget::View => f.write_str("View"),
            RefTarget::Modifier => f.write_str("modifier"),
            RefTarget::PathElement => f.write_str("path point/handle"),
        }
    }
}

/// 문서 불변식 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentViolation {
    /// NIL StableId는 영속 id로 쓸 수 없습니다.
    NilId {
        /// NIL id를 가진 객체.
        location: ViolationLocation,
    },
    /// 같은 StableId가 두 객체에 쓰였습니다.
    DuplicateId {
        /// 중복 id.
        id: StableId,
        /// 먼저 나온 객체.
        first: ViolationLocation,
        /// 나중에 나온 객체(보고 위치).
        second: ViolationLocation,
    },
    /// View의 root Frame node가 없습니다.
    MissingRootFrame {
        /// View.
        view: StableId,
        /// 지정된 root id.
        root: StableId,
    },
    /// View root가 Frame이 아닙니다.
    RootNotFrame {
        /// View.
        view: StableId,
        /// root node.
        root: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// View root가 부모를 가집니다.
    RootHasParent {
        /// View.
        view: StableId,
        /// root node.
        root: StableId,
        /// 지정된 부모.
        parent: StableId,
    },
    /// 두 View가 같은 root Frame을 지정했습니다.
    SharedRootFrame {
        /// 공유된 root.
        root: StableId,
        /// 먼저 지정한 View.
        first_view: StableId,
        /// 나중에 지정한 View(보고 위치).
        second_view: StableId,
    },
    /// 부모 없는 node가 어느 View의 root도 아닙니다(View당 root 둘 이상).
    UnownedRoot {
        /// 해당 node.
        node: ViolationLocation,
    },
    /// 부모 node가 없습니다.
    MissingParent {
        /// 자식 node.
        node: ViolationLocation,
        /// 없는 부모 id.
        parent: StableId,
    },
    /// 부모 chain이 cycle입니다.
    ParentCycle {
        /// cycle에서 StableId가 가장 작은 node(보고 위치).
        node: ViolationLocation,
        /// cycle 경로(보고 node부터 부모 방향, 같은 node로 끝남).
        cycle: Vec<StableId>,
    },
    /// 부모가 자식을 가질 수 없는 종류입니다(Frame·Fragment만 허용).
    ParentNotContainer {
        /// 자식 node.
        node: ViolationLocation,
        /// 부모 id.
        parent: StableId,
        /// 부모 종류.
        parent_kind: NodeKind,
    },
    /// 같은 부모의 형제끼리 OrderKey가 같습니다.
    DuplicateOrderKey {
        /// 부모 id.
        parent: StableId,
        /// 나중 형제(StableId 순, 보고 위치).
        node: ViolationLocation,
        /// 같은 key의 먼저 형제.
        other: StableId,
    },
    /// 참조가 기대와 다른 종류의 객체를 가리킵니다.
    RefTypeMismatch {
        /// 참조 위치.
        location: ViolationLocation,
        /// 참조 대상 id.
        target: StableId,
        /// 기대 대상.
        expected: RefExpectation,
        /// 실제 대상.
        found: RefTarget,
    },
    /// modifier stack의 소유 node가 없습니다.
    ModifierOwnerMissing {
        /// stack 위치.
        stack: ViolationLocation,
    },
    /// 한 node에 modifier stack이 둘 이상입니다.
    DuplicateModifierStack {
        /// 나중 stack 위치.
        stack: ViolationLocation,
    },
    /// 항목이 없는 modifier stack 레코드입니다(stack 없음으로 저장해야 함).
    EmptyModifierStack {
        /// stack 위치.
        stack: ViolationLocation,
    },
    /// Fragment가 modifier를 가집니다(VI.9.6).
    ModifierOnFragment {
        /// modifier 위치.
        location: ViolationLocation,
    },
    /// modifier 참조 graph cycle(VI.9.5).
    ModifierCycle {
        /// cycle 첫 node.
        node: ViolationLocation,
        /// cycle 경로(같은 node로 끝남).
        path: Vec<StableId>,
    },
    /// node depth가 [`MAX_NODE_DEPTH`]를 넘습니다.
    DepthExceeded {
        /// 처음으로 한도를 넘은 node.
        node: ViolationLocation,
        /// 그 node의 depth.
        depth: u32,
    },
}

impl DocumentViolation {
    /// 위반의 보고 위치.
    pub fn location(&self) -> ViolationLocation {
        match self {
            DocumentViolation::NilId { location }
            | DocumentViolation::RefTypeMismatch { location, .. }
            | DocumentViolation::ModifierOnFragment { location } => location.clone(),
            DocumentViolation::DuplicateId { second, .. } => second.clone(),
            DocumentViolation::MissingRootFrame { view, .. }
            | DocumentViolation::RootNotFrame { view, .. }
            | DocumentViolation::RootHasParent { view, .. } => ViolationLocation::View(*view),
            DocumentViolation::SharedRootFrame { second_view, .. } => {
                ViolationLocation::View(*second_view)
            }
            DocumentViolation::UnownedRoot { node }
            | DocumentViolation::MissingParent { node, .. }
            | DocumentViolation::ParentCycle { node, .. }
            | DocumentViolation::ParentNotContainer { node, .. }
            | DocumentViolation::DuplicateOrderKey { node, .. }
            | DocumentViolation::ModifierCycle { node, .. }
            | DocumentViolation::DepthExceeded { node, .. } => node.clone(),
            DocumentViolation::ModifierOwnerMissing { stack }
            | DocumentViolation::DuplicateModifierStack { stack }
            | DocumentViolation::EmptyModifierStack { stack } => stack.clone(),
        }
    }
}

fn join_ids(ids: &[StableId]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(" → ")
}

impl fmt::Display for DocumentViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentViolation::NilId { location } => {
                write!(f, "{location}: NIL id는 쓸 수 없습니다")
            }
            DocumentViolation::DuplicateId { id, first, second } => {
                write!(f, "{second}: id {id}가 {first}와 중복됩니다")
            }
            DocumentViolation::MissingRootFrame { view, root } => {
                write!(f, "View {view}: root Frame {root}이 없습니다")
            }
            DocumentViolation::RootNotFrame { view, root, kind } => write!(
                f,
                "View {view}: root {root}이 Frame이 아니라 {}입니다",
                kind.name()
            ),
            DocumentViolation::RootHasParent { view, root, parent } => {
                write!(f, "View {view}: root {root}이 부모 {parent}를 가집니다")
            }
            DocumentViolation::SharedRootFrame {
                root,
                first_view,
                second_view,
            } => write!(
                f,
                "View {second_view}: root {root}를 View {first_view}와 공유합니다"
            ),
            DocumentViolation::UnownedRoot { node } => {
                write!(f, "{node}: 부모가 없지만 어느 View의 root도 아닙니다")
            }
            DocumentViolation::MissingParent { node, parent } => {
                write!(f, "{node}: 부모 {parent}가 없습니다")
            }
            DocumentViolation::ParentCycle { node, cycle } => {
                write!(f, "{node}: 부모 chain cycle {}", join_ids(cycle))
            }
            DocumentViolation::ParentNotContainer {
                node,
                parent,
                parent_kind,
            } => write!(
                f,
                "{node}: 부모 {parent}({})는 자식을 가질 수 없습니다",
                parent_kind.name()
            ),
            DocumentViolation::DuplicateOrderKey {
                parent,
                node,
                other,
            } => write!(
                f,
                "{node}: 부모 {parent} 아래 형제 {other}와 OrderKey가 같습니다"
            ),
            DocumentViolation::RefTypeMismatch {
                location,
                target,
                expected,
                found,
            } => {
                let expected = match expected {
                    RefExpectation::AnyNode => "node".to_string(),
                    RefExpectation::NodeOfKind(kind) => format!("{} node", kind.name()),
                };
                write!(
                    f,
                    "{location}: 참조 {target}는 {expected}여야 하지만 {found}입니다"
                )
            }
            DocumentViolation::ModifierOwnerMissing { stack } => {
                write!(f, "{stack}: 소유 node가 없습니다")
            }
            DocumentViolation::DuplicateModifierStack { stack } => {
                write!(f, "{stack}: 같은 node의 stack이 이미 있습니다")
            }
            DocumentViolation::EmptyModifierStack { stack } => write!(
                f,
                "{stack}: 빈 stack 레코드는 저장할 수 없습니다(stack 없음으로 저장)"
            ),
            DocumentViolation::ModifierOnFragment { location } => {
                write!(f, "{location}: Fragment는 modifier를 가질 수 없습니다")
            }
            DocumentViolation::ModifierCycle { node, path } => {
                write!(f, "{node}: modifier 참조 cycle {}", join_ids(path))
            }
            DocumentViolation::DepthExceeded { node, depth } => write!(
                f,
                "{node}: depth {depth}가 최대 {MAX_NODE_DEPTH}를 넘습니다"
            ),
        }
    }
}

/// 검증 실패: 하나 이상의 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentValidationError {
    /// 발견 순서(결정론적)의 위반 목록. 비어 있지 않습니다.
    pub violations: Vec<DocumentViolation>,
}

impl fmt::Display for DocumentValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "문서 검증 실패({}건)", self.violations.len())?;
        if let Some(first) = self.violations.first() {
            write!(f, ": {first}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DocumentValidationError {}

/// 유효한 문서의 진단: 대상이 없는(또는 NIL) 참조. 문서를 거부하지 않습니다(III.2.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentDiagnostics {
    /// 미해결 참조(발견 순).
    pub unresolved: Vec<UnresolvedRef>,
}

/// node가 자식을 가질 수 있는 종류인지(§12.3 Frame Children, §12.4 Fragment 자식 순서).
pub fn can_have_children(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Frame | NodeKind::Fragment)
}

/// 문서 전체를 검증합니다. 위반이 없으면 미해결 참조 진단을 반환합니다.
pub fn validate_document(
    records: &DocumentRecords,
) -> Result<DocumentDiagnostics, DocumentValidationError> {
    let mut validator = Validator {
        records,
        violations: Vec::new(),
        unresolved: Vec::new(),
        node_index: BTreeMap::new(),
        targets: BTreeMap::new(),
    };
    validator.check_ids();
    validator.check_roots();
    validator.check_parents();
    validator.check_order_keys();
    validator.check_depth();
    validator.check_references();
    validator.check_modifier_stacks();
    if validator.violations.is_empty() {
        Ok(DocumentDiagnostics {
            unresolved: validator.unresolved,
        })
    } else {
        Err(DocumentValidationError {
            violations: validator.violations,
        })
    }
}

struct Validator<'a> {
    records: &'a DocumentRecords,
    violations: Vec<DocumentViolation>,
    unresolved: Vec<UnresolvedRef>,
    /// node id → 첫 레코드 index.
    node_index: BTreeMap<StableId, usize>,
    /// 모든 영속 id → (대상 종류, 첫 위치).
    targets: BTreeMap<StableId, (RefTarget, ViolationLocation)>,
}

impl Validator<'_> {
    fn node_location(&self, index: usize) -> ViolationLocation {
        ViolationLocation::Node {
            index,
            id: self.records.nodes[index].id,
        }
    }

    fn kind_of(&self, id: StableId) -> Option<NodeKind> {
        self.node_index
            .get(&id)
            .map(|&index| self.records.nodes[index].kind())
    }

    fn register(&mut self, id: StableId, target: RefTarget, location: ViolationLocation) {
        if id == StableId::NIL {
            self.violations.push(DocumentViolation::NilId { location });
            return;
        }
        if let Some((_, first)) = self.targets.get(&id) {
            self.violations.push(DocumentViolation::DuplicateId {
                id,
                first: first.clone(),
                second: location,
            });
            return;
        }
        self.targets.insert(id, (target, location));
    }

    // 1. ID uniqueness.
    fn check_ids(&mut self) {
        let records = self.records;
        let document = &records.definition;
        self.register(
            document.id,
            RefTarget::Document,
            ViolationLocation::Document(document.id),
        );
        for id in document.text_styles.typography.keys() {
            self.register(
                *id,
                RefTarget::Typography,
                ViolationLocation::Typography(*id),
            );
        }
        for page in &document.pages {
            self.register(page.id, RefTarget::Page, ViolationLocation::Page(page.id));
            for view in &page.views {
                self.register(view.id, RefTarget::View, ViolationLocation::View(view.id));
            }
        }
        for (index, node) in records.nodes.iter().enumerate() {
            let location = ViolationLocation::Node { index, id: node.id };
            self.register(node.id, RefTarget::Node(node.kind()), location);
            if node.id != StableId::NIL {
                self.node_index.entry(node.id).or_insert(index);
            }
        }
        for stack in &records.modifier_stacks {
            for entry in stack.entries() {
                self.register(
                    entry.id.stable_id(),
                    RefTarget::Modifier,
                    ViolationLocation::Modifier {
                        node: stack.node,
                        modifier: entry.id,
                    },
                );
            }
        }
        for node in &records.nodes {
            if let NodePayload::Path(path) = &node.payload {
                for ids in path.ids() {
                    let mut elements = Vec::new();
                    if let Some(point) = ids.point {
                        elements.push(point.0);
                    }
                    if let Some((h1, h2)) = ids.handles {
                        elements.extend([h1.0, h2.0]);
                    }
                    for element in elements {
                        self.register(
                            element,
                            RefTarget::PathElement,
                            ViolationLocation::PathElement {
                                node: node.id,
                                element,
                            },
                        );
                    }
                }
            }
        }
    }

    // 2. root count.
    fn check_roots(&mut self) {
        let records = self.records;
        let mut root_owner: BTreeMap<StableId, StableId> = BTreeMap::new();
        for view in records.views() {
            if let Some(first_view) = root_owner.get(&view.root_frame) {
                self.violations.push(DocumentViolation::SharedRootFrame {
                    root: view.root_frame,
                    first_view: *first_view,
                    second_view: view.id,
                });
                continue;
            }
            root_owner.insert(view.root_frame, view.id);
            let Some(&index) = self.node_index.get(&view.root_frame) else {
                self.violations.push(DocumentViolation::MissingRootFrame {
                    view: view.id,
                    root: view.root_frame,
                });
                continue;
            };
            let root = &records.nodes[index];
            if root.kind() != NodeKind::Frame {
                self.violations.push(DocumentViolation::RootNotFrame {
                    view: view.id,
                    root: root.id,
                    kind: root.kind(),
                });
            }
            if let Some(parent) = root.parent {
                self.violations.push(DocumentViolation::RootHasParent {
                    view: view.id,
                    root: root.id,
                    parent,
                });
            }
        }
        for (index, node) in records.nodes.iter().enumerate() {
            if node.parent.is_none() && !root_owner.contains_key(&node.id) {
                self.violations.push(DocumentViolation::UnownedRoot {
                    node: self.node_location(index),
                });
            }
        }
    }

    // 3. parent consistency.
    fn check_parents(&mut self) {
        let records = self.records;
        for (index, node) in records.nodes.iter().enumerate() {
            let Some(parent) = node.parent else {
                continue;
            };
            match self.kind_of(parent) {
                None => self.violations.push(DocumentViolation::MissingParent {
                    node: self.node_location(index),
                    parent,
                }),
                Some(parent_kind) if !can_have_children(parent_kind) => {
                    self.violations.push(DocumentViolation::ParentNotContainer {
                        node: self.node_location(index),
                        parent,
                        parent_kind,
                    })
                }
                Some(_) => {}
            }
        }

        // 부모 chain cycle: StableId 순으로 chain을 따라가며 3색 표시.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mark {
            Visiting,
            Done,
        }
        let parent_of: BTreeMap<StableId, StableId> = self
            .node_index
            .iter()
            .filter_map(|(id, &index)| records.nodes[index].parent.map(|p| (*id, p)))
            .collect();
        let mut marks: BTreeMap<StableId, Mark> = BTreeMap::new();
        let ids: Vec<StableId> = self.node_index.keys().copied().collect();
        for start in ids {
            if marks.contains_key(&start) {
                continue;
            }
            let mut chain: Vec<StableId> = Vec::new();
            let mut current = Some(start);
            while let Some(id) = current {
                match marks.get(&id) {
                    Some(Mark::Done) => break,
                    Some(Mark::Visiting) => {
                        let at = chain.iter().position(|c| *c == id).unwrap_or(0);
                        let members = &chain[at..];
                        let min_pos = members
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, id)| **id)
                            .map_or(0, |(pos, _)| pos);
                        let mut cycle: Vec<StableId> = members[min_pos..]
                            .iter()
                            .chain(members[..min_pos].iter())
                            .copied()
                            .collect();
                        let first = cycle[0];
                        cycle.push(first);
                        let index = self.node_index[&first];
                        self.violations.push(DocumentViolation::ParentCycle {
                            node: self.node_location(index),
                            cycle,
                        });
                        break;
                    }
                    None => {
                        marks.insert(id, Mark::Visiting);
                        chain.push(id);
                        current = parent_of
                            .get(&id)
                            .copied()
                            .filter(|p| self.node_index.contains_key(p));
                    }
                }
            }
            for id in chain {
                marks.insert(id, Mark::Done);
            }
        }
    }

    // 4. OrderKey uniqueness.
    fn check_order_keys(&mut self) {
        let records = self.records;
        let mut siblings: BTreeMap<StableId, Vec<usize>> = BTreeMap::new();
        for (index, node) in records.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                siblings.entry(parent).or_default().push(index);
            }
        }
        for (parent, mut children) in siblings {
            children.sort_by(|a, b| {
                let (na, nb) = (&records.nodes[*a], &records.nodes[*b]);
                (&na.order_key, na.id, *a).cmp(&(&nb.order_key, nb.id, *b))
            });
            for pair in children.windows(2) {
                let (earlier, later) = (&records.nodes[pair[0]], &records.nodes[pair[1]]);
                if earlier.order_key == later.order_key {
                    self.violations.push(DocumentViolation::DuplicateOrderKey {
                        parent,
                        node: self.node_location(pair[1]),
                        other: earlier.id,
                    });
                }
            }
        }
    }

    // 7. depth ≤ MAX_NODE_DEPTH.
    fn check_depth(&mut self) {
        let records = self.records;
        let mut children: BTreeMap<StableId, Vec<usize>> = BTreeMap::new();
        for (index, node) in records.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(index);
            }
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut exceeded: Vec<(usize, u32)> = Vec::new();
        let roots: Vec<usize> = records
            .views()
            .filter_map(|view| self.node_index.get(&view.root_frame).copied())
            .filter(|&index| records.nodes[index].parent.is_none())
            .collect();
        for root in roots {
            if !visited.insert(root) {
                continue;
            }
            let mut stack: Vec<(usize, u32)> = vec![(root, 0)];
            while let Some((index, depth)) = stack.pop() {
                let Some(kids) = children.get(&records.nodes[index].id) else {
                    continue;
                };
                for &child in kids {
                    // 중복 id가 자기 자신을 자식으로 만들어도 종료하도록 index로 방문 표시.
                    if !visited.insert(child) {
                        continue;
                    }
                    let child_depth = depth + 1;
                    if child_depth > MAX_NODE_DEPTH {
                        exceeded.push((child, child_depth));
                    } else {
                        stack.push((child, child_depth));
                    }
                }
            }
        }
        // 보고 순서를 레코드 순서로 고정합니다(DFS 순서와 무관).
        exceeded.sort_unstable();
        for (index, depth) in exceeded {
            self.violations.push(DocumentViolation::DepthExceeded {
                node: self.node_location(index),
                depth,
            });
        }
    }

    /// 참조 하나를 검사합니다. 대상이 없거나 NIL이면 미해결 진단을 남깁니다.
    fn check_ref(
        &mut self,
        target: NodeRef,
        expected: RefExpectation,
        location: ViolationLocation,
        path: PropertyPath,
    ) {
        let id = target.id();
        let found = if id == StableId::NIL {
            None
        } else {
            self.targets.get(&id).map(|(kind, _)| *kind)
        };
        match found {
            None => self.unresolved.push(target.unresolved_at(path)),
            Some(found) => {
                let ok = match (expected, found) {
                    (RefExpectation::AnyNode, RefTarget::Node(_)) => true,
                    (RefExpectation::NodeOfKind(want), RefTarget::Node(kind)) => want == kind,
                    _ => false,
                };
                if !ok {
                    self.violations.push(DocumentViolation::RefTypeMismatch {
                        location,
                        target: id,
                        expected,
                        found,
                    });
                }
            }
        }
    }

    // 5. ref type.
    fn check_references(&mut self) {
        let records = self.records;
        for node in &records.nodes {
            if let NodePayload::Frame(frame) = &node.payload {
                if let Clipping::Path(target) = frame.clipping {
                    let path = PropertyPath::with_keys(node.id, vec![frame_keys::CLIPPING]);
                    self.check_ref(
                        target,
                        RefExpectation::NodeOfKind(NodeKind::Path),
                        ViolationLocation::Property {
                            node: node.id,
                            path: path.clone(),
                        },
                        path,
                    );
                }
            }
        }
        for stack in &records.modifier_stacks {
            for entry in stack.entries() {
                let modifier = entry.id.stable_id();
                let mut refs: Vec<(PropertyPath, NodeRef)> = Vec::new();
                let at = |key: PropertyKey| PropertyPath::with_keys(modifier, vec![key]);
                match &entry.payload {
                    ModifierPayload::Boolean { operands, .. } => {
                        for (i, operand) in operands.iter().enumerate() {
                            refs.push((
                                at(modifier_keys::BOOLEAN_OPERANDS).with_index(i as u32),
                                *operand,
                            ));
                        }
                    }
                    ModifierPayload::Mask { source } => {
                        refs.push((at(modifier_keys::MASK_SOURCE), *source))
                    }
                    ModifierPayload::Clip { source } => {
                        refs.push((at(modifier_keys::CLIP_SOURCE), *source))
                    }
                    _ => {}
                }
                for (path, target) in refs {
                    self.check_ref(
                        target,
                        RefExpectation::AnyNode,
                        ViolationLocation::Property {
                            node: stack.node,
                            path: path.clone(),
                        },
                        path,
                    );
                }
            }
        }
    }

    // 6. modifier stack.
    fn check_modifier_stacks(&mut self) {
        let records = self.records;
        let mut owners: BTreeSet<StableId> = BTreeSet::new();
        for (index, stack) in records.modifier_stacks.iter().enumerate() {
            let location = ViolationLocation::ModifierStack {
                index,
                owner: stack.node,
            };
            if !owners.insert(stack.node) {
                self.violations
                    .push(DocumentViolation::DuplicateModifierStack { stack: location });
                continue;
            }
            if stack.entries().is_empty() {
                self.violations.push(DocumentViolation::EmptyModifierStack {
                    stack: location.clone(),
                });
            }
            match self.kind_of(stack.node) {
                None => self
                    .violations
                    .push(DocumentViolation::ModifierOwnerMissing { stack: location }),
                Some(NodeKind::Fragment) => {
                    for entry in stack.entries() {
                        self.violations.push(DocumentViolation::ModifierOnFragment {
                            location: ViolationLocation::Modifier {
                                node: stack.node,
                                modifier: entry.id,
                            },
                        });
                    }
                }
                Some(_) => {}
            }
        }
        if let Some(cycle) = find_modifier_cycle(&records.modifier_stacks) {
            let first = cycle.path[0];
            let node = match self.node_index.get(&first) {
                Some(&index) => self.node_location(index),
                None => ViolationLocation::ModifierStack {
                    index: records
                        .modifier_stacks
                        .iter()
                        .position(|s| s.node == first)
                        .unwrap_or(0),
                    owner: first,
                },
            };
            self.violations.push(DocumentViolation::ModifierCycle {
                node,
                path: cycle.path,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use crate::fragment::FragmentPayload;
    use crate::frame::{CornerRadius, FramePayload};
    use crate::image::ImagePayload;
    use crate::modifier::{BooleanOp, ModifierKind, ModifierStack};
    use crate::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
    use crate::records::NodeRecord;
    use crate::shape::{EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry};
    use crate::text::TextPayload;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::{AssetRef, ExpectedRefType};
    use stellar_core_types::value::{Rect, Size2, Vec2};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn mid(n: u64) -> ModifierId {
        ModifierId::new(sid(n))
    }

    const DOC: u64 = 1;
    const PAGE: u64 = 2;
    const VIEW_A: u64 = 3;
    const VIEW_B: u64 = 4;
    const ROOT_A: u64 = 10;
    const FRAGMENT: u64 = 11;
    const RECT: u64 = 12;
    const TEXT: u64 = 13;
    const PATH: u64 = 14;
    const IMAGE: u64 = 15;
    const INNER: u64 = 16;
    const ELLIPSE: u64 = 17;
    const ROOT_B: u64 = 20;
    const LINE: u64 = 21;

    fn node(id: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: format!("node-{id}"),
            payload,
        }
    }

    fn frame() -> NodePayload {
        NodePayload::Frame(FramePayload::default())
    }

    fn rect_payload() -> NodePayload {
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(10.0, 20.0).unwrap(),
            corner_radius: CornerRadius::ZERO,
        }))
    }

    fn path_payload() -> NodePayload {
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(Vec2::ZERO),
                PathCommand::LineTo(Vec2::new(5.0, 5.0).unwrap()),
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut next = 900u64;
        NodePayload::Path(IdentifiedPath::assign(data, &mut || {
            next += 1;
            sid(next)
        }))
    }

    /// 유효한 fixture: View 두 개, 모든 node 종류, modifier stack 두 개.
    ///
    /// ```text
    /// VIEW_A: ROOT_A(Frame) ─ FRAGMENT ─ ELLIPSE
    ///                       ├ RECT(Shape)   [Mask→TEXT, Boolean[ELLIPSE]]
    ///                       ├ TEXT, PATH, IMAGE
    ///                       └ INNER(Frame, clip→PATH) [Opacity]
    /// VIEW_B: ROOT_B(Frame) ─ LINE
    /// ```
    fn fixture() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(DOC), "문서".to_string());
        let mut page = PageDefinition::new(sid(PAGE), String::new());
        page.add_view(ViewDefinition::new(sid(VIEW_A), "A".into(), sid(ROOT_A)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(VIEW_B), "B".into(), sid(ROOT_B)))
            .unwrap();
        definition.add_page(page).unwrap();

        let inner = FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(PATH))),
            ..FramePayload::default()
        };
        let nodes = vec![
            node(ROOT_A, None, 0x10, frame()),
            node(
                FRAGMENT,
                Some(ROOT_A),
                0x10,
                NodePayload::Fragment(FragmentPayload),
            ),
            node(RECT, Some(ROOT_A), 0x20, rect_payload()),
            node(
                TEXT,
                Some(ROOT_A),
                0x30,
                NodePayload::Text(TextPayload::empty(0)),
            ),
            node(PATH, Some(ROOT_A), 0x40, path_payload()),
            node(
                IMAGE,
                Some(ROOT_A),
                0x50,
                NodePayload::Image(ImagePayload::new(
                    AssetRef::new(sid(5_000)),
                    Size2::new(4.0, 4.0).unwrap(),
                )),
            ),
            node(INNER, Some(ROOT_A), 0x60, NodePayload::Frame(inner)),
            node(
                ELLIPSE,
                Some(FRAGMENT),
                0x10,
                NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                    bounds: Rect::new(Vec2::ZERO, Size2::new(3.0, 3.0).unwrap()),
                })),
            ),
            node(ROOT_B, None, 0x10, frame()),
            node(
                LINE,
                Some(ROOT_B),
                0x10,
                NodePayload::Shape(ShapeGeometry::Line(LineParams {
                    start: Vec2::ZERO,
                    end: Vec2::new(1.0, 0.0).unwrap(),
                    cap: LineCap::Round,
                })),
            ),
        ];

        let mut rect_stack = ModifierStack::new(sid(RECT));
        rect_stack.push_back(mid(100), ModifierKind::Mask).unwrap();
        rect_stack
            .set_payload(
                mid(100),
                ModifierPayload::Mask {
                    source: NodeRef::new(sid(TEXT)),
                },
            )
            .unwrap();
        rect_stack
            .push_back(mid(101), ModifierKind::Boolean)
            .unwrap();
        rect_stack
            .set_payload(
                mid(101),
                ModifierPayload::Boolean {
                    op: BooleanOp::Subtract,
                    operands: vec![NodeRef::new(sid(ELLIPSE))],
                },
            )
            .unwrap();
        let mut inner_stack = ModifierStack::new(sid(INNER));
        inner_stack
            .push_back(mid(102), ModifierKind::Opacity)
            .unwrap();

        DocumentRecords {
            definition,
            nodes,
            modifier_stacks: vec![rect_stack, inner_stack],
        }
    }

    fn index_of(records: &DocumentRecords, id: u64) -> usize {
        records.nodes.iter().position(|n| n.id == sid(id)).unwrap()
    }

    fn loc(records: &DocumentRecords, id: u64) -> ViolationLocation {
        ViolationLocation::Node {
            index: index_of(records, id),
            id: sid(id),
        }
    }

    fn violations(records: &DocumentRecords) -> Vec<DocumentViolation> {
        validate_document(records)
            .expect_err("위반이 있어야 함")
            .violations
    }

    fn node_mut(records: &mut DocumentRecords, id: u64) -> &mut NodeRecord {
        let index = index_of(records, id);
        &mut records.nodes[index]
    }

    #[test]
    fn valid_fixture_passes_without_diagnostics() {
        let diagnostics = validate_document(&fixture()).unwrap();
        assert!(diagnostics.unresolved.is_empty());
    }

    #[test]
    fn missing_reference_targets_are_diagnostics_not_violations() {
        let mut records = fixture();
        // Mask 대상과 clip 대상을 없는 id로.
        records.modifier_stacks[0]
            .set_payload(
                mid(100),
                ModifierPayload::Mask {
                    source: NodeRef::new(sid(7_777)),
                },
            )
            .unwrap();
        if let NodePayload::Frame(inner) = &mut node_mut(&mut records, INNER).payload {
            inner.clipping = Clipping::Path(NodeRef::new(StableId::NIL));
        }
        let diagnostics = validate_document(&records).unwrap();
        assert_eq!(diagnostics.unresolved.len(), 2);
        let clip = &diagnostics.unresolved[0];
        assert_eq!(clip.original_id(), StableId::NIL);
        assert_eq!(
            clip.location(),
            &PropertyPath::with_keys(sid(INNER), vec![frame_keys::CLIPPING])
        );
        let mask = &diagnostics.unresolved[1];
        assert_eq!(mask.original_id(), sid(7_777));
        assert_eq!(mask.expected(), ExpectedRefType::Node);
        assert_eq!(
            mask.location(),
            &PropertyPath::with_keys(sid(100), vec![modifier_keys::MASK_SOURCE])
        );
    }

    #[test]
    fn duplicate_and_nil_ids_report_exact_locations() {
        // 검증 기준(VI.10.1): ID uniqueness 위반 위치.
        let mut records = fixture();
        node_mut(&mut records, LINE).id = sid(TEXT); // node끼리 중복.
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::DuplicateId {
                id: sid(TEXT),
                first: ViolationLocation::Node {
                    index: index_of(&records, TEXT),
                    id: sid(TEXT)
                },
                second: ViolationLocation::Node {
                    index: records.nodes.len() - 1,
                    id: sid(TEXT)
                },
            }]
        );

        // View id와 node id 충돌, modifier id와 path point id 충돌.
        let mut records = fixture();
        records.definition.pages[0].views[1].id = sid(LINE);
        records.modifier_stacks[1] = {
            let mut stack = ModifierStack::new(sid(INNER));
            stack.push_back(mid(901), ModifierKind::Opacity).unwrap();
            stack
        };
        let found = violations(&records);
        assert_eq!(
            found,
            vec![
                DocumentViolation::DuplicateId {
                    id: sid(LINE),
                    first: ViolationLocation::View(sid(LINE)),
                    second: loc(&records, LINE),
                },
                DocumentViolation::DuplicateId {
                    id: sid(901),
                    first: ViolationLocation::Modifier {
                        node: sid(INNER),
                        modifier: mid(901)
                    },
                    second: ViolationLocation::PathElement {
                        node: sid(PATH),
                        element: sid(901)
                    },
                },
            ]
        );
        assert_eq!(found[1].location(), found[1].clone().location());

        // NIL node id.
        let mut records = fixture();
        node_mut(&mut records, IMAGE).id = StableId::NIL;
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::NilId {
                location: ViolationLocation::Node {
                    index: 5,
                    id: StableId::NIL
                }
            }]
        );
    }

    #[test]
    fn root_count_violations_report_view_and_node() {
        // 검증 기준(VI.10.1): root count 위반 위치.
        // root 지정이 없는 id → root 없음 + 원래 root는 소유 View 없는 root.
        let mut records = fixture();
        records.definition.pages[0].views[1].root_frame = sid(8_888);
        assert_eq!(
            violations(&records),
            vec![
                DocumentViolation::MissingRootFrame {
                    view: sid(VIEW_B),
                    root: sid(8_888)
                },
                DocumentViolation::UnownedRoot {
                    node: loc(&records, ROOT_B)
                },
            ]
        );

        // 부모 없는 node 추가 → View A에 root가 둘.
        let mut records = fixture();
        records.nodes.push(node(30, None, 0x10, frame()));
        let found = violations(&records);
        assert_eq!(
            found,
            vec![DocumentViolation::UnownedRoot {
                node: loc(&records, 30)
            }]
        );
        assert_eq!(found[0].location(), loc(&records, 30));

        // root가 Frame이 아님.
        let mut records = fixture();
        node_mut(&mut records, ROOT_B).payload = NodePayload::Fragment(FragmentPayload);
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::RootNotFrame {
                view: sid(VIEW_B),
                root: sid(ROOT_B),
                kind: NodeKind::Fragment
            }]
        );

        // root가 부모를 가짐(다른 View tree 안으로).
        let mut records = fixture();
        node_mut(&mut records, ROOT_B).parent = Some(sid(ROOT_A));
        node_mut(&mut records, ROOT_B).order_key = OrderKey::from_bytes(vec![0x70]).unwrap();
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::RootHasParent {
                view: sid(VIEW_B),
                root: sid(ROOT_B),
                parent: sid(ROOT_A)
            }]
        );

        // 두 View가 같은 root 공유.
        let mut records = fixture();
        records.definition.pages[0].views[1].root_frame = sid(ROOT_A);
        let found = violations(&records);
        assert_eq!(
            found,
            vec![
                DocumentViolation::SharedRootFrame {
                    root: sid(ROOT_A),
                    first_view: sid(VIEW_A),
                    second_view: sid(VIEW_B)
                },
                DocumentViolation::UnownedRoot {
                    node: loc(&records, ROOT_B)
                },
            ]
        );
        assert_eq!(found[0].location(), ViolationLocation::View(sid(VIEW_B)));
    }

    #[test]
    fn parent_consistency_violations_report_child_node() {
        // 검증 기준(VI.10.1): parent consistency 위반 위치.
        let mut records = fixture();
        node_mut(&mut records, IMAGE).parent = Some(sid(6_666));
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::MissingParent {
                node: loc(&records, IMAGE),
                parent: sid(6_666)
            }]
        );

        // leaf(Text)를 부모로.
        let mut records = fixture();
        node_mut(&mut records, IMAGE).parent = Some(sid(TEXT));
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::ParentNotContainer {
                node: loc(&records, IMAGE),
                parent: sid(TEXT),
                parent_kind: NodeKind::Text
            }]
        );

        // 2-cycle: INNER ↔ FRAGMENT(둘 다 container).
        let mut records = fixture();
        node_mut(&mut records, FRAGMENT).parent = Some(sid(INNER));
        node_mut(&mut records, INNER).parent = Some(sid(FRAGMENT));
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::ParentCycle {
                node: loc(&records, FRAGMENT),
                cycle: vec![sid(FRAGMENT), sid(INNER), sid(FRAGMENT)],
            }]
        );

        // self parent.
        let mut records = fixture();
        node_mut(&mut records, INNER).parent = Some(sid(INNER));
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::ParentCycle {
                node: loc(&records, INNER),
                cycle: vec![sid(INNER), sid(INNER)],
            }]
        );
    }

    #[test]
    fn duplicate_sibling_order_key_reports_later_sibling() {
        // 검증 기준(VI.10.1): OrderKey uniqueness 위반 위치.
        let mut records = fixture();
        node_mut(&mut records, IMAGE).order_key = OrderKey::from_bytes(vec![0x30]).unwrap();
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::DuplicateOrderKey {
                parent: sid(ROOT_A),
                node: loc(&records, IMAGE),
                other: sid(TEXT),
            }]
        );
        // 다른 부모 아래 같은 key는 허용(ELLIPSE 0x10, FRAGMENT 0x10).
        assert!(validate_document(&fixture()).is_ok());
    }

    #[test]
    fn ref_type_mismatch_reports_property_location() {
        // 검증 기준(VI.10.1): ref type 위반 위치.
        let mut records = fixture();
        if let NodePayload::Frame(inner) = &mut node_mut(&mut records, INNER).payload {
            inner.clipping = Clipping::Path(NodeRef::new(sid(TEXT)));
        }
        records.modifier_stacks[0]
            .set_payload(
                mid(101),
                ModifierPayload::Boolean {
                    op: BooleanOp::Union,
                    operands: vec![NodeRef::new(sid(ELLIPSE)), NodeRef::new(sid(VIEW_A))],
                },
            )
            .unwrap();
        assert_eq!(
            violations(&records),
            vec![
                DocumentViolation::RefTypeMismatch {
                    location: ViolationLocation::Property {
                        node: sid(INNER),
                        path: PropertyPath::with_keys(sid(INNER), vec![frame_keys::CLIPPING]),
                    },
                    target: sid(TEXT),
                    expected: RefExpectation::NodeOfKind(NodeKind::Path),
                    found: RefTarget::Node(NodeKind::Text),
                },
                DocumentViolation::RefTypeMismatch {
                    location: ViolationLocation::Property {
                        node: sid(RECT),
                        path: PropertyPath::with_keys(
                            sid(101),
                            vec![modifier_keys::BOOLEAN_OPERANDS]
                        )
                        .with_index(1),
                    },
                    target: sid(VIEW_A),
                    expected: RefExpectation::AnyNode,
                    found: RefTarget::View,
                },
            ]
        );
    }

    #[test]
    fn modifier_cycle_and_fragment_modifier_report_locations() {
        // 검증 기준(VI.10.1): modifier cycle 위반 위치.
        let mut records = fixture();
        let mut text_stack = ModifierStack::new(sid(TEXT));
        text_stack.push_back(mid(103), ModifierKind::Clip).unwrap();
        text_stack
            .set_payload(
                mid(103),
                ModifierPayload::Clip {
                    source: NodeRef::new(sid(RECT)),
                },
            )
            .unwrap();
        records.modifier_stacks.push(text_stack);
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::ModifierCycle {
                node: loc(&records, RECT),
                path: vec![sid(RECT), sid(TEXT), sid(RECT)],
            }]
        );

        // Fragment가 stack을 가짐(VI.9.6) — 항목마다 위치 보고.
        let mut records = fixture();
        let mut fragment_stack = ModifierStack::new(sid(FRAGMENT));
        fragment_stack
            .push_back(mid(104), ModifierKind::Opacity)
            .unwrap();
        fragment_stack
            .push_back(mid(105), ModifierKind::Blur)
            .unwrap();
        records.modifier_stacks.push(fragment_stack);
        assert_eq!(
            violations(&records),
            vec![
                DocumentViolation::ModifierOnFragment {
                    location: ViolationLocation::Modifier {
                        node: sid(FRAGMENT),
                        modifier: mid(104)
                    }
                },
                DocumentViolation::ModifierOnFragment {
                    location: ViolationLocation::Modifier {
                        node: sid(FRAGMENT),
                        modifier: mid(105)
                    }
                },
            ]
        );
        // 빈 stack 레코드는 Fragment에서도 modifier 위반이 아니라 빈 레코드 위반입니다.
        let mut records = fixture();
        records
            .modifier_stacks
            .push(ModifierStack::new(sid(FRAGMENT)));
        assert_eq!(
            violations(&records),
            vec![DocumentViolation::EmptyModifierStack {
                stack: ViolationLocation::ModifierStack {
                    index: 2,
                    owner: sid(FRAGMENT)
                }
            }]
        );
    }

    #[test]
    fn empty_modifier_stack_record_is_rejected() {
        // 런타임 모델은 빈 stack을 보관하지 않으므로 저장된 빈 stack 레코드는 거부합니다
        // (load→save byte 안정성).
        let mut records = fixture();
        records.modifier_stacks.push(ModifierStack::new(sid(TEXT)));
        let found = violations(&records);
        assert_eq!(
            found,
            vec![DocumentViolation::EmptyModifierStack {
                stack: ViolationLocation::ModifierStack {
                    index: 2,
                    owner: sid(TEXT)
                }
            }]
        );
        assert!(found[0].to_string().contains("빈 stack"));
        // 소유 node도 없으면 두 위반을 모두 보고합니다.
        let mut records = fixture();
        records.modifier_stacks.push(ModifierStack::new(sid(4_445)));
        let location = ViolationLocation::ModifierStack {
            index: 2,
            owner: sid(4_445),
        };
        assert_eq!(
            violations(&records),
            vec![
                DocumentViolation::EmptyModifierStack {
                    stack: location.clone()
                },
                DocumentViolation::ModifierOwnerMissing { stack: location },
            ]
        );
    }

    #[test]
    fn modifier_stack_owner_violations() {
        let mut records = fixture();
        // 소유 node 없는 stack(빈 레코드 위반과 섞이지 않게 항목 하나).
        let mut orphan = ModifierStack::new(sid(4_444));
        orphan.push_back(mid(110), ModifierKind::Opacity).unwrap();
        records.modifier_stacks.push(orphan);
        records.modifier_stacks.push(ModifierStack::new(sid(RECT)));
        assert_eq!(
            violations(&records),
            vec![
                DocumentViolation::ModifierOwnerMissing {
                    stack: ViolationLocation::ModifierStack {
                        index: 2,
                        owner: sid(4_444)
                    }
                },
                DocumentViolation::DuplicateModifierStack {
                    stack: ViolationLocation::ModifierStack {
                        index: 3,
                        owner: sid(RECT)
                    }
                },
            ]
        );
    }

    /// ROOT_B 아래로 `levels` 단계 Frame chain을 붙입니다(마지막 depth = levels).
    fn with_chain(levels: u64) -> DocumentRecords {
        let mut records = fixture();
        let mut parent = ROOT_B;
        for level in 1..=levels {
            let id = 50_000 + level;
            records.nodes.push(node(id, Some(parent), 0x80, frame()));
            parent = id;
        }
        records
    }

    #[test]
    fn depth_limit_reports_first_node_over_limit() {
        // 검증 기준(VI.10.1): depth≤64 위반 위치.
        assert!(validate_document(&with_chain(64)).is_ok(), "depth 64 허용");
        let records = with_chain(66);
        let found = violations(&records);
        // depth 65인 첫 node만 보고(그 아래 node는 조상 위반에 포함).
        assert_eq!(
            found,
            vec![DocumentViolation::DepthExceeded {
                node: loc(&records, 50_065),
                depth: 65
            }]
        );
        assert!(found[0].to_string().contains("64"));
    }

    #[test]
    fn duplicate_ids_do_not_loop_depth_or_cycle_checks() {
        // 같은 id 자식이 자기 id를 부모로 지정해도 검사는 종료합니다.
        let mut records = fixture();
        records.nodes.push(node(INNER, Some(INNER), 0x90, frame()));
        let found = violations(&records);
        assert!(matches!(found[0], DocumentViolation::DuplicateId { .. }));
    }

    #[test]
    fn violations_are_deterministic_and_displayable() {
        let mut records = fixture();
        node_mut(&mut records, IMAGE).parent = Some(sid(6_666));
        records.nodes.push(node(31, None, 0x10, frame()));
        let first = violations(&records);
        assert_eq!(first, violations(&records));
        let err = validate_document(&records).unwrap_err();
        assert!(err.to_string().starts_with("문서 검증 실패(2건)"));
        for violation in &err.violations {
            assert!(!violation.to_string().is_empty());
        }
    }
}
