//! Cargo manifest 내부 의존 추출 (XXIX.1.1).
//!
//! crate graph 검사에 필요한 부분만 읽는 제한 TOML 해석기입니다. `[package].name`과 다음 표의
//! 의존 이름을 추출합니다.
//!
//! - `[dependencies]`, `[build-dependencies]`, `[dev-dependencies]`(밑줄 표기 포함)
//! - `[target.<spec>.dependencies]` 등 target 한정 표
//! - `[dependencies.<name>]` 형태의 하위 표와 `<name>.path = ...` 같은 dotted key
//! - 의존 표 자체나 그 상위 표를 inline table로 쓴 형태(`dev-dependencies = { .. }`,
//!   `[target.'cfg(unix)']` 아래 `dependencies = { .. }`, `[target]` 아래 `'cfg(unix)' = { .. }`,
//!   `target = { .. }`, `workspace = { dependencies = { .. } }`)
//! - `package = "..."` 이름 변경과 `[workspace.dependencies]`를 상속하는 `workspace = true`
//!
//! 문자열·배열·inline table·주석을 구분해 문장 단위로 나누므로, 여러 줄 배열이나 문자열 안의
//! `stellar-*` 텍스트를 의존으로 오인하지 않습니다. 해석할 수 없는 구문(값 하나 뒤에 다른 내용이
//! 붙은 문장, 의존 표 위치에 표가 아닌 값 등)은 오류로 거부합니다(fail closed). 입력 크기는
//! [`MAX_MANIFEST_BYTES`]로 제한하고 모든 처리는 반복문입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// manifest 한 개의 최대 크기(bytes). 넘으면 `manifest-too-large`로 거부합니다.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// 내부(workspace 제품) crate 이름 prefix.
pub const INTERNAL_PREFIX: &str = "stellar-";

/// 의존 표 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepKind {
    /// `[dependencies]`
    Normal,
    /// `[build-dependencies]`
    Build,
    /// `[dev-dependencies]`
    Dev,
}

impl DepKind {
    /// 표 이름(정규 표기).
    pub fn table_name(self) -> &'static str {
        match self {
            DepKind::Normal => "dependencies",
            DepKind::Build => "build-dependencies",
            DepKind::Dev => "dev-dependencies",
        }
    }

    /// 이 crate에 의존하는 crate를 빌드할 때도 함께 빌드되는 종류인지 여부입니다.
    /// dev 의존은 그 crate 자신의 test·example·bench에서만 쓰이므로 전이되지 않습니다.
    pub fn propagates(self) -> bool {
        !matches!(self, DepKind::Dev)
    }

    fn from_segment(segment: &str) -> Option<DepKind> {
        match segment {
            "dependencies" => Some(DepKind::Normal),
            "build-dependencies" | "build_dependencies" => Some(DepKind::Build),
            "dev-dependencies" | "dev_dependencies" => Some(DepKind::Dev),
            _ => None,
        }
    }
}

/// 의존이 선언된 표(종류 + target spec).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DepTable {
    pub kind: DepKind,
    /// `[target.<spec>.*]` 표의 spec(따옴표 제거). 일반 표면 `None`.
    pub target: Option<String>,
}

impl DepTable {
    /// 진단용 표 표기(예: `[dev-dependencies]`, `[target.'cfg(unix)'.dependencies]`).
    pub fn label(&self) -> String {
        match &self.target {
            None => format!("[{}]", self.kind.table_name()),
            Some(spec) => format!("[target.'{spec}'.{}]", self.kind.table_name()),
        }
    }
}

/// manifest 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub code: &'static str,
    pub message: String,
}

impl ManifestError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ManifestError {
            code,
            message: message.into(),
        }
    }

    fn at(code: &'static str, line: usize, message: impl Into<String>) -> Self {
        ManifestError::new(code, format!("{line}행: {}", message.into()))
    }
}

/// manifest에 선언된 의존 하나(이름 변경·상속 해석 전).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeclaredDep {
    /// 표에 쓰인 key.
    pub key: String,
    /// `package = "..."`로 지정한 실제 package 이름.
    pub package: Option<String>,
    /// `workspace = true`로 `[workspace.dependencies]`를 상속하는지 여부.
    pub inherits_workspace: bool,
    pub table: DepTable,
}

impl DeclaredDep {
    /// 실제 package 이름(이름 변경과 workspace 상속 반영).
    pub fn resolved_name(&self, workspace_renames: &BTreeMap<String, String>) -> String {
        if let Some(package) = &self.package {
            return package.clone();
        }
        if self.inherits_workspace {
            if let Some(package) = workspace_renames.get(&self.key) {
                return package.clone();
            }
        }
        self.key.clone()
    }
}

/// 해석된 manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedManifest {
    /// `[package].name`. virtual manifest면 `None`.
    pub package: Option<String>,
    /// 모든 의존 표의 의존(외부 포함), 정렬·중복 제거.
    pub deps: Vec<DeclaredDep>,
    /// `[workspace.dependencies]`에서 `package = "..."`로 이름을 바꾼 항목(key → package).
    pub workspace_renames: BTreeMap<String, String>,
}

/// workspace 내부 의존 edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManifestEdge {
    pub from: String,
    pub to: String,
    pub table: DepTable,
}

/// 검사 대상 manifest 전체에서 모은 package와 내부 의존 edge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceGraph {
    /// 검사 대상 package 이름(정렬).
    pub packages: Vec<String>,
    /// 내부(`stellar-*`) 의존 edge(정렬·중복 제거).
    pub edges: Vec<ManifestEdge>,
}

/// 해석 중 문장.
enum Statement {
    Header { array: bool, path: Vec<String> },
    KeyValue { key: Vec<String>, value: String },
}

/// 문자 단위 lexer 상태.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    Basic,
    Literal,
    MultiBasic,
    MultiLiteral,
    Comment,
}

/// manifest 문자열을 해석합니다.
pub fn parse_manifest(text: &str) -> Result<ParsedManifest, ManifestError> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::new(
            "manifest-too-large",
            format!("manifest가 {MAX_MANIFEST_BYTES} bytes를 넘습니다"),
        ));
    }
    let mut parsed = ParsedManifest::default();
    let mut deps: BTreeMap<(DepTable, String), DeclaredDep> = BTreeMap::new();
    let mut header: Vec<String> = Vec::new();
    let mut in_array_table = false;

    for (line, statement) in statements(text)? {
        match statement {
            Statement::Header { array, path } => {
                in_array_table = array;
                header = path;
                if !in_array_table {
                    if let Some((table, key, _rest)) = dep_location(&header) {
                        dep_entry(&mut deps, table, key);
                    }
                }
            }
            Statement::KeyValue { key, value } => {
                if in_array_table {
                    continue;
                }
                let mut full = header.clone();
                full.extend(key);
                apply_statement(&mut parsed, &mut deps, full, value, line)?;
            }
        }
    }
    parsed.deps = deps.into_values().collect();
    Ok(parsed)
}

/// key-value 문장 하나를 적용합니다.
///
/// 경로가 의존 표 자체나 그 상위 표([`is_table_container`])이면 값은 inline table이어야 하며,
/// 항목마다 경로를 이어 붙여 같은 규칙으로 다시 적용합니다. 재귀 대신 명시적 작업 목록을 씁니다.
fn apply_statement(
    parsed: &mut ParsedManifest,
    deps: &mut BTreeMap<(DepTable, String), DeclaredDep>,
    full: Vec<String>,
    value: String,
    line: usize,
) -> Result<(), ManifestError> {
    check_single_value(&value, line)?;
    let mut pending = vec![(full, value)];
    while let Some((full, value)) = pending.pop() {
        if !is_table_container(&full) {
            apply_key_value(parsed, deps, &full, &value, line)?;
            continue;
        }
        let entries = inline_table_entries(&value, line)?.ok_or_else(|| {
            ManifestError::at(
                "manifest-syntax",
                line,
                format!("`{}` 위치의 값은 표여야 합니다", full.join(".")),
            )
        })?;
        // 선언 순서대로 처리하도록 뒤에서부터 넣습니다.
        for (key, entry) in entries.into_iter().rev() {
            let mut next = full.clone();
            next.extend(key);
            pending.push((next, entry));
        }
    }
    Ok(())
}

/// 의존 표 자체(`dependencies`, `target.<spec>.dependencies` 등)이거나 의존 표를 담는 상위 표
/// (`target`, `target.<spec>`, `workspace`, `workspace.dependencies`, `package`)의 경로인지 여부.
///
/// 이 경로에 값으로 온 inline table은 하위 표와 같으므로 펼쳐 읽어야 합니다. 건너뛰면
/// `dev-dependencies = { stellar-ffi = { .. } }` 같은 유효한 manifest의 의존을 놓칩니다.
fn is_table_container(path: &[String]) -> bool {
    match path {
        [single] => {
            DepKind::from_segment(single).is_some()
                || matches!(single.as_str(), "target" | "workspace" | "package")
        }
        [first, second] => first == "target" || (first == "workspace" && second == "dependencies"),
        [target, _spec, kind] => target == "target" && DepKind::from_segment(kind).is_some(),
        _ => false,
    }
}

/// 의존 위치(표, key, 나머지 key 경로)를 찾습니다.
fn dep_location(path: &[String]) -> Option<(DepTable, &str, &[String])> {
    match path {
        [table, key, rest @ ..] => DepKind::from_segment(table)
            .map(|kind| (DepTable { kind, target: None }, key.as_str(), rest)),
        _ => None,
    }
    .or_else(|| match path {
        [target, spec, table, key, rest @ ..] if target == "target" => DepKind::from_segment(table)
            .map(|kind| {
                (
                    DepTable {
                        kind,
                        target: Some(spec.clone()),
                    },
                    key.as_str(),
                    rest,
                )
            }),
        _ => None,
    })
}

fn dep_entry<'a>(
    deps: &'a mut BTreeMap<(DepTable, String), DeclaredDep>,
    table: DepTable,
    key: &str,
) -> &'a mut DeclaredDep {
    deps.entry((table.clone(), key.to_string()))
        .or_insert_with(|| DeclaredDep {
            key: key.to_string(),
            package: None,
            inherits_workspace: false,
            table,
        })
}

fn apply_key_value(
    parsed: &mut ParsedManifest,
    deps: &mut BTreeMap<(DepTable, String), DeclaredDep>,
    full: &[String],
    value: &str,
    line: usize,
) -> Result<(), ManifestError> {
    if full == ["package", "name"] {
        let name = string_value(value, line)?.ok_or_else(|| {
            ManifestError::at("package-name", line, "[package].name은 문자열이어야 합니다")
        })?;
        parsed.package = Some(name);
        return Ok(());
    }
    if let [workspace, dependencies, key, rest @ ..] = full {
        if workspace == "workspace" && dependencies == "dependencies" {
            if let Some(package) = package_override(rest, value, line)? {
                parsed.workspace_renames.insert(key.clone(), package);
            }
            return Ok(());
        }
    }
    let Some((table, key, rest)) = dep_location(full) else {
        return Ok(());
    };
    let package = package_override(rest, value, line)?;
    let inherits = inherits_workspace(rest, value, line)?;
    let entry = dep_entry(deps, table, key);
    if package.is_some() {
        entry.package = package;
    }
    entry.inherits_workspace |= inherits;
    Ok(())
}

/// `package` 지정(`rest == ["package"]` 또는 inline table의 `package` key)을 읽습니다.
fn package_override(
    rest: &[String],
    value: &str,
    line: usize,
) -> Result<Option<String>, ManifestError> {
    let raw = match rest {
        [] => inline_table_value(value, "package", line)?,
        [key] if key == "package" => Some(value.trim().to_string()),
        _ => None,
    };
    match raw {
        None => Ok(None),
        Some(raw) => string_value(&raw, line)?.map(Some).ok_or_else(|| {
            ManifestError::at(
                "dependency-package",
                line,
                "package 값은 문자열이어야 합니다",
            )
        }),
    }
}

/// `workspace = true` 상속 여부를 읽습니다.
fn inherits_workspace(rest: &[String], value: &str, line: usize) -> Result<bool, ManifestError> {
    let raw = match rest {
        [] => inline_table_value(value, "workspace", line)?,
        [key] if key == "workspace" => Some(value.trim().to_string()),
        _ => None,
    };
    Ok(raw.is_some_and(|raw| raw.trim() == "true"))
}

/// inline table 항목 목록: (dotted key 경로, 값 원문).
type InlineEntries = Vec<(Vec<String>, String)>;

/// inline table(`{ a = 1, b.c = "x" }`)의 항목(dotted key 경로, 값 원문)을 선언 순서대로
/// 읽습니다. 값이 inline table이 아니면 `None`. 각 항목 값도 [`check_single_value`]로 검사합니다.
fn inline_table_entries(value: &str, line: usize) -> Result<Option<InlineEntries>, ManifestError> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Ok(None);
    };
    let mut entries = Vec::new();
    for piece in split_top_level(inner, ',') {
        if piece.trim().is_empty() {
            continue;
        }
        let Some((key, raw)) = split_key_value(&piece) else {
            return Err(ManifestError::at(
                "manifest-syntax",
                line,
                "inline table 항목은 key = value여야 합니다",
            ));
        };
        check_single_value(&raw, line)?;
        entries.push((parse_dotted_key(&key, line)?, raw.trim().to_string()));
    }
    Ok(Some(entries))
}

/// inline table에서 최상위 key 하나의 값 원문을 찾습니다.
fn inline_table_value(
    value: &str,
    wanted: &str,
    line: usize,
) -> Result<Option<String>, ManifestError> {
    Ok(inline_table_entries(value, line)?.and_then(|entries| {
        entries
            .into_iter()
            .find(|(key, _)| key.len() == 1 && key[0] == wanted)
            .map(|(_, raw)| raw)
    }))
}

/// 값 원문이 TOML 값 하나인지 확인합니다. 문자열·배열·inline table은 짝이 맞는 끝 뒤에 다른
/// 내용이 없어야 하고, 그 밖의 값(정수·실수·불리언·날짜)은 그런 값에 쓰이는 문자만 가집니다.
/// `stellar-a = { path = "a" } stellar-b = 1`처럼 한 줄에 두 문장을 쓴 manifest를 거부합니다.
fn check_single_value(value: &str, line: usize) -> Result<(), ManifestError> {
    let value = value.trim();
    let single = match value.as_bytes().first() {
        None => false,
        Some(b'"' | b'\'' | b'[' | b'{') => {
            leading_value_end(value).is_some_and(|end| value[end..].trim().is_empty())
        }
        Some(_) => value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '_' | '.' | ':' | ' ')),
    };
    if single {
        Ok(())
    } else {
        Err(ManifestError::at(
            "manifest-syntax",
            line,
            "값이 없거나, 값 하나 뒤에 다른 내용이 있습니다",
        ))
    }
}

/// 값 원문 맨 앞의 문자열·배열·inline table이 끝나는 byte 위치. 짝이 맞지 않으면 `None`.
fn leading_value_end(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    match bytes.first()? {
        b'"' | b'\'' => string_end(value, 0),
        b'[' | b'{' => {
            // 괄호 종류까지 맞는지 확인하려고 닫는 괄호를 쌓습니다(깊이는 입력 길이 이하).
            let mut closers: Vec<u8> = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' | b'\'' => {
                        i = string_end(value, i)?;
                        continue;
                    }
                    b'[' => closers.push(b']'),
                    b'{' => closers.push(b'}'),
                    close @ (b']' | b'}') => {
                        if closers.pop() != Some(close) {
                            return None;
                        }
                        if closers.is_empty() {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => None,
    }
}

/// `text[start..]`가 문자열(basic·literal, 한 줄·여러 줄)로 시작할 때 닫는 따옴표 다음 byte
/// 위치. 닫히지 않았거나 문자열로 시작하지 않으면 `None`.
///
/// 따옴표와 `\`는 ASCII라 UTF-8 연속 byte와 겹치지 않으므로 byte 단위로 훑습니다.
fn string_end(text: &str, start: usize) -> Option<usize> {
    let bytes = &text.as_bytes()[start..];
    let quote = *bytes.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let multi = bytes.starts_with(&[quote, quote, quote]);
    let mut i = if multi { 3 } else { 1 };
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && quote == b'"' {
            i += 2;
            continue;
        }
        if b == quote {
            if !multi {
                return Some(start + i + 1);
            }
            if bytes[i..].starts_with(&[quote, quote, quote]) {
                // 여러 줄 문자열은 닫는 따옴표 셋 앞에 내용 따옴표가 두 개까지 붙을 수 있습니다.
                let mut end = i + 3;
                let mut extra = 0;
                while extra < 2 && bytes.get(end) == Some(&quote) {
                    end += 1;
                    extra += 1;
                }
                return Some(start + end);
            }
        }
        i += 1;
    }
    None
}

/// 문장 목록을 만듭니다. 각 문장은 시작 행 번호와 함께 반환합니다.
fn statements(text: &str) -> Result<Vec<(usize, Statement)>, ManifestError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut state = LexState::Normal;
    let mut depth: usize = 0;
    let mut line = 1usize;
    let mut start_line = 1usize;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        let triple = |q: char| i + 2 < chars.len() && chars[i + 1] == q && chars[i + 2] == q;
        match state {
            LexState::Comment => {
                if c == '\n' {
                    state = LexState::Normal;
                    continue; // 줄바꿈은 Normal 상태에서 처리합니다.
                }
            }
            LexState::Basic => {
                if c == '\n' {
                    return Err(ManifestError::at(
                        "manifest-syntax",
                        line,
                        "닫히지 않은 문자열",
                    ));
                }
                buf.push(c);
                if c == '\\' {
                    if let Some(&next) = chars.get(i + 1) {
                        if next != '\n' {
                            buf.push(next);
                            i += 1;
                        }
                    }
                } else if c == '"' {
                    state = LexState::Normal;
                }
            }
            LexState::Literal => {
                if c == '\n' {
                    return Err(ManifestError::at(
                        "manifest-syntax",
                        line,
                        "닫히지 않은 문자열",
                    ));
                }
                buf.push(c);
                if c == '\'' {
                    state = LexState::Normal;
                }
            }
            LexState::MultiBasic => {
                if c == '\\' {
                    buf.push(c);
                    if let Some(&next) = chars.get(i + 1) {
                        if next == '\n' {
                            line += 1;
                        }
                        buf.push(next);
                        i += 1;
                    }
                } else if c == '"' && triple('"') {
                    let run = closing_run(&chars, i, '"');
                    buf.extend(std::iter::repeat_n('"', run));
                    i += run - 1;
                    state = LexState::Normal;
                } else {
                    if c == '\n' {
                        line += 1;
                    }
                    buf.push(c);
                }
            }
            LexState::MultiLiteral => {
                if c == '\'' && triple('\'') {
                    let run = closing_run(&chars, i, '\'');
                    buf.extend(std::iter::repeat_n('\'', run));
                    i += run - 1;
                    state = LexState::Normal;
                } else {
                    if c == '\n' {
                        line += 1;
                    }
                    buf.push(c);
                }
            }
            LexState::Normal => match c {
                '#' => state = LexState::Comment,
                '\n' => {
                    if depth == 0 {
                        push_statement(&mut out, &mut buf, start_line)?;
                        start_line = line + 1;
                    } else {
                        buf.push(' ');
                    }
                    line += 1;
                }
                '"' => {
                    if triple('"') {
                        buf.push_str("\"\"\"");
                        i += 2;
                        state = LexState::MultiBasic;
                    } else {
                        buf.push(c);
                        state = LexState::Basic;
                    }
                }
                '\'' => {
                    if triple('\'') {
                        buf.push_str("'''");
                        i += 2;
                        state = LexState::MultiLiteral;
                    } else {
                        buf.push(c);
                        state = LexState::Literal;
                    }
                }
                '[' | '{' => {
                    depth += 1;
                    buf.push(c);
                }
                ']' | '}' => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        ManifestError::at("manifest-syntax", line, "짝이 없는 닫는 괄호")
                    })?;
                    buf.push(c);
                }
                _ => buf.push(c),
            },
        }
        i += 1;
    }
    match state {
        LexState::Normal | LexState::Comment => {}
        _ => {
            return Err(ManifestError::at(
                "manifest-syntax",
                line,
                "닫히지 않은 문자열",
            ))
        }
    }
    if depth != 0 {
        return Err(ManifestError::at(
            "manifest-syntax",
            line,
            "닫히지 않은 괄호",
        ));
    }
    push_statement(&mut out, &mut buf, start_line)?;
    Ok(out)
}

/// 여러 줄 문자열을 닫는 따옴표 묶음의 길이(3~5). TOML은 닫는 따옴표 셋 바로 앞에 내용 따옴표를
/// 두 개까지 허용하므로(`"""a"""""`는 `a""`), 이어진 따옴표를 다섯 개까지 한 번에 닫습니다.
fn closing_run(chars: &[char], start: usize, quote: char) -> usize {
    let mut run = 3;
    while run < 5 && chars.get(start + run) == Some(&quote) {
        run += 1;
    }
    run
}

fn push_statement(
    out: &mut Vec<(usize, Statement)>,
    buf: &mut String,
    line: usize,
) -> Result<(), ManifestError> {
    let text = std::mem::take(buf);
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    if let Some(rest) = text.strip_prefix('[') {
        let (array, inner) = match rest.strip_prefix('[') {
            Some(inner) => (true, inner.strip_suffix("]]")),
            None => (false, rest.strip_suffix(']')),
        };
        let inner = inner.ok_or_else(|| {
            ManifestError::at(
                "manifest-syntax",
                line,
                "표 머리글 뒤에 다른 내용이 있습니다",
            )
        })?;
        let path = parse_dotted_key(inner, line)?;
        out.push((line, Statement::Header { array, path }));
        return Ok(());
    }
    let (key, value) = split_key_value(text)
        .ok_or_else(|| ManifestError::at("manifest-syntax", line, "key = value 형식이 아닙니다"))?;
    let key = parse_dotted_key(&key, line)?;
    out.push((line, Statement::KeyValue { key, value }));
    Ok(())
}

/// 따옴표 밖의 첫 `=`에서 key와 value를 나눕니다. key의 따옴표가 닫히지 않으면 `None`.
fn split_key_value(text: &str) -> Option<(String, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = string_end(text, i)?,
            b'=' => return Some((text[..i].to_string(), text[i + 1..].to_string())),
            _ => i += 1,
        }
    }
    None
}

/// 따옴표와 괄호 밖의 ASCII 구분자 `sep`으로 나눕니다(inline table 항목·dotted key 분리용).
fn split_top_level(text: &str, sep: char) -> Vec<String> {
    debug_assert!(sep.is_ascii());
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth: usize = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                // 닫히지 않은 따옴표는 나머지 전체를 한 조각으로 둡니다(이후 해석에서 거부).
                i = string_end(text, i).unwrap_or(bytes.len());
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b if b == sep as u8 && depth == 0 => {
                parts.push(text[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(text[start..].to_string());
    parts
}

/// dotted key(`a."b.c".'d'`)를 segment 목록으로 해석합니다.
fn parse_dotted_key(text: &str, line: usize) -> Result<Vec<String>, ManifestError> {
    let mut segments = Vec::new();
    for raw in split_top_level(text, '.') {
        let raw = raw.trim();
        let segment = if raw.starts_with('"') || raw.starts_with('\'') {
            // 따옴표 key는 한 줄 문자열 하나여야 하며, escape는 값 문자열과 같은 규칙으로 풉니다.
            string_value(raw, line)?
                .ok_or_else(|| ManifestError::at("manifest-syntax", line, "잘못된 따옴표 key"))?
        } else {
            let bare = !raw.is_empty()
                && raw
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !bare {
                return Err(ManifestError::at(
                    "manifest-syntax",
                    line,
                    format!("잘못된 key: `{raw}`"),
                ));
            }
            raw.to_string()
        };
        segments.push(segment);
    }
    Ok(segments)
}

/// 한 줄 문자열 값(`"..."`, `'...'`)을 읽습니다. 문자열이 아닌 값(여러 줄 문자열 포함)은
/// `Ok(None)`, 문자열 본문이 잘못되었으면(escape 오류, 닫는 따옴표 앞의 따옴표) 오류입니다.
fn string_value(value: &str, line: usize) -> Result<Option<String>, ManifestError> {
    let value = value.trim();
    if value.starts_with("\"\"\"") || value.starts_with("'''") {
        return Ok(None);
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return unescape_basic(inner, line).map(Some);
    }
    match value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        Some(inner) if inner.contains('\'') => Err(ManifestError::at(
            "manifest-syntax",
            line,
            "literal string 안에 작은따옴표가 있습니다",
        )),
        Some(inner) => Ok(Some(inner.to_string())),
        None => Ok(None),
    }
}

/// basic string 본문의 escape를 TOML 규칙대로 풉니다(`\b \t \n \f \r \" \\ \uXXXX \UXXXXXXXX`).
///
/// Cargo는 `"stellar-ffi"`를 `stellar-ffi`로 읽으므로 같은 이름으로 풀어야 내부 의존 판정에서
/// 빠지지 않습니다. 그 밖의 escape, 끝나지 않은 escape, escape하지 않은 `"`, 유니코드 scalar가
/// 아닌 코드는 거부합니다(fail closed).
fn unescape_basic(inner: &str, line: usize) -> Result<String, ManifestError> {
    let invalid = || {
        ManifestError::at(
            "manifest-syntax",
            line,
            "basic string의 escape나 따옴표가 잘못되었습니다",
        )
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Err(invalid()),
            '\\' => {
                let decoded = match chars.next().ok_or_else(invalid)? {
                    'b' => '\u{8}',
                    't' => '\t',
                    'n' => '\n',
                    'f' => '\u{c}',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    'u' => hex_scalar(&mut chars, 4).ok_or_else(invalid)?,
                    'U' => hex_scalar(&mut chars, 8).ok_or_else(invalid)?,
                    _ => return Err(invalid()),
                };
                out.push(decoded);
            }
            _ => out.push(c),
        }
    }
    Ok(out)
}

/// 16진 숫자 `digits`개를 읽어 유니코드 scalar 값으로 바꿉니다. 숫자가 모자라거나 16진이 아니거나
/// scalar가 아니면(surrogate, `U+10FFFF` 초과) `None`.
fn hex_scalar(chars: &mut std::str::Chars<'_>, digits: usize) -> Option<char> {
    let mut code: u32 = 0;
    for _ in 0..digits {
        // 8자리여도 `0xFFFF_FFFF` 이하라 u32에서 넘치지 않습니다.
        code = code * 16 + chars.next()?.to_digit(16)?;
    }
    char::from_u32(code)
}

/// 크기 상한을 지켜 manifest를 읽습니다.
fn read_bounded(path: &Path) -> Result<String, ManifestError> {
    let display = path.display();
    let file = File::open(path).map_err(|e| ManifestError::new("io", format!("{display}: {e}")))?;
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| ManifestError::new("io", format!("{display}: {e}")))?;
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::new(
            "manifest-too-large",
            format!("{display}: {MAX_MANIFEST_BYTES} bytes를 넘습니다"),
        ));
    }
    Ok(text)
}

/// 검사 대상 manifest 경로: `crates/*/Cargo.toml`(필수 디렉터리)과
/// `tools/stellar-*/Cargo.toml`. 경로 순으로 정렬합니다.
pub fn member_manifests(repo_root: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    let mut manifests = Vec::new();
    let crates = repo_root.join("crates");
    if !crates.is_dir() {
        return Err(ManifestError::new(
            "missing-crates-dir",
            "crates 디렉터리 없음",
        ));
    }
    collect_manifests(&crates, |_| true, &mut manifests)?;
    let tools = repo_root.join("tools");
    if tools.is_dir() {
        collect_manifests(
            &tools,
            |name| name.starts_with(INTERNAL_PREFIX),
            &mut manifests,
        )?;
    }
    manifests.sort();
    Ok(manifests)
}

fn collect_manifests(
    dir: &Path,
    accept: impl Fn(&str) -> bool,
    out: &mut Vec<PathBuf>,
) -> Result<(), ManifestError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ManifestError::new("io", format!("{}: {e}", dir.display())))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| ManifestError::new("io", format!("{}: {e}", dir.display())))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let manifest = entry.path().join("Cargo.toml");
        if accept(name) && manifest.is_file() {
            out.push(manifest);
        }
    }
    Ok(())
}

/// 저장소의 검사 대상 manifest 전체를 읽어 내부 의존 graph를 만듭니다.
///
/// 루트 `Cargo.toml`의 `[workspace.dependencies]` 이름 변경을 `workspace = true` 의존에 반영합니다.
pub fn scan_workspace(repo_root: &Path) -> Result<WorkspaceGraph, ManifestError> {
    let root_manifest = repo_root.join("Cargo.toml");
    let renames = if root_manifest.is_file() {
        parse_manifest(&read_bounded(&root_manifest)?)
            .map_err(|e| with_path(e, &root_manifest))?
            .workspace_renames
    } else {
        BTreeMap::new()
    };
    let mut sources = Vec::new();
    for path in member_manifests(repo_root)? {
        let text = read_bounded(&path)?;
        let parsed = parse_manifest(&text).map_err(|e| with_path(e, &path))?;
        sources.push((path, parsed));
    }
    build_graph(&sources, &renames)
}

/// 해석된 manifest 목록으로 graph를 만듭니다(fixture 테스트용 진입점).
pub fn build_graph(
    sources: &[(PathBuf, ParsedManifest)],
    workspace_renames: &BTreeMap<String, String>,
) -> Result<WorkspaceGraph, ManifestError> {
    let mut packages = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for (path, parsed) in sources {
        let Some(package) = &parsed.package else {
            return Err(ManifestError::new(
                "missing-package-name",
                format!("{}: [package].name이 없습니다", path.display()),
            ));
        };
        if !packages.insert(package.clone()) {
            return Err(ManifestError::new(
                "duplicate-package",
                format!("{}: package 이름 중복: {package}", path.display()),
            ));
        }
        for dep in &parsed.deps {
            let to = dep.resolved_name(workspace_renames);
            if to.starts_with(INTERNAL_PREFIX) {
                edges.insert(ManifestEdge {
                    from: package.clone(),
                    to,
                    table: dep.table.clone(),
                });
            }
        }
    }
    Ok(WorkspaceGraph {
        packages: packages.into_iter().collect(),
        edges: edges.into_iter().collect(),
    })
}

fn with_path(error: ManifestError, path: &Path) -> ManifestError {
    ManifestError {
        code: error.code,
        message: format!("{}: {}", path.display(), error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn internal(text: &str) -> Vec<(String, String)> {
        let parsed = parse_manifest(text).expect("parse");
        parsed
            .deps
            .iter()
            .map(|d| (d.resolved_name(&BTreeMap::new()), d.table.label()))
            .filter(|(name, _)| name.starts_with(INTERNAL_PREFIX))
            .collect()
    }

    fn pair(name: &str, table: &str) -> (String, String) {
        (name.to_string(), table.to_string())
    }

    #[test]
    fn reads_package_name_and_all_dependency_tables() {
        let text = r#"
[package]
name = "stellar-sample"
version = "0.0.0"

[dependencies]
stellar-core-types = { path = "../core-types" }
yaml-rust2 = "=0.11.0"

[build-dependencies]
stellar-scene = { path = "../scene" }

[dev-dependencies]
stellar-document = { path = "../document", features = ["test-support"] }

[dev_dependencies]
stellar-text = { path = "../text" }

[target.'cfg(target_os = "macos")'.dependencies]
stellar-renderer-metal = { path = "../renderer-metal" }

[target.x86_64-pc-windows-msvc.dev-dependencies]
stellar-renderer-d3d12 = { path = "../renderer-d3d12" }
"#;
        let parsed = parse_manifest(text).unwrap();
        assert_eq!(parsed.package.as_deref(), Some("stellar-sample"));
        let mut got = internal(text);
        got.sort();
        let mut want = vec![
            pair("stellar-core-types", "[dependencies]"),
            pair("stellar-scene", "[build-dependencies]"),
            pair("stellar-document", "[dev-dependencies]"),
            pair("stellar-text", "[dev-dependencies]"),
            pair(
                "stellar-renderer-metal",
                "[target.'cfg(target_os = \"macos\")'.dependencies]",
            ),
            pair(
                "stellar-renderer-d3d12",
                "[target.'x86_64-pc-windows-msvc'.dev-dependencies]",
            ),
        ];
        want.sort();
        assert_eq!(got, want);
        // 외부 의존도 선언 목록에는 남습니다(외부 의존 검사용).
        assert!(parsed.deps.iter().any(|d| d.key == "yaml-rust2"));
    }

    #[test]
    fn reads_sub_tables_dotted_keys_and_renames() {
        let text = r#"
[package]
name = "stellar-sample"

[dependencies.stellar-history]
path = "../history"

[dependencies]
alias = { package = "stellar-commands", path = "../commands" }
stellar-layout.path = "../layout"
"stellar-text" = { path = "../text" }
renamed.package = "stellar-scene"
renamed.path = "../scene"

[target.'cfg(unix)'.build-dependencies.helper]
package = "stellar-diagnostics"
path = "../diagnostics"
"#;
        let mut got = internal(text);
        got.sort();
        let mut want = vec![
            pair("stellar-history", "[dependencies]"),
            pair("stellar-commands", "[dependencies]"),
            pair("stellar-layout", "[dependencies]"),
            pair("stellar-text", "[dependencies]"),
            pair("stellar-scene", "[dependencies]"),
            pair(
                "stellar-diagnostics",
                "[target.'cfg(unix)'.build-dependencies]",
            ),
        ];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn ignores_strings_comments_arrays_and_other_tables() {
        let text = r#"
[package]
name = "stellar-sample" # stellar-ffi = { path = "../ffi" }
description = """
stellar-ffi = { path = "../ffi" }
"""
keywords = [
  "stellar-ffi = 1",
]

[features]
stellar-ffi = []

[[bin]]
name = "stellar-ffi"

[dependencies]
# stellar-runtime = { path = "../runtime" }
stellar-core-types = { path = "../core-types", features = [
  "a",
] }
"#;
        assert_eq!(
            internal(text),
            vec![pair("stellar-core-types", "[dependencies]")]
        );
        assert_eq!(
            parse_manifest(text).unwrap().package.as_deref(),
            Some("stellar-sample")
        );
    }

    #[test]
    fn inline_table_dependency_tables_are_read() {
        // 의존 표 전체나 그 상위 표를 inline table로 쓴 유효한 manifest(Cargo가 그대로 받음).
        // 이전 해석기는 아래 의존을 모두 조용히 건너뛰었습니다.
        let cases: [(&str, Vec<(String, String)>); 7] = [
            (
                "[package]\nname = \"stellar-document\"\n[target.'cfg(unix)']\ndependencies = { stellar-ffi = { path = \"../ffi\" } }\n",
                vec![pair("stellar-ffi", "[target.'cfg(unix)'.dependencies]")],
            ),
            (
                "dev-dependencies = { stellar-ffi = { path = \"../ffi\" } }\n[package]\nname = \"stellar-document\"\n",
                vec![pair("stellar-ffi", "[dev-dependencies]")],
            ),
            (
                "[package]\nname = \"stellar-document\"\n[target]\n'cfg(unix)' = { dependencies = { stellar-ffi = { path = \"../ffi\" } } }\n",
                vec![pair("stellar-ffi", "[target.'cfg(unix)'.dependencies]")],
            ),
            (
                "target = { 'cfg(unix)' = { build-dependencies = { stellar-ffi = { path = \"../ffi\" } } } }\n[package]\nname = \"stellar-document\"\n",
                vec![pair("stellar-ffi", "[target.'cfg(unix)'.build-dependencies]")],
            ),
            (
                "dependencies = { stellar-ffi.path = \"../ffi\", alias = { package = \"stellar-runtime\", path = \"../runtime\" } }\n[package]\nname = \"stellar-document\"\n",
                vec![
                    pair("stellar-ffi", "[dependencies]"),
                    pair("stellar-runtime", "[dependencies]"),
                ],
            ),
            (
                "[target.'cfg(unix)']\ndev_dependencies = { stellar-ffi = \"0.0.0\" }\n",
                vec![pair("stellar-ffi", "[target.'cfg(unix)'.dev-dependencies]")],
            ),
            (
                "[target.'cfg(unix)'.dependencies]\nalias = { package = \"stellar-ffi\", path = \"../ffi\" }\n",
                vec![pair("stellar-ffi", "[target.'cfg(unix)'.dependencies]")],
            ),
        ];
        for (text, want) in cases {
            let mut got = internal(text);
            got.sort();
            let mut want = want;
            want.sort();
            assert_eq!(got, want, "{text:?}");
        }
        // `[package]` 아래의 `dev-dependencies = { .. }`는 `package.dev-dependencies`이므로 Cargo도
        // 의존으로 보지 않습니다(사용하지 않는 key 경고).
        assert!(internal(
            "[package]\nname = \"stellar-a\"\ndev-dependencies = { stellar-ffi = { path = \"f\" } }\n"
        )
        .is_empty());
        // inline `package`·`workspace` 표도 같은 위치로 읽습니다.
        let parsed = parse_manifest("package = { name = \"stellar-a\" }\n").unwrap();
        assert_eq!(parsed.package.as_deref(), Some("stellar-a"));
        let root = parse_manifest(
            "workspace = { dependencies = { core = { package = \"stellar-core-types\", path = \"c\" } } }\n",
        )
        .unwrap();
        assert_eq!(
            root.workspace_renames.get("core").map(String::as_str),
            Some("stellar-core-types")
        );
        let root = parse_manifest(
            "[workspace]\ndependencies = { core = { package = \"stellar-core-types\" } }\n",
        )
        .unwrap();
        assert_eq!(root.workspace_renames.len(), 1);
    }

    #[test]
    fn dependency_table_positions_require_tables() {
        for text in [
            "dependencies = 1\n",
            "dev-dependencies = \"stellar-ffi\"\n",
            "[target]\n'cfg(unix)' = \"x\"\n",
            "[target.'cfg(unix)']\ndependencies = [\"stellar-ffi\"]\n",
            "target = 1\n",
            "workspace = { dependencies = [] }\n",
        ] {
            assert_eq!(
                parse_manifest(text).unwrap_err().code,
                "manifest-syntax",
                "{text:?}"
            );
        }
    }

    #[test]
    fn statements_with_trailing_content_fail_closed() {
        // 값 하나 뒤에 다른 문장이 붙은 잘못된 TOML(Cargo도 거부)을 앞부분만 읽고 넘기지 않습니다.
        for text in [
            "[dependencies]\nstellar-a = { path = \"a\" } stellar-b = 1\n",
            "[dependencies]\nstellar-a = \"1\" stellar-b = \"1\"\n",
            "[dependencies]\nstellar-a = 1 stellar-b = 1\n",
            "[dependencies]\nstellar-a = { path = \"a\" x = 1 }\n",
            "[dependencies]\nstellar-a = { path = \"a\", f = [1] x }\n",
            "[dependencies]\nstellar-a = { path = \"a\" ]\n",
            "[dependencies]\nstellar-a =\n",
            "[dependencies]\nstellar-a = { path = }\n",
            "[package]\ndescription = \"\"\"x\"\"\" stellar-b = 1\n",
        ] {
            assert_eq!(
                parse_manifest(text).unwrap_err().code,
                "manifest-syntax",
                "{text:?}"
            );
        }
        // 한 값으로 끝나는 정상 값은 그대로 받습니다.
        let ok = r#"
[package]
name = "stellar-a"
description = """a "quoted" ""word"""""
readme = '''x''''
keywords = ["""x, "y" """, 'z', { a = [1, { b = "}" }] }]
date = 1979-05-27 07:32:00Z
num = +1_000
ratio = -6.5e-3
flag = true
[dependencies]
stellar-b = { path = "../b", features = ["x"], default-features = false }
"#;
        assert_eq!(internal(ok), vec![pair("stellar-b", "[dependencies]")]);
    }

    #[test]
    fn escaped_names_resolve_like_cargo() {
        // Cargo는 basic string escape를 풀어 읽습니다. 이전 해석기는 `\u0073`를 `u0073`로 읽어
        // 내부 의존을 `stellar-` prefix 판정에서 놓쳤습니다.
        let text = r#"
[package]
name = "\u0073tellar-sample"

[dependencies]
"\u0073tellar-ffi" = { path = "../ffi" }
alias = { package = "stellar\u002Druntime", path = "../runtime" }
other.package = "\U00000073tellar-project"
"tab\tkey" = "1"

[dependencies."stellar\u002ddocument"]
path = "../document"
"#;
        let parsed = parse_manifest(text).unwrap();
        assert_eq!(parsed.package.as_deref(), Some("stellar-sample"));
        let mut got = internal(text);
        got.sort();
        let mut want = vec![
            pair("stellar-ffi", "[dependencies]"),
            pair("stellar-runtime", "[dependencies]"),
            pair("stellar-project", "[dependencies]"),
            pair("stellar-document", "[dependencies]"),
        ];
        want.sort();
        assert_eq!(got, want);
        assert!(parsed.deps.iter().any(|d| d.key == "tab\tkey"));
        let root = parse_manifest(
            "[workspace.dependencies]\ncore = { package = \"stellar\\u002dcore-types\" }\n",
        )
        .unwrap();
        assert_eq!(
            root.workspace_renames.get("core").map(String::as_str),
            Some("stellar-core-types")
        );
    }

    #[test]
    fn invalid_string_escapes_and_quotes_fail_closed() {
        for text in [
            "[dependencies]\n\"\\x73tellar-ffi\" = \"1\"\n",
            "[dependencies]\n\"\\e\" = \"1\"\n",
            "[dependencies]\n\"\\u00\" = \"1\"\n",
            "[dependencies]\n\"\\u00zz\" = \"1\"\n",
            "[dependencies]\n\"\\uD800\" = \"1\"\n",
            "[dependencies]\n\"\\U00110000\" = \"1\"\n",
            "[dependencies]\n\"a\\q\" = \"1\"\n",
            "[dependencies]\n\"a\" \"b\" = \"1\"\n",
            "[dependencies]\n'a' 'b' = \"1\"\n",
            "[dependencies]\na = { package = \"stellar\\-ffi\" }\n",
            "[dependencies]\na.package = \"\\u0073tellar-ffi\\\"\n",
            "[package]\nname = \"\\z\"\n",
        ] {
            assert_eq!(
                parse_manifest(text).unwrap_err().code,
                "manifest-syntax",
                "{text:?}"
            );
        }
    }

    #[test]
    fn workspace_inheritance_resolves_root_renames() {
        let root = r#"
[workspace]
members = ["crates/a"]

[workspace.dependencies]
core = { package = "stellar-core-types", path = "crates/core-types" }
"#;
        let member = r#"
[package]
name = "stellar-a"

[dependencies]
core = { workspace = true }
other.workspace = true
"#;
        let renames = parse_manifest(root).unwrap().workspace_renames;
        assert_eq!(
            renames.get("core").map(String::as_str),
            Some("stellar-core-types")
        );
        let graph = build_graph(
            &[(PathBuf::from("crates/a"), parse_manifest(member).unwrap())],
            &renames,
        )
        .unwrap();
        let to: Vec<&str> = graph.edges.iter().map(|e| e.to.as_str()).collect();
        assert_eq!(to, ["stellar-core-types"]);
    }

    #[test]
    fn malformed_manifests_fail_closed() {
        for (text, code) in [
            ("[package]\nname = \"x\n", "manifest-syntax"),
            ("[dependencies\nstellar-a = 1\n", "manifest-syntax"),
            (
                "[dependencies]\nstellar-a = { path = \"x\" \n",
                "manifest-syntax",
            ),
            ("[dependencies]\nstellar-a\n", "manifest-syntax"),
            ("[dependencies] stellar-a = 1\n", "manifest-syntax"),
            ("[dependencies]\nbad key = 1\n", "manifest-syntax"),
            ("[dependencies]\na = ]\n", "manifest-syntax"),
            ("[package]\nname = 3\n", "package-name"),
            (
                "[dependencies]\na = { package = 3, path = \"x\" }\n",
                "dependency-package",
            ),
            ("description = \"\"\"\nopen\n", "manifest-syntax"),
        ] {
            assert_eq!(parse_manifest(text).unwrap_err().code, code, "{text:?}");
        }
        let huge = "#".repeat(MAX_MANIFEST_BYTES + 1);
        assert_eq!(
            parse_manifest(&huge).unwrap_err().code,
            "manifest-too-large"
        );
    }

    #[test]
    fn graph_rejects_missing_or_duplicate_package_names() {
        let named = parse_manifest("[package]\nname = \"stellar-a\"\n").unwrap();
        let unnamed = parse_manifest("[workspace]\n").unwrap();
        let err = build_graph(
            &[(PathBuf::from("crates/x/Cargo.toml"), unnamed)],
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(err.code, "missing-package-name");
        let err = build_graph(
            &[
                (PathBuf::from("crates/a/Cargo.toml"), named.clone()),
                (PathBuf::from("crates/b/Cargo.toml"), named),
            ],
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(err.code, "duplicate-package");
    }

    #[test]
    fn real_workspace_scan_covers_crates_and_tools() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let graph = scan_workspace(root).expect("scan");
        for package in [
            "stellar-core-types",
            "stellar-project",
            "stellar-cli",
            "stellar-mcp",
        ] {
            assert!(graph.packages.iter().any(|p| p == package), "{package}");
        }
        // xtask와 benchmark fixture crate는 검사 대상이 아닙니다(ADR-0019).
        assert!(!graph.packages.iter().any(|p| p == "xtask"));
        assert!(!graph.packages.iter().any(|p| p == "stellar-bench-fixtures"));
        // dev 의존 표도 읽습니다(project의 test-support document 의존).
        assert!(graph.edges.iter().any(|e| e.from == "stellar-project"
            && e.to == "stellar-document"
            && e.table.kind == DepKind::Dev));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == "stellar-cli" && e.to == "stellar-ops"));
    }
}
