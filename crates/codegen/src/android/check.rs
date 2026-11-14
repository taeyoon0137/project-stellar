//! 생성 Kotlin(Compose) 소스의 정적 검사 (XXII.4.2).
//!
//! Kotlin 컴파일러 없이 확인할 수 있는 것을 검사합니다.
//!
//! | 검사 | issue |
//! |---|---|
//! | 첫 줄이 `package a.b`가 아님 | [`ComposeIssue::MissingPackage`] |
//! | 문자열 밖 `()`·`[]`·`{}` 짝 불일치 | [`ComposeIssue::Unbalanced`] |
//! | 줄이 끝날 때까지 닫히지 않은 문자열 | [`ComposeIssue::UnterminatedString`] |
//! | API 표에 없는 import, 중복 import, 정렬되지 않은 import, 쓰지 않는 import | [`ComposeIssue::UnknownImport`] 등 |
//! | 표의 symbol을 사용 형태(top-level·확장·semantics 문장·위임)대로 쓰는데 import 없음 | [`ComposeIssue::MissingImport`] |
//! | profile Compose UI 버전보다 새 API | [`ComposeIssue::RequiresNewerCompose`] |
//! | Flow layout을 쓰는데 `@OptIn(ExperimentalLayoutApi::class)` 없음 | [`ComposeIssue::MissingOptIn`] |
//!
//! symbol 사용 형태는 단순 tokenizer로 판정합니다: `.이름`은 확장, 문장 첫머리(`{`·`;`·줄 시작 뒤)의 `이름 =`은 semantics
//! 속성, 그 밖의 이름은 top-level입니다. 인자 목록 안의 `이름 =`(named argument)은 symbol이 아닙니다.

use std::collections::BTreeSet;

use crate::android::api::{by_import, lookup, ApiKind, COMPOSE_API};

/// 정적 검사 issue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComposeIssue {
    /// package 선언 없음.
    MissingPackage,
    /// 괄호 짝 불일치(1부터 센 줄 번호).
    Unbalanced {
        /// 줄.
        line: usize,
        /// 문제 문자.
        delimiter: char,
    },
    /// 닫히지 않은 문자열.
    UnterminatedString {
        /// 줄.
        line: usize,
    },
    /// 표에 없는 import.
    UnknownImport(String),
    /// 중복 import.
    DuplicateImport(String),
    /// import가 정렬되어 있지 않음.
    UnsortedImports,
    /// 쓰지 않는 import.
    UnusedImport(String),
    /// import 없이 쓴 symbol.
    MissingImport(&'static str),
    /// profile Compose UI 버전보다 새 API.
    RequiresNewerCompose {
        /// symbol.
        symbol: &'static str,
        /// 최소 버전.
        since: (u16, u16),
    },
    /// opt-in 없이 쓴 실험 API.
    MissingOptIn(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Punct(char),
    Literal,
}

/// 문자열·수 literal을 건너뛴 token 목록과 각 token이 문장 첫머리인지.
fn tokenize(code: &str) -> Vec<(Token, bool)> {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut statement_start = true;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            statement_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            tokens.push((Token::Literal, statement_start));
            statement_start = false;
            continue;
        }
        if c.is_ascii_digit() {
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'f' || chars[i] == 'F') {
                i += 1;
            }
            tokens.push((Token::Literal, statement_start));
            statement_start = false;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            tokens.push((Token::Ident(name), statement_start));
            statement_start = false;
            continue;
        }
        tokens.push((Token::Punct(c), statement_start));
        statement_start = matches!(c, '{' | ';');
        i += 1;
    }
    tokens
}

/// 코드(import 줄 제외)가 사용하는 API symbol.
pub(crate) fn used_symbols(code: &str) -> BTreeSet<&'static str> {
    let tokens = tokenize(code);
    let mut used = BTreeSet::new();
    for (index, (token, statement_start)) in tokens.iter().enumerate() {
        let Token::Ident(name) = token else {
            continue;
        };
        let previous = index.checked_sub(1).map(|i| &tokens[i].0);
        let next = tokens.get(index + 1).map(|t| &t.0);
        let after_next = tokens.get(index + 2).map(|t| &t.0);
        if name == "by" && matches!(next, Some(Token::Ident(n)) if n == "remember") {
            used.insert("getValue");
            used.insert("setValue");
            continue;
        }
        let Some(entry) = lookup(name) else {
            continue;
        };
        let after_dot = previous == Some(&Token::Punct('.'));
        let assignment = next == Some(&Token::Punct('=')) && after_next != Some(&Token::Punct('='));
        let named_argument =
            assignment && matches!(previous, Some(Token::Punct('(')) | Some(Token::Punct(',')));
        let matched = match entry.kind {
            ApiKind::Extension => after_dot,
            ApiKind::SemanticsProperty => *statement_start && assignment,
            ApiKind::SemanticsFunction => !after_dot && next == Some(&Token::Punct('(')),
            ApiKind::TopLevel => !after_dot && !named_argument,
            ApiKind::Delegate => false,
        };
        if matched {
            used.insert(entry.name);
        }
    }
    used
}

fn is_identifier(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn check_delimiters(source: &str, issues: &mut Vec<ComposeIssue>) {
    let mut stack: Vec<char> = Vec::new();
    for (number, line) in source.lines().enumerate() {
        let line_number = number + 1;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                i += 1;
                let mut closed = false;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    issues.push(ComposeIssue::UnterminatedString { line: line_number });
                }
                i += 1;
                continue;
            }
            match c {
                '(' | '[' | '{' => stack.push(c),
                ')' | ']' | '}' => {
                    let expected = match c {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    if stack.pop() != Some(expected) {
                        issues.push(ComposeIssue::Unbalanced {
                            line: line_number,
                            delimiter: c,
                        });
                        return;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    if let Some(open) = stack.pop() {
        issues.push(ComposeIssue::Unbalanced {
            line: source.lines().count(),
            delimiter: open,
        });
    }
}

/// 정적 검사. issue를 정렬해 돌려줍니다(없으면 빈 목록).
pub fn check_kotlin(source: &str, ui_version: (u16, u16)) -> Vec<ComposeIssue> {
    let mut issues = Vec::new();
    let mut lines = source.lines();
    let package_ok = lines
        .find(|line| !line.trim().is_empty())
        .and_then(|line| line.strip_prefix("package "))
        .is_some_and(|name| name.split('.').all(is_identifier));
    if !package_ok {
        issues.push(ComposeIssue::MissingPackage);
    }
    check_delimiters(source, &mut issues);
    let imports: Vec<&str> = source
        .lines()
        .filter_map(|line| line.strip_prefix("import "))
        .map(str::trim)
        .collect();
    let code: String = source
        .lines()
        .filter(|line| !line.starts_with("import ") && !line.starts_with("package "))
        .collect::<Vec<_>>()
        .join("\n");
    let used = used_symbols(&code);
    let mut seen = BTreeSet::new();
    for import in &imports {
        if !seen.insert(*import) {
            issues.push(ComposeIssue::DuplicateImport(import.to_string()));
            continue;
        }
        match by_import(import) {
            None => issues.push(ComposeIssue::UnknownImport(import.to_string())),
            Some(entry) if !used.contains(entry.name) => {
                issues.push(ComposeIssue::UnusedImport(import.to_string()))
            }
            Some(_) => {}
        }
    }
    if imports.windows(2).any(|pair| pair[0] > pair[1]) {
        issues.push(ComposeIssue::UnsortedImports);
    }
    for entry in COMPOSE_API {
        if !used.contains(entry.name) {
            continue;
        }
        if !imports.contains(&entry.import) {
            issues.push(ComposeIssue::MissingImport(entry.name));
        }
        if entry.since > ui_version {
            issues.push(ComposeIssue::RequiresNewerCompose {
                symbol: entry.name,
                since: entry.since,
            });
        }
    }
    for flow in ["FlowRow", "FlowColumn"] {
        if used.contains(flow) && !code.contains("@OptIn(ExperimentalLayoutApi::class)") {
            if let Some(entry) = lookup(flow) {
                issues.push(ComposeIssue::MissingOptIn(entry.name));
            }
        }
    }
    issues.sort();
    issues.dedup();
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_usage_follows_kind_rules() {
        let code = "Box(\n    modifier = Modifier\n        .width(4.dp)\n        .semantics {\n            role = Role.Image\n            if (x) disabled()\n        }\n        .clickable(role = Role.Button, onClick = { })\n    contentAlignment = Alignment.Center,\n)\nvar a by remember { mutableStateOf(\"selected = 1\") }";
        let used = used_symbols(code);
        let expected: BTreeSet<&str> = [
            "Alignment",
            "Box",
            "Modifier",
            "Role",
            "clickable",
            "disabled",
            "dp",
            "getValue",
            "mutableStateOf",
            "remember",
            "role",
            "semantics",
            "setValue",
            "width",
        ]
        .into_iter()
        .collect();
        assert_eq!(used, expected);
        // 인자 목록 안의 `selected =`·문자열 안 이름은 symbol이 아닙니다.
        assert!(!used_symbols("selectable(selected = true)").contains("selected"));
        assert!(used_symbols("{\n selected = true\n}").contains("selected"));
    }
}
