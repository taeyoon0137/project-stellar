//! 테스트 fixture CSS 선언 parser(엔진이 받는 부분집합).
//!
//! fixture 정의의 `style` 문자열(`name: value; ...`)을 [`LayoutStyle`]로 바꿉니다. 같은 문자열이 기대값 생성 때
//! 브라우저 HTML style에 그대로 들어갑니다. 받지 않는 선언은 panic으로 알립니다(fixture 정의 오류).

use stellar_core_types::value::{FiniteF32, Insets, Length, Percentage};
use stellar_layout::style::{
    AspectRatio, ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap, GridAutoFlow,
    GridLine, ItemAlignment, LayoutStyle, LengthPercentage, LengthPercentageAuto, MaxSize,
    MaxTrackSize, MinTrackSize, Position, SelfAlignment, Size, TrackSize,
};

fn number(text: &str) -> f32 {
    text.trim()
        .parse()
        .unwrap_or_else(|_| panic!("숫자가 아닙니다: {text}"))
}

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).unwrap()
}

fn length(text: &str) -> Length {
    let text = text.trim();
    if text == "0" {
        return Length::new(0.0).unwrap();
    }
    let value = text
        .strip_suffix("px")
        .unwrap_or_else(|| panic!("px 길이가 아닙니다: {text}"));
    Length::new(number(value)).unwrap()
}

fn percentage(text: &str) -> Option<Percentage> {
    text.trim()
        .strip_suffix('%')
        .map(|v| Percentage::new(number(v)).unwrap())
}

fn length_percentage(text: &str) -> LengthPercentage {
    match percentage(text) {
        Some(p) => LengthPercentage::Percentage(p),
        None => LengthPercentage::Length(length(text)),
    }
}

fn inset(text: &str) -> LengthPercentageAuto {
    match text.trim() {
        "auto" => LengthPercentageAuto::Auto,
        other => match percentage(other) {
            Some(p) => LengthPercentageAuto::Percentage(p),
            None => LengthPercentageAuto::Length(length(other)),
        },
    }
}

fn size(text: &str) -> Size {
    match text.trim() {
        "auto" => Size::Auto,
        "min-content" => Size::MinContent,
        "max-content" => Size::MaxContent,
        "fit-content" => Size::FitContent,
        other => match percentage(other) {
            Some(p) => Size::Percentage(p),
            None => Size::Length(length(other)),
        },
    }
}

fn max_size(text: &str) -> MaxSize {
    match text.trim() {
        "none" => MaxSize::None,
        "min-content" => MaxSize::MinContent,
        "max-content" => MaxSize::MaxContent,
        "fit-content" => MaxSize::FitContent,
        other => match percentage(other) {
            Some(p) => MaxSize::Percentage(p),
            None => MaxSize::Length(length(other)),
        },
    }
}

fn content_alignment(text: &str) -> ContentAlignment {
    match text.trim() {
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
        other => panic!("content alignment가 아닙니다: {other}"),
    }
}

fn item_alignment(text: &str) -> ItemAlignment {
    match text.trim() {
        "normal" => ItemAlignment::Normal,
        "stretch" => ItemAlignment::Stretch,
        "start" => ItemAlignment::Start,
        "end" => ItemAlignment::End,
        "flex-start" => ItemAlignment::FlexStart,
        "flex-end" => ItemAlignment::FlexEnd,
        "center" => ItemAlignment::Center,
        "baseline" => ItemAlignment::Baseline,
        other => panic!("item alignment가 아닙니다: {other}"),
    }
}

fn self_alignment(text: &str) -> SelfAlignment {
    match text.trim() {
        "auto" => SelfAlignment::Auto,
        other => SelfAlignment::Item(item_alignment(other)),
    }
}

fn grid_line(text: &str) -> GridLine {
    let text = text.trim();
    if text == "auto" {
        return GridLine::Auto;
    }
    if let Some(span) = text.strip_prefix("span") {
        return GridLine::Span(span.trim().parse().unwrap());
    }
    GridLine::Line(text.parse().unwrap())
}

fn min_track(text: &str) -> MinTrackSize {
    match text.trim() {
        "auto" => MinTrackSize::Auto,
        "min-content" => MinTrackSize::MinContent,
        "max-content" => MinTrackSize::MaxContent,
        other => match percentage(other) {
            Some(p) => MinTrackSize::Percentage(p),
            None => MinTrackSize::Length(length(other)),
        },
    }
}

fn max_track(text: &str) -> MaxTrackSize {
    let text = text.trim();
    if let Some(fr) = text.strip_suffix("fr") {
        return MaxTrackSize::Fraction(finite(number(fr)));
    }
    if let Some(inner) = text
        .strip_prefix("fit-content(")
        .and_then(|t| t.strip_suffix(')'))
    {
        return MaxTrackSize::FitContent(length_percentage(inner));
    }
    match text {
        "auto" => MaxTrackSize::Auto,
        "min-content" => MaxTrackSize::MinContent,
        "max-content" => MaxTrackSize::MaxContent,
        other => match percentage(other) {
            Some(p) => MaxTrackSize::Percentage(p),
            None => MaxTrackSize::Length(length(other)),
        },
    }
}

fn track(text: &str) -> TrackSize {
    let text = text.trim();
    if let Some(inner) = text
        .strip_prefix("minmax(")
        .and_then(|t| t.strip_suffix(')'))
    {
        let (min, max) = inner.split_once(',').expect("minmax 인자 2개");
        return TrackSize::minmax(min_track(min), max_track(max));
    }
    if text.ends_with("fr") {
        return TrackSize {
            min: MinTrackSize::Auto,
            max: max_track(text),
        };
    }
    if text.starts_with("fit-content(") {
        return TrackSize {
            min: MinTrackSize::Auto,
            max: max_track(text),
        };
    }
    match text {
        "auto" => TrackSize::AUTO,
        "min-content" => TrackSize::MIN_CONTENT,
        "max-content" => TrackSize::MAX_CONTENT,
        other => match percentage(other) {
            Some(p) => TrackSize {
                min: MinTrackSize::Percentage(p),
                max: MaxTrackSize::Percentage(p),
            },
            None => TrackSize::length(length(other)),
        },
    }
}

/// 괄호 밖 공백으로 나눕니다.
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in text.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn track_list(text: &str) -> Vec<TrackSize> {
    if text.trim() == "none" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for token in split_top(text) {
        if let Some(inner) = token
            .strip_prefix("repeat(")
            .and_then(|t| t.strip_suffix(')'))
        {
            let (count, list) = inner.split_once(',').expect("repeat 인자");
            let count: usize = count.trim().parse().unwrap();
            let tracks = track_list(list);
            for _ in 0..count {
                out.extend(tracks.iter().copied());
            }
        } else {
            out.push(track(&token));
        }
    }
    out
}

fn padding_values(text: &str) -> [f32; 4] {
    let values: Vec<f32> = split_top(text).iter().map(|v| length(v).get()).collect();
    match values.as_slice() {
        [a] => [*a, *a, *a, *a],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => panic!("padding 값 수 오류: {text}"),
    }
}

/// `name: value; ...` 선언을 생략값 style 위에 적용합니다.
pub fn parse_style(text: &str) -> LayoutStyle {
    let mut style = LayoutStyle::default();
    let mut padding = [0.0f32; 4];
    for part in text.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, value) = part
            .split_once(':')
            .unwrap_or_else(|| panic!("선언 형식 오류: {part}"));
        let name = name.trim();
        let value = value.trim();
        match name {
            "display" => {
                style.display = match value {
                    "block" => Display::Block,
                    "flex" => Display::Flex,
                    "grid" => Display::Grid,
                    "none" => Display::None,
                    other => panic!("display: {other}"),
                }
            }
            "position" => {
                style.position = match value {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    other => panic!("position: {other}"),
                }
            }
            "top" => style.top = inset(value),
            "right" => style.right = inset(value),
            "bottom" => style.bottom = inset(value),
            "left" => style.left = inset(value),
            "width" => style.width = size(value),
            "height" => style.height = size(value),
            "min-width" => style.min_width = size(value),
            "min-height" => style.min_height = size(value),
            "max-width" => style.max_width = max_size(value),
            "max-height" => style.max_height = max_size(value),
            "aspect-ratio" => {
                style.aspect_ratio = match value.split_once('/') {
                    Some((w, h)) => AspectRatio::new(number(w), number(h)),
                    None => AspectRatio::new(number(value), 1.0),
                }
            }
            "padding" => padding = padding_values(value),
            "padding-top" => padding[0] = length(value).get(),
            "padding-right" => padding[1] = length(value).get(),
            "padding-bottom" => padding[2] = length(value).get(),
            "padding-left" => padding[3] = length(value).get(),
            "flex-direction" => {
                style.flex_direction = match value {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    other => panic!("flex-direction: {other}"),
                }
            }
            "flex-wrap" => {
                style.flex_wrap = match value {
                    "nowrap" => FlexWrap::NoWrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    other => panic!("flex-wrap: {other}"),
                }
            }
            "justify-content" => style.justify_content = content_alignment(value),
            "align-content" => style.align_content = content_alignment(value),
            "align-items" => style.align_items = item_alignment(value),
            "justify-items" => style.justify_items = item_alignment(value),
            "align-self" => style.align_self = self_alignment(value),
            "justify-self" => style.justify_self = self_alignment(value),
            "row-gap" => style.row_gap = length(value),
            "column-gap" => style.column_gap = length(value),
            "gap" => {
                let parts = split_top(value);
                style.row_gap = length(&parts[0]);
                style.column_gap = length(parts.get(1).unwrap_or(&parts[0]));
            }
            "flex-grow" => style.flex_grow = finite(number(value)),
            "flex-shrink" => style.flex_shrink = finite(number(value)),
            "flex-basis" => {
                style.flex_basis = match value {
                    "auto" => FlexBasis::Auto,
                    "content" => FlexBasis::Content,
                    other => match percentage(other) {
                        Some(p) => FlexBasis::Percentage(p),
                        None => FlexBasis::Length(length(other)),
                    },
                }
            }
            "grid-template-columns" => style.grid_template_columns = track_list(value),
            "grid-template-rows" => style.grid_template_rows = track_list(value),
            "grid-auto-columns" => style.grid_auto_columns = track_list(value),
            "grid-auto-rows" => style.grid_auto_rows = track_list(value),
            "grid-auto-flow" => {
                style.grid_auto_flow = match value {
                    "row" => GridAutoFlow::Row,
                    "column" => GridAutoFlow::Column,
                    other => panic!("grid-auto-flow: {other}"),
                }
            }
            "grid-column-start" => style.grid_column_start = grid_line(value),
            "grid-column-end" => style.grid_column_end = grid_line(value),
            "grid-row-start" => style.grid_row_start = grid_line(value),
            "grid-row-end" => style.grid_row_end = grid_line(value),
            "grid-column" | "grid-row" => {
                let (start, end) = match value.split_once('/') {
                    Some((s, e)) => (grid_line(s), grid_line(e)),
                    None => (grid_line(value), GridLine::Auto),
                };
                if name == "grid-column" {
                    style.grid_column_start = start;
                    style.grid_column_end = end;
                } else {
                    style.grid_row_start = start;
                    style.grid_row_end = end;
                }
            }
            "grid-area" => {
                let parts: Vec<&str> = value.split('/').collect();
                let line = |i: usize| parts.get(i).map_or(GridLine::Auto, |p| grid_line(p));
                style.grid_row_start = line(0);
                style.grid_column_start = line(1);
                style.grid_row_end = line(2);
                style.grid_column_end = line(3);
            }
            other => panic!("받지 않는 선언: {other}"),
        }
    }
    style.padding = Insets::new(padding[0], padding[1], padding[2], padding[3]).unwrap();
    style
}
