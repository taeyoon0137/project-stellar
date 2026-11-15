//! CSS longhand를 보존하는 Typography record v1 (ADR-0024, IX.11.11).
//! font-family는 작성자 순서이며, face가 없는 family는 명시적 미해결 참조입니다.
//! CSS line-height는 각 inline box의 ascent/descent에 half-leading을 적용합니다.

use crate::font::{ByteReader, FontFaceRef, FontRecordError};
use crate::value::FiniteF32;
use std::fmt;

/// family 이름과 이 프로젝트에서 해석된 고정 face. name만 보고 installed font로 대체하지 않습니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    /// 인용·escape를 해석한 family 이름.
    pub name: String,
    /// 파일 hash, face index, variation을 지정한 해석 결과.
    pub face: Option<FontFaceRef>,
}

/// CSS font-size. Medium의 px 해석은 design profile에서 정합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FontSize {
    /// CSS initial.
    #[default]
    Medium,
    /// 1/64 logical px 단위. CSS의 0px도 보존합니다.
    Subpixels(u32),
}

/// CSS font-style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FontStyle {
    /// 직립체.
    #[default]
    Normal,
    /// 이탤릭.
    Italic,
    /// -90 ~ 90도 기울기.
    Oblique(FiniteF32),
}

/// CSS line-height. Number는 각 inline box 자신의 font-size에 곱합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineHeight {
    /// font metric 기반.
    #[default]
    Normal,
    /// 단위 없는 비음수 배수.
    Number(FiniteF32),
    /// 1/64 logical px 고정값.
    Subpixels(u32),
}

/// CSS letter-spacing. normal과 명시적 0은 양쪽 정렬 의미가 다를 수 있습니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LetterSpacing {
    /// CSS initial.
    #[default]
    Normal,
    /// 부호 있는 logical px.
    Px(FiniteF32),
    /// 자신의 글꼴 크기에 상대적인 값.
    Em(FiniteF32),
}

/// 새 format으로 해석할 수 없는 payload. 기존 TypedValue envelope는 유지합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypographyError;

impl TypographyError {
    /// 저장/해석 진단 코드.
    pub const CODE: &'static str = "typography-unrecognized";
}

impl fmt::Display for TypographyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::CODE)
    }
}
impl std::error::Error for TypographyError {}
impl From<FontRecordError> for TypographyError {
    fn from(_: FontRecordError) -> Self {
        Self
    }
}

/// Typography에서 관리하는 7개의 longhand. 지정하지 않은 family/medium의 해석에는 profile이 필요합니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typography {
    /// font-family 순서. 비면 UA/profile 본래의 기본 family입니다.
    pub families: Vec<Family>,
    /// font-size.
    pub size: FontSize,
    /// CSS numeric font-weight (1..=1000, normal = 400).
    pub weight: FiniteF32,
    /// font-style.
    pub style: FontStyle,
    /// font-stretch percentage (normal = 100%).
    pub stretch_percent: FiniteF32,
    /// line-height.
    pub line_height: LineHeight,
    /// letter-spacing.
    pub letter_spacing: LetterSpacing,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            families: Vec::new(),
            size: FontSize::Medium,
            weight: FiniteF32::new(400.).unwrap(),
            style: FontStyle::Normal,
            stretch_percent: FiniteF32::new(100.).unwrap(),
            line_height: LineHeight::Normal,
            letter_spacing: LetterSpacing::Normal,
        }
    }
}

const HEADER: &[u8; 5] = b"STTY\x01";
const MAX_FAMILIES: usize = 64;
const MAX_NAME_BYTES: usize = 4096;
// header+고정 longhand 및 family record(이름 길이·flag 5, axis 없는 face 41).
const MAX_RECORD_BYTES: usize = 64 + MAX_FAMILIES * (5 + MAX_NAME_BYTES + 41);

fn scalar(reader: &mut ByteReader<'_>) -> Result<FiniteF32, TypographyError> {
    let raw = reader.take_array::<4>()?;
    let value = FiniteF32::from_le_bytes(raw).map_err(|_| TypographyError)?;
    (value.to_le_bytes() == raw)
        .then_some(value)
        .ok_or(TypographyError)
}

impl Typography {
    /// 값과 길이 상한, empty/NUL family를 검증합니다.
    ///
    /// family의 고정 face는 파일 hash와 face index만 가리킵니다. variable axis 값은 font-weight·font-stretch와
    /// run의 font-variation-settings가 정하므로, face 안에 variation을 넣은 record는 거부합니다.
    pub fn validate(&self) -> Result<(), TypographyError> {
        if self.families.len() > MAX_FAMILIES
            || !(1.0..=1000.0).contains(&self.weight.get())
            || self.stretch_percent.get() < 0.
            || matches!(self.style, FontStyle::Oblique(v) if v.get().abs() > 90.)
            || matches!(self.line_height, LineHeight::Number(v) if v.get() < 0.)
            || self.families.iter().any(|f| {
                f.name.trim().is_empty()
                    || f.name.len() > MAX_NAME_BYTES
                    || f.name.contains('\0')
                    || f.face
                        .as_ref()
                        .is_some_and(|face| !face.variations().is_empty())
            })
        {
            return Err(TypographyError);
        }
        Ok(())
    }

    /// v1 bytes. 정수·float는 LE이고 각 enum은 고정 u8 discriminator를 사용합니다.
    pub fn encode(&self) -> Result<Vec<u8>, TypographyError> {
        let mut out = HEADER.to_vec();
        for bytes in self.longhand_bytes()? {
            out.extend(bytes);
        }
        Ok(out)
    }

    /// font-family, size, weight, style, stretch, line-height, letter-spacing 순서의 canonical 값.
    pub fn longhand_bytes(&self) -> Result<[Vec<u8>; 7], TypographyError> {
        self.validate()?;
        let mut pieces: [Vec<u8>; 7] = Default::default();
        let out = &mut pieces[0];
        out.extend_from_slice(&(self.families.len() as u32).to_le_bytes());
        for family in &self.families {
            out.extend_from_slice(&(family.name.len() as u32).to_le_bytes());
            out.extend_from_slice(family.name.as_bytes());
            out.push(u8::from(family.face.is_some()));
            if let Some(face) = &family.face {
                face.encode_into(out);
            }
        }
        let out = &mut pieces[1];
        match self.size {
            FontSize::Medium => out.push(0),
            FontSize::Subpixels(n) => {
                out.push(1);
                out.extend(n.to_le_bytes());
            }
        }
        let out = &mut pieces[2];
        out.extend(self.weight.to_le_bytes());
        let out = &mut pieces[3];
        match self.style {
            FontStyle::Normal => out.push(0),
            FontStyle::Italic => out.push(1),
            FontStyle::Oblique(v) => {
                out.push(2);
                out.extend(v.to_le_bytes());
            }
        }
        let out = &mut pieces[4];
        out.extend(self.stretch_percent.to_le_bytes());
        let out = &mut pieces[5];
        match self.line_height {
            LineHeight::Normal => out.push(0),
            LineHeight::Number(v) => {
                out.push(1);
                out.extend(v.to_le_bytes());
            }
            LineHeight::Subpixels(n) => {
                out.push(2);
                out.extend(n.to_le_bytes());
            }
        }
        let out = &mut pieces[6];
        match self.letter_spacing {
            LetterSpacing::Normal => out.push(0),
            LetterSpacing::Px(v) => {
                out.push(1);
                out.extend(v.to_le_bytes());
            }
            LetterSpacing::Em(v) => {
                out.push(2);
                out.extend(v.to_le_bytes());
            }
        }
        Ok(pieces)
    }

    /// Semantic merge에서 독립적으로 편집된 longhand를 같은 versioned decoder로 검증합니다.
    pub fn from_longhand_bytes(fields: &[Vec<u8>; 7]) -> Result<Self, TypographyError> {
        if fields.iter().map(Vec::len).sum::<usize>() > MAX_RECORD_BYTES {
            return Err(TypographyError);
        }
        let mut merged = HEADER.to_vec();
        for field in fields {
            merged.extend(field);
        }
        let record = Self::decode(&merged)?;
        if &record.longhand_bytes()? != fields {
            return Err(TypographyError);
        }
        Ok(record)
    }

    /// 현재 버전의 canonical bytes만 해석합니다.
    pub fn decode(bytes: &[u8]) -> Result<Self, TypographyError> {
        // 최대 크기를 먼저 검사해 face의 axis count도 bounded input에서만 읽습니다. axis가 있는 face는
        // decode 뒤 validate가 거부합니다.
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(TypographyError);
        }
        let mut r = ByteReader::new(bytes);
        if &r.take_array::<5>()? != HEADER {
            return Err(TypographyError);
        }
        let n = r.take_u32()? as usize;
        if n > MAX_FAMILIES {
            return Err(TypographyError);
        }
        let mut families = Vec::with_capacity(n);
        for _ in 0..n {
            let size = r.take_u32()? as usize;
            if size > MAX_NAME_BYTES {
                return Err(TypographyError);
            }
            let name = std::str::from_utf8(r.take(size)?)
                .map_err(|_| TypographyError)?
                .to_owned();
            let face = match r.take_u8()? {
                0 => None,
                1 => Some(FontFaceRef::decode_from(&mut r)?),
                _ => return Err(TypographyError),
            };
            families.push(Family { name, face });
        }
        let size = match r.take_u8()? {
            0 => FontSize::Medium,
            1 => FontSize::Subpixels(r.take_u32()?),
            _ => return Err(TypographyError),
        };
        let weight = scalar(&mut r)?;
        let style = match r.take_u8()? {
            0 => FontStyle::Normal,
            1 => FontStyle::Italic,
            2 => FontStyle::Oblique(scalar(&mut r)?),
            _ => return Err(TypographyError),
        };
        let stretch_percent = scalar(&mut r)?;
        let line_height = match r.take_u8()? {
            0 => LineHeight::Normal,
            1 => LineHeight::Number(scalar(&mut r)?),
            2 => LineHeight::Subpixels(r.take_u32()?),
            _ => return Err(TypographyError),
        };
        let letter_spacing = match r.take_u8()? {
            0 => LetterSpacing::Normal,
            1 => LetterSpacing::Px(scalar(&mut r)?),
            2 => LetterSpacing::Em(scalar(&mut r)?),
            _ => return Err(TypographyError),
        };
        r.finish()?;
        let value = Self {
            families,
            size,
            weight,
            style,
            stretch_percent,
            line_height,
            letter_spacing,
        };
        value.validate()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FontFileHash, FontSource};
    use crate::typed_value::{TypedValue, TypographyValue};
    #[test]
    fn pinned_families_and_initial_keywords_round_trip_without_system_lookup() {
        let initial = Typography::default();
        assert_eq!(initial.size, FontSize::Medium);
        assert!(initial.families.is_empty());
        assert_eq!(
            (initial.weight.get(), initial.stretch_percent.get()),
            (400., 100.)
        );
        assert_eq!(
            Typography::decode(&initial.encode().unwrap()).unwrap(),
            initial
        );
        let record = Typography {
            families: vec![
                Family {
                    name: "Missing".into(),
                    face: None,
                },
                Family {
                    name: "한글".into(),
                    face: Some(FontFaceRef::new(
                        FontSource::Project,
                        FontFileHash::from_sha256_digest([4; 32]),
                        2,
                        vec![],
                    )),
                },
            ],
            size: FontSize::Subpixels(1024),
            style: FontStyle::Oblique(FiniteF32::new(-12.).unwrap()),
            line_height: LineHeight::Number(FiniteF32::new(1.4).unwrap()),
            letter_spacing: LetterSpacing::Em(FiniteF32::new(-0.01).unwrap()),
            ..initial
        };
        let typed = TypographyValue::from_record(&record).unwrap();
        let envelope = TypedValue::Typography(typed.clone()).to_bytes();
        assert_eq!(envelope[0], 0x0C);
        assert_eq!(
            TypedValue::from_bytes(&envelope).unwrap(),
            TypedValue::Typography(typed.clone())
        );
        assert_eq!(typed.record().unwrap(), record);
        let mut bad = typed.as_bytes().to_vec();
        bad[4] = 2;
        for blob in [bad, vec![10, 9, 8], vec![0; 20]] {
            let opaque = TypographyValue::new(blob.clone());
            assert_eq!(
                opaque.record().unwrap_err().to_string(),
                "typography-unrecognized"
            );
            assert_eq!(
                TypedValue::from_bytes(&TypedValue::Typography(opaque.clone()).to_bytes()).unwrap(),
                TypedValue::Typography(opaque)
            );
        }
        for len in 0..typed.as_bytes().len() {
            assert!(Typography::decode(&typed.as_bytes()[..len]).is_err());
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn stty_v1_golden_bytes() {
        // 저장 형식 계약입니다. 이 golden이 바뀌면 기존 Typography 값과 호환이 깨집니다.
        assert_eq!(
            hex(&Typography::default().encode().unwrap()),
            "5354545901\
             00000000\
             00\
             0000c843\
             00\
             0000c842\
             00\
             00"
        );
        assert_eq!(
            TypographyValue::default().as_bytes(),
            Typography::default().encode().unwrap().as_slice()
        );
        assert_eq!(
            TypographyValue::default().record(),
            Ok(Typography::default())
        );
        let record = Typography {
            families: vec![Family {
                name: "A".into(),
                face: Some(FontFaceRef::new(
                    FontSource::Project,
                    FontFileHash::from_sha256_digest([4; 32]),
                    2,
                    vec![],
                )),
            }],
            size: FontSize::Subpixels(1024),
            weight: FiniteF32::new(700.).unwrap(),
            style: FontStyle::Italic,
            stretch_percent: FiniteF32::new(100.).unwrap(),
            line_height: LineHeight::Number(FiniteF32::new(1.5).unwrap()),
            letter_spacing: LetterSpacing::Px(FiniteF32::new(-0.5).unwrap()),
        };
        assert_eq!(
            hex(&record.encode().unwrap()),
            format!(
                "5354545901\
                 01000000 01000000 41 01 01{} 02000000 00000000\
                 0100040000\
                 00002f44\
                 01\
                 0000c842\
                 010000c03f\
                 01000000bf",
                "04".repeat(32)
            )
            .replace(' ', "")
        );
    }

    #[test]
    fn pinned_face_with_variation_axes_is_rejected() {
        let face = FontFaceRef::new(
            FontSource::Project,
            FontFileHash::from_sha256_digest([4; 32]),
            0,
            vec![crate::font::FontVariation {
                tag: crate::font::OpenTypeTag::parse("wght").unwrap(),
                value: FiniteF32::new(700.).unwrap(),
            }],
        );
        let record = Typography {
            families: vec![Family {
                name: "Variable".into(),
                face: Some(face),
            }],
            ..Default::default()
        };
        assert_eq!(record.encode(), Err(TypographyError));
        // 같은 bytes를 직접 만들어도 decode가 거부합니다.
        let mut bytes = Typography::default().encode().unwrap();
        let mut family = Vec::new();
        family.extend(1u32.to_le_bytes());
        family.extend(8u32.to_le_bytes());
        family.extend(b"Variable");
        family.push(1);
        record.families[0]
            .face
            .as_ref()
            .unwrap()
            .encode_into(&mut family);
        bytes.splice(5..9, family);
        assert_eq!(Typography::decode(&bytes), Err(TypographyError));
    }
}
