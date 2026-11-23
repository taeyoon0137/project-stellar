//! CSS layout 내보내기 검증. 브라우저 없이 출력 CSS의 cascade를 계산해 문서 해석 값과 대조합니다.
//!
//! 대조 규칙: 요소 하나의 longhand마다 출력 규칙 중 그 요소에 맞는 선언을 특이도(`#id` 1-0-0, `:where()` 0)와
//! 순서로 고르고, 없으면 대상 브라우저 기본값(`css` 세트)을 씁니다. 그 계산값이 문서 해석 값
//! (`style_defaults::resolve_kind`, 활성 세트)과 같아야 합니다. 배치 규칙이 정한 값(`position`, Free `left`·`top`,
//! 숨김 `display: none`)만 ADR-0089 결정 4의 기대값으로 바꿔 비교합니다.

use super::fixture::{self, id};
use super::*;
use crate::snapshot::open_snapshot;
use crate::test_support::{file_digest, save_documents, scaffold_project, TempDir};
use stellar_project::manifest_style_defaults::set_active;

fn project(name: &str, set: &str) -> TempDir {
    let dir = TempDir::new(name);
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    if set != sd::CSS {
        set_active(&dir.path, set).unwrap();
    }
    dir
}

fn export(dir: &TempDir, view: u64) -> WebFrameHtml {
    let snapshot = open_snapshot(&dir.path).unwrap();
    let report = export_web_frame_node(&snapshot, id(1), id(view), None).unwrap();
    assert!(
        report.html.is_some(),
        "{:#?}",
        report
            .diagnostics
            .iter()
            .filter(|d| d.error)
            .collect::<Vec<_>>()
    );
    report
}

/// 출력 `<style>`의 규칙 목록(선택자, 선언).
fn rules(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let start = html.find("<style>\n").unwrap() + "<style>\n".len();
    let end = html.find("</style>").unwrap();
    html[start..end]
        .split("}\n")
        .filter(|r| !r.trim().is_empty())
        .map(|rule| {
            let (selector, body) = rule.split_once(" {\n").unwrap();
            let declarations = body
                .lines()
                .map(|line| {
                    let (name, value) = line.trim().trim_end_matches(';').split_once(": ").unwrap();
                    (name.to_string(), value.to_string())
                })
                .collect();
            (selector.to_string(), declarations)
        })
        .collect()
}

/// 이 출력이 쓰는 선택자 형태만의 특이도(id 수, class 수, type 수).
fn specificity(selector: &str) -> (u32, u32, u32) {
    if selector.starts_with(":where(") {
        (0, 0, 0)
    } else if selector.starts_with('#') && !selector.contains([' ', ',', '.']) {
        (1, 0, 0)
    } else if selector == "html, body" {
        (0, 0, 1)
    } else {
        panic!("예상하지 않은 선택자 {selector}")
    }
}

fn matches(selector: &str, root: &str, element: &ElementStyle) -> bool {
    if let Some(inner) = selector.strip_prefix(":where(") {
        let inner = inner.trim_end_matches(')');
        if inner == format!("#{root}, #{root} *") {
            return true;
        }
        let class = web_layout::class_name(element.class.name());
        return inner == format!("#{root}.{class}, #{root} .{class}");
    }
    selector == format!("#{}", element.element)
}

/// cascade 계산값.
fn computed(
    rules: &[(String, Vec<(String, String)>)],
    root: &str,
    element: &ElementStyle,
    web: &FlatSet,
    longhand: L,
) -> String {
    let mut best: Option<((u32, u32, u32), usize, String)> = None;
    for (order, (selector, declarations)) in rules.iter().enumerate() {
        if !matches(selector, root, element) {
            continue;
        }
        for (name, value) in declarations {
            if name == longhand.name() {
                let key = (specificity(selector), order);
                if best.as_ref().is_none_or(|(s, o, _)| key >= (*s, *o)) {
                    best = Some((key.0, key.1, value.clone()));
                }
            }
        }
    }
    best.map(|(_, _, v)| v)
        .unwrap_or_else(|| format_value(web.value(element.class, longhand)))
}

/// 배치 규칙을 반영한 기대 계산값. `None`이면 배치가 정한 값이라 따로 확인합니다.
fn expected(element: &ElementStyle, longhand: L) -> Option<String> {
    let value = &element.resolved.get(longhand).value;
    let position = keyword(&element.resolved.get(L::Position).value).unwrap();
    match (element.placement, longhand) {
        (Placement::Root, L::Position) => Some("relative".into()),
        (Placement::Root, l) if ROOT_IGNORED.contains(&l) && l != L::BoxSizing => None,
        (Placement::Free | Placement::Inset, L::Position) => Some("absolute".into()),
        (Placement::Flow, L::Position) if position == "static" => Some("relative".into()),
        (Placement::Free, L::Top | L::Left) => None,
        (Placement::Flow, L::Top | L::Right | L::Bottom | L::Left) if position == "static" => {
            Some("auto".into())
        }
        (_, L::Display) if !element.visible => Some("none".into()),
        _ => Some(format_value(value)),
    }
}

fn check_cascade(report: &WebFrameHtml) {
    let html = report.html.as_deref().unwrap();
    if report.format == FORMAT_FIXED {
        // 고정 Free Frame 출력은 기존 계약(모든 값을 명시)이라 이 대조의 대상이 아닙니다.
        assert!(report.styles.iter().all(|s| !s.has_record));
        return;
    }
    let rules = rules(html);
    let root = &report.styles[0].element;
    let web = sd::builtin(sd::CSS).unwrap();
    for element in &report.styles {
        for longhand in L::ALL {
            // Stellar border 보정(box-sizing·크기·padding)은 보정 규칙 test가 값을 확인합니다.
            let converted = element.children.border > 0.0
                && (matches!(longhand, L::BoxSizing)
                    || PADDING_OF.contains(&longhand)
                    || SIZES.iter().any(|(l, _)| *l == longhand));
            if converted {
                continue;
            }
            let Some(expected) = expected(element, longhand) else {
                continue;
            };
            assert_eq!(
                computed(&rules, root, element, &web, longhand),
                expected,
                "set {}, element {}, {longhand}",
                report.style_defaults,
                element.element
            );
        }
        if element.placement == Placement::Free {
            let rule = rules
                .iter()
                .find(|(s, _)| *s == format!("#{}", element.element))
                .unwrap();
            assert!(rule
                .1
                .iter()
                .any(|(n, v)| n == "left" && v.starts_with("calc(")));
        }
    }
    if report.base_rules.is_empty() {
        assert!(!html.contains(":where("));
    }
    assert_eq!(
        html.contains(" class=\""),
        report.base_rules.contains(".stellar-")
    );
}

#[test]
fn computed_cascade_matches_document_resolution_for_both_sets() {
    for set in [sd::CSS, sd::REACT_NATIVE] {
        let dir = project(&format!("export-css-cascade-{set}"), set);
        let before = file_digest(&dir.path);
        for view in [3, 4, 5] {
            let report = export(&dir, view);
            assert_eq!(report.style_defaults, set);
            check_cascade(&report);
            // 같은 입력은 같은 bytes입니다.
            assert_eq!(export(&dir, view), report);
        }
        assert_eq!(file_digest(&dir.path), before);
    }
}

#[test]
fn record_free_view_with_css_set_keeps_fixed_output() {
    let dir = project("export-css-fixed", sd::CSS);
    let report = export(&dir, 4);
    assert_eq!(report.format, FORMAT_FIXED);
    assert!(report.base_rules.is_empty());
    let html = report.html.unwrap();
    assert!(html.contains("  box-sizing: border-box;\n  display: block;\n  position: absolute;"));
    assert!(!html.contains(":where("));
    // 같은 View를 react-native 세트로 열면 root가 flex container가 되어 CSS layout 출력입니다.
    let dir = project("export-css-fixed-rn", sd::REACT_NATIVE);
    let report = export(&dir, 4);
    assert_eq!(report.format, FORMAT_LAYOUT);
    assert!(report.styles[1..]
        .iter()
        .all(|s| s.placement == Placement::Flow));
}

#[test]
fn same_set_as_target_adds_no_default_declarations() {
    let dir = project("export-css-zero", sd::CSS);
    for view in [3, 5] {
        let report = export(&dir, view);
        assert_eq!(report.format, FORMAT_LAYOUT);
        assert!(report.base_rules.is_empty());
        for element in &report.styles {
            for d in &element.declarations {
                let source = &element.resolved.get(L::from_name(d.name).unwrap()).source;
                // 기본값 세트 값은 요소 규칙에 쓰지 않습니다. 명시·배치·보정·숨김 값만 나옵니다.
                assert!(
                    *source == ValueSource::Explicit || d.origin != DeclarationOrigin::Explicit,
                    "{} {}",
                    element.element,
                    d.name
                );
            }
        }
    }
    // react-native 세트 → RN 대상: 차이가 없으므로 명시 값과 배치 항목만 나옵니다.
    let dir = project("export-css-zero-rn", sd::REACT_NATIVE);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let rn = export_react_native(&snapshot, id(1), id(3), None).unwrap();
    let target = sd::builtin(sd::REACT_NATIVE).unwrap();
    for element in &rn.web.styles {
        let (style, _) = react_native_style(element, &target);
        for (key, _) in &style.entries {
            let placement = ["position", "left", "top", "transform", "display"];
            let explicit = L::ALL.iter().any(|l| {
                property_name(l.name()) == *key
                    && element.resolved.get(*l).source == ValueSource::Explicit
            });
            assert!(
                explicit || placement.contains(&key.as_str()),
                "{} {key}",
                element.element
            );
        }
    }
}

#[test]
fn author_rule_beats_zero_specificity_base_rule() {
    let dir = project("export-css-specificity", sd::REACT_NATIVE);
    let report = export(&dir, 3);
    let html = report.html.as_deref().unwrap();
    let rules = rules(html);
    let base = rules
        .iter()
        .find(|(s, d)| {
            s.starts_with(":where(")
                && d.iter()
                    .any(|(n, v)| n == "flex-direction" && v == "column")
        })
        .expect("react-native 기반 규칙의 flex-direction: column");
    let element = report.styles.iter().find(|s| s.node == id(12)).unwrap();
    let author = rules
        .iter()
        .find(|(s, _)| *s == format!("#{}", element.element))
        .unwrap();
    assert!(author
        .1
        .iter()
        .any(|(n, v)| n == "flex-direction" && v == "row"));
    assert!(specificity(&author.0) > specificity(&base.0));
    assert_eq!(specificity(&base.0), (0, 0, 0));
    // 기반 규칙이 저자 규칙보다 먼저 나와도 특이도로 저자 규칙이 이깁니다.
    let web = sd::builtin(sd::CSS).unwrap();
    assert_eq!(
        computed(
            &rules,
            &report.styles[0].element,
            element,
            &web,
            L::FlexDirection
        ),
        "row"
    );
    // 명시하지 않은 Frame(11)은 기반 규칙 값을 받습니다.
    let plain = report.styles.iter().find(|s| s.node == id(11)).unwrap();
    assert_eq!(
        computed(
            &rules,
            &report.styles[0].element,
            plain,
            &web,
            L::FlexDirection
        ),
        "column"
    );
    assert!(report.base_rules.starts_with(&format!(
        ":where(#{root}, #{root} *) {{\n",
        root = report.styles[0].element
    )));
}

#[test]
fn stellar_border_is_converted_without_moving_layout_boxes() {
    let dir = project("export-css-border", sd::CSS);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let report = export_web_frame_node(&snapshot, id(1), id(5), None).unwrap();
    let decl = |node: u64, name: &str| {
        report
            .styles
            .iter()
            .find(|s| s.node == id(node))
            .unwrap()
            .declarations
            .iter()
            .find(|d| d.name == name)
            .map(|d| (d.value.clone(), d.origin))
    };
    let converted = DeclarationOrigin::Converted;
    assert_eq!(
        decl(31, "box-sizing"),
        Some(("border-box".into(), converted))
    );
    assert_eq!(decl(31, "width"), Some(("108px".into(), converted)));
    assert_eq!(
        decl(31, "min-width"),
        Some(("calc(50% + 8px)".into(), converted))
    );
    assert_eq!(decl(31, "padding-left"), Some(("1px".into(), converted)));
    assert_eq!(decl(31, "padding-right"), Some(("0px".into(), converted)));
    assert_eq!(decl(30, "padding-top"), Some(("4px".into(), converted)));
    assert_eq!(
        decl(30, "box-sizing"),
        Some(("border-box".into(), converted))
    );
    // 테두리 없는 자식은 보정하지 않습니다.
    assert_eq!(decl(32, "box-sizing"), None);
    let notes: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "approximated-border-padding")
        .collect();
    assert_eq!(notes.len(), 3, "{notes:#?}");
    assert!(notes.iter().all(|d| d.node == id(30) && !d.error));
}

#[test]
fn placements_follow_parent_display_and_explicit_position() {
    let dir = project("export-css-placement", sd::CSS);
    let report = export(&dir, 3);
    let placement = |n: u64| {
        report
            .styles
            .iter()
            .find(|s| s.node == id(n))
            .unwrap()
            .placement
    };
    assert_eq!(placement(10), Placement::Root);
    assert_eq!(placement(11), Placement::Flow);
    assert_eq!(placement(12), Placement::Flow);
    // 12는 display를 명시했으므로 그 자식은 block 흐름 item입니다.
    assert_eq!(placement(16), Placement::Flow);
    assert_eq!(placement(13), Placement::Flow);
    assert_eq!(placement(14), Placement::Inset);
    let hidden = report.styles.iter().find(|s| s.node == id(15)).unwrap();
    assert!(hidden
        .declarations
        .iter()
        .any(|d| d.name == "display" && d.value == "none"));
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == "layout-not-on-canvas"));
    // 흐름 item의 transform 이동은 시각 이동으로 matrix에 남습니다.
    let moved = report.styles.iter().find(|s| s.node == id(12)).unwrap();
    assert_eq!(moved.paint_translation, (3.0, 4.0));
    assert!(report
        .html
        .unwrap()
        .contains("transform: matrix(1, 0, 0, 1, 3, 4);"));
}

#[test]
fn react_native_output_states_css_set_differences() {
    let dir = project("export-css-rn", sd::CSS);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let rn = export_react_native(&snapshot, id(1), id(3), None).unwrap();
    let source = rn.source.as_deref().unwrap();
    let block = |node: u64| {
        let key = style_key(&web_layout_element(&rn.web, node));
        let start = source.find(&format!("  {key}: {{")).unwrap();
        let end = source[start..]
            .find("  },\n")
            .map_or(source.len(), |e| start + e);
        source[start..end].to_string()
    };
    let plain = block(11);
    // css 세트의 생략값이 RN 기본값과 다른 항목을 명시합니다.
    // static은 Web 투영처럼 relative(RN 기본값)로 두어 항목을 쓰지 않습니다. absolute 자식의 containing block이
    // Web 출력과 같게 부모 요소로 남습니다.
    assert!(!plain.contains("position:"), "{plain}");
    assert!(!source.contains("'static'"), "{source}");
    assert!(plain.contains("    boxSizing: 'content-box',"), "{plain}");
    assert!(plain.contains("    flexShrink: 1,"), "{plain}");
    assert!(plain.contains("    width: 100,"), "{plain}");
    let root = block(10);
    assert!(root.contains("    display: 'flex',"));
    assert!(root.contains("    flexDirection: 'row',"));
    assert!(root.contains("    columnGap: 8,"));
    // block 부모(12)의 자식 16에는 flex item 속성을 내지 않습니다.
    assert!(!block(16).contains("flexShrink"));
    assert!(block(12).contains("    transform: [{ translateX: 3 }, { translateY: 4 }],"));
    assert!(rn
        .diagnostics
        .iter()
        .any(|d| d.code == "approximated-react-native" && d.message.contains("min-width")));
    // react-native 세트에서는 차이가 없어 같은 Frame에 크기만 남습니다.
    let dir = project("export-css-rn-rn", sd::REACT_NATIVE);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let rn = export_react_native(&snapshot, id(1), id(3), None).unwrap();
    let key = style_key(&web_layout_element(&rn.web, 11));
    let source = rn.source.unwrap();
    assert!(source.contains(&format!(
        "  {key}: {{\n    width: 100,\n    height: 50,\n  }},\n"
    )));
}

fn web_layout_element(report: &WebFrameHtml, node: u64) -> String {
    report
        .styles
        .iter()
        .find(|s| s.node == id(node))
        .unwrap()
        .element
        .clone()
}

const PADDING_OF: [L; 4] = [
    L::PaddingTop,
    L::PaddingRight,
    L::PaddingBottom,
    L::PaddingLeft,
];
