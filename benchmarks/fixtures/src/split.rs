//! fixture의 chunk split 계획 요약 (V.6.4 split planner 연결).
//!
//! 문서 serializer(VI.10.2)는 현재 View 하나를 subtree chunk 하나로 저장합니다. fixture가
//! 실제 저장 정책에서 어떤 chunk 모양이 될지 확인하려고, 각 View subtree를 storage split
//! planner([`plan_chunk_split`])에 넣고 `config/performance-budgets.yaml`의 임계치
//! ([`ChunkBudget`])로 계획합니다. Frame을 의미 경계로 봅니다(아키텍처 §21.1).
//!
//! 크기 모델은 subtree payload 인코딩(`SubtreeRecords::encode`)을 따릅니다: chunk마다 root id
//! 16 + node 수 4 + stack 수 4 bytes, node마다 길이 프리픽스 4 + record bytes(+ stack이 있으면
//! 4 + stack bytes), 분리된 자식 chunk 참조는 `SubtreeRef`(root id 16 + chunk id 16) bytes.

use std::collections::{BTreeMap, BTreeSet};

use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_node_record;
use stellar_storage::chunk::budget::ChunkBudget;
use stellar_storage::chunk::split::{
    plan_chunk_split, ChunkCostModel, SplitNode, SplitPolicy, SplitTree,
};

use crate::error::FixtureError;

/// subtree payload의 고정 bytes(root id + node 수 + stack 수).
pub const SUBTREE_BASE_BYTES: u64 = 16 + 4 + 4;

/// 분리된 자식 chunk 참조 bytes(SubtreeRef: root node id + chunk id).
pub const CHILD_REF_BYTES: u64 = 16 + 16;

/// 문서 하나의 split 계획 요약.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SplitSummary {
    /// View 수.
    pub views: usize,
    /// 계획된 chunk 수(모든 View 합).
    pub planned_chunks: usize,
    /// 임계치를 넘지만 의미 경계로 더 나눌 수 없는 chunk 수.
    pub oversized_chunks: usize,
    /// 가장 큰 계획 chunk의 payload bytes.
    pub max_chunk_bytes: u64,
    /// 가장 큰 View subtree 전체(분할 전) payload bytes.
    pub max_view_bytes: u64,
}

/// 예산의 임계치와 subtree 크기 모델로 만든 split 정책.
pub fn fixture_split_policy(budget: &ChunkBudget) -> SplitPolicy {
    SplitPolicy::from_budget(
        budget,
        ChunkCostModel {
            chunk_base_bytes: SUBTREE_BASE_BYTES,
            child_ref_bytes: CHILD_REF_BYTES,
        },
    )
}

/// 문서의 모든 View subtree를 split planner로 계획해 요약합니다.
pub fn summarize_split(
    model: &DocumentModel,
    policy: &SplitPolicy,
) -> Result<SplitSummary, FixtureError> {
    let records = model.to_records().map_err(FixtureError::Model)?;
    let stack_bytes: BTreeMap<_, u64> = records
        .modifier_stacks
        .iter()
        .map(|stack| (stack.node, 4 + stack.encode().len() as u64))
        .collect();
    let mut summary = SplitSummary::default();
    for (_, subtree) in records.view_subtrees().map_err(FixtureError::Partition)? {
        let nodes: Vec<SplitNode> = subtree
            .nodes
            .iter()
            .map(|node| SplitNode {
                id: node.id,
                parent: node.parent.filter(|_| node.id != subtree.root),
                boundary: node.kind() == NodeKind::Frame,
                own_bytes: 4
                    + encode_node_record(node).len() as u64
                    + stack_bytes.get(&node.id).copied().unwrap_or(0),
            })
            .collect();
        let view_bytes = SUBTREE_BASE_BYTES + nodes.iter().map(|n| n.own_bytes).sum::<u64>();
        let tree = SplitTree::new(nodes).map_err(FixtureError::SplitTree)?;
        let plan =
            plan_chunk_split(&tree, policy, &BTreeSet::new()).map_err(FixtureError::Split)?;
        summary.views += 1;
        summary.planned_chunks += plan.layout.chunks().len();
        summary.oversized_chunks += plan.layout.oversized().count();
        summary.max_view_bytes = summary.max_view_bytes.max(view_bytes);
        for chunk in plan.layout.chunks() {
            summary.max_chunk_bytes = summary.max_chunk_bytes.max(chunk.payload_bytes);
        }
    }
    Ok(summary)
}
