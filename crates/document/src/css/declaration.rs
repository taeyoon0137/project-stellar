//! CSS 선언 블록 텍스트 해석·정규 출력 (IX.11.15, 속성 모델 8.3절).
//!
//! 정규 출력은 `property: value;`를 한 줄에 하나씩 registry 순서로 씁니다([`format_block`]). 해석
//! ([`parse_block`])은 주석(`/* … */`)을 지우고, `selector { … }` 규칙 하나가 오면 중괄호 안만 읽으며, 괄호·따옴표
//! 밖의 `;`로 선언을 나눕니다. 형식이 틀린 선언은 버리지 않고 이유와 함께 돌려줍니다. 블록 길이와 선언 수에
//! 상한이 있고 문법은 재귀가 없습니다.

/// 블록 텍스트 상한(byte).
pub const MAX_BLOCK_LEN: usize = 16 * 1024;
/// 선언 수 상한.
pub const MAX_DECLARATIONS: usize = 256;

/// 선언 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// 속성 이름(소문자, custom property `--x`는 원문 유지).
    pub name: String,
    /// 값 텍스트(앞뒤 공백 제거).
    pub value: String,
}

/// 해석하지 못한 선언 조각.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedDeclaration {
    /// 원문(앞뒤 공백 제거).
    pub text: String,
    /// 진단 코드(`invalid-declaration`, `unsupported-value`).
    pub code: &'static str,
    /// 이유.
    pub message: String,
}

/// 블록의 항목 하나(원문 순서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockItem {
    /// 해석한 선언.
    Declaration(Declaration),
    /// 해석하지 못한 조각.
    Malformed(MalformedDeclaration),
}

/// 블록 전체 오류(상한 초과, 괄호 불일치).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockError {
    /// 진단 코드.
    pub code: &'static str,
    /// 이유.
    pub message: String,
}

fn block_error(message: impl Into<String>) -> BlockError {
    BlockError {
        code: "invalid-declaration-block",
        message: message.into(),
    }
}

fn strip_comments(text: &str) -> Result<String, BlockError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let end = rest[start + 2..]
            .find("*/")
            .ok_or_else(|| block_error("주석이 닫히지 않았습니다"))?;
        out.push(' ');
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `selector { body }` 규칙 하나면 body, 아니면 전체.
fn rule_body(text: &str) -> Result<&str, BlockError> {
    match (text.find('{'), text.rfind('}')) {
        (None, None) => Ok(text),
        (Some(open), Some(close)) if open < close => {
            if text[close + 1..].trim().is_empty() && !text[open + 1..close].contains('{') {
                Ok(&text[open + 1..close])
            } else {
                Err(block_error("규칙은 하나만 받습니다"))
            }
        }
        _ => Err(block_error("중괄호가 맞지 않습니다")),
    }
}

/// 괄호·따옴표 밖의 `;`로 나눕니다.
fn split_declarations(body: &str) -> Result<Vec<&str>, BlockError> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let mut quote: Option<char> = None;
    for (index, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth < 0 {
                    return Err(block_error("닫는 괄호가 남습니다"));
                }
            }
            (None, ';') if depth == 0 => {
                out.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() || depth != 0 {
        return Err(block_error("따옴표나 괄호가 닫히지 않았습니다"));
    }
    out.push(&body[start..]);
    Ok(out)
}

/// 선언 블록 텍스트를 해석합니다.
pub fn parse_block(text: &str) -> Result<Vec<BlockItem>, BlockError> {
    if text.len() > MAX_BLOCK_LEN {
        return Err(block_error(format!(
            "선언 블록은 {MAX_BLOCK_LEN} byte까지입니다"
        )));
    }
    let cleaned = strip_comments(text)?;
    let body = rule_body(&cleaned)?;
    let mut items = Vec::new();
    for piece in split_declarations(body)? {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if items.len() == MAX_DECLARATIONS {
            return Err(block_error(format!(
                "선언은 {MAX_DECLARATIONS}개까지입니다"
            )));
        }
        let malformed = |code, message: &str| {
            BlockItem::Malformed(MalformedDeclaration {
                text: piece.to_string(),
                code,
                message: message.to_string(),
            })
        };
        let Some((name, value)) = piece.split_once(':') else {
            items.push(malformed(
                "invalid-declaration",
                "`속성: 값` 형식이 아닙니다",
            ));
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        let valid_name = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii());
        if !valid_name {
            items.push(malformed("invalid-declaration", "속성 이름이 아닙니다"));
            continue;
        }
        if value.is_empty() {
            items.push(malformed("invalid-declaration", "값이 비어 있습니다"));
            continue;
        }
        if value.to_ascii_lowercase().ends_with("!important") {
            items.push(malformed(
                "unsupported-value",
                "문서 모델에는 cascade가 없어 !important를 받지 않습니다",
            ));
            continue;
        }
        let name = if name.starts_with("--") {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        items.push(BlockItem::Declaration(Declaration {
            name,
            value: value.to_string(),
        }));
    }
    Ok(items)
}

/// 정규 선언 블록: `property: value;`를 한 줄에 하나씩(입력 순서 = registry 순서).
pub fn format_block<'a>(declarations: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (name, value) in declarations {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str(";\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, value: &str) -> BlockItem {
        BlockItem::Declaration(Declaration {
            name: name.into(),
            value: value.into(),
        })
    }

    #[test]
    fn blocks_parse_with_comments_rules_and_parenthesized_semicolons() {
        let items = parse_block(
            ".card { /* 배경 */ Background-Color: rgb(0 0 0 / 50%);\n width:320px ; ; --Brand: url(\"a;b\") }",
        )
        .unwrap();
        assert_eq!(
            items,
            vec![
                decl("background-color", "rgb(0 0 0 / 50%)"),
                decl("width", "320px"),
                decl("--Brand", "url(\"a;b\")"),
            ]
        );
        let items = parse_block("width 3px; height: ; color: red !important; a.b: 1").unwrap();
        let codes: Vec<&str> = items
            .iter()
            .map(|i| match i {
                BlockItem::Malformed(m) => m.code,
                BlockItem::Declaration(_) => "ok",
            })
            .collect();
        assert_eq!(
            codes,
            [
                "invalid-declaration",
                "invalid-declaration",
                "unsupported-value",
                "invalid-declaration"
            ]
        );
    }

    #[test]
    fn block_level_errors_and_limits() {
        assert!(parse_block("a: b /* x").is_err());
        assert!(parse_block("a { b: c } d { e: f }").is_err());
        assert!(parse_block("a: calc(1px").is_err());
        assert!(parse_block(&"x".repeat(MAX_BLOCK_LEN + 1)).is_err());
        assert!(parse_block(&"a: b;".repeat(MAX_DECLARATIONS + 1)).is_err());
        assert_eq!(
            parse_block(&"a: b;".repeat(MAX_DECLARATIONS))
                .unwrap()
                .len(),
            MAX_DECLARATIONS
        );
    }

    #[test]
    fn format_block_writes_one_declaration_per_line() {
        let text = format_block([("width", "320px"), ("background-color", "#ff0000")]);
        assert_eq!(text, "width: 320px;\nbackground-color: #ff0000;\n");
        let items = parse_block(&text).unwrap();
        assert_eq!(
            items,
            vec![decl("width", "320px"), decl("background-color", "#ff0000")]
        );
    }
}
