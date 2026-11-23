//! journal 대상 상대 경로 검사(ADR-0078).
//!
//! 대상은 프로젝트 루트 아래 `/`로 구분한 상대 경로입니다. journal 자체(`.stellar/`)와 Git 내부(`.git`)는
//! 대상이 될 수 없습니다. 복구는 신뢰할 수 없는 journal manifest의 경로도 같은 규칙으로 다시 검사합니다.

/// 경로 전체 bytes 상한.
const PATH_LIMIT: usize = 4096;

/// 구성 요소 하나의 bytes 상한(APFS 이름 상한).
const COMPONENT_LIMIT: usize = 255;

/// 상대 경로를 구성 요소로 나눕니다. 거부하면 이유를 돌려줍니다.
pub(crate) fn components(path: &str) -> Result<Vec<&str>, &'static str> {
    if path.is_empty() {
        return Err("빈 경로입니다");
    }
    if path.len() > PATH_LIMIT {
        return Err("경로가 너무 깁니다");
    }
    if path.starts_with('/') {
        return Err("절대 경로입니다");
    }
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts {
        if part.is_empty() {
            return Err("빈 구성 요소가 있습니다");
        }
        if *part == "." || *part == ".." {
            return Err("`.` 또는 `..` 구성 요소가 있습니다");
        }
        if part.len() > COMPONENT_LIMIT {
            return Err("구성 요소가 너무 깁니다");
        }
        if part.chars().any(|c| c.is_control() || c == '\\') {
            return Err("제어 문자 또는 `\\`가 있습니다");
        }
        if part.eq_ignore_ascii_case(".git") {
            return Err("Git 내부 경로입니다");
        }
    }
    if parts[0].eq_ignore_ascii_case(".stellar") {
        return Err("Stellar 로컬 상태 경로입니다");
    }
    Ok(parts)
}

/// 같은 대상 판정 key. macOS 기본 볼륨은 ASCII 대소문자를 구분하지 않으므로 소문자로 비교합니다.
pub(crate) fn identity_key(path: &str) -> String {
    path.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_project_paths() {
        assert_eq!(
            components("design/views/home.yaml").unwrap(),
            ["design", "views", "home.yaml"]
        );
        assert_eq!(components("a b/c.yaml").unwrap(), ["a b", "c.yaml"]);
        assert!(
            components("design/.stellar/x").is_ok(),
            "루트 밖 위치만 금지"
        );
    }

    #[test]
    fn refuses_escapes_and_reserved_locations() {
        for path in [
            "",
            "/etc/passwd",
            "../outside",
            "design/../../x",
            "design//x",
            "design/./x",
            "design/",
            ".git/hooks/pre-commit",
            "design/.GIT/config",
            ".stellar/journal/x",
            ".Stellar/cache",
            "bad\nname",
            "bad\0name",
            "back\\slash",
        ] {
            assert!(components(path).is_err(), "{path:?}");
        }
        assert!(components(&"a".repeat(256)).is_err());
        assert!(components(&["a"; 3000].join("/")).is_err());
    }
}
