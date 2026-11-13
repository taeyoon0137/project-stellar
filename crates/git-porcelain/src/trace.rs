//! Git command trace와 debug log redaction (XIV.1.6).
//!
//! [`GitRunner`](crate::runner::GitRunner)는 프로세스를 만든 모든 실행을 [`GitCommandTrace`]로
//! 기록합니다. trace에는 **명령 종류·소요 시간·종료 상태만** 들어갑니다. 필드가 모두 고정 enum·숫자인
//! `Copy` 타입이라 URL·branch·경로·commit hash 같은 문자열을 담을 수 없습니다(telemetry 허용 범위,
//! 아키텍처 39절). 명령 종류는 알려진 subcommand 고정 목록에서만 고르고, 모르면
//! [`GitCommandKind::Other`]입니다.
//!
//! 로컬 debug log([`GitDebugLine`])는 argv 모양과 실패 시 stderr를 담되 항상 redaction을 거칩니다.
//! 생성자가 redaction을 수행하므로 원문을 넣을 수 없습니다.
//!
//! - argv: subcommand와 option 이름은 유지하고, option 값(`--x=<값>`)·`-c`·`-C` 값·위치 인자는
//!   `<url>`·`<oid>`·`<ref>`·`<path>`·`<config>`·`<arg>`로 바꿉니다. `--` 뒤 인자는 모두 바꿉니다.
//! - 텍스트(stderr): URL(자격 증명 포함)·scp 형식 원격·따옴표로 감싼 이름(Git이 branch·경로를 따옴표로
//!   출력)·절대 경로·7자 이상 16진수(commit hash)를 바꾸고 [`MAX_DEBUG_TEXT_BYTES`]에서 자릅니다.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

/// debug log에 남기는 텍스트 최대 길이(byte, redaction 전 입력 기준).
pub const MAX_DEBUG_TEXT_BYTES: usize = 16 * 1024;

/// Git 명령 종류(telemetry에 남길 수 있는 고정 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitCommandKind {
    /// `add`.
    Add,
    /// `bisect`.
    Bisect,
    /// `branch`.
    Branch,
    /// `cat-file`.
    CatFile,
    /// `check-attr`.
    CheckAttr,
    /// `checkout`.
    Checkout,
    /// `cherry-pick`.
    CherryPick,
    /// `clone`.
    Clone,
    /// `commit`.
    Commit,
    /// `config`.
    Config,
    /// `diff`.
    Diff,
    /// `fetch`.
    Fetch,
    /// `for-each-ref`.
    ForEachRef,
    /// `hash-object`.
    HashObject,
    /// `init`.
    Init,
    /// `lfs`(git-lfs).
    Lfs,
    /// `log`.
    Log,
    /// `ls-files`.
    LsFiles,
    /// `ls-remote`.
    LsRemote,
    /// `ls-tree`.
    LsTree,
    /// `merge`.
    Merge,
    /// `merge-base`.
    MergeBase,
    /// `notes`.
    Notes,
    /// `pull`.
    Pull,
    /// `push`.
    Push,
    /// `rebase`.
    Rebase,
    /// `reflog`.
    Reflog,
    /// `remote`.
    Remote,
    /// `reset`.
    Reset,
    /// `restore`.
    Restore,
    /// `rev-list`.
    RevList,
    /// `rev-parse`.
    RevParse,
    /// `rm`.
    Rm,
    /// `show`.
    Show,
    /// `sparse-checkout`.
    SparseCheckout,
    /// `stash`.
    Stash,
    /// `status`.
    Status,
    /// `submodule`.
    Submodule,
    /// `switch`.
    Switch,
    /// `symbolic-ref`.
    SymbolicRef,
    /// `tag`.
    Tag,
    /// `update-index`.
    UpdateIndex,
    /// `update-ref`.
    UpdateRef,
    /// `--version`.
    Version,
    /// `worktree`.
    Worktree,
    /// subcommand 없이 global option만(`--exec-path` 등).
    GlobalOption,
    /// 목록에 없는 subcommand(이름은 기록하지 않음).
    Other,
}

const KINDS: &[(&str, GitCommandKind)] = &[
    ("add", GitCommandKind::Add),
    ("bisect", GitCommandKind::Bisect),
    ("branch", GitCommandKind::Branch),
    ("cat-file", GitCommandKind::CatFile),
    ("check-attr", GitCommandKind::CheckAttr),
    ("checkout", GitCommandKind::Checkout),
    ("cherry-pick", GitCommandKind::CherryPick),
    ("clone", GitCommandKind::Clone),
    ("commit", GitCommandKind::Commit),
    ("config", GitCommandKind::Config),
    ("diff", GitCommandKind::Diff),
    ("fetch", GitCommandKind::Fetch),
    ("for-each-ref", GitCommandKind::ForEachRef),
    ("hash-object", GitCommandKind::HashObject),
    ("init", GitCommandKind::Init),
    ("lfs", GitCommandKind::Lfs),
    ("log", GitCommandKind::Log),
    ("ls-files", GitCommandKind::LsFiles),
    ("ls-remote", GitCommandKind::LsRemote),
    ("ls-tree", GitCommandKind::LsTree),
    ("merge", GitCommandKind::Merge),
    ("merge-base", GitCommandKind::MergeBase),
    ("notes", GitCommandKind::Notes),
    ("pull", GitCommandKind::Pull),
    ("push", GitCommandKind::Push),
    ("rebase", GitCommandKind::Rebase),
    ("reflog", GitCommandKind::Reflog),
    ("remote", GitCommandKind::Remote),
    ("reset", GitCommandKind::Reset),
    ("restore", GitCommandKind::Restore),
    ("rev-list", GitCommandKind::RevList),
    ("rev-parse", GitCommandKind::RevParse),
    ("rm", GitCommandKind::Rm),
    ("show", GitCommandKind::Show),
    ("sparse-checkout", GitCommandKind::SparseCheckout),
    ("stash", GitCommandKind::Stash),
    ("status", GitCommandKind::Status),
    ("submodule", GitCommandKind::Submodule),
    ("switch", GitCommandKind::Switch),
    ("symbolic-ref", GitCommandKind::SymbolicRef),
    ("tag", GitCommandKind::Tag),
    ("update-index", GitCommandKind::UpdateIndex),
    ("update-ref", GitCommandKind::UpdateRef),
    ("worktree", GitCommandKind::Worktree),
];

/// 값을 다음 인자로 받는 Git global option.
const GLOBAL_WITH_VALUE: &[&str] = &[
    "-c",
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
    "--super-prefix",
];

/// 값이 다음 인자인 global option의 placeholder.
fn global_value_placeholder(option: &str) -> &'static str {
    match option {
        "-c" | "--config-env" => "<config>",
        _ => "<path>",
    }
}

/// 이름을 기록해도 되는 2단계 subcommand(`git <kind> <sub>`).
fn second_level(kind: GitCommandKind) -> &'static [&'static str] {
    match kind {
        GitCommandKind::Lfs => &[
            "checkout", "env", "fetch", "fsck", "install", "ls-files", "migrate", "pointer",
            "pull", "push", "status", "track", "untrack", "version",
        ],
        GitCommandKind::Worktree => &[
            "add", "list", "lock", "move", "prune", "remove", "repair", "unlock",
        ],
        GitCommandKind::Stash => &[
            "apply", "branch", "clear", "create", "drop", "list", "pop", "push", "show", "store",
        ],
        GitCommandKind::Remote => &[
            "add", "get-url", "prune", "remove", "rename", "set-head", "set-url", "show",
        ],
        GitCommandKind::Notes => &["add", "append", "copy", "list", "remove", "show"],
        GitCommandKind::Bisect => &["bad", "good", "log", "new", "old", "reset", "skip", "start"],
        GitCommandKind::SparseCheckout => &["add", "disable", "init", "list", "reapply", "set"],
        GitCommandKind::Submodule => &["foreach", "init", "status", "summary", "sync", "update"],
        GitCommandKind::Reflog => &["delete", "exists", "expire", "show"],
        _ => &[],
    }
}

impl GitCommandKind {
    /// 고정 이름(telemetry 이벤트 이름으로 사용 가능).
    pub fn as_str(self) -> &'static str {
        match self {
            GitCommandKind::Version => "--version",
            GitCommandKind::GlobalOption => "(global-option)",
            GitCommandKind::Other => "(other)",
            kind => KINDS
                .iter()
                .find(|(_, k)| *k == kind)
                .map(|(name, _)| *name)
                .unwrap_or("(other)"),
        }
    }

    /// argv에서 명령 종류를 고릅니다(global option을 건너뛴 첫 인자).
    pub fn from_argv(args: &[OsString]) -> GitCommandKind {
        match subcommand_index(args) {
            Some(i) => {
                let name = args[i].to_str().unwrap_or_default();
                KINDS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, k)| *k)
                    .unwrap_or(GitCommandKind::Other)
            }
            None if args.iter().any(|a| a == "--version") => GitCommandKind::Version,
            None => GitCommandKind::GlobalOption,
        }
    }
}

/// global option 뒤 첫 subcommand 위치.
fn subcommand_index(args: &[OsString]) -> Option<usize> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].to_str().unwrap_or_default();
        if GLOBAL_WITH_VALUE.contains(&arg) {
            i += 2;
        } else if arg.starts_with('-') {
            i += 1;
        } else {
            return Some(i);
        }
    }
    None
}

/// 실행 종료 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTraceExit {
    /// 종료 코드.
    Code(i32),
    /// 시그널로 종료(종료 코드 없음).
    Signal,
    /// 프로세스 생성 실패.
    SpawnFailed,
    /// 제한 시간 초과로 종료시킴.
    TimedOut,
    /// 취소로 종료시킴.
    Cancelled,
    /// 종료 대기 실패.
    WaitFailed,
}

/// Git 명령 trace(telemetry 허용 필드만: 종류·소요 시간·종료 상태).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitCommandTrace {
    /// 명령 종류.
    pub kind: GitCommandKind,
    /// spawn부터 종료까지 걸린 시간.
    pub duration: Duration,
    /// 종료 상태.
    pub exit: GitTraceExit,
}

/// redaction을 거친 로컬 debug log 한 줄(원문을 담을 수 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDebugLine {
    trace: GitCommandTrace,
    argv: String,
    stderr: Option<String>,
}

impl GitDebugLine {
    /// trace와 argv(redaction 적용)로 만듭니다.
    pub fn new(trace: GitCommandTrace, args: &[OsString]) -> GitDebugLine {
        GitDebugLine {
            trace,
            argv: redact_argv(args),
            stderr: None,
        }
    }

    /// stderr(redaction 적용)를 덧붙입니다.
    pub fn with_stderr(mut self, stderr: &[u8]) -> GitDebugLine {
        self.stderr = Some(redact_debug_text(&String::from_utf8_lossy(stderr)));
        self
    }

    /// trace.
    pub fn trace(&self) -> &GitCommandTrace {
        &self.trace
    }

    /// redaction된 argv(`git <...>`).
    pub fn redacted_argv(&self) -> &str {
        &self.argv
    }

    /// redaction된 stderr.
    pub fn redacted_stderr(&self) -> Option<&str> {
        self.stderr.as_deref()
    }

    /// 한 줄 텍스트로 만듭니다.
    pub fn render(&self) -> String {
        let exit = match self.trace.exit {
            GitTraceExit::Code(code) => code.to_string(),
            other => format!("{other:?}"),
        };
        let mut line = format!(
            "{} kind={} exit={exit} duration_ms={}",
            self.argv,
            self.trace.kind.as_str(),
            self.trace.duration.as_millis()
        );
        if let Some(stderr) = &self.stderr {
            line.push_str(" stderr=");
            line.push_str(&stderr.replace('\n', "\\n"));
        }
        line
    }
}

/// trace 수신자. 실행 스레드에서 호출되므로 빠르게 반환해야 합니다.
pub trait GitTraceSink: Send + Sync {
    /// telemetry용 trace(종류·시간·종료 상태만).
    fn record(&self, trace: &GitCommandTrace);

    /// 로컬 debug log 한 줄(redaction 적용). 기본은 무시합니다.
    fn record_debug(&self, _line: &GitDebugLine) {}
}

/// 실행기가 공유하는 sink handle.
#[derive(Clone)]
pub(crate) struct TraceSinkHandle(pub(crate) Arc<dyn GitTraceSink>);

impl std::fmt::Debug for TraceSinkHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TraceSinkHandle(..)")
    }
}

fn is_hex_run(s: &str) -> bool {
    s.len() >= 7 && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn looks_like_url(s: &str) -> bool {
    if s.contains("://") {
        return true;
    }
    // scp 형식 `user@host:path`.
    match (s.find('@'), s.find(':')) {
        (Some(at), Some(colon)) => at < colon && !s[..colon].contains('/'),
        _ => false,
    }
}

/// option 이름으로 유지해도 되는 모양(`-x`, `--name`, 소문자·숫자·`-`).
fn is_option_name(s: &str) -> bool {
    let body = s
        .strip_prefix("--")
        .or_else(|| s.strip_prefix('-'))
        .unwrap_or_default();
    !body.is_empty()
        && body.len() <= 32
        && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 위치 인자 placeholder.
fn positional_placeholder(s: &str) -> &'static str {
    if looks_like_url(s) {
        "<url>"
    } else if is_hex_run(s) {
        "<oid>"
    } else if s.starts_with("refs/") || (s.contains(':') && s.contains("refs/")) {
        "<ref>"
    } else if s.starts_with('/') || s.starts_with("~/") {
        "<path>"
    } else {
        "<arg>"
    }
}

/// argv를 debug log용으로 redaction합니다(`git <subcommand> <options/placeholders>`).
pub fn redact_argv(args: &[OsString]) -> String {
    let mut out = String::from("git");
    let sub = subcommand_index(args);
    let kind = GitCommandKind::from_argv(args);
    let mut after_separator = false;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].to_string_lossy();
        let arg = raw.as_ref();
        out.push(' ');
        let before_sub = sub.is_none_or(|s| i < s);
        if Some(i) == sub {
            out.push_str(match kind {
                GitCommandKind::Other => "<subcommand>",
                known => known.as_str(),
            });
            // 2단계 subcommand(`worktree add`, `lfs track` 등)는 고정 목록에 있을 때만 남깁니다.
            if let Some(next) = args.get(i + 1).and_then(|a| a.to_str()) {
                if second_level(kind).contains(&next) {
                    out.push(' ');
                    out.push_str(next);
                    i += 1;
                }
            }
        } else if before_sub && GLOBAL_WITH_VALUE.contains(&arg) {
            out.push_str(arg);
            if args.get(i + 1).is_some() {
                out.push(' ');
                out.push_str(global_value_placeholder(arg));
                i += 1;
            }
        } else if after_separator {
            out.push_str(positional_placeholder(arg));
        } else if arg == "--" {
            out.push_str("--");
            after_separator = true;
        } else if let Some((name, _)) = arg.split_once('=').filter(|(n, _)| is_option_name(n)) {
            out.push_str(name);
            out.push_str("=<value>");
        } else if is_option_name(arg) {
            out.push_str(arg);
        } else {
            out.push_str(positional_placeholder(arg));
        }
        i += 1;
    }
    out
}

/// 영숫자 조각 중 hash 모양(7자 이상 16진수)만 `<oid>`로 바꿉니다(`abc1234..def5678` 등).
fn redact_hex_pieces(core: &str, out: &mut String) {
    let mut piece = String::new();
    let flush = |piece: &mut String, out: &mut String| {
        if is_hex_run(piece) {
            out.push_str("<oid>");
        } else {
            out.push_str(piece);
        }
        piece.clear();
    };
    for c in core.chars() {
        if c.is_ascii_alphanumeric() {
            piece.push(c);
        } else {
            flush(&mut piece, out);
            out.push(c);
        }
    }
    flush(&mut piece, out);
}

/// 공백 사이 token 하나를 redaction합니다(앞뒤 구두점은 보존).
fn redact_token(token: &str, out: &mut String) {
    let start = token
        .find(|c: char| !"([<,".contains(c))
        .unwrap_or(token.len());
    let end = token
        .trim_end_matches(|c: char| ")]>,.;:".contains(c))
        .len()
        .max(start);
    let (head, core, tail) = (&token[..start], &token[start..end], &token[end..]);
    out.push_str(head);
    if looks_like_url(core) {
        out.push_str("<url>");
    } else if core.starts_with('/') || core.starts_with("~/") {
        out.push_str("<path>");
    } else if core.starts_with("refs/") {
        out.push_str("<ref>");
    } else {
        redact_hex_pieces(core, out);
    }
    out.push_str(tail);
}

/// 로컬 debug log에 넣을 텍스트(예: stderr)를 redaction합니다.
///
/// 따옴표(`'…'`, `"…"`) 안은 통째로 `<redacted>`로 바꿉니다. Git은 branch·경로·remote 이름을
/// 따옴표로 출력하므로 이름이 남지 않습니다. 따옴표 없는 자유 문장 속 branch 이름은 일반 단어와 구분할
/// 수 없으므로, debug log는 이 함수의 결과만 저장하고 telemetry에는 stderr를 보내지 않습니다.
pub fn redact_debug_text(text: &str) -> String {
    let mut limit = text.len().min(MAX_DEBUG_TEXT_BYTES);
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let input = &text[..limit];
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                redact_token(&token, &mut out);
                token.clear();
                out.push(c);
                out.push_str("<redacted>");
                // 닫는 따옴표까지(줄 끝을 넘지 않음) 통째로 가립니다.
                for inner in chars.by_ref() {
                    if inner == c || inner == '\n' {
                        out.push(inner);
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {
                redact_token(&token, &mut out);
                token.clear();
                out.push(c);
            }
            c => token.push(c),
        }
    }
    redact_token(&token, &mut out);
    if limit < text.len() {
        out.push_str("…(truncated)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::argv;

    #[test]
    fn kind_comes_from_fixed_list_after_global_options() {
        assert_eq!(
            GitCommandKind::from_argv(&argv(["status", "-z"])),
            GitCommandKind::Status
        );
        assert_eq!(
            GitCommandKind::from_argv(&argv(["-c", "a=b", "-C", "/x", "push", "origin"])),
            GitCommandKind::Push
        );
        assert_eq!(
            GitCommandKind::from_argv(&argv(["--no-pager", "lfs", "track"])),
            GitCommandKind::Lfs
        );
        assert_eq!(
            GitCommandKind::from_argv(&argv(["--version"])),
            GitCommandKind::Version
        );
        assert_eq!(
            GitCommandKind::from_argv(&argv(["--exec-path"])),
            GitCommandKind::GlobalOption
        );
        assert_eq!(
            GitCommandKind::from_argv(&argv(["secret-alias-name"])),
            GitCommandKind::Other
        );
        assert_eq!(GitCommandKind::Other.as_str(), "(other)");
        for (name, kind) in KINDS {
            assert_eq!(kind.as_str(), *name);
            assert_eq!(GitCommandKind::from_argv(&argv([*name])), *kind);
        }
    }

    #[test]
    fn trace_is_copy_and_has_no_text_fields() {
        // 문자열 필드가 생기면 Copy가 깨져 컴파일 실패합니다.
        fn assert_copy<T: Copy>() {}
        assert_copy::<GitCommandTrace>();
        assert_copy::<GitCommandKind>();
        assert_copy::<GitTraceExit>();
    }

    #[test]
    fn argv_redaction_keeps_shape_only() {
        let cases: Vec<(Vec<&str>, &str)> = vec![
            (
                vec!["status", "--porcelain=v2", "-z", "--branch"],
                "git status --porcelain=<value> -z --branch",
            ),
            (
                vec![
                    "-c",
                    "credential.helper=store",
                    "push",
                    "--porcelain",
                    "https://alice:tok@github.com/o/r.git",
                    "refs/heads/secret:refs/heads/secret",
                ],
                "git -c <config> push --porcelain <url> <ref>",
            ),
            (
                vec!["switch", "--no-guess", "secret-branch"],
                "git switch --no-guess <arg>",
            ),
            (
                vec![
                    "rev-parse",
                    "-q",
                    "--verify",
                    "0123456789abcdef0123456789abcdef01234567^{commit}",
                ],
                "git rev-parse -q --verify <arg>",
            ),
            (
                vec!["cat-file", "-p", "0123456789abcdef0123456789abcdef01234567"],
                "git cat-file -p <oid>",
            ),
            (
                vec!["worktree", "add", "-b", "feat", "--", "/Users/a/wt", "main"],
                "git worktree add -b <arg> -- <path> <arg>",
            ),
            (
                vec!["lfs", "track", "--filename", "--", "assets/secret [1].psd"],
                "git lfs track --filename -- <arg>",
            ),
            (vec!["lfs", "secret-ext"], "git lfs <arg>"),
            (vec!["my-alias", "x"], "git <subcommand> <arg>"),
            (
                vec!["remote", "add", "origin", "git@github.com:o/secret.git"],
                "git remote add <arg> <url>",
            ),
            (
                vec!["-C", "/Users/a/repo", "status"],
                "git -C <path> status",
            ),
            (
                vec!["commit", "-m", "--secret=x"],
                "git commit -m --secret=<value>",
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(redact_argv(&argv(args.clone())), expected, "{args:?}");
        }
    }

    #[test]
    fn text_redaction_hides_urls_names_paths_and_hashes() {
        let stderr =
            "fatal: unable to access 'https://alice:ghp_SECRET@github.com/o/private.git/': 403\n\
                      error: pathspec \"design/secret page.yaml\" did not match\n\
                      ! [rejected] secret-branch -> secret-branch (fetch first)\n\
                      hint: see /Users/alice/Projects/secret/stellar.yaml at deadbeefcafe1234\n\
                      Updating abc1234..def5678\n\
                      remote: git@github.com:o/private.git\n\
                      ref refs/heads/secret-branch\n";
        let out = redact_debug_text(stderr);
        for secret in [
            "ghp_SECRET",
            "alice",
            "github.com",
            "private",
            "secret page",
            "/Users",
            "deadbeefcafe1234",
            "abc1234",
            "def5678",
            "refs/heads",
        ] {
            assert!(!out.contains(secret), "{secret:?} 노출: {out}");
        }
        assert!(
            out.contains("fatal: unable to access '<redacted>': 403"),
            "{out}"
        );
        assert!(out.contains("Updating <oid>..<oid>"), "{out}");
        assert!(out.contains("<path>"), "{out}");
        // 따옴표 없는 자유 문장의 branch 이름은 구분할 수 없습니다(문서화된 한계).
        assert!(out.contains("[rejected]"));
        // 닫히지 않은 따옴표도 줄 끝까지 가립니다.
        assert_eq!(redact_debug_text("x 'open secret\ny"), "x '<redacted>\ny");
        // 길이 상한.
        let long = "a".repeat(MAX_DEBUG_TEXT_BYTES + 10);
        assert!(redact_debug_text(&long).ends_with("…(truncated)"));
        let multibyte = "가".repeat(MAX_DEBUG_TEXT_BYTES);
        assert!(redact_debug_text(&multibyte).ends_with("…(truncated)"));
    }

    #[test]
    fn debug_line_renders_only_redacted_fields() {
        let trace = GitCommandTrace {
            kind: GitCommandKind::Push,
            duration: Duration::from_millis(42),
            exit: GitTraceExit::Code(1),
        };
        let line = GitDebugLine::new(
            trace,
            &argv([
                "push",
                "https://alice:tok@github.com/o/r.git",
                "refs/heads/x",
            ]),
        )
        .with_stderr(b"fatal: repository 'https://alice:tok@github.com/o/r.git/' not found\n");
        assert_eq!(line.trace(), &trace);
        assert_eq!(line.redacted_argv(), "git push <url> <ref>");
        assert_eq!(
            line.render(),
            "git push <url> <ref> kind=push exit=1 duration_ms=42 stderr=fatal: repository '<redacted>' not found\\n"
        );
        let timed = GitDebugLine::new(
            GitCommandTrace {
                exit: GitTraceExit::TimedOut,
                ..trace
            },
            &argv(["fetch"]),
        );
        assert_eq!(
            timed.render(),
            "git fetch kind=push exit=TimedOut duration_ms=42"
        );
        assert_eq!(timed.redacted_stderr(), None);
    }
}
