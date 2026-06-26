//! `TypedValue` tagged union (III.3.5).
//!
//! 표현식·토큰·속성 값을 담는 타입드 값 union입니다. 각 variant는 **안정적인 numeric
//! type tag**를 가지며, 이 tag는 저장 포맷의 일부이므로 한 번 정한 뒤 절대 바꾸지 않습니다.
//! 모든 variant는 [`TypedValue::to_bytes`] ↔ [`TypedValue::from_bytes`]로 정확히 round-trip
//! 합니다.
//!
//! 일부 payload 타입은 아직 정식 정의되지 않았습니다(`Gradient` III.4.3, `Shadow` III.4.4,
//! `Typography` VI.7.2/X.8.1). III.3.5에서는 이들의 **tag 위치와 round-trip envelope만**
//! 고정하고, 구체 구조는 해당 후속 작업이 채웁니다. 그때까지는 불투명(opaque) byte payload
//! placeholder로 둡니다([`GradientValue`] 등). `Color`는 III.4.1에서 정식 타입([`Color`])으로
//! 교체되었습니다(tag `0x09` 유지).

use std::fmt;

use crate::color::{Color, ColorError};
use crate::ids::StableId;
use crate::value::{Angle, Duration, FiniteF32, Length, Percentage};

/// 리소스 참조 값. resource(컴포넌트·토큰 외 자원)를 `StableId`로 가리킵니다.
///
/// 강한 타입 참조([`crate::reference`])와 달리 `TypedValue` payload로 쓰이는 값 타입입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceRef(StableId);

impl ResourceRef {
    /// `StableId`로부터 리소스 참조를 만듭니다.
    pub const fn new(id: StableId) -> ResourceRef {
        ResourceRef(id)
    }

    /// 대상 `StableId`.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

/// 열거형 값. 어떤 enum 타입(`enum_type`)의 몇 번째 variant(`variant`)인지 numeric으로
/// 식별합니다. 사람이 읽는 이름은 상위 계층 registry가 보관합니다(문자열을 값에 담지 않음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnumValue {
    enum_type: u32,
    variant: u32,
}

impl EnumValue {
    /// enum 타입 ID와 variant ordinal로 enum 값을 만듭니다.
    pub const fn new(enum_type: u32, variant: u32) -> EnumValue {
        EnumValue { enum_type, variant }
    }

    /// enum 타입 ID.
    pub const fn enum_type(&self) -> u32 {
        self.enum_type
    }

    /// variant ordinal.
    pub const fn variant(&self) -> u32 {
        self.variant
    }
}

/// 아직 정식 정의되지 않은 payload의 placeholder를 정의합니다. 불투명 byte buffer를
/// 보존하며 round-trip만 보장합니다. 정식 타입은 명시한 후속 작업이 대체합니다.
macro_rules! define_opaque_placeholder {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(Vec<u8>);

        impl $name {
            /// 불투명 payload bytes로부터 만듭니다.
            pub fn new(bytes: Vec<u8>) -> $name {
                $name(bytes)
            }

            /// slice로부터 만듭니다.
            pub fn from_slice(bytes: &[u8]) -> $name {
                $name(bytes.to_vec())
            }

            /// 보존된 payload bytes.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

define_opaque_placeholder!(
    GradientValue,
    "그라디언트 값 placeholder. 정식 정의는 III.4.3(stop·position). 현재는 tag와 round-trip만 고정."
);
define_opaque_placeholder!(
    ShadowValue,
    "그림자 값 placeholder. 정식 정의는 III.4.4(radius·blur). 현재는 tag와 round-trip만 고정."
);
define_opaque_placeholder!(
    TypographyValue,
    "타이포그래피 값 placeholder. 정식 정의는 VI.7.2/X.8.1. 현재는 tag와 round-trip만 고정."
);

/// 타입드 값 union (III.3.5).
///
/// variant 선언 순서와 type tag(아래 `TAG_*`)는 저장 포맷 계약입니다. 새 variant는 항상
/// 끝에 추가하고 기존 tag는 재사용·변경하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypedValue {
    /// 불리언.
    Boolean(bool),
    /// 무차원 수치(유한 float).
    Number(FiniteF32),
    /// UTF-8 문자열.
    String(String),
    /// 열거형 값.
    Enum(EnumValue),
    /// 길이.
    Length(Length),
    /// 백분율.
    Percentage(Percentage),
    /// 각도.
    Angle(Angle),
    /// 지속 시간.
    Duration(Duration),
    /// 색상(III.4.1).
    Color(Color),
    /// 그라디언트(placeholder, III.4.3).
    Gradient(GradientValue),
    /// 그림자(placeholder, III.4.4).
    Shadow(ShadowValue),
    /// 타이포그래피(placeholder, VI.7.2/X.8.1).
    Typography(TypographyValue),
    /// 리소스 참조.
    ResourceRef(ResourceRef),
}

/// 안정 type tag. 저장 포맷 계약이므로 값을 변경·재사용하지 않습니다.
impl TypedValue {
    const TAG_BOOLEAN: u8 = 0x01;
    const TAG_NUMBER: u8 = 0x02;
    const TAG_STRING: u8 = 0x03;
    const TAG_ENUM: u8 = 0x04;
    const TAG_LENGTH: u8 = 0x05;
    const TAG_PERCENTAGE: u8 = 0x06;
    const TAG_ANGLE: u8 = 0x07;
    const TAG_DURATION: u8 = 0x08;
    const TAG_COLOR: u8 = 0x09;
    const TAG_GRADIENT: u8 = 0x0A;
    const TAG_SHADOW: u8 = 0x0B;
    const TAG_TYPOGRAPHY: u8 = 0x0C;
    const TAG_RESOURCE_REF: u8 = 0x0D;

    /// 이 값의 안정 type tag.
    pub const fn type_tag(&self) -> u8 {
        match self {
            TypedValue::Boolean(_) => TypedValue::TAG_BOOLEAN,
            TypedValue::Number(_) => TypedValue::TAG_NUMBER,
            TypedValue::String(_) => TypedValue::TAG_STRING,
            TypedValue::Enum(_) => TypedValue::TAG_ENUM,
            TypedValue::Length(_) => TypedValue::TAG_LENGTH,
            TypedValue::Percentage(_) => TypedValue::TAG_PERCENTAGE,
            TypedValue::Angle(_) => TypedValue::TAG_ANGLE,
            TypedValue::Duration(_) => TypedValue::TAG_DURATION,
            TypedValue::Color(_) => TypedValue::TAG_COLOR,
            TypedValue::Gradient(_) => TypedValue::TAG_GRADIENT,
            TypedValue::Shadow(_) => TypedValue::TAG_SHADOW,
            TypedValue::Typography(_) => TypedValue::TAG_TYPOGRAPHY,
            TypedValue::ResourceRef(_) => TypedValue::TAG_RESOURCE_REF,
        }
    }

    /// type tag(1바이트) + payload로 직렬화합니다.
    ///
    /// 레이아웃:
    /// - Boolean: `0x00`/`0x01` 1바이트
    /// - Number/Length/Percentage/Angle/Duration: `FiniteF32` little-endian 4바이트
    /// - String/Color/Gradient/Shadow/Typography: 길이 `u32` big-endian + bytes
    /// - Enum: `enum_type` `u32` BE + `variant` `u32` BE
    /// - ResourceRef: `StableId` 16바이트
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.type_tag());
        match self {
            TypedValue::Boolean(b) => out.push(u8::from(*b)),
            TypedValue::Number(n) => out.extend_from_slice(&n.to_le_bytes()),
            TypedValue::String(s) => write_len_prefixed(&mut out, s.as_bytes()),
            TypedValue::Enum(e) => {
                out.extend_from_slice(&e.enum_type().to_be_bytes());
                out.extend_from_slice(&e.variant().to_be_bytes());
            }
            TypedValue::Length(v) => out.extend_from_slice(&v.raw().to_le_bytes()),
            TypedValue::Percentage(v) => out.extend_from_slice(&v.raw().to_le_bytes()),
            TypedValue::Angle(v) => out.extend_from_slice(&v.raw().to_le_bytes()),
            TypedValue::Duration(v) => out.extend_from_slice(&v.raw().to_le_bytes()),
            TypedValue::Color(c) => out.extend_from_slice(&c.to_bytes()),
            TypedValue::Gradient(v) => write_len_prefixed(&mut out, v.as_bytes()),
            TypedValue::Shadow(v) => write_len_prefixed(&mut out, v.as_bytes()),
            TypedValue::Typography(v) => write_len_prefixed(&mut out, v.as_bytes()),
            TypedValue::ResourceRef(r) => out.extend_from_slice(&r.id().to_bytes()),
        }
        out
    }

    /// [`TypedValue::to_bytes`]의 역. 전체 입력을 정확히 소비하지 않으면(trailing) 거부합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<TypedValue, ValueDecodeError> {
        let mut reader = Reader::new(bytes);
        let tag = reader.take_u8().map_err(|_| ValueDecodeError::Empty)?;
        let value = match tag {
            TypedValue::TAG_BOOLEAN => {
                let raw = reader.take_u8()?;
                let b = match raw {
                    0 => false,
                    1 => true,
                    other => return Err(ValueDecodeError::InvalidBool(other)),
                };
                TypedValue::Boolean(b)
            }
            TypedValue::TAG_NUMBER => TypedValue::Number(reader.take_finite()?),
            TypedValue::TAG_STRING => TypedValue::String(reader.take_string()?),
            TypedValue::TAG_ENUM => {
                let enum_type = reader.take_u32_be()?;
                let variant = reader.take_u32_be()?;
                TypedValue::Enum(EnumValue::new(enum_type, variant))
            }
            TypedValue::TAG_LENGTH => TypedValue::Length(Length::from_raw(reader.take_finite()?)),
            TypedValue::TAG_PERCENTAGE => {
                TypedValue::Percentage(Percentage::from_raw(reader.take_finite()?))
            }
            TypedValue::TAG_ANGLE => TypedValue::Angle(Angle::from_raw(reader.take_finite()?)),
            TypedValue::TAG_DURATION => {
                TypedValue::Duration(Duration::from_raw(reader.take_finite()?))
            }
            TypedValue::TAG_COLOR => {
                let color =
                    Color::from_bytes(reader.take_array()?).map_err(ValueDecodeError::Color)?;
                TypedValue::Color(color)
            }
            TypedValue::TAG_GRADIENT => {
                TypedValue::Gradient(GradientValue::new(reader.take_blob()?))
            }
            TypedValue::TAG_SHADOW => TypedValue::Shadow(ShadowValue::new(reader.take_blob()?)),
            TypedValue::TAG_TYPOGRAPHY => {
                TypedValue::Typography(TypographyValue::new(reader.take_blob()?))
            }
            TypedValue::TAG_RESOURCE_REF => {
                let id = StableId::from_bytes(reader.take_array()?);
                TypedValue::ResourceRef(ResourceRef::new(id))
            }
            other => return Err(ValueDecodeError::UnknownTag(other)),
        };
        reader.finish()?;
        Ok(value)
    }
}

/// 길이(`u32` big-endian) 프리픽스 + bytes를 기록합니다.
fn write_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// `TypedValue` 디코드 오류(III.3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueDecodeError {
    /// 빈 입력(type tag 없음).
    Empty,
    /// 알 수 없는 type tag.
    UnknownTag(u8),
    /// 입력이 payload 도중에 끊김.
    Truncated,
    /// 값을 읽은 뒤 잉여 bytes가 남음.
    TrailingBytes,
    /// float payload가 비유한(NaN·±무한대).
    NonFiniteFloat,
    /// 문자열 payload가 유효한 UTF-8이 아님.
    InvalidUtf8,
    /// Boolean payload가 `0`/`1`이 아님.
    InvalidBool(u8),
    /// 색상 payload 디코드 실패(III.4.1).
    Color(ColorError),
}

impl fmt::Display for ValueDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueDecodeError::Empty => f.write_str("빈 입력(type tag 없음)"),
            ValueDecodeError::UnknownTag(tag) => write!(f, "알 수 없는 type tag: 0x{tag:02X}"),
            ValueDecodeError::Truncated => f.write_str("payload 도중에 입력이 끊김"),
            ValueDecodeError::TrailingBytes => f.write_str("값을 읽은 뒤 잉여 bytes가 남음"),
            ValueDecodeError::NonFiniteFloat => f.write_str("float payload가 유한하지 않음"),
            ValueDecodeError::InvalidUtf8 => f.write_str("문자열 payload가 유효한 UTF-8이 아님"),
            ValueDecodeError::InvalidBool(byte) => {
                write!(f, "Boolean payload가 0/1이 아님: 0x{byte:02X}")
            }
            ValueDecodeError::Color(err) => write!(f, "색상 payload 디코드 실패: {err}"),
        }
    }
}

impl std::error::Error for ValueDecodeError {}

/// 슬라이스를 앞에서부터 읽는 커서. 경계를 넘으면 `Truncated`.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ValueDecodeError> {
        let end = self.pos.checked_add(n).ok_or(ValueDecodeError::Truncated)?;
        if end > self.bytes.len() {
            return Err(ValueDecodeError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn take_u8(&mut self) -> Result<u8, ValueDecodeError> {
        Ok(self.take(1)?[0])
    }

    fn take_u32_be(&mut self) -> Result<u32, ValueDecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], ValueDecodeError> {
        let b = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(b);
        Ok(out)
    }

    fn take_finite(&mut self) -> Result<FiniteF32, ValueDecodeError> {
        let b = self.take(4)?;
        FiniteF32::from_le_bytes([b[0], b[1], b[2], b[3]])
            .map_err(|_| ValueDecodeError::NonFiniteFloat)
    }

    /// 길이 프리픽스 blob을 소유 `Vec<u8>`로 읽습니다.
    fn take_blob(&mut self) -> Result<Vec<u8>, ValueDecodeError> {
        let len = self.take_u32_be()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    /// 길이 프리픽스 UTF-8 문자열을 읽습니다.
    fn take_string(&mut self) -> Result<String, ValueDecodeError> {
        let len = self.take_u32_be()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| ValueDecodeError::InvalidUtf8)
    }

    fn finish(self) -> Result<(), ValueDecodeError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(ValueDecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorSpace;

    /// 모든 variant를 대표하는 fixture 목록.
    fn sample_values() -> Vec<TypedValue> {
        vec![
            TypedValue::Boolean(true),
            TypedValue::Boolean(false),
            TypedValue::Number(FiniteF32::new(3.25).unwrap()),
            TypedValue::String("café \u{1F600} 한글".to_string()),
            TypedValue::String(String::new()),
            TypedValue::Enum(EnumValue::new(7, 2)),
            TypedValue::Length(Length::new(-12.5).unwrap()),
            TypedValue::Percentage(Percentage::new(50.0).unwrap()),
            TypedValue::Angle(Angle::new(90.0).unwrap()),
            TypedValue::Duration(Duration::new(0.0).unwrap()),
            TypedValue::Color(Color::new(ColorSpace::DisplayP3, 0.1, 0.2, 0.3, 0.5).unwrap()),
            TypedValue::Gradient(GradientValue::from_slice(&[])),
            TypedValue::Shadow(ShadowValue::from_slice(&[9, 8, 7])),
            TypedValue::Typography(TypographyValue::from_slice(&[0xFF])),
            TypedValue::ResourceRef(ResourceRef::new(StableId::new_v7(1234, 5, 6))),
        ]
    }

    #[test]
    fn every_variant_round_trips() {
        for value in sample_values() {
            let bytes = value.to_bytes();
            let decoded = TypedValue::from_bytes(&bytes).expect("round-trip decode");
            assert_eq!(decoded, value, "round-trip 불일치: {value:?}");
            // 첫 바이트는 안정 type tag.
            assert_eq!(bytes[0], value.type_tag());
        }
    }

    #[test]
    fn type_tags_are_stable_golden() {
        // type tag는 저장 포맷 계약입니다. 이 golden이 바뀌면 기존 파일과 호환이 깨집니다.
        let expected: &[(u8, TypedValue)] = &[
            (0x01, TypedValue::Boolean(true)),
            (0x02, TypedValue::Number(FiniteF32::ZERO)),
            (0x03, TypedValue::String(String::new())),
            (0x04, TypedValue::Enum(EnumValue::new(0, 0))),
            (0x05, TypedValue::Length(Length::new(0.0).unwrap())),
            (0x06, TypedValue::Percentage(Percentage::new(0.0).unwrap())),
            (0x07, TypedValue::Angle(Angle::new(0.0).unwrap())),
            (0x08, TypedValue::Duration(Duration::new(0.0).unwrap())),
            (
                0x09,
                TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap()),
            ),
            (0x0A, TypedValue::Gradient(GradientValue::default())),
            (0x0B, TypedValue::Shadow(ShadowValue::default())),
            (0x0C, TypedValue::Typography(TypographyValue::default())),
            (
                0x0D,
                TypedValue::ResourceRef(ResourceRef::new(StableId::NIL)),
            ),
        ];
        for (tag, value) in expected {
            assert_eq!(value.type_tag(), *tag, "type tag 변동: {value:?}");
            assert_eq!(value.to_bytes()[0], *tag);
        }
        // 13개 variant 모두 포함했는지 확인(새 variant 추가 시 golden 갱신 강제).
        assert_eq!(expected.len(), 13);
    }

    #[test]
    fn golden_bytes_for_simple_variants() {
        // tag 안정성을 byte 수준으로 고정한 golden.
        assert_eq!(TypedValue::Boolean(true).to_bytes(), vec![0x01, 0x01]);
        assert_eq!(TypedValue::Boolean(false).to_bytes(), vec![0x01, 0x00]);
        // Enum: tag + enum_type(BE) + variant(BE).
        assert_eq!(
            TypedValue::Enum(EnumValue::new(1, 2)).to_bytes(),
            vec![0x04, 0, 0, 0, 1, 0, 0, 0, 2]
        );
        // ResourceRef(NIL): tag + 16바이트 0.
        let mut expected = vec![0x0D];
        expected.extend_from_slice(&[0u8; 16]);
        assert_eq!(
            TypedValue::ResourceRef(ResourceRef::new(StableId::NIL)).to_bytes(),
            expected
        );
        // 빈 문자열: tag + 길이 0(BE u32).
        assert_eq!(
            TypedValue::String(String::new()).to_bytes(),
            vec![0x03, 0, 0, 0, 0]
        );
    }

    #[test]
    fn rejects_unknown_tag() {
        assert_eq!(
            TypedValue::from_bytes(&[0xFF]),
            Err(ValueDecodeError::UnknownTag(0xFF))
        );
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(TypedValue::from_bytes(&[]), Err(ValueDecodeError::Empty));
    }

    #[test]
    fn rejects_truncated_payload() {
        // Number(0x02)인데 4바이트 float이 부족.
        assert_eq!(
            TypedValue::from_bytes(&[0x02, 0x00, 0x00]),
            Err(ValueDecodeError::Truncated)
        );
        // String 길이 프리픽스가 실제 데이터보다 큼.
        assert_eq!(
            TypedValue::from_bytes(&[0x03, 0, 0, 0, 5, b'a']),
            Err(ValueDecodeError::Truncated)
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = TypedValue::Boolean(true).to_bytes();
        bytes.push(0x00); // 잉여 바이트.
        assert_eq!(
            TypedValue::from_bytes(&bytes),
            Err(ValueDecodeError::TrailingBytes)
        );
    }

    #[test]
    fn rejects_non_finite_float_bytes() {
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            TypedValue::from_bytes(&bytes),
            Err(ValueDecodeError::NonFiniteFloat)
        );
        // 단위 variant도 동일.
        let mut len_bytes = vec![0x05];
        len_bytes.extend_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(
            TypedValue::from_bytes(&len_bytes),
            Err(ValueDecodeError::NonFiniteFloat)
        );
    }

    #[test]
    fn rejects_invalid_utf8() {
        // String tag + 길이 1 + 잘못된 UTF-8 바이트(0xFF).
        assert_eq!(
            TypedValue::from_bytes(&[0x03, 0, 0, 0, 1, 0xFF]),
            Err(ValueDecodeError::InvalidUtf8)
        );
    }

    #[test]
    fn rejects_invalid_bool() {
        assert_eq!(
            TypedValue::from_bytes(&[0x01, 0x02]),
            Err(ValueDecodeError::InvalidBool(0x02))
        );
    }

    #[test]
    fn negative_zero_number_normalizes_on_decode() {
        // -0.0을 직접 직렬화해도 디코드 시 +0.0으로 정규화(FiniteF32 계약).
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&(-0.0f32).to_le_bytes());
        let decoded = TypedValue::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, TypedValue::Number(FiniteF32::ZERO));
    }

    #[test]
    fn opaque_placeholder_preserves_bytes() {
        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let value = TypedValue::Gradient(GradientValue::new(payload.clone()));
        let decoded = TypedValue::from_bytes(&value.to_bytes()).unwrap();
        match decoded {
            TypedValue::Gradient(g) => assert_eq!(g.as_bytes(), payload.as_slice()),
            other => panic!("예상과 다른 variant: {other:?}"),
        }
    }

    #[test]
    fn color_variant_round_trips_with_real_type() {
        let color = Color::new(ColorSpace::DisplayP3, 1.0, 0.0, 0.25, 0.5).unwrap();
        let value = TypedValue::Color(color);
        let bytes = value.to_bytes();
        // tag 0x09 + Color 17바이트 = 18바이트.
        assert_eq!(bytes.len(), 1 + Color::BYTE_LEN);
        assert_eq!(bytes[0], 0x09);
        assert_eq!(TypedValue::from_bytes(&bytes).unwrap(), value);
    }

    #[test]
    fn color_variant_rejects_invalid_payload() {
        // tag 0x09 + 미지 색공간 tag(0x00) + 0으로 채운 16바이트.
        let mut bytes = vec![0x09, 0x00];
        bytes.extend_from_slice(&[0u8; 16]);
        assert_eq!(
            TypedValue::from_bytes(&bytes),
            Err(ValueDecodeError::Color(
                crate::color::ColorError::UnknownColorSpace(0x00)
            ))
        );
    }
}
