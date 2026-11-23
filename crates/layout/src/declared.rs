//! 풀린 CSS 선언 목록 → 엔진 입력 (ADR-0082 결정 1, ADR-0086 결정 8).
//!
//! 문서 경로의 엔진 입력은 기본값 세트가 **모든** layout longhand를 푼 선언 목록입니다
//! (`stellar_document::css::style_defaults::ResolvedLayout::declarations`). 이 함수는 목록을 1:1로 [`LayoutStyle`]
//! 필드에 옮기고, longhand 하나라도 빠지거나 두 번 나오면 거부합니다. 엔진이 생략값을 채우는 경로는 없습니다.
//!
//! 값 텍스트는 문서 쪽 정규 formatter(`stellar_document::css::value`의 수 표기,
//! `stellar_document::layout_record::format_value`)가 쓴 정규형만 받습니다. crate 그래프가 layout → document 의존을
//! 막으므로 같은 정규형을 이 파일이 읽습니다. 두 쪽이 같은 정규형을 쓰는지는 공유 fixture
//! (`tests/fixtures/declared_values.txt`)를 양쪽 test가 읽어 고정합니다.

use std::fmt;

use stellar_core_types::value::{FiniteF32, Insets, Length, Percentage};

use crate::style::{
    AspectRatio, BoxSizing, ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap,
    GridAutoFlow, GridLine, ItemAlignment, LayoutStyle, LengthPercentage, LengthPercentageAuto,
    MaxSize, MaxTrackSize, MinTrackSize, Position, SelfAlignment, Size, TrackSize,
};

/// 엔진이 받는 layout longhand 이름(문서 `LayoutLonghand::ALL`과 같은 집합).
pub const LONGHANDS: [&str; 40] = [
    "position",
    "box-sizing",
    "top",
    "right",
    "bottom",
    "left",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "aspect-ratio",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "align-self",
    "justify-self",
    "grid-column-start",
    "grid-column-end",
    "grid-row-start",
    "grid-row-end",
    "display",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "flex-direction",
    "flex-wrap",
    "justify-content",
    "align-content",
    "align-items",
    "justify-items",
    "row-gap",
    "column-gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-auto-columns",
    "grid-auto-rows",
    "grid-auto-flow",
];

/// 선언 목록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationError {
    /// 엔진이 모르는 longhand.
    UnknownLonghand {
        /// 이름.
        name: String,
    },
    /// 같은 longhand가 두 번 있습니다.
    Duplicate {
        /// 이름.
        name: &'static str,
    },
    /// longhand가 빠졌습니다(엔진은 생략값을 풀지 않습니다).
    Missing {
        /// 이름.
        name: &'static str,
    },
    /// 값이 정규형이 아닙니다.
    InvalidValue {
        /// 이름.
        name: &'static str,
        /// 값.
        value: String,
    },
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclarationError::UnknownLonghand { name } => {
                write!(f, "엔진이 모르는 longhand {name}")
            }
            DeclarationError::Duplicate { name } => write!(f, "{name}가 두 번 있습니다"),
            DeclarationError::Missing { name } => {
                write!(f, "{name}가 없습니다(엔진은 생략값을 풀지 않습니다)")
            }
            DeclarationError::InvalidValue { name, value } => {
                write!(f, "{name}: {value}은 정규 값이 아닙니다")
            }
        }
    }
}

impl std::error::Error for DeclarationError {}

struct Values<'a> {
    values: [Option<&'a str>; 40],
}

fn index_of(name: &str) -> Option<usize> {
    LONGHANDS.iter().position(|n| *n == name)
}

fn number(text: &str) -> Option<f32> {
    // 정규형: 지수 없는 10진, `-0` 없음, 선행 `+` 없음.
    if text.is_empty() || text.starts_with('+') || text.contains(['e', 'E']) || text == "-0" {
        return None;
    }
    let value: f32 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

fn px(text: &str) -> Option<Length> {
    Length::new(number(text.strip_suffix("px")?)?).ok()
}

fn percent(text: &str) -> Option<Percentage> {
    Percentage::new(number(text.strip_suffix('%')?)?).ok()
}

fn finite(text: &str) -> Option<FiniteF32> {
    FiniteF32::new(number(text)?).ok()
}

fn length_percentage(text: &str) -> Option<LengthPercentage> {
    px(text)
        .map(LengthPercentage::Length)
        .or_else(|| percent(text).map(LengthPercentage::Percentage))
}

fn inset(text: &str) -> Option<LengthPercentageAuto> {
    match text {
        "auto" => Some(LengthPercentageAuto::Auto),
        _ => px(text)
            .map(LengthPercentageAuto::Length)
            .or_else(|| percent(text).map(LengthPercentageAuto::Percentage)),
    }
}

fn size(text: &str) -> Option<Size> {
    Some(match text {
        "auto" => Size::Auto,
        "min-content" => Size::MinContent,
        "max-content" => Size::MaxContent,
        "fit-content" => Size::FitContent,
        _ => match length_percentage(text)? {
            LengthPercentage::Length(l) => Size::Length(l),
            LengthPercentage::Percentage(p) => Size::Percentage(p),
        },
    })
}

fn max_size(text: &str) -> Option<MaxSize> {
    Some(match text {
        "none" => MaxSize::None,
        "min-content" => MaxSize::MinContent,
        "max-content" => MaxSize::MaxContent,
        "fit-content" => MaxSize::FitContent,
        _ => match length_percentage(text)? {
            LengthPercentage::Length(l) => MaxSize::Length(l),
            LengthPercentage::Percentage(p) => MaxSize::Percentage(p),
        },
    })
}

fn item(text: &str) -> Option<ItemAlignment> {
    Some(match text {
        "normal" => ItemAlignment::Normal,
        "stretch" => ItemAlignment::Stretch,
        "start" => ItemAlignment::Start,
        "end" => ItemAlignment::End,
        "flex-start" => ItemAlignment::FlexStart,
        "flex-end" => ItemAlignment::FlexEnd,
        "center" => ItemAlignment::Center,
        "baseline" => ItemAlignment::Baseline,
        _ => return None,
    })
}

fn self_alignment(text: &str) -> Option<SelfAlignment> {
    match text {
        "auto" => Some(SelfAlignment::Auto),
        _ => item(text).map(SelfAlignment::Item),
    }
}

fn content(text: &str) -> Option<ContentAlignment> {
    Some(match text {
        "normal" => ContentAlignment::Normal,
        "start" => ContentAlignment::Start,
        "end" => ContentAlignment::End,
        "flex-start" => ContentAlignment::FlexStart,
        "flex-end" => ContentAlignment::FlexEnd,
        "center" => ContentAlignment::Center,
        "space-between" => ContentAlignment::SpaceBetween,
        "space-around" => ContentAlignment::SpaceAround,
        "space-evenly" => ContentAlignment::SpaceEvenly,
        "stretch" => ContentAlignment::Stretch,
        _ => return None,
    })
}

fn grid_line(text: &str) -> Option<GridLine> {
    if text == "auto" {
        return Some(GridLine::Auto);
    }
    if let Some(n) = text.strip_prefix("span ") {
        let n: u16 = n.parse().ok()?;
        return (n >= 1 && n.to_string() == text[5..]).then_some(GridLine::Span(n));
    }
    let n: i16 = text.parse().ok()?;
    (n != 0 && n.to_string() == text).then_some(GridLine::Line(n))
}

fn min_track(text: &str) -> Option<MinTrackSize> {
    Some(match text {
        "auto" => MinTrackSize::Auto,
        "min-content" => MinTrackSize::MinContent,
        "max-content" => MinTrackSize::MaxContent,
        _ => match length_percentage(text)? {
            LengthPercentage::Length(l) => MinTrackSize::Length(l),
            LengthPercentage::Percentage(p) => MinTrackSize::Percentage(p),
        },
    })
}

fn max_track(text: &str) -> Option<MaxTrackSize> {
    if let Some(fr) = text.strip_suffix("fr") {
        let fr = finite(fr)?;
        return (fr.get() >= 0.0).then_some(MaxTrackSize::Fraction(fr));
    }
    Some(match min_track(text)? {
        MinTrackSize::Auto => MaxTrackSize::Auto,
        MinTrackSize::Length(l) => MaxTrackSize::Length(l),
        MinTrackSize::Percentage(p) => MaxTrackSize::Percentage(p),
        MinTrackSize::MinContent => MaxTrackSize::MinContent,
        MinTrackSize::MaxContent => MaxTrackSize::MaxContent,
    })
}

fn track(text: &str) -> Option<TrackSize> {
    if let Some(body) = text
        .strip_prefix("minmax(")
        .and_then(|t| t.strip_suffix(')'))
    {
        let (min, max) = body.split_once(", ")?;
        return Some(TrackSize::minmax(min_track(min)?, max_track(max)?));
    }
    if let Some(body) = text
        .strip_prefix("fit-content(")
        .and_then(|t| t.strip_suffix(')'))
    {
        return Some(TrackSize::fit_content(length_percentage(body)?));
    }
    Some(match max_track(text)? {
        MaxTrackSize::Auto => TrackSize::AUTO,
        MaxTrackSize::Fraction(fr) => TrackSize::fr(fr),
        MaxTrackSize::Length(l) => TrackSize::length(l),
        MaxTrackSize::Percentage(p) => {
            TrackSize::minmax(MinTrackSize::Percentage(p), MaxTrackSize::Percentage(p))
        }
        MaxTrackSize::MinContent => TrackSize::MIN_CONTENT,
        MaxTrackSize::MaxContent => TrackSize::MAX_CONTENT,
        MaxTrackSize::FitContent(_) => return None,
    })
}

/// track 목록(공백 구분, 함수 안 `", "`는 나누지 않음).
fn tracks(text: &str, allow_none: bool) -> Option<Vec<TrackSize>> {
    if text == "none" {
        return allow_none.then(Vec::new);
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            ' ' if depth == 0 => {
                out.push(track(&text[start..i])?);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    out.push(track(&text[start..])?);
    Some(out)
}

impl<'a> Values<'a> {
    fn get(&self, name: &'static str) -> &'a str {
        self.values[index_of(name).expect("LONGHANDS 이름")].expect("검사한 목록")
    }

    fn parse<T>(
        &self,
        name: &'static str,
        f: impl FnOnce(&str) -> Option<T>,
    ) -> Result<T, DeclarationError> {
        let value = self.get(name);
        f(value).ok_or_else(|| DeclarationError::InvalidValue {
            name,
            value: value.to_string(),
        })
    }
}

fn keyword<'c, T: Copy>(choices: &'c [(&'c str, T)]) -> impl Fn(&str) -> Option<T> + 'c {
    move |text| choices.iter().find(|(k, _)| *k == text).map(|(_, v)| *v)
}

/// 모든 layout longhand의 정규 CSS 값 목록을 엔진 입력으로 옮깁니다.
pub fn style_from_declarations(
    declarations: &[(&str, &str)],
) -> Result<LayoutStyle, DeclarationError> {
    let mut values = Values { values: [None; 40] };
    for (name, value) in declarations {
        let index = index_of(name).ok_or_else(|| DeclarationError::UnknownLonghand {
            name: (*name).to_string(),
        })?;
        if values.values[index].replace(value).is_some() {
            return Err(DeclarationError::Duplicate {
                name: LONGHANDS[index],
            });
        }
    }
    if let Some(index) = values.values.iter().position(Option::is_none) {
        return Err(DeclarationError::Missing {
            name: LONGHANDS[index],
        });
    }
    let v = &values;
    let pad = |name| v.parse(name, |t| px(t).map(|l| l.get()));
    let gap = |name| {
        v.parse(name, |t| match t {
            "normal" => Some(Length::from_raw(FiniteF32::ZERO)),
            _ => px(t),
        })
    };
    let non_negative = |name| v.parse(name, |t| finite(t).filter(|n| n.get() >= 0.0));
    Ok(LayoutStyle {
        display: v.parse(
            "display",
            keyword(&[
                ("block", Display::Block),
                ("flex", Display::Flex),
                ("grid", Display::Grid),
                ("none", Display::None),
            ]),
        )?,
        position: v.parse(
            "position",
            keyword(&[
                ("static", Position::Static),
                ("relative", Position::Relative),
                ("absolute", Position::Absolute),
            ]),
        )?,
        box_sizing: v.parse(
            "box-sizing",
            keyword(&[
                ("content-box", BoxSizing::ContentBox),
                ("border-box", BoxSizing::BorderBox),
            ]),
        )?,
        top: v.parse("top", inset)?,
        right: v.parse("right", inset)?,
        bottom: v.parse("bottom", inset)?,
        left: v.parse("left", inset)?,
        width: v.parse("width", size)?,
        height: v.parse("height", size)?,
        min_width: v.parse("min-width", size)?,
        min_height: v.parse("min-height", size)?,
        max_width: v.parse("max-width", max_size)?,
        max_height: v.parse("max-height", max_size)?,
        aspect_ratio: v.parse("aspect-ratio", |t| match t {
            "auto" => Some(None),
            _ => {
                let (w, h) = t.split_once(" / ")?;
                Some(Some(AspectRatio {
                    width: finite(w)?,
                    height: finite(h)?,
                }))
            }
        })?,
        padding: Insets::new(
            pad("padding-top")?,
            pad("padding-right")?,
            pad("padding-bottom")?,
            pad("padding-left")?,
        )
        .expect("유한한 길이"),
        flex_direction: v.parse(
            "flex-direction",
            keyword(&[
                ("row", FlexDirection::Row),
                ("row-reverse", FlexDirection::RowReverse),
                ("column", FlexDirection::Column),
                ("column-reverse", FlexDirection::ColumnReverse),
            ]),
        )?,
        flex_wrap: v.parse(
            "flex-wrap",
            keyword(&[
                ("nowrap", FlexWrap::NoWrap),
                ("wrap", FlexWrap::Wrap),
                ("wrap-reverse", FlexWrap::WrapReverse),
            ]),
        )?,
        justify_content: v.parse("justify-content", content)?,
        align_content: v.parse("align-content", content)?,
        align_items: v.parse("align-items", item)?,
        justify_items: v.parse("justify-items", item)?,
        row_gap: gap("row-gap")?,
        column_gap: gap("column-gap")?,
        flex_grow: non_negative("flex-grow")?,
        flex_shrink: non_negative("flex-shrink")?,
        flex_basis: v.parse("flex-basis", |t| match t {
            "auto" => Some(FlexBasis::Auto),
            "content" => Some(FlexBasis::Content),
            _ => match length_percentage(t)? {
                LengthPercentage::Length(l) => Some(FlexBasis::Length(l)),
                LengthPercentage::Percentage(p) => Some(FlexBasis::Percentage(p)),
            },
        })?,
        align_self: v.parse("align-self", self_alignment)?,
        justify_self: v.parse("justify-self", self_alignment)?,
        grid_template_columns: v.parse("grid-template-columns", |t| tracks(t, true))?,
        grid_template_rows: v.parse("grid-template-rows", |t| tracks(t, true))?,
        grid_auto_columns: v.parse("grid-auto-columns", |t| tracks(t, false))?,
        grid_auto_rows: v.parse("grid-auto-rows", |t| tracks(t, false))?,
        grid_auto_flow: v.parse(
            "grid-auto-flow",
            keyword(&[("row", GridAutoFlow::Row), ("column", GridAutoFlow::Column)]),
        )?,
        grid_column_start: v.parse("grid-column-start", grid_line)?,
        grid_column_end: v.parse("grid-column-end", grid_line)?,
        grid_row_start: v.parse("grid-row-start", grid_line)?,
        grid_row_end: v.parse("grid-row-end", grid_line)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `css` 세트(문서 `style_defaults`)의 공통 값 그대로입니다.
    fn css_set() -> Vec<(&'static str, &'static str)> {
        vec![
            ("position", "static"),
            ("box-sizing", "content-box"),
            ("top", "auto"),
            ("right", "auto"),
            ("bottom", "auto"),
            ("left", "auto"),
            ("width", "auto"),
            ("height", "auto"),
            ("min-width", "auto"),
            ("min-height", "auto"),
            ("max-width", "none"),
            ("max-height", "none"),
            ("aspect-ratio", "auto"),
            ("flex-grow", "0"),
            ("flex-shrink", "1"),
            ("flex-basis", "auto"),
            ("align-self", "auto"),
            ("justify-self", "auto"),
            ("grid-column-start", "auto"),
            ("grid-column-end", "auto"),
            ("grid-row-start", "auto"),
            ("grid-row-end", "auto"),
            ("display", "block"),
            ("padding-top", "0px"),
            ("padding-right", "0px"),
            ("padding-bottom", "0px"),
            ("padding-left", "0px"),
            ("flex-direction", "row"),
            ("flex-wrap", "nowrap"),
            ("justify-content", "normal"),
            ("align-content", "normal"),
            ("align-items", "normal"),
            ("justify-items", "normal"),
            ("row-gap", "normal"),
            ("column-gap", "normal"),
            ("grid-template-columns", "none"),
            ("grid-template-rows", "none"),
            ("grid-auto-columns", "auto"),
            ("grid-auto-rows", "auto"),
            ("grid-auto-flow", "row"),
        ]
    }

    #[test]
    fn css_set_maps_to_initial_values_with_content_box() {
        let style = style_from_declarations(&css_set()).unwrap();
        assert_eq!(
            style,
            LayoutStyle {
                box_sizing: BoxSizing::ContentBox,
                ..LayoutStyle::default()
            }
        );
    }

    #[test]
    fn missing_duplicate_unknown_and_non_canonical_are_rejected() {
        let mut list = css_set();
        list.pop();
        assert_eq!(
            style_from_declarations(&list).unwrap_err(),
            DeclarationError::Missing {
                name: "grid-auto-flow"
            }
        );
        let mut list = css_set();
        list.push(("display", "flex"));
        assert!(matches!(
            style_from_declarations(&list),
            Err(DeclarationError::Duplicate { name: "display" })
        ));
        let mut list = css_set();
        list.push(("float", "left"));
        assert!(matches!(
            style_from_declarations(&list),
            Err(DeclarationError::UnknownLonghand { .. })
        ));
        for (name, bad) in [
            ("width", "1.0e1px"),
            ("width", "-0px"),
            ("left", "+1px"),
            ("display", "inline"),
            ("grid-column-start", "0"),
            ("grid-column-start", "span 01"),
            ("aspect-ratio", "2"),
            ("grid-auto-rows", "none"),
            ("flex-grow", "-1"),
        ] {
            let list: Vec<_> = css_set()
                .into_iter()
                .map(|(n, v)| if n == name { (n, bad) } else { (n, v) })
                .collect();
            assert!(
                matches!(
                    style_from_declarations(&list),
                    Err(DeclarationError::InvalidValue { .. })
                ),
                "{name}: {bad}"
            );
        }
    }

    /// 문서 formatter가 쓴 값 목록(공유 fixture)을 모두 받습니다. 문서 쪽 test가 같은 파일이 자기 정규형과 같은지
    /// 확인합니다(`stellar_document::layout_record` `declared_values_fixture_matches_formatter`).
    #[test]
    fn shared_fixture_values_are_all_accepted() {
        let fixture = include_str!("../tests/fixtures/declared_values.txt");
        let mut count = 0;
        for line in fixture
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (name, value) = line.split_once(": ").expect("name: value");
            let list: Vec<_> = css_set()
                .into_iter()
                .map(|(n, v)| if n == name { (n, value) } else { (n, v) })
                .collect();
            style_from_declarations(&list).unwrap_or_else(|e| panic!("{line}: {e}"));
            count += 1;
        }
        assert!(count > 60, "fixture 항목 {count}");
    }
}
