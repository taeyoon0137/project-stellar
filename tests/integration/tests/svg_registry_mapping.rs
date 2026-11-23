//! SVG export 매핑표와 속성 registry CSS 매핑 대조 (IX.11.20).
//!
//! export crate는 document에 의존하지 않습니다(crate 그래프). 그래서 매핑표 행은 registry CSS longhand 이름으로
//! 원천을 잇고(`SvgFeature::css_longhands`), 이 test가 두 쪽을 대조합니다.
//!
//! - 매핑표가 이름을 대는 longhand는 모두 registry 항목의 CSS 매핑에 있습니다.
//! - registry에서 SVG로 그리는 칠하기·합성 longhand(`fill`, `stroke`, `stroke-width`, `opacity`, `mix-blend-mode`,
//!   `filter` 등)는 모두 매핑표 어느 행에 있습니다(누락 0).
//! - 매핑표 행의 원천 registry 항목(`SvgFeature::registry_items`, 범위·이름)은 모두 registry에 있고, 그 항목들의 CSS
//!   매핑 longhand가 그 행의 `css_longhands`를 모두 덮습니다(ADR-0128 결정 5).
//! - SVG 값 표기의 수 부분은 CSS 값 formatter(`css::value::format_number`·`format_percentage`)와 같습니다. 두 쪽은
//!   같은 함수(`stellar_core_types::number`, ADR-0128 결정 4)를 부르고, 이 test는 색·행렬 profile까지 대조합니다.

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
fn every_registry_item_exists_and_covers_the_rows_css_longhands() {
    let entries = registry::entries();
    let mut rows_with_longhands = 0;
    for feature in SvgFeature::ALL {
        let items = feature.registry_items();
        assert!(
            !items.is_empty(),
            "{feature:?}에 원천 registry 항목이 없습니다"
        );
        let mut covered = BTreeSet::new();
        for (scope, name) in items {
            let found: Vec<_> = entries
                .iter()
                .filter(|e| e.scope.name() == *scope && e.name == *name && e.bundle.is_empty())
                .collect();
            assert_eq!(
                found.len(),
                1,
                "{feature:?}의 {scope}/{name}은 registry 편집 항목 하나가 아닙니다"
            );
            covered.extend(found[0].css.longhands.iter().copied());
        }
        for longhand in feature.css_longhands() {
            assert!(
                covered.contains(longhand),
                "{feature:?}의 CSS longhand {longhand}를 원천 항목 {items:?}의 CSS 매핑이 덮지 않습니다"
            );
        }
        if !feature.css_longhands().is_empty() {
            rows_with_longhands += 1;
        }
    }
    assert!(rows_with_longhands > 0);
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
