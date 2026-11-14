//! 색공간을 Apple API 표현으로 내보냅니다 (XXII.3.3).
//!
//! Stellar 색은 색공간 + 비연관 알파이고 채널은 encoded(감마) 값입니다(III.4.1). Apple API는 같은 encoded 값을
//! 색공간 생성자로 받습니다.
//!
//! | API | sRGB | Display P3 |
//! |---|---|---|
//! | SwiftUI | `Color(.sRGB, red:, green:, blue:, opacity:)` | `Color(.displayP3, red:, green:, blue:, opacity:)` |
//! | AppKit | `NSColor(srgbRed:green:blue:alpha:)`, 확장 범위는 `NSColor(colorSpace: .extendedSRGB, components:count:)` | `NSColor(displayP3Red:green:blue:alpha:)` |
//! | UIKit | `UIColor(red:green:blue:alpha:)`(extended sRGB) | `UIColor(displayP3Red:green:blue:alpha:)` |
//!
//! 채널이 `[0, 1]` 밖이면(확장 범위) 값은 그대로 두고 [`ColorNote::ExtendedRange`]로 알립니다. AppKit·UIKit의
//! Display P3 생성자와 AppKit sRGB 생성자는 확장 범위를 표현하지 못할 수 있어 [`ColorNote::MayClamp`]도 붙입니다.

use std::fmt::Write as _;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::property::PathSegment;
use stellar_core_types::typed_value::TypedValue;

use crate::apple::swift::{identifiers, number};
use crate::apple::{AppleNote, AppleNoteKind, AppleNoteSubject};
use crate::ir::snapshot::id_hex;
use crate::ir::CodegenIr;
use crate::text::identifier;

/// 색 표현 API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorApi {
    /// SwiftUI `Color`.
    SwiftUi,
    /// AppKit `NSColor`.
    AppKit,
    /// UIKit `UIColor`.
    UiKit,
}

/// 색 변환 노트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorNote {
    /// 채널이 `[0, 1]` 밖입니다(확장 범위, 대상 색공간 gamut 밖).
    ExtendedRange,
    /// 선택한 생성자가 확장 범위 채널을 `[0, 1]`로 자를 수 있습니다.
    MayClamp,
}

/// 색 하나의 Apple 표현.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleColor {
    /// Swift 식.
    pub expression: String,
    /// 노트(오름차순).
    pub notes: Vec<ColorNote>,
}

fn extended(color: &Color) -> bool {
    [color.r(), color.g(), color.b()]
        .iter()
        .any(|channel| !(0.0..=1.0).contains(channel))
}

/// 색을 API 표현으로 바꿉니다.
pub fn color_expression(color: &Color, api: ColorApi) -> AppleColor {
    let (r, g, b, a) = (
        number(color.r()),
        number(color.g()),
        number(color.b()),
        number(color.alpha()),
    );
    let extended = extended(color);
    let mut notes = Vec::new();
    if extended {
        notes.push(ColorNote::ExtendedRange);
    }
    let expression = match (api, color.space()) {
        (ColorApi::SwiftUi, ColorSpace::Srgb) => {
            format!("Color(.sRGB, red: {r}, green: {g}, blue: {b}, opacity: {a})")
        }
        (ColorApi::SwiftUi, ColorSpace::DisplayP3) => {
            format!("Color(.displayP3, red: {r}, green: {g}, blue: {b}, opacity: {a})")
        }
        (ColorApi::AppKit, ColorSpace::Srgb) if extended => format!(
            "NSColor(colorSpace: .extendedSRGB, components: [{r}, {g}, {b}, {a}], count: 4)"
        ),
        (ColorApi::AppKit, ColorSpace::Srgb) => {
            format!("NSColor(srgbRed: {r}, green: {g}, blue: {b}, alpha: {a})")
        }
        (ColorApi::AppKit, ColorSpace::DisplayP3) => {
            if extended {
                notes.push(ColorNote::MayClamp);
            }
            format!("NSColor(displayP3Red: {r}, green: {g}, blue: {b}, alpha: {a})")
        }
        (ColorApi::UiKit, ColorSpace::Srgb) => {
            format!("UIColor(red: {r}, green: {g}, blue: {b}, alpha: {a})")
        }
        (ColorApi::UiKit, ColorSpace::DisplayP3) => {
            if extended {
                notes.push(ColorNote::MayClamp);
            }
            format!("UIColor(displayP3Red: {r}, green: {g}, blue: {b}, alpha: {a})")
        }
    };
    AppleColor { expression, notes }
}

fn path_suffix(path: &stellar_core_types::property::PropertyPath) -> String {
    let mut parts: Vec<String> = path.keys().iter().map(|key| key.0.to_string()).collect();
    match path.tail() {
        Some(PathSegment::Index(index)) => parts.push(format!("i{index}")),
        Some(PathSegment::Key(key)) => parts.push(identifier(key, "k")),
        None => {}
    }
    parts.join("_")
}

/// `<prefix>Colors` enum: Color token(id 오름차순)과 Color 값 node 속성(node preorder, path 오름차순).
pub(crate) fn colors_enum(
    ir: &CodegenIr,
    api: ColorApi,
    prefix: &str,
    notes: &mut Vec<AppleNote>,
) -> String {
    let mut out = format!("enum {prefix}Colors {{\n");
    let color_tokens: Vec<(&crate::ir::IrToken, &Color)> = ir
        .tokens
        .iter()
        .filter_map(|token| match &token.value {
            TypedValue::Color(color) => Some((token, color)),
            _ => None,
        })
        .collect();
    let names = identifiers(color_tokens.iter().map(|(t, _)| t.name.as_str()), "token");
    let mut token_names = std::collections::BTreeMap::new();
    for ((token, color), name) in color_tokens.iter().zip(&names) {
        let expression = color_expression(color, api);
        for note in &expression.notes {
            notes.push(AppleNote {
                subject: AppleNoteSubject::Token(token.id),
                kind: AppleNoteKind::Color(*note),
            });
        }
        let _ = writeln!(out, "    static let {name} = {}", expression.expression);
        token_names.insert(token.id, name.clone());
    }
    for node in &ir.nodes {
        for property in &node.properties {
            let TypedValue::Color(color) = &property.value else {
                continue;
            };
            let name = format!("n{}_{}", id_hex(node.id), path_suffix(&property.path));
            match property.token.and_then(|token| token_names.get(&token)) {
                Some(token) => {
                    let _ = writeln!(out, "    static let {name} = {token}");
                }
                None => {
                    let expression = color_expression(color, api);
                    for note in &expression.notes {
                        notes.push(AppleNote {
                            subject: AppleNoteSubject::Node(node.id),
                            kind: AppleNoteKind::Color(*note),
                        });
                    }
                    let _ = writeln!(out, "    static let {name} = {}", expression.expression);
                }
            }
        }
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> Color {
        Color::new(space, r, g, b, a).expect("색")
    }

    #[test]
    fn color_spaces_map_to_api_constructors() {
        let srgb = color(ColorSpace::Srgb, 1.0, 0.5, 0.0, 0.25);
        let p3 = color(ColorSpace::DisplayP3, 0.2, 0.4, 0.6, 1.0);
        assert_eq!(
            color_expression(&srgb, ColorApi::SwiftUi).expression,
            "Color(.sRGB, red: 1, green: 0.5, blue: 0, opacity: 0.25)"
        );
        assert_eq!(
            color_expression(&p3, ColorApi::SwiftUi).expression,
            "Color(.displayP3, red: 0.2, green: 0.4, blue: 0.6, opacity: 1)"
        );
        assert_eq!(
            color_expression(&srgb, ColorApi::AppKit).expression,
            "NSColor(srgbRed: 1, green: 0.5, blue: 0, alpha: 0.25)"
        );
        assert_eq!(
            color_expression(&p3, ColorApi::AppKit).expression,
            "NSColor(displayP3Red: 0.2, green: 0.4, blue: 0.6, alpha: 1)"
        );
        assert_eq!(
            color_expression(&srgb, ColorApi::UiKit).expression,
            "UIColor(red: 1, green: 0.5, blue: 0, alpha: 0.25)"
        );
        assert_eq!(
            color_expression(&p3, ColorApi::UiKit).expression,
            "UIColor(displayP3Red: 0.2, green: 0.4, blue: 0.6, alpha: 1)"
        );
        assert!(color_expression(&p3, ColorApi::UiKit).notes.is_empty());
    }

    #[test]
    fn extended_range_is_kept_and_reported() {
        let wide = color(ColorSpace::Srgb, 1.1, 0.5, -0.05, 1.0);
        let swiftui = color_expression(&wide, ColorApi::SwiftUi);
        assert_eq!(
            swiftui.expression,
            "Color(.sRGB, red: 1.1, green: 0.5, blue: -0.05, opacity: 1)"
        );
        assert_eq!(swiftui.notes, vec![ColorNote::ExtendedRange]);
        assert_eq!(
            color_expression(&wide, ColorApi::AppKit).expression,
            "NSColor(colorSpace: .extendedSRGB, components: [1.1, 0.5, -0.05, 1], count: 4)"
        );
        let wide_p3 = color(ColorSpace::DisplayP3, 1.2, 0.0, 0.0, 1.0);
        assert_eq!(
            color_expression(&wide_p3, ColorApi::AppKit).notes,
            vec![ColorNote::ExtendedRange, ColorNote::MayClamp]
        );
    }
}
