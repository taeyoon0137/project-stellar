//! schema version별 고정 payload decoder (IX.11.21).
//!
//! 현재 decoder(`FramePayload::decode` 등)는 version 없이 잉여 bytes를 거부하므로, 저장 형식이 바뀐 뒤에는
//! 현재 decoder로 옛 bytes를 읽을 수 없습니다. 그래서 migration step은 옛 version의 bytes를 이 모듈의 고정
//! 사본으로 읽어 그 version의 레코드로 만들고, 현재 레코드로 바꾸는 일은 step이 합니다.
//!
//! - schema 1 Frame은 [`FrameV1`] 고정 사본입니다. IX.11.9(transform 분리)·IX.11.10(border 4변)이 바꿀
//!   payload라 현재 타입과 독립된 레코드로 보관합니다.
//! - schema 1의 다른 payload는 현재 decoder와 bytes가 같아 현재 decoder로 읽고, golden bytes 테스트가 그
//!   동일성을 고정합니다. 현재 decoder가 바뀌어 golden이 깨지면 그 kind의 사본을 이 모듈에 둡니다.

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::decode_payload;
use stellar_document::records::NodePayload;

/// 고정 decoder 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedDecodeError {
    /// 이 schema의 decoder가 없습니다.
    UnknownSchema(u32),
    /// bytes 형식 오류.
    Malformed(String),
}

impl std::fmt::Display for FixedDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixedDecodeError::UnknownSchema(schema) => {
                write!(f, "schema {schema}의 고정 decoder가 없습니다")
            }
            FixedDecodeError::Malformed(reason) => write!(f, "옛 payload 형식 오류: {reason}"),
        }
    }
}

impl std::error::Error for FixedDecodeError {}

fn malformed(reason: impl Into<String>) -> FixedDecodeError {
    FixedDecodeError::Malformed(reason.into())
}

/// schema 1 Frame clipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClippingV1 {
    /// tag 0.
    Disabled,
    /// tag 1.
    FrameBounds,
    /// tag 2 + 대상 StableId.
    Path(StableId),
}

/// schema 1 Frame payload 레코드. 필드 순서가 bytes 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameV1 {
    /// width, height(비음수 유한).
    pub size: [f32; 2],
    /// a, b, c, d, tx, ty(유한).
    pub transform: [f32; 6],
    /// clipping.
    pub clipping: ClippingV1,
    /// 배경색.
    pub background: Option<Color>,
    /// 테두리 색과 두께.
    pub border: Option<(Color, f32)>,
    /// 모서리 반경 tl, tr, br, bl.
    pub corner_radius: [f32; 4],
    /// layout mode 참조.
    pub layout_mode: Option<StableId>,
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FixedDecodeError> {
        if self.input.len() - self.pos < n {
            return Err(malformed("입력 부족"));
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, FixedDecodeError> {
        Ok(self.take(1)?[0])
    }

    fn f32(&mut self) -> Result<f32, FixedDecodeError> {
        let value = f32::from_le_bytes(self.take(4)?.try_into().expect("4바이트"));
        if !value.is_finite() {
            return Err(malformed("유한하지 않은 float"));
        }
        Ok(value)
    }

    fn id(&mut self) -> Result<StableId, FixedDecodeError> {
        Ok(StableId::from_bytes(
            self.take(16)?.try_into().expect("16바이트"),
        ))
    }

    fn color(&mut self) -> Result<Color, FixedDecodeError> {
        Color::from_bytes(self.take(Color::BYTE_LEN)?.try_into().expect("길이 고정"))
            .map_err(|_| malformed("색"))
    }

    fn flag(&mut self) -> Result<bool, FixedDecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(malformed("flag")),
        }
    }
}

impl FrameV1 {
    /// schema 1 Frame bytes를 읽습니다. 잉여 bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<FrameV1, FixedDecodeError> {
        let mut reader = Reader {
            input: bytes,
            pos: 0,
        };
        let size = [reader.f32()?, reader.f32()?];
        let mut transform = [0.0; 6];
        for slot in &mut transform {
            *slot = reader.f32()?;
        }
        let clipping = match reader.u8()? {
            0 => ClippingV1::Disabled,
            1 => ClippingV1::FrameBounds,
            2 => ClippingV1::Path(reader.id()?),
            _ => return Err(malformed("알 수 없는 clipping tag")),
        };
        let background = if reader.flag()? {
            Some(reader.color()?)
        } else {
            None
        };
        let border = if reader.flag()? {
            Some((reader.color()?, reader.f32()?))
        } else {
            None
        };
        let mut corner_radius = [0.0; 4];
        for slot in &mut corner_radius {
            *slot = reader.f32()?;
        }
        let layout_mode = if reader.flag()? {
            Some(reader.id()?)
        } else {
            None
        };
        if reader.pos != bytes.len() {
            return Err(malformed("trailing bytes"));
        }
        Ok(FrameV1 {
            size,
            transform,
            clipping,
            background,
            border,
            corner_radius,
            layout_mode,
        })
    }

    /// 현재 Frame 레코드로 바꿉니다. 지금 제품 schema가 1이라 의미 변환이 없습니다.
    pub fn into_current(self) -> Result<FramePayload, FixedDecodeError> {
        let [width, height] = self.size;
        let [a, b, c, d, tx, ty] = self.transform;
        let [tl, tr, br, bl] = self.corner_radius;
        let error = |e: &dyn std::fmt::Display| malformed(e.to_string());
        Ok(FramePayload {
            size: Size2::new(width, height).map_err(|e| error(&e))?,
            transform: Transform2D::new(a, b, c, d, tx, ty).map_err(|e| error(&e))?,
            clipping: match self.clipping {
                ClippingV1::Disabled => Clipping::Disabled,
                ClippingV1::FrameBounds => Clipping::FrameBounds,
                ClippingV1::Path(id) => Clipping::Path(NodeRef::new(id)),
            },
            background: self.background,
            border: self
                .border
                .map(|(color, thickness)| Border::new(color, thickness))
                .transpose()
                .map_err(|e| error(&e))?,
            corner_radius: CornerRadius::new(tl, tr, br, bl).map_err(|e| error(&e))?,
            layout_mode: self.layout_mode,
        })
    }
}

/// `schema` generation에 저장된 `kind` payload bytes를 그 version의 고정 decoder로 읽어 현재 레코드로 돌려줍니다.
pub fn decode_node_payload(
    schema: u32,
    kind: NodeKind,
    bytes: &[u8],
) -> Result<NodePayload, FixedDecodeError> {
    match (schema, kind) {
        (1, NodeKind::Frame) => Ok(NodePayload::Frame(FrameV1::decode(bytes)?.into_current()?)),
        (1, kind) => decode_payload(kind, bytes).map_err(|e| malformed(e.to_string())),
        (schema, _) => Err(FixedDecodeError::UnknownSchema(schema)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::text::UserText;
    use stellar_document::record_codec::encode_payload;
    use stellar_document::shape::{RectangleParams, ShapeGeometry};
    use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};

    /// schema 1 Rectangle golden bytes.
    const RECTANGLE_V1_GOLDEN: &str = "010000404000008040000000000000803f000000000000803f";
    /// schema 1 Text golden bytes(두 문단, style run 하나).
    const TEXT_V1_GOLDEN: &str =
        "03000000610a6202000000000000000200000001000000000000000300000001000000";

    /// schema 1 Frame golden bytes(경로 clipping·배경·테두리·반경·layout 참조). 이 bytes는 고정입니다.
    const FRAME_V1_GOLDEN: &str = concat!(
        "0000c8420000484300000000000080bf0000803f0000000000002041",
        "0000a0c1020000000000000000000000000000006301010000803f00",
        "000000000000000000803f010200000000000000000000803f000080",
        "3f0000003f0000803f00000040000040400000804001000000000000",
        "00000000000000000064",
    );

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn golden_frame() -> FramePayload {
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
        let blue = Color::new(ColorSpace::DisplayP3, 0.0, 0.0, 1.0, 1.0).unwrap();
        FramePayload {
            size: Size2::new(100.0, 200.0).unwrap(),
            transform: Transform2D::new(0.0, -1.0, 1.0, 0.0, 10.0, -20.0).unwrap(),
            clipping: Clipping::Path(NodeRef::new(StableId::from_bytes({
                let mut b = [0; 16];
                b[15] = 0x63;
                b
            }))),
            background: Some(red),
            border: Some(Border::new(blue, 0.5).unwrap()),
            corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap(),
            layout_mode: Some(StableId::from_bytes({
                let mut b = [0; 16];
                b[15] = 0x64;
                b
            })),
        }
    }

    #[test]
    fn frame_v1_golden_bytes_decode_with_the_fixed_copy() {
        let bytes = hex(FRAME_V1_GOLDEN);
        // 현재 encoder가 같은 bytes를 만드는 동안 schema 1은 현재 형식과 같습니다. 형식이 바뀌면 이 단언만
        // 바뀌고 고정 사본의 golden decode는 그대로 통과해야 합니다.
        assert_eq!(golden_frame().encode(), bytes);
        let decoded = decode_node_payload(1, NodeKind::Frame, &bytes).unwrap();
        assert_eq!(decoded, NodePayload::Frame(golden_frame()));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(FrameV1::decode(&trailing).is_err());
        assert!(FrameV1::decode(&bytes[..bytes.len() - 1]).is_err());
        assert_eq!(
            decode_node_payload(2, NodeKind::Frame, &bytes),
            Err(FixedDecodeError::UnknownSchema(2))
        );
    }

    #[test]
    fn other_schema_1_payloads_are_frozen_by_golden_bytes() {
        // Frame 밖 schema 1 payload는 현재 decoder로 읽습니다. 그 전제(현재 encoder·decoder가 schema 1 bytes와
        // 같음)를 고정 bytes로 묶어, 현재 형식이 바뀌면 이 테스트가 깨져 그 kind의 고정 사본을 두게 합니다.
        let rectangle = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(3.0, 4.0).unwrap(),
            corner_radius: CornerRadius::new(0.0, 1.0, 0.0, 1.0).unwrap(),
        }));
        let text = NodePayload::Text(
            TextPayload::new(
                UserText::new("a\nb"),
                vec![Paragraph { style: 0 }, Paragraph { style: 2 }],
                vec![StyleRun {
                    range: TextRange::new(0, 3).unwrap(),
                    style: 1,
                }],
            )
            .unwrap(),
        );
        for (payload, golden) in [(rectangle, RECTANGLE_V1_GOLDEN), (text, TEXT_V1_GOLDEN)] {
            let bytes = hex(golden);
            assert_eq!(encode_payload(&payload), bytes, "{:?}", payload.kind());
            assert_eq!(
                decode_node_payload(1, payload.kind(), &bytes).unwrap(),
                payload
            );
        }
    }

    /// schema 1 golden bytes를 만들 나머지 node 종류 payload(Ellipse·Line·Polygon·Path·Image·Fragment·
    /// ComponentInstance).
    fn remaining_schema_1_payloads() -> Vec<(&'static str, NodePayload)> {
        use stellar_core_types::reference::{AssetRef, ComponentRef};
        use stellar_core_types::typed_value::TypedValue;
        use stellar_core_types::value::{Angle, Rect, Vec2};
        use stellar_document::component::definition::{ComponentPropertyId, ComponentSlotId};
        use stellar_document::component::instance::{
            ComponentInstancePayload, OverrideKey, SlotOverride,
        };
        use stellar_document::fragment::FragmentPayload;
        use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
        use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
        use stellar_document::shape::{EllipseParams, LineCap, LineParams, PolygonParams};

        let id = |n: u8| {
            let mut b = [0; 16];
            b[15] = n;
            StableId::from_bytes(b)
        };
        let point = |x: f32, y: f32| Vec2::new(x, y).unwrap();
        let rect = |x, y, w, h| Rect::new(point(x, y), Size2::new(w, h).unwrap());
        let path = PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(point(0.0, 0.0)),
                PathCommand::LineTo(point(5.0, 2.5)),
                PathCommand::CubicTo {
                    c1: point(1.0, 2.0),
                    c2: point(3.0, 4.0),
                    to: point(5.0, 6.0),
                },
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut next = 0x40u8;
        let path = IdentifiedPath::assign(path, &mut || {
            next += 1;
            id(next)
        });
        let mut instance = ComponentInstancePayload::new(ComponentRef::new(id(0x50)));
        instance.set_property_override(
            OverrideKey::direct(ComponentPropertyId::new(id(0x51))),
            TypedValue::Boolean(true),
        );
        instance.set_slot_override(
            ComponentSlotId::new(id(0x52)),
            SlotOverride::new(vec![id(0x53)]).unwrap(),
        );
        vec![
            (
                "ellipse",
                NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                    bounds: rect(1.0, -2.0, 30.0, 20.5),
                })),
            ),
            (
                "line",
                NodePayload::Shape(ShapeGeometry::Line(LineParams {
                    start: point(-1.5, 2.0),
                    end: point(40.0, 8.25),
                    cap: LineCap::Round,
                })),
            ),
            (
                "polygon",
                NodePayload::Shape(ShapeGeometry::Polygon(
                    PolygonParams::new(
                        rect(0.0, 0.0, 64.0, 48.0),
                        6,
                        Angle::new(15.0).unwrap(),
                        2.5,
                    )
                    .unwrap(),
                )),
            ),
            ("path", NodePayload::Path(path)),
            (
                "image",
                NodePayload::Image(ImagePayload {
                    asset: AssetRef::new(id(0x60)),
                    size: Size2::new(64.0, 48.0).unwrap(),
                    fit: FitMode::Cover,
                    crop: CropRect::from_components(0.0, 0.25, 1.0, 0.5).unwrap(),
                    sampling: SamplingMode::Nearest,
                    color_space_override: Some(ColorSpace::DisplayP3),
                }),
            ),
            ("fragment", NodePayload::Fragment(FragmentPayload)),
            (
                "component-instance",
                NodePayload::ComponentInstance(instance),
            ),
        ]
    }

    /// 나머지 node 종류의 schema 1 golden bytes(이름 순서는 [`remaining_schema_1_payloads`]와 같음). Shape는
    /// 종류 tag(Ellipse 2, Line 3, Polygon 4) 뒤 f32 LE 필드, Fragment는 빈 payload입니다. 이 bytes는 고정입니다.
    const REMAINING_V1_GOLDEN: [(&str, &str); 7] = [
        ("ellipse", "020000803f000000c00000f0410000a441"),
        ("line", "030000c0bf00000040000020420000044101"),
        (
            "polygon",
            "0400000000000000000000804200004042060000000000704100002040",
        ),
        (
            "path",
            concat!(
                "310000000104000000010000000000000000020000a04000002040030000803f",
                "0000004000004040000080400000a0400000c040040400000001000000000000",
                "0000000000000000004101000000000000000000000000000000420300000000",
                "0000000000000000000000430000000000000000000000000000004400000000",
                "00000000000000000000004500",
            ),
        ),
        (
            "image",
            "00000000000000000000000000000060000080420000404202000000000000803e0000803f0000003f0102",
        ),
        ("fragment", ""),
        (
            "component-instance",
            concat!(
                "0000000000000000000000000000005001000000000000000000000000000000",
                "0000000000000051020000000101010000000000000000000000000000000000",
                "00520100000000000000000000000000000000000053",
            ),
        ),
    ];

    #[test]
    fn every_other_node_kind_is_frozen_by_schema_1_golden_bytes() {
        // IX.11.21: Frame·Rectangle·Text 밖의 모든 node 종류(Ellipse·Line·Polygon·Path·Image·Fragment·
        // ComponentInstance)도 schema 1 bytes를 고정합니다. 현재 encoder·decoder가 이 bytes와 달라지면 그 종류의
        // 고정 사본을 이 모듈에 두어야 합니다.
        let payloads = remaining_schema_1_payloads();
        assert_eq!(payloads.len(), REMAINING_V1_GOLDEN.len());
        for ((name, payload), (golden_name, golden)) in
            payloads.into_iter().zip(REMAINING_V1_GOLDEN)
        {
            assert_eq!(name, golden_name);
            let bytes = hex(golden);
            assert_eq!(encode_payload(&payload), bytes, "{name}");
            assert_eq!(
                decode_node_payload(1, payload.kind(), &bytes).unwrap(),
                payload,
                "{name}"
            );
            if !bytes.is_empty() {
                let mut trailing = bytes.clone();
                trailing.push(0);
                assert!(
                    decode_node_payload(1, payload.kind(), &trailing).is_err(),
                    "{name}"
                );
            }
        }
    }
}
