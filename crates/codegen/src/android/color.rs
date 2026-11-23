//! 색공간을 Compose `Color` 표현으로 내보냅니다 (XXII.4.1).
//!
//! Stellar 색(색공간 + 비연관 알파, encoded 채널)을 같은 encoded 값으로 `Color(red, green, blue, alpha, colorSpace)`에
//! 넘깁니다. sRGB는 `ColorSpaces.Srgb`, Display P3는 `ColorSpaces.DisplayP3`입니다. 채널이 `[0, 1]` 밖인 sRGB는
//! `ColorSpaces.ExtendedSrgb`로 값을 그대로 둡니다. Compose `Color` 생성자는 색공간 범위 밖 채널을 거부하므로 확장
//! 범위 Display P3는 채널을 `[0, 1]`로 자르고 노트를 남깁니다.

use std::fmt::Write as _;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::property::PathSegment;
use stellar_core_types::typed_value::TypedValue;

use crate::android::kotlin::{float, RESERVED};
use crate::android::{AndroidNote, AndroidNoteKind, AndroidNoteSubject, ColorNote};
use crate::ir::snapshot::id_hex;
use crate::ir::CodegenIr;
use crate::text::{allocate_identifier, identifier};

/// 색 하나의 Compose 식과 노트.
pub fn color_expression(color: &Color) -> (String, Vec<ColorNote>) {
    let channels = [color.r(), color.g(), color.b()];
    let extended = channels.iter().any(|c| !(0.0..=1.0).contains(c));
    let mut notes = Vec::new();
    if extended {
        notes.push(ColorNote::ExtendedRange);
    }
    let (space, channels) = match color.space() {
        ColorSpace::Srgb if extended => ("ExtendedSrgb", channels),
        ColorSpace::Srgb => ("Srgb", channels),
        ColorSpace::DisplayP3 => {
            if extended {
                notes.push(ColorNote::Clamped);
            }
            ("DisplayP3", channels.map(|c| c.clamp(0.0, 1.0)))
        }
    };
    (
        format!(
            "Color(red = {}, green = {}, blue = {}, alpha = {}, colorSpace = ColorSpaces.{space})",
            float(channels[0]),
            float(channels[1]),
            float(channels[2]),
            float(color.alpha())
        ),
        notes,
    )
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

/// `object <prefix>Colors`: Color token(id 오름차순)과 Color 값 node 속성(node preorder, path 오름차순).
pub(crate) fn colors_object(ir: &CodegenIr, prefix: &str, notes: &mut Vec<AndroidNote>) -> String {
    let mut out = format!("object {prefix}Colors {{\n");
    let mut used = RESERVED.iter().map(|word| word.to_string()).collect();
    let mut token_names = std::collections::BTreeMap::new();
    for token in &ir.tokens {
        let TypedValue::Color(color) = &token.value else {
            continue;
        };
        let name = allocate_identifier(&mut used, identifier(&token.name, "token"));
        let (expression, color_notes) = color_expression(color);
        for note in color_notes {
            notes.push(AndroidNote {
                subject: AndroidNoteSubject::Token(token.id),
                kind: AndroidNoteKind::Color(note),
            });
        }
        let _ = writeln!(out, "    val {name} = {expression}");
        token_names.insert(token.id, name);
    }
    for node in &ir.nodes {
        for property in &node.properties {
            let TypedValue::Color(color) = &property.value else {
                continue;
            };
            let name = allocate_identifier(
                &mut used,
                format!("n{}_{}", id_hex(node.id), path_suffix(&property.path)),
            );
            match property.token.and_then(|token| token_names.get(&token)) {
                Some(token) => {
                    let _ = writeln!(out, "    val {name} = {token}");
                }
                None => {
                    let (expression, color_notes) = color_expression(color);
                    for note in color_notes {
                        notes.push(AndroidNote {
                            subject: AndroidNoteSubject::Node(node.id),
                            kind: AndroidNoteKind::Color(note),
                        });
                    }
                    let _ = writeln!(out, "    val {name} = {expression}");
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
    fn color_spaces_map_to_compose_color_spaces() {
        assert_eq!(
            color_expression(&color(ColorSpace::Srgb, 1.0, 0.5, 0.0, 0.25)).0,
            "Color(red = 1f, green = 0.5f, blue = 0f, alpha = 0.25f, colorSpace = ColorSpaces.Srgb)"
        );
        assert_eq!(
            color_expression(&color(ColorSpace::DisplayP3, 0.2, 0.4, 0.6, 1.0)).0,
            "Color(red = 0.2f, green = 0.4f, blue = 0.6f, alpha = 1f, colorSpace = ColorSpaces.DisplayP3)"
        );
        let wide = color_expression(&color(ColorSpace::Srgb, 1.1, 0.5, -0.05, 1.0));
        assert_eq!(
            wide,
            (
                "Color(red = 1.1f, green = 0.5f, blue = -0.05f, alpha = 1f, colorSpace = ColorSpaces.ExtendedSrgb)".to_string(),
                vec![ColorNote::ExtendedRange]
            )
        );
        let wide_p3 = color_expression(&color(ColorSpace::DisplayP3, 1.2, 0.0, 0.0, 1.0));
        assert!(wide_p3.0.starts_with("Color(red = 1f,"));
        assert_eq!(
            wide_p3.1,
            vec![ColorNote::ExtendedRange, ColorNote::Clamped]
        );
    }
}
