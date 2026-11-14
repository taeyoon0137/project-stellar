//! Cargo manifest 내부 의존 추출 (XXIX.1.1).
//!
//! crate graph 검사에 필요한 부분만 읽는 제한 TOML 해석기입니다. `[package].name`과 다음 표의
//! 의존 이름을 추출합니다.
//!
//! - `[dependencies]`, `[build-dependencies]`, `[dev-dependencies]`(밑줄 표기 포함)
//! - `[target.<spec>.dependencies]` 등 target 한정 표
//! - `[dependencies.<name>]` 형태의 하위 표와 `<name>.path = ...` 같은 dotted key
//! - `package = "..."` 이름 변경과 `[workspace.dependencies]`를 상속하는 `workspace = true`
//!
//! 문자열·배열·inline table·주석을 구분해 문장 단위로 나누므로, 여러 줄 배열이나 문자열 안의
//! `stellar-*` 텍스트를 의존으로 오인하지 않습니다. 해석할 수 없는 구문은 오류로 거부합니다
//! (fail closed). 입력 크기는 [`MAX_MANIFEST_BYTES`]로 제한하고 모든 처리는 반복문입니다.

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
                apply_key_value(&mut parsed, &mut deps, &full, &value, line)?;
            }
        }
    }
    parsed.deps = deps.into_values().collect();
    Ok(parsed)
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
        let name = string_value(value).ok_or_else(|| {
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
        Some(raw) => string_value(&raw).map(Some).ok_or_else(|| {
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

/// inline table(`{ a = 1, b = "x" }`)에서 최상위 key 하나의 값 원문을 찾습니다.
fn inline_table_value(
    value: &str,
    wanted: &str,
    line: usize,
) -> Result<Option<String>, ManifestError> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Ok(None);
    };
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
        let key = parse_dotted_key(&key, line)?;
        if key.len() == 1 && key[0] == wanted {
            return Ok(Some(raw.trim().to_string()));
        }
    }
    Ok(None)
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
                    buf.push_str("\"\"\"");
                    i += 2;
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
                    buf.push_str("'''");
                    i += 2;
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

/// 따옴표 밖의 첫 `=`에서 key와 value를 나눕니다.
fn split_key_value(text: &str) -> Option<(String, String)> {
    let mut quote: Option<char> = None;
    for (index, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '=') => {
                return Some((text[..index].to_string(), text[index + 1..].to_string()));
            }
            _ => {}
        }
    }
    None
}

/// 따옴표와 괄호 밖의 `sep`으로 나눕니다(inline table 항목 분리용).
fn split_top_level(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth: usize = 0;
    for c in text.chars() {
        if let Some(q) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                current.push(c);
            }
            '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            _ if c == sep && depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// dotted key(`a."b.c".'d'`)를 segment 목록으로 해석합니다.
fn parse_dotted_key(text: &str, line: usize) -> Result<Vec<String>, ManifestError> {
    let mut segments = Vec::new();
    for raw in split_top_level(text, '.') {
        let raw = raw.trim();
        let segment = if let Some(inner) = raw.strip_prefix('"') {
            inner
                .strip_suffix('"')
                .map(unescape_basic)
                .ok_or_else(|| ManifestError::at("manifest-syntax", line, "잘못된 따옴표 key"))?
        } else if let Some(inner) = raw.strip_prefix('\'') {
            inner
                .strip_suffix('\'')
                .map(str::to_string)
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

/// 한 줄 문자열 값(`"..."`, `'...'`)을 읽습니다. 그 밖의 값은 `None`.
fn string_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("\"\"\"") || value.starts_with("'''") {
        return None;
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Some(unescape_basic(inner));
    }
    value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .map(str::to_string)
}

/// basic string의 단순 escape(`\"`, `\\`)만 풉니다. package 이름에는 그 밖의 escape가 없습니다.
fn unescape_basic(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
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
