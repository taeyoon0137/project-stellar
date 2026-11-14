//! `TypedValue` tagged union (III.3.5).
//!
//! 표현식·토큰·속성 값을 담는 타입드 값 union입니다. 각 variant는 **안정적인 numeric
//! type tag**를 가지며, 이 tag는 저장 포맷의 일부이므로 한 번 정한 뒤 절대 바꾸지 않습니다.
//! 모든 variant는 [`TypedValue::to_bytes`] ↔ [`TypedValue::from_bytes`]로 정확히 round-trip
//! 합니다.
//!
//! Typography는 인식 가능한 버전만 구조화해 읽고 기존 blob을 그대로 보존합니다.

use std::fmt;

use crate::color::{Color, ColorError};
use crate::gradient::{Gradient, GradientError};
use crate::ids::StableId;
use crate::shadow::{Shadow, ShadowError};
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

/// Typography type tag의 원본 bytes. 알 수 없는 기존 형식은 읽기와 저장 동안 그대로 유지합니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TypographyValue(Vec<u8>);

impl TypographyValue {
    /// 기존 또는 새 payload envelope의 원본 bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    /// bytes를 복사해 보존합니다.
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }
    /// 원본 payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    /// 검증된 longhand로부터 canonical payload를 만듭니다.
    pub fn from_record(
        record: &crate::typography::Typography,
    ) -> Result<Self, crate::typography::TypographyError> {
        Ok(Self::new(record.encode()?))
    }
    /// 인식하는 형식만 해석합니다. 실패해도 원본 bytes는 유지합니다.
    pub fn record(
        &self,
    ) -> Result<crate::typography::Typography, crate::typography::TypographyError> {
        crate::typography::Typography::decode(self.as_bytes())
    }
}

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
    /// 그라디언트(III.4.3).
    Gradient(Gradient),
    /// 그림자(III.4.4).
    Shadow(Shadow),
    /// 타이포그래피 원본(IX.11.11).
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
            TypedValue::Gradient(g) => write_len_prefixed(&mut out, &g.to_bytes()),
            TypedValue::Shadow(s) => out.extend_from_slice(&s.to_bytes()),
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
                let blob = reader.take_blob()?;
                let gradient = Gradient::from_bytes(&blob).map_err(ValueDecodeError::Gradient)?;
                TypedValue::Gradient(gradient)
            }
            TypedValue::TAG_SHADOW => {
                let shadow =
                    Shadow::from_bytes(reader.take_array()?).map_err(ValueDecodeError::Shadow)?;
                TypedValue::Shadow(shadow)
            }
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
    /// 그라디언트 payload 디코드 실패(III.4.3).
    Gradient(GradientError),
    /// 그림자 payload 디코드 실패(III.4.4).
    Shadow(ShadowError),
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
            ValueDecodeError::Gradient(err) => write!(f, "그라디언트 payload 디코드 실패: {err}"),
            ValueDecodeError::Shadow(err) => write!(f, "그림자 payload 디코드 실패: {err}"),
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
    use crate::gradient::GradientStop;
    use crate::shadow::{BlurRadius, Shadow};
    use crate::value::Vec2;

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
            TypedValue::Gradient(Gradient::new(vec![
                GradientStop::new(
                    StableId::new_v7(1, 0, 1),
                    0.0,
                    Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap(),
                )
                .unwrap(),
                GradientStop::new(
                    StableId::new_v7(2, 0, 2),
                    1.0,
                    Color::new(ColorSpace::Srgb, 0.0, 0.0, 1.0, 1.0).unwrap(),
                )
                .unwrap(),
            ])),
            TypedValue::Gradient(Gradient::default()),
            TypedValue::Shadow(Shadow::new(
                Vec2::new(-2.0, 4.0).unwrap(),
                BlurRadius::new(6.0).unwrap(),
                Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.5).unwrap(),
            )),
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
            (0x0A, TypedValue::Gradient(Gradient::default())),
            (
                0x0B,
                TypedValue::Shadow(Shadow::new(
                    Vec2::ZERO,
                    BlurRadius::ZERO,
                    Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap(),
                )),
            ),
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
        let value = TypedValue::Typography(TypographyValue::new(payload.clone()));
        let decoded = TypedValue::from_bytes(&value.to_bytes()).unwrap();
        match decoded {
            TypedValue::Typography(t) => assert_eq!(t.as_bytes(), payload.as_slice()),
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

    #[test]
    fn gradient_variant_rejects_invalid_payload() {
        // tag 0x0A + 길이 4 blob(count=1) — stop bytes가 없어 그라디언트 디코드가 Truncated.
        let bytes = vec![0x0A, 0, 0, 0, 4, 0, 0, 0, 1];
        assert_eq!(
            TypedValue::from_bytes(&bytes),
            Err(ValueDecodeError::Gradient(
                crate::gradient::GradientError::Truncated
            ))
        );
    }

    #[test]
    fn shadow_variant_rejects_negative_blur() {
        // 정상 그림자 직렬화 후 blur 바이트를 음수로 바꾸면 디코드가 거부.
        let shadow = Shadow::new(
            Vec2::ZERO,
            BlurRadius::new(2.0).unwrap(),
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap(),
        );
        let mut bytes = TypedValue::Shadow(shadow).to_bytes();
        // tag(1) + offset(8) 다음의 blur(4바이트)를 음수로 덮어씀.
        bytes[9..13].copy_from_slice(&(-2.0f32).to_le_bytes());
        assert_eq!(
            TypedValue::from_bytes(&bytes),
            Err(ValueDecodeError::Shadow(crate::shadow::ShadowError::Blur(
                crate::shadow::BlurError::Negative
            )))
        );
    }
}
