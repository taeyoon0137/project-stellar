//! 문서의 Text style 표. Text 노드가 가진 원래 u32 index를 변경하지 않습니다.
//! Typography resource는 stable id로 참조합니다. 구버전 문서는 표가 없어 미해결 상태로 유지합니다.

use crate::text::{
    LineHeight, OpenTypeTag, ParagraphStyle, TextAlignment, TextStyle, WrappingMode,
};
use std::collections::BTreeMap;
use stellar_core_types::{
    color::Color,
    font::{ByteReader, FontFileHash, FontRecordError},
    ids::StableId,
    typed_value::TypographyValue,
    typography::LetterSpacing,
    value::FiniteF32,
};

/// Typography CSS longhand의 영속 PropertyKey. 순서는 Typography::longhand_bytes와 같습니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;
    /// font-family, font-size, font-weight, font-style, font-stretch, line-height, letter-spacing.
    pub const LONGHANDS: [PropertyKey; 7] = [
        PropertyKey(0x0501),
        PropertyKey(0x0502),
        PropertyKey(0x0503),
        PropertyKey(0x0504),
        PropertyKey(0x0505),
        PropertyKey(0x0506),
        PropertyKey(0x0507),
    ];
}

/// 프로젝트 계층에서 검증한 ProjectFontRecord와 AssetRecord의 canonical bytes.
/// 원본 파일은 content-addressed asset store에서 공유합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentFontAsset {
    /// face metadata, asset ID 및 사용자가 지정한 license metadata.
    pub font_record: Vec<u8>,
    /// content hash, media type, 크기와 출처 정보.
    pub asset_record: Vec<u8>,
}

/// 문서 전체에서 공유하는 style 표. 알 수 없는 typography bytes도 그대로 보존합니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentTextStyles {
    /// run/문단에 typography가 없을 때 상속할 resource.
    pub default_typography: Option<StableId>,
    /// 선택한 design profile에 없는 색을 추정하지 않도록 기본 color도 기록합니다.
    pub default_fill: Option<Color>,
    /// resource ID → canonical TypedValue::Typography의 내부 bytes.
    pub typography: BTreeMap<StableId, TypographyValue>,
    /// 명시적으로 등록한 project fonts. 기기 설치 경로를 저장하지 않습니다.
    pub fonts: BTreeMap<FontFileHash, DocumentFontAsset>,
    /// TextPayload paragraph.style → paragraph style.
    pub paragraphs: BTreeMap<u32, ParagraphStyle>,
    /// TextPayload style_run.style → character style.
    pub runs: BTreeMap<u32, TextStyle>,
}

impl DocumentTextStyles {
    /// 새 편집 문서의 0번 문단/문자와 CSS initial을 연결합니다.
    /// 글꼴 family와 medium 크기는 design profile이나 작성자가 정할 때까지 미해결입니다.
    pub fn initial(resource: StableId) -> Self {
        let value =
            TypographyValue::from_record(&stellar_core_types::typography::Typography::default())
                .expect("CSS initial typography");
        Self {
            default_typography: Some(resource),
            typography: BTreeMap::from([(resource, value)]),
            paragraphs: BTreeMap::from([(0, ParagraphStyle::default())]),
            runs: BTreeMap::from([(0, TextStyle::new(None, None, vec![], vec![]))]),
            ..Self::default()
        }
    }

    /// 저장된 style 정보가 전혀 없는 기존 정의인지.
    pub fn is_empty(&self) -> bool {
        self.default_typography.is_none()
            && self.default_fill.is_none()
            && self.typography.is_empty()
            && self.fonts.is_empty()
            && self.runs.is_empty()
            && self.paragraphs.is_empty()
    }

    /// 명시된 index의 표 항목 누락은 다른 style로 대체하지 않습니다.
    pub fn resolve(&self, paragraph: u32, run: Option<u32>) -> Option<ResolvedTextStyle<'_>> {
        let paragraph = self.paragraphs.get(&paragraph)?;
        let run = match run {
            Some(id) => Some(self.runs.get(&id)?),
            None => None,
        };
        let id = run
            .and_then(|r| r.typography)
            .or(paragraph.typography)
            .or(self.default_typography)?;
        Some(ResolvedTextStyle {
            typography: self.typography.get(&id)?,
            fill: run.and_then(|r| r.fill).or(self.default_fill),
            run,
            paragraph,
        })
    }
    /// canonical STTS version 1. 순서는 ID/index 오름차순입니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = b"STTS\x01".to_vec();
        put_id(&mut out, self.default_typography);
        put_color(&mut out, self.default_fill);
        count(&mut out, self.typography.len());
        for (id, v) in &self.typography {
            out.extend(id.to_bytes());
            count(&mut out, v.as_bytes().len());
            out.extend(v.as_bytes());
        }
        count(&mut out, self.fonts.len());
        for (hash, asset) in &self.fonts {
            out.extend(hash.sha256_digest());
            for record in [&asset.font_record, &asset.asset_record] {
                count(&mut out, record.len());
                out.extend(record);
            }
        }
        count(&mut out, self.paragraphs.len());
        for (id, p) in &self.paragraphs {
            out.extend(id.to_le_bytes());
            put_id(&mut out, p.typography);
            out.push(p.alignment as u8);
            out.push(p.wrapping as u8);
            match p.line_height {
                LineHeight::Auto => out.push(0),
                LineHeight::Multiplier(v) => {
                    out.push(1);
                    out.extend(v.to_le_bytes())
                }
                LineHeight::Fixed(v) => {
                    out.push(2);
                    out.extend(v.to_le_bytes())
                }
            }
        }
        count(&mut out, self.runs.len());
        for (id, r) in &self.runs {
            out.extend(id.to_le_bytes());
            put_id(&mut out, r.typography);
            put_color(&mut out, r.fill);
            match r.letter_spacing {
                None => out.push(0),
                Some(LetterSpacing::Normal) => out.push(1),
                Some(LetterSpacing::Px(v)) => {
                    out.push(2);
                    out.extend(v.to_le_bytes())
                }
                Some(LetterSpacing::Em(v)) => {
                    out.push(3);
                    out.extend(v.to_le_bytes())
                }
            }
            count(&mut out, r.features().len());
            for (tag, value) in r.features() {
                out.extend(tag.bytes());
                out.extend(value.to_le_bytes());
            }
            count(&mut out, r.variations().len());
            for (tag, value) in r.variations() {
                out.extend(tag.bytes());
                out.extend(value.to_le_bytes());
            }
        }
        out
    }

    /// 잘못된 형식, 중복/역순 entry, 알 수 없는 버전은 해석하지 않습니다.
    pub fn decode(bytes: &[u8]) -> Result<Self, StyleTableError> {
        let mut r = ByteReader::new(bytes);
        if &r.take_array::<5>()? != b"STTS\x01" {
            return Err(StyleTableError);
        }
        let default_typography = id(&mut r)?;
        let default_fill = color(&mut r)?;
        let mut typography = BTreeMap::new();
        for _ in 0..r.take_u32()? {
            let id = StableId::from_bytes(r.take_array()?);
            let n = r.take_u32()? as usize;
            typography.insert(id, TypographyValue::from_slice(r.take(n)?));
        }
        let mut fonts = BTreeMap::new();
        for _ in 0..r.take_u32()? {
            let hash = FontFileHash::from_sha256_digest(r.take_array()?);
            let mut record = || -> Result<Vec<u8>, StyleTableError> {
                let n = r.take_u32()? as usize;
                Ok(r.take(n)?.to_vec())
            };
            fonts.insert(
                hash,
                DocumentFontAsset {
                    font_record: record()?,
                    asset_record: record()?,
                },
            );
        }
        let mut paragraphs = BTreeMap::new();
        for _ in 0..r.take_u32()? {
            let index = r.take_u32()?;
            let typography = id(&mut r)?;
            let alignment = match r.take_u8()? {
                0 => TextAlignment::Start,
                1 => TextAlignment::Center,
                2 => TextAlignment::End,
                3 => TextAlignment::Justify,
                _ => return Err(StyleTableError),
            };
            let wrapping = match r.take_u8()? {
                0 => WrappingMode::Wrap,
                1 => WrappingMode::NoWrap,
                _ => return Err(StyleTableError),
            };
            let line_height = match r.take_u8()? {
                0 => LineHeight::Auto,
                1 => LineHeight::Multiplier(finite(&mut r)?),
                2 => LineHeight::Fixed(finite(&mut r)?),
                _ => return Err(StyleTableError),
            };
            let p = ParagraphStyle {
                typography,
                alignment,
                wrapping,
                line_height,
            };
            p.validate().map_err(|_| StyleTableError)?;
            paragraphs.insert(index, p);
        }
        let mut runs = BTreeMap::new();
        for _ in 0..r.take_u32()? {
            let index = r.take_u32()?;
            let typography = id(&mut r)?;
            let fill = color(&mut r)?;
            let spacing = match r.take_u8()? {
                0 => None,
                1 => Some(LetterSpacing::Normal),
                2 => Some(LetterSpacing::Px(finite(&mut r)?)),
                3 => Some(LetterSpacing::Em(finite(&mut r)?)),
                _ => return Err(StyleTableError),
            };
            let mut features = Vec::new();
            for _ in 0..r.take_u32()? {
                features.push((tag(&mut r)?, r.take_u32()?));
            }
            let mut axes = Vec::new();
            for _ in 0..r.take_u32()? {
                axes.push((tag(&mut r)?, finite(&mut r)?));
            }
            let mut run = TextStyle::new(typography, fill, features, axes);
            run.letter_spacing = spacing;
            runs.insert(index, run);
        }
        r.finish()?;
        let table = Self {
            default_typography,
            default_fill,
            fonts,
            typography,
            paragraphs,
            runs,
        };
        if table.encode() != bytes {
            return Err(StyleTableError);
        }
        Ok(table)
    }
}

/// 하나의 scalar 구간에 실제로 적용할 문서 값.
pub struct ResolvedTextStyle<'a> {
    /// run → paragraph → 문서 기본 순서로 찾은 값.
    pub typography: &'a TypographyValue,
    /// 실효 글자색.
    pub fill: Option<Color>,
    /// 선택된 run의 features/variation/spacing.
    pub run: Option<&'a TextStyle>,
    /// 문단 설정.
    pub paragraph: &'a ParagraphStyle,
}
/// style table 형식 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleTableError;
impl From<FontRecordError> for StyleTableError {
    fn from(_: FontRecordError) -> Self {
        Self
    }
}
fn count(out: &mut Vec<u8>, n: usize) {
    out.extend((n as u32).to_le_bytes());
}
fn put_id(out: &mut Vec<u8>, v: Option<StableId>) {
    out.push(u8::from(v.is_some()));
    if let Some(id) = v {
        out.extend(id.to_bytes());
    }
}
fn put_color(out: &mut Vec<u8>, v: Option<Color>) {
    out.push(u8::from(v.is_some()));
    if let Some(c) = v {
        out.extend(c.to_bytes());
    }
}
fn id(r: &mut ByteReader<'_>) -> Result<Option<StableId>, StyleTableError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => Ok(Some(StableId::from_bytes(r.take_array()?))),
        _ => Err(StyleTableError),
    }
}
fn color(r: &mut ByteReader<'_>) -> Result<Option<Color>, StyleTableError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => Ok(Some(
            Color::from_bytes(r.take_array()?).map_err(|_| StyleTableError)?,
        )),
        _ => Err(StyleTableError),
    }
}
fn finite(r: &mut ByteReader<'_>) -> Result<FiniteF32, StyleTableError> {
    FiniteF32::from_le_bytes(r.take_array()?).map_err(|_| StyleTableError)
}
fn tag(r: &mut ByteReader<'_>) -> Result<OpenTypeTag, StyleTableError> {
    OpenTypeTag::new(r.take_array()?).ok_or(StyleTableError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::{color::ColorSpace, typography::Typography};

    #[test]
    fn sparse_indexes_and_opaque_resources_survive_and_resolve_in_order() {
        let ids = [1, 2, 3].map(|n| StableId::new_v7(n, 0, n));
        let mut table = DocumentTextStyles::initial(ids[0]);
        table
            .typography
            .insert(ids[1], TypographyValue::new(vec![8, 9]));
        table
            .typography
            .insert(ids[2], TypographyValue::new(vec![9, 10]));
        table.default_fill = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).ok();
        let red = Color::new(ColorSpace::DisplayP3, 1., 0., 0., 0.5).unwrap();
        table.paragraphs.insert(
            u32::MAX,
            ParagraphStyle {
                typography: Some(ids[1]),
                ..Default::default()
            },
        );
        let mut style = TextStyle::new(
            Some(ids[2]),
            Some(red),
            vec![(OpenTypeTag::new(*b"kern").unwrap(), 0)],
            vec![],
        );
        style.letter_spacing = Some(LetterSpacing::Px(FiniteF32::ZERO));
        table.runs.insert(9, style);
        assert_eq!(
            table
                .resolve(0, Some(0))
                .unwrap()
                .typography
                .record()
                .unwrap(),
            Typography::default()
        );
        assert_eq!(
            table.resolve(u32::MAX, None).unwrap().typography,
            &table.typography[&ids[1]]
        );
        let run = table.resolve(u32::MAX, Some(9)).unwrap();
        assert_eq!(run.typography.as_bytes(), &[9, 10]);
        assert_eq!(run.fill, Some(red));
        assert!(run.typography.record().is_err());
        assert!(table.resolve(4, None).is_none());
        assert!(table.resolve(0, Some(5)).is_none());
        let bytes = table.encode();
        assert_eq!(DocumentTextStyles::decode(&bytes).unwrap(), table);
        for end in 0..bytes.len() {
            assert!(DocumentTextStyles::decode(&bytes[..end]).is_err());
        }
        let mut future = bytes;
        future[4] = 2;
        assert!(DocumentTextStyles::decode(&future).is_err());
    }
}
