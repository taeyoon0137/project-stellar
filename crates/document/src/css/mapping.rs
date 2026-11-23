//! registry 항목별 CSS 매핑 metadata (IX.11.1, 속성 모델 1장, ADR-0075 결정 4).
//!
//! Frame·Shape·Path·Text·Image·Modifier registry 항목은 각자 [`CssMapping`] 하나를 가집니다. 필드는 대응 CSS
//! longhand 이름 목록, 값 문법 종류, 매핑 상태([`MappingStatus`]), `initial-differs`·`subset` 표시, 차이 설명,
//! 변형별 변환 규칙입니다. 저장 bytes와 무관한 metadata입니다(ADR-0024 결정 2).
//!
//! 매핑이 쓰는 CSS 이름은 모두 [`CSS_LONGHAND_NAMES`] inventory에 있어야 합니다(test). shorthand와 그 longhand
//! 펼침은 [`CSS_SHORTHANDS`]입니다(속성 모델 11장).

use super::grammar::CssGrammar;

/// 매핑 상태(속성 모델 1.1절).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingStatus {
    /// 같은 이름·값·의미. Web export는 그대로 출력합니다.
    Exact,
    /// 결정론적 변환으로 같은 결과(이름·값 형태가 다름).
    Converted,
    /// CSS로 같은 결과를 보장할 수 없음(근사 출력 + 진단).
    Approximated,
    /// 대응 CSS 속성이 없음(구조 변환·path 굽기·래스터화 또는 출력 없음).
    StellarOnly,
}

impl MappingStatus {
    /// 정의된 모든 상태.
    pub const ALL: [MappingStatus; 4] = [
        MappingStatus::Exact,
        MappingStatus::Converted,
        MappingStatus::Approximated,
        MappingStatus::StellarOnly,
    ];

    /// 속성 모델 표기(`exact`·`converted`·`approximated`·`stellar-only`).
    pub fn name(self) -> &'static str {
        match self {
            MappingStatus::Exact => "exact",
            MappingStatus::Converted => "converted",
            MappingStatus::Approximated => "approximated",
            MappingStatus::StellarOnly => "stellar-only",
        }
    }

    /// 표기에서 찾습니다.
    pub fn from_name(name: &str) -> Option<MappingStatus> {
        MappingStatus::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// 값 변형 하나의 CSS 변환 규칙(예: `clipping`의 FrameBounds → `overflow: clip` …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConversionRule {
    /// 저장 값 변형 이름(소문자 kebab).
    pub variant: &'static str,
    /// 같은 결과를 내는 CSS 선언 텍스트(`;`로 끝나지 않는 `property: value` 목록, `; `로 구분).
    pub css: &'static str,
    /// 이 변형의 매핑 상태.
    pub status: MappingStatus,
}

/// registry 항목 하나의 CSS 매핑.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CssMapping {
    /// 대응 CSS longhand 이름(registry 순서). `stellar-only`면 비어 있을 수 있습니다.
    pub longhands: &'static [&'static str],
    /// 값 문법. 값에 CSS 텍스트 표현이 없으면(참조·구조 값) `None`입니다.
    pub grammar: Option<CssGrammar>,
    /// 매핑 상태.
    pub status: MappingStatus,
    /// 이미 저장되는 속성의 기본값이 CSS initial value와 다름(export는 항상 명시 출력).
    pub initial_differs: bool,
    /// CSS가 받는 값 일부를 Stellar가 거부함(받은 값의 의미는 같음).
    pub subset: bool,
    /// 차이 설명(속성 모델 해당 행 요약). 같으면 빈 문자열.
    pub note: &'static str,
    /// 값 변형별 변환 규칙. 변형마다 CSS 대응이 다를 때만 씁니다.
    pub rules: &'static [ConversionRule],
}

impl CssMapping {
    const fn new(
        status: MappingStatus,
        longhands: &'static [&'static str],
        grammar: Option<CssGrammar>,
    ) -> CssMapping {
        CssMapping {
            longhands,
            grammar,
            status,
            initial_differs: false,
            subset: false,
            note: "",
            rules: &[],
        }
    }

    /// `exact` 매핑.
    pub const fn exact(longhands: &'static [&'static str], grammar: CssGrammar) -> CssMapping {
        CssMapping::new(MappingStatus::Exact, longhands, Some(grammar))
    }

    /// `converted` 매핑.
    pub const fn converted(
        longhands: &'static [&'static str],
        grammar: Option<CssGrammar>,
    ) -> CssMapping {
        CssMapping::new(MappingStatus::Converted, longhands, grammar)
    }

    /// `approximated` 매핑.
    pub const fn approximated(
        longhands: &'static [&'static str],
        grammar: Option<CssGrammar>,
    ) -> CssMapping {
        CssMapping::new(MappingStatus::Approximated, longhands, grammar)
    }

    /// `stellar-only` 매핑. `longhands`는 export가 굽거나 바꿔 쓰는 CSS 대상이 있을 때만 적습니다.
    pub const fn stellar_only(grammar: Option<CssGrammar>) -> CssMapping {
        CssMapping::new(MappingStatus::StellarOnly, &[], grammar)
    }

    /// 차이 설명을 붙입니다.
    pub const fn note(mut self, note: &'static str) -> CssMapping {
        self.note = note;
        self
    }

    /// `initial-differs` 표시.
    pub const fn initial_differs(mut self) -> CssMapping {
        self.initial_differs = true;
        self
    }

    /// `subset` 표시.
    pub const fn subset(mut self) -> CssMapping {
        self.subset = true;
        self
    }

    /// 변형별 변환 규칙.
    pub const fn rules(mut self, rules: &'static [ConversionRule]) -> CssMapping {
        self.rules = rules;
        self
    }

    /// 표시 목록(`initial-differs`, `subset`).
    pub fn flags(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.initial_differs {
            out.push("initial-differs");
        }
        if self.subset {
            out.push("subset");
        }
        out
    }
}

/// Stellar가 대응하는 CSS longhand 이름 inventory(속성 모델 3–9장). 중복이 없고 소문자 kebab 형식입니다.
///
/// registry 매핑이 쓰는 이름과, 아직 영속 record가 없지만 이름이 정해진 레이아웃·Typography longhand를
/// 모두 담습니다. 새 longhand는 여기에 먼저 추가합니다.
pub const CSS_LONGHAND_NAMES: &[&str] = &[
    // 상자·크기
    "display",
    "box-sizing",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "aspect-ratio",
    // 위치·변환
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "z-index",
    "translate",
    "rotate",
    "scale",
    "transform",
    "transform-origin",
    // flex
    "flex-direction",
    "flex-wrap",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "justify-content",
    "align-items",
    "align-self",
    "align-content",
    "row-gap",
    "column-gap",
    // grid
    "grid-template-columns",
    "grid-template-rows",
    "grid-auto-rows",
    "grid-auto-columns",
    "grid-auto-flow",
    "grid-column-start",
    "grid-column-end",
    "grid-row-start",
    "grid-row-end",
    "justify-items",
    "justify-self",
    // 간격
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    // overflow·clip
    "overflow-x",
    "overflow-y",
    "overflow-clip-margin",
    "clip-path",
    // 배경
    "background-color",
    "background-image",
    // 테두리
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
    // SVG 칠하기
    "fill",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-miterlimit",
    "d",
    // 이미지
    "object-fit",
    "object-position",
    "image-rendering",
    // 텍스트
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-stretch",
    "line-height",
    "letter-spacing",
    "text-align",
    "white-space-collapse",
    "text-wrap-mode",
    "text-wrap-style",
    "overflow-wrap",
    "font-feature-settings",
    "font-variation-settings",
    // 효과
    "opacity",
    "mix-blend-mode",
    "isolation",
    "filter",
    "box-shadow",
    "mask-image",
    "mask-type",
];

/// shorthand와 펼친 longhand(속성 모델 11장, CSS 확장 순서).
pub const CSS_SHORTHANDS: &[(&str, &[&str])] = &[
    (
        "padding",
        &[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
    ),
    ("inset", &["top", "right", "bottom", "left"]),
    ("gap", &["row-gap", "column-gap"]),
    ("flex", &["flex-grow", "flex-shrink", "flex-basis"]),
    ("flex-flow", &["flex-direction", "flex-wrap"]),
    ("place-items", &["align-items", "justify-items"]),
    ("place-content", &["align-content", "justify-content"]),
    ("place-self", &["align-self", "justify-self"]),
    (
        "border",
        &[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
    ),
    (
        "border-width",
        &[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
    ),
    (
        "border-style",
        &[
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ],
    ),
    (
        "border-color",
        &[
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
    ),
    (
        "border-radius",
        &[
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ],
    ),
    ("grid-row", &["grid-row-start", "grid-row-end"]),
    ("grid-column", &["grid-column-start", "grid-column-end"]),
    (
        "grid-area",
        &[
            "grid-row-start",
            "grid-column-start",
            "grid-row-end",
            "grid-column-end",
        ],
    ),
    (
        "grid-template",
        &["grid-template-rows", "grid-template-columns"],
    ),
    ("overflow", &["overflow-x", "overflow-y"]),
    ("background", &["background-color"]),
    (
        "font",
        &[
            "font-style",
            "font-weight",
            "font-stretch",
            "font-size",
            "line-height",
            "font-family",
        ],
    ),
    ("white-space", &["white-space-collapse", "text-wrap-mode"]),
    ("text-wrap", &["text-wrap-mode", "text-wrap-style"]),
];

/// CSS 속성 이름 형식(소문자 ASCII, 숫자, 하이픈; 하이픈으로 시작·끝나거나 연속하지 않음)인지.
pub fn is_css_property_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.as_bytes()[0].is_ascii_lowercase()
        && !name.ends_with('-')
        && !name.contains("--")
}

/// `name`이 inventory의 longhand인지.
pub fn is_known_longhand(name: &str) -> bool {
    CSS_LONGHAND_NAMES.contains(&name)
}

/// `name` shorthand가 펼쳐지는 longhand 목록.
pub fn shorthand_longhands(name: &str) -> Option<&'static [&'static str]> {
    CSS_SHORTHANDS
        .iter()
        .find(|(shorthand, _)| *shorthand == name)
        .map(|(_, longhands)| *longhands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn status_names_are_the_four_property_model_terms() {
        // 검증 기준(IX.11.1): 타입 golden.
        let names: Vec<&str> = MappingStatus::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            ["exact", "converted", "approximated", "stellar-only"]
        );
        for status in MappingStatus::ALL {
            assert_eq!(MappingStatus::from_name(status.name()), Some(status));
        }
        assert_eq!(MappingStatus::from_name("partial"), None);
    }

    #[test]
    fn mapping_builders_set_each_field() {
        const RULES: &[ConversionRule] = &[ConversionRule {
            variant: "disabled",
            css: "overflow: visible",
            status: MappingStatus::Exact,
        }];
        const M: CssMapping = CssMapping::converted(&["overflow-x", "overflow-y"], None)
            .initial_differs()
            .subset()
            .note("차이")
            .rules(RULES);
        assert_eq!(M.status, MappingStatus::Converted);
        assert_eq!(M.longhands, &["overflow-x", "overflow-y"]);
        assert_eq!(M.grammar, None);
        assert_eq!(M.flags(), vec!["initial-differs", "subset"]);
        assert_eq!(M.note, "차이");
        assert_eq!(M.rules.len(), 1);
        let exact = CssMapping::exact(&["opacity"], CssGrammar::UnitInterval);
        assert_eq!(exact.status, MappingStatus::Exact);
        assert!(exact.flags().is_empty());
        assert_eq!(
            CssMapping::stellar_only(Some(CssGrammar::Bool)).longhands,
            &[] as &[&str]
        );
    }

    #[test]
    fn longhand_inventory_is_unique_and_well_formed() {
        // 검증 기준(IX.11.1): CSS longhand 이름 inventory의 중복·형식 검사.
        let mut seen = BTreeSet::new();
        for name in CSS_LONGHAND_NAMES {
            assert!(is_css_property_name(name), "형식 위반: {name}");
            assert!(seen.insert(*name), "중복: {name}");
        }
        for (shorthand, longhands) in CSS_SHORTHANDS {
            assert!(is_css_property_name(shorthand), "{shorthand}");
            assert!(
                !is_known_longhand(shorthand),
                "shorthand는 longhand inventory에 없음: {shorthand}"
            );
            assert!(!longhands.is_empty());
            let unique: BTreeSet<&str> = longhands.iter().copied().collect();
            assert_eq!(unique.len(), longhands.len(), "{shorthand}");
            for longhand in *longhands {
                assert!(is_known_longhand(longhand), "{shorthand} → {longhand}");
            }
        }
        let shorthands: BTreeSet<&str> = CSS_SHORTHANDS.iter().map(|(s, _)| *s).collect();
        assert_eq!(shorthands.len(), CSS_SHORTHANDS.len(), "shorthand 중복");
        assert_eq!(
            shorthand_longhands("gap"),
            Some(&["row-gap", "column-gap"][..])
        );
        assert_eq!(shorthand_longhands("width"), None);
        for bad in ["", "Width", "-x", "x-", "a--b", "1px", "a_b"] {
            assert!(!is_css_property_name(bad), "{bad}");
        }
    }
}
