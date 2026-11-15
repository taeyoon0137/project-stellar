//! 공통 text layout을 renderer outline 요청과 scene의 줄 hit 영역으로 변환합니다.
//! 입력 face·크기·style은 shaping이 확정한 값입니다. 누락된 문서 typography를 대체하지 않습니다.

use stellar_core_types::value::{FiniteF32, Rect, Size2, Vec2};
use stellar_document::path::quadratic_to_cubic;
use stellar_renderer_core::glyph::{
    AxisTag, FontHash, GlyphError, GlyphId, GlyphImage, GlyphKey, GlyphRasterizer, GlyphRenderMode,
    GlyphSize, VariationCoords,
};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::narrow_phase::TextHitGeometry;
use stellar_scene::node::TextLayoutKey;
use stellar_text::font::{FontFileHash, FontVariation, OpenTypeTag};
use stellar_text::font_runs::FaceSource;
use stellar_text::outline::OutlineElement;
use stellar_text::registry::{FontFaceKey, FontRegistry};
use stellar_text::shaping;
use stellar_text::text_layout::{PlacedRun, TextLayout};
use stellar_text::units::{FontSize, TextUnit, SUBPIXELS_PER_PX};

const fn failure(reason: &'static str) -> GlyphError {
    GlyphError::Rasterize { reason }
}

/// 이미 로드한 bytes와 동일한 registry를 사용합니다. 이 경로는 디스크에 접근하지 않습니다.
pub struct TextOutlineRasterizer<'a, R, S> {
    /// font hash/face index → shaping handle.
    pub registry: &'a R,
    /// shaping에 사용한 기본 face들.
    pub faces: &'a S,
}

impl<R: FontRegistry + Sync, S: FaceSource + Sync> GlyphRasterizer
    for TextOutlineRasterizer<'_, R, S>
{
    fn rasterize(&self, key: &GlyphKey) -> Result<GlyphImage, GlyphError> {
        if key.mode != GlyphRenderMode::Outline {
            return Err(failure("outline mode 필요"));
        }
        let face_key = FontFaceKey::new(
            FontFileHash::from_sha256_digest(*key.font.file_sha256()),
            key.font.face_index(),
        );
        let face = self
            .registry
            .lookup(face_key)
            .and_then(|h| self.faces.face(h))
            .filter(|f| f.face_index() == key.font.face_index())
            .ok_or(failure("글꼴 face 누락"))?;
        let axes = key
            .variations
            .axes()
            .map(|(tag, value)| {
                Ok(FontVariation {
                    tag: OpenTypeTag::new(tag.bytes()).ok_or(GlyphError::InvalidTag)?,
                    value: FiniteF32::new(value).map_err(|_| GlyphError::NonFiniteVariation)?,
                })
            })
            .collect::<Result<Vec<_>, GlyphError>>()?;
        let instance;
        let face = if face.variations() == axes {
            face
        } else {
            instance = face.with_variations(&axes);
            &instance
        };
        let size = FontSize::from_raw(key.size.sixty_fourths() as i32)
            .map_err(|_| GlyphError::InvalidSize)?;
        let outline = face
            .glyph_outline(shaping::GlyphId::new(key.glyph.0), size)
            .map_err(|err| match err {
                stellar_text::outline::OutlineError::Unavailable => failure("glyph outline 누락"),
                stellar_text::outline::OutlineError::Invalid => failure("잘못된 glyph outline"),
                stellar_text::outline::OutlineError::TooComplex => {
                    failure("glyph outline 예산 초과")
                }
            })?;
        if outline.is_empty() {
            return Ok(GlyphImage::Empty);
        }
        let mut pen = None;
        let mut path = Vec::with_capacity(outline.len());
        for element in outline {
            let cmd = match element {
                OutlineElement::Move(to) => {
                    pen = Some(to);
                    PathCommand::MoveTo(to)
                }
                OutlineElement::Line(to) => {
                    pen = Some(to);
                    PathCommand::LineTo(to)
                }
                OutlineElement::Quad(control, to) => {
                    let from = pen.replace(to).ok_or(failure("시작점 없는 glyph 곡선"))?;
                    match quadratic_to_cubic(from, control, to)
                        .map_err(|_| failure("glyph 좌표 범위 초과"))?
                    {
                        stellar_document::path::PathCommand::CubicTo { c1, c2, to } => {
                            PathCommand::CubicTo { c1, c2, to }
                        }
                        _ => return Err(failure("glyph 곡선 변환 실패")),
                    }
                }
                OutlineElement::Cubic(c1, c2, to) => {
                    pen = Some(to);
                    PathCommand::CubicTo { c1, c2, to }
                }
                OutlineElement::Close => {
                    pen = None;
                    PathCommand::Close
                }
            };
            path.push(cmd);
        }
        PathGeometry::new(path)
            .map(GlyphImage::Outline)
            .map_err(|_| failure("잘못된 glyph contour"))
    }
}

/// logical text box 안의 pen 위치와 outline 요청. draw 시 camera/device 배율을 적용합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasGlyph {
    /// pen 원점 (x + offset_x, baseline - offset_y).
    pub origin: Vec2,
    /// cache용 정확한 face·glyph·logical 크기·variation.
    pub key: GlyphKey,
    /// input style 표의 index(색 등은 호출자가 이 표에서 찾습니다).
    pub style: u32,
}

/// 하나의 공통 layout에 대응하는 draw 순서와 scene picking 영역.
pub struct CanvasTextGeometry {
    key: TextLayoutKey,
    line_rects: Vec<Rect>,
    /// visual 순서입니다. 렌더러의 GlyphResolver가 같은 key의 outline을 공유합니다.
    pub glyphs: Vec<CanvasGlyph>,
}

fn px(unit: TextUnit) -> f32 {
    unit.raw() as f32 / SUBPIXELS_PER_PX as f32
}

fn run_key(run: &PlacedRun) -> Result<Option<GlyphKey>, GlyphError> {
    let Some(face) = &run.face else {
        return Ok(None);
    };
    let axes = face
        .variations()
        .iter()
        .map(|v| Ok((AxisTag::new(v.tag.bytes())?, v.value.get())))
        .collect::<Result<Vec<_>, GlyphError>>()?;
    Ok(Some(GlyphKey {
        font: FontHash::new(*face.file_hash().sha256_digest(), face.face_index()),
        size: GlyphSize::from_sixty_fourths(run.size.raw() as u32)?,
        glyph: GlyphId::NOTDEF,
        variations: VariationCoords::new(axes)?,
        mode: GlyphRenderMode::Outline,
    }))
}

impl CanvasTextGeometry {
    /// 이미 배치된 줄과 glyph를 그대로 씁니다. 문자열을 재순회하거나 shaping하지 않습니다.
    pub fn from_layout(key: TextLayoutKey, layout: &TextLayout) -> Result<Self, GlyphError> {
        let styles = layout
            .runs
            .iter()
            .map(|runs| {
                runs.iter()
                    .map(|run| Ok((run_key(run)?, run.style)))
                    .collect::<Result<Vec<_>, GlyphError>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut glyphs = Vec::new();
        let mut line_rects = Vec::with_capacity(layout.lines.len());
        for line in &layout.lines {
            line_rects.push(Rect::new(
                Vec2::new(px(line.x), px(line.top)).map_err(|_| failure("줄 좌표"))?,
                Size2::new(px(line.width), px(line.height)).map_err(|_| failure("줄 크기"))?,
            ));
            for g in &line.glyphs {
                let (Some(base), style) = styles
                    .get(line.paragraph)
                    .and_then(|runs| runs.get(g.run))
                    .ok_or(failure("glyph run 누락"))?
                else {
                    return Err(failure("glyph face 누락"));
                };
                let x =
                    g.x.checked_add(g.x_offset)
                        .map_err(|_| failure("glyph x 범위 초과"))?;
                let y = g
                    .baseline
                    .checked_sub(g.y_offset)
                    .map_err(|_| failure("glyph y 범위 초과"))?;
                glyphs.push(CanvasGlyph {
                    key: GlyphKey {
                        glyph: GlyphId(g.glyph.raw()),
                        ..base.clone()
                    },
                    origin: Vec2::new(px(x), px(y)).map_err(|_| failure("glyph 좌표"))?,
                    style: *style,
                });
            }
        }
        Ok(Self {
            key,
            glyphs,
            line_rects,
        })
    }
}

impl TextHitGeometry for CanvasTextGeometry {
    fn line_rects(&self, key: TextLayoutKey) -> Option<&[Rect]> {
        (key == self.key).then_some(self.line_rects.as_slice())
    }
}

#[cfg(test)]
mod tests;

pub(crate) mod document;
pub(crate) mod fonts;
pub(crate) mod saved;
pub use document::DocumentText;

/// 확정 전 IME 조합을 반영한 Text 한 개(ADR-0067 §6). 문서와 history에는 없습니다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextPreview {
    /// (조합 token, 갱신 순번). 같은 값이면 게시한 scene을 다시 씁니다.
    pub(crate) key: (u64, u64),
    /// 조합 대상 node. 새 Text면 확정 때 만들 ID입니다.
    pub(crate) node: stellar_core_types::ids::StableId,
    /// 조합 중 본문을 넣은 payload.
    pub(crate) payload: stellar_document::text::TextPayload,
    /// 새 Text: 부모 Frame과 확정 때 기록할 modifier stack(원점 Transform).
    pub(crate) new_in: Option<(
        stellar_core_types::ids::StableId,
        stellar_document::modifier::ModifierStack,
    )>,
}

/// Native paint bridge에서 공통 outline을 순회합니다.
pub use stellar_scene::geometry::PathCommand as CanvasPathCommand;
