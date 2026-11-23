//! 증분 레이아웃 단일 leaf 변경 benchmark (X.7.4).
//!
//! 결정적 합성 문서(기본 200,201 node)를 [`IncrementalLayout`]으로 한 번 전체 배치한 뒤, 서로 다른 section의
//! leaf 너비를 하나씩 바꾸며 edit마다 방문 node 수(`visited` = 배치를 다시 계산한 node), 재사용 subtree 수, 측정
//! cache hit 수, 경과 시간을 기록합니다. [`LeafEditBenchmark::to_json`]이 결과 JSON을 만듭니다(외부 serializer
//! 없음). 실행은 `cargo run --release -p stellar-layout --example incremental_leaf_bench`입니다.
//!
//! # 합성 문서
//!
//! ```text
//! root(block, 40000px × 40000px)
//! └─ section × S (position: absolute, left·top, 1900px × 3900px, display: flex, flex-direction: column)
//!    └─ row × R (display: flex, column-gap: 2px, width: fit-content, flex-shrink: 0)
//!       └─ leaf × L: px 크기 box, fit-content Text(측정), width 16px Image(측정)를 섞음(flex-shrink: 0)
//! ```
//!
//! leaf 너비 변경은 row(콘텐츠 크기 flex) → section(px 크기 absolute = 배치 경계)까지 전파되므로 기대 방문은 edit당
//! 3 node(section, row, leaf)이고, 나머지 row·leaf는 재사용(cache hit)됩니다. 시간 값은 실행 환경마다 다르며 결정적 비교 대상이
//! 아닙니다(방문·재사용·cache 수는 결정적).

use std::fmt::Write as _;
use std::time::Instant;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Length;

use stellar_core_types::value::FiniteF32;

use crate::engine::compute_layout_with;
use crate::error::LayoutError;
use crate::incremental::{IncrementalLayout, LayoutEdit, UpdateStats};
use crate::input::{LayoutContent, LayoutInput, LayoutNode};
use crate::intrinsic::{DefaultMeasurer, ImageIntrinsic, MeasuredContent, TextIntrinsic};
use crate::style::{Display, FlexDirection, LayoutStyle, LengthPercentageAuto, Position, Size};

/// benchmark 이름(JSON `benchmark` 값).
pub const BENCHMARK_NAME: &str = "layout-incremental-single-leaf-edit";

/// 합성 문서와 edit 설정 (X.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafEditConfig {
    /// section 수.
    pub sections: usize,
    /// section당 row 수.
    pub rows: usize,
    /// row당 leaf 수.
    pub leaves: usize,
    /// 측정할 단일 leaf edit 수.
    pub edits: usize,
}

impl LeafEditConfig {
    /// 기본 설정: 200 section × 100 row × 9 leaf = 200,201 node, edit 64회.
    pub const DEFAULT: LeafEditConfig = LeafEditConfig {
        sections: 200,
        rows: 100,
        leaves: 9,
        edits: 64,
    };

    /// 문서 node 수(root 포함).
    pub const fn node_count(&self) -> usize {
        1 + self.sections * (1 + self.rows * (1 + self.leaves))
    }
}

impl Default for LeafEditConfig {
    fn default() -> LeafEditConfig {
        LeafEditConfig::DEFAULT
    }
}

/// node 번호 → 결정적 StableId(version 7 형식).
fn synthetic_id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// 정확히 표현되는 길이(입력은 고정 상수라 실패하지 않음).
fn length(px: f32) -> Result<Length, LayoutError> {
    Length::new(px).map_err(|_| LayoutError::NonFinite { field: "benchmark" })
}

/// leaf `(section, row, leaf)`의 id.
fn leaf_id(config: &LeafEditConfig, section: usize, row: usize, leaf: usize) -> StableId {
    synthetic_id(leaf_number(config, section, row, leaf))
}

fn section_number(config: &LeafEditConfig, section: usize) -> u64 {
    (2 + section * (1 + config.rows * (1 + config.leaves))) as u64
}

fn row_number(config: &LeafEditConfig, section: usize, row: usize) -> u64 {
    section_number(config, section) + 1 + (row * (1 + config.leaves)) as u64
}

fn leaf_number(config: &LeafEditConfig, section: usize, row: usize, leaf: usize) -> u64 {
    row_number(config, section, row) + 1 + leaf as u64
}

/// flex item main 축을 줄이지 않는 style(`flex-shrink: 0`).
fn no_shrink(style: LayoutStyle) -> LayoutStyle {
    LayoutStyle {
        flex_shrink: FiniteF32::ZERO,
        ..style
    }
}

/// 결정적 합성 문서를 만듭니다 (X.7.4).
pub fn synthetic_input(config: &LeafEditConfig) -> Result<LayoutInput, LayoutError> {
    let mut nodes = Vec::with_capacity(config.node_count());
    let section_keys = OrderKey::evenly_spaced(config.sections);
    let row_keys = OrderKey::evenly_spaced(config.rows);
    let leaf_keys = OrderKey::evenly_spaced(config.leaves);
    let mut root = LayoutNode::new(synthetic_id(1)).with_style(LayoutStyle {
        width: Size::Length(length(40_000.0)?),
        height: Size::Length(length(40_000.0)?),
        ..LayoutStyle::default()
    });
    for (section, key) in section_keys.iter().enumerate() {
        root = root.with_child(synthetic_id(section_number(config, section)), key.clone());
    }
    nodes.push(root);
    let row_style = no_shrink(LayoutStyle {
        display: Display::Flex,
        column_gap: length(2.0)?,
        width: Size::FitContent,
        ..LayoutStyle::default()
    });
    for section in 0..config.sections {
        let x = (section % 20) as f32 * 2000.0;
        let y = (section / 20) as f32 * 4000.0;
        let mut node = LayoutNode::new(synthetic_id(section_number(config, section))).with_style(
            LayoutStyle {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                position: Position::Absolute,
                left: LengthPercentageAuto::Length(length(x)?),
                top: LengthPercentageAuto::Length(length(y)?),
                width: Size::Length(length(1900.0)?),
                height: Size::Length(length(3900.0)?),
                ..LayoutStyle::default()
            },
        );
        for (row, key) in row_keys.iter().enumerate() {
            node = node.with_child(synthetic_id(row_number(config, section, row)), key.clone());
        }
        nodes.push(node);
        for row in 0..config.rows {
            let mut row_node = LayoutNode::new(synthetic_id(row_number(config, section, row)))
                .with_style(row_style.clone());
            for (leaf, key) in leaf_keys.iter().enumerate() {
                row_node = row_node.with_child(leaf_id(config, section, row, leaf), key.clone());
            }
            nodes.push(row_node);
            for leaf in 0..config.leaves {
                nodes.push(synthetic_leaf(
                    leaf_id(config, section, row, leaf),
                    leaf,
                    section + row,
                )?);
            }
        }
    }
    Ok(LayoutInput::new(synthetic_id(1), nodes))
}

/// leaf 종류: 7번째는 `fit-content` Text, 8번째는 너비 16px Image, 나머지는 px 크기 box. 모두 `flex-shrink: 0`.
fn synthetic_leaf(id: StableId, leaf: usize, salt: usize) -> Result<LayoutNode, LayoutError> {
    let width = 8.0 + ((salt + leaf) % 5) as f32;
    Ok(match leaf % 9 {
        7 => LayoutNode::new(id)
            .with_style(no_shrink(LayoutStyle {
                width: Size::FitContent,
                height: Size::FitContent,
                ..LayoutStyle::default()
            }))
            .with_content(LayoutContent::Measured(MeasuredContent::Text(
                TextIntrinsic {
                    words: vec![length(width)?, length(12.0)?, length(6.0)?],
                    space: length(3.0)?,
                    line_height: length(14.0)?,
                    ascent: length(11.0)?,
                },
            ))),
        8 => LayoutNode::new(id)
            .with_style(no_shrink(LayoutStyle {
                width: Size::Length(length(16.0)?),
                height: Size::FitContent,
                ..LayoutStyle::default()
            }))
            .with_content(LayoutContent::Measured(MeasuredContent::Image(
                ImageIntrinsic {
                    width: length(32.0)?,
                    height: length(24.0)?,
                },
            ))),
        _ => LayoutNode::new(id).with_style(no_shrink(LayoutStyle {
            width: Size::Length(length(width)?),
            height: Size::Length(length(10.0)?),
            ..LayoutStyle::default()
        })),
    })
}

/// edit 하나의 측정값 (X.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafEditSample {
    /// 바꾼 leaf.
    pub leaf: StableId,
    /// 증분 레이아웃 집계.
    pub stats: UpdateStats,
    /// edit 적용(검증 + 배치) 경과 시간(ns).
    pub duration_ns: u64,
}

/// benchmark 결과 (X.7.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafEditBenchmark {
    /// 설정.
    pub config: LeafEditConfig,
    /// 문서 node 수.
    pub nodes: usize,
    /// 초기 전체 레이아웃 집계.
    pub initial: UpdateStats,
    /// 초기 전체 레이아웃 시간(ns).
    pub initial_ns: u64,
    /// edit별 측정값.
    pub samples: Vec<LeafEditSample>,
    /// 마지막 입력을 증분 상태 없이 전체 계산([`compute_layout_with`])한 시간(ns, 비교 기준).
    pub full_compute_ns: u64,
}

/// 경과 시간(ns, `u64` 포화).
fn elapsed_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// benchmark를 실행합니다 (X.7.4). edit `i`는 section `i % S`, row `(i * 7) % R`, leaf 0의 너비를 1px 늘립니다.
pub fn run_leaf_edit_benchmark(config: &LeafEditConfig) -> Result<LeafEditBenchmark, LayoutError> {
    let input = synthetic_input(config)?;
    let nodes = input.nodes.len();
    let start = Instant::now();
    let (mut session, initial) = IncrementalLayout::new(input, &DefaultMeasurer)?;
    let initial_ns = elapsed_ns(start);
    let mut samples = Vec::with_capacity(config.edits);
    let edits = if config.sections == 0 || config.rows == 0 || config.leaves == 0 {
        0
    } else {
        config.edits
    };
    for edit in 0..edits {
        let section = edit % config.sections;
        let row = (edit * 7) % config.rows;
        let leaf = leaf_id(config, section, row, 0);
        let mut node = session
            .node(leaf)
            .cloned()
            .ok_or(LayoutError::UnknownNode { node: leaf })?;
        let width = match node.style.width {
            Size::Length(width) => width.get() + 1.0,
            _ => 10.0,
        };
        node.style.width = Size::Length(length(width)?);
        let start = Instant::now();
        let stats = session.apply(vec![LayoutEdit::upsert(node)], &DefaultMeasurer)?;
        let duration_ns = elapsed_ns(start);
        samples.push(LeafEditSample {
            leaf,
            stats,
            duration_ns,
        });
    }
    let start = Instant::now();
    compute_layout_with(session.input(), &DefaultMeasurer)?;
    let full_compute_ns = elapsed_ns(start);
    Ok(LeafEditBenchmark {
        config: *config,
        nodes,
        initial,
        initial_ns,
        samples,
        full_compute_ns,
    })
}

/// 정렬된 값의 백분위(nearest-rank). 비어 있으면 0.
fn percentile(sorted: &[u64], percent: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (percent * sorted.len() as u64).div_ceil(100).max(1);
    sorted[(rank as usize - 1).min(sorted.len() - 1)]
}

fn stats_json(out: &mut String, stats: &UpdateStats) {
    let _ = write!(
        out,
        "\"visited\":{},\"reused\":{},\"relayout_roots\":{},\"overflow_updates\":{},\"hashed\":{},\
         \"measured\":{},\"memo_hits\":{},\"cache_hits\":{},\"measurer_calls\":{},\"full_layouts\":{}",
        stats.arranged,
        stats.reused,
        stats.relayout_roots,
        stats.overflow_updates,
        stats.hashed,
        stats.measured,
        stats.memo_hits,
        stats.cache_hits,
        stats.measurer_calls,
        stats.full_layouts
    );
}

impl LeafEditBenchmark {
    /// 결과 JSON(한 줄 객체들을 줄바꿈으로 구분, key 순서 고정).
    ///
    /// 최상위 key: `benchmark`, `profile`(`debug`/`release`), `nodes`, `sections`, `rows`, `leaves`, `edits`,
    /// `initial`(집계 + `duration_ns`), `full_compute_ns`(전체 계산 비교 기준), `summary`(`visited_max`, `visited_p50`, `reused_total`, `cache_hits_total`,
    /// `full_layouts_total`, `duration_ns_p50`, `duration_ns_p95`, `duration_ns_max`), `samples`(edit별 `leaf`,
    /// 집계, `duration_ns`).
    pub fn to_json(&self) -> String {
        let mut visited: Vec<u64> = self.samples.iter().map(|s| s.stats.arranged).collect();
        visited.sort_unstable();
        let mut durations: Vec<u64> = self.samples.iter().map(|s| s.duration_ns).collect();
        durations.sort_unstable();
        let sum = |f: fn(&UpdateStats) -> u64| -> u64 {
            self.samples
                .iter()
                .map(|s| f(&s.stats))
                .fold(0u64, u64::saturating_add)
        };
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\n  \"benchmark\": \"{BENCHMARK_NAME}\",\n  \"profile\": \"{profile}\",\n  \"nodes\": {},\n  \
             \"sections\": {},\n  \"rows\": {},\n  \"leaves\": {},\n  \"edits\": {},\n  \"initial\": {{",
            self.nodes,
            self.config.sections,
            self.config.rows,
            self.config.leaves,
            self.samples.len()
        );
        stats_json(&mut out, &self.initial);
        let _ = writeln!(out, ",\"duration_ns\":{}}},", self.initial_ns);
        let _ = writeln!(out, "  \"full_compute_ns\": {},", self.full_compute_ns);
        let _ = write!(
            out,
            "  \"summary\": {{\"visited_max\":{},\"visited_p50\":{},\"reused_total\":{},\
             \"cache_hits_total\":{},\"full_layouts_total\":{},\"duration_ns_p50\":{},\
             \"duration_ns_p95\":{},\"duration_ns_max\":{}}},\n  \"samples\": [",
            visited.last().copied().unwrap_or(0),
            percentile(&visited, 50),
            sum(|s| s.reused),
            sum(|s| s.cache_hits),
            sum(|s| s.full_layouts),
            percentile(&durations, 50),
            percentile(&durations, 95),
            durations.last().copied().unwrap_or(0)
        );
        for (index, sample) in self.samples.iter().enumerate() {
            let separator = if index == 0 { "\n" } else { ",\n" };
            let _ = write!(out, "{separator}    {{\"leaf\":\"{}\",", sample.leaf);
            stats_json(&mut out, &sample.stats);
            let _ = write!(out, ",\"duration_ns\":{}}}", sample.duration_ns);
        }
        out.push_str("\n  ]\n}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;

    const SMALL: LeafEditConfig = LeafEditConfig {
        sections: 6,
        rows: 12,
        leaves: 9,
        edits: 10,
    };

    #[test]
    fn synthetic_document_is_valid_and_sized_as_configured() {
        let input = synthetic_input(&SMALL).unwrap();
        assert_eq!(input.nodes.len(), SMALL.node_count());
        input.validate().unwrap();
        assert_eq!(LeafEditConfig::DEFAULT.node_count(), 200_201);
        // 결정적.
        assert_eq!(synthetic_input(&SMALL).unwrap(), input);
    }

    #[test]
    fn single_leaf_edits_visit_three_nodes_and_reuse_the_rest() {
        let report = run_leaf_edit_benchmark(&SMALL).unwrap();
        assert_eq!(report.samples.len(), SMALL.edits);
        assert_eq!(report.initial.full_layouts, 1);
        assert_eq!(report.initial.arranged as usize, SMALL.node_count());
        for sample in &report.samples {
            // section, row, leaf만 다시 배치합니다.
            assert_eq!(sample.stats.arranged, 3, "{sample:?}");
            assert_eq!(sample.stats.full_layouts, 0);
            // 다른 row(11)와 같은 row의 다른 leaf(8)를 재사용합니다.
            assert_eq!(
                sample.stats.reused,
                (SMALL.rows - 1 + SMALL.leaves - 1) as u64
            );
            // 다른 row의 배치는 cache에서 읽습니다.
            assert!(
                sample.stats.cache_hits >= (SMALL.rows - 1) as u64,
                "{sample:?}"
            );
        }
        let json = report.to_json();
        for key in [
            "\"benchmark\": \"layout-incremental-single-leaf-edit\"",
            "\"nodes\": 727",
            "\"visited\":3",
            "\"cache_hits\":",
            "\"duration_ns\":",
            "\"visited_max\":3",
            "\"cache_hits_total\":",
            "\"duration_ns_p95\":",
            "\"full_compute_ns\":",
        ] {
            assert!(json.contains(key), "{key} 없음:\n{json}");
        }
        assert_eq!(json.matches("\"leaf\":").count(), SMALL.edits);
    }

    #[test]
    fn benchmark_edits_keep_results_equal_to_full_layout() {
        let config = LeafEditConfig { edits: 0, ..SMALL };
        let input = synthetic_input(&config).unwrap();
        let (mut session, _) = IncrementalLayout::new(input, &DefaultMeasurer).unwrap();
        for edit in 0..4 {
            let leaf = leaf_id(&config, edit % config.sections, edit, 0);
            let mut node = session.node(leaf).unwrap().clone();
            node.style.width = Size::Length(length(30.0 + edit as f32).unwrap());
            session
                .apply(vec![LayoutEdit::upsert(node)], &DefaultMeasurer)
                .unwrap();
        }
        assert_eq!(
            session.result().unwrap(),
            compute_layout(session.input()).unwrap()
        );
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[5], 95), 5);
        assert_eq!(percentile(&[1, 2, 3, 4], 50), 2);
        assert_eq!(percentile(&[1, 2, 3, 4], 95), 4);
    }
}
