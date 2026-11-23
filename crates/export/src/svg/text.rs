//! Text glyph run → outline path 변환 (XX.3.2).
//!
//! 아키텍처 §34 SVG 정책대로 모든 텍스트를 Path로 씁니다. `<text>`·`<tspan>` 요소, font 파일·`@font-face`,
//! SVG font(`<font>`·`<glyph>`)는 만들지 않습니다.
//!
//! - glyph마다 outline([`GlyphOutline::Path`], pen 원점 기준 px)을 pen 원점(1/64 px)만큼 옮겨 text local 좌표
//!   path로 만듭니다. 크기·variation은 outline key에 이미 들어 있으므로 다시 scale하지 않습니다.
//! - run마다 한 path([`RunOutline`])를 만듭니다. run 고유 색(`fill`)이 있으면 그 색으로 칠합니다.
//! - 모든 run의 path를 이은 [`TextOutlines::combined`]가 Text content의 칠하기 기하입니다. paint layer
//!   (Fill·Stroke modifier)는 이 기하를 run 색 **위**에 칠합니다(scene builder paint 순서 규칙).
//! - 채우기 규칙은 non-zero입니다(OpenType outline). 서로 겹치는 glyph도 non-zero 합집합이 됩니다.
//! - 잉크 없는 glyph([`GlyphOutline::Empty`])는 건너뜁니다. outline이 없는 glyph([`GlyphOutline::NoOutline`],
//!   bitmap emoji 등)는 벡터로 쓸 수 없어 [`TextOutlineError::NoOutline`]으로 멈춥니다(조용히 빼지 않음).
//!
//! 같은 key의 outline 조회는 호출자가 cache합니다([`text_outlines`]의 `outline` 인자).

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::value::Vec2;
use stellar_scene::geometry::{PathCommand, PathGeometry};

use crate::text::{
    ExportFontFace, GlyphOutline, GlyphOutlineError, GlyphOutlineKey, TextGlyphLayout,
};

/// run 하나의 outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutline {
    /// run 고유 색.
    pub fill: Option<Color>,
    /// run의 모든 glyph outline(text local 좌표).
    pub geometry: PathGeometry,
}

/// text layout 전체의 outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextOutlines {
    /// run별 outline(그리는 순서). 잉크가 없는 run도 순서를 유지하려고 빈 geometry로 둡니다.
    pub runs: Vec<RunOutline>,
    /// 모든 run의 outline(paint layer 기하).
    pub combined: PathGeometry,
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

fn translated(p: Vec2, dx: f32, dy: f32) -> Result<Vec2, TextOutlineError> {
    Vec2::new(p.x() + dx, p.y() + dy).map_err(|_| TextOutlineError::NonFinite)
}

/// `outline` command를 `(dx, dy)`만큼 옮겨 `out`에 덧붙입니다.
fn push_translated(
    out: &mut Vec<PathCommand>,
    outline: &PathGeometry,
    dx: f32,
    dy: f32,
    limit: usize,
) -> Result<(), TextOutlineError> {
    if out.len().saturating_add(outline.commands().len()) > limit {
        return Err(TextOutlineError::TooManyCommands { limit });
    }
    for command in outline.commands() {
        out.push(match *command {
            PathCommand::MoveTo(p) => PathCommand::MoveTo(translated(p, dx, dy)?),
            PathCommand::LineTo(p) => PathCommand::LineTo(translated(p, dx, dy)?),
            PathCommand::CubicTo { c1, c2, to } => PathCommand::CubicTo {
                c1: translated(c1, dx, dy)?,
                c2: translated(c2, dx, dy)?,
                to: translated(to, dx, dy)?,
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
/// `outline`은 key의 outline을 돌려줍니다(호출자 cache). `max_commands`는 combined path의 command 상한이며,
/// run path 합도 같은 상한을 씁니다.
pub fn text_outlines(
    layout: &TextGlyphLayout,
    outline: &mut dyn FnMut(&GlyphOutlineKey) -> Result<GlyphOutline, GlyphOutlineError>,
    max_commands: usize,
) -> Result<TextOutlines, TextOutlineError> {
    let mut combined: Vec<PathCommand> = Vec::new();
    let mut runs = Vec::with_capacity(layout.runs.len());
    for run in &layout.runs {
        let mut commands: Vec<PathCommand> = Vec::new();
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
                    push_translated(
                        &mut commands,
                        &path,
                        px(placed.x),
                        px(placed.y),
                        max_commands,
                    )?;
                }
                GlyphOutline::NoOutline => {
                    return Err(TextOutlineError::NoOutline {
                        font: run.font,
                        glyph: placed.glyph,
                    });
                }
            }
        }
        if combined.len().saturating_add(commands.len()) > max_commands {
            return Err(TextOutlineError::TooManyCommands {
                limit: max_commands,
            });
        }
        combined.extend_from_slice(&commands);
        runs.push(RunOutline {
            fill: run.fill,
            geometry: geometry(commands)?,
        });
    }
    Ok(TextOutlines {
        runs,
        combined: geometry(combined)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::test_fixtures as fx;
    use crate::text::{AxisTag, GlyphRun, GlyphSize, GlyphVariations, PlacedGlyph};

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
        ExportFontFace::new([n; 32], 0)
    }

    fn run(font: u8, size: f32, fill: Option<Color>, glyphs: &[(u32, i32, i32)]) -> GlyphRun {
        GlyphRun {
            font: face(font),
            size: GlyphSize::from_px(size).unwrap(),
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
        let outlines = text_outlines(&layout, &mut source, usize::MAX).unwrap();
        assert_eq!(outlines.runs.len(), 2);
        assert_eq!(outlines.runs[0].fill, Some(red));
        assert_eq!(outlines.runs[1].fill, None);
        assert_eq!(
            points(&outlines.runs[0].geometry),
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
            points(&outlines.runs[1].geometry),
            vec![(-1.0, -1.5), (1.0, -1.5), (1.0, 0.5), (-1.0, 0.5)]
        );
        // combined = run path를 순서대로 이은 것.
        let mut joined = outlines.runs[0].geometry.commands().to_vec();
        joined.extend_from_slice(outlines.runs[1].geometry.commands());
        assert_eq!(outlines.combined.commands(), joined.as_slice());
    }

    #[test]
    fn outline_failures_are_typed() {
        let missing = TextGlyphLayout {
            runs: vec![run(9, 8.0, None, &[(1, 0, 0)])],
        };
        assert_eq!(
            text_outlines(&missing, &mut source, usize::MAX),
            Err(TextOutlineError::Outline {
                font: face(9),
                glyph: 1,
                error: GlyphOutlineError::FontUnavailable
            })
        );
        let color = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &[(1, 0, 0), (3, 64, 0)])],
        };
        let error = text_outlines(&color, &mut source, usize::MAX).unwrap_err();
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
            text_outlines(&many, &mut source, 9),
            Err(TextOutlineError::TooManyCommands { limit: 9 })
        );
        assert!(text_outlines(&many, &mut source, 10).is_ok());
        let far = TextGlyphLayout {
            runs: vec![run(1, 8.0, None, &[(1, i32::MAX, 0)])],
        };
        assert!(text_outlines(&far, &mut source, usize::MAX).is_ok());
        for error in [
            TextOutlineError::NonFinite,
            TextOutlineError::InvalidPath,
            TextOutlineError::TooManyCommands { limit: 1 },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn keys_use_the_run_instance() {
        let wght = AxisTag::parse("wght").unwrap();
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
        text_outlines(&layout, &mut counting, usize::MAX).unwrap();
        assert_eq!(seen.len(), 2, "variation이 다르면 다른 key");
        let heavy = seen
            .keys()
            .find(|k| k.variations.get(wght) == Some(700.0))
            .unwrap();
        assert_eq!(heavy.size.sixty_fourths(), 768);
        assert_eq!(seen[heavy], 2);
    }
}
