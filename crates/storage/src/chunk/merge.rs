//! 작은 인접 chunk 병합 planner (V.6.5).
//!
//! split planner(V.6.4)는 저장 경로에서 이전 layout의 chunk root를 유지하고 병합하지
//! 않습니다. 편집으로 작아진 chunk는 이 planner가 **idle 작업에서만** 합칩니다.
//!
//! ## 병합 기준
//!
//! - **인접**: chunk tree의 부모–자식 관계입니다. chunk는 root 하나를 가진 연결된
//!   subtree라서, 같은 부모 아래 형제 chunk는 각자 부모 chunk로 흡수되는 방식으로만
//!   합쳐집니다.
//! - **작음**: 별도 수치를 두지 않고 승인 임계치(ADR-0012)로 판정합니다. 자식 chunk를
//!   부모 chunk에 흡수한 결과 payload가 `≤ threshold`일 때만 병합합니다. 결과가
//!   임계치를 넘지 않으므로 다음 저장의 split이 곧바로 다시 나누지 않습니다
//!   (split/merge 진동 없음). 임계치를 넘는 chunk는 더 커지지 않습니다.
//! - **순서(결정론)**: chunk root의 입력 역순(깊은 chunk부터)으로 순회하며, 각 chunk를
//!   그 시점의 부모 chunk에 흡수할 수 있으면 흡수합니다. 흡수가 있었으면 바뀐 layout으로
//!   다시 순회하고, 더 흡수할 것이 없으면 멈춥니다. 결과 layout에서는 어떤 자식 chunk도
//!   부모에 흡수할 수 없습니다.
//!
//! ## 의미 보존
//!
//! 병합은 chunk root 집합만 바꿉니다. 노드, 부모, 순서, 노드 payload는 입력 그대로이며
//! 소속 chunk만 바뀝니다(§21.1: 분할·병합은 저장 최적화). 흡수된 chunk의 ChunkId는
//! [`MergePlan::remap_manifest`]로 살아 있는 chunk에 재매핑합니다(V.3.4).
//!
//! ## 저장과의 배타
//!
//! 병합은 [`GcScheduler`]의 idle 유지보수 guard를 잡은 동안에만 실행합니다(V.7.4 패턴).
//!
//! - 활성 저장이 있거나, idle threshold가 지나지 않았거나, GC·다른 병합이 실행 중이면
//!   시작하지 않습니다([`IdleMergeOutcome::NotStarted`]).
//! - 실행 중 저장이 시작되면 결과를 폐기합니다([`IdleMergeOutcome::Superseded`]). 저장을
//!   막거나 기다리게 하지 않습니다.
//! - 받아들인 계획은 이후 저장이 이전 layout(`plan.layout().split_roots()`)으로 씁니다.
//!   그 전에 [`MergePlan::check_base`]로 계획의 기준 layout이 현재 layout과 같은지
//!   확인하고, 다르면 버립니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Instant;

use stellar_core_types::ids::StableId;

use crate::gc::scheduler::{GcGuard, GcScheduler};

use super::id::ChunkId;
use super::remap::{ChunkRemapManifest, RemapInsertError};
use super::split::{materialize_layout, ChunkLayout, SplitError, SplitPolicy, SplitTree};

/// 흡수되는 chunk 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkAbsorption {
    /// 흡수되어 사라지는 chunk의 root.
    pub chunk_root: StableId,
    /// 내용을 받는 살아 있는 chunk의 root(chain을 따라간 최종 대상).
    pub into: StableId,
}

/// 병합 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    base_roots: BTreeSet<StableId>,
    absorbed: Vec<ChunkAbsorption>,
    layout: ChunkLayout,
}

/// 계획의 기준 layout이 현재 layout과 다릅니다(그 사이 저장이 layout을 바꿈).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleMergePlan;

impl fmt::Display for StaleMergePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("병합 계획의 기준 chunk layout이 현재 layout과 다릅니다")
    }
}

impl std::error::Error for StaleMergePlan {}

impl MergePlan {
    /// 계획을 만든 기준 layout의 chunk root(tree root 제외).
    pub fn base_roots(&self) -> &BTreeSet<StableId> {
        &self.base_roots
    }

    /// 흡수되는 chunk 목록(chunk root의 입력 순서).
    pub fn absorbed(&self) -> &[ChunkAbsorption] {
        &self.absorbed
    }

    /// 병합 후 layout.
    pub fn layout(&self) -> &ChunkLayout {
        &self.layout
    }

    /// 병합할 것이 없는지 여부.
    pub fn is_noop(&self) -> bool {
        self.absorbed.is_empty()
    }

    /// 현재 layout이 계획의 기준 layout과 같은지 확인합니다.
    pub fn check_base(&self, current_roots: &BTreeSet<StableId>) -> Result<(), StaleMergePlan> {
        if *current_roots == self.base_roots {
            Ok(())
        } else {
            Err(StaleMergePlan)
        }
    }

    /// 흡수된 chunk의 ChunkId를 살아 있는 chunk의 ChunkId로 재매핑하는 manifest를
    /// 만듭니다. `chunk_id_of`는 chunk root에서 논리 ChunkId를 유도하는 문서 계층의
    /// 규칙입니다.
    pub fn remap_manifest(
        &self,
        chunk_id_of: impl Fn(StableId) -> ChunkId,
    ) -> Result<ChunkRemapManifest, RemapInsertError> {
        let mut manifest = ChunkRemapManifest::new();
        for absorption in &self.absorbed {
            manifest.insert(
                chunk_id_of(absorption.chunk_root),
                chunk_id_of(absorption.into),
            )?;
        }
        Ok(manifest)
    }
}

/// layout을 한 번 순회하며 흡수할 (자식 root, 직접 부모 root)를 찾습니다.
fn merge_pass(
    layout: &ChunkLayout,
    policy: &SplitPolicy,
) -> Result<Vec<(StableId, StableId)>, SplitError> {
    let chunks = layout.chunks();
    let base = policy.cost.chunk_base_bytes;
    let ref_bytes = policy.cost.child_ref_bytes;
    let pos_of: BTreeMap<StableId, usize> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| (c.root, i))
        .collect();
    let mut size: Vec<u64> = chunks.iter().map(|c| c.payload_bytes).collect();
    let mut absorbed = Vec::new();
    // 부모 chunk는 root의 입력 순서가 앞서므로 역순 순회에서 아직 살아 있습니다.
    for pos in (1..chunks.len()).rev() {
        let Some(parent) = chunks[pos]
            .parent_chunk
            .and_then(|p| pos_of.get(&p).copied())
        else {
            continue;
        };
        // 부모 payload는 이 자식 참조를 포함하고(≥ ref), 자식 payload는 base를 포함하므로
        // 뺄셈이 underflow하지 않습니다.
        let merged = (size[parent] - ref_bytes)
            .checked_add(size[pos] - base)
            .ok_or(SplitError::SizeOverflow)?;
        if merged <= policy.threshold_bytes {
            size[parent] = merged;
            absorbed.push((chunks[pos].root, chunks[parent].root));
        }
    }
    Ok(absorbed)
}

/// 작은 인접 chunk의 병합을 계획합니다.
///
/// - `roots`: 현재 layout의 chunk root(tree root 제외). tree에 있는 의미 경계여야 합니다.
///
/// 병합 기준과 순서는 모듈 문서를 따릅니다. 같은 입력은 항상 같은 계획을 만듭니다.
pub fn plan_chunk_merges(
    tree: &SplitTree,
    policy: &SplitPolicy,
    roots: &BTreeSet<StableId>,
) -> Result<MergePlan, SplitError> {
    let mut layout = materialize_layout(tree, policy, roots)?;
    let mut direct: BTreeMap<StableId, StableId> = BTreeMap::new();
    loop {
        let pass = merge_pass(&layout, policy)?;
        if pass.is_empty() {
            break;
        }
        let mut current = layout.split_roots();
        for (child, parent) in pass {
            current.remove(&child);
            direct.insert(child, parent);
        }
        layout = materialize_layout(tree, policy, &current)?;
    }

    // 흡수 chain(c → p → g)을 최종 대상으로 펼칩니다. 대상은 항상 입력 순서가 앞선
    // chunk이므로 cycle이 없습니다.
    let resolve = |mut root: StableId| {
        while let Some(next) = direct.get(&root) {
            root = *next;
        }
        root
    };
    let mut absorbed: Vec<ChunkAbsorption> = direct
        .keys()
        .map(|&chunk_root| ChunkAbsorption {
            chunk_root,
            into: resolve(chunk_root),
        })
        .collect();
    absorbed.sort_by_key(|a| tree.index_of(a.chunk_root));

    Ok(MergePlan {
        base_roots: roots.clone(),
        absorbed,
        layout,
    })
}

/// idle 병합 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleMergeOutcome {
    /// 저장 진행 중, idle threshold 전, 또는 GC·다른 병합 실행 중이라 시작하지 않았습니다.
    NotStarted,
    /// 실행 중 저장이 시작되어 결과를 폐기했습니다.
    Superseded,
    /// 저장과 겹치지 않고 끝난 계획(병합할 것이 없을 수도 있음).
    Ready(MergePlan),
}

/// idle 병합 실행 창. scheduler의 idle 유지보수 guard를 잡고 있습니다.
#[derive(Debug)]
pub struct IdleMergeWindow<'a> {
    guard: GcGuard<'a>,
}

/// idle 병합 창을 엽니다. 활성 저장 0, idle threshold 경과, GC·다른 병합 미실행일 때만
/// `Some`입니다.
pub fn begin_idle_merge(scheduler: &GcScheduler, now: Instant) -> Option<IdleMergeWindow<'_>> {
    scheduler
        .try_begin_gc(now)
        .map(|guard| IdleMergeWindow { guard })
}

impl IdleMergeWindow<'_> {
    /// 창 안에서 병합을 계획합니다.
    pub fn plan(
        &self,
        tree: &SplitTree,
        policy: &SplitPolicy,
        roots: &BTreeSet<StableId>,
    ) -> Result<MergePlan, SplitError> {
        plan_chunk_merges(tree, policy, roots)
    }

    /// 창을 닫고 계획을 확정합니다. 창이 열린 뒤 저장이 시작됐으면 폐기합니다.
    pub fn finish(self, plan: MergePlan) -> IdleMergeOutcome {
        if self.guard.save_started_since() {
            IdleMergeOutcome::Superseded
        } else {
            IdleMergeOutcome::Ready(plan)
        }
    }
}

/// idle 병합을 한 번 실행합니다(창 열기 → 계획 → 확정).
pub fn run_idle_merge(
    scheduler: &GcScheduler,
    now: Instant,
    tree: &SplitTree,
    policy: &SplitPolicy,
    roots: &BTreeSet<StableId>,
) -> Result<IdleMergeOutcome, SplitError> {
    let Some(window) = begin_idle_merge(scheduler, now) else {
        return Ok(IdleMergeOutcome::NotStarted);
    };
    let plan = window.plan(tree, policy, roots)?;
    Ok(window.finish(plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::budget::ChunkBudget;
    use crate::chunk::split::tests::{assert_layout_invariants, node, random_tree, sid};
    use crate::chunk::split::{plan_chunk_split, ChunkCostModel, PlannedChunk, SplitNode};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    fn policy(threshold: u64, base: u64, child_ref: u64) -> SplitPolicy {
        SplitPolicy {
            threshold_bytes: threshold,
            cost: ChunkCostModel {
                chunk_base_bytes: base,
                child_ref_bytes: child_ref,
            },
        }
    }

    fn set(ids: &[u64]) -> BTreeSet<StableId> {
        ids.iter().map(|i| sid(*i)).collect()
    }

    fn roots_of(layout: &ChunkLayout) -> Vec<StableId> {
        layout.chunks().iter().map(|c| c.root).collect()
    }

    // --- 의미 snapshot fixture -------------------------------------------------

    /// 테스트 문서 노드(의미 속성 포함).
    #[derive(Debug, Clone)]
    struct DocNode {
        id: StableId,
        parent: Option<StableId>,
        boundary: bool,
        /// 형제 사이 순서(OrderKey 역할 — 위치가 아니라 노드 속성).
        order: u32,
        content: Vec<u8>,
    }

    /// chunk header: root id 16 + node count u32 + child ref count u32.
    const HEADER_BYTES: u64 = 24;
    /// 노드 record 고정부: id 16 + parent 16 + order u32 + boundary u8 + content len u32.
    const RECORD_FIXED_BYTES: u64 = 41;
    /// 자식 chunk 참조: root id 16.
    const REF_BYTES: u64 = 16;

    /// 의미 snapshot: node id → (parent, order, boundary, content).
    type Snapshot = BTreeMap<StableId, (Option<StableId>, u32, bool, Vec<u8>)>;

    fn random_document(seed: u64, len: usize) -> Vec<DocNode> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(7);
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        let mut nodes: Vec<DocNode> = Vec::with_capacity(len);
        let mut child_counts: BTreeMap<StableId, u32> = BTreeMap::new();
        for i in 0..len as u64 {
            let parent = if i == 0 {
                None
            } else {
                Some(nodes[(next() % i) as usize].id)
            };
            let order = parent.map_or(0, |p| {
                let count = child_counts.entry(p).or_insert(0);
                *count += 1;
                *count - 1
            });
            let content_len = (next() % 400) as usize;
            let content = (0..content_len)
                .map(|k| (next() as u8) ^ (k as u8))
                .collect();
            nodes.push(DocNode {
                id: sid(10_000 + i),
                parent,
                boundary: i == 0 || next() % 3 == 0,
                order,
                content,
            });
        }
        nodes
    }

    fn split_tree_of(doc: &[DocNode]) -> SplitTree {
        SplitTree::new(
            doc.iter()
                .map(|n| SplitNode {
                    id: n.id,
                    parent: n.parent,
                    boundary: n.boundary,
                    own_bytes: RECORD_FIXED_BYTES + n.content.len() as u64,
                })
                .collect(),
        )
        .unwrap()
    }

    fn encode_chunk(chunk: &PlannedChunk, by_id: &BTreeMap<StableId, &DocNode>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&chunk.root.to_bytes());
        out.extend_from_slice(&(chunk.nodes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(chunk.child_chunks.len() as u32).to_le_bytes());
        for id in &chunk.nodes {
            let n = by_id[id];
            out.extend_from_slice(&n.id.to_bytes());
            out.extend_from_slice(&n.parent.unwrap_or(StableId::NIL).to_bytes());
            out.extend_from_slice(&n.order.to_le_bytes());
            out.push(u8::from(n.boundary));
            out.extend_from_slice(&(n.content.len() as u32).to_le_bytes());
            out.extend_from_slice(&n.content);
        }
        for child in &chunk.child_chunks {
            out.extend_from_slice(&child.to_bytes());
        }
        out
    }

    fn take<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> &'a [u8] {
        let slice = &bytes[*at..*at + len];
        *at += len;
        slice
    }

    fn id_at(bytes: &[u8], at: &mut usize) -> StableId {
        StableId::from_bytes(take(bytes, at, 16).try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], at: &mut usize) -> u32 {
        u32::from_le_bytes(take(bytes, at, 4).try_into().unwrap())
    }

    /// layout을 실제 payload로 인코딩한 뒤 payload만으로 의미 snapshot을 복원합니다.
    /// payload 길이가 계획 크기와 같은지, chunk 참조가 정확히 한 번씩인지도 확인합니다.
    fn snapshot_via_payloads(doc: &[DocNode], layout: &ChunkLayout) -> Snapshot {
        let by_id: BTreeMap<StableId, &DocNode> = doc.iter().map(|n| (n.id, n)).collect();
        let mut snapshot = Snapshot::new();
        let mut chunk_roots = BTreeSet::new();
        let mut referenced: Vec<StableId> = Vec::new();
        for chunk in layout.chunks() {
            let payload = encode_chunk(chunk, &by_id);
            assert_eq!(
                payload.len() as u64,
                chunk.payload_bytes,
                "크기 모델이 실제 payload 길이와 다름"
            );
            let mut at = 0;
            chunk_roots.insert(id_at(&payload, &mut at));
            let node_count = u32_at(&payload, &mut at);
            let ref_count = u32_at(&payload, &mut at);
            for _ in 0..node_count {
                let id = id_at(&payload, &mut at);
                let parent = id_at(&payload, &mut at);
                let order = u32_at(&payload, &mut at);
                let boundary = take(&payload, &mut at, 1)[0] == 1;
                let len = u32_at(&payload, &mut at) as usize;
                let content = take(&payload, &mut at, len).to_vec();
                let parent = (parent != StableId::NIL).then_some(parent);
                assert!(
                    snapshot
                        .insert(id, (parent, order, boundary, content))
                        .is_none(),
                    "노드가 두 chunk에 기록됨"
                );
            }
            for _ in 0..ref_count {
                referenced.push(id_at(&payload, &mut at));
            }
            assert_eq!(at, payload.len(), "잉여 bytes");
        }
        referenced.sort();
        let expected: Vec<StableId> = layout.chunks()[1..].iter().map(|c| c.root).collect();
        let mut expected = expected;
        expected.sort();
        assert_eq!(
            referenced, expected,
            "모든 하위 chunk가 정확히 한 번 참조됨"
        );
        assert_eq!(chunk_roots.len(), layout.chunks().len());
        snapshot
    }

    fn source_snapshot(doc: &[DocNode]) -> Snapshot {
        doc.iter()
            .map(|n| (n.id, (n.parent, n.order, n.boundary, n.content.clone())))
            .collect()
    }

    /// 모든 의미 경계를 chunk root로 둔 layout(편집으로 작아진 문서를 모사).
    fn all_boundary_roots(tree: &SplitTree) -> BTreeSet<StableId> {
        tree.nodes()[1..]
            .iter()
            .filter(|n| n.boundary)
            .map(|n| n.id)
            .collect()
    }

    // --- 병합 규칙 ----------------------------------------------------------------

    #[test]
    fn merge_keeps_semantic_snapshot_identical() {
        // 검증 기준(V.6.5): merge 전후 의미 snapshot 동일.
        let cost = ChunkCostModel {
            chunk_base_bytes: HEADER_BYTES,
            child_ref_bytes: REF_BYTES,
        };
        let mut merged_any = false;
        for seed in 0..40u64 {
            let doc = random_document(seed, 20 + (seed as usize * 13) % 180);
            let tree = split_tree_of(&doc);
            for threshold in [1_024u64, 4_096, 32_768] {
                let policy = SplitPolicy {
                    threshold_bytes: threshold,
                    cost,
                };
                let roots = all_boundary_roots(&tree);
                let before = materialize_layout(&tree, &policy, &roots).unwrap();
                let plan = plan_chunk_merges(&tree, &policy, &roots).unwrap();
                merged_any |= !plan.is_noop();
                assert_eq!(
                    plan.layout().chunks().len() + plan.absorbed().len(),
                    before.chunks().len()
                );

                let source = source_snapshot(&doc);
                assert_eq!(snapshot_via_payloads(&doc, &before), source);
                assert_eq!(
                    snapshot_via_payloads(&doc, plan.layout()),
                    source,
                    "seed {seed}, threshold {threshold}: 병합 후 의미 snapshot이 다름"
                );
            }
        }
        assert!(merged_any, "병합이 한 번도 일어나지 않아 검사가 공허합니다");
    }

    #[test]
    fn small_child_chunks_merge_into_parent_up_to_threshold() {
        // R > A, B, C(모두 Frame, 각 30). 역순으로 C, B를 흡수하고 A는 넘쳐서 남습니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 30),
            node(3, Some(1), true, 30),
            node(4, Some(1), true, 30),
        ])
        .unwrap();
        let policy = policy(80, 0, 5);
        let plan = plan_chunk_merges(&tree, &policy, &set(&[2, 3, 4])).unwrap();
        assert_eq!(roots_of(plan.layout()), vec![sid(1), sid(2)]);
        assert_eq!(
            plan.absorbed(),
            &[
                ChunkAbsorption {
                    chunk_root: sid(3),
                    into: sid(1)
                },
                ChunkAbsorption {
                    chunk_root: sid(4),
                    into: sid(1)
                },
            ]
        );
        assert_eq!(plan.layout().root_chunk().payload_bytes, 75);
        assert_layout_invariants(&tree, &policy, plan.layout());
    }

    #[test]
    fn merge_boundary_uses_approved_threshold_inclusively() {
        // 승인 임계치(262,144) 기준: 병합 결과 == T는 병합, T + 1은 병합하지 않습니다.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/performance-budgets.yaml"
        );
        let budget = ChunkBudget::from_budget_file(Path::new(path)).unwrap();
        let cost = ChunkCostModel {
            chunk_base_bytes: 20,
            child_ref_bytes: 32,
        };
        let policy = SplitPolicy::from_budget(&budget, cost);
        let t = policy.threshold_bytes;
        // 병합 결과 = 20 + R + A.
        let a_at = t - 20 - 100_000;
        for (a, merges) in [(a_at, true), (a_at + 1, false)] {
            let tree = SplitTree::new(vec![
                node(1, None, true, 100_000),
                node(2, Some(1), true, a),
            ])
            .unwrap();
            let plan = plan_chunk_merges(&tree, &policy, &set(&[2])).unwrap();
            assert_eq!(!plan.is_noop(), merges, "A = {a}");
            assert!(plan.layout().chunks().iter().all(|c| !c.oversized));
        }
    }

    #[test]
    fn chain_merge_resolves_final_target_and_remaps() {
        // R > A > B: B → A, A → R. B의 최종 대상은 R입니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 10),
            node(3, Some(2), true, 10),
        ])
        .unwrap();
        let policy = policy(100, 0, 1);
        let plan = plan_chunk_merges(&tree, &policy, &set(&[2, 3])).unwrap();
        assert_eq!(roots_of(plan.layout()), vec![sid(1)]);
        assert_eq!(
            plan.absorbed(),
            &[
                ChunkAbsorption {
                    chunk_root: sid(2),
                    into: sid(1)
                },
                ChunkAbsorption {
                    chunk_root: sid(3),
                    into: sid(1)
                },
            ]
        );
        let manifest = plan.remap_manifest(ChunkId::from_stable_id).unwrap();
        assert_eq!(manifest.len(), 2);
        let live: BTreeSet<ChunkId> = [ChunkId::from_stable_id(sid(1))].into_iter().collect();
        assert_eq!(manifest.validate(&live), Ok(()));
        assert_eq!(
            manifest.resolve(ChunkId::from_stable_id(sid(3))),
            Some(ChunkId::from_stable_id(sid(1)))
        );
    }

    #[test]
    fn oversized_chunks_are_never_grown() {
        // Big은 임계치보다 큽니다. Big으로는 흡수하지 않고 Big도 R로 흡수되지 않습니다.
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 1_000),
            node(3, Some(2), true, 5),
            node(4, Some(1), true, 10),
        ])
        .unwrap();
        let policy = policy(100, 0, 4);
        let plan = plan_chunk_merges(&tree, &policy, &set(&[2, 3, 4])).unwrap();
        assert_eq!(roots_of(plan.layout()), vec![sid(1), sid(2), sid(3)]);
        let big = &plan.layout().chunks()[1];
        assert!(big.oversized);
        assert_eq!(big.payload_bytes, 1_004);
    }

    #[test]
    fn merge_reaches_maximal_fixpoint_with_split_planner() {
        // 무작위 tree: 결과는 불변식·결정론을 지키고, 더 흡수할 chunk가 없으며,
        // 다음 저장의 split(이전 layout = 병합 결과)이 그대로 유지합니다(진동 없음).
        let cost = ChunkCostModel {
            chunk_base_bytes: 20,
            child_ref_bytes: 32,
        };
        for seed in 0..60u64 {
            let tree = random_tree(seed, 1 + (seed as usize * 11) % 200, 5_000);
            for (split_threshold, merge_threshold) in [(2_000u64, 2_000u64), (2_000, 20_000)] {
                let split_policy = policy(split_threshold, cost.chunk_base_bytes, 32);
                let merge_policy = policy(merge_threshold, cost.chunk_base_bytes, 32);
                for roots in [
                    all_boundary_roots(&tree),
                    plan_chunk_split(&tree, &split_policy, &BTreeSet::new())
                        .unwrap()
                        .layout
                        .split_roots(),
                ] {
                    let before = materialize_layout(&tree, &merge_policy, &roots).unwrap();
                    let plan = plan_chunk_merges(&tree, &merge_policy, &roots).unwrap();
                    assert_layout_invariants(&tree, &merge_policy, plan.layout());
                    assert_eq!(
                        plan_chunk_merges(&tree, &merge_policy, &roots).unwrap(),
                        plan
                    );
                    // 임계치를 넘는 chunk의 내용은 병합 전과 같습니다(병합이 키우지 않음).
                    // 부모 chunk가 흡수되면 parent_chunk만 바뀔 수 있습니다.
                    for chunk in plan.layout().oversized() {
                        let old = before
                            .chunks()
                            .iter()
                            .find(|c| c.root == chunk.root)
                            .unwrap();
                        assert_eq!(old.nodes, chunk.nodes);
                        assert_eq!(old.child_chunks, chunk.child_chunks);
                        assert_eq!(old.payload_bytes, chunk.payload_bytes);
                    }
                    // 최대성: 남은 자식 chunk는 어느 것도 부모에 흡수할 수 없습니다.
                    let after = plan.layout();
                    for chunk in &after.chunks()[1..] {
                        let parent = after
                            .chunks()
                            .iter()
                            .find(|c| Some(c.root) == chunk.parent_chunk)
                            .unwrap();
                        let merged = parent.payload_bytes - 32 + chunk.payload_bytes - 20;
                        assert!(merged > merge_threshold);
                    }
                    // split 고정점(저장 경로가 병합 결과를 되돌리거나 다시 나누지 않음).
                    let resplit =
                        plan_chunk_split(&tree, &merge_policy, &after.split_roots()).unwrap();
                    assert_eq!(&resplit.layout, after, "seed {seed}");
                    // 이미 병합된 layout은 다시 병합할 것이 없습니다.
                    assert!(
                        plan_chunk_merges(&tree, &merge_policy, &after.split_roots())
                            .unwrap()
                            .is_noop()
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_layout_is_rejected() {
        let tree =
            SplitTree::new(vec![node(1, None, true, 0), node(2, Some(1), false, 0)]).unwrap();
        assert_eq!(
            plan_chunk_merges(&tree, &policy(10, 0, 0), &set(&[2])),
            Err(SplitError::NotBoundary { node: sid(2) })
        );
    }

    #[test]
    fn stale_plan_is_detected() {
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 10),
            node(3, Some(1), true, 10),
        ])
        .unwrap();
        let plan = plan_chunk_merges(&tree, &policy(100, 0, 1), &set(&[2, 3])).unwrap();
        assert_eq!(plan.check_base(&set(&[2, 3])), Ok(()));
        assert_eq!(plan.check_base(&set(&[2])), Err(StaleMergePlan));
        assert!(!StaleMergePlan.to_string().is_empty());
    }

    // --- 저장과의 배타 -------------------------------------------------------------

    fn small_fixture() -> (SplitTree, SplitPolicy, BTreeSet<StableId>) {
        let tree = SplitTree::new(vec![
            node(1, None, true, 10),
            node(2, Some(1), true, 10),
            node(3, Some(1), true, 10),
        ])
        .unwrap();
        (tree, policy(100, 0, 1), set(&[2, 3]))
    }

    #[test]
    fn merge_does_not_start_while_save_in_progress() {
        let (tree, policy, roots) = small_fixture();
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(Duration::ZERO, t0);
        {
            let _save = scheduler.begin_save(t0);
            assert_eq!(
                run_idle_merge(&scheduler, t0, &tree, &policy, &roots).unwrap(),
                IdleMergeOutcome::NotStarted
            );
        }
        let outcome = run_idle_merge(&scheduler, t0, &tree, &policy, &roots).unwrap();
        let IdleMergeOutcome::Ready(plan) = outcome else {
            panic!("저장 종료 후에는 병합이 실행되어야 합니다: {outcome:?}");
        };
        assert_eq!(plan, plan_chunk_merges(&tree, &policy, &roots).unwrap());
    }

    #[test]
    fn merge_waits_for_idle_threshold() {
        let (tree, policy, roots) = small_fixture();
        let idle = Duration::from_secs(30);
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(idle, t0);
        assert_eq!(
            run_idle_merge(&scheduler, t0 + idle / 2, &tree, &policy, &roots).unwrap(),
            IdleMergeOutcome::NotStarted
        );
        assert!(matches!(
            run_idle_merge(&scheduler, t0 + idle, &tree, &policy, &roots).unwrap(),
            IdleMergeOutcome::Ready(_)
        ));
    }

    #[test]
    fn merge_and_gc_are_mutually_exclusive() {
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(Duration::ZERO, t0);
        let window = begin_idle_merge(&scheduler, t0).unwrap();
        assert!(scheduler.try_begin_gc(t0).is_none(), "병합 중 GC 시작 금지");
        assert!(begin_idle_merge(&scheduler, t0).is_none(), "병합은 하나씩");
        drop(window);
        let gc = scheduler.try_begin_gc(t0).unwrap();
        assert!(
            begin_idle_merge(&scheduler, t0).is_none(),
            "GC 중 병합 시작 금지"
        );
        drop(gc);
    }

    #[test]
    fn save_started_during_merge_supersedes_result() {
        let (tree, policy, roots) = small_fixture();
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(Duration::ZERO, t0);
        let window = begin_idle_merge(&scheduler, t0).unwrap();
        let plan = window.plan(&tree, &policy, &roots).unwrap();
        // 계획 중 저장이 시작되고 끝났어도 결과는 폐기합니다.
        drop(scheduler.begin_save(t0));
        assert_eq!(window.finish(plan), IdleMergeOutcome::Superseded);
    }

    #[test]
    fn concurrent_saves_never_overlap_an_accepted_merge() {
        // 검증: 창이 열린 뒤 저장이 시작됐으면 결과를 받아들이지 않고(Superseded), 시작되지
        // 않았으면 받아들입니다(Ready). 두 방향 모두 정확해야 합니다.
        //
        // 저장 스레드는 read gate 안에서 저장 guard를 잡고 `saves_started`를 올립니다. 병합
        // 스레드는 write gate를 잡은 동안 창을 열고 확정하므로, 그 두 시점의 counter 차이는
        // scheduler가 본 저장 시작 수와 정확히 같습니다.
        let (tree, policy, roots) = small_fixture();
        let scheduler = GcScheduler::new(Duration::ZERO, Instant::now());
        let gate = std::sync::RwLock::new(());
        let saves_started = AtomicUsize::new(0);
        let saves_active = AtomicUsize::new(0);
        let savers_done = AtomicUsize::new(0);
        let violation = AtomicBool::new(false);
        let superseded = AtomicUsize::new(0);
        const SAVERS: usize = 4;

        std::thread::scope(|scope| {
            for _ in 0..SAVERS {
                scope.spawn(|| {
                    for _ in 0..200 {
                        let guard = {
                            let _read = gate.read().expect("poisoned");
                            let guard = scheduler.begin_save(Instant::now());
                            saves_started.fetch_add(1, Ordering::SeqCst);
                            saves_active.fetch_add(1, Ordering::SeqCst);
                            guard
                        };
                        for _ in 0..200 {
                            std::hint::spin_loop();
                        }
                        std::thread::yield_now();
                        saves_active.fetch_sub(1, Ordering::SeqCst);
                        drop(guard);
                    }
                    savers_done.fetch_add(1, Ordering::SeqCst);
                });
            }
            scope.spawn(|| {
                while savers_done.load(Ordering::SeqCst) < SAVERS {
                    let opened = {
                        let _write = gate.write().expect("poisoned");
                        // 진행 중 저장이 끝나기를 기다립니다(저장 종료에는 gate가 필요 없음).
                        // counter가 0이어도 guard drop 직전일 수 있으므로 창 열기는 실패할 수
                        // 있고, 그때는 다시 시도합니다.
                        while saves_active.load(Ordering::SeqCst) > 0 {
                            std::hint::spin_loop();
                        }
                        begin_idle_merge(&scheduler, Instant::now())
                            .map(|w| (w, saves_started.load(Ordering::SeqCst)))
                    };
                    let Some((window, started_before)) = opened else {
                        std::thread::yield_now();
                        continue;
                    };
                    let plan = window.plan(&tree, &policy, &roots).unwrap();
                    // 저장 스레드가 끼어들 기회를 줍니다.
                    while saves_started.load(Ordering::SeqCst) == started_before
                        && savers_done.load(Ordering::SeqCst) < SAVERS
                    {
                        std::thread::yield_now();
                    }
                    let _write = gate.write().expect("poisoned");
                    let overlapped = saves_started.load(Ordering::SeqCst) != started_before;
                    match window.finish(plan) {
                        IdleMergeOutcome::Ready(_) if overlapped => {
                            violation.store(true, Ordering::SeqCst);
                        }
                        IdleMergeOutcome::Superseded if !overlapped => {
                            violation.store(true, Ordering::SeqCst);
                        }
                        IdleMergeOutcome::Superseded => {
                            superseded.fetch_add(1, Ordering::SeqCst);
                        }
                        IdleMergeOutcome::Ready(_) => {}
                        IdleMergeOutcome::NotStarted => violation.store(true, Ordering::SeqCst),
                    }
                }
            });
        });

        assert!(
            superseded.load(Ordering::SeqCst) > 0,
            "저장과 겹친 병합이 한 번도 없어 검사가 공허합니다"
        );
        assert!(
            !violation.load(Ordering::SeqCst),
            "저장과 겹친 병합 결과를 받아들였거나 저장 중에 시작했습니다"
        );
    }
}
