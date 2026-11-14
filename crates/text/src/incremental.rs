//! 증분 텍스트 레이아웃 (X.10.6).
//!
//! [`IncrementalTextLayout`]은 paragraph마다 shaping 결과와 줄 레이아웃 결과를 X.10.5 key로 cache합니다.
//! 텍스트 일부를 편집하면 내용·style이 바뀐 paragraph만 다시 shaping·줄 나누기를 하고, 바뀌지 않은
//! paragraph는 cache 결과를 재사용합니다. 그 뒤 모든 줄의 위치(y·정렬)만 다시 계산합니다(X.10.2
//! `compose_text`, glyph 수에 비례하는 저비용 단계). paragraph 내용으로 key를 만들므로 paragraph가 삽입·
//! 삭제되어 index가 밀려도 내용이 같은 paragraph는 재사용됩니다.
//!
//! cache 항목은 key가 hash로만 요약한 원본 입력 전체(paragraph 텍스트·span·기준 방향, paragraph가 쓰는
//! style의 글꼴 크기·feature·variation·fallback chain 항목·사용 가능 handle, 줄 레이아웃은 paragraph 속성·
//! 줄바꿈 옵션까지)를 검증 값으로 보관해 hash 충돌로 다른 결과를 쓰지 않습니다. 레이아웃이 끝나면 이번 요청에
//! 쓰지 않은 항목을 지워 메모리를 문서 크기로 제한합니다.
//!
//! [`affected_paragraphs`]는 편집 범위가 바꾸는 paragraph 구간을 계산하는 helper입니다(문서 모델 편집
//! helper VI.7.5가 바꾼 범위를 layout 무효화 범위로 옮길 때 사용).

use std::collections::BTreeSet;
use std::ops::Range;

use crate::bidi::BaseDirection;
use crate::cache::{
    loaded_handles, paragraph_styles, style_at, LayoutCache, LineLayoutCacheKey, ShapingCacheKey,
    TargetProfileKey,
};
use crate::fallback::FontFallbackChain;
use crate::font_runs::{FaceSource, ShapingStyle};
use crate::itemize::StyleSpan;
use crate::line_break::{line_break_opportunities, LineBreakOptions};
use crate::paragraph::{
    shape_paragraph, ParagraphInput, ShapedParagraph, TextLayoutError, TextStyleSpec,
};
use crate::registry::FontHandle;
use crate::text_layout::{
    compose_text, lay_out_paragraph, ParagraphLayoutStyle, ParagraphLines, TextBox, TextLayout,
};
use crate::units::{FontSize, TextUnit};

/// paragraph 하나의 입력.
#[derive(Debug, Clone, Copy)]
pub struct ParagraphSpec<'a> {
    /// paragraph 텍스트(`'\n'` 없음).
    pub text: &'a str,
    /// paragraph 안 style span.
    pub spans: &'a [StyleSpan],
    /// 기준 방향.
    pub base_direction: BaseDirection,
    /// paragraph 레이아웃 속성.
    pub style: ParagraphLayoutStyle,
}

/// 레이아웃 요청.
#[derive(Debug, Clone, Copy)]
pub struct LayoutRequest<'a> {
    /// paragraph(순서대로).
    pub paragraphs: &'a [ParagraphSpec<'a>],
    /// style 표.
    pub styles: &'a [TextStyleSpec<'a>],
    /// 줄바꿈 옵션(locale 포함).
    pub line_break: &'a LineBreakOptions,
    /// target profile.
    pub target_profile: TargetProfileKey,
    /// 너비 제약.
    pub max_width: Option<TextUnit>,
    /// paragraph 간격.
    pub paragraph_spacing: TextUnit,
    /// text box.
    pub text_box: TextBox,
}

/// 이번 레이아웃에서 다시 계산한 paragraph index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutReport {
    /// 다시 shaping한 paragraph.
    pub reshaped: Vec<usize>,
    /// 줄을 다시 나눈 paragraph.
    pub relaid: Vec<usize>,
}

/// shaping cache 검증 값: key가 hash로만 요약한 원본 입력 전체. key가 같아도(hash 충돌) 이 값이 다르면
/// miss입니다. locale·target profile은 key에 원본 그대로 있어 key 비교로 충분합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShapingCheck {
    text: String,
    spans: Vec<StyleSpan>,
    base: BaseDirection,
    /// paragraph가 쓰는 style([`paragraph_styles`])의 원본 값(index 오름차순).
    styles: Vec<StyleCheck>,
}

/// style 하나의 원본 값. style 표에 없는 index는 `value`가 `None`입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleCheck {
    index: u32,
    value: Option<StyleValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleValue {
    font_size: FontSize,
    shaping: ShapingStyle,
    /// fallback chain 항목(file hash·face index·axes).
    chain: FontFallbackChain,
    /// 항목별 사용 가능한(registry에 있고 로드된) handle. 결과 run이 handle을 담으므로 사용 가능 여부와 함께
    /// handle 정체성도 비교합니다.
    loaded: Vec<Option<FontHandle>>,
}

/// 줄 레이아웃 cache 검증 값: shaping 입력과 줄 key가 hash로만 요약한 paragraph 속성·줄바꿈 옵션 원본. 너비
/// 제약은 key에 원본 그대로 있습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineCheck {
    shaping: ShapingCheck,
    style: ParagraphLayoutStyle,
    line_break: LineBreakOptions,
}

impl ShapingCheck {
    fn new<S: FaceSource + ?Sized>(
        input: &ParagraphInput<'_>,
        styles: &[TextStyleSpec<'_>],
        faces: &S,
    ) -> ShapingCheck {
        let styles = paragraph_styles(input, styles.len())
            .into_iter()
            .map(|index| StyleCheck {
                index,
                value: style_at(styles, index).map(|style| StyleValue {
                    font_size: style.font_size,
                    shaping: style.shaping.clone(),
                    chain: style.chain.chain().clone(),
                    loaded: loaded_handles(style, faces),
                }),
            })
            .collect();
        ShapingCheck {
            text: input.text.to_string(),
            spans: input.spans.to_vec(),
            base: input.base_direction,
            styles,
        }
    }
}

/// 증분 레이아웃 엔진 (X.10.6).
#[derive(Debug, Default)]
pub struct IncrementalTextLayout {
    shaping: LayoutCache<ShapingCacheKey, ShapingCheck, ShapedParagraph>,
    lines: LayoutCache<LineLayoutCacheKey, LineCheck, ParagraphLines>,
    shape_calls: u64,
    line_calls: u64,
}

impl IncrementalTextLayout {
    /// 빈 엔진.
    pub fn new() -> IncrementalTextLayout {
        IncrementalTextLayout::default()
    }

    /// 지금까지 paragraph shaping 횟수.
    pub fn shape_calls(&self) -> u64 {
        self.shape_calls
    }

    /// 지금까지 paragraph 줄 나누기 횟수.
    pub fn line_layout_calls(&self) -> u64 {
        self.line_calls
    }

    /// 요청을 레이아웃합니다. 바뀐 paragraph만 다시 계산합니다.
    pub fn layout<S: FaceSource + ?Sized>(
        &mut self,
        request: &LayoutRequest<'_>,
        faces: &S,
    ) -> Result<(TextLayout, LayoutReport), TextLayoutError> {
        let mut report = LayoutReport::default();
        let mut keys = Vec::with_capacity(request.paragraphs.len());
        for (index, paragraph) in request.paragraphs.iter().enumerate() {
            let input = ParagraphInput {
                text: paragraph.text,
                spans: paragraph.spans,
                base_direction: paragraph.base_direction,
            };
            let shaping_key = ShapingCacheKey::new(
                &input,
                request.styles,
                faces,
                &request.line_break.locale,
                request.target_profile,
            );
            let check = ShapingCheck::new(&input, request.styles, faces);
            if self.shaping.get(&shaping_key, &check).is_none() {
                let shaped = shape_paragraph(&input, request.styles, faces)?;
                self.shape_calls += 1;
                report.reshaped.push(index);
                self.shaping
                    .insert(shaping_key.clone(), check.clone(), shaped);
            }
            let line_key = LineLayoutCacheKey::new(
                shaping_key.clone(),
                &paragraph.style,
                request.line_break,
                request.max_width,
            );
            let line_check = LineCheck {
                shaping: check,
                style: paragraph.style,
                line_break: request.line_break.clone(),
            };
            if self.lines.get(&line_key, &line_check).is_none() {
                let shaped = self
                    .shaping
                    .peek(&shaping_key)
                    .ok_or(TextLayoutError::CacheEntryMissing)?;
                let breaks = line_break_opportunities(shaped.text(), request.line_break);
                let laid = lay_out_paragraph(shaped, &breaks, &paragraph.style, request.max_width)?;
                self.line_calls += 1;
                report.relaid.push(index);
                self.lines.insert(line_key.clone(), line_check, laid);
            }
            keys.push((shaping_key, line_key));
        }

        let mut paragraphs: Vec<&ParagraphLines> = Vec::with_capacity(keys.len());
        for (_, line_key) in &keys {
            paragraphs.push(
                self.lines
                    .peek(line_key)
                    .ok_or(TextLayoutError::CacheEntryMissing)?,
            );
        }
        let layout = compose_text(&paragraphs, request.paragraph_spacing, &request.text_box)?;

        // 이번 요청에 쓰인 항목만 남깁니다.
        let live_shaping: BTreeSet<ShapingCacheKey> = keys.iter().map(|(s, _)| s.clone()).collect();
        let live_lines: BTreeSet<LineLayoutCacheKey> = keys.into_iter().map(|(_, l)| l).collect();
        self.shaping.retain(|k| live_shaping.contains(k));
        self.lines.retain(|k| live_lines.contains(k));
        Ok((layout, report))
    }
}

/// 편집으로 바뀌는 paragraph 구간.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedParagraphs {
    /// 편집 전 텍스트의 paragraph index 범위.
    pub old: Range<usize>,
    /// 편집 후 텍스트의 paragraph index 범위.
    pub new: Range<usize>,
}

/// `'\n'`으로 나눈 paragraph byte 범위(`'\n'` 제외). 빈 텍스트도 paragraph 하나입니다.
pub fn paragraph_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, _) in text.match_indices('\n') {
        out.push(start..i);
        start = i + 1;
    }
    out.push(start..text.len());
    out
}

/// 전체 텍스트의 `edit` byte 범위를 `replacement`로 바꿀 때 바뀌는 paragraph 구간을 계산합니다. 범위 앞
/// paragraph는 그대로이고, 범위 뒤 paragraph는 index만 `new.len() - old.len()`만큼 밀립니다.
///
/// `edit`이 문자 경계가 아니거나 텍스트를 벗어나면 `None`입니다.
pub fn affected_paragraphs(
    old_text: &str,
    edit: Range<usize>,
    replacement: &str,
) -> Option<AffectedParagraphs> {
    if edit.start > edit.end
        || edit.end > old_text.len()
        || !old_text.is_char_boundary(edit.start)
        || !old_text.is_char_boundary(edit.end)
    {
        return None;
    }
    let first = old_text[..edit.start].matches('\n').count();
    let old_last = first + old_text[edit.clone()].matches('\n').count();
    let new_last = first + replacement.matches('\n').count();
    Some(AffectedParagraphs {
        old: first..old_last + 1,
        new: first..new_last + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paragraph::tests::Fixture;
    use crate::shaping::LanguageTag;

    fn spans(text: &str) -> Vec<StyleSpan> {
        vec![StyleSpan {
            range: 0..text.len(),
            style: 0,
            language: None,
        }]
    }

    struct Doc {
        texts: Vec<String>,
        spans: Vec<Vec<StyleSpan>>,
    }

    impl Doc {
        fn new(texts: &[&str]) -> Doc {
            Doc {
                texts: texts.iter().map(|t| t.to_string()).collect(),
                spans: texts.iter().map(|t| spans(t)).collect(),
            }
        }

        fn set(&mut self, index: usize, text: &str) {
            self.texts[index] = text.to_string();
            self.spans[index] = spans(text);
        }

        fn specs(&self) -> Vec<ParagraphSpec<'_>> {
            self.texts
                .iter()
                .zip(&self.spans)
                .map(|(text, spans)| ParagraphSpec {
                    text,
                    spans,
                    base_direction: BaseDirection::LeftToRight,
                    style: ParagraphLayoutStyle::default(),
                })
                .collect()
        }
    }

    fn run(
        engine: &mut IncrementalTextLayout,
        fx: &Fixture,
        doc: &Doc,
        width: i32,
    ) -> (TextLayout, LayoutReport) {
        let specs = doc.specs();
        let styles = fx.styles(16);
        let options = LineBreakOptions::new(LanguageTag::parse("en").expect("en"));
        engine
            .layout(
                &LayoutRequest {
                    paragraphs: &specs,
                    styles: &styles,
                    line_break: &options,
                    target_profile: TargetProfileKey(1),
                    max_width: Some(TextUnit::from_px(width).expect("폭")),
                    paragraph_spacing: TextUnit::from_px(8).expect("간격"),
                    text_box: TextBox::default(),
                },
                &fx.set.faces,
            )
            .expect("layout")
    }

    #[test]
    fn first_paragraph_edit_does_not_reshape_independent_second_paragraph() {
        let fx = Fixture::new();
        let mut engine = IncrementalTextLayout::new();
        let mut doc = Doc::new(&["first line", "second paragraph"]);
        let (before, report) = run(&mut engine, &fx, &doc, 200);
        assert_eq!(report.reshaped, vec![0, 1]);
        assert_eq!(report.relaid, vec![0, 1]);
        assert_eq!((engine.shape_calls(), engine.line_layout_calls()), (2, 2));

        // 첫 paragraph만 편집해 줄 수가 늘게 합니다(200px에서 두 줄).
        doc.set(0, "first line grows much longer now");
        let (after, report) = run(&mut engine, &fx, &doc, 200);
        assert_eq!(report.reshaped, vec![0]);
        assert_eq!(report.relaid, vec![0]);
        assert_eq!((engine.shape_calls(), engine.line_layout_calls()), (3, 3));

        // 둘째 paragraph 줄은 재shaping 없이 glyph가 같고, 위치(y)만 첫 paragraph 높이 변화만큼 밀립니다.
        let second_before: Vec<_> = before.lines.iter().filter(|l| l.paragraph == 1).collect();
        let second_after: Vec<_> = after.lines.iter().filter(|l| l.paragraph == 1).collect();
        assert_eq!(second_before.len(), second_after.len());
        let first_lines_before = before.lines.iter().filter(|l| l.paragraph == 0).count();
        let first_lines_after = after.lines.iter().filter(|l| l.paragraph == 0).count();
        assert_eq!((first_lines_before, first_lines_after), (1, 2));
        for (b, a) in second_before.iter().zip(&second_after) {
            assert_eq!(a.top.raw() - b.top.raw(), 1024);
            let bx: Vec<_> = b.glyphs.iter().map(|g| (g.glyph, g.x)).collect();
            let ax: Vec<_> = a.glyphs.iter().map(|g| (g.glyph, g.x)).collect();
            assert_eq!(bx, ax);
        }
    }

    #[test]
    fn edits_width_changes_and_paragraph_insertions_reuse_unchanged_work() {
        let fx = Fixture::new();
        let mut engine = IncrementalTextLayout::new();
        let mut doc = Doc::new(&["alpha beta", "gamma delta", "epsilon"]);
        run(&mut engine, &fx, &doc, 300);
        // 둘째 paragraph만 편집.
        doc.set(1, "gamma delta zeta");
        let (_, report) = run(&mut engine, &fx, &doc, 300);
        assert_eq!(report.reshaped, vec![1]);
        // 같은 입력을 다시 레이아웃하면 아무것도 다시 계산하지 않습니다.
        let (_, report) = run(&mut engine, &fx, &doc, 300);
        assert_eq!(report, LayoutReport::default());
        // 너비만 바꾸면 shaping은 재사용하고 줄만 다시 나눕니다.
        let (_, report) = run(&mut engine, &fx, &doc, 120);
        assert!(report.reshaped.is_empty());
        assert_eq!(report.relaid, vec![0, 1, 2]);
        // paragraph를 앞에 삽입해도 내용이 같은 기존 paragraph는 재사용합니다(index만 밀림).
        let mut inserted = Doc::new(&["new first", "alpha beta", "gamma delta zeta", "epsilon"]);
        let (layout, report) = run(&mut engine, &fx, &inserted, 120);
        assert_eq!(report.reshaped, vec![0]);
        assert_eq!(report.relaid, vec![0]);
        assert_eq!(layout.lines.last().map(|l| l.paragraph), Some(3));
        // 쓰이지 않은 항목은 정리되어 cache 크기가 paragraph 수를 넘지 않습니다.
        inserted.set(0, "replaced");
        run(&mut engine, &fx, &inserted, 120);
        assert_eq!(engine.shaping.len(), 4);
        assert_eq!(engine.lines.len(), 4);
    }

    /// 글꼴 크기만 다른 fixture style.
    fn sized_style(fx: &Fixture, px: i32) -> TextStyleSpec<'_> {
        TextStyleSpec {
            chain: fx.chain.resolve(&fx.set.registry),
            shaping: crate::font_runs::ShapingStyle::default(),
            font_size: crate::units::FontSize::from_px(px).expect("크기"),
        }
    }

    #[test]
    fn style_change_reshapes_only_paragraphs_using_that_style() {
        // posthoc TS-4 회귀: style·font face hash가 style 표 전체를 담으면, 다른 paragraph만 쓰는 style이
        // 바뀌어도 모든 paragraph를 다시 shaping합니다.
        let fx = Fixture::new();
        let texts = ["first uses style zero", "second uses style one", ""];
        let spans = [
            spans(texts[0]),
            vec![StyleSpan {
                range: 0..texts[1].len(),
                style: 1,
                language: None,
            }],
            // span 없는 빈 paragraph: 기본 줄 metric은 첫 style을 씁니다.
            vec![],
        ];
        let specs: Vec<ParagraphSpec<'_>> = texts
            .iter()
            .zip(&spans)
            .map(|(text, spans)| ParagraphSpec {
                text,
                spans,
                base_direction: BaseDirection::LeftToRight,
                style: ParagraphLayoutStyle::default(),
            })
            .collect();
        let options = LineBreakOptions::new(LanguageTag::parse("en").expect("en"));
        let layout = |engine: &mut IncrementalTextLayout, sizes: [i32; 2]| {
            let styles = vec![sized_style(&fx, sizes[0]), sized_style(&fx, sizes[1])];
            engine
                .layout(
                    &LayoutRequest {
                        paragraphs: &specs,
                        styles: &styles,
                        line_break: &options,
                        target_profile: TargetProfileKey(1),
                        max_width: Some(TextUnit::from_px(200).expect("폭")),
                        paragraph_spacing: TextUnit::ZERO,
                        text_box: TextBox::default(),
                    },
                    &fx.set.faces,
                )
                .expect("layout")
        };
        let mut engine = IncrementalTextLayout::new();
        let (_, report) = layout(&mut engine, [16, 16]);
        assert_eq!(report.reshaped, vec![0, 1, 2]);
        // style 1(둘째 paragraph만 씀)만 바꾸면 둘째만 다시 shaping·줄 나누기합니다.
        let (after, report) = layout(&mut engine, [16, 20]);
        assert_eq!(report.reshaped, vec![1]);
        assert_eq!(report.relaid, vec![1]);
        assert_eq!(after, layout(&mut IncrementalTextLayout::new(), [16, 20]).0);
        // style 0은 첫 paragraph와 span 없는 빈 paragraph(기본 줄 metric)가 씁니다.
        let (after, report) = layout(&mut engine, [18, 20]);
        assert_eq!(report.reshaped, vec![0, 2]);
        assert_eq!(report.relaid, vec![0, 2]);
        assert_eq!(after, layout(&mut IncrementalTextLayout::new(), [18, 20]).0);
    }

    #[test]
    fn hash_collision_between_different_inputs_is_a_miss() {
        // posthoc TS-4 회귀: cache 검증 값이 텍스트·span·기준 방향만 비교하면, style 값이나 줄바꿈 옵션만 다른
        // 입력의 key가 hash 충돌로 같을 때 앞 입력의 결과를 그대로 돌려줍니다. 충돌은 cache 항목을 다른 입력의
        // key로 옮겨 재현합니다.
        let fx = Fixture::new();
        let text = "한국어 텍스트";
        let text_spans = spans(text);
        let specs = [ParagraphSpec {
            text,
            spans: &text_spans,
            base_direction: BaseDirection::LeftToRight,
            style: ParagraphLayoutStyle::default(),
        }];
        let normal = LineBreakOptions::new(LanguageTag::parse("ko").expect("ko"));
        let keep_all = LineBreakOptions {
            word_break: crate::line_break::WordBreak::KeepAll,
            ..normal.clone()
        };
        let width = Some(TextUnit::from_px(40).expect("폭"));
        let layout = |engine: &mut IncrementalTextLayout, px: i32, options: &LineBreakOptions| {
            let styles = vec![sized_style(&fx, px)];
            engine
                .layout(
                    &LayoutRequest {
                        paragraphs: &specs,
                        styles: &styles,
                        line_break: options,
                        target_profile: TargetProfileKey(1),
                        max_width: width,
                        paragraph_spacing: TextUnit::ZERO,
                        text_box: TextBox::default(),
                    },
                    &fx.set.faces,
                )
                .expect("layout")
        };
        let keys = |px: i32, options: &LineBreakOptions| {
            let styles = vec![sized_style(&fx, px)];
            let input = ParagraphInput {
                text,
                spans: &text_spans,
                base_direction: BaseDirection::LeftToRight,
            };
            let shaping = ShapingCacheKey::new(
                &input,
                &styles,
                &fx.set.faces,
                &options.locale,
                TargetProfileKey(1),
            );
            let line = LineLayoutCacheKey::new(
                shaping.clone(),
                &ParagraphLayoutStyle::default(),
                options,
                width,
            );
            (shaping, line)
        };

        // 1. 글꼴 크기만 다른 입력: 16px 항목이 20px 요청 key와 충돌.
        let mut engine = IncrementalTextLayout::new();
        layout(&mut engine, 16, &normal);
        let (from_shaping, from_line) = keys(16, &normal);
        let (to_shaping, to_line) = keys(20, &normal);
        assert_ne!(from_shaping, to_shaping);
        assert!(engine.shaping.rekey(&from_shaping, to_shaping));
        assert!(engine.lines.rekey(&from_line, to_line));
        let (got, report) = layout(&mut engine, 20, &normal);
        assert_eq!(report.reshaped, vec![0]);
        assert_eq!(
            got,
            layout(&mut IncrementalTextLayout::new(), 20, &normal).0
        );

        // 2. 줄바꿈 옵션만 다른 입력: Normal 줄 항목이 KeepAll 요청 key와 충돌(shaping key는 같음).
        let mut engine = IncrementalTextLayout::new();
        let (normal_layout, _) = layout(&mut engine, 16, &normal);
        let (normal_shaping, from_line) = keys(16, &normal);
        let (keep_all_shaping, to_line) = keys(16, &keep_all);
        assert_eq!(normal_shaping, keep_all_shaping);
        assert!(engine.lines.rekey(&from_line, to_line));
        let (got, report) = layout(&mut engine, 16, &keep_all);
        assert!(report.reshaped.is_empty());
        assert_eq!(report.relaid, vec![0]);
        let expected = layout(&mut IncrementalTextLayout::new(), 16, &keep_all).0;
        assert_ne!(
            expected, normal_layout,
            "두 옵션의 줄 나누기가 달라야 합니다"
        );
        assert_eq!(got, expected);
    }

    #[test]
    fn affected_paragraph_ranges() {
        let text = "one\ntwo\nthree";
        assert_eq!(paragraph_ranges(text), vec![0..3, 4..7, 8..13]);
        assert_eq!(paragraph_ranges(""), vec![0..0]);
        assert_eq!(paragraph_ranges("a\n"), vec![0..1, 2..2]);
        // 첫 paragraph 안 편집: paragraph 0만.
        assert_eq!(
            affected_paragraphs(text, 1..2, "X"),
            Some(AffectedParagraphs {
                old: 0..1,
                new: 0..1
            })
        );
        // 줄바꿈 삽입: 0 → 0..2(이후 paragraph는 하나씩 밀림).
        assert_eq!(
            affected_paragraphs(text, 1..1, "\n"),
            Some(AffectedParagraphs {
                old: 0..1,
                new: 0..2
            })
        );
        // paragraph 경계를 지우면(1..2 병합) 두 paragraph가 하나로.
        assert_eq!(
            affected_paragraphs(text, 3..4, ""),
            Some(AffectedParagraphs {
                old: 0..2,
                new: 0..1
            })
        );
        // 마지막 paragraph 편집.
        assert_eq!(
            affected_paragraphs(text, 9..10, "hh"),
            Some(AffectedParagraphs {
                old: 2..3,
                new: 2..3
            })
        );
        assert_eq!(affected_paragraphs(text, 5..99, ""), None);
        assert_eq!(affected_paragraphs("한", 1..2, ""), None);
    }
}
