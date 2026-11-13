//! 의미 경계 기반 chunk split planner (V.6.4, ADR-0012).
//!
//! 기술 아키텍처 §21.1에 따라 Frame·Section·Component 등 **의미 경계**를 chunk
//! 경계로 사용하고, 분할은 문서 의미가 아니라 저장 최적화입니다. storage는 문서
//! 노드 타입을 모르므로 문서 계층이 subtree 하나(예: View root 아래)를
//! [`SplitTree`]로 넘깁니다. 노드마다 StableId, 부모, 의미 경계 여부, 그 노드가
//! chunk payload에 더하는 bytes를 담습니다.
//!
//! ## 크기 모델
//!
//! chunk payload 크기는 가산 모델([`ChunkCostModel`])로 계산합니다:
//! `chunk_base_bytes + Σ(소속 노드 own_bytes) + child_ref_bytes × 분리된 자식 chunk 수`.
//! 문서 계층이 노드 record를 서로 독립적으로(길이 프리픽스로) 인코딩하면 이 값은
//! envelope 비압축 payload 길이(ADR-0010)와 같습니다. ADR-0012에 따라
//! `payload > threshold`일 때만 분할합니다(같으면 분할하지 않음).
//!
//! ## 알고리즘(결정론)
//!
//! 입력 순서의 역순(자식 → 부모)으로 한 번 순회합니다.
//!
//! 1. tree root와 의미 경계 노드에서만 판정합니다. 그 노드의 region(아직 분리되지
//!    않은 자손 전체)이 임계치를 넘으면, region 안의 **가장 가까운 의미 경계
//!    자손**(사이에 다른 의미 경계가 없는 것)을 후보로 삼아 region이 큰 순서(같으면
//!    입력 순서가 앞선 것)로 분리합니다. region이 임계치 이하가 되면 멈춥니다.
//! 2. 분리해도 줄지 않는 후보(region ≤ `child_ref_bytes`)는 분리하지 않습니다.
//! 3. 의미 경계가 아닌 노드는 판정하지 않고 후보를 부모로 넘깁니다.
//!
//! 자손 판정이 먼저 끝나므로 후보의 region은 분리 시점에 이미 가능한 한 임계치
//! 이하입니다. 같은 입력은 항상 같은 계획을 만듭니다.
//!
//! ## 이전 layout 유지(저장 중 병합 금지)
//!
//! 저장 경로의 split은 이전 layout의 chunk root를 **그대로 유지**합니다(sticky).
//! 작은 chunk를 합치는 일은 저장이 아니라 idle 작업(V.6.5)이 맡습니다. 단, 이전
//! root가 삭제됐거나 더 이상 의미 경계가 아니면 chunk root가 될 수 없으므로 부모
//! chunk로 흡수하고 [`SplitPlan::released_roots`]로 알립니다. 결과 layout을 다시
//! 이전 layout으로 넘기면 같은 layout이 나옵니다(고정점).
//!
//! ## 임계치보다 큰 단일 노드(oversized)
//!
//! 노드 하나를 바이트 단위로 여러 chunk에 쪼개지 않습니다(의미 경계 밖 분할 금지).
//! 의미 경계로 더 나눌 수 없는 region이 임계치를 넘으면 다음처럼 처리합니다.
//!
//! - region 안에서 분리 가능한 의미 경계 후보는 **모두** 분리합니다. oversized
//!   범위를 그 노드와 의미 경계가 아닌 자손으로 줄여, 형제 subtree 편집이 큰 chunk를
//!   다시 쓰지 않게 합니다.
//! - 남은 region은 chunk 하나로 두고 [`PlannedChunk::oversized`]를 `true`로
//!   표시합니다. 오류가 아닙니다(임계치는 포맷 제약이 아니라 최적화 정책). 큰 노드가
//!   의미 경계 노드 안에 있으면 그 경계 노드가 부모 판정에서 가장 큰 후보가 되어
//!   별도 chunk로 격리됩니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use super::budget::ChunkBudget;

/// split planner 입력 노드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitNode {
    /// 노드 StableId.
    pub id: StableId,
    /// 부모 노드. tree root(입력의 첫 노드)만 `None`입니다.
    pub parent: Option<StableId>,
    /// Frame·Section·Component 등 의미 경계 여부. 의미 경계만 chunk root가 됩니다
    /// (tree root는 예외로 항상 chunk root).
    pub boundary: bool,
    /// 이 노드 하나가(자식 제외) chunk payload에 더하는 bytes.
    pub own_bytes: u64,
}

/// [`SplitTree`] 구성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitTreeError {
    /// 노드가 없습니다.
    Empty,
    /// 첫 노드(tree root)에 부모가 있습니다.
    RootHasParent {
        /// 해당 노드.
        node: StableId,
    },
    /// 첫 노드가 아닌데 부모가 없습니다(root가 둘 이상).
    MultipleRoots {
        /// 해당 노드.
        node: StableId,
    },
    /// 부모가 입력에서 자식보다 앞에 없습니다(없는 부모 포함).
    ParentNotBefore {
        /// 해당 노드.
        node: StableId,
        /// 찾지 못한 부모.
        parent: StableId,
    },
    /// 같은 StableId가 두 번 나타납니다.
    DuplicateNode {
        /// 중복 노드.
        node: StableId,
    },
}

impl fmt::Display for SplitTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplitTreeError::Empty => f.write_str("split tree에 노드가 없습니다"),
            SplitTreeError::RootHasParent { node } => {
                write!(f, "split tree root에 부모가 있습니다: {node}")
            }
            SplitTreeError::MultipleRoots { node } => {
                write!(f, "split tree에 root가 둘 이상입니다: {node}")
            }
            SplitTreeError::ParentNotBefore { node, parent } => write!(
                f,
                "split tree 노드 {node}의 부모 {parent}가 입력에서 앞에 없습니다"
            ),
            SplitTreeError::DuplicateNode { node } => {
                write!(f, "split tree 노드가 중복됩니다: {node}")
            }
        }
    }
}

impl std::error::Error for SplitTreeError {}

/// 검증된 split 입력 tree.
///
/// 노드 순서는 "부모가 자식보다 앞"이어야 하며, 같은 부모의 자식 순서와 동률
/// 판정은 이 입력 순서를 따릅니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitTree {
    nodes: Vec<SplitNode>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    index: BTreeMap<StableId, usize>,
}

impl SplitTree {
    /// 노드 목록을 검증해 tree를 만듭니다.
    pub fn new(nodes: Vec<SplitNode>) -> Result<SplitTree, SplitTreeError> {
        if nodes.is_empty() {
            return Err(SplitTreeError::Empty);
        }
        let mut index: BTreeMap<StableId, usize> = BTreeMap::new();
        let mut parents: Vec<Option<usize>> = Vec::with_capacity(nodes.len());
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        for (i, node) in nodes.iter().enumerate() {
            let parent = match (i, node.parent) {
                (0, Some(_)) => return Err(SplitTreeError::RootHasParent { node: node.id }),
                (0, None) => None,
                (_, None) => return Err(SplitTreeError::MultipleRoots { node: node.id }),
                (_, Some(parent)) => {
                    let p = *index.get(&parent).ok_or(SplitTreeError::ParentNotBefore {
                        node: node.id,
                        parent,
                    })?;
                    children[p].push(i);
                    Some(p)
                }
            };
            if index.insert(node.id, i).is_some() {
                return Err(SplitTreeError::DuplicateNode { node: node.id });
            }
            parents.push(parent);
        }
        Ok(SplitTree {
            nodes,
            parents,
            children,
            index,
        })
    }

    /// 입력 순서의 노드 목록.
    pub fn nodes(&self) -> &[SplitNode] {
        &self.nodes
    }

    /// tree root StableId.
    pub fn root(&self) -> StableId {
        self.nodes[0].id
    }

    /// 노드가 tree에 있는지 여부.
    pub fn contains(&self, id: StableId) -> bool {
        self.index.contains_key(&id)
    }

    /// StableId의 입력 index.
    pub(crate) fn index_of(&self, id: StableId) -> Option<usize> {
        self.index.get(&id).copied()
    }
}

/// chunk payload 크기의 가산 모델.
///
/// 값은 문서 계층의 payload 인코딩을 따라야 합니다. 기본값(0, 0)은 노드 bytes만
/// 셉니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkCostModel {
    /// chunk payload마다 붙는 고정 bytes(header 등).
    pub chunk_base_bytes: u64,
    /// 분리된 자식 chunk 하나를 부모 chunk payload에서 참조하는 bytes.
    pub child_ref_bytes: u64,
}

/// split/merge 정책: 임계치와 크기 모델.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPolicy {
    /// chunk 분할 임계치(bytes). payload가 이 값을 넘으면 분할 대상입니다.
    pub threshold_bytes: u64,
    /// payload 크기 모델.
    pub cost: ChunkCostModel,
}

impl SplitPolicy {
    /// 예산 config(`storage.chunk_split_threshold_bytes`)의 임계치로 정책을 만듭니다.
    pub fn from_budget(budget: &ChunkBudget, cost: ChunkCostModel) -> SplitPolicy {
        SplitPolicy {
            threshold_bytes: budget.split_threshold_bytes,
            cost,
        }
    }
}

/// 계획된 chunk 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedChunk {
    /// chunk root 노드.
    pub root: StableId,
    /// 이 chunk를 참조하는 부모 chunk의 root(tree root chunk는 `None`).
    pub parent_chunk: Option<StableId>,
    /// 소속 노드(입력 순서, root가 첫 항목).
    pub nodes: Vec<StableId>,
    /// 이 chunk에서 분리된 자식 chunk의 root(입력 순서).
    pub child_chunks: Vec<StableId>,
    /// 크기 모델로 계산한 payload bytes.
    pub payload_bytes: u64,
    /// payload가 임계치를 넘지만 의미 경계로 더 나눌 수 없는 chunk인지 여부.
    pub oversized: bool,
}

/// chunk layout: tree를 chunk로 나눈 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLayout {
    chunks: Vec<PlannedChunk>,
}

impl ChunkLayout {
    /// chunk 목록(root의 입력 순서). 첫 항목은 tree root chunk입니다.
    pub fn chunks(&self) -> &[PlannedChunk] {
        &self.chunks
    }

    /// tree root chunk.
    pub fn root_chunk(&self) -> &PlannedChunk {
        &self.chunks[0]
    }

    /// tree root를 제외한 chunk root 집합. 다음 저장의 이전 layout과 병합 planner
    /// 입력으로 씁니다.
    pub fn split_roots(&self) -> BTreeSet<StableId> {
        self.chunks[1..].iter().map(|c| c.root).collect()
    }

    /// oversized chunk들.
    pub fn oversized(&self) -> impl Iterator<Item = &PlannedChunk> {
        self.chunks.iter().filter(|c| c.oversized)
    }
}

/// split planner 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPlan {
    /// 새 chunk layout.
    pub layout: ChunkLayout,
    /// 이전 layout의 root 중 chunk root로 남지 못한 것(삭제됐거나 의미 경계가 아님),
    /// StableId 오름차순. 해당 chunk의 내용은 부모 chunk로 흡수됩니다.
    pub released_roots: Vec<StableId>,
}

/// split/layout 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    /// layout root가 tree에 없습니다.
    UnknownRoot {
        /// 해당 root.
        node: StableId,
    },
    /// layout root가 의미 경계가 아닙니다.
    NotBoundary {
        /// 해당 root.
        node: StableId,
    },
    /// tree root는 항상 chunk root이므로 layout root 목록에 넣지 않습니다.
    TreeRootListed {
        /// tree root.
        node: StableId,
    },
    /// payload 크기 합이 `u64` 범위를 넘습니다.
    SizeOverflow,
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplitError::UnknownRoot { node } => {
                write!(f, "chunk layout root가 tree에 없습니다: {node}")
            }
            SplitError::NotBoundary { node } => {
                write!(f, "chunk layout root가 의미 경계가 아닙니다: {node}")
            }
            SplitError::TreeRootListed { node } => {
                write!(
                    f,
                    "tree root는 chunk layout root 목록에 넣지 않습니다: {node}"
                )
            }
            SplitError::SizeOverflow => f.write_str("chunk payload 크기가 u64 범위를 넘습니다"),
        }
    }
}

impl std::error::Error for SplitError {}

fn add(a: u64, b: u64) -> Result<u64, SplitError> {
    a.checked_add(b).ok_or(SplitError::SizeOverflow)
}

/// chunk root 표시(`is_root[i]`, tree root는 무시)로 layout을 만듭니다.
pub(crate) fn build_layout(
    tree: &SplitTree,
    policy: &SplitPolicy,
    is_root: &[bool],
) -> Result<ChunkLayout, SplitError> {
    let n = tree.nodes.len();
    // 노드 index → 소속 chunk 위치.
    let mut chunk_pos: Vec<usize> = vec![0; n];
    let mut chunks: Vec<PlannedChunk> = Vec::new();
    let mut own_sums: Vec<u64> = Vec::new();
    for i in 0..n {
        let node = &tree.nodes[i];
        match tree.parents[i] {
            Some(p) if !is_root[i] => {
                let pos = chunk_pos[p];
                chunk_pos[i] = pos;
                chunks[pos].nodes.push(node.id);
                own_sums[pos] = add(own_sums[pos], node.own_bytes)?;
            }
            parent => {
                let parent_chunk = parent.map(|p| {
                    let pos = chunk_pos[p];
                    chunks[pos].child_chunks.push(node.id);
                    chunks[pos].root
                });
                chunk_pos[i] = chunks.len();
                chunks.push(PlannedChunk {
                    root: node.id,
                    parent_chunk,
                    nodes: vec![node.id],
                    child_chunks: Vec::new(),
                    payload_bytes: 0,
                    oversized: false,
                });
                own_sums.push(node.own_bytes);
            }
        }
    }
    for (chunk, own) in chunks.iter_mut().zip(own_sums) {
        let refs = u64::try_from(chunk.child_chunks.len())
            .ok()
            .and_then(|count| count.checked_mul(policy.cost.child_ref_bytes))
            .ok_or(SplitError::SizeOverflow)?;
        chunk.payload_bytes = add(add(policy.cost.chunk_base_bytes, own)?, refs)?;
        chunk.oversized = chunk.payload_bytes > policy.threshold_bytes;
    }
    Ok(ChunkLayout { chunks })
}

/// 주어진 chunk root 집합(tree root 제외)으로 layout을 만듭니다.
///
/// root는 tree에 있고 의미 경계여야 합니다. 분할 판단은 하지 않습니다.
pub fn materialize_layout(
    tree: &SplitTree,
    policy: &SplitPolicy,
    roots: &BTreeSet<StableId>,
) -> Result<ChunkLayout, SplitError> {
    let mut is_root = vec![false; tree.nodes.len()];
    for &id in roots {
        match tree.index_of(id) {
            None => return Err(SplitError::UnknownRoot { node: id }),
            Some(0) => return Err(SplitError::TreeRootListed { node: id }),
            Some(i) if !tree.nodes[i].boundary => return Err(SplitError::NotBoundary { node: id }),
            Some(i) => is_root[i] = true,
        }
    }
    build_layout(tree, policy, &is_root)
}

/// 의미 경계 기준으로 chunk split을 계획합니다.
///
/// - `previous_roots`: 이전 layout의 chunk root(tree root 제외). 아직 tree에 있고
///   의미 경계이면 그대로 유지하고, 아니면 [`SplitPlan::released_roots`]로
///   돌려줍니다. 처음 저장은 빈 집합입니다. tree root가 들어 있으면 무시합니다.
///
/// 분할 규칙과 oversized 처리는 모듈 문서를 따릅니다.
pub fn plan_chunk_split(
    tree: &SplitTree,
    policy: &SplitPolicy,
    previous_roots: &BTreeSet<StableId>,
) -> Result<SplitPlan, SplitError> {
    let n = tree.nodes.len();
    let base = policy.cost.chunk_base_bytes;
    let ref_bytes = policy.cost.child_ref_bytes;

    let mut is_root = vec![false; n];
    let mut released_roots = Vec::new();
    for &id in previous_roots {
        match tree.index_of(id) {
            Some(0) => {}
            Some(i) if tree.nodes[i].boundary => is_root[i] = true,
            _ => released_roots.push(id),
        }
    }

    // region[i]: i를 root로 볼 때 chunk_base를 뺀 region 크기(판정 후 값).
    let mut region = vec![0u64; n];
    // pending[i]: 의미 경계가 아닌 i 아래에서 부모로 넘길 가장 가까운 의미 경계 후보.
    let mut pending: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in (0..n).rev() {
        let mut size = tree.nodes[i].own_bytes;
        let mut candidates: Vec<usize> = Vec::new();
        for &c in &tree.children[i] {
            if is_root[c] {
                size = add(size, ref_bytes)?;
                continue;
            }
            size = add(size, region[c])?;
            if tree.nodes[c].boundary {
                candidates.push(c);
            } else {
                candidates.append(&mut pending[c]);
            }
        }
        if i == 0 || tree.nodes[i].boundary {
            if add(base, size)? > policy.threshold_bytes {
                // region이 큰 순서, 같으면 입력 순서.
                candidates.sort_by(|&a, &b| region[b].cmp(&region[a]).then(a.cmp(&b)));
                for c in candidates {
                    if add(base, size)? <= policy.threshold_bytes || region[c] <= ref_bytes {
                        break;
                    }
                    is_root[c] = true;
                    // size는 region[c]를 포함하므로 underflow가 없습니다.
                    size -= region[c] - ref_bytes;
                }
            }
        } else {
            pending[i] = candidates;
        }
        region[i] = size;
    }

    let layout = build_layout(tree, policy, &is_root)?;
    Ok(SplitPlan {
        layout,
        released_roots,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0abc, 0x0123_4567_89ab)
    }

    pub(crate) fn node(id: u64, parent: Option<u64>, boundary: bool, own: u64) -> SplitNode {
        SplitNode {
            id: sid(id),
            parent: parent.map(sid),
            boundary,
            own_bytes: own,
        }
    }

    fn policy(threshold: u64, base: u64, child_ref: u64) -> SplitPolicy {
        SplitPolicy {
            threshold_bytes: threshold,
            cost: ChunkCostModel {
                chunk_base_bytes: base,
                child_ref_bytes: child_ref,
            },
        }
    }

    /// 저장소 예산 파일의 승인 임계치(ADR-0012)로 만든 정책.
    fn repo_policy(cost: ChunkCostModel) -> SplitPolicy {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/performance-budgets.yaml"
        );
        let budget = ChunkBudget::from_budget_file(Path::new(path)).unwrap();
        SplitPolicy::from_budget(&budget, cost)
    }

    fn roots_of(layout: &ChunkLayout) -> Vec<StableId> {
        layout.chunks().iter().map(|c| c.root).collect()
    }

    fn fresh(tree: &SplitTree, policy: &SplitPolicy) -> SplitPlan {
        plan_chunk_split(tree, policy, &BTreeSet::new()).unwrap()
    }

    /// layout 불변식을 검사합니다(모든 계획 결과에 적용).
    pub(crate) fn assert_layout_invariants(
        tree: &SplitTree,
        policy: &SplitPolicy,
        layout: &ChunkLayout,
    ) {
        let mut owner: BTreeMap<StableId, StableId> = BTreeMap::new();
        for chunk in layout.chunks() {
            assert_eq!(chunk.nodes[0], chunk.root, "chunk 첫 노드는 root");
            for &member in &chunk.nodes {
                assert!(
                    owner.insert(member, chunk.root).is_none(),
                    "노드 {member}가 두 chunk에 속함"
                );
            }
        }
        assert_eq!(
            owner.len(),
            tree.nodes.len(),
            "모든 노드가 정확히 한 chunk에 속함"
        );
        assert_eq!(layout.root_chunk().root, tree.root());

        for (pos, chunk) in layout.chunks().iter().enumerate() {
            let root_index = tree.index_of(chunk.root).unwrap();
            // chunk root는 tree root 또는 의미 경계.
            assert!(root_index == 0 || tree.nodes[root_index].boundary);
            // 연결성: root가 아닌 소속 노드의 부모는 같은 chunk.
            for &member in &chunk.nodes[1..] {
                let parent = tree.nodes[tree.index_of(member).unwrap()].parent.unwrap();
                assert_eq!(owner[&parent], chunk.root, "chunk가 연결되어 있지 않음");
            }
            // 부모 chunk 참조 일치.
            let expected_parent = tree.nodes[root_index].parent.map(|p| owner[&p]);
            assert_eq!(chunk.parent_chunk, expected_parent);
            if pos == 0 {
                assert!(chunk.parent_chunk.is_none());
            }
            // payload 재계산 일치.
            let own: u64 = chunk
                .nodes
                .iter()
                .map(|m| tree.nodes[tree.index_of(*m).unwrap()].own_bytes)
                .sum();
            let expected = policy.cost.chunk_base_bytes
                + own
                + policy.cost.child_ref_bytes * chunk.child_chunks.len() as u64;
            assert_eq!(chunk.payload_bytes, expected);
            assert_eq!(chunk.oversized, expected > policy.threshold_bytes);
        }
    }

    /// oversized chunk 안에는 분리해서 줄일 수 있는 의미 경계가 남아 있지 않아야 합니다.
    fn assert_oversized_is_irreducible(
        tree: &SplitTree,
        policy: &SplitPolicy,
        layout: &ChunkLayout,
    ) {
        for chunk in layout.oversized() {
            let members: BTreeSet<StableId> = chunk.nodes.iter().copied().collect();
            let child_chunks: BTreeSet<StableId> = chunk.child_chunks.iter().copied().collect();
            for &member in &chunk.nodes[1..] {
                let i = tree.index_of(member).unwrap();
                if !tree.nodes[i].boundary {
                    continue;
                }
                // member를 root로 하는 chunk 안 region 크기.
                let mut region = 0u64;
                let mut stack = vec![i];
                while let Some(x) = stack.pop() {
                    region += tree.nodes[x].own_bytes;
                    for &c in &tree.children[x] {
                        let cid = tree.nodes[c].id;
                        if child_chunks.contains(&cid) {
                            region += policy.cost.child_ref_bytes;
                        } else if members.contains(&cid) {
                            stack.push(c);
                        }
                    }
                }
                assert!(
                    region <= policy.cost.child_ref_bytes,
                    "oversized chunk {}에 분리 가능한 의미 경계 {member}가 남음",
                    chunk.root
                );
            }
        }
    }

    #[test]
    fn threshold_boundary_fixtures_split_as_expected() {
        // 검증 기준(V.6.4): 경계 바로 아래·위 fixture에서 예상 split.
        // 승인 임계치(262,144 bytes)를 저장소 예산 파일에서 읽습니다.
        let cost = ChunkCostModel {
            chunk_base_bytes: 20,
            child_ref_bytes: 32,
        };
        let policy = repo_policy(cost);
        let t = policy.threshold_bytes;
        assert_eq!(t, 262_144);
        // R(Frame) > F1(Frame), F2(Section). base + R + F1 + F2 == T.
        let f2_at_threshold = t - 20 - 1_000 - 100_000;
        let build = |f2: u64| {
            SplitTree::new(vec![
                node(1, None, true, 1_000),
                node(2, Some(1), true, 100_000),
                node(3, Some(1), true, f2),
            ])
            .unwrap()
        };

        // 바로 아래(같음): payload == T → 분할하지 않습니다.
        let tree = build(f2_at_threshold);
        let at = fresh(&tree, &policy);
        assert_eq!(roots_of(&at.layout), vec![sid(1)]);
        assert_eq!(at.layout.root_chunk().payload_bytes, t);
        assert!(!at.layout.root_chunk().oversized);
        assert_layout_invariants(&tree, &policy, &at.layout);

        // 바로 위(T + 1): 가장 큰 의미 경계 자식 F2만 분리합니다.
        let tree = build(f2_at_threshold + 1);
        let above = fresh(&tree, &policy);
        assert_eq!(roots_of(&above.layout), vec![sid(1), sid(3)]);
        let chunks = above.layout.chunks();
        assert_eq!(chunks[0].nodes, vec![sid(1), sid(2)]);
        assert_eq!(chunks[0].child_chunks, vec![sid(3)]);
        assert_eq!(chunks[0].payload_bytes, 20 + 1_000 + 100_000 + 32);
        assert_eq!(chunks[1].parent_chunk, Some(sid(1)));
        assert_eq!(chunks[1].payload_bytes, 20 + f2_at_threshold + 1);
        assert!(chunks.iter().all(|c| c.payload_bytes <= t && !c.oversized));
        assert_layout_invariants(&tree, &policy, &above.layout);
    }

    #[test]
    fn nested_boundary_under_plain_group_is_a_candidate() {
        // R(Frame) > G(Group, 의미 경계 아님) > A(Frame), B(Frame).
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), false, 10),
            node(3, Some(2), true, 60),
            node(4, Some(2), true, 50),
        ])
        .unwrap();
        let policy = policy(100, 0, 5);
        let plan = fresh(&tree, &policy);
        // 130 > 100 → 가장 큰 후보 A 분리 → 10+10+50+5 = 75.
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(3)]);
        assert_eq!(plan.layout.chunks()[0].nodes, vec![sid(1), sid(2), sid(4)]);
        assert_eq!(plan.layout.chunks()[0].payload_bytes, 75);
        assert_layout_invariants(&tree, &policy, &plan.layout);
    }

    #[test]
    fn nearest_boundary_is_split_and_nested_one_stays_inside() {
        // R > C1(Frame) > C2(Frame). R 판정의 후보는 가장 가까운 C1뿐입니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 10),
            node(3, Some(2), true, 30),
        ])
        .unwrap();
        let policy = policy(45, 0, 1);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2)]);
        assert_eq!(plan.layout.chunks()[1].nodes, vec![sid(2), sid(3)]);
        assert_eq!(plan.layout.chunks()[0].payload_bytes, 11);
        assert_layout_invariants(&tree, &policy, &plan.layout);
    }

    #[test]
    fn descendants_are_decided_before_ancestors() {
        // R > C1 > C2: C1 판정에서 C2가 먼저 분리되어 R은 분리할 필요가 없습니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 30),
            node(3, Some(2), true, 30),
        ])
        .unwrap();
        let policy = policy(50, 0, 1);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(3)]);
        assert_eq!(plan.layout.chunks()[0].nodes, vec![sid(1), sid(2)]);
        assert_eq!(plan.layout.chunks()[0].payload_bytes, 41);
        assert_layout_invariants(&tree, &policy, &plan.layout);
    }

    #[test]
    fn equal_candidates_tie_break_by_input_order() {
        let tree = SplitTree::new(vec![
            node(1, None, true, 0),
            node(2, Some(1), true, 50),
            node(3, Some(1), true, 50),
        ])
        .unwrap();
        let policy = policy(60, 0, 0);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2)]);
        assert_eq!(plan.layout.chunks()[0].nodes, vec![sid(1), sid(3)]);
    }

    #[test]
    fn oversized_leaf_inside_frame_is_isolated() {
        // R > F1 > [Huge(의미 경계 아님), F3], F2. Huge 하나가 임계치보다 큽니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 100),
            node(2, Some(1), true, 100),
            node(3, Some(2), false, 1_000),
            node(4, Some(2), true, 50),
            node(5, Some(1), true, 100),
        ])
        .unwrap();
        let policy = policy(500, 0, 4);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2), sid(4)]);
        let chunks = plan.layout.chunks();
        // R chunk: R + F2 + ref(F1) = 204.
        assert_eq!(chunks[0].nodes, vec![sid(1), sid(5)]);
        assert_eq!(chunks[0].payload_bytes, 204);
        // F1 chunk: F1 + Huge + ref(F3) = 1104, oversized.
        assert_eq!(chunks[1].nodes, vec![sid(2), sid(3)]);
        assert_eq!(chunks[1].payload_bytes, 1_104);
        assert!(chunks[1].oversized);
        // F3는 F1 판정에서 분리됩니다(oversized 범위 최소화).
        assert_eq!(chunks[2].parent_chunk, Some(sid(2)));
        assert_eq!(
            plan.layout.oversized().map(|c| c.root).collect::<Vec<_>>(),
            vec![sid(2)]
        );
        assert_layout_invariants(&tree, &policy, &plan.layout);
        assert_oversized_is_irreducible(&tree, &policy, &plan.layout);
    }

    #[test]
    fn single_boundary_node_larger_than_threshold_gets_own_chunk() {
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 1_000),
            node(3, Some(1), true, 10),
        ])
        .unwrap();
        let policy = policy(100, 0, 4);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2)]);
        assert_eq!(plan.layout.chunks()[0].payload_bytes, 24);
        assert!(plan.layout.chunks()[1].oversized);
        assert_eq!(plan.layout.chunks()[1].nodes, vec![sid(2)]);
    }

    #[test]
    fn oversized_root_region_splits_every_reducing_candidate() {
        // root 자체가 임계치보다 큽니다. 분리 가능한 후보를 전부 분리합니다.
        let tree = SplitTree::new(vec![
            node(1, None, false, 1_000),
            node(2, Some(1), true, 10),
            node(3, Some(1), true, 20),
            node(4, Some(1), false, 5),
            node(5, Some(4), true, 30),
        ])
        .unwrap();
        let policy = policy(100, 0, 1);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2), sid(3), sid(5)]);
        assert_eq!(plan.layout.root_chunk().nodes, vec![sid(1), sid(4)]);
        assert_eq!(plan.layout.root_chunk().payload_bytes, 1_008);
        assert!(plan.layout.root_chunk().oversized);
        assert_layout_invariants(&tree, &policy, &plan.layout);
        assert_oversized_is_irreducible(&tree, &policy, &plan.layout);
    }

    #[test]
    fn candidate_that_does_not_reduce_size_is_not_split() {
        // A의 region(40)이 참조 비용(50) 이하라 분리해도 줄지 않습니다.
        let tree =
            SplitTree::new(vec![node(1, None, true, 200), node(2, Some(1), true, 40)]).unwrap();
        let policy = policy(100, 0, 50);
        let plan = fresh(&tree, &policy);
        assert_eq!(roots_of(&plan.layout), vec![sid(1)]);
        assert!(plan.layout.root_chunk().oversized);
        assert_oversized_is_irreducible(&tree, &policy, &plan.layout);
    }

    #[test]
    fn previous_roots_are_kept_or_released() {
        // 저장 경로는 병합하지 않습니다: 작은 이전 root A도 유지합니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 10),
            node(3, Some(1), false, 10),
            node(4, Some(1), true, 10),
        ])
        .unwrap();
        let policy = policy(1_000, 0, 1);
        let previous: BTreeSet<StableId> = [sid(1), sid(2), sid(3), sid(99)].into_iter().collect();
        let plan = plan_chunk_split(&tree, &policy, &previous).unwrap();
        assert_eq!(roots_of(&plan.layout), vec![sid(1), sid(2)]);
        // 의미 경계가 아닌 B와 삭제된 X는 release(오름차순), tree root는 무시.
        let mut expected = vec![sid(3), sid(99)];
        expected.sort();
        assert_eq!(plan.released_roots, expected);
        assert_layout_invariants(&tree, &policy, &plan.layout);
    }

    /// 결정론적 pseudo-random tree(LCG).
    pub(crate) fn random_tree(seed: u64, len: usize, max_own: u64) -> SplitTree {
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        let mut nodes = vec![node(1, None, next() % 2 == 0, next() % max_own)];
        for i in 1..len as u64 {
            let parent = 1 + next() % i;
            nodes.push(node(i + 1, Some(parent), next() % 3 == 0, next() % max_own));
        }
        SplitTree::new(nodes).unwrap()
    }

    #[test]
    fn random_trees_satisfy_invariants_deterministically_and_are_fixpoints() {
        let cost = ChunkCostModel {
            chunk_base_bytes: 20,
            child_ref_bytes: 32,
        };
        for seed in 0..60u64 {
            let tree = random_tree(seed, 1 + (seed as usize * 7) % 200, 5_000);
            for threshold in [2_000u64, 10_000, 60_000] {
                let policy = SplitPolicy {
                    threshold_bytes: threshold,
                    cost,
                };
                let plan = fresh(&tree, &policy);
                assert_layout_invariants(&tree, &policy, &plan.layout);
                assert_oversized_is_irreducible(&tree, &policy, &plan.layout);
                // 결정론.
                assert_eq!(fresh(&tree, &policy), plan);
                // 고정점: 결과 layout을 이전 layout으로 넘기면 같은 layout.
                let again = plan_chunk_split(&tree, &policy, &plan.layout.split_roots()).unwrap();
                assert_eq!(
                    again.layout, plan.layout,
                    "seed {seed}, threshold {threshold}"
                );
                assert!(again.released_roots.is_empty());
                // materialize는 같은 root로 같은 layout을 만듭니다.
                assert_eq!(
                    materialize_layout(&tree, &policy, &plan.layout.split_roots()).unwrap(),
                    plan.layout
                );
            }
        }
    }

    #[test]
    fn invalid_trees_are_rejected() {
        assert_eq!(SplitTree::new(vec![]), Err(SplitTreeError::Empty));
        assert_eq!(
            SplitTree::new(vec![node(1, Some(9), true, 0)]),
            Err(SplitTreeError::RootHasParent { node: sid(1) })
        );
        assert_eq!(
            SplitTree::new(vec![node(1, None, true, 0), node(2, None, true, 0)]),
            Err(SplitTreeError::MultipleRoots { node: sid(2) })
        );
        assert_eq!(
            SplitTree::new(vec![
                node(1, None, true, 0),
                node(2, Some(3), true, 0),
                node(3, Some(1), true, 0),
            ]),
            Err(SplitTreeError::ParentNotBefore {
                node: sid(2),
                parent: sid(3)
            })
        );
        assert_eq!(
            SplitTree::new(vec![node(1, None, true, 0), node(1, Some(1), true, 0)]),
            Err(SplitTreeError::DuplicateNode { node: sid(1) })
        );
    }

    #[test]
    fn invalid_layout_roots_are_rejected() {
        let tree = SplitTree::new(vec![
            node(1, None, true, 0),
            node(2, Some(1), false, 0),
            node(3, Some(1), true, 0),
        ])
        .unwrap();
        let policy = policy(10, 0, 0);
        let set = |ids: &[u64]| ids.iter().map(|i| sid(*i)).collect::<BTreeSet<_>>();
        assert_eq!(
            materialize_layout(&tree, &policy, &set(&[9])),
            Err(SplitError::UnknownRoot { node: sid(9) })
        );
        assert_eq!(
            materialize_layout(&tree, &policy, &set(&[2])),
            Err(SplitError::NotBoundary { node: sid(2) })
        );
        assert_eq!(
            materialize_layout(&tree, &policy, &set(&[1])),
            Err(SplitError::TreeRootListed { node: sid(1) })
        );
        assert!(materialize_layout(&tree, &policy, &set(&[3])).is_ok());
    }

    #[test]
    fn size_overflow_is_an_error() {
        let tree = SplitTree::new(vec![
            node(1, None, true, u64::MAX),
            node(2, Some(1), true, 1),
        ])
        .unwrap();
        assert_eq!(
            plan_chunk_split(&tree, &policy(10, 0, 0), &BTreeSet::new()),
            Err(SplitError::SizeOverflow)
        );
        assert!(!SplitError::SizeOverflow.to_string().is_empty());
    }
}
