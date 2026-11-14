//! paragraph shaping pipeline (X.10.1).
//!
//! paragraph 하나를 itemization(X.9.2) → grapheme 단위 font fallback(X.9.5) → shaping(X.9.1·X.9.6)
//! 순서로 처리해 [`ShapedParagraph`]를 만듭니다. glyph 위치는 style 글꼴 크기로 정수 변환한 subpixel
//! ([`TextUnit`]) 값입니다.
//!
//! [`ShapedParagraph::segments`]는 줄바꿈·caret의 **원자 단위**입니다. shaping은 grapheme을 쪼개지
//! 않는 cluster 수준(`MonotoneGraphemes`)으로 하므로 shaping cluster 경계는 grapheme 경계의 부분집합이고,
//! ligature처럼 여러 grapheme이 한 cluster가 되면 segment도 하나입니다. 줄은 segment 경계에서만
//! 끊기므로 cluster가 두 줄로 쪼개지지 않습니다.

use std::fmt;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::bidi::{BaseDirection, BidiError, BidiLevel};
use crate::fallback::ResolvedFallbackChain;
use crate::font_runs::{
    assign_fonts, shape_font_runs, FaceSource, FontRunError, MissingGlyphCluster, RunFace,
    ShapingStyle,
};
use crate::itemize::{itemize, Itemization, ItemizeError, StyleSpan};
use crate::shaping::{GlyphId, LineMetrics};
use crate::units::{FontSize, TextUnit, UnitError};

/// style 하나의 레이아웃 설정.
#[derive(Debug, Clone)]
pub struct TextStyleSpec<'a> {
    /// 해석된 fallback chain.
    pub chain: ResolvedFallbackChain<'a>,
    /// OpenType feature·variation.
    pub shaping: ShapingStyle,
    /// 글꼴 크기.
    pub font_size: FontSize,
}

/// paragraph 입력.
#[derive(Debug, Clone, Copy)]
pub struct ParagraphInput<'a> {
    /// paragraph 텍스트(`'\n'` 없음).
    pub text: &'a str,
    /// style span(X.9.2 규칙).
    pub spans: &'a [StyleSpan],
    /// 기준 방향.
    pub base_direction: BaseDirection,
}

/// 텍스트 레이아웃 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextLayoutError {
    /// itemization 입력 오류.
    Itemize(ItemizeError),
    /// font 할당·shaping 오류.
    FontRun(FontRunError),
    /// BiDi 재배치 오류.
    Bidi(BidiError),
    /// 단위 범위 오류.
    Unit(UnitError),
    /// paragraph 텍스트에 `'\n'`이 있습니다(paragraph 구분자).
    ParagraphSeparatorInText {
        /// 위치.
        offset: usize,
    },
    /// 너비 제약이 음수입니다.
    NegativeWidth,
    /// 증분 레이아웃이 방금 채운 cache 항목을 찾지 못했습니다(내부 불변식 위반, panic 대신 오류).
    CacheEntryMissing,
}

impl fmt::Display for TextLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextLayoutError::Itemize(err) => write!(f, "itemization 오류: {err}"),
            TextLayoutError::FontRun(err) => write!(f, "font run 오류: {err}"),
            TextLayoutError::Bidi(err) => write!(f, "BiDi 오류: {err}"),
            TextLayoutError::Unit(err) => write!(f, "단위 오류: {err}"),
            TextLayoutError::ParagraphSeparatorInText { offset } => {
                write!(f, "paragraph 텍스트 {offset} 위치에 '\\n'이 있습니다")
            }
            TextLayoutError::NegativeWidth => f.write_str("너비 제약은 음수일 수 없습니다"),
            TextLayoutError::CacheEntryMissing => {
                f.write_str("증분 레이아웃 cache 항목이 없습니다(내부 불변식 위반)")
            }
        }
    }
}

impl std::error::Error for TextLayoutError {}

impl From<ItemizeError> for TextLayoutError {
    fn from(err: ItemizeError) -> TextLayoutError {
        TextLayoutError::Itemize(err)
    }
}

impl From<FontRunError> for TextLayoutError {
    fn from(err: FontRunError) -> TextLayoutError {
        TextLayoutError::FontRun(err)
    }
}

impl From<BidiError> for TextLayoutError {
    fn from(err: BidiError) -> TextLayoutError {
        TextLayoutError::Bidi(err)
    }
}

impl From<UnitError> for TextLayoutError {
    fn from(err: UnitError) -> TextLayoutError {
        TextLayoutError::Unit(err)
    }
}

/// 위치가 정해진 glyph(subpixel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositionedGlyph {
    /// glyph.
    pub glyph: GlyphId,
    /// paragraph byte offset(cluster 시작).
    pub cluster: u32,
    /// 가로 advance.
    pub x_advance: TextUnit,
    /// 가로 offset.
    pub x_offset: TextUnit,
    /// 세로 offset(위쪽이 양수, font 좌표계).
    pub y_offset: TextUnit,
}

/// 줄 metric(subpixel). descent는 음수입니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ScaledLineMetrics {
    /// baseline 위 높이.
    pub ascent: TextUnit,
    /// baseline 아래 깊이(≤ 0).
    pub descent: TextUnit,
    /// line gap.
    pub line_gap: TextUnit,
}

/// shaping된 glyph run(한 item 안 같은 face 구간).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlyphRun {
    /// paragraph byte 범위.
    pub range: Range<usize>,
    /// itemization item index.
    pub item: usize,
    /// style index.
    pub style: u32,
    /// BiDi level.
    pub level: BidiLevel,
    /// face(없으면 glyph 없음).
    pub face: RunFace,
    /// 글꼴 크기.
    pub font_size: FontSize,
    /// 이 run 글꼴의 줄 metric(face가 없으면 0).
    pub metrics: ScaledLineMetrics,
    /// glyph(visual 순서).
    pub glyphs: Vec<PositionedGlyph>,
}

/// 줄바꿈·caret 원자 단위(shaping cluster).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Segment {
    /// paragraph byte 범위.
    pub range: Range<usize>,
    /// 가로 advance 합.
    pub advance: TextUnit,
    /// 줄 끝에서 폭에 넣지 않는 공백(hanging whitespace)인지.
    pub is_whitespace: bool,
    /// 속한 glyph run index.
    pub run: usize,
}

/// shaping된 paragraph (X.10.1 입력).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedParagraph {
    text: String,
    itemization: Itemization,
    runs: Vec<GlyphRun>,
    segments: Vec<Segment>,
    missing: Vec<MissingGlyphCluster>,
    default_metrics: ScaledLineMetrics,
    default_font_size: Option<FontSize>,
}

impl ShapedParagraph {
    /// paragraph 텍스트.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// itemization(BiDi 포함).
    pub fn itemization(&self) -> &Itemization {
        &self.itemization
    }

    /// 논리 순서 glyph run.
    pub fn runs(&self) -> &[GlyphRun] {
        &self.runs
    }

    /// 논리 순서 원자 segment.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// missing glyph cluster.
    pub fn missing(&self) -> &[MissingGlyphCluster] {
        &self.missing
    }

    /// 빈 paragraph·glyph 없는 줄에 쓰는 metric(첫 style 첫 사용 가능 face 기준).
    pub fn default_metrics(&self) -> ScaledLineMetrics {
        self.default_metrics
    }

    /// 빈 줄의 글꼴 크기(첫 span style, style이 없으면 `None`).
    pub fn default_font_size(&self) -> Option<FontSize> {
        self.default_font_size
    }

    /// segment 경계 offset인지(0과 텍스트 끝 포함).
    pub fn is_segment_boundary(&self, offset: usize) -> bool {
        offset == 0
            || offset == self.text.len()
            || self
                .segments
                .binary_search_by_key(&offset, |s| s.range.start)
                .is_ok()
    }
}

/// 줄 끝 hanging 공백 문자(CSS `white-space: normal`의 공백·탭·전각 공백 등).
pub fn is_hanging_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200A}' | '\u{205F}' | '\u{3000}'
    )
}

fn scale_metrics(
    metrics: LineMetrics,
    size: FontSize,
    upem: u16,
) -> Result<ScaledLineMetrics, UnitError> {
    Ok(ScaledLineMetrics {
        ascent: size.scale(i64::from(metrics.ascender), upem)?,
        descent: size.scale(i64::from(metrics.descender), upem)?,
        line_gap: size.scale(i64::from(metrics.line_gap), upem)?,
    })
}

/// paragraph를 shaping합니다 (X.10.1 pipeline). `styles[style]`이 span style의 설정입니다.
pub fn shape_paragraph<S: FaceSource + ?Sized>(
    input: &ParagraphInput<'_>,
    styles: &[TextStyleSpec<'_>],
    faces: &S,
) -> Result<ShapedParagraph, TextLayoutError> {
    let text = input.text;
    if let Some(offset) = text.find('\n') {
        return Err(TextLayoutError::ParagraphSeparatorInText { offset });
    }
    let itemization = itemize(text, input.spans, input.base_direction)?;
    let chains: Vec<ResolvedFallbackChain<'_>> = styles.iter().map(|s| s.chain.clone()).collect();
    let shaping: Vec<ShapingStyle> = styles.iter().map(|s| s.shaping.clone()).collect();
    let assignment = assign_fonts(text, &itemization.items, &chains, faces)?;
    let (shaped_runs, missing) = shape_font_runs(
        text,
        &itemization.items,
        &assignment,
        &chains,
        &shaping,
        faces,
    )?;

    let mut runs = Vec::with_capacity(shaped_runs.len());
    let mut segments = Vec::new();
    for shaped_run in shaped_runs {
        // assign_fonts·shape_font_runs가 item index와 style index를 이미 검증했지만, 색인 대신 조회로
        // 다시 확인해 panic 경로를 두지 않습니다.
        let invalid_item = FontRunError::InvalidItem {
            item: shaped_run.run.item,
        };
        let item = itemization
            .items
            .get(shaped_run.run.item)
            .ok_or(invalid_item)?;
        let spec = usize::try_from(item.style)
            .ok()
            .and_then(|s| styles.get(s))
            .ok_or(FontRunError::UnknownStyle {
                item: shaped_run.run.item,
                style: item.style,
            })?;
        let size = spec.font_size;
        let (glyphs, metrics) = match (&shaped_run.shaped, shaped_run.run.face) {
            (Some(shaped), RunFace::Face { handle, .. }) => {
                let upem = shaped.units_per_em;
                let mut glyphs = Vec::with_capacity(shaped.glyphs.len());
                for g in &shaped.glyphs {
                    glyphs.push(PositionedGlyph {
                        glyph: g.glyph,
                        cluster: g.cluster,
                        x_advance: size.scale(i64::from(g.x_advance), upem)?,
                        x_offset: size.scale(i64::from(g.x_offset), upem)?,
                        y_offset: size.scale(i64::from(g.y_offset), upem)?,
                    });
                }
                let metrics = match faces.face(handle) {
                    Some(face) => scale_metrics(face.line_metrics(), size, upem)?,
                    None => ScaledLineMetrics::default(),
                };
                (glyphs, metrics)
            }
            _ => (Vec::new(), ScaledLineMetrics::default()),
        };
        let run_index = runs.len();
        push_segments(
            text,
            &shaped_run.run.range,
            &glyphs,
            run_index,
            &mut segments,
        )?;
        runs.push(GlyphRun {
            range: shaped_run.run.range.clone(),
            item: shaped_run.run.item,
            style: item.style,
            level: item.level,
            face: shaped_run.run.face,
            font_size: size,
            metrics,
            glyphs,
        });
    }

    // 줄 배치의 이분 탐색이 기대는 불변식을 만든 곳에서 확인합니다(debug 빌드, `crate::tiling`).
    crate::tiling::debug_assert_tiles("glyph run", runs.iter().map(|r| &r.range), text.len());
    crate::tiling::debug_assert_tiles("segment", segments.iter().map(|s| &s.range), text.len());

    let (default_metrics, default_font_size) = default_metrics(input, styles, faces)?;
    Ok(ShapedParagraph {
        text: text.to_string(),
        itemization,
        runs,
        segments,
        missing,
        default_metrics,
        default_font_size,
    })
}

/// 첫 style의 첫 로드된 face metric(없으면 0). 빈 paragraph 줄 높이에 씁니다.
fn default_metrics<S: FaceSource + ?Sized>(
    input: &ParagraphInput<'_>,
    styles: &[TextStyleSpec<'_>],
    faces: &S,
) -> Result<(ScaledLineMetrics, Option<FontSize>), UnitError> {
    let style = input
        .spans
        .first()
        .and_then(|s| usize::try_from(s.style).ok())
        .and_then(|s| styles.get(s))
        .or_else(|| styles.first());
    let Some(style) = style else {
        return Ok((ScaledLineMetrics::default(), None));
    };
    let face = style
        .chain
        .available()
        .find_map(|(_, handle)| faces.face(handle));
    let metrics = match face {
        Some(face) => scale_metrics(face.line_metrics(), style.font_size, face.units_per_em())?,
        None => ScaledLineMetrics::default(),
    };
    Ok((metrics, Some(style.font_size)))
}

/// run 하나를 shaping cluster segment로 나눕니다. glyph가 없는 run은 grapheme segment(폭 0)입니다.
///
/// glyph를 cluster 오름차순으로 한 번 정렬한 뒤 segment 순서로 한 번만 훑어 advance를 더합니다. segment
/// 마다 run 전체 glyph를 다시 훑지 않으므로 비용은 glyph 수 n에 대해 O(n log n)입니다.
fn push_segments(
    text: &str,
    range: &Range<usize>,
    glyphs: &[PositionedGlyph],
    run: usize,
    out: &mut Vec<Segment>,
) -> Result<(), UnitError> {
    // (cluster, advance). RTL run은 glyph가 cluster 내림차순이므로 정렬해 논리 순서로 맞춥니다.
    let mut by_cluster: Vec<(usize, i64)> = glyphs
        .iter()
        .map(|g| (g.cluster as usize, i64::from(g.x_advance.raw())))
        .collect();
    by_cluster.sort_unstable();
    let mut starts: Vec<usize> = if glyphs.is_empty() {
        text[range.clone()]
            .grapheme_indices(true)
            .map(|(i, _)| range.start + i)
            .collect()
    } else {
        by_cluster.iter().map(|&(cluster, _)| cluster).collect()
    };
    starts.push(range.start);
    starts.sort_unstable();
    starts.dedup();
    // by_cluster에서 아직 segment에 더하지 않은 첫 glyph 위치.
    let mut next = 0;
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(range.end);
        if start >= end {
            continue;
        }
        // segment `[start, end)`에 cluster가 든 glyph만 더합니다. start 앞 glyph는 앞 segment 몫입니다.
        while by_cluster
            .get(next)
            .is_some_and(|&(cluster, _)| cluster < start)
        {
            crate::op_count::tick(1);
            next += 1;
        }
        let mut advance = 0i64;
        while let Some(&(cluster, glyph_advance)) = by_cluster.get(next) {
            if cluster >= end {
                break;
            }
            crate::op_count::tick(1);
            advance += glyph_advance;
            next += 1;
        }
        out.push(Segment {
            range: start..end,
            advance: TextUnit::from_i64(advance)?,
            is_whitespace: text[start..end].chars().all(is_hanging_whitespace),
            run,
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::font_runs::tests::FontSet;
    use crate::test_fonts::{cjk_spec, latin_liga_spec, latin_rtl_spec};

    /// Latin(liga) → CJK → Latin+히브리어·아랍어 chain, 16px 단일 style fixture.
    pub(crate) struct Fixture {
        pub set: FontSet,
        pub chain: crate::fallback::FontFallbackChain,
    }

    impl Fixture {
        pub(crate) fn new() -> Fixture {
            let set = FontSet::new(&[latin_liga_spec(), cjk_spec(), latin_rtl_spec()]);
            let chain = set.chain(&[0, 1, 2]);
            Fixture { set, chain }
        }

        pub(crate) fn styles(&self, px: i32) -> Vec<TextStyleSpec<'_>> {
            vec![TextStyleSpec {
                chain: self.chain.resolve(&self.set.registry),
                shaping: ShapingStyle::default(),
                font_size: FontSize::from_px(px).expect("크기"),
            }]
        }

        pub(crate) fn shape(&self, text: &str, px: i32, base: BaseDirection) -> ShapedParagraph {
            let spans = vec![StyleSpan {
                range: 0..text.len(),
                style: 0,
                language: None,
            }];
            shape_paragraph(
                &ParagraphInput {
                    text,
                    spans: &spans,
                    base_direction: base,
                },
                &self.styles(px),
                &self.set.faces,
            )
            .expect("shape")
        }
    }

    /// 테스트용: 정렬 불변식이 깨진 shaping 결과를 만들려고 run·segment·itemization을 직접 고칩니다(debug
    /// 빌드 panic 검사).
    #[cfg(debug_assertions)]
    pub(crate) fn parts_mut(
        para: &mut ShapedParagraph,
    ) -> (&mut Vec<GlyphRun>, &mut Vec<Segment>, &mut Itemization) {
        (&mut para.runs, &mut para.segments, &mut para.itemization)
    }

    #[test]
    fn segments_follow_shaping_clusters_and_scale_to_subpixels() {
        let fx = Fixture::new();
        // 16px, upem 1000: 500fu → 512 subpx, ffi ligature 800fu → 819.2 → 819.
        let para = fx.shape("office 日本", 16, BaseDirection::LeftToRight);
        let segments: Vec<(Range<usize>, i32, bool)> = para
            .segments()
            .iter()
            .map(|s| (s.range.clone(), s.advance.raw(), s.is_whitespace))
            .collect();
        assert_eq!(
            segments,
            vec![
                (0..1, 512, false),
                // ligature cluster는 grapheme 3개를 한 segment로 묶습니다.
                (1..4, 819, false),
                (4..5, 512, false),
                (5..6, 512, false),
                (6..7, 256, true),
                (7..10, 1024, false),
                (10..13, 1024, false),
            ]
        );
        assert!(para.is_segment_boundary(1));
        assert!(!para.is_segment_boundary(2));
        assert!(para.is_segment_boundary(13));
        assert_eq!(para.runs().len(), 2);
        assert_eq!(
            para.runs()[0].metrics,
            ScaledLineMetrics {
                ascent: TextUnit::from_raw(819),
                descent: TextUnit::from_raw(-205),
                line_gap: TextUnit::ZERO
            }
        );
        assert_eq!(para.default_metrics(), para.runs()[0].metrics);
        assert!(para.missing().is_empty());
        assert_eq!(para.text(), "office 日本");
    }

    #[test]
    fn segment_work_is_linear_in_glyph_count() {
        // posthoc TS-1 회귀: segment마다 run 전체 glyph를 다시 훑으면 작업량이 glyph 수의 제곱입니다.
        // 단일 run Latin paragraph는 glyph·segment가 글자마다 하나입니다.
        let fx = Fixture::new();
        let work = |n: usize| {
            let text = "abcd ".repeat(n / 5);
            let (para, units) =
                crate::op_count::measure(|| fx.shape(&text, 16, BaseDirection::LeftToRight));
            assert_eq!(para.runs().len(), 1);
            assert_eq!(para.segments().len(), n);
            units
        };
        let (small, large) = (work(2_000), work(8_000));
        assert!(
            large <= 4 * 8_000,
            "glyph 하나당 상수 번만 방문해야 합니다: 작업량 {large}"
        );
        assert!(
            large <= 5 * small,
            "입력 4배에 작업량 {small} → {large}(선형이면 4배 남짓, 제곱이면 16배)"
        );
    }

    #[test]
    fn run_segment_and_bidi_ranges_tile_the_text() {
        // 줄 배치의 이분 탐색이 기대는 불변식을 release 빌드에서도 고정합니다(debug 빌드는 shape_paragraph가
        // 직접 확인). ligature, CJK fallback, RTL, missing script, 결합 부호, ZWJ, BiDi 제어, U+2028·U+2029.
        let fx = Fixture::new();
        let texts = [
            "office 日本",
            "abc שלום مرحبا 123",
            "\u{0E01}\u{0E02}\u{0E03} a",
            "e\u{0301} \u{1F469}\u{200D}\u{1F4BB}",
            "a\u{202E}bc\u{202C}d",
            "a\u{2028}b\u{2029}\u{05D0}\u{2029}c",
        ];
        for text in texts {
            for base in [BaseDirection::LeftToRight, BaseDirection::RightToLeft] {
                let para = fx.shape(text, 16, base);
                let len = text.len();
                let runs = para.runs().iter().map(|r| &r.range);
                let segments = para.segments().iter().map(|s| &s.range);
                let bidi = para.itemization().bidi.paragraphs().iter().map(|(r, _)| r);
                assert!(crate::tiling::tiles(runs, len), "{text:?} run");
                assert!(crate::tiling::tiles(segments, len), "{text:?} segment");
                assert!(crate::tiling::tiles(bidi, len), "{text:?} BiDi paragraph");
            }
        }
    }

    #[test]
    fn newline_in_paragraph_is_rejected() {
        let fx = Fixture::new();
        let spans = vec![StyleSpan {
            range: 0..3,
            style: 0,
            language: None,
        }];
        let err = shape_paragraph(
            &ParagraphInput {
                text: "a\nb",
                spans: &spans,
                base_direction: BaseDirection::LeftToRight,
            },
            &fx.styles(16),
            &fx.set.faces,
        );
        assert_eq!(
            err.map(|_| ()),
            Err(TextLayoutError::ParagraphSeparatorInText { offset: 1 })
        );
        assert!(TextLayoutError::NegativeWidth.to_string().contains("음수"));
        assert!(is_hanging_whitespace('\u{3000}'));
        assert!(!is_hanging_whitespace('\u{00A0}'));
    }
}
