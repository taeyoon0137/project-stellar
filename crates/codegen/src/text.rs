//! 결정론 텍스트 도우미(수 표기, 식별자 정리).

/// `f32`의 최단 왕복 10진 표기(Rust `Display`, 지수 없음). `-0`은 `0`입니다.
pub fn num(value: f32) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    format!("{value}")
}

/// 이름을 ASCII 식별자 조각(`[A-Za-z0-9_]`, 숫자로 시작하지 않음)으로 바꿉니다. 허용하지 않는 문자는 `_`이고,
/// 남는 글자가 없으면 `fallback`을 씁니다. 같은 입력은 항상 같은 결과입니다.
pub fn identifier(name: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("{fallback}_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// 이름 목록에 겹치지 않는 식별자를 붙입니다(입력 순서대로, 겹치면 `_2`, `_3` …).
pub fn unique_identifiers<'a>(
    names: impl IntoIterator<Item = &'a str>,
    fallback: &str,
    reserved: &[&str],
) -> Vec<String> {
    let mut used: std::collections::BTreeSet<String> =
        reserved.iter().map(|word| word.to_string()).collect();
    names
        .into_iter()
        .map(|name| allocate_identifier(&mut used, identifier(name, fallback)))
        .collect()
}

/// 같은 출력 범위에서 만든 식별자까지 고려해 다음 이름을 배정합니다.
pub(crate) fn allocate_identifier(
    used: &mut std::collections::BTreeSet<String>,
    base: String,
) -> String {
    let mut candidate = base.clone();
    let mut suffix = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_use_shortest_decimal_without_exponent() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(-1.25), "-1.25");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(1e-7), "0.0000001");
        assert_eq!(num(16.0), "16");
    }

    #[test]
    fn identifiers_are_ascii_and_unique() {
        assert_eq!(identifier("brand color", "token"), "brand_color");
        assert_eq!(identifier("선택", "state"), "state");
        assert_eq!(identifier("2x", "token"), "token_2x");
        assert_eq!(identifier("--a--b--", "token"), "a_b");
        assert_eq!(
            unique_identifiers(["a", "a", "선택", "", "body"], "state", &["body"]),
            vec!["a", "a_2", "state", "state_2", "body_2"]
        );
    }
}
