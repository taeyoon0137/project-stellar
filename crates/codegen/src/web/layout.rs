//! Web layout CSS 규칙 조립 (IX.11.30, XXII.2.2, ADR-0089).
//!
//! 문서·기본값 세트 표를 읽지 않습니다. 상위(ops)가 문서 해석 함수(`style_defaults::resolve`)와 문서 CSS 값
//! formatter로 만든 `(longhand 이름, 정규 값)` 목록을 넘기고, 이 모듈은 규칙 텍스트만 씁니다. 그래서 값 표기가
//! 두 곳에서 갈라지지 않습니다.
//!
//! - 기반 규칙: 활성 기본값 세트가 대상(Web: `css` 세트) 기본값과 다른 longhand를 특이도 0 규칙
//!   (`:where(#<root>, #<root> *)`)으로 냅니다. 저자 규칙(`#<id>`, 특이도 1-0-0)이 언제나 이깁니다(ADR-0082 결정 6).
//! - 구획(frame·text·image·shape)마다 값이 다른 차이는 구획 class(`stellar-<구획>`)를 붙인 기반 규칙으로 냅니다.
//!   모든 구획에 같은 값인 차이는 공통 규칙 하나에 둡니다.
//! - 차이가 없으면 아무것도 내지 않습니다(빈 문자열, class 속성도 없음).

use std::fmt::Write as _;

/// 구획 class 이름 접두사.
pub const CLASS_PREFIX: &str = "stellar-";

/// 구획 class 이름(`stellar-frame` 등).
pub fn class_name(class: &str) -> String {
    format!("{CLASS_PREFIX}{class}")
}

/// CSS 선언 하나. 값은 상위가 문서 formatter로 만든 정규 텍스트입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// longhand 이름.
    pub name: &'static str,
    /// 정규 CSS 값 텍스트.
    pub value: String,
}

impl Declaration {
    /// 선언을 만듭니다.
    pub fn new(name: &'static str, value: impl Into<String>) -> Declaration {
        Declaration {
            name,
            value: value.into(),
        }
    }
}

/// 구획 하나의 기본값 차이(활성 세트 값, longhand 순서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDefaults {
    /// 구획 이름(`frame` 등).
    pub class: &'static str,
    /// 대상 기본값과 다른 활성 세트 값.
    pub declarations: Vec<Declaration>,
}

/// 기반 규칙 출력.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BaseRules {
    /// 규칙 텍스트(차이가 없으면 빈 문자열).
    pub css: String,
    /// 구획 규칙이 있어 요소에 구획 class를 붙여야 하는지.
    pub needs_class: bool,
    /// 공통 규칙 선언.
    pub common: Vec<Declaration>,
    /// 구획 규칙 선언(구획 순서, 빈 구획 제외).
    pub by_class: Vec<ClassDefaults>,
}

/// 선언 목록을 규칙 본문 줄로 씁니다(`  name: value;`).
pub fn declaration_block(declarations: &[Declaration]) -> String {
    let mut out = String::new();
    for d in declarations {
        let _ = writeln!(out, "  {}: {};", d.name, d.value);
    }
    out
}

/// 내보내는 구획의 차이로 특이도 0 기반 규칙을 만듭니다. `root_id`는 내보내는 root 요소의 DOM id이고,
/// `classes`는 출력에 나오는 구획만 담습니다(순서 유지).
pub fn base_rules(root_id: &str, classes: &[ClassDefaults]) -> BaseRules {
    if classes.iter().all(|c| c.declarations.is_empty()) {
        return BaseRules::default();
    }
    let first = &classes[0];
    let common: Vec<Declaration> = first
        .declarations
        .iter()
        .filter(|d| classes.iter().all(|c| c.declarations.contains(d)))
        .cloned()
        .collect();
    let by_class: Vec<ClassDefaults> = classes
        .iter()
        .map(|c| ClassDefaults {
            class: c.class,
            declarations: c
                .declarations
                .iter()
                .filter(|d| !common.contains(d))
                .cloned()
                .collect(),
        })
        .filter(|c| !c.declarations.is_empty())
        .collect();
    let mut css = String::new();
    if !common.is_empty() {
        let _ = write!(
            css,
            ":where(#{root_id}, #{root_id} *) {{\n{}}}\n",
            declaration_block(&common)
        );
    }
    for c in &by_class {
        let class = class_name(c.class);
        let _ = write!(
            css,
            ":where(#{root_id}.{class}, #{root_id} .{class}) {{\n{}}}\n",
            declaration_block(&c.declarations)
        );
    }
    BaseRules {
        css,
        needs_class: !by_class.is_empty(),
        common,
        by_class,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &'static str, value: &str) -> Declaration {
        Declaration::new(name, value)
    }

    #[test]
    fn no_difference_writes_nothing() {
        let rules = base_rules(
            "s-root",
            &[ClassDefaults {
                class: "frame",
                declarations: Vec::new(),
            }],
        );
        assert_eq!(rules, BaseRules::default());
        assert!(rules.css.is_empty());
        assert!(!rules.needs_class);
    }

    #[test]
    fn shared_values_go_to_common_rule_and_the_rest_to_class_rules() {
        let frame = ClassDefaults {
            class: "frame",
            declarations: vec![
                d("position", "relative"),
                d("display", "flex"),
                d("flex-shrink", "0"),
            ],
        };
        let shape = ClassDefaults {
            class: "shape",
            declarations: vec![d("position", "relative"), d("flex-shrink", "0")],
        };
        let rules = base_rules("s-r", &[frame, shape]);
        assert_eq!(
            rules.css,
            ":where(#s-r, #s-r *) {\n  position: relative;\n  flex-shrink: 0;\n}\n\
             :where(#s-r.stellar-frame, #s-r .stellar-frame) {\n  display: flex;\n}\n"
        );
        assert!(rules.needs_class);
    }

    #[test]
    fn single_class_differences_are_all_common() {
        let rules = base_rules(
            "s-r",
            &[ClassDefaults {
                class: "frame",
                declarations: vec![d("display", "flex")],
            }],
        );
        assert_eq!(rules.css, ":where(#s-r, #s-r *) {\n  display: flex;\n}\n");
        assert!(!rules.needs_class);
    }
}
