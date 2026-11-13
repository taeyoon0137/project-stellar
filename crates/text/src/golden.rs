//! glyph id·advance·줄바꿈 golden 비교 (X.10.7, 테스트 전용).
//!
//! 고정 폰트 파일(합성 fixture 폰트, bytes fingerprint를 golden에 기록)과 고정 입력으로 전체 pipeline
//! (itemization → fallback → shaping → 줄 나누기 → 정렬)을 돌려 [`TextLayout::encode_logical`] 결과를
//! `tests/golden/logical_layout_v1.txt`와 비교합니다. 파일에는 사람이 읽을 수 있는 줄·glyph dump와 canonical
//! bytes의 FNV-1a 128 digest가 함께 있습니다. 플랫폼 간 비교는 같은 golden을 다른 플랫폼에서 통과시키는
//! 것으로 합니다(ADR-0006: 현재 macOS만 실행).
//!
//! golden 갱신: `STELLAR_TEXT_UPDATE_GOLDEN=1 cargo test -p stellar-text golden` (의도한 변경일 때만).

use std::fmt::Write as _;

use crate::bidi::BaseDirection;
use crate::cache::{StableHasher, TargetProfileKey};
use crate::fallback::FontFallbackChain;
use crate::font::tests::axis;
use crate::font::FontVariation;
use crate::font_runs::tests::FontSet;
use crate::font_runs::ShapingStyle;
use crate::incremental::{IncrementalTextLayout, LayoutRequest, ParagraphSpec};
use crate::itemize::StyleSpan;
use crate::line_break::{LineBreakOptions, LineBreakStrictness, WordBreak};
use crate::line_layout::WrapMode;
use crate::paragraph::TextStyleSpec;
use crate::shaping::{FontFeature, LanguageTag};
use crate::test_fonts::{
    cjk_spec, emoji_spec, latin_liga_spec, latin_rtl_spec, latin_spec, variable_latin_spec,
    FontSpec,
};
use crate::text_layout::{
    LineHeight, LineHeightRatio, ParagraphLayoutStyle, TextAlign, TextBox, TextLayout,
    VerticalAlign,
};
use crate::units::{FontSize, TextUnit};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/logical_layout_v1.txt"
);
const GOLDEN: &str = include_str!("../tests/golden/logical_layout_v1.txt");

/// style 하나(글꼴 크기 px, feature, variation, fallback chain 순서 — 비면 case 폰트 순서 그대로).
struct StyleDef {
    px: i32,
    features: Vec<FontFeature>,
    variations: Vec<FontVariation>,
    chain: Vec<usize>,
}

impl StyleDef {
    fn plain(px: i32) -> StyleDef {
        StyleDef {
            px,
            features: vec![],
            variations: vec![],
            chain: vec![],
        }
    }
}

/// paragraph 하나(텍스트, (byte 범위, style) span, 기준 방향, paragraph 속성).
struct ParagraphDef {
    text: &'static str,
    spans: Vec<(std::ops::Range<usize>, u32)>,
    base: BaseDirection,
    style: ParagraphLayoutStyle,
}

impl ParagraphDef {
    fn simple(text: &'static str, base: BaseDirection) -> ParagraphDef {
        ParagraphDef {
            text,
            spans: vec![(0..text.len(), 0)],
            base,
            style: ParagraphLayoutStyle::default(),
        }
    }
}

struct Case {
    name: &'static str,
    fonts: Vec<FontSpec>,
    styles: Vec<StyleDef>,
    paragraphs: Vec<ParagraphDef>,
    locale: &'static str,
    word_break: WordBreak,
    strictness: LineBreakStrictness,
    width: Option<i32>,
    spacing: i32,
    text_box: TextBox,
}

impl Case {
    fn new(name: &'static str, fonts: Vec<FontSpec>, paragraphs: Vec<ParagraphDef>) -> Case {
        Case {
            name,
            fonts,
            styles: vec![StyleDef::plain(16)],
            paragraphs,
            locale: "en",
            word_break: WordBreak::Normal,
            strictness: LineBreakStrictness::Normal,
            width: None,
            spacing: 0,
            text_box: TextBox::default(),
        }
    }
}

fn tag(s: &str) -> crate::font::OpenTypeTag {
    crate::font::tests::tag(s)
}

fn cases() -> Vec<Case> {
    let ltr = BaseDirection::LeftToRight;
    let rtl = BaseDirection::RightToLeft;
    let mut out = Vec::new();

    let mut c = Case::new(
        "latin-liga-narrow",
        vec![latin_liga_spec()],
        vec![ParagraphDef::simple("office fit fluffy waffle", ltr)],
    );
    c.width = Some(80);
    out.push(c);

    let mut c = Case::new(
        "cjk-ja-kinsoku",
        vec![latin_spec(), cjk_spec()],
        vec![ParagraphDef::simple("日本語の文章。です。キャット", ltr)],
    );
    c.locale = "ja";
    c.width = Some(64);
    out.push(c);

    let mut c = Case::new(
        "hangul-normal",
        vec![latin_spec(), cjk_spec()],
        vec![ParagraphDef::simple("한국어 텍스트 레이아웃 결정론", ltr)],
    );
    c.locale = "ko";
    c.width = Some(60);
    out.push(c);

    let mut c = Case::new(
        "hangul-keep-all",
        vec![latin_spec(), cjk_spec()],
        vec![ParagraphDef::simple("한국어 텍스트 레이아웃 결정론", ltr)],
    );
    c.locale = "ko";
    c.word_break = WordBreak::KeepAll;
    c.strictness = LineBreakStrictness::Strict;
    c.width = Some(60);
    out.push(c);

    out.push(Case::new(
        "bidi-ltr-mixed",
        vec![latin_rtl_spec()],
        vec![ParagraphDef::simple("abc אבג 123 def", ltr)],
    ));

    let mut c = Case::new(
        "bidi-rtl-arabic",
        vec![latin_rtl_spec()],
        vec![
            ParagraphDef::simple("مرحبا بالعالم 2026 ok", rtl),
            ParagraphDef::simple("שלום world 42", rtl),
        ],
    );
    c.width = Some(120);
    c.spacing = 4;
    out.push(c);

    out.push(Case::new(
        "emoji-cjk-fallback",
        vec![latin_spec(), cjk_spec(), emoji_spec()],
        vec![ParagraphDef::simple("Hi 😀 你好 👩\u{200D}💻!", ltr)],
    ));

    let mut c = Case::new(
        "variable-axis-and-features",
        vec![variable_latin_spec(), latin_liga_spec()],
        vec![ParagraphDef {
            text: "Bold fi lighter fi",
            spans: vec![(0..8, 0), (8..18, 1)],
            base: ltr,
            style: ParagraphLayoutStyle::default(),
        }],
    );
    // style 0: variable 폰트 wght 900. style 1: liga 폰트 20px, liga 끔.
    c.styles = vec![
        StyleDef {
            px: 16,
            features: vec![],
            variations: vec![axis("wght", 900.0)],
            chain: vec![0],
        },
        StyleDef {
            px: 20,
            features: vec![FontFeature {
                tag: tag("liga"),
                value: 0,
            }],
            variations: vec![],
            chain: vec![1],
        },
    ];
    out.push(c);

    let ratio = LineHeightRatio::from_raw(1434).unwrap_or(LineHeightRatio::ONE);
    let mut c = Case::new(
        "justify-center-boxed",
        vec![latin_spec()],
        vec![
            ParagraphDef {
                text: "the quick brown fox jumps over the lazy dog",
                spans: vec![(0..43, 0)],
                base: ltr,
                style: ParagraphLayoutStyle {
                    align: TextAlign::Justify,
                    line_height: LineHeight::Multiplier(ratio),
                    wrap: WrapMode::Wrap,
                },
            },
            ParagraphDef {
                text: "centered tail",
                spans: vec![(0..13, 0)],
                base: ltr,
                style: ParagraphLayoutStyle {
                    align: TextAlign::Center,
                    line_height: LineHeight::Fixed(TextUnit::from_raw(1300)),
                    wrap: WrapMode::Wrap,
                },
            },
            ParagraphDef::simple("", ltr),
        ],
    );
    c.width = Some(110);
    c.spacing = 6;
    c.text_box = TextBox {
        width: Some(TextUnit::from_raw(110 * 64)),
        height: Some(TextUnit::from_raw(200 * 64)),
        vertical_align: VerticalAlign::Middle,
    };
    out.push(c);

    out.push(Case::new(
        "missing-glyph-and-hard-break",
        vec![latin_spec()],
        vec![ParagraphDef::simple("a世b\u{2028}next line", ltr)],
    ));
    out
}

fn hex(value: u128) -> String {
    format!("{value:032x}")
}

fn fnv(bytes: &[u8]) -> u128 {
    let mut h = StableHasher::new();
    h.write_bytes(bytes);
    h.finish()
}

/// case 하나를 레이아웃합니다.
fn run(case: &Case) -> (Vec<u128>, TextLayout) {
    let set = FontSet::new(&case.fonts);
    let fingerprints: Vec<u128> = case.fonts.iter().map(|f| fnv(&f.build())).collect();
    let chains: Vec<FontFallbackChain> = case
        .styles
        .iter()
        .map(|s| {
            if s.chain.is_empty() {
                FontFallbackChain::new(set.refs.clone()).expect("chain")
            } else {
                set.chain(&s.chain)
            }
        })
        .collect();
    let styles: Vec<TextStyleSpec<'_>> = case
        .styles
        .iter()
        .zip(&chains)
        .map(|(s, chain)| TextStyleSpec {
            chain: chain.resolve(&set.registry),
            shaping: ShapingStyle::new(s.features.clone(), s.variations.clone()),
            font_size: FontSize::from_px(s.px).expect("크기"),
        })
        .collect();
    let spans: Vec<Vec<StyleSpan>> = case
        .paragraphs
        .iter()
        .map(|p| {
            p.spans
                .iter()
                .map(|(range, style)| StyleSpan {
                    range: range.clone(),
                    style: *style,
                    language: None,
                })
                .collect()
        })
        .collect();
    let specs: Vec<ParagraphSpec<'_>> = case
        .paragraphs
        .iter()
        .zip(&spans)
        .map(|(p, spans)| ParagraphSpec {
            text: p.text,
            spans,
            base_direction: p.base,
            style: p.style,
        })
        .collect();
    let options = LineBreakOptions {
        word_break: case.word_break,
        strictness: case.strictness,
        ..LineBreakOptions::new(LanguageTag::parse(case.locale).expect("locale"))
    };
    let mut engine = IncrementalTextLayout::new();
    let (layout, _) = engine
        .layout(
            &LayoutRequest {
                paragraphs: &specs,
                styles: &styles,
                line_break: &options,
                target_profile: TargetProfileKey(0),
                max_width: case.width.map(|w| TextUnit::from_raw(w * 64)),
                paragraph_spacing: TextUnit::from_raw(case.spacing * 64),
                text_box: case.text_box,
            },
            &set.faces,
        )
        .expect("golden layout");
    (fingerprints, layout)
}

fn dump(case: &Case, fingerprints: &[u128], layout: &TextLayout) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# fixture {}", case.name);
    let fonts: Vec<String> = fingerprints.iter().map(|f| hex(*f)).collect();
    let _ = writeln!(s, "fonts {}", fonts.join(" "));
    let _ = writeln!(s, "digest {}", hex(fnv(&layout.encode_logical())));
    let _ = writeln!(
        s,
        "box w={} h={} content_h={} top={}",
        layout.width.raw(),
        layout.height.raw(),
        layout.content_height.raw(),
        layout.content_top.raw()
    );
    for line in &layout.lines {
        let _ = writeln!(
            s,
            "line p={} l={} r={}..{} top={} h={} base={} x={} w={}",
            line.paragraph,
            line.line,
            line.range.start,
            line.range.end,
            line.top.raw(),
            line.height.raw(),
            line.baseline.raw(),
            line.x.raw(),
            line.width.raw()
        );
        for g in &line.glyphs {
            let _ = writeln!(
                s,
                "  g id={} c={} x={} adv={} j={} dx={} dy={}",
                g.glyph.raw(),
                g.cluster,
                g.x.raw(),
                g.advance.raw(),
                g.justify_extra.raw(),
                g.x_offset.raw(),
                g.y_offset.raw()
            );
        }
    }
    s
}

fn render_all() -> String {
    let mut out = String::from(
        "# stellar-text logical layout golden v1 (X.10.7). 생성: STELLAR_TEXT_UPDATE_GOLDEN=1 cargo test -p stellar-text golden\n",
    );
    for case in cases() {
        let (fingerprints, layout) = run(&case);
        out.push_str(&dump(&case, &fingerprints, &layout));
    }
    out
}

#[test]
fn logical_layout_matches_golden() {
    let actual = render_all();
    if std::env::var_os("STELLAR_TEXT_UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN_PATH, &actual).expect("golden 쓰기");
        return;
    }
    if actual != GOLDEN {
        // 첫 차이 줄을 보여 줍니다.
        let diff = actual
            .lines()
            .zip(GOLDEN.lines())
            .enumerate()
            .find(|(_, (a, g))| a != g)
            .map(|(i, (a, g))| format!("줄 {}: 실제 `{a}` / golden `{g}`", i + 1))
            .unwrap_or_else(|| "길이가 다릅니다".to_string());
        panic!("논리 레이아웃이 golden과 다릅니다: {diff}");
    }
}

#[test]
fn logical_layout_bytes_are_identical_across_independent_runs() {
    // 폰트를 다시 만들고 엔진·registry를 새로 만들어도 canonical bytes가 같습니다.
    for case in cases() {
        let (fa, a) = run(&case);
        let (fb, b) = run(&case);
        assert_eq!(fa, fb, "{}", case.name);
        assert_eq!(a.encode_logical(), b.encode_logical(), "{}", case.name);
    }
}

#[test]
fn encode_logical_header_and_layout() {
    let case = Case::new(
        "header",
        vec![latin_spec()],
        vec![ParagraphDef::simple("ab", BaseDirection::LeftToRight)],
    );
    let (_, layout) = run(&case);
    let bytes = layout.encode_logical();
    assert_eq!(&bytes[..4], b"STXL");
    assert_eq!(&bytes[4..6], &[1, 0]);
    // width(1024) LE.
    assert_eq!(&bytes[6..10], &1024i32.to_le_bytes());
    // 줄 수 1.
    assert_eq!(&bytes[22..26], &1u32.to_le_bytes());
    // 줄 header 16 + 20 bytes, glyph 수 2, glyph 하나 32 bytes.
    assert_eq!(bytes.len(), 26 + 16 + 20 + 4 + 2 * 32);
}
