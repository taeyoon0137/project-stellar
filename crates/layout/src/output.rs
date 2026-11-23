//! 레이아웃 출력 (X.1.1).
//!
//! [`LayoutBox`]는 node 하나의 결과입니다.
//!
//! - **local rect**: 부모 node local 좌표(부모 border box 좌상단 원점)에서의 border box.
//! - **content rect**: node 자신의 local 좌표에서 padding을 뺀 content box.
//! - **baseline**: node top에서 첫 baseline까지의 거리. 없으면 `None`.
//! - **overflow bounds**: node 자신의 local 좌표에서 자신과 (표시되는) 자손 box를 모두 포함하는
//!   bounds. 항상 자신의 bounds `(0, 0, width, height)`를 포함합니다.
//!
//! 값은 모두 [`LayoutUnit`] 정수라 유한하며, [`LayoutResult::encode`] bytes가 플랫폼 간 비교
//! 단위입니다(magic `SLYR`, version 2).

use std::collections::HashMap;

use stellar_core_types::ids::StableId;

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::geometry::{Axis, LayoutRect};
use crate::rounding::LayoutUnit;

/// 결과 canonical bytes magic.
const RESULT_MAGIC: &[u8; 4] = b"SLYR";
/// 결과 canonical bytes version.
const RESULT_VERSION: u16 = 2;

/// 진단 tag.
const DIAGNOSTIC_CYCLIC_PERCENTAGE: u8 = 0;

fn decode_axis(r: &mut Reader<'_>) -> Result<Axis, LayoutError> {
    match r.u8()? {
        0 => Ok(Axis::Horizontal),
        1 => Ok(Axis::Vertical),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 진단 axis",
        }),
    }
}

/// 레이아웃을 실패시키지 않는 진단 (ADR-0076).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutDiagnostic {
    /// 순환 백분율(CSS Sizing 3 §5.2.1, IX.11.5): `node`의 백분율 크기 기준인 `ancestor`의 그 축 크기가 콘텐츠로
    /// 정해졌습니다. CSS처럼 `ancestor`의 크기 계산에서는 `node`의 백분율을 `auto`로 보고, 정해진 크기로 백분율을
    /// 다시 해석했습니다. 그래서 `node`가 `ancestor`를 넘치거나 남는 공간이 생길 수 있습니다.
    CyclicPercentage {
        /// 크기가 콘텐츠로 정해진 containing block(또는 grid container).
        ancestor: StableId,
        /// 백분율 크기 node.
        node: StableId,
        /// 축.
        axis: Axis,
    },
}

impl std::fmt::Display for LayoutDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutDiagnostic::CyclicPercentage {
                ancestor,
                node,
                axis,
            } => write!(
                f,
                "node {node}의 {axis} 백분율 크기 기준 {ancestor}의 크기가 콘텐츠로 정해져, 백분율을 auto로 보고 측정한 뒤 다시 해석했습니다"
            ),
        }
    }
}

/// node 하나의 레이아웃 결과 (X.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutBox {
    id: StableId,
    rect: LayoutRect,
    content: LayoutRect,
    baseline: Option<LayoutUnit>,
    overflow: LayoutRect,
}

impl LayoutBox {
    /// 결과 box를 만듭니다. overflow bounds가 자신의 bounds를 포함하지 않으면 오류입니다.
    pub fn new(
        id: StableId,
        rect: LayoutRect,
        content: LayoutRect,
        baseline: Option<LayoutUnit>,
        overflow: LayoutRect,
    ) -> Result<LayoutBox, LayoutError> {
        if !overflow.contains_rect(&LayoutRect::at_origin(rect.size())) {
            return Err(LayoutError::InvalidBox {
                node: id,
                reason: "overflow bounds가 자신의 bounds를 포함하지 않습니다",
            });
        }
        Ok(LayoutBox {
            id,
            rect,
            content,
            baseline,
            overflow,
        })
    }

    /// node.
    pub const fn id(&self) -> StableId {
        self.id
    }

    /// 부모 local 좌표의 border box.
    pub const fn local_rect(&self) -> LayoutRect {
        self.rect
    }

    /// 자신의 local 좌표의 content box.
    pub const fn content_rect(&self) -> LayoutRect {
        self.content
    }

    /// node top 기준 첫 baseline.
    pub const fn baseline(&self) -> Option<LayoutUnit> {
        self.baseline
    }

    /// 자신의 local 좌표의 overflow bounds.
    pub const fn overflow_bounds(&self) -> LayoutRect {
        self.overflow
    }
}

/// 레이아웃 tree 전체 결과 (X.1.1).
///
/// box 순서는 tree pre-order(부모 다음 자식, 자식은 visual order)입니다.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    boxes: Vec<LayoutBox>,
    index: HashMap<StableId, usize>,
    diagnostics: Vec<LayoutDiagnostic>,
}

impl PartialEq for LayoutResult {
    fn eq(&self, other: &LayoutResult) -> bool {
        // index는 boxes에서 파생되므로 boxes와 진단만 비교합니다.
        self.boxes == other.boxes && self.diagnostics == other.diagnostics
    }
}

impl Eq for LayoutResult {}

impl LayoutResult {
    /// box 목록으로 결과를 만듭니다. 같은 node의 box가 둘 이상이면 오류입니다.
    pub fn new(boxes: Vec<LayoutBox>) -> Result<LayoutResult, LayoutError> {
        LayoutResult::with_diagnostics(boxes, Vec::new())
    }

    /// box 목록과 진단으로 결과를 만듭니다. 같은 node의 box가 둘 이상이면 오류입니다.
    pub fn with_diagnostics(
        boxes: Vec<LayoutBox>,
        diagnostics: Vec<LayoutDiagnostic>,
    ) -> Result<LayoutResult, LayoutError> {
        let mut index = HashMap::with_capacity(boxes.len());
        for (position, layout_box) in boxes.iter().enumerate() {
            if index.insert(layout_box.id, position).is_some() {
                return Err(LayoutError::DuplicateBox {
                    node: layout_box.id,
                });
            }
        }
        Ok(LayoutResult {
            boxes,
            index,
            diagnostics,
        })
    }

    /// 비치명 진단(배치 순서).
    pub fn diagnostics(&self) -> &[LayoutDiagnostic] {
        &self.diagnostics
    }

    /// 모든 box(pre-order).
    pub fn boxes(&self) -> &[LayoutBox] {
        &self.boxes
    }

    /// node의 box.
    pub fn get(&self, id: StableId) -> Option<&LayoutBox> {
        self.index.get(&id).map(|&position| &self.boxes[position])
    }

    /// box 수.
    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// canonical bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(RESULT_MAGIC);
        w.u16(RESULT_VERSION);
        w.len(self.boxes.len());
        for layout_box in &self.boxes {
            w.id(layout_box.id);
            encode_rect(&mut w, &layout_box.rect);
            encode_rect(&mut w, &layout_box.content);
            match layout_box.baseline {
                Some(baseline) => {
                    w.bool(true);
                    w.unit(baseline);
                }
                None => w.bool(false),
            }
            encode_rect(&mut w, &layout_box.overflow);
        }
        w.len(self.diagnostics.len());
        for diagnostic in &self.diagnostics {
            match diagnostic {
                LayoutDiagnostic::CyclicPercentage {
                    ancestor,
                    node,
                    axis,
                } => {
                    w.u8(DIAGNOSTIC_CYCLIC_PERCENTAGE);
                    w.id(*ancestor);
                    w.id(*node);
                    w.u8(axis.index() as u8);
                }
            }
        }
        w.finish()
    }

    /// canonical bytes에서 디코딩하고 box 불변식을 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<LayoutResult, LayoutError> {
        let mut r = Reader::new(bytes);
        r.expect_magic(RESULT_MAGIC, RESULT_VERSION)?;
        let count = r.len(16 + 16 * 3 + 1)?;
        let mut boxes = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.id()?;
            let rect = decode_rect(&mut r)?;
            let content = decode_rect(&mut r)?;
            let baseline = if r.bool("baseline flag")? {
                Some(r.unit()?)
            } else {
                None
            };
            let overflow = decode_rect(&mut r)?;
            boxes.push(LayoutBox::new(id, rect, content, baseline, overflow)?);
        }
        let count = r.len(18)?;
        let mut diagnostics = Vec::with_capacity(count);
        for _ in 0..count {
            let diagnostic = match r.u8()? {
                DIAGNOSTIC_CYCLIC_PERCENTAGE => {
                    let ancestor = r.id()?;
                    let node = r.id()?;
                    let axis = decode_axis(&mut r)?;
                    LayoutDiagnostic::CyclicPercentage {
                        ancestor,
                        node,
                        axis,
                    }
                }
                _ => {
                    return Err(LayoutError::Malformed {
                        reason: "알 수 없는 진단 tag",
                    })
                }
            };
            diagnostics.push(diagnostic);
        }
        r.expect_end()?;
        LayoutResult::with_diagnostics(boxes, diagnostics)
    }
}

fn encode_rect(w: &mut Writer, rect: &LayoutRect) {
    w.unit(rect.x());
    w.unit(rect.y());
    w.unit(rect.width());
    w.unit(rect.height());
}

fn decode_rect(r: &mut Reader<'_>) -> Result<LayoutRect, LayoutError> {
    let x = r.unit()?;
    let y = r.unit()?;
    let width = r.unit()?;
    let height = r.unit()?;
    LayoutRect::new(x, y, width, height).ok_or(LayoutError::Malformed {
        reason: "음수 크기 rect",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{id, px};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> LayoutRect {
        LayoutRect::new(px(x), px(y), px(w), px(h)).unwrap()
    }

    fn sample() -> LayoutResult {
        let root = LayoutBox::new(
            id(1),
            rect(0, 0, 200, 100),
            rect(8, 4, 184, 92),
            None,
            rect(-10, 0, 230, 100),
        )
        .unwrap();
        let text = LayoutBox::new(
            id(2),
            LayoutRect::new(LayoutUnit::from_raw(21), px(4), px(50), px(20)).unwrap(),
            rect(0, 0, 50, 20),
            Some(LayoutUnit::from_raw(14 * 64 + 21)),
            rect(0, 0, 50, 20),
        )
        .unwrap();
        LayoutResult::new(vec![root, text]).unwrap()
    }

    #[test]
    fn result_round_trips_through_canonical_bytes() {
        let result = sample();
        let bytes = result.encode();
        assert_eq!(&bytes[..4], b"SLYR");
        let decoded = LayoutResult::decode(&bytes).unwrap();
        assert_eq!(decoded, result);
        assert_eq!(decoded.encode(), bytes);
        let text = decoded.get(id(2)).unwrap();
        assert_eq!(text.local_rect().x().raw(), 21);
        assert_eq!(text.baseline(), Some(LayoutUnit::from_raw(917)));
        assert_eq!(
            decoded.get(id(1)).unwrap().content_rect(),
            rect(8, 4, 184, 92)
        );
        assert_eq!(decoded.len(), 2);
        assert!(decoded.get(id(3)).is_none());
    }

    #[test]
    fn result_values_convert_to_finite_rects() {
        // 모든 결과 값은 정수 subpixel이라 core-types finite Rect 변환이 항상 성공합니다.
        for layout_box in sample().boxes() {
            for r in [
                layout_box.local_rect(),
                layout_box.content_rect(),
                layout_box.overflow_bounds(),
            ] {
                let converted = r.to_rect();
                assert!(converted.origin().x().is_finite() && converted.origin().y().is_finite());
                assert!(converted.size().width().is_finite());
            }
        }
    }

    #[test]
    fn box_and_result_invariants_are_enforced() {
        // overflow가 자신의 bounds를 포함하지 않으면 거부.
        assert!(matches!(
            LayoutBox::new(
                id(1),
                rect(5, 5, 10, 10),
                rect(0, 0, 10, 10),
                None,
                rect(0, 0, 9, 10)
            ),
            Err(LayoutError::InvalidBox { .. })
        ));
        // 중복 box 거부.
        let one = sample().boxes()[0];
        assert_eq!(
            LayoutResult::new(vec![one, one]),
            Err(LayoutError::DuplicateBox { node: id(1) })
        );
        // 디코딩: 음수 크기와 overflow 불변식 위반 거부.
        let bytes = sample().encode();
        let mut negative = bytes.clone();
        // 첫 box rect width: magic4+ver2+count4+id16+x4+y4.
        let width_offset = 4 + 2 + 4 + 16 + 8;
        negative[width_offset..width_offset + 4].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(
            LayoutResult::decode(&negative),
            Err(LayoutError::Malformed {
                reason: "음수 크기 rect"
            })
        );
        let mut shrunk = bytes.clone();
        // 첫 box overflow width(rect16+content16+baseline flag1 이후 x4+y4).
        let overflow_width = 4 + 2 + 4 + 16 + 16 + 16 + 1 + 8;
        shrunk[overflow_width..overflow_width + 4].copy_from_slice(&64i32.to_le_bytes());
        assert!(matches!(
            LayoutResult::decode(&shrunk),
            Err(LayoutError::InvalidBox { .. })
        ));
        let mut bad_flag = bytes;
        bad_flag[4 + 2 + 4 + 16 + 32] = 2;
        assert!(matches!(
            LayoutResult::decode(&bad_flag),
            Err(LayoutError::Malformed { .. })
        ));
    }
}
