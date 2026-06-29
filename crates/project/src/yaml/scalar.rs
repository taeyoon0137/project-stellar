//! 제한 YAML 스칼라의 엄격한 타입 해석 (IV.1.4).
//!
//! 제한 loader(IV.1.2)는 모든 스칼라를 **문자열로 보존**하며 암묵 타입을 추론하지 않습니다.
//! 타입이 필요한 곳에서는 본 모듈의 엄격 해석기로만 변환합니다. YAML 1.1의 혼동되는 암묵
//! 타입은 **따르지 않습니다**:
//! - 불리언: `yes`/`no`/`on`/`off`/`y`/`n`/`True`/`TRUE` 등은 불리언이 아닙니다. `true`/`false`
//!   소문자 canonical만 불리언입니다.
//! - 정수: 8진수처럼 보이는 leading-zero(`010`), 진법 접두(`0x`,`0o`,`0b`), 부호(`+`),
//!   자릿수 구분 `_`, 소수점은 정수가 아닙니다. canonical base-10만 정수입니다.
//! - 날짜(`2020-01-01`)·timestamp는 타입으로 추론하지 않고 문자열로만 둡니다.
//!
//! 해석에 실패하면 `None`을 반환하므로, 호출부가 "문자열로만 허용" 또는 "명시 오류"를 선택할
//! 수 있습니다.

/// 엄격 불리언 해석. 정확히 `"true"`/`"false"`만 허용합니다.
///
/// YAML 1.1의 `yes`/`no`/`on`/`off`/`y`/`n`이나 대문자 변형(`True`/`TRUE`)은 `None`입니다.
pub fn parse_bool(text: &str) -> Option<bool> {
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 엄격 비음수 정수 해석(canonical base-10). 실패 시 `None`.
///
/// 거부: 빈 문자열, 부호(`+`/`-`), leading-zero(`0` 자체는 허용), 진법 접두(`0x`/`0o`/`0b`),
/// 자릿수 구분 `_`, 소수점·공백 등 비숫자.
pub fn parse_u32(text: &str) -> Option<u32> {
    if !is_canonical_nonneg_decimal(text) {
        return None;
    }
    text.parse::<u32>().ok()
}

/// canonical base-10 비음수 정수 형식인지 검사합니다(값 범위는 검사하지 않음).
fn is_canonical_nonneg_decimal(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // leading zero 거부(단일 "0"은 허용).
    !(text.len() > 1 && text.starts_with('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_accepts_only_canonical() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
    }

    #[test]
    fn bool_rejects_yaml_1_1_confusables() {
        for s in [
            "yes", "no", "on", "off", "y", "n", "Yes", "No", "ON", "OFF", "True", "TRUE", "False",
            "1", "0", "", "truee", " true",
        ] {
            assert_eq!(parse_bool(s), None, "입력: {s:?}");
        }
    }

    #[test]
    fn u32_accepts_canonical_decimal() {
        assert_eq!(parse_u32("0"), Some(0));
        assert_eq!(parse_u32("1"), Some(1));
        assert_eq!(parse_u32("123"), Some(123));
        assert_eq!(parse_u32("4294967295"), Some(u32::MAX));
    }

    #[test]
    fn u32_rejects_yaml_1_1_and_nonstandard_forms() {
        for s in [
            "",           // 빈 문자열
            "01",         // leading zero(8진수 혼동)
            "007",        // leading zero
            "0x1F",       // 16진수
            "0o17",       // 8진수 접두
            "0b101",      // 2진수 접두
            "+5",         // 부호
            "-5",         // 음수
            "1_000",      // 자릿수 구분
            "1.0",        // 소수
            " 1",         // 선행 공백
            "1 ",         // 후행 공백
            "yes",        // 불리언 혼동
            "4294967296", // u32 범위 초과
        ] {
            assert_eq!(parse_u32(s), None, "입력: {s:?}");
        }
    }
}
