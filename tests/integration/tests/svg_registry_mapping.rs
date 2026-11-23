//! SVG export 매핑표와 속성 registry CSS 매핑 대조 (IX.11.20).
//!
//! export crate는 document에 의존하지 않습니다(crate 그래프). 그래서 매핑표 행은 registry CSS longhand 이름으로
//! 원천을 잇고(`SvgFeature::css_longhands`), 이 test가 두 쪽을 대조합니다.
//!
//! - 매핑표가 이름을 대는 longhand는 모두 registry 항목의 CSS 매핑에 있습니다.
//! - registry에서 SVG로 그리는 칠하기·합성 longhand(`fill`, `stroke`, `stroke-width`, `opacity`, `mix-blend-mode`,
//!   `filter` 등)는 모두 매핑표 어느 행에 있습니다(누락 0).
//! - SVG 값 표기의 수 부분은 CSS 값 formatter(`css::value::format_number`·`format_percentage`)와 같습니다.

use std::collections::BTreeSet;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::Transform2D;
use stellar_document::css::registry;
use stellar_document::css::value::{format_number, format_percentage};
use stellar_export::svg::format::{color_value, transform_value, write_number};
use stellar_export::svg::mapping::{SvgFeature, SVG_MAPPING};

/// SVG가 칠하기·합성으로 그리는 registry longhand.
const PAINT_LONGHANDS: [&str; 10] = [
    "fill",
    "stroke",
    "stroke-width",
    "opacity",
    "mix-blend-mode",
    "filter",
    "fill-rule",
    "stroke-linecap",
    "object-fit",
    "image-rendering",
];

#[test]
fn every_named_longhand_exists_in_the_registry_css_mapping() {
    let registry_longhands: BTreeSet<&str> = registry::entries()
        .iter()
        .flat_map(|e| e.css.longhands.iter().copied())
        .collect();
    for row in &SVG_MAPPING {
        for name in row.feature.css_longhands() {
            assert!(
                registry_longhands.contains(name),
                "{:?}의 {name}은 registry CSS 매핑에 없습니다",
                row.feature
            );
        }
    }
    assert_eq!(SVG_MAPPING.len(), SvgFeature::ALL.len());
}

#[test]
fn every_registry_paint_longhand_has_a_mapping_row() {
    let mapped: BTreeSet<&str> = SvgFeature::ALL
        .iter()
        .flat_map(|f| f.css_longhands().iter().copied())
        .collect();
    let mut checked = 0;
    for entry in registry::entries() {
        for name in entry.css.longhands {
            if PAINT_LONGHANDS.contains(name) {
                assert!(
                    mapped.contains(name),
                    "registry {} {:#06x}의 {name}이 SVG 매핑표에 없습니다",
                    entry.scope.name(),
                    entry.key.0
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= PAINT_LONGHANDS.len(), "{checked}");
}

#[test]
fn svg_number_notation_equals_css_value_formatter() {
    let samples = [
        0.0f32,
        -0.0,
        1.0,
        -1.0,
        0.5,
        12.25,
        1440.0,
        0.1,
        1.0 / 3.0,
        f32::MIN_POSITIVE,
        1.0e-7,
        123_456.79,
        -2.5e-3,
    ];
    for value in samples {
        let mut svg = String::new();
        write_number(&mut svg, value).unwrap();
        assert_eq!(svg, format_number(value), "{value:?}");
    }
    // sRGB 범위 안 색은 `rgb(R%,G%,B%)`이고 채널은 CSS 백분율 표기와 같습니다.
    let color = Color::new(ColorSpace::Srgb, 0.25, 0.5, 1.0, 0.75).unwrap();
    let svg = color_value(color).unwrap();
    let expected = format!(
        "rgb({},{},{})",
        format_percentage(25.0),
        format_percentage(50.0),
        format_percentage(100.0)
    );
    assert_eq!(svg.paint, expected);
    assert_eq!(svg.opacity, Some(0.75));
    // 행렬은 `matrix(a b c d e f)`이고 각 수는 CSS 수 표기와 같습니다.
    let matrix = Transform2D::new(0.5, -1.0, 2.0, 0.25, 10.5, -3.0).unwrap();
    let expected = format!(
        "matrix({})",
        [0.5f32, -1.0, 2.0, 0.25, 10.5, -3.0]
            .map(format_number)
            .join(" ")
    );
    assert_eq!(transform_value(&matrix).unwrap(), Some(expected));
}
