//! Shaping에 사용한 같은 face instance로 그리는 unhinted OpenType 외곽선.

use harfrust::font::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use stellar_core_types::value::Vec2;

use crate::shaping::{FontFace, GlyphId};
use crate::units::{FontSize, SUBPIXELS_PER_PX};

/// 글리프 pen 원점 기준 logical px. 레이아웃과 마찬가지로 아래가 +y입니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineElement {
    /// contour 시작.
    Move(Vec2),
    /// 직선.
    Line(Vec2),
    /// 2차 Bézier의 제어점과 끝점.
    Quad(Vec2, Vec2),
    /// 3차 Bézier의 제어점 둘과 끝점.
    Cubic(Vec2, Vec2, Vec2),
    /// contour 닫기.
    Close,
}

/// 외곽선 실패. 잉크가 없는 정상 glyph(space 등)는 빈 path입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineError {
    /// 범위 밖 glyph, 외곽선이 없는 포맷 또는 읽을 수 없는 glyph입니다.
    Unavailable,
    /// 폰트의 경로 데이터가 잘못되었습니다.
    Invalid,
    /// glyph 하나가 worker의 path/임시 메모리 예산을 넘었습니다.
    TooComplex,
}

// 폰트가 요청한 scratch나 contour 수로 worker 메모리가 무한히 늘지 않도록 합니다.
const MAX_ELEMENTS: usize = 65_536;
const MAX_SCRATCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct Pen {
    path: Vec<OutlineElement>,
    error: Option<OutlineError>,
}

impl Pen {
    fn emit(&mut self, element: Option<OutlineElement>) {
        if self.error.is_some() {
            return;
        }
        if self.path.len() >= MAX_ELEMENTS {
            self.error = Some(OutlineError::TooComplex);
        } else if let Some(element) = element {
            self.path.push(element);
        } else {
            self.error = Some(OutlineError::Invalid);
        }
    }
}

fn point(x: f32, y: f32) -> Option<Vec2> {
    Vec2::new(x, -y).ok()
}

impl PathSink for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.emit(point(x, y).map(OutlineElement::Move));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.emit(point(x, y).map(OutlineElement::Line));
    }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.emit(
            point(a, b)
                .zip(point(x, y))
                .map(|(q, end)| OutlineElement::Quad(q, end)),
        );
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.emit(
            point(a, b)
                .zip(point(c, d))
                .zip(point(x, y))
                .map(|((p, q), end)| OutlineElement::Cubic(p, q, end)),
        );
    }
    fn close(&mut self) {
        self.emit(Some(OutlineElement::Close));
    }
}

impl FontFace {
    /// 실제 shaping glyph id·variation instance의 벡터 경로를 반환합니다. bitmap·color glyph
    /// rasterization은 별도 mode이며 이 함수는 outline만 요청합니다.
    pub fn glyph_outline(
        &self,
        glyph: GlyphId,
        size: FontSize,
    ) -> Result<Vec<OutlineElement>, OutlineError> {
        let glyph = self
            .engine_font()
            .outline_glyphs()
            .get(harfrust::GlyphId::new(glyph.raw()))
            .ok_or(OutlineError::Unavailable)?;
        if glyph.required_memory_size() > MAX_SCRATCH_BYTES {
            return Err(OutlineError::TooComplex);
        }
        let ppem = size.raw() as f32 / SUBPIXELS_PER_PX as f32;
        let mut pen = Pen::default();
        glyph
            .draw(
                &DrawOptions::new().with_ppem(Some(ppem)),
                DrawMemory::Scratch,
                &mut pen,
            )
            .map_err(|_| OutlineError::Invalid)?;
        match pen.error {
            Some(error) => Err(error),
            None => Ok(pen.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_face_outline_scales_and_empty_glyph_has_no_ink() {
        let face = FontFace::from_bytes(crate::test_fonts::outline_test_font(), 0).unwrap();
        let id = face.nominal_glyph('A').unwrap();
        let draw = |size| {
            face.glyph_outline(id, FontSize::from_px(size).unwrap())
                .unwrap()
        };
        let small = draw(20);
        assert_eq!(small, draw(20));
        assert!(small.iter().any(
            |cmd| matches!(cmd, OutlineElement::Quad(c, to) if c.y() == -10. && to.x() == 10.)
        ));
        assert!(draw(40).iter().any(
            |cmd| matches!(cmd, OutlineElement::Quad(c, to) if c.y() == -20. && to.x() == 20.)
        ));
        assert!(face
            .glyph_outline(
                face.nominal_glyph(' ').unwrap(),
                FontSize::from_px(20).unwrap()
            )
            .unwrap()
            .is_empty());
        assert_eq!(
            face.glyph_outline(GlyphId::new(404), FontSize::from_px(20).unwrap()),
            Err(OutlineError::Unavailable)
        );
    }
}
