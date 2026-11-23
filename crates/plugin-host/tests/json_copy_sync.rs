//! 엄격 JSON reader·writer 사본 일치 검사.
//!
//! `src/json.rs`는 `crates/mcp-core/src/json.rs`의 의도적 사본입니다(module 문서 참조). module
//! 문서(`//!` 줄) 뒤의 본문이 byte 단위로 같아야 합니다. 한쪽만 고치면 이 테스트가 실패합니다.
//! crate 의존이 아니라 소스 파일 비교이므로 plugin host가 MCP crate에 의존하지 않습니다.

/// module 문서(앞쪽 `//!` 줄과 그 뒤 빈 줄)를 뺀 본문.
fn body(text: &str) -> &str {
    let mut rest = text;
    while let Some(line_end) = rest.find('\n') {
        let line = &rest[..line_end];
        if line.starts_with("//!") || line.trim().is_empty() {
            rest = &rest[line_end + 1..];
        } else {
            break;
        }
    }
    rest
}

#[test]
fn json_copies_have_identical_bodies() {
    let own = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/json.rs"));
    let other = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../mcp-core/src/json.rs"
    ));
    let (own, other) = (body(own), body(other));
    assert!(own.starts_with("use "), "본문은 use 선언부터 시작");
    if own != other {
        let line = own
            .lines()
            .zip(other.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| own.lines().count().min(other.lines().count()));
        panic!(
            "두 JSON 사본의 본문이 본문 {}번째 줄부터 다릅니다",
            line + 1
        );
    }
}

#[test]
fn body_skips_only_the_module_doc() {
    assert_eq!(body("//! a\n//!\n\nuse x;\n//! b\n"), "use x;\n//! b\n");
    assert_eq!(body("use x;\n"), "use x;\n");
}
