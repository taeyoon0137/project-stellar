//! caret 위치·선택 사각형·hit test cluster mapping (X.10.3).
//!
//! 모든 계산은 [`TextLayout`](X.10.2) 줄의 glyph 위치와 BiDi logical cluster mapping(X.9.3)으로 합니다.
//!
//! - **caret 단위**: grapheme cluster 경계입니다. caret 위치는 (paragraph, byte offset, [`Affinity`])로
//!   표현합니다. BiDi 경계에서는 같은 offset이 화면의 두 곳에 해당하므로 affinity가 고릅니다.
//!   `Downstream`은 offset **뒤** cluster의 leading edge(LTR 왼쪽, RTL 오른쪽), `Upstream`은 offset **앞**
//!   cluster의 trailing edge(LTR 오른쪽, RTL 왼쪽)입니다. 줄 경계 offset도 affinity로 앞·뒤 줄을 고릅니다.
//! - **cluster 폭**: shaping cluster(glyph 묶음)의 폭입니다. ligature처럼 한 shaping cluster에 grapheme이
//!   여럿이면 폭을 grapheme 수로 정수 등분하고(RTL은 오른쪽부터), glyph가 없는 cluster는 폭 0으로 visual
//!   순서상 앞 cluster 오른쪽에 둡니다.
//! - **hit test**: 점이 든 줄(줄 사이·위·아래는 가까운 줄)을 고르고, visual 순서로 cluster를 찾아 leading
//!   절반이면 (cluster 시작, Downstream), trailing 절반이면 (cluster 끝, Upstream)을 돌려줍니다. 결과를
//!   [`caret_position`]에 넣으면 hit한 cluster 가장자리 x가 나옵니다.

use std::fmt;
use std::ops::Range;

use crate::shaping::TextDirection;
use crate::text_layout::{PositionedLine, TextLayout};
use crate::units::TextUnit;

/// caret 연관 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Affinity {
    /// offset 앞 cluster에 붙습니다.
    Upstream,
    /// offset 뒤 cluster에 붙습니다.
    #[default]
    Downstream,
}

/// 텍스트 안 caret 위치(논리).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextPosition {
    /// paragraph index.
    pub paragraph: usize,
    /// paragraph byte offset(grapheme 경계).
    pub offset: usize,
    /// 연관 방향.
    pub affinity: Affinity,
}

/// 화면 caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaretPosition {
    /// [`TextLayout::lines`] index.
    pub line: usize,
    /// x(text box 기준).
    pub x: TextUnit,
    /// 줄 위 y.
    pub top: TextUnit,
    /// 줄 높이.
    pub height: TextUnit,
    /// caret이 붙은 cluster의 방향(cluster가 없으면 LTR).
    pub direction: TextDirection,
}

/// 선택 사각형.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectionRect {
    /// [`TextLayout::lines`] index.
    pub line: usize,
    /// 왼쪽 x.
    pub x: TextUnit,
    /// 위 y.
    pub top: TextUnit,
    /// 너비.
    pub width: TextUnit,
    /// 높이.
    pub height: TextUnit,
}

/// hit test 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HitTestResult {
    /// caret 위치.
    pub position: TextPosition,
    /// [`TextLayout::lines`] index.
    pub line: usize,
    /// 맞은 grapheme cluster(빈 줄이면 `None`).
    pub cluster: Option<Range<usize>>,
    /// 점이 줄 상자와 cluster 안에 있었는지.
    pub inside: bool,
}

/// caret·선택 입력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaretError {
    /// 레이아웃에 그 paragraph가 없습니다.
    UnknownParagraph {
        /// paragraph index.
        paragraph: usize,
    },
    /// offset이 grapheme cluster 경계가 아니거나 paragraph 밖입니다.
    NotClusterBoundary {
        /// offset.
        offset: usize,
    },
    /// 선택 범위가 거꾸로입니다.
    ReversedRange {
        /// 시작.
        start: usize,
        /// 끝.
        end: usize,
    },
}

impl fmt::Display for CaretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaretError::UnknownParagraph { paragraph } => {
                write!(f, "레이아웃에 paragraph {paragraph}이(가) 없습니다")
            }
            CaretError::NotClusterBoundary { offset } => {
                write!(f, "offset {offset}이(가) grapheme cluster 경계가 아닙니다")
            }
            CaretError::ReversedRange { start, end } => {
                write!(f, "선택 범위 {start}..{end}이(가) 거꾸로입니다")
            }
        }
    }
}

impl std::error::Error for CaretError {}

/// 줄 하나의 grapheme cluster 기하(논리 cluster별 [왼쪽, 오른쪽) x, text box 기준).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineGeometry {
    /// 논리 cluster 범위.
    pub clusters: Vec<Range<usize>>,
    /// 논리 cluster별 (왼쪽, 오른쪽) x.
    pub extents: Vec<(TextUnit, TextUnit)>,
    /// 논리 cluster별 방향.
    pub directions: Vec<TextDirection>,
    /// visual 위치 → 논리 cluster.
    pub visual_to_logical: Vec<usize>,
}

/// 줄의 grapheme 기하를 계산합니다.
pub fn line_geometry(line: &PositionedLine) -> LineGeometry {
    let order = &line.order;
    let clusters = order.clusters().to_vec();
    let directions: Vec<TextDirection> = order
        .cluster_levels()
        .iter()
        .map(|l| l.direction())
        .collect();
    // shaping cluster 시작(논리 순서)과 각 묶음의 x 범위.
    let mut starts: Vec<u32> = line.glyphs.iter().map(|g| g.cluster).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut group_extent: Vec<Option<(i64, i64)>> = vec![None; starts.len()];
    for g in &line.glyphs {
        if let Ok(i) = starts.binary_search(&g.cluster) {
            let left = i64::from(g.x.raw());
            let right = left + i64::from(g.advance.raw()) + i64::from(g.justify_extra.raw());
            let entry = group_extent[i].get_or_insert((left, right));
            entry.0 = entry.0.min(left);
            entry.1 = entry.1.max(right);
        }
    }
    let group_of = |offset: usize| -> Option<usize> {
        let index = starts.partition_point(|s| *s as usize <= offset);
        index.checked_sub(1)
    };
    // 각 shaping 묶음에 속한 grapheme 목록(논리 순서).
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); starts.len()];
    for (k, cluster) in clusters.iter().enumerate() {
        if let Some(g) = group_of(cluster.start) {
            members[g].push(k);
        }
    }
    let mut extents = vec![(TextUnit::ZERO, TextUnit::ZERO); clusters.len()];
    let mut pen = i64::from(line.x.raw());
    for &k in order.visual_to_logical() {
        let extent = group_of(clusters[k].start).and_then(|g| {
            let (left, right) = group_extent[g]?;
            let share = &members[g];
            let n = share.len() as i64;
            let i = share.iter().position(|m| *m == k)? as i64;
            let width = right - left;
            Some(match directions[k] {
                TextDirection::LeftToRight => (left + width * i / n, left + width * (i + 1) / n),
                TextDirection::RightToLeft => (right - width * (i + 1) / n, right - width * i / n),
            })
        });
        let (left, right) = extent.unwrap_or((pen, pen));
        pen = right;
        extents[k] = (to_unit(left), to_unit(right));
    }
    LineGeometry {
        clusters,
        extents,
        directions,
        visual_to_logical: order.visual_to_logical().to_vec(),
    }
}

/// 레이아웃 좌표는 생성 시 `i32` 범위로 검증되었으므로 범위 안 값만 들어옵니다.
fn to_unit(value: i64) -> TextUnit {
    TextUnit::from_raw(i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX }))
}

/// paragraph의 줄 index 목록(`TextLayout::lines` 기준).
fn paragraph_lines(layout: &TextLayout, paragraph: usize) -> Result<Vec<usize>, CaretError> {
    let lines: Vec<usize> = layout
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.paragraph == paragraph)
        .map(|(i, _)| i)
        .collect();
    if lines.is_empty() {
        Err(CaretError::UnknownParagraph { paragraph })
    } else {
        Ok(lines)
    }
}

/// paragraph의 caret 정지 위치(grapheme 경계, 논리 순서, 중복 없음).
pub fn caret_stops(layout: &TextLayout, paragraph: usize) -> Result<Vec<usize>, CaretError> {
    let mut stops = Vec::new();
    for index in paragraph_lines(layout, paragraph)? {
        let line = &layout.lines[index];
        stops.push(line.range.start);
        stops.extend(line.order.clusters().iter().map(|c| c.end));
    }
    stops.dedup();
    Ok(stops)
}

/// caret 위치를 계산합니다.
pub fn caret_position(
    layout: &TextLayout,
    position: &TextPosition,
) -> Result<CaretPosition, CaretError> {
    let lines = paragraph_lines(layout, position.paragraph)?;
    let offset = position.offset;
    let candidates: Vec<usize> = lines
        .iter()
        .copied()
        .filter(|&i| {
            let r = &layout.lines[i].range;
            r.start <= offset && offset <= r.end
        })
        .collect();
    let line_index = match (candidates.as_slice(), position.affinity) {
        ([], _) => return Err(CaretError::NotClusterBoundary { offset }),
        ([only], _) => *only,
        ([first, ..], Affinity::Upstream) => *first,
        ([.., last], Affinity::Downstream) => *last,
    };
    let line = &layout.lines[line_index];
    let geometry = line_geometry(line);
    let after = geometry.clusters.iter().position(|c| c.start == offset);
    let before = geometry.clusters.iter().position(|c| c.end == offset);
    if after.is_none()
        && before.is_none()
        && !(geometry.clusters.is_empty() && offset == line.range.start)
    {
        return Err(CaretError::NotClusterBoundary { offset });
    }
    let leading = |k: usize| match geometry.directions[k] {
        TextDirection::LeftToRight => geometry.extents[k].0,
        TextDirection::RightToLeft => geometry.extents[k].1,
    };
    let trailing = |k: usize| match geometry.directions[k] {
        TextDirection::LeftToRight => geometry.extents[k].1,
        TextDirection::RightToLeft => geometry.extents[k].0,
    };
    let (x, direction) = match (position.affinity, after, before) {
        (Affinity::Downstream, Some(k), _) | (Affinity::Upstream, Some(k), None) => {
            (leading(k), geometry.directions[k])
        }
        (Affinity::Upstream, _, Some(k)) | (Affinity::Downstream, None, Some(k)) => {
            (trailing(k), geometry.directions[k])
        }
        (_, None, None) => (line.x, TextDirection::LeftToRight),
    };
    Ok(CaretPosition {
        line: line_index,
        x,
        top: line.top,
        height: line.height,
        direction,
    })
}

/// paragraph 안 `[start, end)` 선택의 사각형(줄 순서, 줄 안 visual 순서, 인접 구간 병합).
pub fn selection_rects(
    layout: &TextLayout,
    paragraph: usize,
    range: Range<usize>,
) -> Result<Vec<SelectionRect>, CaretError> {
    if range.start > range.end {
        return Err(CaretError::ReversedRange {
            start: range.start,
            end: range.end,
        });
    }
    let stops = caret_stops(layout, paragraph)?;
    for offset in [range.start, range.end] {
        if stops.binary_search(&offset).is_err() {
            return Err(CaretError::NotClusterBoundary { offset });
        }
    }
    let mut rects = Vec::new();
    for index in paragraph_lines(layout, paragraph)? {
        let line = &layout.lines[index];
        let geometry = line_geometry(line);
        let mut current: Option<(TextUnit, TextUnit)> = None;
        for &k in &geometry.visual_to_logical {
            let cluster = &geometry.clusters[k];
            let selected = range.start <= cluster.start && cluster.end <= range.end;
            if selected {
                let (left, right) = geometry.extents[k];
                current = match current {
                    Some((l, r)) if r == left => Some((l, right)),
                    Some((l, r)) => {
                        rects.push(rect(index, line, l, r));
                        Some((left, right))
                    }
                    None => Some((left, right)),
                };
            } else if let Some((l, r)) = current.take() {
                rects.push(rect(index, line, l, r));
            }
        }
        if let Some((l, r)) = current {
            rects.push(rect(index, line, l, r));
        }
    }
    Ok(rects)
}

fn rect(index: usize, line: &PositionedLine, left: TextUnit, right: TextUnit) -> SelectionRect {
    SelectionRect {
        line: index,
        x: left,
        top: line.top,
        width: TextUnit::from_raw(right.raw() - left.raw()),
        height: line.height,
    }
}

/// 점 `(x, y)`(text box 기준)에 가장 가까운 caret 위치를 찾습니다. 줄이 없으면 `None`.
pub fn hit_test(layout: &TextLayout, x: TextUnit, y: TextUnit) -> Option<HitTestResult> {
    let first = layout.lines.first()?;
    let line_index = layout.lines.iter().rposition(|l| l.top <= y).unwrap_or(0);
    let line = &layout.lines[line_index];
    let inside_line =
        line.top <= y && y.raw() < line.top.raw() + line.height.raw() && first.top <= y;
    let geometry = line_geometry(line);
    let position = |offset: usize, affinity: Affinity| TextPosition {
        paragraph: line.paragraph,
        offset,
        affinity,
    };
    let Some((&leftmost, &rightmost)) = geometry
        .visual_to_logical
        .first()
        .zip(geometry.visual_to_logical.last())
    else {
        return Some(HitTestResult {
            position: position(line.range.start, Affinity::Downstream),
            line: line_index,
            cluster: None,
            inside: false,
        });
    };
    // (cluster, leading 절반인지) → caret.
    let edge = |k: usize, leading_half: bool| {
        let cluster = geometry.clusters[k].clone();
        if leading_half {
            (position(cluster.start, Affinity::Downstream), cluster)
        } else {
            (position(cluster.end, Affinity::Upstream), cluster)
        }
    };
    let is_ltr = |k: usize| geometry.directions[k] == TextDirection::LeftToRight;
    if x < geometry.extents[leftmost].0 {
        // visual 왼쪽 가장자리: LTR이면 leading, RTL이면 trailing.
        let (pos, cluster) = edge(leftmost, is_ltr(leftmost));
        return Some(HitTestResult {
            position: pos,
            line: line_index,
            cluster: Some(cluster),
            inside: false,
        });
    }
    for &k in &geometry.visual_to_logical {
        let (left, right) = geometry.extents[k];
        if left <= x && x < right {
            let mid = left.raw() + (right.raw() - left.raw()) / 2;
            let left_half = x.raw() < mid;
            // LTR 왼쪽 절반과 RTL 오른쪽 절반이 leading입니다.
            let (pos, cluster) = edge(k, left_half == is_ltr(k));
            return Some(HitTestResult {
                position: pos,
                line: line_index,
                cluster: Some(cluster),
                inside: inside_line,
            });
        }
    }
    // visual 오른쪽 가장자리 너머: LTR이면 trailing, RTL이면 leading.
    let (pos, cluster) = edge(rightmost, !is_ltr(rightmost));
    Some(HitTestResult {
        position: pos,
        line: line_index,
        cluster: Some(cluster),
        inside: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::BaseDirection;
    use crate::paragraph::tests::Fixture;
    use crate::text_layout::tests::breaks;
    use crate::text_layout::{compose_text, lay_out_paragraph, ParagraphLayoutStyle, TextBox};

    fn layout(text: &str, base: BaseDirection, width: Option<i32>) -> TextLayout {
        let fx = Fixture::new();
        let shaped = fx.shape(text, 16, base);
        let width = width.map(|px| TextUnit::from_px(px).expect("px"));
        let lines = lay_out_paragraph(
            &shaped,
            &breaks(text),
            &ParagraphLayoutStyle::default(),
            width,
        )
        .expect("lines");
        compose_text(&[&lines], TextUnit::ZERO, &TextBox::default()).expect("compose")
    }

    fn caret_x(layout: &TextLayout, offset: usize, affinity: Affinity) -> i32 {
        caret_position(
            layout,
            &TextPosition {
                paragraph: 0,
                offset,
                affinity,
            },
        )
        .expect("caret")
        .x
        .raw()
    }

    #[test]
    fn bidi_caret_fixture() {
        // "abc אבג def" LTR. visual: a b c ␠ ג ב א ␠ d e f.
        // x: a 0, b 512, c 1024, ␠ 1536..1792, ג 1792..2253, ב 2253..2714, א 2714..3175, ␠ 3175..3431, d 3431.
        let layout = layout("abc אבג def", BaseDirection::LeftToRight, None);
        // 경계 3|4 (␠ | א): Downstream은 א의 leading(오른쪽 3175), Upstream은 ␠의 trailing(1792).
        assert_eq!(caret_x(&layout, 4, Affinity::Downstream), 3175);
        assert_eq!(caret_x(&layout, 4, Affinity::Upstream), 1792);
        // 경계 ג|␠ (offset 10): Downstream은 ␠ leading(3175), Upstream은 ג trailing(왼쪽 1792).
        assert_eq!(caret_x(&layout, 10, Affinity::Downstream), 3175);
        assert_eq!(caret_x(&layout, 10, Affinity::Upstream), 1792);
        // RTL 안쪽(א|ב, offset 6)은 affinity와 무관하게 2714.
        assert_eq!(caret_x(&layout, 6, Affinity::Downstream), 2714);
        assert_eq!(caret_x(&layout, 6, Affinity::Upstream), 2714);
        // 줄 시작·끝.
        assert_eq!(caret_x(&layout, 0, Affinity::Upstream), 0);
        // d·e 512, f(좁은 글자 300fu) 307.
        assert_eq!(
            caret_x(&layout, 14, Affinity::Downstream),
            3431 + 512 * 2 + 307
        );
        let caret = caret_position(
            &layout,
            &TextPosition {
                paragraph: 0,
                offset: 4,
                affinity: Affinity::Downstream,
            },
        )
        .expect("caret");
        assert_eq!(caret.direction, TextDirection::RightToLeft);
        assert_eq!(caret.height, TextUnit::from_raw(1024));
        // caret 정지 위치는 grapheme 경계.
        assert_eq!(
            caret_stops(&layout, 0).expect("stops"),
            vec![0, 1, 2, 3, 4, 6, 8, 10, 11, 12, 13, 14]
        );
    }

    #[test]
    fn bidi_selection_is_split_visually() {
        let layout = layout("abc אבג def", BaseDirection::LeftToRight, None);
        // [3, 6) = ␠ + א: 화면에서 떨어진 두 구간.
        let rects: Vec<(i32, i32)> = selection_rects(&layout, 0, 3..6)
            .expect("rects")
            .iter()
            .map(|r| (r.x.raw(), r.width.raw()))
            .collect();
        assert_eq!(rects, vec![(1536, 256), (2714, 461)]);
        // 히브리어 전체 [4, 10)는 연속 한 구간.
        let rects = selection_rects(&layout, 0, 4..10).expect("rects");
        assert_eq!(rects.len(), 1);
        assert_eq!((rects[0].x.raw(), rects[0].width.raw()), (1792, 461 * 3));
        assert!(selection_rects(&layout, 0, 2..2)
            .expect("빈 선택")
            .is_empty());
        assert_eq!(
            selection_rects(&layout, 0, 5..6),
            Err(CaretError::NotClusterBoundary { offset: 5 })
        );
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 6..3;
        assert_eq!(
            selection_rects(&layout, 0, reversed),
            Err(CaretError::ReversedRange { start: 6, end: 3 })
        );
    }

    #[test]
    fn hit_test_maps_points_to_clusters_and_back_to_carets() {
        let layout = layout("abc אבג def", BaseDirection::LeftToRight, None);
        let y = TextUnit::from_raw(500);
        let hit = |x: i32| hit_test(&layout, TextUnit::from_raw(x), y).expect("hit");
        // ג(1792..2253)의 왼쪽 절반 = RTL trailing → (10, Upstream).
        let h = hit(1800);
        assert_eq!(
            (h.position.offset, h.position.affinity),
            (10, Affinity::Upstream)
        );
        assert_eq!(h.cluster, Some(8..10));
        assert!(h.inside);
        // א(2714..3175)의 오른쪽 절반 = RTL leading → (4, Downstream).
        let h = hit(3170);
        assert_eq!(
            (h.position.offset, h.position.affinity),
            (4, Affinity::Downstream)
        );
        // b(512..1024) 왼쪽 절반 → (1, Downstream), 오른쪽 절반 → (2, Upstream).
        assert_eq!(hit(600).position.offset, 1);
        assert_eq!(hit(900).position.offset, 2);
        // 왕복: hit 결과 caret x는 맞은 cluster 가장자리입니다.
        for x in [10, 600, 900, 1700, 1800, 2300, 3170, 3500, 4000] {
            let h = hit(x);
            let cx = caret_position(&layout, &h.position).expect("caret").x.raw();
            let cluster = h.cluster.expect("cluster");
            let geometry = line_geometry(&layout.lines[h.line]);
            let k = geometry
                .clusters
                .iter()
                .position(|c| *c == cluster)
                .expect("cluster");
            let (l, r) = geometry.extents[k];
            assert!(
                cx == l.raw() || cx == r.raw(),
                "x {x}: caret {cx} vs {l:?}..{r:?}"
            );
        }
        // 줄 밖: 왼쪽 → 첫 cluster leading(0), 오른쪽 → 끝(14).
        let left = hit(-100);
        assert_eq!((left.position.offset, left.inside), (0, false));
        let right = hit(100_000);
        assert_eq!((right.position.offset, right.inside), (14, false));
    }

    #[test]
    fn multi_line_caret_affinity_and_hit_rows() {
        // 40px 폭: "aa bb " | "cc dd".
        let layout = layout("aa bb cc dd", BaseDirection::LeftToRight, Some(40));
        assert_eq!(layout.lines.len(), 2);
        // 줄 경계 offset 6: Upstream은 첫 줄 끝, Downstream은 둘째 줄 시작.
        let up = caret_position(
            &layout,
            &TextPosition {
                paragraph: 0,
                offset: 6,
                affinity: Affinity::Upstream,
            },
        )
        .expect("up");
        let down = caret_position(
            &layout,
            &TextPosition {
                paragraph: 0,
                offset: 6,
                affinity: Affinity::Downstream,
            },
        )
        .expect("down");
        assert_eq!((up.line, down.line), (0, 1));
        assert_eq!(down.x, TextUnit::ZERO);
        // 둘째 줄 높이 위치 hit → 둘째 줄.
        let h = hit_test(&layout, TextUnit::from_raw(10), TextUnit::from_raw(1100)).expect("hit");
        assert_eq!((h.line, h.position.offset), (1, 6));
        // 위·아래 밖은 첫·마지막 줄.
        assert_eq!(
            hit_test(&layout, TextUnit::ZERO, TextUnit::from_raw(-50))
                .expect("hit")
                .line,
            0
        );
        assert_eq!(
            hit_test(&layout, TextUnit::ZERO, TextUnit::from_raw(99_999))
                .expect("hit")
                .line,
            1
        );
        assert_eq!(
            caret_position(
                &layout,
                &TextPosition {
                    paragraph: 3,
                    offset: 0,
                    affinity: Affinity::Downstream
                }
            ),
            Err(CaretError::UnknownParagraph { paragraph: 3 })
        );
    }

    #[test]
    fn ligature_cluster_is_split_evenly_and_empty_line_has_caret() {
        // "office": ffi ligature(819 subpx)는 grapheme 3개라 273씩 나뉩니다.
        let layout = layout("office", BaseDirection::LeftToRight, None);
        assert_eq!(caret_x(&layout, 1, Affinity::Downstream), 512);
        assert_eq!(caret_x(&layout, 2, Affinity::Downstream), 512 + 273);
        assert_eq!(caret_x(&layout, 3, Affinity::Downstream), 512 + 546);
        assert_eq!(caret_x(&layout, 4, Affinity::Downstream), 512 + 819);
        let empty = super::tests::layout("", BaseDirection::LeftToRight, None);
        assert_eq!(caret_x(&empty, 0, Affinity::Downstream), 0);
        let h = hit_test(&empty, TextUnit::from_raw(30), TextUnit::ZERO).expect("hit");
        assert_eq!((h.position.offset, h.cluster), (0, None));
        assert!(hit_test(
            &TextLayout {
                lines: vec![],
                width: TextUnit::ZERO,
                height: TextUnit::ZERO,
                content_height: TextUnit::ZERO,
                content_top: TextUnit::ZERO
            },
            TextUnit::ZERO,
            TextUnit::ZERO
        )
        .is_none());
    }

    #[test]
    fn rtl_paragraph_caret_edges() {
        // RTL 문단 "אבג": 오른쪽 정렬이 아닌 auto 폭이라 x 0부터. visual ג ב א.
        let layout = layout("אבג", BaseDirection::RightToLeft, None);
        // 논리 시작(offset 0, א 앞)은 א의 leading = 오른쪽 끝(1383).
        assert_eq!(caret_x(&layout, 0, Affinity::Downstream), 1383);
        // 논리 끝(6)은 ג trailing = 왼쪽 끝(0).
        assert_eq!(caret_x(&layout, 6, Affinity::Upstream), 0);
        // 화면 왼쪽 밖을 누르면 논리 끝.
        let h = hit_test(&layout, TextUnit::from_raw(-10), TextUnit::ZERO).expect("hit");
        assert_eq!(h.position.offset, 6);
        assert!(CaretError::NotClusterBoundary { offset: 5 }
            .to_string()
            .contains('5'));
    }
}
