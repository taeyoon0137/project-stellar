//! 줄 높이·paragraph 간격·정렬 (X.10.2).
//!
//! 두 단계로 나눕니다.
//!
//! 1. [`lay_out_paragraph`] (paragraph 단위, 캐시 가능): X.10.1 줄 나누기 결과에 BiDi 줄 재배치(X.9.3)를
//!    적용해 줄마다 visual 순서 glyph([`LineGlyph`], 줄 시작 기준 x)를 놓고, 줄 높이·baseline을 정합니다.
//! 2. [`compose_text`] (text box 단위): paragraph를 위에서부터 쌓고 paragraph 간격을 넣은 뒤, **가로
//!    정렬**(paragraph 속성 [`TextAlign`])과 **text box 세로 정렬**([`VerticalAlign`] Top·Middle·Bottom)을
//!    따로 적용합니다. 두 정렬은 서로 독립입니다.
//!
//! 줄 높이(문서 `LineHeight`, VI.7.3)는 CSS 의미입니다(CSS 2 §10.8, IX.11.12, ADR-0075 개정 결정 12). 줄 상자 안의 inline
//! box(줄에 걸친 glyph run)마다 **자기 글꼴**의 ascent `A`·depth `D`로 자기 line-height `L`을 정하고, leading
//! `L − (A + D)`를 위아래로 반씩(위는 내림, 나머지는 아래) 더합니다. 문단 root inline box의 strut([`Strut`], 문단
//! typography의 첫 사용 가능 글꼴과 font-size)도 같은 규칙의 inline box 하나입니다. 모든 box는 baseline에 맞추므로 줄
//! 높이는 baseline 위로 가장 높은 box와 아래로 가장 깊은 box의 합이고, baseline은 위쪽 최댓값입니다. 그래서 크기가 섞인
//! 줄은 지정 line-height보다 클 수 있습니다.
//! - `Auto`(`normal`): `L = A + D + line gap`(box 자기 글꼴 metric).
//! - `Multiplier`(`<number>`): `L` = box **자기** 글꼴 크기 × 배율(1/1024 정수 배율, 반올림).
//! - `Fixed`(`<length>`): `L` = 고정 길이(box마다 같음).
//!
//! 크기가 하나인 문단(run과 strut의 metric·크기가 같음)의 결과는 이전 규칙(줄 전체 높이 하나와 최대 ascent·descent)과
//! 같습니다.
//!
//! 가로 정렬은 paragraph 기준 방향 기준입니다(Start = LTR 왼쪽·RTL 오른쪽). `Justify`는 마지막 줄과 강제
//! 줄바꿈 줄을 제외한 줄에서 줄 안 공백 segment에 남는 폭을 정수로 나눠 주고, 나머지 subpixel은 visual
//! 순서 앞쪽 공백부터 1씩 더합니다. 공백이 없으면 Start입니다. 모든 계산은 정수 subpixel입니다.

use std::ops::Range;

use crate::bidi::{BidiLevel, LineOrder};
use crate::line_break::BreakOpportunity;
use crate::line_layout::{break_lines, LineBreak, WrapMode};
use crate::paragraph::{
    GlyphRun, PositionedGlyph, ScaledLineMetrics, ShapedParagraph, TextLayoutError,
};
use crate::shaping::GlyphId;
use crate::units::{FontSize, TextUnit, UnitError};

/// 가로 정렬(문서 `TextAlignment`, VI.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    /// 쓰기 방향 시작.
    #[default]
    Start,
    /// 가운데.
    Center,
    /// 쓰기 방향 끝.
    End,
    /// 양쪽(마지막 줄은 Start).
    Justify,
}

/// 줄 높이 배율(1/1024 단위 정수). `1024`가 1배입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineHeightRatio(u32);

impl LineHeightRatio {
    /// 1배.
    pub const ONE: LineHeightRatio = LineHeightRatio(1024);

    /// raw 값(1/1024 단위)으로 만듭니다. 0은 거부합니다.
    pub fn from_raw(per_1024: u32) -> Option<LineHeightRatio> {
        (per_1024 > 0).then_some(LineHeightRatio(per_1024))
    }

    /// raw 값.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// 줄 높이(문서 `LineHeight`, VI.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineHeight {
    /// 글꼴 metric 기반.
    #[default]
    Auto,
    /// 글꼴 크기 배수.
    Multiplier(LineHeightRatio),
    /// 고정 높이.
    Fixed(TextUnit),
}

/// paragraph 레이아웃 속성(문서 `ParagraphStyle`, VI.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ParagraphLayoutStyle {
    /// 가로 정렬.
    pub align: TextAlign,
    /// 줄 높이.
    pub line_height: LineHeight,
    /// 줄바꿈 방식.
    pub wrap: WrapMode,
}

/// text box 세로 정렬(paragraph 가로 정렬과 별개).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VerticalAlign {
    /// 위.
    #[default]
    Top,
    /// 가운데(넘치면 위로도 넘침).
    Middle,
    /// 아래(넘치면 위로 넘침).
    Bottom,
}

/// text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextBox {
    /// 너비(없으면 내용 너비 = 가장 긴 줄).
    pub width: Option<TextUnit>,
    /// 높이(없으면 내용 높이).
    pub height: Option<TextUnit>,
    /// 세로 정렬.
    pub vertical_align: VerticalAlign,
}

/// 줄 위 glyph(줄 시작 기준, 정렬 전).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineGlyph {
    /// glyph.
    pub glyph: GlyphId,
    /// paragraph byte offset(cluster 시작).
    pub cluster: u32,
    /// 줄 시작 기준 pen x(정렬·양쪽 정렬 전).
    pub x: TextUnit,
    /// advance.
    pub advance: TextUnit,
    /// glyph 가로 offset.
    pub x_offset: TextUnit,
    /// glyph 세로 offset(위가 양수).
    pub y_offset: TextUnit,
    /// glyph run index.
    pub run: usize,
    /// 양쪽 정렬 공간을 받는 공백 segment의 glyph인지.
    pub expansion: bool,
}

/// paragraph 안 줄 하나(정렬 전).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphLine {
    /// 줄 나누기 결과.
    pub line: LineBreak,
    /// BiDi visual 순서.
    pub order: LineOrder,
    /// visual 순서 glyph.
    pub glyphs: Vec<LineGlyph>,
    /// 줄 높이.
    pub height: TextUnit,
    /// 줄 위에서 baseline까지.
    pub baseline: TextUnit,
    /// 줄 metric(최대 ascent·descent).
    pub metrics: ScaledLineMetrics,
}

/// Glyph 번호를 해석할 face와 style. glyph마다 복제하지 않고 run별로 보관합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRun {
    /// 실제 shaping instance. 해당 run에 사용 가능한 face가 없으면 None.
    pub face: Option<crate::font::FontFaceRef>,
    /// shaping에 사용한 글꼴 크기.
    pub size: FontSize,
    /// 입력 style index.
    pub style: u32,
}

/// paragraph 하나의 줄 목록 (X.10.2 1단계 결과, 캐시 단위).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphLines {
    /// glyph의 run index 순서.
    pub runs: std::sync::Arc<[PlacedRun]>,
    /// 줄.
    pub lines: Vec<ParagraphLine>,
    /// 가로 정렬.
    pub align: TextAlign,
    /// paragraph 기준 level(정렬 방향).
    pub base_level: BidiLevel,
    /// 높이 합.
    pub height: TextUnit,
}

/// paragraph를 줄로 나누고 줄 높이·baseline·visual glyph를 정합니다 (X.10.2 1단계).
///
/// 줄마다 하는 일은 그 줄의 크기(glyph·segment·run 수)에 비례합니다. run·segment·BiDi paragraph는 논리
/// 순서로 정렬되어 있어 줄과 겹치는 구간을 이분 탐색으로 찾고, run glyph는 cluster 정렬 표([`ClusterIndex`])
/// 로 찾습니다. paragraph 전체 비용은 O(n log n)입니다.
pub fn lay_out_paragraph(
    shaped: &ShapedParagraph,
    breaks: &[BreakOpportunity],
    style: &ParagraphLayoutStyle,
    max_width: Option<TextUnit>,
) -> Result<ParagraphLines, TextLayoutError> {
    lay_out_paragraph_with_strut(shaped, breaks, style, Strut::of(shaped), max_width)
}

/// root inline box strut: 문단 typography의 첫 사용 가능 글꼴 metric과 font-size(CSS 2 §10.8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Strut {
    /// 글꼴 metric(scaled).
    pub metrics: ScaledLineMetrics,
    /// 글꼴 크기. 없으면 `Multiplier`는 metric 높이(`A + D`)를 씁니다.
    pub font_size: Option<FontSize>,
}

impl Strut {
    /// shaping 결과의 기본값(첫 span style의 첫 사용 가능 face와 크기)으로 만든 strut. 문단 typography가 첫 span과
    /// 다르면 호출자가 [`lay_out_paragraph_with_strut`]에 문단 typography의 strut를 줍니다.
    pub fn of(shaped: &ShapedParagraph) -> Strut {
        Strut {
            metrics: shaped.default_metrics(),
            font_size: shaped.default_font_size(),
        }
    }
}

/// [`lay_out_paragraph`]에 문단 root inline box strut를 따로 주는 판.
pub fn lay_out_paragraph_with_strut(
    shaped: &ShapedParagraph,
    breaks: &[BreakOpportunity],
    style: &ParagraphLayoutStyle,
    strut: Strut,
    max_width: Option<TextUnit>,
) -> Result<ParagraphLines, TextLayoutError> {
    let bidi = &shaped.itemization().bidi;
    // 아래 이분 탐색이 기대는 불변식을 쓰는 곳에서도 확인합니다(debug 빌드, paragraph당 O(n) 한 번).
    // 깨지면 잘못된 배치를 조용히 내지 않고 panic합니다.
    let len = shaped.text().len();
    crate::tiling::debug_assert_tiles("glyph run", shaped.runs().iter().map(|r| &r.range), len);
    crate::tiling::debug_assert_tiles("segment", shaped.segments().iter().map(|s| &s.range), len);
    crate::tiling::debug_assert_tiles(
        "BiDi paragraph",
        bidi.paragraphs().iter().map(|(range, _)| range),
        len,
    );
    let breaks_out = break_lines(shaped, breaks, max_width, style.wrap)?;
    let index = ClusterIndex::new(shaped.runs());
    let mut lines = Vec::with_capacity(breaks_out.len());
    let mut total = 0i64;
    for line in breaks_out {
        let order = bidi.line_order(shaped.text(), line.range.clone())?;
        let glyphs = place_glyphs(shaped, &index, &line, &order)?;
        let metrics = line_metrics(shaped, &line.range);
        let (height, baseline) = line_box(shaped, &line.range, strut, style.line_height)?;
        total += i64::from(height.raw());
        lines.push(ParagraphLine {
            line,
            order,
            glyphs,
            height,
            baseline,
            metrics,
        });
    }
    Ok(ParagraphLines {
        runs: shaped
            .runs()
            .iter()
            .map(|run| PlacedRun {
                face: run.instance.clone(),
                size: run.font_size,
                style: run.style,
            })
            .collect(),
        lines,
        align: style.align,
        base_level: bidi.base_level(),
        height: TextUnit::from_i64(total)?,
    })
}

/// run마다 glyph를 (cluster, glyph index) 오름차순으로 정렬한 표. 줄 조각에 든 glyph를 run glyph 전체를
/// 훑지 않고 이분 탐색으로 찾습니다.
struct ClusterIndex(Vec<Vec<(u32, usize)>>);

impl ClusterIndex {
    fn new(runs: &[GlyphRun]) -> ClusterIndex {
        ClusterIndex(
            runs.iter()
                .map(|run| {
                    let mut sorted: Vec<(u32, usize)> = run
                        .glyphs
                        .iter()
                        .enumerate()
                        .map(|(i, g)| (g.cluster, i))
                        .collect();
                    sorted.sort_unstable();
                    sorted
                })
                .collect(),
        )
    }

    /// run `run`에서 cluster가 `piece` 안에 있는 glyph index. run glyph 순서(run 방향 visual 순서)입니다.
    fn glyphs_in(&self, run: usize, piece: &Range<usize>) -> Vec<usize> {
        let Some(sorted) = self.0.get(run) else {
            return Vec::new();
        };
        let lo = sorted.partition_point(|&(cluster, _)| (cluster as usize) < piece.start);
        let hi = sorted.partition_point(|&(cluster, _)| (cluster as usize) < piece.end);
        let mut picked: Vec<usize> = sorted[lo..hi].iter().map(|&(_, i)| i).collect();
        crate::op_count::tick(picked.len());
        picked.sort_unstable();
        picked
    }
}

/// `range`와 겹치는 run과 그 index(논리 순서). run은 paragraph를 논리 순서로 빈틈·겹침 없이 나누므로(범위
/// 오름차순, `shape_paragraph` 불변식) 첫 run을 이분 탐색하고 겹치는 동안만 훑습니다. 불변식은 debug 빌드에서
/// `shape_paragraph`와 [`lay_out_paragraph`]가 확인합니다(`crate::tiling`).
fn overlapping_runs<'a>(
    runs: &'a [GlyphRun],
    range: &Range<usize>,
) -> impl Iterator<Item = (usize, &'a GlyphRun)> + 'a {
    let first = runs.partition_point(|run| run.range.end <= range.start);
    let end = range.end;
    runs[first..]
        .iter()
        .enumerate()
        .map(move |(i, run)| (first + i, run))
        .take_while(move |(_, run)| run.range.start < end)
        .inspect(|_| crate::op_count::tick(1))
}

/// 줄의 visual 순서 glyph를 줄 시작 기준으로 놓습니다.
fn place_glyphs(
    shaped: &ShapedParagraph,
    index: &ClusterIndex,
    line: &LineBreak,
    order: &LineOrder,
) -> Result<Vec<LineGlyph>, TextLayoutError> {
    let runs = shaped.runs();
    let segments = shaped.segments();
    // 줄 끝 hanging 공백 앞까지가 양쪽 정렬 공간을 받을 수 있는 구간입니다.
    let content_end = line.range.end - trailing_len(shaped, line);
    let mut out = Vec::new();
    let mut pen = 0i64;
    for visual in order.visual_runs() {
        // visual run 안의 glyph run 조각: 논리 순서, RTL이면 역순.
        let mut pieces: Vec<(usize, Range<usize>)> = overlapping_runs(runs, &visual.range)
            .filter_map(|(i, run)| {
                let start = run.range.start.max(visual.range.start);
                let end = run.range.end.min(visual.range.end);
                (start < end).then_some((i, start..end))
            })
            .collect();
        if visual.level.is_rtl() {
            pieces.reverse();
        }
        for (run_index, piece) in pieces {
            let run = &runs[run_index];
            let mut glyphs: Vec<&PositionedGlyph> = index
                .glyphs_in(run_index, &piece)
                .into_iter()
                .filter_map(|i| run.glyphs.get(i))
                .collect();
            // run glyph는 run 방향 visual 순서입니다. 조각 level 방향이 다르면(L1로 바뀐 공백) 뒤집습니다.
            if run.level.is_rtl() != visual.level.is_rtl() {
                glyphs.reverse();
            }
            for g in glyphs {
                let expansion = segments
                    .binary_search_by(|s| {
                        if s.range.end <= g.cluster as usize {
                            std::cmp::Ordering::Less
                        } else if s.range.start > g.cluster as usize {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    })
                    .ok()
                    .and_then(|i| segments.get(i))
                    .is_some_and(|s| s.is_whitespace && s.range.end <= content_end);
                out.push(LineGlyph {
                    glyph: g.glyph,
                    cluster: g.cluster,
                    x: TextUnit::from_i64(pen)?,
                    advance: g.x_advance,
                    x_offset: g.x_offset,
                    y_offset: g.y_offset,
                    run: run_index,
                    expansion,
                });
                pen += i64::from(g.x_advance.raw());
            }
        }
    }
    Ok(out)
}

/// 줄 끝 hanging 공백 byte 길이. segment는 논리 순서로 빈틈없이 이어지므로(범위 오름차순) 줄 끝 segment를
/// 이분 탐색하고, 거기서 거꾸로 줄 안 공백 segment인 동안만 훑습니다. 불변식은 debug 빌드에서
/// [`lay_out_paragraph`]가 확인합니다.
fn trailing_len(shaped: &ShapedParagraph, line: &LineBreak) -> usize {
    let segments = shaped.segments();
    let end = segments.partition_point(|s| s.range.end <= line.range.end);
    segments[..end]
        .iter()
        .rev()
        .take_while(|s| {
            crate::op_count::tick(1);
            s.range.start >= line.range.start && s.is_whitespace
        })
        .map(|s| s.range.len())
        .sum()
}

/// 줄에 걸친 run의 최대 metric(표시·caret용). run이 없으면 paragraph 기본값.
fn line_metrics(shaped: &ShapedParagraph, range: &Range<usize>) -> ScaledLineMetrics {
    let mut metrics: Option<ScaledLineMetrics> = None;
    for (_, run) in overlapping_runs(shaped.runs(), range) {
        let m = metrics.get_or_insert(run.metrics);
        m.ascent = m.ascent.max(run.metrics.ascent);
        m.descent = m.descent.min(run.metrics.descent);
        m.line_gap = m.line_gap.max(run.metrics.line_gap);
    }
    metrics.unwrap_or(shaped.default_metrics())
}

/// inline box 하나의 (baseline 위, baseline 아래) 크기. CSS half-leading: leading `L − (A + D)`의 내림 절반을 위에,
/// 나머지를 아래에 더합니다.
fn inline_box(
    metrics: ScaledLineMetrics,
    font_size: Option<FontSize>,
    mode: LineHeight,
) -> (i64, i64) {
    let ascent = i64::from(metrics.ascent.raw());
    let depth = -i64::from(metrics.descent.raw());
    let content = ascent + depth;
    let height = match mode {
        LineHeight::Auto => content + i64::from(metrics.line_gap.raw()),
        LineHeight::Multiplier(ratio) => {
            let size = font_size.map_or(content, |s| i64::from(s.raw()));
            let product = size * i64::from(ratio.raw());
            (product + 512).div_euclid(1024)
        }
        LineHeight::Fixed(fixed) => i64::from(fixed.raw()),
    };
    let leading = height - content;
    let top = leading.div_euclid(2);
    (ascent + top, depth + (leading - top))
}

/// (줄 높이, 줄 위→baseline). strut와 줄에 걸친 run(글꼴이 있는 것)마다 inline box를 만들어 baseline에 맞춥니다.
fn line_box(
    shaped: &ShapedParagraph,
    range: &Range<usize>,
    strut: Strut,
    mode: LineHeight,
) -> Result<(TextUnit, TextUnit), UnitError> {
    let (mut above, mut below) = inline_box(strut.metrics, strut.font_size, mode);
    for (_, run) in overlapping_runs(shaped.runs(), range) {
        // 글꼴이 없는 run(metric 0)은 glyph가 없어 inline box 높이에 들어가지 않습니다.
        if run.metrics == ScaledLineMetrics::default() {
            continue;
        }
        let (a, b) = inline_box(run.metrics, Some(run.font_size), mode);
        above = above.max(a);
        below = below.max(b);
    }
    Ok((
        TextUnit::from_i64(above + below)?,
        TextUnit::from_i64(above)?,
    ))
}

/// 최종 위치 glyph(text box 좌상단 기준).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacedGlyph {
    /// glyph.
    pub glyph: GlyphId,
    /// paragraph byte offset.
    pub cluster: u32,
    /// pen x(정렬·양쪽 정렬 반영).
    pub x: TextUnit,
    /// baseline y.
    pub baseline: TextUnit,
    /// advance(양쪽 정렬 추가분 제외).
    pub advance: TextUnit,
    /// 양쪽 정렬로 이 glyph 뒤에 더한 폭.
    pub justify_extra: TextUnit,
    /// glyph offset.
    pub x_offset: TextUnit,
    /// glyph offset(위가 양수).
    pub y_offset: TextUnit,
    /// glyph run index.
    pub run: usize,
}

/// 최종 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionedLine {
    /// paragraph index.
    pub paragraph: usize,
    /// paragraph 안 줄 index.
    pub line: usize,
    /// paragraph byte 범위.
    pub range: Range<usize>,
    /// 줄 위 y.
    pub top: TextUnit,
    /// 줄 높이.
    pub height: TextUnit,
    /// baseline y.
    pub baseline: TextUnit,
    /// 줄 내용 시작 x(정렬 반영).
    pub x: TextUnit,
    /// 줄 내용 폭(양쪽 정렬 반영, 끝 공백 제외).
    pub width: TextUnit,
    /// glyph(visual 순서).
    pub glyphs: Vec<PlacedGlyph>,
    /// BiDi visual 순서와 logical cluster mapping(caret·hit test용, X.10.3).
    pub order: LineOrder,
}

/// text box 레이아웃 결과 (X.10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayout {
    /// paragraph 및 glyph run index로 찾는 shaping 결과.
    pub runs: Vec<std::sync::Arc<[PlacedRun]>>,
    /// 모든 줄(paragraph 순서).
    pub lines: Vec<PositionedLine>,
    /// box 너비(지정값 또는 내용 너비).
    pub width: TextUnit,
    /// box 높이(지정값 또는 내용 높이).
    pub height: TextUnit,
    /// 내용 높이(paragraph 간격 포함).
    pub content_height: TextUnit,
    /// 세로 정렬로 내용을 내린 양(넘치면 음수 가능).
    pub content_top: TextUnit,
}

impl TextLayout {
    /// 첫 줄 baseline(줄이 없으면 `None`).
    pub fn first_baseline(&self) -> Option<TextUnit> {
        self.lines.first().map(|l| l.baseline)
    }

    /// 마지막 줄 baseline.
    pub fn last_baseline(&self) -> Option<TextUnit> {
        self.lines.last().map(|l| l.baseline)
    }

    /// 논리 레이아웃 canonical bytes (X.10.7). 플랫폼 간 비교·golden에 씁니다.
    ///
    /// ```text
    /// [4] "STXL"  u16 version(1)
    /// i32 width, height, content_height, content_top   u32 줄 수
    /// 줄마다: u32 paragraph, line, range start, range end
    ///         i32 top, height, baseline, x, width     u32 glyph 수
    ///         glyph마다: u32 glyph id, cluster  i32 x, baseline, advance, justify_extra, x_offset, y_offset
    /// ```
    ///
    /// 모든 정수는 little-endian입니다. 값은 정수 subpixel이므로 같은 입력·같은 폰트 bytes는 같은 bytes를
    /// 냅니다. 개수·offset은 `u32`로 기록합니다(텍스트 길이는 shaping 단계에서 `u32`로 제한됨).
    pub fn encode_logical(&self) -> Vec<u8> {
        fn u32_of(value: usize) -> [u8; 4] {
            u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes()
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"STXL");
        out.extend_from_slice(&1u16.to_le_bytes());
        for value in [
            self.width,
            self.height,
            self.content_height,
            self.content_top,
        ] {
            out.extend_from_slice(&value.raw().to_le_bytes());
        }
        out.extend_from_slice(&u32_of(self.lines.len()));
        for line in &self.lines {
            for value in [line.paragraph, line.line, line.range.start, line.range.end] {
                out.extend_from_slice(&u32_of(value));
            }
            for value in [line.top, line.height, line.baseline, line.x, line.width] {
                out.extend_from_slice(&value.raw().to_le_bytes());
            }
            out.extend_from_slice(&u32_of(line.glyphs.len()));
            for g in &line.glyphs {
                out.extend_from_slice(&g.glyph.raw().to_le_bytes());
                out.extend_from_slice(&g.cluster.to_le_bytes());
                for value in [
                    g.x,
                    g.baseline,
                    g.advance,
                    g.justify_extra,
                    g.x_offset,
                    g.y_offset,
                ] {
                    out.extend_from_slice(&value.raw().to_le_bytes());
                }
            }
        }
        out
    }
}

/// paragraph를 쌓고 정렬합니다 (X.10.2 2단계). `paragraph_spacing`은 paragraph 사이에만 넣습니다.
pub fn compose_text(
    paragraphs: &[&ParagraphLines],
    paragraph_spacing: TextUnit,
    text_box: &TextBox,
) -> Result<TextLayout, TextLayoutError> {
    let content_width = paragraphs
        .iter()
        .flat_map(|p| p.lines.iter())
        .map(|l| l.line.width)
        .max()
        .unwrap_or(TextUnit::ZERO);
    let box_width = text_box.width.unwrap_or(content_width);
    let mut content_height = 0i64;
    for (i, p) in paragraphs.iter().enumerate() {
        content_height += i64::from(p.height.raw());
        if i + 1 < paragraphs.len() {
            content_height += i64::from(paragraph_spacing.raw());
        }
    }
    let box_height = match text_box.height {
        Some(h) => i64::from(h.raw()),
        None => content_height,
    };
    let content_top = match text_box.vertical_align {
        VerticalAlign::Top => 0,
        VerticalAlign::Middle => (box_height - content_height).div_euclid(2),
        VerticalAlign::Bottom => box_height - content_height,
    };

    let mut lines = Vec::new();
    let mut y = content_top;
    for (p_index, paragraph) in paragraphs.iter().enumerate() {
        let last_line = paragraph.lines.len().saturating_sub(1);
        for (l_index, line) in paragraph.lines.iter().enumerate() {
            let free = i64::from(box_width.raw()) - i64::from(line.line.width.raw());
            let justify = paragraph.align == TextAlign::Justify
                && l_index != last_line
                && !line.line.hard_break
                && free > 0;
            let opportunities = line.glyphs.iter().filter(|g| g.expansion).count() as i64;
            let (x, extra, remainder) = if justify && opportunities > 0 {
                (0, free / opportunities, free % opportunities)
            } else {
                let start_is_left = !paragraph.base_level.is_rtl();
                let x = match (paragraph.align, start_is_left) {
                    (TextAlign::Center, _) => free.div_euclid(2),
                    (TextAlign::Start | TextAlign::Justify, true) | (TextAlign::End, false) => 0,
                    (TextAlign::Start | TextAlign::Justify, false) | (TextAlign::End, true) => free,
                };
                (x, 0, 0)
            };
            let baseline = y + i64::from(line.baseline.raw());
            let mut shift = 0i64;
            let mut seen = 0i64;
            let mut glyphs = Vec::with_capacity(line.glyphs.len());
            for g in &line.glyphs {
                let mut add = 0i64;
                if g.expansion && (extra > 0 || remainder > 0) {
                    add = extra + i64::from(seen < remainder);
                    seen += 1;
                }
                glyphs.push(PlacedGlyph {
                    glyph: g.glyph,
                    cluster: g.cluster,
                    x: TextUnit::from_i64(x + i64::from(g.x.raw()) + shift)?,
                    baseline: TextUnit::from_i64(baseline)?,
                    advance: g.advance,
                    justify_extra: TextUnit::from_i64(add)?,
                    x_offset: g.x_offset,
                    y_offset: g.y_offset,
                    run: g.run,
                });
                shift += add;
            }
            lines.push(PositionedLine {
                paragraph: p_index,
                line: l_index,
                range: line.line.range.clone(),
                top: TextUnit::from_i64(y)?,
                height: line.height,
                baseline: TextUnit::from_i64(baseline)?,
                x: TextUnit::from_i64(x)?,
                width: TextUnit::from_i64(i64::from(line.line.width.raw()) + shift)?,
                glyphs,
                order: line.order.clone(),
            });
            y += i64::from(line.height.raw());
        }
        if p_index + 1 < paragraphs.len() {
            y += i64::from(paragraph_spacing.raw());
        }
    }
    Ok(TextLayout {
        runs: paragraphs.iter().map(|p| p.runs.clone()).collect(),
        lines,
        width: box_width,
        height: TextUnit::from_i64(box_height)?,
        content_height: TextUnit::from_i64(content_height)?,
        content_top: TextUnit::from_i64(content_top)?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bidi::BaseDirection;
    use crate::line_break::{line_break_opportunities, LineBreakOptions};
    use crate::paragraph::tests::Fixture;
    use crate::shaping::LanguageTag;

    pub(crate) fn breaks(text: &str) -> Vec<BreakOpportunity> {
        line_break_opportunities(
            text,
            &LineBreakOptions::new(LanguageTag::parse("en").expect("en")),
        )
    }

    fn px(v: i32) -> TextUnit {
        TextUnit::from_px(v).expect("px")
    }

    fn paragraph(
        fx: &Fixture,
        text: &str,
        base: BaseDirection,
        style: ParagraphLayoutStyle,
        width: Option<TextUnit>,
    ) -> ParagraphLines {
        let shaped = fx.shape(text, 16, base);
        lay_out_paragraph(&shaped, &breaks(text), &style, width).expect("paragraph")
    }

    #[test]
    fn line_height_modes_fixture() {
        let fx = Fixture::new();
        // 16px Latin: ascent 819, descent -205, gap 0 → Auto 높이 1024, baseline 819.
        let auto = paragraph(
            &fx,
            "ab",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        assert_eq!(auto.lines[0].height, TextUnit::from_raw(1024));
        assert_eq!(auto.lines[0].baseline, TextUnit::from_raw(819));
        // Multiplier 1.5(1536/1024) → 1536, leading 512 → baseline 256 + 819.
        let ratio = LineHeightRatio::from_raw(1536).expect("1.5");
        let multiplied = paragraph(
            &fx,
            "ab",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle {
                line_height: LineHeight::Multiplier(ratio),
                ..ParagraphLayoutStyle::default()
            },
            None,
        );
        assert_eq!(multiplied.lines[0].height, TextUnit::from_raw(1536));
        assert_eq!(multiplied.lines[0].baseline, TextUnit::from_raw(256 + 819));
        // Fixed 10px(640): 내용보다 작으면 leading 음수 → baseline 819 - 192.
        let fixed = paragraph(
            &fx,
            "ab",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle {
                line_height: LineHeight::Fixed(px(10)),
                ..ParagraphLayoutStyle::default()
            },
            None,
        );
        assert_eq!(fixed.lines[0].height, px(10));
        assert_eq!(fixed.lines[0].baseline, TextUnit::from_raw(819 - 192));
        // CJK run(asc 880, desc -120, gap 100 @16px → 901/-123/102)과 섞인 줄: CSS는 inline box마다 자기 metric에
        // half-leading을 더합니다. Latin box(와 strut)는 위 819·아래 205, CJK box는 위 901 + 51·아래 123 + 51입니다.
        // 줄 높이는 위 최댓값 952 + 아래 최댓값 205 = 1157입니다(이전 규칙은 최대 ascent·descent·gap 합 1208).
        let mixed = paragraph(
            &fx,
            "a日",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        assert_eq!(mixed.lines[0].metrics.ascent, TextUnit::from_raw(901));
        assert_eq!(mixed.lines[0].metrics.descent, TextUnit::from_raw(-205));
        assert_eq!(mixed.lines[0].height, TextUnit::from_raw(901 + 51 + 205));
        assert_eq!(mixed.lines[0].baseline, TextUnit::from_raw(51 + 901));
        // 빈 paragraph도 기본 metric 한 줄.
        let empty = paragraph(
            &fx,
            "",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        assert_eq!(empty.lines.len(), 1);
        assert_eq!(empty.height, TextUnit::from_raw(1024));
        assert_eq!(LineHeightRatio::from_raw(0), None);
        assert_eq!(LineHeightRatio::ONE.raw(), 1024);
    }

    /// 크기가 다른 style span으로 shaping합니다(style i의 크기 = `sizes[i]` px).
    fn shape_sizes(
        fx: &Fixture,
        text: &str,
        parts: &[(Range<usize>, u32)],
        sizes: &[i32],
    ) -> ShapedParagraph {
        use crate::itemize::StyleSpan;
        use crate::paragraph::{shape_paragraph, ParagraphInput, TextStyleSpec};
        let styles: Vec<TextStyleSpec<'_>> = sizes
            .iter()
            .map(|size| fx.styles(*size).remove(0))
            .collect();
        let spans: Vec<StyleSpan> = parts
            .iter()
            .map(|(range, style)| StyleSpan {
                range: range.clone(),
                style: *style,
                language: None,
            })
            .collect();
        shape_paragraph(
            &ParagraphInput {
                text,
                spans: &spans,
                base_direction: BaseDirection::LeftToRight,
            },
            &styles,
            &fx.set.faces,
        )
        .expect("shape")
    }

    fn one_line(shaped: &ShapedParagraph, mode: LineHeight) -> ParagraphLine {
        let style = ParagraphLayoutStyle {
            line_height: mode,
            ..ParagraphLayoutStyle::default()
        };
        let mut lines = lay_out_paragraph(shaped, &breaks(shaped.text()), &style, None)
            .expect("paragraph")
            .lines;
        assert_eq!(lines.len(), 1);
        lines.remove(0)
    }

    #[test]
    fn mixed_size_runs_use_their_own_half_leading_css_line_box() {
        // 검증 기준(IX.11.12): 고정 길이 혼합 크기 run line box. Latin 글꼴은 ascent 0.8em·descent 0.2em입니다.
        let fx = Fixture::new();
        let shaped = shape_sizes(&fx, "abcd", &[(0..2, 0), (2..4, 1)], &[10, 40]);
        // line-height: 20px(1280). 10px box(640): leading 640 → 위 512 + 320, 아래 128 + 320. 40px box(2560): leading −1280
        // → 위 2048 − 640, 아래 512 − 640. strut는 첫 span(10px)과 같습니다. 줄 = 위 1408 + 아래 448 = 1856 > 1280.
        let fixed = one_line(&shaped, LineHeight::Fixed(px(20)));
        assert_eq!(fixed.height, TextUnit::from_raw(1408 + 448));
        assert!(fixed.height > px(20));
        assert_eq!(fixed.baseline, TextUnit::from_raw(1408));
        // 배율(<number>): box마다 자기 font-size × 배율. Latin 16px와 CJK 16px(위 901·아래 123)에 1.5를 주면 둘 다
        // L = 1536이고, Latin은 위 819 + 256·아래 205 + 256, CJK는 위 901 + 256·아래 123 + 256입니다. 줄 = 1157 + 461 =
        // 1618 > 1536(이전 규칙: 줄의 최대 글꼴 크기 × 배율 = 1536).
        let ratio = LineHeightRatio::from_raw(1536).expect("1.5");
        let mixed_fonts = fx.shape("a日", 16, BaseDirection::LeftToRight);
        let multiplied = one_line(&mixed_fonts, LineHeight::Multiplier(ratio));
        assert_eq!(multiplied.height, TextUnit::from_raw(1157 + 461));
        assert_eq!(multiplied.baseline, TextUnit::from_raw(1157));
        // 배율, 같은 글꼴 다른 크기(10px·40px, ×2): 40px box가 위 2048 + 1280·아래 512 + 1280을 정합니다.
        let two = LineHeightRatio::from_raw(2048).expect("2");
        let sized = one_line(&shaped, LineHeight::Multiplier(two));
        assert_eq!(sized.height, TextUnit::from_raw(3328 + 1792));
    }

    #[test]
    fn strut_from_paragraph_typography_sets_the_minimum_line_box() {
        // 검증 기준(IX.11.12): 문단 글꼴보다 작은 run만 있는 줄의 strut 높이 fixture.
        let fx = Fixture::new();
        let small = fx.shape("ab", 10, BaseDirection::LeftToRight);
        let paragraph_font = fx.shape("", 40, BaseDirection::LeftToRight);
        let strut = Strut::of(&paragraph_font);
        assert_eq!(strut.font_size, Some(FontSize::from_px(40).unwrap()));
        let style = ParagraphLayoutStyle::default();
        let with_strut =
            lay_out_paragraph_with_strut(&small, &breaks("ab"), &style, strut, None).unwrap();
        // normal: strut(40px Latin) 위 2048·아래 512가 10px run(위 512·아래 128)보다 큽니다.
        assert_eq!(with_strut.lines[0].height, TextUnit::from_raw(2048 + 512));
        assert_eq!(with_strut.lines[0].baseline, TextUnit::from_raw(2048));
        // 문단 글꼴이 run과 같으면(기본 strut) run 높이입니다.
        let own = lay_out_paragraph(&small, &breaks("ab"), &style, None).unwrap();
        assert_eq!(own.lines[0].height, TextUnit::from_raw(512 + 128));
        // 고정 길이에서도 strut는 inline box 하나입니다: 20px, 40px strut → 위 2048 − 640, 아래 512 − 640.
        let fixed = ParagraphLayoutStyle {
            line_height: LineHeight::Fixed(px(20)),
            ..ParagraphLayoutStyle::default()
        };
        let fixed_lines =
            lay_out_paragraph_with_strut(&small, &breaks("ab"), &fixed, strut, None).unwrap();
        // 10px run: 위 512 + 320, 아래 128 + 320. 줄 = 위 max(1408, 832) + 아래 max(−128, 448).
        assert_eq!(fixed_lines.lines[0].height, TextUnit::from_raw(1408 + 448));
    }

    #[test]
    fn run_letter_spacing_adds_after_each_character_unit() {
        // 검증 기준(IX.11.12): run별 letter-spacing fixture.
        use stellar_core_types::typography::LetterSpacing;
        use stellar_core_types::value::FiniteF32;
        let fx = Fixture::new();
        let shaped = shape_sizes(&fx, "abcd", &[(0..2, 0), (2..4, 1)], &[16, 16]);
        let width = |p: &ShapedParagraph| -> i64 {
            let line = one_line(p, LineHeight::Auto);
            line.glyphs.iter().map(|g| i64::from(g.advance.raw())).sum()
        };
        let segment_sum = |p: &ShapedParagraph| -> i64 {
            p.segments()
                .iter()
                .map(|s| i64::from(s.advance.raw()))
                .sum()
        };
        let base = width(&shaped);
        let spaced = shaped
            .with_letter_spacing(|style| match style {
                0 => LetterSpacing::Px(FiniteF32::new(1.0).unwrap()),
                _ => LetterSpacing::Em(FiniteF32::new(0.5).unwrap()),
            })
            .unwrap();
        // run 0: 글자 2개 × 1px(64), run 1: 글자 2개 × 0.5em(16px → 512).
        assert_eq!(width(&spaced), base + 2 * 64 + 2 * 512);
        assert_eq!(
            segment_sum(&spaced),
            width(&spaced),
            "줄 나누기 폭도 같은 값"
        );
        // normal은 바꾸지 않습니다.
        assert_eq!(
            shaped
                .with_letter_spacing(|_| LetterSpacing::Normal)
                .unwrap(),
            shaped
        );
        // ligature cluster(ffi)는 grapheme 수만큼 더합니다: "office"는 글자 6개.
        let office = fx.shape("office", 16, BaseDirection::LeftToRight);
        let spaced_office = office
            .with_letter_spacing(|_| LetterSpacing::Px(FiniteF32::new(1.0).unwrap()))
            .unwrap();
        assert_eq!(width(&spaced_office), width(&office) + 6 * 64);
    }

    #[test]
    fn single_size_paragraphs_keep_previous_line_boxes() {
        // 검증 기준(IX.11.12): 단일 크기 문단 결과 불변. 이전 규칙(줄 높이 = 모드 높이, baseline = leading/2 + ascent)과
        // 같아야 합니다. Latin 글꼴은 ascent 0.8em·descent 0.2em·gap 0입니다.
        let fx = Fixture::new();
        for size in [10, 16, 40] {
            let shaped = fx.shape("ab cd", size, BaseDirection::LeftToRight);
            let unit = i64::from(FontSize::from_px(size).unwrap().raw());
            let ascent = i64::from(shaped.default_metrics().ascent.raw());
            let content = ascent - i64::from(shaped.default_metrics().descent.raw());
            for (mode, height) in [
                (LineHeight::Auto, content),
                (
                    LineHeight::Multiplier(LineHeightRatio::from_raw(1536).unwrap()),
                    (unit * 1536 + 512).div_euclid(1024),
                ),
                (LineHeight::Fixed(px(10)), 640),
            ] {
                let line = one_line(&shaped, mode);
                assert_eq!(i64::from(line.height.raw()), height, "{size}px {mode:?}");
                assert_eq!(
                    i64::from(line.baseline.raw()),
                    (height - content).div_euclid(2) + ascent,
                    "{size}px {mode:?}"
                );
            }
        }
    }

    #[test]
    fn horizontal_alignment_fixture() {
        let fx = Fixture::new();
        let text_box = TextBox {
            width: Some(px(100)),
            ..TextBox::default()
        };
        let x_of = |align: TextAlign, base: BaseDirection| {
            let p = paragraph(
                &fx,
                "ab",
                base,
                ParagraphLayoutStyle {
                    align,
                    ..ParagraphLayoutStyle::default()
                },
                Some(px(100)),
            );
            compose_text(&[&p], TextUnit::ZERO, &text_box)
                .expect("compose")
                .lines[0]
                .x
        };
        // "ab" = 16px(1024). 빈 폭 84px(5376).
        assert_eq!(
            x_of(TextAlign::Start, BaseDirection::LeftToRight),
            TextUnit::ZERO
        );
        assert_eq!(
            x_of(TextAlign::End, BaseDirection::LeftToRight),
            TextUnit::from_raw(5376)
        );
        assert_eq!(
            x_of(TextAlign::Center, BaseDirection::LeftToRight),
            TextUnit::from_raw(2688)
        );
        // RTL 문단: Start는 오른쪽, End는 왼쪽.
        assert_eq!(
            x_of(TextAlign::Start, BaseDirection::RightToLeft),
            TextUnit::from_raw(5376)
        );
        assert_eq!(
            x_of(TextAlign::End, BaseDirection::RightToLeft),
            TextUnit::ZERO
        );
        // 마지막 줄 Justify는 Start.
        assert_eq!(
            x_of(TextAlign::Justify, BaseDirection::LeftToRight),
            TextUnit::ZERO
        );
    }

    #[test]
    fn justify_distributes_integer_space_to_inner_spaces() {
        let fx = Fixture::new();
        // 40px 폭: "aa bb cc dd"는 "aa bb " | "cc dd"로 나뉩니다. 첫 줄 내용 "aa bb" = 36px.
        let p = paragraph(
            &fx,
            "aa bb cc dd",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle {
                align: TextAlign::Justify,
                ..ParagraphLayoutStyle::default()
            },
            Some(px(40)),
        );
        assert_eq!(p.lines.len(), 2);
        let layout = compose_text(
            &[&p],
            TextUnit::ZERO,
            &TextBox {
                width: Some(px(40)),
                ..TextBox::default()
            },
        )
        .expect("compose");
        let first = &layout.lines[0];
        // 내부 공백 1개가 남는 4px(256)를 모두 받습니다. 끝 공백은 받지 않습니다.
        assert_eq!(first.width, px(40));
        let extras: Vec<i32> = first.glyphs.iter().map(|g| g.justify_extra.raw()).collect();
        assert_eq!(extras, vec![0, 0, 256, 0, 0, 0]);
        assert_eq!(first.glyphs[3].x, TextUnit::from_raw(512 * 2 + 256 + 256));
        // 마지막 줄은 양쪽 정렬하지 않습니다.
        assert!(layout.lines[1]
            .glyphs
            .iter()
            .all(|g| g.justify_extra == TextUnit::ZERO));
    }

    #[test]
    fn paragraph_spacing_and_stacking() {
        let fx = Fixture::new();
        let style = ParagraphLayoutStyle::default();
        let a = paragraph(&fx, "ab", BaseDirection::LeftToRight, style, None);
        let b = paragraph(&fx, "cd", BaseDirection::LeftToRight, style, None);
        let layout = compose_text(&[&a, &b], px(10), &TextBox::default()).expect("compose");
        let tops: Vec<i32> = layout.lines.iter().map(|l| l.top.raw()).collect();
        // 1024 + 간격 640.
        assert_eq!(tops, vec![0, 1024 + 640]);
        assert_eq!(layout.content_height, TextUnit::from_raw(1024 * 2 + 640));
        assert_eq!(layout.first_baseline(), Some(TextUnit::from_raw(819)));
        assert_eq!(
            layout.last_baseline(),
            Some(TextUnit::from_raw(1024 + 640 + 819))
        );
        // 간격은 paragraph 사이에만: paragraph 하나면 내용 높이 = 줄 높이.
        let single = compose_text(&[&a], px(10), &TextBox::default()).expect("compose");
        assert_eq!(single.content_height, TextUnit::from_raw(1024));
        assert_eq!(single.width, TextUnit::from_raw(1024));
    }

    #[test]
    fn text_box_vertical_alignment_fixture() {
        // paragraph 가로 정렬과 별개인 text box 세로 정렬: 높이 100px box, 내용 16px 한 줄.
        let fx = Fixture::new();
        let p = paragraph(
            &fx,
            "ab",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        let top_of = |vertical_align: VerticalAlign, height: i32| {
            let layout = compose_text(
                &[&p],
                TextUnit::ZERO,
                &TextBox {
                    width: Some(px(50)),
                    height: Some(px(height)),
                    vertical_align,
                },
            )
            .expect("compose");
            (
                layout.content_top.raw(),
                layout.lines[0].top.raw(),
                layout.lines[0].x.raw(),
            )
        };
        assert_eq!(top_of(VerticalAlign::Top, 100), (0, 0, 0));
        assert_eq!(top_of(VerticalAlign::Middle, 100), (2688, 2688, 0));
        assert_eq!(top_of(VerticalAlign::Bottom, 100), (5376, 5376, 0));
        // 내용이 box보다 크면 Middle·Bottom은 위로 넘칩니다.
        assert_eq!(top_of(VerticalAlign::Middle, 10), (-192, -192, 0));
        assert_eq!(top_of(VerticalAlign::Bottom, 10), (-384, -384, 0));
    }

    #[test]
    fn paragraph_layout_work_is_linear_in_text_length() {
        // posthoc TS-2 회귀: 줄·visual run마다 run 전체, run glyph 전체, segment 전체, BiDi paragraph 전체를
        // 훑으면 작업량이 (줄 수 × paragraph 크기)로 커집니다. 40px 폭에서 줄이 많이 생기는 세 입력:
        // 단일 run Latin, 글자마다 run이 바뀌는 Latin·CJK, 줄마다 BiDi paragraph가 바뀌는 U+2029 구분.
        let fx = Fixture::new();
        let style = ParagraphLayoutStyle::default();
        for (unit, min_lines_per_unit) in [("abcd ", 1), ("a日", 0), ("a\u{2029}", 1)] {
            let work = |repeat: usize| {
                let text = unit.repeat(repeat);
                let shaped = fx.shape(&text, 16, BaseDirection::LeftToRight);
                let opportunities = breaks(&text);
                let (laid, units) = crate::op_count::measure(|| {
                    lay_out_paragraph(&shaped, &opportunities, &style, Some(px(40)))
                        .expect("paragraph")
                });
                assert!(laid.lines.len() >= repeat * min_lines_per_unit);
                assert!(
                    laid.lines.len() >= repeat / 2,
                    "{unit:?}: 줄이 많아야 합니다"
                );
                (units, shaped.segments().len() as u64)
            };
            let (small, _) = work(500);
            let (large, segments) = work(2_000);
            assert!(
                large <= 8 * segments,
                "{unit:?}: segment {segments}개에 작업량 {large}(segment당 상수 번이어야 함)"
            );
            assert!(
                large <= 5 * small,
                "{unit:?}: 입력 4배에 작업량 {small} → {large}(선형이면 4배 남짓, 제곱이면 16배)"
            );
        }
    }

    /// 정렬 불변식을 깨뜨린 shaping 결과로 `lay_out_paragraph`를 부릅니다. `corrupt`가 run·segment·BiDi
    /// paragraph 목록을 고칩니다.
    #[cfg(debug_assertions)]
    fn lay_out_corrupted(
        text: &str,
        corrupt: impl FnOnce(
            &mut Vec<GlyphRun>,
            &mut Vec<crate::paragraph::Segment>,
            &mut Vec<(Range<usize>, BidiLevel)>,
        ),
    ) {
        let fx = Fixture::new();
        let mut shaped = fx.shape(text, 16, BaseDirection::LeftToRight);
        let (runs, segments, itemization) = crate::paragraph::tests::parts_mut(&mut shaped);
        corrupt(runs, segments, itemization.bidi.paragraphs_mut());
        let _ = lay_out_paragraph(
            &shaped,
            &breaks(text),
            &ParagraphLayoutStyle::default(),
            Some(px(40)),
        );
    }

    // 리뷰 지적 회귀: 줄 배치의 이분 탐색(`overlapping_runs`, `trailing_len`, `line_levels`)은 run·segment·BiDi
    // paragraph가 텍스트를 순서대로 빈틈없이 나눈다는 불변식에 기댑니다. 불변식이 깨지면 잘못된 배치를 조용히
    // 내지 않고 debug 빌드에서 panic해야 합니다.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "glyph run 범위가 텍스트를 순서대로 빈틈없이 나누지 않습니다")]
    fn unordered_runs_panic_in_debug_build() {
        lay_out_corrupted("a日a日", |runs, _, _| {
            assert!(runs.len() >= 2, "run이 둘 이상이어야 합니다");
            runs.swap(0, 1);
        });
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "segment 범위가 텍스트를 순서대로 빈틈없이 나누지 않습니다")]
    fn unordered_segments_panic_in_debug_build() {
        lay_out_corrupted("ab cd ef", |_, segments, _| {
            assert!(segments.len() >= 2, "segment가 둘 이상이어야 합니다");
            segments.swap(0, 1);
        });
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "BiDi paragraph 범위가 텍스트를 순서대로 빈틈없이 나누지 않습니다")]
    fn unordered_bidi_paragraphs_panic_in_debug_build() {
        lay_out_corrupted("ab\u{2029}cd\u{2029}ef", |_, _, paragraphs| {
            assert!(
                paragraphs.len() >= 2,
                "BiDi paragraph가 둘 이상이어야 합니다"
            );
            paragraphs.swap(0, 1);
        });
    }

    #[test]
    fn line_glyphs_are_placed_in_visual_order() {
        let fx = Fixture::new();
        // "ab 日本" LTR: visual = a b 공백 日 本 순.
        let p = paragraph(
            &fx,
            "ab 日本",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        let clusters: Vec<u32> = p.lines[0].glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 2, 3, 6]);
        let xs: Vec<i32> = p.lines[0].glyphs.iter().map(|g| g.x.raw()).collect();
        assert_eq!(xs, vec![0, 512, 1024, 1280, 2304]);
        // "abc אבג": 히브리어(461 subpx)는 visual 순서상 오른쪽부터 논리 순서로(ג ב א).
        let p = paragraph(
            &fx,
            "abc אבג",
            BaseDirection::LeftToRight,
            ParagraphLayoutStyle::default(),
            None,
        );
        let placed: Vec<(u32, i32)> = p.lines[0]
            .glyphs
            .iter()
            .map(|g| (g.cluster, g.x.raw()))
            .collect();
        assert_eq!(
            placed,
            vec![
                (0, 0),
                (1, 512),
                (2, 1024),
                (3, 1536),
                (8, 1792),
                (6, 2253),
                (4, 2714)
            ]
        );
        // RTL 문단 "אבג abc"(level 1,1,1,1,2,2,2): L2 결과 왼쪽부터 a b c, 공백, ג ב א.
        let p = paragraph(
            &fx,
            "אבג abc",
            BaseDirection::RightToLeft,
            ParagraphLayoutStyle::default(),
            None,
        );
        let clusters: Vec<u32> = p.lines[0].glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![7, 8, 9, 6, 4, 2, 0]);
    }
}
