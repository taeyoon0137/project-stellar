//! Text glyph run → outline path 변환 (XX.3.2).
//!
//! 아키텍처 §34 SVG 정책대로 모든 텍스트를 Path로 씁니다. `<text>`·`<tspan>` 요소, font 파일·`@font-face`,
//! SVG font(`<font>`·`<glyph>`)는 만들지 않습니다.
//!
//! - glyph마다 outline([`GlyphOutline::Path`], pen 원점 기준 px)을 pen 원점(1/64 px, layout 상자 좌상단 기준)과
//!   layout 상자 원점(`TextPrimitive::bounds`의 원점, node local)만큼 옮겨 node local 좌표 path로 만듭니다.
//!   크기·variation은 outline key에 이미 들어 있으므로 다시 scale하지 않습니다.
//! - 단 key 크기 상한을 넘는 run은 상한 크기 outline을
//!   [`RunSize::outline_scale`](crate::text::RunSize::outline_scale)배 합니다([`crate::text`] "큰 글자").
//!   상한 이하 run의 배율은 정확히 1이라 결과가 바뀌지 않습니다.
//! - 모든 run의 path를 순서대로 이은 [`TextOutlines::combined`] 하나만 만듭니다. 이것이 Text content의 칠하기
//!   기하이고, paint layer(Fill·Stroke modifier)는 이 기하를 run 색 **위**에 칠합니다(scene builder paint 순서 규칙).
//! - run([`RunOutline`])은 combined 안 command 범위만 가집니다(복제 없음). run 고유 색(`fill`)이 있으면 그 범위를
//!   그 색으로 칠합니다([`TextOutlines::run_geometry`]는 쓸 때만 잠깐 만드는 복사본).
//! - 메모리: command 목록은 Text 하나에 [`MAX_TEXT_OUTLINE_COMMANDS`]개까지입니다(호출자가 넘기는 상한과 별개인
//!   고정 상한). command 하나는 32 byte 이하라 목록은 64 MiB 이하이고, geometry로 옮기는 동안 복사본까지 최대 그
//!   2배입니다.
//! - 채우기 규칙은 non-zero입니다(OpenType outline). 서로 겹치는 glyph도 non-zero 합집합이 됩니다.
//! - 잉크 없는 glyph([`GlyphOutline::Empty`])는 건너뜁니다. outline이 없는 glyph([`GlyphOutline::NoOutline`],
//!   bitmap emoji 등)는 벡터로 쓸 수 없어 [`TextOutlineError::NoOutline`]으로 멈춥니다(조용히 빼지 않음).
//!
//! 같은 key의 outline 조회는 호출자가 cache합니다([`text_outlines`]의 `outline` 인자).

use std::fmt;
use std::ops::Range;

use stellar_core_types::color::Color;
use stellar_core_types::value::Vec2;
use stellar_scene::geometry::{PathCommand, PathGeometry};

use crate::text::{
    ExportFontFace, GlyphOutline, GlyphOutlineError, GlyphOutlineKey, TextGlyphLayout,
};

/// Text 하나의 outline path command 상한(메모리 기준 고정 상수, 2²¹).
///
/// 출력 크기 상한과 별개입니다. 출력 예산이 남아 있어도 command 목록이 이보다 커지면 멈춥니다.
pub const MAX_TEXT_OUTLINE_COMMANDS: usize = 1 << 21;

/// run 하나의 outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutline {
    /// run 고유 색.
    pub fill: Option<Color>,
    /// [`TextOutlines::combined`] 안 이 run의 command 범위. 빈 범위거나 MoveTo로 시작합니다.
    pub commands: Range<usize>,
}

/// text layout 전체의 outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextOutlines {
    /// run별 outline(그리는 순서). 잉크가 없는 run도 순서를 유지하려고 빈 범위로 둡니다.
    pub runs: Vec<RunOutline>,
    /// 모든 run의 outline(node local 좌표, paint layer 기하).
    pub combined: PathGeometry,
}

impl TextOutlines {
    /// run의 command(combined의 부분).
    pub fn run_commands(&self, run: &RunOutline) -> &[PathCommand] {
        &self.combined.commands()[run.commands.clone()]
    }

    /// run의 geometry(복사본). 쓸 때만 만들고 바로 버립니다.
    pub fn run_geometry(&self, run: &RunOutline) -> Result<PathGeometry, TextOutlineError> {
        geometry(self.run_commands(run).to_vec())
    }
}

/// outline 변환 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextOutlineError {
    /// glyph outline 조회 실패.
    Outline {
        /// 폰트 face.
        font: ExportFontFace,
        /// glyph 번호.
        glyph: u32,
        /// 원인.
        error: GlyphOutlineError,
    },
    /// outline이 없는 glyph(bitmap·color 전용)입니다.
    NoOutline {
        /// 폰트 face.
        font: ExportFontFace,
        /// glyph 번호.
        glyph: u32,
    },
    /// 좌표 계산 결과가 유한하지 않습니다.
    NonFinite,
    /// 이어 붙인 path가 subpath 규칙을 어깁니다(입력 outline 불변식 위반).
    InvalidPath,
    /// path command가 상한을 넘습니다.
    TooManyCommands {
        /// 상한.
        limit: usize,
    },
}

impl fmt::Display for TextOutlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextOutlineError::Outline { font, glyph, error } => {
                write!(f, "폰트 {font}의 glyph {glyph}: {error}")
            }
            TextOutlineError::NoOutline { font, glyph } => write!(
                f,
                "폰트 {font}의 glyph {glyph}는 outline이 없어 벡터로 쓸 수 없습니다"
            ),
            TextOutlineError::NonFinite => {
                f.write_str("glyph outline 좌표 계산 결과가 유한하지 않습니다")
            }
            TextOutlineError::InvalidPath => {
                f.write_str("glyph outline path가 subpath 규칙을 어깁니다")
            }
            TextOutlineError::TooManyCommands { limit } => {
                write!(f, "text outline path command가 {limit}개 상한을 넘습니다")
            }
        }
    }
}

impl std::error::Error for TextOutlineError {}

/// 1/64 px 정수를 px로 옮깁니다(|값| < 2²⁴이면 정확).
fn px(subpixels: i32) -> f32 {
    subpixels as f32 / 64.0
}

/// glyph 배치: outline 배율과 pen 원점 이동.
#[derive(Clone, Copy)]
struct Placement {
    scale: f32,
    dx: f32,
    dy: f32,
}

impl Placement {
    /// `p × scale + (dx, dy)`. `scale`이 1이면 곱셈이 정확해 이동만 한 값과 같습니다.
    fn apply(self, p: Vec2) -> Result<Vec2, TextOutlineError> {
        Vec2::new(p.x() * self.scale + self.dx, p.y() * self.scale + self.dy)
            .map_err(|_| TextOutlineError::NonFinite)
    }
}

/// `outline` command를 `placement`로 옮겨 `out`에 덧붙입니다.
fn push_placed(
    out: &mut Vec<PathCommand>,
    outline: &PathGeometry,
    placement: Placement,
    limit: usize,
) -> Result<(), TextOutlineError> {
    if out.len().saturating_add(outline.commands().len()) > limit {
        return Err(TextOutlineError::TooManyCommands { limit });
    }
    for command in outline.commands() {
        out.push(match *command {
            PathCommand::MoveTo(p) => PathCommand::MoveTo(placement.apply(p)?),
            PathCommand::LineTo(p) => PathCommand::LineTo(placement.apply(p)?),
            PathCommand::CubicTo { c1, c2, to } => PathCommand::CubicTo {
                c1: placement.apply(c1)?,
                c2: placement.apply(c2)?,
                to: placement.apply(to)?,
            },
            PathCommand::Close => PathCommand::Close,
        });
    }
    Ok(())
}

/// 이어 붙인 command를 geometry로 검증합니다. 각 outline이 이미 검증된 geometry(빈 것이거나 MoveTo로
/// 시작)라 실패하지 않지만, 실패를 조용히 빈 path로 바꾸지 않고 오류로 돌려줍니다.
fn geometry(commands: Vec<PathCommand>) -> Result<PathGeometry, TextOutlineError> {
    PathGeometry::new(commands).map_err(|_| TextOutlineError::InvalidPath)
}

/// glyph layout을 outline path로 바꿉니다.
///
/// `origin`은 layout 상자 좌상단(node local 좌표, `TextPrimitive::bounds`의 원점)입니다. `outline`은 key의
/// outline을 돌려줍니다(호출자 cache). `max_commands`는 combined path의 command 상한이며
/// [`MAX_TEXT_OUTLINE_COMMANDS`]보다 크면 그 값으로 줄입니다.
pub fn text_outlines(
    layout: &TextGlyphLayout,
    origin: Vec2,
    outline: &mut dyn FnMut(&GlyphOutlineKey) -> Result<GlyphOutline, GlyphOutlineError>,
    max_commands: usize,
) -> Result<TextOutlines, TextOutlineError> {
    let max_commands = max_commands.min(MAX_TEXT_OUTLINE_COMMANDS);
    let mut commands: Vec<PathCommand> = Vec::new();
    let mut runs = Vec::with_capacity(layout.runs.len());
    for run in &layout.runs {
        let scale = run.size.outline_scale();
        let start = commands.len();
        for placed in &run.glyphs {
            let key = run.key(placed.glyph);
            let glyph = outline(&key).map_err(|error| TextOutlineError::Outline {
                font: run.font,
                glyph: placed.glyph,
                error,
            })?;
            match glyph {
                GlyphOutline::Empty => {}
                GlyphOutline::Path(path) => {
                    let placement = Placement {
                        scale,
                        dx: origin.x() + px(placed.x),
                        dy: origin.y() + px(placed.y),
                    };
                    push_placed(&mut commands, &path, placement, max_commands)?;
                }
                GlyphOutline::NoOutline => {
                    return Err(TextOutlineError::NoOutline {
                        font: run.font,
                        glyph: placed.glyph,
                    });
                }
            }
        }
        runs.push(RunOutline {
            fill: run.fill,
            commands: start..commands.len(),
        });
    }
    Ok(TextOutlines {
        runs,
        combined: geometry(commands)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::test_fixtures as fx;
    use crate::text::{
        FontFileHash, GlyphRun, GlyphVariations, OpenTypeTag, PlacedGlyph, RunSize, MAX_GLYPH_PX,
        MAX_RUN_PX,
    };

    fn square(size: f32) -> PathGeometry {
        PathGeometry::new(vec![
            PathCommand::MoveTo(fx::p(0.0, -size)),
            PathCommand::LineTo(fx::p(size, -size)),
            PathCommand::LineTo(fx::p(size, 0.0)),
            PathCommand::LineTo(fx::p(0.0, 0.0)),
            PathCommand::Close,
        ])
        .unwrap()
    }

    fn face(n: u8) -> ExportFontFace {
        ExportFontFace::new(FontFileHash::from_sha256_digest([n; 32]), 0)
    }

    fn run(font: u8, size: f32, fill: Option<Color>, glyphs: &[(u32, i32, i32)]) -> GlyphRun {
        GlyphRun {
            font: face(font),
            size: RunSize::from_px(size).unwrap(),
            variations: GlyphVariations::none(),
            fill,
            glyphs: glyphs
                .iter()
                .map(|&(glyph, x, y)| PlacedGlyph { glyph, x, y })
                .collect(),
        }
    }

    /// glyph 1 = 크기 비례 사각형, glyph 2 = 공백, glyph 3 = outline 없음, font 9 = 없음.
    fn source(key: &GlyphOutlineKey) -> Result<GlyphOutline, GlyphOutlineError> {
        if key.font == face(9) {
            return Err(GlyphOutlineError::FontUnavailable);
        }
        Ok(match key.glyph {
            1 => GlyphOutline::Path(square(key.size.px() / 2.0)),
            2 => GlyphOutline::Empty,
            _ => GlyphOutline::NoOutline,
        })
    }

    fn run_geometry(outlines: &TextOutlines, index: usize) -> PathGeometry {
        outlines.run_geometry(&outlines.runs[index]).unwrap()
    }

    fn points(geometry: &PathGeometry) -> Vec<(f32, f32)> {
        geometry
            .commands()
            .iter()
            .filter_map(|c| match c {
                PathCommand::MoveTo(p) | PathCommand::LineTo(p) => Some((p.x(), p.y())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn glyphs_move_to_their_pen_origin() {
        let red = fx::srgb(1.0, 0.0, 0.0, 1.0);
        let layout = TextGlyphLayout {
            runs: vec![
                // (glyph, x, y) 1/64 px: (0, 10 px), (12.5 px, 10 px), 공백.
                run(
                    1,
                    8.0,
                    Some(red),
                    &[(1, 0, 640), (1, 800, 640), (2, 1600, 640)],
                ),
                run(1, 4.0, None, &[(1, -64, 32)]),
            ],
        };
        let outlines = text_outlines(&layout, Vec2::ZERO, &mut source, usize::MAX).unwrap();
        assert_eq!(outlines.runs.len(), 2);
        assert_eq!(outlines.runs[0].fill, Some(red));
        assert_eq!(outlines.runs[1].fill, None);
        assert_eq!(
            points(&run_geometry(&outlines, 0)),
            vec![
                (0.0, 6.0),
                (4.0, 6.0),
                (4.0, 10.0),
                (0.0, 10.0),
                (12.5, 6.0),
                (16.5, 6.0),
                (16.5, 10.0),
                (12.5, 10.0),
            ]
        );
        assert_eq!(
            points(&run_geometry(&outlines, 1)),
            vec![(-1.0, -1.5), (1.0, -1.5), (1.0, 0.5), (-1.0, 0.5)]
        );
        // combined = run path를 순서대로 이은 것이고, run은 그 안의 범위입니다(복제 없음).
        assert_eq!(outlines.runs[0].commands, 0..10);
        assert_eq!(outlines.runs[1].commands, 10..15);
        let mut joined = outlines.run_commands(&outlines.runs[0]).to_vec();
        joined.extend_from_slice(outlines.run_commands(&outlines.runs[1]));
        assert_eq!(outlines.combined.commands(), joined.as_slice());
        // layout 상자 원점이 0이 아니면 모든 glyph가 그만큼 옮겨집니다.
        let moved = text_outlines(&layout, fx::p(2.0, -3.0), &mut source, usize::MAX).unwrap();
        assert_eq!(
            points(&run_geometry(&moved, 1)),
            vec![(1.0, -4.5), (3.0, -4.5), (3.0, -2.5), (1.0, -2.5)]
        );
    }

    #[test]
    fn outline_failures_are_typed() {
        let missing = TextGlyphLayout {
            runs: vec![run(9, 8.0, None, &[(1, 0, 0)])],
        };
        assert_eq!(
            text_outlines(&missing, Vec2::ZERO, &mut source, usize::MAX),
            Err(TextOutlineError::Outline {
                font: face(9),
                glyph: 1,
                error: GlyphOutlineError::FontUnavailable
            })
        );
        let color = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &[(1, 0, 0), (3, 64, 0)])],
        };
        let error = text_outlines(&color, Vec2::ZERO, &mut source, usize::MAX).unwrap_err();
        assert_eq!(
            error,
            TextOutlineError::NoOutline {
                font: face(1),
                glyph: 3
            }
        );
        assert!(error.to_string().contains("outline"));
        let many = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &[(1, 0, 0), (1, 64, 0)])],
        };
        assert_eq!(
            text_outlines(&many, Vec2::ZERO, &mut source, 9),
            Err(TextOutlineError::TooManyCommands { limit: 9 })
        );
        assert!(text_outlines(&many, Vec2::ZERO, &mut source, 10).is_ok());
        let far = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &[(1, i32::MAX, 0)])],
        };
        assert!(text_outlines(&far, Vec2::ZERO, &mut source, usize::MAX).is_ok());
        for error in [
            TextOutlineError::NonFinite,
            TextOutlineError::InvalidPath,
            TextOutlineError::TooManyCommands { limit: 1 },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn runs_above_the_key_limit_scale_the_capped_outline() {
        // key 상한(4096 px)을 넘는 run도 text engine 상한(10,000 px)까지 outline으로 씁니다.
        let mut keys: Vec<GlyphOutlineKey> = Vec::new();
        let mut recording = |key: &GlyphOutlineKey| {
            keys.push(key.clone());
            source(key)
        };
        let layout = TextGlyphLayout {
            runs: vec![
                run(1, 8192.0, None, &[(1, 640, 64_000)]),
                run(1, MAX_RUN_PX, None, &[(1, 0, 0)]),
            ],
        };
        let outlines = text_outlines(&layout, Vec2::ZERO, &mut recording, usize::MAX).unwrap();
        // 두 run 모두 상한 크기 key 하나로 조회합니다.
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| k.size.px() == MAX_GLYPH_PX));
        // 8192 px: 상한 outline(한 변 2048)의 2배 = 같은 glyph를 8192 px로 직접 만든 사각형(한 변 4096).
        assert_eq!(
            points(&run_geometry(&outlines, 0)),
            vec![
                (10.0, -3096.0),
                (4106.0, -3096.0),
                (4106.0, 1000.0),
                (10.0, 1000.0)
            ]
        );
        // 10,000 px: 배율 2.44140625(정확), 한 변 5000.
        assert_eq!(
            points(&run_geometry(&outlines, 1)),
            vec![(0.0, -5000.0), (5000.0, -5000.0), (5000.0, 0.0), (0.0, 0.0)]
        );
    }

    #[test]
    fn command_memory_is_capped_independently_of_the_caller_limit() {
        // command 하나는 32 byte 이하라 고정 상한의 목록은 64 MiB 이하입니다(모듈 문서 "메모리").
        assert!(std::mem::size_of::<PathCommand>() <= 32);
        const { assert!(MAX_TEXT_OUTLINE_COMMANDS * 32 <= 64 * 1024 * 1024) };
        // 호출자 상한이 고정 상한보다 커도(출력 예산이 넉넉해도) 고정 상한에서 멈춥니다.
        let glyphs: Vec<(u32, i32, i32)> = (0..MAX_TEXT_OUTLINE_COMMANDS / 5 + 1)
            .map(|i| (1, (i % 1000) as i32 * 64, 0))
            .collect();
        let huge = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &glyphs)],
        };
        assert_eq!(
            text_outlines(&huge, Vec2::ZERO, &mut source, usize::MAX),
            Err(TextOutlineError::TooManyCommands {
                limit: MAX_TEXT_OUTLINE_COMMANDS
            })
        );
        // 호출자 상한이 더 작으면 그 상한입니다.
        assert_eq!(
            text_outlines(&huge, Vec2::ZERO, &mut source, 100),
            Err(TextOutlineError::TooManyCommands { limit: 100 })
        );
    }

    #[test]
    fn keys_use_the_run_instance() {
        let wght = OpenTypeTag::parse("wght").unwrap();
        let mut first = run(1, 12.0, None, &[(1, 0, 0), (1, 64, 0)]);
        first.variations = GlyphVariations::new([(wght, 700.0)]).unwrap();
        let second = run(1, 12.0, None, &[(1, 0, 0)]);
        let layout = TextGlyphLayout {
            runs: vec![first, second],
        };
        let mut seen: HashMap<GlyphOutlineKey, u32> = HashMap::new();
        let mut counting = |key: &GlyphOutlineKey| {
            *seen.entry(key.clone()).or_default() += 1;
            source(key)
        };
        text_outlines(&layout, Vec2::ZERO, &mut counting, usize::MAX).unwrap();
        assert_eq!(seen.len(), 2, "variation이 다르면 다른 key");
        let heavy = seen
            .keys()
            .find(|k| k.variations.get(wght) == Some(700.0))
            .unwrap();
        assert_eq!(heavy.size.sixty_fourths(), 768);
        assert_eq!(seen[heavy], 2);
    }
}
