//! 금지 필드 scanner (XVI.8.2, ADR-0109 결정 4).
//!
//! 진단 번들(또는 임의 진단 text)에서 아키텍처 §39 수집 금지 정보를 찾습니다. 세 층으로 봅니다.
//!
//! 1. **내용 규칙**: 경로, Git 원격 URL, branch 표기, commit hash, 터미널 입출력, MCP·에이전트 payload, 프로젝트
//!    텍스트, 자격 증명·계정 식별 표기를 문맥 없이 찾습니다. 규칙마다 금지 범주가 정해져 있습니다.
//! 2. **문맥 값**: 앱이 아는 열린 프로젝트의 이름·경로·텍스트, Git 원격·branch·commit을 [`ScanContext`]로 받아 그
//!    값이 나타나는지 봅니다(ASCII 대소문자 무시). 4 bytes 미만이거나 schema 어휘와 같은 값은 공개 어휘와 구별할
//!    수 없으므로 비교하지 않습니다. 그런 값이 어긋난 자리에 들어오면 구조 검사가 막습니다.
//! 3. **구조 검사**([`scan_bundle`]만): 번들 형식과 schema에 정확히 맞지 않는 줄은 [`ForbiddenCategory::Unrecognized`]입니다.
//!
//! 결과 [`Finding`]은 줄 번호, 범주, 규칙 이름만 담고, 찾은 문자열 자체는 담지 않습니다.

use std::collections::BTreeSet;
use std::fmt;

use crate::bundle::{decode_bundle, BundleEnvironment, BUNDLE_HEADER, BUNDLE_SCHEMAS, END_SCHEMA};
use crate::store::{
    decode_line, parse_value, schema_vocabulary, DiagnosticRecord, FieldType, FieldValue,
    StoreError, MAX_DIAGNOSTIC_BYTES, META_SCHEMA, STORE_HEADER,
};

/// 금지 범주(아키텍처 §39 수집 금지 항목).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForbiddenCategory {
    /// 프로젝트 내용·텍스트.
    ProjectText,
    /// 프로젝트·문서·레이어·토큰 이름.
    ProjectName,
    /// 파일 경로.
    ProjectPath,
    /// Git 원격 URL.
    GitUrl,
    /// Git branch.
    GitBranch,
    /// Git commit.
    GitCommit,
    /// 터미널 입출력.
    TerminalPayload,
    /// MCP·에이전트 입출력.
    McpPayload,
    /// 자격 증명·계정 식별 정보.
    Credential,
    /// 번들 schema에 없는 내용.
    Unrecognized,
}

impl ForbiddenCategory {
    /// 모든 범주.
    pub const ALL: [ForbiddenCategory; 10] = [
        ForbiddenCategory::ProjectText,
        ForbiddenCategory::ProjectName,
        ForbiddenCategory::ProjectPath,
        ForbiddenCategory::GitUrl,
        ForbiddenCategory::GitBranch,
        ForbiddenCategory::GitCommit,
        ForbiddenCategory::TerminalPayload,
        ForbiddenCategory::McpPayload,
        ForbiddenCategory::Credential,
        ForbiddenCategory::Unrecognized,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ForbiddenCategory::ProjectText => "project_text",
            ForbiddenCategory::ProjectName => "project_name",
            ForbiddenCategory::ProjectPath => "project_path",
            ForbiddenCategory::GitUrl => "git_url",
            ForbiddenCategory::GitBranch => "git_branch",
            ForbiddenCategory::GitCommit => "git_commit",
            ForbiddenCategory::TerminalPayload => "terminal_payload",
            ForbiddenCategory::McpPayload => "mcp_payload",
            ForbiddenCategory::Credential => "credential",
            ForbiddenCategory::Unrecognized => "unrecognized",
        }
    }
}

/// 탐지 하나. 찾은 문자열은 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Finding {
    /// 1부터 센 줄 번호(0은 파일 전체).
    pub line: usize,
    /// 범주.
    pub category: ForbiddenCategory,
    /// 규칙 이름.
    pub rule: &'static str,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}번째 줄: {} ({})",
            self.line,
            self.category.name(),
            self.rule
        )
    }
}

/// 검사 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// 탐지(줄·범주·규칙 순, 중복 없음).
    pub findings: Vec<Finding>,
}

impl ScanReport {
    /// 탐지가 없는지.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// 탐지된 범주.
    pub fn categories(&self) -> BTreeSet<ForbiddenCategory> {
        self.findings.iter().map(|f| f.category).collect()
    }
}

/// 문맥 값 최소 길이(bytes).
pub const MIN_CONTEXT_VALUE_BYTES: usize = 4;

/// 앱이 아는 금지 값. 값은 비교에만 쓰고 결과에 넣지 않습니다.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ScanContext {
    values: Vec<(ForbiddenCategory, String)>,
}

impl fmt::Debug for ScanContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 값은 출력하지 않습니다.
        f.debug_struct("ScanContext")
            .field("values", &self.values.len())
            .finish()
    }
}

impl ScanContext {
    /// 빈 문맥.
    pub fn new() -> ScanContext {
        ScanContext::default()
    }

    /// 금지 값을 더합니다. 비교할 수 없는 값(짧거나 schema 어휘)은 더하지 않고 `false`를 돌려줍니다.
    pub fn add(&mut self, category: ForbiddenCategory, value: &str) -> bool {
        let value = value.trim();
        if value.len() < MIN_CONTEXT_VALUE_BYTES || is_public_vocabulary(value) {
            return false;
        }
        self.values.push((category, value.to_ascii_lowercase()));
        true
    }

    /// builder 형식의 [`Self::add`].
    pub fn with(mut self, category: ForbiddenCategory, value: &str) -> ScanContext {
        self.add(category, value);
        self
    }

    /// 비교하는 값 수.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 비교하는 값이 없는지.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// 번들 schema 어휘(정렬, 한 번만 만듭니다).
fn bundle_vocabulary() -> &'static [&'static str] {
    static VOCABULARY: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    VOCABULARY.get_or_init(|| schema_vocabulary(&BUNDLE_SCHEMAS))
}

/// schema 어휘, 형식 이름, 정수·version처럼 번들에 정상으로 나오는 값인지.
fn is_public_vocabulary(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower == BUNDLE_HEADER
        || lower == STORE_HEADER
        || bundle_vocabulary().binary_search(&lower.as_str()).is_ok()
        || parse_value(FieldType::OptU64, &lower).is_some()
        || parse_value(FieldType::Version, &lower).is_some()
}

/// 줄의 `[start, end)` 구간이 공개 조각 하나 안에 들어 있는지.
///
/// 조각은 형식 이름 줄이면 줄 전체이고, 그 밖에는 공백과 `=`로 나눈 token입니다. 공개 조각은 schema 어휘,
/// 형식 이름, 정수·`none`·version입니다. 문맥 값이 `uptime_ms` 안의 `time`, `renderer_error` 안의 `error`,
/// 형식 이름 안의 `stellar`처럼 공개 조각의 일부로만 나오면 프로젝트 값이 아니라 schema 어휘이므로 탐지하지
/// 않습니다. 공개 조각 밖으로 걸치거나 공개가 아닌 조각에 나오면 탐지합니다.
fn within_public_piece(lower: &str, start: usize, end: usize) -> bool {
    if lower == BUNDLE_HEADER || lower == STORE_HEADER {
        return true;
    }
    let mut piece_start = 0;
    for piece in lower.split([' ', '=']) {
        let piece_end = piece_start + piece.len();
        if start >= piece_start && end <= piece_end {
            return is_public_vocabulary(piece);
        }
        piece_start = piece_end + 1;
    }
    false
}

/// 문맥 값이 공개 조각 밖에 나오는지.
fn context_value_appears(lower: &str, value: &str) -> bool {
    lower
        .match_indices(value)
        .any(|(start, found)| !within_public_piece(lower, start, start + found.len()))
}

const PATH_PREFIXES: [&str; 12] = [
    "/users/",
    "/home/",
    "/volumes/",
    "/private/",
    "/var/",
    "/tmp/",
    "/opt/",
    "/library/",
    "/applications/",
    "~/",
    "file:",
    "\\\\",
];

const FILE_EXTENSIONS: [&str; 16] = [
    "stellar", "stl", "png", "jpg", "jpeg", "svg", "pdf", "json", "yaml", "yml", "md", "txt",
    "sketch", "fig", "ttf", "otf",
];

const BRANCH_MARKERS: [&str; 9] = [
    "refs/heads/",
    "refs/remotes/",
    "refs/tags/",
    "origin/",
    "upstream/",
    "feature/",
    "bugfix/",
    "hotfix/",
    "release/",
];

const TERMINAL_MARKERS: [&str; 22] = [
    "$ ", "% ", "# ", "❯", "#!", "/bin/", "bash", "zsh", "sudo ", "&&", "||", "| ", "2>&1", "$(",
    "`", "terminal", "pty", "tty", "stdin", "stdout", "stderr", "shell",
];

const MCP_MARKERS: [&str; 12] = [
    "jsonrpc",
    "\"method\"",
    "\"params\"",
    "\"result\"",
    "\"role\"",
    "tools/call",
    "tools/list",
    "resources/read",
    "prompts/get",
    "mcp",
    "agent",
    "transcript",
];

const CREDENTIAL_MARKERS: [&str; 14] = [
    "bearer ",
    "sk-",
    "ghp_",
    "gho_",
    "github_pat_",
    "xoxb-",
    "akia",
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "token=",
    "-----begin",
];

fn has_any(lower: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| lower.contains(marker))
}

/// 영숫자 덩어리 중 commit hash처럼 보이는 것이 있는지.
fn has_commit_like_run(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphanumeric()).any(|run| {
        let all_hex = !run.is_empty() && run.bytes().all(|b| b.is_ascii_hexdigit());
        let digits = run.bytes().filter(u8::is_ascii_digit).count();
        let letters = run.len() - digits;
        all_hex && ((7..=64).contains(&run.len()) && digits > 0 && letters > 0 || digits >= 21)
    })
}

/// `user@host:` 같은 scp 형식 원격인지.
fn has_scp_remote(line: &str) -> bool {
    line.match_indices('@').any(|(at, _)| {
        let host: String = line[at + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            .collect();
        !host.is_empty() && line[at + 1 + host.len()..].starts_with(':')
    })
}

/// `a@b` 형식(계정 식별)인지.
fn has_account_like(line: &str) -> bool {
    let bytes = line.as_bytes();
    line.match_indices('@').any(|(at, _)| {
        at > 0
            && bytes[at - 1].is_ascii_alphanumeric()
            && bytes.get(at + 1).is_some_and(u8::is_ascii_alphanumeric)
    })
}

fn has_file_extension(line: &str) -> bool {
    line.split(|c: char| c.is_whitespace() || c == '=')
        .any(|token| {
            token.rsplit_once('.').is_some_and(|(stem, ext)| {
                !stem.is_empty() && FILE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
            })
        })
}

fn has_windows_drive(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes
        .windows(3)
        .any(|w| w[0].is_ascii_alphabetic() && w[1] == b':' && (w[2] == b'\\' || w[2] == b'/'))
}

/// 내용 규칙 하나: (규칙 이름, 범주, 검사).
type Rule = (&'static str, ForbiddenCategory, fn(&str, &str) -> bool);

const RULES: [Rule; 19] = [
    (
        "control_char",
        ForbiddenCategory::TerminalPayload,
        |l, _| l.chars().any(char::is_control),
    ),
    (
        "terminal_marker",
        ForbiddenCategory::TerminalPayload,
        |_, lower| has_any(lower, &TERMINAL_MARKERS),
    ),
    ("mcp_marker", ForbiddenCategory::McpPayload, |_, lower| {
        has_any(lower, &MCP_MARKERS)
    }),
    ("json_like", ForbiddenCategory::McpPayload, |l, _| {
        l.contains('{') || l.contains('}') || l.contains('[') || l.contains(']')
    }),
    (
        "credential_marker",
        ForbiddenCategory::Credential,
        |_, lower| has_any(lower, &CREDENTIAL_MARKERS),
    ),
    ("account_like", ForbiddenCategory::Credential, |l, _| {
        has_account_like(l)
    }),
    ("url_scheme", ForbiddenCategory::GitUrl, |l, _| {
        l.contains("://")
    }),
    ("scp_remote", ForbiddenCategory::GitUrl, |l, _| {
        has_scp_remote(l)
    }),
    ("git_suffix", ForbiddenCategory::GitUrl, |_, lower| {
        lower.contains(".git ") || lower.ends_with(".git") || lower.contains(".git/")
    }),
    ("branch_marker", ForbiddenCategory::GitBranch, |l, lower| {
        has_any(lower, &BRANCH_MARKERS) || l.contains("HEAD")
    }),
    ("commit_hash", ForbiddenCategory::GitCommit, |l, _| {
        has_commit_like_run(l)
    }),
    ("path_prefix", ForbiddenCategory::ProjectPath, |_, lower| {
        has_any(lower, &PATH_PREFIXES)
    }),
    ("windows_drive", ForbiddenCategory::ProjectPath, |l, _| {
        has_windows_drive(l)
    }),
    ("path_separator", ForbiddenCategory::ProjectPath, |l, _| {
        l.contains('/') || l.contains('\\')
    }),
    ("file_extension", ForbiddenCategory::ProjectPath, |l, _| {
        has_file_extension(l)
    }),
    ("non_ascii", ForbiddenCategory::ProjectText, |l, _| {
        !l.is_ascii()
    }),
    ("quote", ForbiddenCategory::ProjectText, |l, _| {
        l.contains('"') || l.contains('\'')
    }),
    ("uppercase", ForbiddenCategory::ProjectText, |l, _| {
        l.bytes().any(|b| b.is_ascii_uppercase())
    }),
    ("punctuation", ForbiddenCategory::ProjectText, |l, _| {
        l.chars()
            .any(|c| matches!(c, ',' | '!' | '?' | ';' | '(' | ')' | '<' | '>' | '*'))
    }),
];

/// 모든 내용 규칙 이름(문서·CLI 표시용).
pub fn rule_names() -> Vec<&'static str> {
    RULES.iter().map(|(name, _, _)| *name).collect()
}

fn scan_line(line_no: usize, line: &str, ctx: &ScanContext, out: &mut BTreeSet<Finding>) {
    let lower = line.to_ascii_lowercase();
    for (rule, category, check) in RULES {
        if check(line, &lower) {
            out.insert(Finding {
                line: line_no,
                category,
                rule,
            });
        }
    }
    for (category, value) in &ctx.values {
        if context_value_appears(&lower, value) {
            out.insert(Finding {
                line: line_no,
                category: *category,
                rule: "context_value",
            });
        }
    }
}

fn scan_content(bytes: &[u8], ctx: &ScanContext, out: &mut BTreeSet<Finding>) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let line_no = index + 1;
        let line = match std::str::from_utf8(raw) {
            Ok(text) => text.to_owned(),
            Err(_) => {
                out.insert(Finding {
                    line: line_no,
                    category: ForbiddenCategory::ProjectText,
                    rule: "non_utf8",
                });
                String::from_utf8_lossy(raw).into_owned()
            }
        };
        scan_line(line_no, &line, ctx, out);
        lines.push(line);
    }
    lines
}

fn too_large() -> ScanReport {
    ScanReport {
        findings: vec![Finding {
            line: 0,
            category: ForbiddenCategory::Unrecognized,
            rule: "too_large",
        }],
    }
}

/// 임의 진단 text를 내용 규칙과 문맥 값으로 검사합니다(구조 검사 없음).
pub fn scan_text(bytes: &[u8], ctx: &ScanContext) -> ScanReport {
    if bytes.len() > MAX_DIAGNOSTIC_BYTES {
        return too_large();
    }
    let mut out = BTreeSet::new();
    scan_content(bytes, ctx, &mut out);
    ScanReport {
        findings: out.into_iter().collect(),
    }
}

/// 진단 번들을 내용 규칙, 문맥 값, 구조 검사로 검사합니다.
pub fn scan_bundle(bytes: &[u8], ctx: &ScanContext) -> ScanReport {
    if bytes.len() > MAX_DIAGNOSTIC_BYTES {
        return too_large();
    }
    let mut out = BTreeSet::new();
    let lines = scan_content(bytes, ctx, &mut out);
    check_structure(&lines, &mut out);
    if out.is_empty() {
        // 줄마다 맞아도 순서·중복·개수 규칙(정규형)을 어기면 그 줄을 알립니다.
        if let Err(e) = decode_bundle(bytes) {
            let line = match e {
                StoreError::Line(line) => line.line,
                _ => 0,
            };
            out.insert(Finding {
                line,
                category: ForbiddenCategory::Unrecognized,
                rule: "layout",
            });
        }
    }
    ScanReport {
        findings: out.into_iter().collect(),
    }
}

fn unrecognized(out: &mut BTreeSet<Finding>, line: usize, rule: &'static str) {
    out.insert(Finding {
        line,
        category: ForbiddenCategory::Unrecognized,
        rule,
    });
}

/// 줄마다 자리에 맞는 schema로 봅니다. 첫 오류에서 멈추지 않고 모든 줄을 봅니다.
fn check_structure(lines: &[String], out: &mut BTreeSet<Finding>) {
    // 파일은 줄바꿈으로 끝나므로 마지막 조각은 비어 있어야 합니다.
    let Some((last, lines)) = lines.split_last() else {
        return;
    };
    if !last.is_empty() {
        unrecognized(out, lines.len() + 1, "final_newline");
    }
    let count = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let line_no = index + 1;
        let ok = match line_no {
            1 => line == BUNDLE_HEADER,
            2 => BundleEnvironment::from_line(line, line_no).is_ok(),
            3 => decode_line(&[&META_SCHEMA], line, line_no).is_ok(),
            n if n == count && n > 3 => {
                matches!(decode_line(&[&END_SCHEMA], line, line_no),
                    Ok((_, values)) if values[0] == FieldValue::Int((count - 4) as u64))
            }
            _ => DiagnosticRecord::from_line(line, line_no).is_ok(),
        };
        if !ok {
            unrecognized(
                out,
                line_no,
                match line_no {
                    1 => "header",
                    2 => "env",
                    3 => "meta",
                    n if n == count => "end",
                    _ => "record",
                },
            );
        }
    }
    if count < 4 {
        unrecognized(out, count + 1, "truncated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::encode_bundle;
    use crate::interaction::InteractionKind;
    use crate::store::{
        AppCrash, Arch, CrashKind, DiagnosticStore, Feature, PerformanceSample, RendererBackend,
        RendererError, RendererStage, ThreadRole, Version3, CACHE_STATES, DOCUMENT_BUCKETS,
        MAX_EVENT_CAPACITY, VISIBLE_BUCKETS,
    };
    use ForbiddenCategory::*;

    /// 모든 enum 값과 경계 정수를 쓴 정상 store.
    fn full_store() -> DiagnosticStore {
        let mut store = DiagnosticStore::new(MAX_EVENT_CAPACITY);
        for (i, kind) in CrashKind::ALL.iter().enumerate() {
            for thread in ThreadRole::ALL {
                store.push_crash(AppCrash {
                    at_ms: u64::MAX - i as u64,
                    kind: *kind,
                    code: u32::MAX,
                    thread: *thread,
                    uptime_ms: 0,
                });
            }
        }
        for backend in RendererBackend::ALL {
            for stage in RendererStage::ALL {
                store.push_renderer_error(RendererError {
                    at_ms: 1_763_856_000_000,
                    backend: *backend,
                    stage: *stage,
                    code: 12_345_678,
                });
            }
        }
        for interaction in InteractionKind::ALL {
            for document in DOCUMENT_BUCKETS {
                for visible in VISIBLE_BUCKETS {
                    for cache in CACHE_STATES {
                        store.push_performance(PerformanceSample {
                            at_ms: 99_999_999_999_999,
                            interaction,
                            document,
                            visible,
                            cache,
                            latency_ns: u64::MAX,
                            gpu_ns: (cache == CACHE_STATES[0]).then_some(1_000_000),
                        });
                    }
                }
            }
        }
        for feature in Feature::ALL {
            store.record_feature_failure(*feature, 0, 1);
            store.record_feature_failure(*feature, u32::MAX, u64::MAX);
        }
        store
    }

    fn bundle(arch: Arch) -> Vec<u8> {
        encode_bundle(
            &BundleEnvironment {
                app_version: Version3::new(4_294_967_295, 0, 12),
                os_version: Version3::new(26, 1, 0),
                arch,
            },
            &full_store(),
        )
    }

    #[test]
    fn valid_bundles_have_no_findings() {
        // 오탐 0: 모든 enum 값·경계 정수·두 CPU 구조.
        for arch in Arch::ALL {
            let bytes = bundle(*arch);
            let report = scan_bundle(&bytes, &ScanContext::new());
            assert!(report.is_clean(), "{:?}", report.findings);
            assert!(scan_text(&bytes, &ScanContext::new()).is_clean());
        }
        let empty = encode_bundle(
            &BundleEnvironment::current(Version3::default()),
            &DiagnosticStore::default(),
        );
        assert!(scan_bundle(&empty, &ScanContext::new()).is_clean());
    }

    /// (기대 범주, secret, 문맥 범주).
    fn corpus() -> Vec<(ForbiddenCategory, &'static str, Option<ForbiddenCategory>)> {
        vec![
            // 프로젝트 텍스트.
            (ProjectText, "안녕하세요 버튼", None),
            (ProjectText, "Checkout Screen", None),
            (ProjectText, "Hello, world!", None),
            (ProjectText, "\"quoted text\"", None),
            (ProjectText, "Ünïcödé", None),
            (ProjectText, "it's done", None),
            (ProjectText, "welcome to acme", Some(ProjectText)),
            // 프로젝트·레이어·토큰 이름(문맥 값).
            (ProjectName, "acme-checkout", Some(ProjectName)),
            (ProjectName, "primary_button", Some(ProjectName)),
            (ProjectName, "color.brand.primary", Some(ProjectName)),
            (ProjectName, "herobanner", Some(ProjectName)),
            // 문맥 없는 이름도 구조 검사가 잡습니다.
            (Unrecognized, "checkout_flow", None),
            (Unrecognized, "herocard", None),
            // 경로.
            (ProjectPath, "/Users/alice/Design/app.stellar", None),
            (ProjectPath, "/home/bob/work/proj", None),
            (ProjectPath, "~/Projects/site", None),
            (ProjectPath, "C:\\Users\\carol\\proj", None),
            (ProjectPath, "/Volumes/External/Design", None),
            (ProjectPath, "file:///tmp/x", None),
            (ProjectPath, "design/pages/home.stellar", None),
            (ProjectPath, "logo.png", None),
            (ProjectPath, "acme_site_root", Some(ProjectPath)),
            // Git URL.
            (GitUrl, "https://github.com/acme/design.git", None),
            (GitUrl, "git@github.com:acme/design.git", None),
            (GitUrl, "ssh://git@host/repo", None),
            (GitUrl, "http://intranet.local/repo", None),
            (GitUrl, "intranet.local", Some(GitUrl)),
            // branch.
            (GitBranch, "refs/heads/main", None),
            (GitBranch, "origin/feature-x", None),
            (GitBranch, "feature/login-page", None),
            (GitBranch, "release/2026.11", None),
            (GitBranch, "HEAD", None),
            (GitBranch, "develop-2026", Some(GitBranch)),
            // commit.
            (GitCommit, "3f786850e387550fdab836ed7e6dc881de23001b", None),
            (GitCommit, "9fceb02", None),
            (
                GitCommit,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                None,
            ),
            (GitCommit, "a1b2c3d4e5", None),
            (GitCommit, "123456789012345678901234", None),
            // 터미널 입출력.
            (TerminalPayload, "\u{1b}[31merror\u{1b}[0m", None),
            (TerminalPayload, "user@mac ~ % ls -la", None),
            (TerminalPayload, "$ cargo build", None),
            (TerminalPayload, "zsh: command not found", None),
            (TerminalPayload, "sudo rm -rf x", None),
            (TerminalPayload, "line1\rline2", None),
            (TerminalPayload, "a\tb", None),
            (TerminalPayload, "make && make install", None),
            (TerminalPayload, "pty_output", None),
            // MCP·에이전트 입출력.
            (
                McpPayload,
                "{\"jsonrpc\":\"2.0\",\"method\":\"tools/call\"}",
                None,
            ),
            (McpPayload, "tools/list", None),
            (McpPayload, "\"params\":{\"name\":\"x\"}", None),
            (McpPayload, "mcp_request", None),
            (McpPayload, "agent_reply", None),
            (McpPayload, "[1,2,3]", None),
            // 자격 증명·계정 식별.
            (Credential, "Bearer abcdef", None),
            (Credential, "sk-live-1234", None),
            (Credential, "ghp_abcdef123456", None),
            (Credential, "password=hunter2", None),
            (Credential, "-----BEGIN PRIVATE KEY-----", None),
            (Credential, "AKIAIOSFODNN7EXAMPLE", None),
            (Credential, "alice@example.com", None),
        ]
    }

    /// secret을 정상 번들의 네 위치(값, 덧붙인 token, 새 줄, key)에 넣습니다.
    fn injections(secret: &str) -> Vec<Vec<u8>> {
        let mut store = DiagnosticStore::new(8);
        store.push_crash(AppCrash {
            at_ms: 1,
            kind: CrashKind::Panic,
            code: 0,
            thread: ThreadRole::Main,
            uptime_ms: 2,
        });
        let good = String::from_utf8(encode_bundle(
            &BundleEnvironment {
                app_version: Version3::new(0, 1, 0),
                os_version: Version3::new(26, 1, 0),
                arch: Arch::Arm64,
            },
            &store,
        ))
        .unwrap();
        let crash = "crash at_ms=1 kind=panic code=0 thread=main uptime_ms=2";
        assert!(good.contains(crash));
        vec![
            good.replace("thread=main", &format!("thread={secret}")),
            good.replace(crash, &format!("{crash} {secret}")),
            good.replace("end records=1", &format!("{secret}\nend records=1")),
            good.replace("code=0", &format!("{secret}=0")),
        ]
        .into_iter()
        .map(String::into_bytes)
        .collect()
    }

    #[test]
    fn secret_corpus_is_detected_completely() {
        // 검증 기준(XVI.8.2): secret corpus 탐지 100%.
        let corpus = corpus();
        let mut total = 0;
        let mut detected = 0;
        let mut misses = Vec::new();
        for (expected, secret, context) in &corpus {
            let ctx = match context {
                Some(category) => {
                    let mut ctx = ScanContext::new();
                    assert!(ctx.add(*category, secret), "{secret}");
                    ctx
                }
                None => ScanContext::new(),
            };
            for (position, bytes) in injections(secret).iter().enumerate() {
                total += 1;
                let report = scan_bundle(bytes, &ctx);
                if report.categories().contains(expected) {
                    detected += 1;
                } else {
                    misses.push(format!("{secret:?}@{position}: {:?}", report.findings));
                }
                // 내용 규칙만으로도(구조 검사 없이) 범주가 맞습니다. 구조 범주는 제외.
                if *expected != Unrecognized {
                    let text = scan_text(bytes, &ctx);
                    assert!(
                        text.categories().contains(expected),
                        "{secret:?}@{position}: {:?}",
                        text.findings
                    );
                }
            }
        }
        assert_eq!(detected, total, "{misses:#?}");
        assert_eq!(total, corpus.len() * 4);
        // 모든 금지 범주가 corpus에 있습니다.
        let covered: BTreeSet<ForbiddenCategory> = corpus.iter().map(|(c, _, _)| *c).collect();
        assert_eq!(covered, ForbiddenCategory::ALL.into_iter().collect());
    }

    #[test]
    fn findings_never_carry_the_secret() {
        let secret = "/Users/alice/secret-project";
        let bytes = &injections(secret)[0];
        let report = scan_bundle(bytes, &ScanContext::new().with(ProjectPath, secret));
        assert!(!report.is_clean());
        for finding in &report.findings {
            assert!(!finding.to_string().contains("alice"), "{finding}");
        }
        let ctx = ScanContext::new().with(ProjectName, "acme-secret");
        assert!(!format!("{ctx:?}").contains("acme"));
    }

    #[test]
    fn context_skips_values_that_cannot_be_told_apart() {
        let mut ctx = ScanContext::new();
        // 짧은 값, schema 어휘, 정수·version은 비교하지 않습니다.
        for value in [
            "ab", "main", "save", "crash", "arch", "12345", "1.2.3", "  ",
        ] {
            assert!(!ctx.add(ProjectName, value), "{value}");
        }
        assert!(ctx.is_empty());
        assert!(ctx.add(ProjectName, "Acme Studio"));
        assert_eq!(ctx.len(), 1);
        // 대소문자 무시.
        let bytes = injections("acme studio")[2].clone();
        assert!(scan_bundle(&bytes, &ctx)
            .findings
            .iter()
            .any(|f| f.rule == "context_value" && f.category == ProjectName));
    }

    #[test]
    fn context_values_inside_schema_words_are_not_findings() {
        // 열린 프로젝트·레이어 이름이 schema 어휘나 형식 이름의 일부(`stellar`, `uptime_ms`의 `time`,
        // `renderer_error`의 `error`, `feature_failure`의 `failure`)여도 정상 번들은 거부하지 않습니다.
        let mut ctx = ScanContext::new();
        for value in ["Stellar", "Time", "Error", "Failure", "Diagnostic"] {
            ctx.add(ProjectName, value);
        }
        // `none`(gpu 시간 없음)은 정상 값이라 비교하지 않습니다.
        assert!(!ctx.clone().add(ProjectName, "None"));
        assert_eq!(ctx.len(), 5);
        let bytes = bundle(Arch::Arm64);
        let report = scan_bundle(&bytes, &ctx);
        assert!(report.is_clean(), "{:?}", report.findings);
        assert!(scan_text(&bytes, &ctx).is_clean());
        // 같은 값이 schema 조각 밖에 나오면 탐지합니다(값 자리, 덧붙인 token, 새 줄, key, 공백을 걸친 값).
        for value in ["stellar", "time", "error", "failure"] {
            for (position, bytes) in injections(value).iter().enumerate() {
                let report = scan_bundle(bytes, &ctx);
                assert!(
                    report
                        .findings
                        .iter()
                        .any(|f| f.rule == "context_value" && f.category == ProjectName),
                    "{value}@{position}: {:?}",
                    report.findings
                );
            }
        }
        let text = b"feature_failure stellar site\n";
        let ctx = ScanContext::new().with(ProjectName, "Stellar Site");
        assert!(scan_text(text, &ctx)
            .findings
            .iter()
            .any(|f| f.rule == "context_value"));
    }

    #[test]
    fn structure_errors_report_every_bad_line() {
        let good = String::from_utf8(bundle(Arch::Arm64)).unwrap();
        let mut lines: Vec<&str> = good.lines().collect();
        let n = lines.len();
        lines[3] = "crash at_ms=1";
        lines[5] = "perf at_ms=x";
        let text = format!("{}\n", lines.join("\n"));
        let report = scan_bundle(text.as_bytes(), &ScanContext::new());
        let bad: Vec<usize> = report.findings.iter().map(|f| f.line).collect();
        assert_eq!(bad, vec![4, 6], "{:?}", report.findings);
        // end 개수 불일치, 끝 줄바꿈 없음, 잘림, 순서 위반.
        let wrong_end = good.replace(&format!("end records={}", n - 4), "end records=1");
        assert_eq!(
            scan_bundle(wrong_end.as_bytes(), &ScanContext::new()).findings,
            vec![Finding {
                line: n,
                category: Unrecognized,
                rule: "end"
            }]
        );
        let no_newline = good.trim_end();
        assert!(scan_bundle(no_newline.as_bytes(), &ScanContext::new())
            .findings
            .iter()
            .any(|f| f.rule == "final_newline"));
        assert!(!scan_bundle(b"stellar.diagnostic-bundle v1\n", &ScanContext::new()).is_clean());
        assert!(!scan_bundle(b"", &ScanContext::new()).is_clean());
        let feature = "feature_failure feature=open_project code=0 count=1 first_ms=1 last_ms=1";
        let swapped = good
            .replacen(feature, "", 1)
            .replace("end records", &format!("{feature}\nend records"));
        let swapped = swapped.replace("\n\n", "\n");
        assert!(!scan_bundle(swapped.as_bytes(), &ScanContext::new()).is_clean());
        // 비UTF-8 bytes.
        let mut bytes = good.into_bytes();
        bytes.splice(0..0, [0xff, b'\n']);
        assert!(scan_bundle(&bytes, &ScanContext::new())
            .findings
            .iter()
            .any(|f| f.rule == "non_utf8" && f.line == 1));
    }

    #[test]
    fn oversized_input_is_one_finding() {
        let bytes = vec![b'a'; MAX_DIAGNOSTIC_BYTES + 1];
        assert_eq!(
            scan_bundle(&bytes, &ScanContext::new()).findings[0].rule,
            "too_large"
        );
        assert_eq!(scan_text(&bytes, &ScanContext::new()).findings.len(), 1);
    }

    #[test]
    fn schema_vocabulary_triggers_no_rule() {
        // schema 어휘 하나하나가 어떤 내용 규칙에도 걸리지 않습니다(오탐 0의 근거).
        for word in schema_vocabulary(&BUNDLE_SCHEMAS) {
            let report = scan_text(word.as_bytes(), &ScanContext::new());
            assert!(report.is_clean(), "{word}: {:?}", report.findings);
        }
        assert_eq!(rule_names().len(), RULES.len());
    }
}
