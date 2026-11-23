//! 결정론적 자동 배치 (XXX.2.2, ADR-0025 결정 2.6, ADR-0094 결정 5).
//!
//! hint가 없는 node의 위치를 계산하고, hint가 있는 node는 hint 좌표에 고정합니다. 규칙은 다음과 같습니다.
//!
//! 1. 열(layer): 상태 변수로 들어오는 쓰기 edge(`write`)는 되먹임이므로 빼고, 남은 edge로 가장 긴 경로 길이를 셉니다
//!    (Kahn 순서, 같은 단계는 node 순). 그래도 남는 순환 node는 마지막 열 다음에 둡니다.
//! 2. 행(row): 열마다 node 순서((StableId, 종류) 순)로 위에서부터 채웁니다. hint node는 행을 차지하지 않습니다.
//! 3. 좌표: `x = 열 × COLUMN_WIDTH`, `y = 행 × ROW_HEIGHT`.
//!
//! 같은 입력은 언제나 같은 좌표를 냅니다. 계산은 정수 열·행 번호에 상수를 곱하는 것뿐입니다.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stellar_document::prototype::layout_hint::LayoutHints;

use crate::graph::{GraphNodeId, PrototypeGraphView};

/// 열 간격.
pub const COLUMN_WIDTH: f32 = 240.0;
/// 행 간격.
pub const ROW_HEIGHT: f32 = 120.0;

/// node 위치.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// x.
    pub x: f32,
    /// y.
    pub y: f32,
    /// hint로 고정된 위치인지.
    pub pinned: bool,
}

/// 그래프의 node 위치((StableId, 종류) 순).
pub fn layout(view: &PrototypeGraphView, hints: &LayoutHints) -> BTreeMap<GraphNodeId, Placement> {
    let ids: Vec<GraphNodeId> = view.nodes().iter().map(|n| n.id).collect();
    let mut incoming: BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>> =
        ids.iter().map(|id| (*id, BTreeSet::new())).collect();
    let mut outgoing: BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>> =
        ids.iter().map(|id| (*id, BTreeSet::new())).collect();
    for edge in view.edges() {
        if edge.to.name == "write" || edge.from.node == edge.to.node {
            continue;
        }
        if let Some(set) = incoming.get_mut(&edge.to.node) {
            set.insert(edge.from.node);
        }
        if let Some(set) = outgoing.get_mut(&edge.from.node) {
            set.insert(edge.to.node);
        }
    }
    let mut remaining: BTreeMap<GraphNodeId, usize> =
        incoming.iter().map(|(id, s)| (*id, s.len())).collect();
    let mut layer: BTreeMap<GraphNodeId, u32> = BTreeMap::new();
    let mut queue: VecDeque<GraphNodeId> = remaining
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect();
    while let Some(id) = queue.pop_front() {
        let here = incoming[&id]
            .iter()
            .filter_map(|p| layer.get(p))
            .map(|l| l + 1)
            .max()
            .unwrap_or(0);
        layer.insert(id, here);
        for next in &outgoing[&id] {
            if let Some(n) = remaining.get_mut(next) {
                *n -= 1;
                if *n == 0 {
                    queue.push_back(*next);
                }
            }
        }
    }
    let last = layer.values().copied().max().map(|l| l + 1).unwrap_or(0);
    for id in &ids {
        layer.entry(*id).or_insert(last);
    }
    let mut rows: BTreeMap<u32, u32> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for id in ids {
        if let Some(point) = hints.get(id.id) {
            out.insert(
                id,
                Placement {
                    x: point.x(),
                    y: point.y(),
                    pinned: true,
                },
            );
            continue;
        }
        let column = layer[&id];
        let row = rows.entry(column).or_insert(0);
        out.insert(
            id,
            Placement {
                x: column as f32 * COLUMN_WIDTH,
                y: *row as f32 * ROW_HEIGHT,
                pinned: false,
            },
        );
        *row += 1;
    }
    out
}
