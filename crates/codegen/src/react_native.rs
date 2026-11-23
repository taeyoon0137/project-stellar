//! React Native 스타일 객체 출력 (IX.11.30, ADR-0089 결정 7).
//!
//! CSS layout longhand 하나를 RN 스타일 항목 하나로 낮추는 규칙([`lower`])과 `StyleSheet.create` 모듈 writer
//! ([`style_sheet`])입니다. 어떤 longhand를 낼지(명시 값 또는 활성 세트와 RN 기본값의 차이)는 상위(ops)가 문서
//! 해석 함수로 정합니다. 이 모듈은 문서·세트 표를 읽지 않습니다.
//!
//! 낮춤 결과는 Target Profile 상태 어휘(`Native`·`Approximated`·`Unsupported`)를 씁니다. RN에는 grid,
//! `justify-items`·`justify-self`, `display: block`, 자동 최소 크기(`min-*: auto`), intrinsic 크기 keyword가
//! 없습니다. 근사할 수 있는 것은 근사 값과 이유를, 없는 것은 이유만 돌려줍니다.

use std::fmt::Write as _;

use stellar_target_profile::capability::CapabilityStatus;

use crate::text::num;

/// 낮출 CSS 값(상위가 문서 정규 값에서 만듭니다).
#[derive(Debug, Clone, PartialEq)]
pub enum RnInput {
    /// keyword.
    Keyword(&'static str),
    /// `<length>`(px).
    Px(f32),
    /// `<percentage>`.
    Percent(f32),
    /// `<number>`.
    Number(f32),
    /// 그 밖의 정규 CSS 텍스트(비율, grid line, track 목록).
    Text(String),
}

/// RN 스타일 값.
#[derive(Debug, Clone, PartialEq)]
pub enum RnValue {
    /// 수(px 또는 무단위).
    Number(f32),
    /// 문자열(keyword, `%`, 비율).
    Text(String),
    /// `transform: [{ translateX }, { translateY }]`.
    Translate(f32, f32),
}

/// 낮춤 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct RnLowering {
    /// Target Profile 상태.
    pub status: CapabilityStatus,
    /// 쓸 값. `None`이면 항목을 쓰지 않습니다(RN 기본값과 같은 뜻이거나 표현할 수 없음).
    pub value: Option<RnValue>,
    /// 근사·미지원 이유.
    pub reason: Option<&'static str>,
}

fn native(value: RnValue) -> RnLowering {
    RnLowering {
        status: CapabilityStatus::Native,
        value: Some(value),
        reason: None,
    }
}

fn native_none() -> RnLowering {
    RnLowering {
        status: CapabilityStatus::Native,
        value: None,
        reason: None,
    }
}

fn approximated(value: Option<RnValue>, reason: &'static str) -> RnLowering {
    RnLowering {
        status: CapabilityStatus::Approximated,
        value,
        reason: Some(reason),
    }
}

fn unsupported(reason: &'static str) -> RnLowering {
    RnLowering {
        status: CapabilityStatus::Unsupported,
        value: None,
        reason: Some(reason),
    }
}

fn text(value: &str) -> RnValue {
    RnValue::Text(value.to_string())
}

/// CSS longhand 이름 → RN 스타일 key(camelCase).
pub fn property_name(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut upper = false;
    for ch in css.chars() {
        if ch == '-' {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

const NO_GRID: &str = "React Native에는 grid 배치가 없습니다";
const NO_JUSTIFY_ITEM: &str = "React Native에는 justify-items·justify-self가 없습니다";
const INTRINSIC: &str =
    "React Native에는 min-content·max-content·fit-content·content 크기 keyword가 없습니다";
const LOGICAL_ALIGN: &str = "React Native에는 start·end 정렬이 없어 flex-start·flex-end로 근사합니다(reverse 방향이면 반대쪽입니다)";
const BLOCK: &str =
    "React Native View는 언제나 flex column이라 block 흐름을 column 배치로 근사합니다";
const AUTO_MIN: &str = "React Native(Yoga)에는 자동 최소 크기(min-*: auto)가 없습니다";

fn size(value: &RnInput, allow_auto: bool) -> RnLowering {
    match value {
        RnInput::Px(v) => native(RnValue::Number(*v)),
        RnInput::Percent(v) => native(RnValue::Text(format!("{}%", num(*v)))),
        RnInput::Keyword("auto") if allow_auto => native(text("auto")),
        _ => unsupported(INTRINSIC),
    }
}

fn alignment(value: &RnInput, normal: &'static str, allowed: &[&'static str]) -> RnLowering {
    match value {
        RnInput::Keyword("normal") => native(text(normal)),
        RnInput::Keyword("start") => approximated(Some(text("flex-start")), LOGICAL_ALIGN),
        RnInput::Keyword("end") => approximated(Some(text("flex-end")), LOGICAL_ALIGN),
        RnInput::Keyword(k) if allowed.contains(k) => native(text(k)),
        _ => unsupported("React Native에 같은 정렬 값이 없습니다"),
    }
}

/// CSS layout longhand 값 하나를 RN 스타일 항목으로 낮춥니다.
pub fn lower(name: &str, value: &RnInput) -> RnLowering {
    match name {
        "grid-template-columns"
        | "grid-template-rows"
        | "grid-auto-columns"
        | "grid-auto-rows"
        | "grid-auto-flow"
        | "grid-column-start"
        | "grid-column-end"
        | "grid-row-start"
        | "grid-row-end" => unsupported(NO_GRID),
        "justify-items" | "justify-self" => unsupported(NO_JUSTIFY_ITEM),
        "display" => match value {
            RnInput::Keyword("flex") => native(text("flex")),
            RnInput::Keyword("none") => native(text("none")),
            RnInput::Keyword("block") => approximated(None, BLOCK),
            _ => unsupported(NO_GRID),
        },
        "position" | "box-sizing" | "flex-direction" | "flex-wrap" => match value {
            RnInput::Keyword(k) => native(text(k)),
            _ => unsupported("keyword가 아닙니다"),
        },
        "top" | "right" | "bottom" | "left" => match value {
            RnInput::Keyword("auto") => native_none(),
            other => size(other, false),
        },
        "width" | "height" | "flex-basis" => size(value, true),
        "min-width" | "min-height" => match value {
            RnInput::Keyword("auto") => approximated(None, AUTO_MIN),
            other => size(other, false),
        },
        "max-width" | "max-height" => match value {
            RnInput::Keyword("none") => native_none(),
            other => size(other, false),
        },
        "aspect-ratio" => match value {
            RnInput::Keyword("auto") => native_none(),
            RnInput::Text(t) => native(RnValue::Text(t.clone())),
            RnInput::Number(n) => native(RnValue::Number(*n)),
            _ => unsupported("비율이 아닙니다"),
        },
        "flex-grow" | "flex-shrink" => match value {
            RnInput::Number(n) => native(RnValue::Number(*n)),
            _ => unsupported("수가 아닙니다"),
        },
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => match value {
            RnInput::Px(v) => native(RnValue::Number(*v)),
            _ => unsupported("px 길이가 아닙니다"),
        },
        "row-gap" | "column-gap" => match value {
            RnInput::Keyword("normal") => native(RnValue::Number(0.0)),
            RnInput::Px(v) => native(RnValue::Number(*v)),
            _ => unsupported("px 길이가 아닙니다"),
        },
        "justify-content" => alignment(
            value,
            "flex-start",
            &[
                "flex-start",
                "flex-end",
                "center",
                "space-between",
                "space-around",
                "space-evenly",
            ],
        )
        .stretch_as_start(value),
        "align-content" => alignment(
            value,
            "stretch",
            &[
                "flex-start",
                "flex-end",
                "center",
                "stretch",
                "space-between",
                "space-around",
                "space-evenly",
            ],
        ),
        "align-items" => alignment(
            value,
            "stretch",
            &["flex-start", "flex-end", "center", "stretch", "baseline"],
        ),
        "align-self" => alignment(
            value,
            "stretch",
            &[
                "auto",
                "flex-start",
                "flex-end",
                "center",
                "stretch",
                "baseline",
            ],
        ),
        _ => unsupported("React Native 낮춤 규칙이 없는 속성입니다"),
    }
}

impl RnLowering {
    /// flex container의 `justify-content: stretch`는 CSS에서 `flex-start`처럼 동작합니다.
    fn stretch_as_start(self, value: &RnInput) -> RnLowering {
        if *value == RnInput::Keyword("stretch") {
            native(text("flex-start"))
        } else {
            self
        }
    }
}

/// 요소 하나의 스타일 객체.
#[derive(Debug, Clone, PartialEq)]
pub struct RnStyle {
    /// `StyleSheet` key(식별자).
    pub key: String,
    /// `(RN key, 값)`(출력 순서).
    pub entries: Vec<(String, RnValue)>,
}

/// DOM id(`s-<hex>`)를 `StyleSheet` key로 바꿉니다(`-` → `_`).
pub fn style_key(element_id: &str) -> String {
    element_id.replace('-', "_")
}

fn quote(text: &str) -> String {
    let mut out = String::from("'");
    for ch in text.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn value_text(value: &RnValue) -> String {
    match value {
        RnValue::Number(n) => num(*n),
        RnValue::Text(t) => quote(t),
        RnValue::Translate(x, y) => {
            format!(
                "[{{ translateX: {} }}, {{ translateY: {} }}]",
                num(*x),
                num(*y)
            )
        }
    }
}

/// `StyleSheet.create` 모듈 텍스트. 같은 입력은 같은 bytes입니다.
pub fn style_sheet(styles: &[RnStyle]) -> String {
    let mut out = String::from(
        "import { StyleSheet } from 'react-native';\n\nexport const styles = StyleSheet.create({\n",
    );
    for style in styles {
        if style.entries.is_empty() {
            let _ = writeln!(out, "  {}: {{}},", style.key);
            continue;
        }
        let _ = writeln!(out, "  {}: {{", style.key);
        for (key, value) in &style.entries {
            let _ = writeln!(out, "    {key}: {},", value_text(value));
        }
        out.push_str("  },\n");
    }
    out.push_str("});\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_camel_case() {
        assert_eq!(property_name("flex-direction"), "flexDirection");
        assert_eq!(property_name("min-width"), "minWidth");
        assert_eq!(property_name("top"), "top");
    }

    #[test]
    fn css_keywords_lower_to_rn_equivalents() {
        let k = RnInput::Keyword;
        assert_eq!(
            lower("justify-content", &k("normal")).value,
            Some(text("flex-start"))
        );
        assert_eq!(
            lower("justify-content", &k("stretch")).value,
            Some(text("flex-start"))
        );
        assert_eq!(
            lower("align-items", &k("normal")).value,
            Some(text("stretch"))
        );
        assert_eq!(
            lower("row-gap", &k("normal")).value,
            Some(RnValue::Number(0.0))
        );
        assert_eq!(
            lower("width", &RnInput::Percent(50.0)).value,
            Some(text("50%"))
        );
        assert_eq!(
            lower("width", &RnInput::Px(12.5)).value,
            Some(RnValue::Number(12.5))
        );
        let start = lower("align-items", &k("start"));
        assert_eq!(start.status, CapabilityStatus::Approximated);
        assert_eq!(start.value, Some(text("flex-start")));
    }

    #[test]
    fn grid_block_and_auto_minimum_are_diagnosed() {
        let k = RnInput::Keyword;
        assert_eq!(
            lower("grid-auto-flow", &k("row")).status,
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            lower("display", &k("grid")).status,
            CapabilityStatus::Unsupported
        );
        let block = lower("display", &k("block"));
        assert_eq!(block.status, CapabilityStatus::Approximated);
        assert_eq!(block.value, None);
        let min = lower("min-width", &k("auto"));
        assert_eq!(min.status, CapabilityStatus::Approximated);
        assert_eq!(
            lower("width", &k("fit-content")).status,
            CapabilityStatus::Unsupported
        );
    }

    #[test]
    fn style_sheet_is_deterministic_and_quotes_text() {
        let styles = [
            RnStyle {
                key: style_key("s-ab"),
                entries: vec![
                    ("flexDirection".into(), text("row")),
                    ("flexShrink".into(), RnValue::Number(1.0)),
                    ("transform".into(), RnValue::Translate(2.0, -3.5)),
                ],
            },
            RnStyle {
                key: "s_cd".into(),
                entries: Vec::new(),
            },
        ];
        let sheet = style_sheet(&styles);
        assert_eq!(
            sheet,
            "import { StyleSheet } from 'react-native';\n\nexport const styles = StyleSheet.create({\n  s_ab: {\n    flexDirection: 'row',\n    flexShrink: 1,\n    transform: [{ translateX: 2 }, { translateY: -3.5 }],\n  },\n  s_cd: {},\n});\n"
        );
        assert_eq!(sheet, style_sheet(&styles));
        assert_eq!(quote("a'b\\"), "'a\\'b\\\\'");
    }
}
