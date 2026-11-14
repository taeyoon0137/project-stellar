//! resolved View → [`CodegenIr`] 변환 (XXII.1.1). 규칙은 [`crate::ir`] 모듈 문서를 따릅니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_accessibility::node::{AccessibilityBinding, AccessibilityNode, ActionTarget};
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_core_types::ids::StableId;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::overrides::resolve_for_profile;
use stellar_target_profile::registry::RegistryCatalog;
use stellar_target_profile::spec::TargetProfile;
use stellar_target_profile::validate::{validate_profiles, ProfileOutcome, ViewNode};

use super::{
    CodegenIr, CodegenSources, IrAccessibilityEntry, IrError, IrLayout, IrNode, IrProperty,
    IrState, IrTarget, IrToken, IrTree, MAX_IR_NODES,
};

/// View 구조(문서 순서 index 기준).
struct ViewTree {
    /// StableId → view.nodes index.
    index: BTreeMap<StableId, usize>,
    /// view.nodes index → 자식 view.nodes index(문서 순서).
    children: Vec<Vec<usize>>,
    /// root view.nodes index.
    root: usize,
}

fn view_tree(nodes: &[ViewNode]) -> Result<ViewTree, IrError> {
    if nodes.is_empty() {
        return Err(IrError::EmptyView);
    }
    if nodes.len() > MAX_IR_NODES {
        return Err(IrError::TooManyNodes);
    }
    let mut index = BTreeMap::new();
    for (position, node) in nodes.iter().enumerate() {
        if index.insert(node.id, position).is_some() {
            return Err(IrError::DuplicateNode(node.id));
        }
    }
    let mut children = vec![Vec::new(); nodes.len()];
    let mut root: Option<usize> = None;
    for (position, node) in nodes.iter().enumerate() {
        match node.parent {
            None => match root {
                None => root = Some(position),
                Some(first) => {
                    return Err(IrError::MultipleRoots {
                        first: nodes[first].id,
                        second: node.id,
                    })
                }
            },
            Some(parent) => {
                let Some(&parent_position) = index.get(&parent) else {
                    return Err(IrError::UnknownParent {
                        node: node.id,
                        parent,
                    });
                };
                children[parent_position].push(position);
            }
        }
    }
    let root = root.ok_or(IrError::MissingRoot)?;
    // 모든 node가 부모 하나를 가지므로 root에서 닿는 node는 tree이고, 닿지 않는 node는 순환에 걸린 것입니다.
    let mut reached = vec![false; nodes.len()];
    let mut stack = vec![root];
    while let Some(position) = stack.pop() {
        reached[position] = true;
        stack.extend(children[position].iter().copied());
    }
    if let Some(position) = reached.iter().position(|r| !r) {
        return Err(IrError::Unreachable(nodes[position].id));
    }
    Ok(ViewTree {
        index,
        children,
        root,
    })
}

/// IR을 만듭니다.
pub fn build_ir(
    sources: &CodegenSources<'_>,
    profile: &TargetProfile,
    catalog: &RegistryCatalog,
) -> Result<CodegenIr, IrError> {
    let view = sources.view;
    let tree = view_tree(&view.nodes)?;

    // capability 진단(registry pin 해석 실패는 IR을 만들지 않음).
    let report = validate_profiles(
        view,
        sources.capability_map,
        std::slice::from_ref(profile),
        catalog,
        sources.variants,
    );
    // profile 하나를 넣었으므로 보고도 하나입니다. 보고가 없으면 검사하지 못한 것으로 봅니다.
    let outcome = report
        .profiles
        .into_iter()
        .next()
        .map(|report| report.outcome)
        .unwrap_or(ProfileOutcome::RegistryUnavailable {
            pinned: profile.registry_version,
            latest: catalog.latest().version(),
        });
    let (registry_version, findings) = match outcome {
        ProfileOutcome::Validated {
            registry_version,
            findings,
        } => (registry_version, findings),
        ProfileOutcome::RegistryUnavailable { pinned, latest } => {
            return Err(IrError::RegistryUnavailable { pinned, latest })
        }
    };

    // profile 해석과 제외 판정.
    let resolved = resolve_for_profile(&view.common, &view.overrides, profile, sources.variants);
    let parent_of = |id: StableId| {
        tree.index
            .get(&id)
            .and_then(|position| view.nodes[*position].parent)
    };
    let excluded = resolved.excluded_among(view.nodes.iter().map(|n| n.id), parent_of);

    let (mut nodes, ir_index) = build_nodes(&view.nodes, &tree, &excluded);
    // root는 제외할 수 없습니다(제외하면 빈 IR) — 제외된 root는 빈 View와 같게 봅니다.
    if nodes.is_empty() {
        return Err(IrError::EmptyView);
    }

    attach_layout(sources, &tree, &excluded, &mut nodes, &ir_index)?;

    // 속성 값.
    for (path, value) in resolved.values {
        let root = path.root();
        match ir_index.get(&root) {
            Some(&index) => {
                let capability = sources.capability_map.capability_of(&path);
                nodes[index as usize].properties.push(IrProperty {
                    path,
                    value: value.value,
                    origin: value.origin,
                    token: None,
                    capability,
                });
            }
            None if tree.index.contains_key(&root) => {}
            None => return Err(IrError::PropertyOutsideView(root)),
        }
    }
    // resolved.values는 path 오름차순이므로 node별 목록도 path 오름차순입니다.

    // token.
    let tokens = declared_tokens(sources)?;
    for (path, token) in sources.token_bindings {
        let root = path.root();
        if tokens.binary_search_by(|t| t.id.cmp(token)).is_err() {
            return Err(IrError::UnknownToken {
                node: root,
                token: *token,
            });
        }
        match ir_index.get(&root) {
            Some(&index) => {
                let node = &mut nodes[index as usize];
                let Ok(position) = node.properties.binary_search_by(|p| p.path.cmp(path)) else {
                    return Err(IrError::TokenBindingWithoutValue(root));
                };
                node.properties[position].token = Some(*token);
            }
            None if tree.index.contains_key(&root) => {}
            None => return Err(IrError::PropertyOutsideView(root)),
        }
    }

    // 텍스트.
    for (id, text) in sources.texts {
        match ir_index.get(id) {
            Some(&index) => {
                let node = &mut nodes[index as usize];
                if node.element != ElementKind::TEXT {
                    return Err(IrError::TextOnNonTextNode(*id));
                }
                node.text = Some(text.clone());
            }
            None if tree.index.contains_key(id) => {}
            None => return Err(IrError::TextOutsideView(*id)),
        }
    }

    // capability 진단(validator가 제외 node를 이미 건너뜀).
    for finding in findings {
        if let Some(&index) = ir_index.get(&finding.node) {
            nodes[index as usize].capabilities.push(finding);
        }
    }

    let states = declared_states(sources)?;
    let accessibility = attach_accessibility(sources.accessibility, &tree, &mut nodes, &ir_index)?;
    for entry in &accessibility {
        check_state_references(&entry.semantics, &states)?;
    }

    Ok(CodegenIr {
        target: IrTarget {
            profile: profile.clone(),
            registry_version,
        },
        tree: IrTree {
            nodes,
            accessibility,
            states,
            tokens,
            index: ir_index,
        },
    })
}

/// Fragment를 펼친 IR 자식(StableId, 문서 순서).
fn flattened_children(nodes: &[IrNode], index: u32) -> Vec<StableId> {
    let mut out = Vec::new();
    let mut stack: Vec<u32> = nodes[index as usize]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(child) = stack.pop() {
        let node = &nodes[child as usize];
        if node.element == ElementKind::FRAGMENT {
            stack.extend(node.children.iter().rev().copied());
        } else {
            out.push(node.id);
        }
    }
    out
}

fn attach_layout(
    sources: &CodegenSources<'_>,
    tree: &ViewTree,
    excluded: &BTreeSet<StableId>,
    nodes: &mut [IrNode],
    ir_index: &BTreeMap<StableId, u32>,
) -> Result<(), IrError> {
    let layout = sources.layout;
    layout.validate().map_err(IrError::InvalidLayout)?;
    let view_root = nodes[0].id;
    if layout.root != view_root {
        return Err(IrError::LayoutRootMismatch {
            view: view_root,
            layout: layout.root,
        });
    }
    let by_id: BTreeMap<StableId, &stellar_layout::input::LayoutNode> =
        layout.nodes.iter().map(|n| (n.id, n)).collect();
    // 먼저 모든 node의 layout 존재를 확인하고(누락을 자식 순서 불일치보다 먼저 보고), 그다음 자식 순서를 비교합니다.
    if let Some(node) = nodes
        .iter()
        .find(|n| n.element != ElementKind::FRAGMENT && !by_id.contains_key(&n.id))
    {
        return Err(IrError::MissingLayout(node.id));
    }
    for index in 0..nodes.len() {
        if nodes[index].element == ElementKind::FRAGMENT {
            continue;
        }
        let id = nodes[index].id;
        let Some(layout_node) = by_id.get(&id) else {
            return Err(IrError::MissingLayout(id));
        };
        let mut ordered: Vec<(&stellar_core_types::order_key::OrderKey, StableId)> = layout_node
            .children
            .iter()
            .map(|child| (&child.order_key, child.id))
            .collect();
        ordered.sort();
        let mut layout_children = Vec::with_capacity(ordered.len());
        for (_, child) in ordered {
            if excluded.contains(&child) {
                continue;
            }
            if !tree.index.contains_key(&child) || !ir_index.contains_key(&child) {
                // View에 없거나 제외 subtree 안의 node(제외 node의 하위)를 layout 자식으로 가짐.
                return Err(IrError::LayoutChildrenMismatch(id));
            }
            layout_children.push(child);
        }
        if layout_children != flattened_children(nodes, index as u32) {
            return Err(IrError::LayoutChildrenMismatch(id));
        }
        nodes[index].layout = Some(IrLayout::from_node(layout_node));
    }
    Ok(())
}

fn declared_states(sources: &CodegenSources<'_>) -> Result<Vec<IrState>, IrError> {
    let mut states: Vec<IrState> = Vec::with_capacity(sources.states.len());
    for decl in sources.states {
        if decl.name.trim().is_empty() {
            return Err(IrError::EmptyStateName(decl.id));
        }
        states.push(IrState {
            id: decl.id,
            name: decl.name.clone(),
            initial: decl.initial.clone(),
        });
    }
    states.sort_by_key(|state| state.id);
    if let Some(pair) = states.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(IrError::DuplicateState(pair[0].id));
    }
    Ok(states)
}

fn declared_tokens(sources: &CodegenSources<'_>) -> Result<Vec<IrToken>, IrError> {
    let mut tokens: Vec<IrToken> = Vec::with_capacity(sources.tokens.len());
    for decl in sources.tokens {
        if decl.name.trim().is_empty() {
            return Err(IrError::EmptyTokenName(decl.id));
        }
        tokens.push(IrToken {
            id: decl.id,
            name: decl.name.clone(),
            value: decl.value.clone(),
        });
    }
    tokens.sort_by_key(|token| token.id);
    if let Some(pair) = tokens.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(IrError::DuplicateToken(pair[0].id));
    }
    Ok(tokens)
}

fn attach_accessibility(
    snapshot: &AccessibilitySnapshot,
    tree: &ViewTree,
    nodes: &mut [IrNode],
    ir_index: &BTreeMap<StableId, u32>,
) -> Result<Vec<IrAccessibilityEntry>, IrError> {
    // snapshot entry index → 그 entry 또는 가장 가까운 남은 조상의 IR entry index.
    let mut effective: Vec<Option<u32>> = Vec::with_capacity(snapshot.len());
    let mut entries: Vec<IrAccessibilityEntry> = Vec::new();
    for entry in snapshot.entries() {
        let parent_effective = entry
            .parent
            .and_then(|parent| effective.get(parent as usize).copied().flatten());
        let source = entry.node.source;
        match ir_index.get(&source) {
            Some(&node) => {
                let index = entries.len() as u32;
                let depth = parent_effective
                    .map(|p| entries[p as usize].depth + 1)
                    .unwrap_or(0);
                let mut semantics = entry.node.clone();
                semantics.reading_order = None;
                entries.push(IrAccessibilityEntry {
                    node,
                    parent: parent_effective,
                    depth,
                    semantics,
                });
                nodes[node as usize].semantics = Some(index);
                effective.push(Some(index));
            }
            None if tree.index.contains_key(&source) => effective.push(parent_effective),
            None => return Err(IrError::AccessibilityOutsideView(source)),
        }
    }
    Ok(entries)
}

fn binding_states(binding: &AccessibilityBinding) -> Option<StableId> {
    match binding {
        AccessibilityBinding::State(id) => Some(*id),
        AccessibilityBinding::SelectionMatches { selection, .. } => Some(*selection),
        _ => None,
    }
}

fn check_state_references(
    semantics: &AccessibilityNode,
    states: &[IrState],
) -> Result<(), IrError> {
    let declared = |id: StableId| states.binary_search_by(|s| s.id.cmp(&id)).is_ok();
    let bindings = semantics
        .label
        .iter()
        .chain(semantics.value.iter())
        .chain(semantics.states.values());
    let referenced =
        bindings
            .filter_map(binding_states)
            .chain(
                semantics
                    .actions
                    .values()
                    .filter_map(|target| match target {
                        ActionTarget::State(id) => Some(*id),
                        _ => None,
                    }),
            );
    for state in referenced {
        if !declared(state) {
            return Err(IrError::UnknownState {
                node: semantics.source,
                state,
            });
        }
    }
    Ok(())
}

/// 저장된 정적 구조·텍스트·접근성만 투영합니다. layout, profile, capability, 상태나 token은
/// 받거나 추정하지 않습니다. 전체 코드 생성용 `CodegenIr`로 사용할 수 없습니다.
pub fn build_accessibility_ir(
    source: &[ViewNode],
    snapshot: &AccessibilitySnapshot,
    texts: &BTreeMap<StableId, String>,
) -> Result<IrTree, IrError> {
    let tree = view_tree(source)?;
    let (mut nodes, index) = build_nodes(source, &tree, &BTreeSet::new());
    for (id, text) in texts {
        let current = *index.get(id).ok_or(IrError::TextOutsideView(*id))?;
        if nodes[current as usize].element != ElementKind::TEXT {
            return Err(IrError::TextOnNonTextNode(*id));
        }
        nodes[current as usize].text = Some(text.clone());
    }
    let accessibility = attach_accessibility(snapshot, &tree, &mut nodes, &index)?;
    for entry in &accessibility {
        check_state_references(&entry.semantics, &[])?;
    }
    Ok(IrTree {
        nodes,
        accessibility,
        states: Vec::new(),
        tokens: Vec::new(),
        index,
    })
}

fn build_nodes(
    source: &[ViewNode],
    tree: &ViewTree,
    excluded: &BTreeSet<StableId>,
) -> (Vec<IrNode>, BTreeMap<StableId, u32>) {
    // 시각 preorder(제외 subtree 생략).
    let mut nodes: Vec<IrNode> = Vec::new();
    let mut ir_index: BTreeMap<StableId, u32> = BTreeMap::new();
    let mut stack: Vec<(usize, Option<u32>, u32)> = vec![(tree.root, None, 0)];
    while let Some((position, parent, depth)) = stack.pop() {
        let source = &source[position];
        if excluded.contains(&source.id) {
            continue;
        }
        let index = nodes.len() as u32;
        let mut contracts = source.contracts.clone();
        contracts.sort();
        contracts.dedup();
        nodes.push(IrNode {
            id: source.id,
            parent,
            depth,
            children: Vec::new(),
            element: source.element,
            contracts,
            layout: None,
            text: None,
            properties: Vec::new(),
            semantics: None,
            capabilities: Vec::new(),
        });
        ir_index.insert(source.id, index);
        if let Some(parent) = parent {
            nodes[parent as usize].children.push(index);
        }
        stack.extend(
            tree.children[position]
                .iter()
                .rev()
                .map(|child| (*child, Some(index), depth + 1)),
        );
    }
    (nodes, ir_index)
}
