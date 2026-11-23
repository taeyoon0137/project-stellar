//! View v2의 문서 style 표(`document_font`).
//!
//! 기본 typography resource 하나의 CSS longhand 일곱 개, 기본 글자색, 문단 style 0, 문자 style 0을
//! 모두 명시합니다(ADR-0061 결정 3). 파일에 없는 값을 버전 번호나 앱 기본값에서 복원하지 않습니다.
//! 등록 글꼴의 asset record와 face 참조는 텍스트 asset catalog(결정 6)가 생기기 전까지 거부합니다.
use super::*;
use stellar_core_types::typed_value::TypographyValue;
use stellar_core_types::typography::{
    Family, FontSize, FontStyle, LetterSpacing, LineHeight, Typography,
};
use stellar_core_types::value::FiniteF32;
use stellar_document::text::{
    LineHeight as ParagraphLineHeight, ParagraphStyle, TextAlignment, TextStyle, WrappingMode,
};

fn unsupported(what: &str) -> TextSourceError {
    TextSourceError::Unsupported(format!("문서 style 표: {what}"))
}

fn px(subpixels: u32) -> String {
    format!("{}px", f64::from(subpixels) / 64.0)
}

/// CSS px를 문서의 1/64 px 정수로 정확히 변환합니다. 실수 반올림으로 다른 입력을 허용하지 않습니다.
fn parse_px(css: &str) -> Result<u32, TextSourceError> {
    let error = || invalid("길이는 1/64 px 단위의 비음수 px여야 합니다");
    let px = css.strip_suffix("px").ok_or_else(error)?;
    let (whole, fraction) = px.split_once('.').unwrap_or((px, ""));
    let fraction = fraction.trim_end_matches('0');
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
    {
        return Err(error());
    }
    let denominator = 10u64.pow(fraction.len() as u32);
    let remainder = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| error())? * 64
    };
    if remainder % denominator != 0 {
        return Err(error());
    }
    whole
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(remainder / denominator))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(error)
}

fn number(text: &str) -> Result<FiniteF32, TextSourceError> {
    let value: f32 = text
        .parse()
        .map_err(|_| invalid("숫자 형식이 잘못되었습니다"))?;
    FiniteF32::new(value).map_err(invalid)
}

/// CSS `letter-spacing` 값(`normal`, `px`, `em`).
pub(crate) fn letter_spacing_text(value: LetterSpacing) -> String {
    letter_spacing(value)
}

/// [`letter_spacing_text`]를 읽습니다.
pub(crate) fn parse_letter_spacing_text(text: &str) -> Result<LetterSpacing, TextSourceError> {
    parse_letter_spacing(text)
}

fn letter_spacing(value: LetterSpacing) -> String {
    match value {
        LetterSpacing::Normal => "normal".into(),
        LetterSpacing::Px(v) => format!("{}px", v.get()),
        LetterSpacing::Em(v) => format!("{}em", v.get()),
    }
}

fn parse_letter_spacing(text: &str) -> Result<LetterSpacing, TextSourceError> {
    if text == "normal" {
        Ok(LetterSpacing::Normal)
    } else if let Some(v) = text.strip_suffix("px") {
        Ok(LetterSpacing::Px(number(v)?))
    } else if let Some(v) = text.strip_suffix("em") {
        Ok(LetterSpacing::Em(number(v)?))
    } else {
        Err(invalid(
            "letter-spacing은 normal, px, em 중 하나여야 합니다",
        ))
    }
}

/// font-family 한 줄: 작성자 순서의 큰따옴표 이름 목록. 빈 목록은 `none`입니다.
fn families(families: &[Family]) -> Result<String, TextSourceError> {
    if families.is_empty() {
        return Ok("none".into());
    }
    let mut names = Vec::with_capacity(families.len());
    for family in families {
        if family.face.is_some() {
            return Err(unsupported(
                "고정 face 참조는 텍스트 asset catalog의 글꼴 항목이 필요합니다",
            ));
        }
        names.push(format!(
            "\"{}\"",
            family.name.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    Ok(names.join(", "))
}

fn parse_families(text: &str) -> Result<Vec<Family>, TextSourceError> {
    if text == "none" {
        return Ok(Vec::new());
    }
    let error = || invalid("font-family는 큰따옴표 이름을 \", \"로 구분해야 합니다");
    let mut families = Vec::new();
    let mut chars = text.chars();
    loop {
        if chars.next() != Some('"') {
            return Err(error());
        }
        let mut name = String::new();
        loop {
            match chars.next().ok_or_else(error)? {
                '"' => break,
                '\\' => match chars.next() {
                    Some(c @ ('\\' | '"')) => name.push(c),
                    _ => return Err(error()),
                },
                c => name.push(c),
            }
        }
        families.push(Family { name, face: None });
        match (chars.next(), chars.next()) {
            (None, _) => return Ok(families),
            (Some(','), Some(' ')) => {}
            _ => return Err(error()),
        }
    }
}

pub(crate) fn typography(id: StableId, value: &Typography) -> Result<YamlValue, TextSourceError> {
    Ok(mapping([
        ("id", scalar(id)),
        ("font-family", scalar(families(&value.families)?)),
        (
            "font-size",
            scalar(match value.size {
                FontSize::Medium => "medium".into(),
                FontSize::Subpixels(n) => px(n),
            }),
        ),
        (
            "font-stretch",
            scalar(format!("{}%", value.stretch_percent.get())),
        ),
        (
            "font-style",
            scalar(match value.style {
                FontStyle::Normal => "normal".into(),
                FontStyle::Italic => "italic".into(),
                FontStyle::Oblique(angle) => format!("oblique {}deg", angle.get()),
            }),
        ),
        ("font-weight", scalar(value.weight.get())),
        (
            "letter-spacing",
            scalar(letter_spacing(value.letter_spacing)),
        ),
        (
            "line-height",
            scalar(match value.line_height {
                LineHeight::Normal => "normal".into(),
                LineHeight::Number(v) => v.get().to_string(),
                LineHeight::Subpixels(n) => px(n),
            }),
        ),
    ]))
}

pub(crate) fn parse_typography(
    value: &YamlValue,
) -> Result<(StableId, Typography), TextSourceError> {
    fields(
        value,
        &[
            "id",
            "font-family",
            "font-size",
            "font-stretch",
            "font-style",
            "font-weight",
            "letter-spacing",
            "line-height",
        ],
    )?;
    let id: StableId = parse(field(value, "id")?)?;
    if id == StableId::NIL {
        return Err(invalid("typography resource ID가 비어 있습니다"));
    }
    let line = |key| text(field(value, key)?);
    let size = match line("font-size")? {
        "medium" => FontSize::Medium,
        other => FontSize::Subpixels(parse_px(other)?),
    };
    let style = match line("font-style")? {
        "normal" => FontStyle::Normal,
        "italic" => FontStyle::Italic,
        other => FontStyle::Oblique(number(
            other
                .strip_prefix("oblique ")
                .and_then(|v| v.strip_suffix("deg"))
                .ok_or_else(|| invalid("font-style 형식이 잘못되었습니다"))?,
        )?),
    };
    let stretch = line("font-stretch")?
        .strip_suffix('%')
        .ok_or_else(|| invalid("font-stretch는 백분율이어야 합니다"))?;
    let line_height = match line("line-height")? {
        "normal" => LineHeight::Normal,
        other if other.ends_with("px") => LineHeight::Subpixels(parse_px(other)?),
        other => LineHeight::Number(number(other)?),
    };
    let record = Typography {
        families: parse_families(line("font-family")?)?,
        size,
        weight: number(line("font-weight")?)?,
        style,
        stretch_percent: number(stretch)?,
        line_height,
        letter_spacing: parse_letter_spacing(line("letter-spacing")?)?,
    };
    record
        .validate()
        .map_err(|_| invalid("typography 값이 범위를 벗어났습니다"))?;
    Ok((id, record))
}

pub(crate) fn paragraph(style: &ParagraphStyle) -> YamlValue {
    mapping([
        (
            "line-height",
            scalar(match style.line_height {
                ParagraphLineHeight::Auto => "auto".into(),
                ParagraphLineHeight::Multiplier(v) => v.get().to_string(),
                ParagraphLineHeight::Fixed(v) => format!("{}px", v.get()),
            }),
        ),
        (
            "text-align",
            scalar(match style.alignment {
                TextAlignment::Start => "start",
                TextAlignment::Center => "center",
                TextAlignment::End => "end",
                TextAlignment::Justify => "justify",
            }),
        ),
        ("typography", optional(style.typography, scalar)),
        (
            "wrapping",
            scalar(match style.wrapping {
                WrappingMode::Wrap => "wrap",
                WrappingMode::NoWrap => "nowrap",
            }),
        ),
    ])
}

pub(crate) fn parse_paragraph(value: &YamlValue) -> Result<ParagraphStyle, TextSourceError> {
    fields(
        value,
        &["line-height", "text-align", "typography", "wrapping"],
    )?;
    let line_height = match text(field(value, "line-height")?)? {
        "auto" => ParagraphLineHeight::Auto,
        other => match other.strip_suffix("px") {
            Some(fixed) => ParagraphLineHeight::Fixed(number(fixed)?),
            None => ParagraphLineHeight::Multiplier(number(other)?),
        },
    };
    let style = ParagraphStyle {
        typography: parse_optional(field(value, "typography")?, parse)?,
        alignment: match text(field(value, "text-align")?)? {
            "start" => TextAlignment::Start,
            "center" => TextAlignment::Center,
            "end" => TextAlignment::End,
            "justify" => TextAlignment::Justify,
            _ => return Err(invalid("text-align 값이 잘못되었습니다")),
        },
        line_height,
        wrapping: match text(field(value, "wrapping")?)? {
            "wrap" => WrappingMode::Wrap,
            "nowrap" => WrappingMode::NoWrap,
            _ => return Err(invalid("wrapping 값이 잘못되었습니다")),
        },
    };
    style.validate().map_err(invalid)?;
    Ok(style)
}

// 읽기 전용 v2 fixture를 만드는 테스트에서만 씁니다(ADR-0077 결정 2).
#[cfg_attr(not(test), allow(dead_code))]
fn run(style: &TextStyle) -> Result<YamlValue, TextSourceError> {
    if !style.features().is_empty() || !style.variations().is_empty() {
        return Err(unsupported("OpenType feature·variation"));
    }
    Ok(mapping([
        ("color", color_line(style.fill)),
        ("font-feature-settings", scalar("normal")),
        ("font-variation-settings", scalar("normal")),
        (
            "letter-spacing",
            optional(style.letter_spacing, |v| scalar(letter_spacing(v))),
        ),
        ("typography", optional(style.typography, scalar)),
    ]))
}

fn parse_run(value: &YamlValue) -> Result<TextStyle, TextSourceError> {
    fields(
        value,
        &[
            "color",
            "font-feature-settings",
            "font-variation-settings",
            "letter-spacing",
            "typography",
        ],
    )?;
    for key in ["font-feature-settings", "font-variation-settings"] {
        if text(field(value, key)?)? != "normal" {
            return Err(unsupported("OpenType feature·variation"));
        }
    }
    let mut style = TextStyle::new(
        parse_optional(field(value, "typography")?, parse)?,
        parse_optional(field(value, "color")?, parse_color_line)?,
        Vec::new(),
        Vec::new(),
    );
    style.letter_spacing = parse_optional(field(value, "letter-spacing")?, |v| {
        parse_letter_spacing(text(v)?)
    })?;
    Ok(style)
}

/// 표 전체를 다시 읽어 비교하므로 이 블록으로 표현할 수 없는 항목은 저장 전에 거부됩니다.
// 읽기 전용 v2 fixture를 만드는 테스트에서만 씁니다(ADR-0077 결정 2).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn encode(styles: &DocumentTextStyles) -> Result<YamlValue, TextSourceError> {
    if styles.is_empty() {
        return Ok(scalar("none"));
    }
    if !styles.fonts.is_empty() {
        return Err(unsupported(
            "등록 글꼴의 asset record는 텍스트 asset catalog에 둡니다(ADR-0061 결정 6)",
        ));
    }
    let id = styles
        .default_typography
        .ok_or_else(|| unsupported("기본 typography가 없는 표"))?;
    let (Some(resource), 1) = (styles.typography.get(&id), styles.typography.len()) else {
        return Err(unsupported("기본 typography 외의 resource"));
    };
    let (Some(paragraph_style), 1) = (styles.paragraphs.get(&0), styles.paragraphs.len()) else {
        return Err(unsupported("문단 style 0 외의 항목"));
    };
    let (Some(run_style), 1) = (styles.runs.get(&0), styles.runs.len()) else {
        return Err(unsupported("문자 style 0 외의 항목"));
    };
    let record = resource
        .record()
        .map_err(|_| unsupported("해석할 수 없는 typography"))?;
    let value = mapping([
        ("color", color_line(styles.default_fill)),
        ("paragraph", paragraph(paragraph_style)),
        ("run", run(run_style)?),
        ("typography", typography(id, &record)?),
    ]);
    if &decode(&value)? != styles {
        return Err(unsupported("한 줄 표현으로 정확히 복원할 수 없는 값"));
    }
    Ok(value)
}

pub(super) fn decode(value: &YamlValue) -> Result<DocumentTextStyles, TextSourceError> {
    if value.as_scalar() == Some("none") {
        return Ok(DocumentTextStyles::default());
    }
    fields(value, &["color", "paragraph", "run", "typography"])?;
    let (id, record) = parse_typography(field(value, "typography")?)?;
    Ok(DocumentTextStyles {
        default_typography: Some(id),
        default_fill: parse_optional(field(value, "color")?, parse_color_line)?,
        typography: [(id, TypographyValue::from_record(&record).map_err(invalid)?)].into(),
        fonts: Default::default(),
        paragraphs: [(0, parse_paragraph(field(value, "paragraph")?)?)].into(),
        runs: [(0, parse_run(field(value, "run")?)?)].into(),
    })
}
