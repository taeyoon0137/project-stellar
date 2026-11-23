//! SVG text outline 테스트 (XX.3.2).
//!
//! - golden: 여러 run(폰트 2개, 크기 2개, variation 2개, run 색 있음·없음)과 Fill·Stroke paint layer를 가진
//!   Text의 SVG가 golden과 byte 단위로 같습니다.
//! - 외형: paint layer 없는 같은 Text의 SVG를 테스트 rasterizer로 칠한 결과가, writer를 거치지 않고 glyph
//!   모양 정의에서 직접 칠한 reference와 같고, coverage golden과도 같습니다.

use std::cell::RefCell;
use std::collections::BTreeMap;

use stellar_core_types::value::{Rect, Size2};
use stellar_scene::builder::{NodeInput, StrokeInput, TextInput};
use stellar_scene::geometry::PathCommand;

use super::*;
use crate::svg::text::MAX_TEXT_OUTLINE_COMMANDS;
use crate::text::{
    AxisTag, ExportFontFace, ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey,
    GlyphRun, GlyphVariations, PlacedGlyph, RunSize, TextGlyphLayout,
};

const TEXT_GOLDEN: &str = include_str!("../../../../tests/fixtures/text_view_v1.svg");
const COVERAGE_GOLDEN: &str = include_str!("../../../../tests/fixtures/text_view_v1.coverage.txt");

const FONT_A: ExportFontFace = ExportFontFace::new([0xA1; 32], 0);
const FONT_B: ExportFontFace = ExportFontFace::new([0xB2; 32], 1);
const FONT_MISSING: ExportFontFace = ExportFontFace::new([0xEE; 32], 0);

const LAYOUT: [u8; 16] = [0x51; 16];

/// text node 위치(world).
const ORIGIN: (f32, f32) = (10.0, 5.0);

/// em 단위(y 위 방향) contour: 시작점과 선분·3차 곡선.
#[derive(Debug, Clone, Copy)]
enum Seg {
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
}

type Contour = ((f32, f32), Vec<Seg>);

fn rect_contour(x0: f32, y0: f32, x1: f32, y1: f32, reverse: bool) -> Contour {
    if reverse {
        (
            (x0, y0),
            vec![Seg::Line(x0, y1), Seg::Line(x1, y1), Seg::Line(x1, y0)],
        )
    } else {
        (
            (x0, y0),
            vec![Seg::Line(x1, y0), Seg::Line(x1, y1), Seg::Line(x0, y1)],
        )
    }
}

fn wght(variations: &GlyphVariations) -> f32 {
    variations
        .get(AxisTag::parse("wght").unwrap())
        .unwrap_or(400.0)
}

/// 합성 폰트의 glyph 모양(em 단위). `None`이면 잉크 없음.
///
/// - A1 사각형, A2 구멍 있는 고리(안쪽 contour 역방향), A3 공백, A4 막대(폭이 `wght`에 비례).
/// - B7 한쪽이 곡선인 D 모양, B9 outline 없는 color glyph.
fn glyph_shape(
    font: ExportFontFace,
    glyph: u32,
    variations: &GlyphVariations,
) -> Option<Vec<Contour>> {
    match (font, glyph) {
        (FONT_A, 1) => Some(vec![rect_contour(0.05, 0.0, 0.55, 0.7, false)]),
        (FONT_A, 2) => Some(vec![
            rect_contour(0.05, 0.0, 0.65, 0.7, false),
            rect_contour(0.2, 0.15, 0.5, 0.55, true),
        ]),
        (FONT_A, 4) => {
            let w = ((wght(variations) - 100.0) / 800.0).clamp(0.0, 1.0);
            Some(vec![rect_contour(0.05, 0.0, 0.15 + 0.25 * w, 0.7, false)])
        }
        (FONT_B, 7) => Some(vec![(
            (0.05, 0.0),
            vec![
                Seg::Line(0.3, 0.0),
                Seg::Cubic(0.65, 0.0, 0.65, 0.7, 0.3, 0.7),
                Seg::Line(0.05, 0.7),
            ],
        )]),
        _ => None,
    }
}

/// em 모양을 pen 원점 기준 px(y 아래 방향) outline으로 바꿉니다(host outline 규약).
fn outline_path(contours: &[Contour], size: f32) -> PathGeometry {
    let point = |x: f32, y: f32| fx::p(x * size, -y * size);
    let mut commands = Vec::new();
    for ((sx, sy), segs) in contours {
        commands.push(PathCommand::MoveTo(point(*sx, *sy)));
        for seg in segs {
            commands.push(match *seg {
                Seg::Line(x, y) => PathCommand::LineTo(point(x, y)),
                Seg::Cubic(x1, y1, x2, y2, x, y) => PathCommand::CubicTo {
                    c1: point(x1, y1),
                    c2: point(x2, y2),
                    to: point(x, y),
                },
            });
        }
        commands.push(PathCommand::Close);
    }
    PathGeometry::new(commands).unwrap()
}

/// host text source fixture: font revision별 layout, 조회한 outline key 기록.
struct Fonts {
    layouts: BTreeMap<([u8; 16], u64), Arc<TextGlyphLayout>>,
    requested: RefCell<Vec<GlyphOutlineKey>>,
}

impl ExportTextSource for Fonts {
    fn glyph_layout(
        &self,
        layout: TextLayoutKey,
        fonts: FontRevision,
    ) -> Option<Arc<TextGlyphLayout>> {
        self.layouts.get(&(layout.to_bytes(), fonts.0)).cloned()
    }

    fn glyph_outline(
        &self,
        key: &GlyphOutlineKey,
        _fonts: FontRevision,
    ) -> Result<GlyphOutline, GlyphOutlineError> {
        self.requested.borrow_mut().push(key.clone());
        if key.font == FONT_MISSING {
            return Err(GlyphOutlineError::FontUnavailable);
        }
        if key.font == FONT_B && key.glyph == 9 {
            return Ok(GlyphOutline::NoOutline);
        }
        Ok(match glyph_shape(key.font, key.glyph, &key.variations) {
            Some(contours) => GlyphOutline::Path(outline_path(&contours, key.size.px())),
            None => GlyphOutline::Empty,
        })
    }
}

fn px64(px: f32) -> i32 {
    (px * 64.0) as i32
}

fn glyph_run(
    font: ExportFontFace,
    size: f32,
    weight: Option<f32>,
    fill: Option<Color>,
    glyphs: &[(u32, f32, f32)],
) -> GlyphRun {
    GlyphRun {
        font,
        size: RunSize::from_px(size).unwrap(),
        variations: match weight {
            Some(w) => GlyphVariations::new([(AxisTag::parse("wght").unwrap(), w)]).unwrap(),
            None => GlyphVariations::none(),
        },
        fill,
        glyphs: glyphs
            .iter()
            .map(|&(glyph, x, y)| PlacedGlyph {
                glyph,
                x: px64(x),
                y: px64(y),
            })
            .collect(),
    }
}

fn red() -> Color {
    fx::srgb(1.0, 0.0, 0.0, 1.0)
}

fn green() -> Color {
    fx::srgb(0.0, 0.5, 0.0, 1.0)
}

fn blue() -> Color {
    Color::new(ColorSpace::DisplayP3, 0.0, 0.2, 0.9, 1.0).unwrap()
}

/// fixture layout(그리는 순서). baseline 30 px.
fn fixture_layout() -> TextGlyphLayout {
    TextGlyphLayout {
        runs: vec![
            glyph_run(
                FONT_A,
                20.0,
                None,
                Some(red()),
                &[(1, 4.0, 30.0), (2, 18.0, 30.0), (3, 33.0, 30.0)],
            ),
            glyph_run(FONT_A, 20.0, Some(700.0), Some(green()), &[(4, 36.0, 30.0)]),
            glyph_run(FONT_A, 20.0, Some(300.0), Some(green()), &[(4, 46.0, 30.0)]),
            glyph_run(
                FONT_B,
                16.0,
                None,
                Some(blue()),
                &[(7, 58.0, 30.0), (7, 72.0, 26.0)],
            ),
            glyph_run(FONT_A, 20.0, None, None, &[(1, 86.0, 30.0)]),
        ],
    }
}

fn fonts_with(entries: Vec<(u64, TextGlyphLayout)>) -> Fonts {
    Fonts {
        layouts: entries
            .into_iter()
            .map(|(revision, layout)| ((LAYOUT, revision), Arc::new(layout)))
            .collect(),
        requested: RefCell::new(Vec::new()),
    }
}

fn fonts() -> Fonts {
    fonts_with(vec![(1, fixture_layout())])
}

/// text node 하나(id 10)의 snapshot. `layered`면 반투명 Fill과 Center Stroke modifier가 있습니다.
fn text_snapshot(layered: bool) -> SceneSnapshot {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let mut node = NodeInput::new(fx::node_id(10), fx::translate(ORIGIN.0, ORIGIN.1));
    if layered {
        node.paint
            .fills
            .push(Paint::Solid(fx::srgb(0.0, 0.0, 0.0, 0.25)));
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(fx::srgb(0.0, 0.0, 1.0, 1.0)),
            width: 0.5,
        });
    }
    builder
        .build_text(
            &mut root,
            &TextInput {
                node,
                layout: TextLayoutKey::from_bytes(LAYOUT),
                size: Size2::new(100.0, 40.0).unwrap(),
            },
        )
        .unwrap();
    snapshot_of_roots(builder.finish(root))
}

fn snapshot_of_roots(roots: Vec<Arc<SceneNode>>) -> SceneSnapshot {
    SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap()
}

fn text_capture(snapshot: SceneSnapshot, fonts: u64) -> ExportCapture {
    ExportCapture::new(
        svg_request(),
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        ResourcePins {
            fonts: FontRevision(fonts),
            assets: AssetRevision(3),
        },
    )
    .unwrap()
}

fn write_text(capture: &ExportCapture, fonts: &Fonts) -> Result<SvgDocument, SvgExportError> {
    write_svg(
        &confirmed(capture),
        &assets(),
        fonts,
        &SvgWriteOptions::default(),
    )
}

/// 텍스트를 글꼴·텍스트 요소로 넣지 않았는지 확인합니다.
fn assert_no_text_or_font(svg: &str) {
    for forbidden in [
        "<text",
        "<tspan",
        "<font",
        "<glyph",
        "@font-face",
        "font-family",
        "data:font",
        "font/",
        "<style",
    ] {
        assert!(!svg.contains(forbidden), "{forbidden} 없음");
    }
}

fn update_or_compare(name: &str, actual: &str, golden: &str) {
    if std::env::var_os("STELLAR_EXPORT_UPDATE_GOLDEN").is_some() {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::write(path, actual).unwrap();
        return;
    }
    assert_eq!(actual, golden, "{name}");
}

#[test]
fn text_view_matches_golden_without_text_elements() {
    // 검증 기준(XX.3.2): SVG에 `<text` 문자열 0.
    let fonts = fonts();
    let document = write_text(&text_capture(text_snapshot(true), 1), &fonts).unwrap();
    update_or_compare("text_view_v1.svg", document.as_str(), TEXT_GOLDEN);
    let svg = document.as_str();
    validate(svg);
    assert_no_text_or_font(svg);
    assert_eq!(svg.matches("<text").count(), 0);
    assert_eq!(document.groups(), 1);
    // run 색 path 4개(색 없는 run 제외) + Fill layer 1 + Stroke layer 1.
    assert_eq!(svg.matches("<path ").count(), 6);
    assert_eq!(svg.matches("fill=\"rgb(100%,0%,0%)\"").count(), 1);
    assert_eq!(svg.matches("fill=\"rgb(0%,50%,0%)\"").count(), 2);
    assert_eq!(
        svg.matches("fill=\"color(display-p3 0 0.2 0.9)\"").count(),
        1
    );
    assert_eq!(svg.matches("fill-opacity=\"0.25\"").count(), 1);
    assert_eq!(svg.matches("stroke-width=\"0.5\"").count(), 1);
    // paint layer path는 색 없는 run(86 px의 사각형)도 포함합니다.
    let layer = svg
        .lines()
        .find(|line| line.contains("fill-opacity=\"0.25\""))
        .unwrap();
    assert!(layer.contains("M87 30 L97 30 L97 16 L87 16 Z"), "{layer}");
}

#[test]
fn text_outlines_render_like_the_glyph_shapes() {
    // 검증 기준(XX.3.2): 외형 golden 통과.
    let fonts = fonts();
    let capture = text_capture(text_snapshot(false), 1);
    let document = write_text(&capture, &fonts).unwrap();
    assert_no_text_or_font(document.as_str());
    let bounds = capture.bounds();
    assert_eq!(
        (
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            bounds.height()
        ),
        (10.0, 5.0, 100.0, 40.0)
    );
    let rendered = raster::render_svg(document.as_str(), bounds);
    let reference = raster::render_reference(&fixture_layout(), ORIGIN, bounds);
    assert_eq!(
        rendered.text(),
        reference.text(),
        "SVG 외형 = glyph 모양 reference"
    );
    update_or_compare(
        "text_view_v1.coverage.txt",
        &rendered.text(),
        COVERAGE_GOLDEN,
    );
    // 각 run 색이 실제로 칠해졌는지(색 없는 run은 보이지 않음).
    let counts = rendered.counts();
    for (fill, expected) in [
        ("rgb(100%,0%,0%)", true),
        ("rgb(0%,50%,0%)", true),
        ("color(display-p3 0 0.2 0.9)", true),
    ] {
        assert_eq!(
            counts.get(fill).copied().unwrap_or(0) > 0,
            expected,
            "{fill}"
        );
    }
    // 고리 glyph의 구멍(18 + 0.2em..0.5em, baseline 위 0.15em..0.55em)은 칠하지 않습니다.
    assert_eq!(rendered.at(10.0 + 25.0, 5.0 + 23.0), None, "구멍");
    assert_eq!(
        rendered.at(10.0 + 20.0, 5.0 + 23.0),
        Some("rgb(100%,0%,0%)")
    );
    // wght 700 막대(폭 6.75 px)가 wght 300 막대(폭 4.25 px)보다 넓습니다.
    assert_eq!(rendered.at(10.0 + 42.5, 5.0 + 25.0), Some("rgb(0%,50%,0%)"));
    assert_eq!(rendered.at(10.0 + 51.5, 5.0 + 25.0), None);
}

#[test]
fn outline_keys_carry_font_size_and_variations() {
    let fonts = fonts();
    write_text(&text_capture(text_snapshot(true), 1), &fonts).unwrap();
    let requested = fonts.requested.borrow();
    // 같은 key는 한 번만 조회합니다(cache). A1은 두 run에서 쓰지만 같은 instance입니다.
    let unique: BTreeSet<String> = requested.iter().map(|k| format!("{k:?}")).collect();
    assert_eq!(unique.len(), requested.len(), "중복 조회 없음");
    let summary: BTreeSet<(u8, u32, u32, Option<u32>)> = requested
        .iter()
        .map(|k| {
            (
                k.font.file_sha256()[0],
                k.glyph,
                k.size.sixty_fourths(),
                k.variations
                    .get(AxisTag::parse("wght").unwrap())
                    .map(|w| w as u32),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (0xA1, 1, 1280, None),
            (0xA1, 2, 1280, None),
            (0xA1, 3, 1280, None),
            (0xA1, 4, 1280, Some(300)),
            (0xA1, 4, 1280, Some(700)),
            (0xB2, 7, 1024, None),
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn text_reads_the_pinned_font_revision() {
    let mut moved = fixture_layout();
    for run in &mut moved.runs {
        for glyph in &mut run.glyphs {
            glyph.x += 64;
        }
    }
    let fonts = fonts_with(vec![(1, fixture_layout()), (2, moved)]);
    let at_one = write_text(&text_capture(text_snapshot(true), 1), &fonts).unwrap();
    assert_eq!(at_one.as_str(), TEXT_GOLDEN, "고정 revision 1의 layout");
    let at_two = write_text(&text_capture(text_snapshot(true), 2), &fonts).unwrap();
    assert_ne!(at_two.as_bytes(), at_one.as_bytes());
    // 고정 revision에 layout이 없으면 다른 revision으로 대체하지 않습니다.
    let only_two = fonts_with(vec![(2, fixture_layout())]);
    assert_eq!(
        write_text(&text_capture(text_snapshot(true), 1), &only_two).unwrap_err(),
        SvgExportError::MissingTextLayout {
            node: fx::node_id(10),
            layout: TextLayoutKey::from_bytes(LAYOUT)
        }
    );
}

#[test]
fn same_text_snapshot_is_byte_identical() {
    let fonts = fonts();
    let first = write_text(&text_capture(text_snapshot(true), 1), &fonts).unwrap();
    let again = write_text(&text_capture(text_snapshot(true), 1), &fonts).unwrap();
    assert_eq!(first.as_bytes(), again.as_bytes());
    let fresh = write_text(&text_capture(text_snapshot(true), 1), &self::fonts()).unwrap();
    assert_eq!(first.as_bytes(), fresh.as_bytes(), "cache 상태와 무관");
}

#[test]
fn unavailable_fonts_and_color_glyphs_are_errors() {
    let mut missing = fixture_layout();
    missing.runs[3].font = FONT_MISSING;
    let error = write_text(
        &text_capture(text_snapshot(false), 1),
        &fonts_with(vec![(1, missing)]),
    )
    .unwrap_err();
    assert_eq!(
        error,
        SvgExportError::Text {
            node: fx::node_id(10),
            error: TextOutlineError::Outline {
                font: FONT_MISSING,
                glyph: 7,
                error: GlyphOutlineError::FontUnavailable
            }
        }
    );
    assert!(error.to_string().contains("폰트"));
    let mut color = fixture_layout();
    color.runs[3].glyphs.push(PlacedGlyph {
        glyph: 9,
        x: 0,
        y: 0,
    });
    assert_eq!(
        write_text(
            &text_capture(text_snapshot(false), 1),
            &fonts_with(vec![(1, color)])
        )
        .unwrap_err(),
        SvgExportError::Text {
            node: fx::node_id(10),
            error: TextOutlineError::NoOutline {
                font: FONT_B,
                glyph: 9
            }
        }
    );
    // text가 없는 source로는 Text를 쓸 수 없습니다.
    assert!(matches!(
        write(&text_capture(text_snapshot(false), 1)),
        Err(SvgExportError::MissingTextLayout { .. })
    ));
}

/// `snapshot` 첫 root Text의 layout 상자 원점을 `origin`으로 옮긴 snapshot(scene builder는 항상 원점 0).
fn with_text_box_origin(snapshot: &SceneSnapshot, origin: (f32, f32)) -> SceneSnapshot {
    let root = snapshot.root(0).unwrap();
    let mut spec = root.spec();
    let SceneContent::Text(text) = &mut spec.content else {
        panic!("Text root");
    };
    text.bounds = Rect::new(fx::p(origin.0, origin.1), text.bounds.size());
    snapshot_of_roots(vec![Arc::new(SceneNode::leaf(spec))])
}

#[test]
fn glyphs_start_at_the_layout_box_origin() {
    // TextPrimitive.bounds는 node local 좌표의 layout 상자이고 glyph pen 원점은 그 좌상단 기준입니다.
    // 상자 원점이 0이 아니면(codec·직접 생성) glyph를 그만큼 옮깁니다(Image가 bounds를 그대로 쓰는 것과 같은 규약).
    let fonts = fonts();
    let moved = with_text_box_origin(&text_snapshot(true), (5.0, 3.0));
    let document = write_text(&text_capture(moved, 1), &fonts).unwrap();
    let layer = document
        .as_str()
        .lines()
        .find(|line| line.contains("fill-opacity=\"0.25\""))
        .unwrap();
    // 원점 0일 때 "M87 30 L97 30 L97 16 L87 16 Z"인 색 없는 run glyph가 (5, 3)만큼 옮겨집니다.
    assert!(layer.contains("M92 33 L102 33 L102 19 L92 19 Z"), "{layer}");
    // 원점 0인 상자는 기존 golden과 byte 단위로 같습니다.
    let zero = with_text_box_origin(&text_snapshot(true), (0.0, 0.0));
    assert_eq!(
        write_text(&text_capture(zero, 1), &fonts).unwrap().as_str(),
        TEXT_GOLDEN
    );
}

#[test]
fn text_output_respects_the_size_limit() {
    let fonts = fonts();
    let capture = text_capture(text_snapshot(true), 1);
    let full = write_text(&capture, &fonts).unwrap();
    for limit in [64, 400, full.as_bytes().len() - 1] {
        assert_eq!(
            write_svg(
                &confirmed(&capture),
                &assets(),
                &fonts,
                &SvgWriteOptions {
                    max_output_bytes: limit
                }
            ),
            Err(SvgExportError::OutputTooLarge { limit })
        );
    }
    let exact = SvgWriteOptions {
        max_output_bytes: full.as_bytes().len(),
    };
    assert_eq!(
        write_svg(&confirmed(&capture), &assets(), &fonts, &exact).unwrap(),
        full
    );
}

#[test]
fn huge_text_stops_at_the_memory_cap_before_the_output_limit() {
    // 기본 출력 상한(256 MiB)의 command 예산(약 1.3억 개)보다 메모리 상한(2²¹개)이 먼저 걸립니다.
    let glyphs: Vec<(u32, f32, f32)> = (0..MAX_TEXT_OUTLINE_COMMANDS / 5 + 1)
        .map(|i| (1, (i % 100) as f32, 30.0))
        .collect();
    let layout = TextGlyphLayout {
        runs: vec![glyph_run(FONT_A, 20.0, None, None, &glyphs)],
    };
    assert_eq!(
        write_text(
            &text_capture(text_snapshot(false), 1),
            &fonts_with(vec![(1, layout)])
        ),
        Err(SvgExportError::Text {
            node: fx::node_id(10),
            error: TextOutlineError::TooManyCommands {
                limit: MAX_TEXT_OUTLINE_COMMANDS
            }
        })
    );
}

/// 테스트 전용 point-sampling rasterizer. pixel 중심 한 점에서 non-zero winding으로 덮는 마지막 채우기의 색을
/// 기록합니다(투명도·stroke는 보지 않음).
mod raster {
    use std::collections::BTreeMap;

    use stellar_scene::bounds::Aabb;

    use super::{glyph_shape, Seg};
    use crate::text::TextGlyphLayout;

    /// 3차 곡선 분할 수.
    const CUBIC_STEPS: usize = 16;

    type Polygon = Vec<(f64, f64)>;

    pub(super) struct Coverage {
        width: usize,
        height: usize,
        origin: (f32, f32),
        cells: Vec<Option<String>>,
    }

    impl Coverage {
        fn new(bounds: Aabb) -> Coverage {
            let width = bounds.width().round() as usize;
            let height = bounds.height().round() as usize;
            Coverage {
                width,
                height,
                origin: (bounds.min_x(), bounds.min_y()),
                cells: vec![None; width * height],
            }
        }

        fn paint(&mut self, polygons: &[Polygon], fill: &str) {
            for row in 0..self.height {
                for col in 0..self.width {
                    let x = f64::from(self.origin.0) + col as f64 + 0.5;
                    let y = f64::from(self.origin.1) + row as f64 + 0.5;
                    if winding(polygons, x, y) != 0 {
                        self.cells[row * self.width + col] = Some(fill.to_string());
                    }
                }
            }
        }

        /// world 좌표 점이 속한 pixel의 색.
        pub(super) fn at(&self, x: f32, y: f32) -> Option<&str> {
            let col = (x - self.origin.0).floor() as usize;
            let row = (y - self.origin.1).floor() as usize;
            self.cells[row * self.width + col].as_deref()
        }

        /// 색별 pixel 수.
        pub(super) fn counts(&self) -> BTreeMap<String, usize> {
            let mut out = BTreeMap::new();
            for fill in self.cells.iter().flatten() {
                *out.entry(fill.clone()).or_default() += 1;
            }
            out
        }

        /// 범례(색 → 문자, 처음 나온 순서)와 pixel 문자 격자.
        pub(super) fn text(&self) -> String {
            let mut legend: Vec<String> = Vec::new();
            for fill in self.cells.iter().flatten() {
                if !legend.contains(fill) {
                    legend.push(fill.clone());
                }
            }
            let mut out = String::new();
            for (index, fill) in legend.iter().enumerate() {
                out.push_str(&format!("# {} = {fill}\n", (b'a' + index as u8) as char));
            }
            for row in 0..self.height {
                for col in 0..self.width {
                    out.push(match &self.cells[row * self.width + col] {
                        None => '.',
                        Some(fill) => {
                            let index = legend.iter().position(|f| f == fill).unwrap();
                            (b'a' + index as u8) as char
                        }
                    });
                }
                out.push('\n');
            }
            out
        }
    }

    fn winding(polygons: &[Polygon], x: f64, y: f64) -> i32 {
        let mut total = 0;
        for polygon in polygons {
            for i in 0..polygon.len() {
                let (x0, y0) = polygon[i];
                let (x1, y1) = polygon[(i + 1) % polygon.len()];
                if (y0 <= y) != (y1 <= y) {
                    let t = (y - y0) / (y1 - y0);
                    if x0 + t * (x1 - x0) > x {
                        total += if y1 > y0 { 1 } else { -1 };
                    }
                }
            }
        }
        total
    }

    fn cubic(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64), out: &mut Polygon) {
        for step in 1..=CUBIC_STEPS {
            let t = step as f64 / CUBIC_STEPS as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push((
                a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1,
            ));
        }
    }

    fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
        let key = format!(" {name}=\"");
        let start = tag.find(&key)? + key.len();
        let end = tag[start..].find('"')? + start;
        Some(&tag[start..end])
    }

    /// writer가 쓰는 path 문법(M, L, C, Z, 절대 좌표, 공백 구분)만 읽습니다.
    fn parse_path(d: &str, matrix: [f64; 6]) -> Vec<Polygon> {
        let map = |x: f64, y: f64| {
            let [a, b, c, dd, e, f] = matrix;
            (a * x + c * y + e, b * x + dd * y + f)
        };
        let mut tokens: Vec<String> = Vec::new();
        for part in d.split_whitespace() {
            let (head, rest) = part.split_at(1);
            if head.chars().all(|c| c.is_ascii_alphabetic()) {
                tokens.push(head.to_string());
                if !rest.is_empty() {
                    tokens.push(rest.to_string());
                }
            } else {
                tokens.push(part.to_string());
            }
        }
        let mut polygons = Vec::new();
        let mut current: Polygon = Vec::new();
        let mut i = 0;
        let number = |i: &mut usize| -> f64 {
            *i += 1;
            tokens[*i].parse().unwrap()
        };
        while i < tokens.len() {
            match tokens[i].as_str() {
                "M" => {
                    if current.len() > 1 {
                        polygons.push(std::mem::take(&mut current));
                    }
                    current.clear();
                    let (x, y) = (number(&mut i), number(&mut i));
                    current.push(map(x, y));
                }
                "L" => {
                    let (x, y) = (number(&mut i), number(&mut i));
                    current.push(map(x, y));
                }
                "C" => {
                    let p0 = *current.last().unwrap();
                    let c1 = (number(&mut i), number(&mut i));
                    let c2 = (number(&mut i), number(&mut i));
                    let to = (number(&mut i), number(&mut i));
                    cubic(
                        p0,
                        map(c1.0, c1.1),
                        map(c2.0, c2.1),
                        map(to.0, to.1),
                        &mut current,
                    );
                }
                "Z" => {
                    if current.len() > 1 {
                        polygons.push(std::mem::take(&mut current));
                    }
                }
                other => panic!("지원하지 않는 path command {other}"),
            }
            i += 1;
        }
        if current.len() > 1 {
            polygons.push(current);
        }
        polygons
    }

    /// SVG 문서의 채우기 `<path>`를 문서 순서로 칠합니다.
    pub(super) fn render_svg(svg: &str, bounds: Aabb) -> Coverage {
        let mut coverage = Coverage::new(bounds);
        for piece in svg.split("<path ").skip(1) {
            let tag = &piece[..piece.find("/>").unwrap()];
            let tag = format!(" {tag}");
            let fill = attribute(&tag, "fill").unwrap();
            if fill == "none" {
                continue;
            }
            assert!(attribute(&tag, "fill-rule").is_none(), "text는 non-zero");
            let matrix = match attribute(&tag, "transform") {
                Some(value) => {
                    let inner = value
                        .strip_prefix("matrix(")
                        .and_then(|v| v.strip_suffix(')'))
                        .unwrap();
                    let v: Vec<f64> = inner.split(' ').map(|n| n.parse().unwrap()).collect();
                    [v[0], v[1], v[2], v[3], v[4], v[5]]
                }
                None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            };
            let polygons = parse_path(attribute(&tag, "d").unwrap(), matrix);
            coverage.paint(&polygons, fill);
        }
        coverage
    }

    fn color_name(color: stellar_core_types::color::Color) -> String {
        crate::svg::format::color_value(color).unwrap().paint
    }

    /// writer를 거치지 않고 glyph 모양 정의에서 직접 칠한 reference.
    pub(super) fn render_reference(
        layout: &TextGlyphLayout,
        origin: (f32, f32),
        bounds: Aabb,
    ) -> Coverage {
        let mut coverage = Coverage::new(bounds);
        for run in &layout.runs {
            let Some(fill) = run.fill else { continue };
            let size = f64::from(run.size.px());
            let mut polygons = Vec::new();
            for placed in &run.glyphs {
                let Some(contours) = glyph_shape(run.font, placed.glyph, &run.variations) else {
                    continue;
                };
                let pen = (
                    f64::from(origin.0) + f64::from(placed.x) / 64.0,
                    f64::from(origin.1) + f64::from(placed.y) / 64.0,
                );
                let at =
                    |x: f32, y: f32| (pen.0 + f64::from(x) * size, pen.1 - f64::from(y) * size);
                for ((sx, sy), segs) in contours {
                    let mut polygon = vec![at(sx, sy)];
                    for seg in segs {
                        match seg {
                            Seg::Line(x, y) => polygon.push(at(x, y)),
                            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                                let p0 = *polygon.last().unwrap();
                                cubic(p0, at(x1, y1), at(x2, y2), at(x, y), &mut polygon);
                            }
                        }
                    }
                    polygons.push(polygon);
                }
            }
            coverage.paint(&polygons, &color_name(fill));
        }
        coverage
    }
}
