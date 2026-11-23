//! 문서 Text style 표 v4(ADR-0077 결정 2).
//!
//! 문서 하나의 표 전체를 씁니다: 기본 typography·글자색, typography resource(CSS longhand 일곱 개), 문단 style,
//! 문자 style(OpenType feature·variation 포함), 문서가 등록한 글꼴 file hash 목록. 다중 파일 원본에서는 asset
//! catalog가 이 블록의 원본이고, 단일 View 원본은 같은 문법을 문서에 둡니다. 숨은 기본값은 없습니다.

use std::collections::BTreeMap;

use stellar_core_types::color::Color;
use stellar_core_types::font::FontFileHash;
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypographyValue;
use stellar_core_types::value::FiniteF32;
use stellar_document::text::{OpenTypeTag, TextStyle};
use stellar_document::text_styles::{DocumentFontAsset, DocumentTextStyles};

use super::super::text_codec::document_font::{
    letter_spacing_text, paragraph, parse_letter_spacing_text, parse_paragraph, parse_typography,
    typography,
};
use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::css::{self, invalid, unsupported};
use crate::yaml::YamlValue;

fn line(value: impl Into<String>) -> YamlValue {
    YamlValue::scalar(value.into())
}

fn get<'a>(value: &'a YamlValue, key: &str) -> Result<&'a str, TextSourceError> {
    text(field(value, key)?)
}

fn sequence<'a>(value: &'a YamlValue, key: &str) -> Result<&'a [YamlValue], TextSourceError> {
    field(value, key)?
        .as_sequence()
        .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
}

fn color_or_inherit(value: Option<Color>) -> YamlValue {
    line(value.map_or_else(|| "inherit".to_owned(), css::color))
}

fn parse_color_or_inherit(value: &str) -> Result<Option<Color>, TextSourceError> {
    if value == "inherit" {
        Ok(None)
    } else {
        css::parse_color(value).map(Some)
    }
}

fn tag_text(tag: OpenTypeTag) -> Result<String, TextSourceError> {
    let bytes = tag.bytes();
    let text = std::str::from_utf8(&bytes).map_err(|_| unsupported("ASCII가 아닌 OpenType tag"))?;
    if text.contains(['"', '\\']) {
        return Err(unsupported("따옴표가 든 OpenType tag"));
    }
    Ok(format!("\"{text}\""))
}

fn parse_tag(text: &str) -> Result<OpenTypeTag, TextSourceError> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| invalid("OpenType tag는 큰따옴표 네 글자입니다"))?;
    let bytes: [u8; 4] = inner
        .as_bytes()
        .try_into()
        .map_err(|_| invalid("OpenType tag는 네 글자입니다"))?;
    OpenTypeTag::new(bytes).ok_or_else(|| invalid("OpenType tag 형식"))
}

/// CSS `font-feature-settings`: `normal` 또는 `"liga" 1, "kern" 0`.
fn features(items: &[(OpenTypeTag, u32)]) -> Result<String, TextSourceError> {
    if items.is_empty() {
        return Ok("normal".into());
    }
    Ok(items
        .iter()
        .map(|(tag, value)| Ok(format!("{} {value}", tag_text(*tag)?)))
        .collect::<Result<Vec<_>, TextSourceError>>()?
        .join(", "))
}

fn parse_features(text: &str) -> Result<Vec<(OpenTypeTag, u32)>, TextSourceError> {
    if text == "normal" {
        return Ok(Vec::new());
    }
    text.split(", ")
        .map(|item| {
            let (tag, value) = item
                .rsplit_once(' ')
                .ok_or_else(|| invalid("feature는 \"tag\" 값입니다"))?;
            Ok((parse_tag(tag)?, css::parse_u32(value)?))
        })
        .collect()
}

/// CSS `font-variation-settings`: `normal` 또는 `"wght" 400`.
fn variations(items: &[(OpenTypeTag, FiniteF32)]) -> Result<String, TextSourceError> {
    if items.is_empty() {
        return Ok("normal".into());
    }
    Ok(items
        .iter()
        .map(|(tag, value)| Ok(format!("{} {}", tag_text(*tag)?, css::number(value.get()))))
        .collect::<Result<Vec<_>, TextSourceError>>()?
        .join(", "))
}

fn parse_variations(text: &str) -> Result<Vec<(OpenTypeTag, FiniteF32)>, TextSourceError> {
    if text == "normal" {
        return Ok(Vec::new());
    }
    text.split(", ")
        .map(|item| {
            let (tag, value) = item
                .rsplit_once(' ')
                .ok_or_else(|| invalid("variation은 \"tag\" 값입니다"))?;
            Ok((
                parse_tag(tag)?,
                FiniteF32::new(css::parse_number(value)?).map_err(invalid)?,
            ))
        })
        .collect()
}

fn run(index: u32, style: &TextStyle) -> Result<YamlValue, TextSourceError> {
    Ok(mapping([
        ("index", line(index.to_string())),
        ("color", color_or_inherit(style.fill)),
        ("font-feature-settings", line(features(style.features())?)),
        (
            "font-variation-settings",
            line(variations(style.variations())?),
        ),
        (
            "letter-spacing",
            line(
                style
                    .letter_spacing
                    .map_or_else(|| "inherit".to_owned(), letter_spacing_text),
            ),
        ),
        ("typography", line(css::optional_id(style.typography))),
    ]))
}

fn parse_run(value: &YamlValue) -> Result<(u32, TextStyle), TextSourceError> {
    fields(
        value,
        &[
            "index",
            "color",
            "font-feature-settings",
            "font-variation-settings",
            "letter-spacing",
            "typography",
        ],
    )?;
    let features = parse_features(get(value, "font-feature-settings")?)?;
    let variations = parse_variations(get(value, "font-variation-settings")?)?;
    let mut style = TextStyle::new(
        css::parse_optional_id(get(value, "typography")?)?,
        parse_color_or_inherit(get(value, "color")?)?,
        features.clone(),
        variations.clone(),
    );
    // 정렬·중복 제거된 canonical 목록만 받습니다(결정론적 bytes).
    if style.features() != features.as_slice() || style.variations() != variations.as_slice() {
        return Err(invalid(
            "feature·variation은 tag 오름차순, 중복 없이 씁니다",
        ));
    }
    style.letter_spacing = match get(value, "letter-spacing")? {
        "inherit" => None,
        other => Some(parse_letter_spacing_text(other)?),
    };
    Ok((css::parse_u32(get(value, "index")?)?, style))
}

/// 문서 하나의 표를 씁니다.
pub(crate) fn text_styles_value(
    document: StableId,
    styles: &DocumentTextStyles,
) -> Result<YamlValue, TextSourceError> {
    let typography = styles
        .typography
        .iter()
        .map(|(id, value)| {
            let record = value
                .record()
                .map_err(|_| unsupported("해석할 수 없는 typography bytes"))?;
            let encoded = typography(*id, &record)?;
            // 알 수 없는 bytes를 보존한 값은 record로 다시 만들면 달라지므로 거부합니다.
            if &TypographyValue::from_record(&record).map_err(invalid)? != value {
                return Err(unsupported("typography bytes를 정확히 복원할 수 없습니다"));
            }
            Ok(encoded)
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let paragraphs = styles
        .paragraphs
        .iter()
        .map(|(index, style)| {
            let YamlValue::Mapping(mut fields) = paragraph(style) else {
                unreachable!("mapping");
            };
            fields.insert(0, (scalar("index"), line(index.to_string())));
            YamlValue::Mapping(fields)
        })
        .collect();
    let runs = styles
        .runs
        .iter()
        .map(|(index, style)| run(*index, style))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(mapping([
        ("document", line(css::id(document))),
        ("color", color_or_inherit(styles.default_fill)),
        (
            "default-typography",
            line(css::optional_id(styles.default_typography)),
        ),
        (
            "fonts",
            YamlValue::Sequence(styles.fonts.keys().map(|h| line(h.to_string())).collect()),
        ),
        ("typography", YamlValue::Sequence(typography)),
        ("paragraphs", YamlValue::Sequence(paragraphs)),
        ("runs", YamlValue::Sequence(runs)),
    ]))
}

/// [`text_styles_value`]를 읽습니다. 글꼴 file hash는 `fonts`에서 찾습니다.
pub(crate) fn parse_text_styles_value(
    value: &YamlValue,
    fonts: &BTreeMap<FontFileHash, DocumentFontAsset>,
) -> Result<(StableId, DocumentTextStyles), TextSourceError> {
    fields(
        value,
        &[
            "document",
            "color",
            "default-typography",
            "fonts",
            "typography",
            "paragraphs",
            "runs",
        ],
    )?;
    let mut styles = DocumentTextStyles {
        default_typography: css::parse_optional_id(get(value, "default-typography")?)?,
        default_fill: parse_color_or_inherit(get(value, "color")?)?,
        ..DocumentTextStyles::default()
    };
    for item in sequence(value, "fonts")? {
        let hash: FontFileHash = text(item)?
            .parse()
            .map_err(|_| invalid("글꼴 file hash 형식"))?;
        let asset = fonts
            .get(&hash)
            .ok_or_else(|| invalid(format!("catalog에 없는 글꼴 {hash}")))?;
        if styles.fonts.insert(hash, asset.clone()).is_some() {
            return Err(invalid("중복 글꼴 참조"));
        }
    }
    for item in sequence(value, "typography")? {
        let (id, record) = parse_typography(item)?;
        let value = TypographyValue::from_record(&record).map_err(invalid)?;
        if styles.typography.insert(id, value).is_some() {
            return Err(invalid("중복 typography resource"));
        }
    }
    for item in sequence(value, "paragraphs")? {
        let mut keys: Vec<(YamlValue, YamlValue)> = item
            .as_mapping()
            .ok_or_else(|| invalid("문단 style은 mapping입니다"))?
            .to_vec();
        let index = match keys.first() {
            Some((key, index)) if key.as_scalar() == Some("index") => css::parse_u32(text(index)?)?,
            _ => return Err(invalid("문단 style의 첫 키는 index입니다")),
        };
        keys.remove(0);
        let style = parse_paragraph(&YamlValue::Mapping(keys))?;
        if styles.paragraphs.insert(index, style).is_some() {
            return Err(invalid("중복 문단 style index"));
        }
    }
    for item in sequence(value, "runs")? {
        let (index, style) = parse_run(item)?;
        if styles.runs.insert(index, style).is_some() {
            return Err(invalid("중복 문자 style index"));
        }
    }
    Ok((css::parse_id(get(value, "document")?)?, styles))
}
