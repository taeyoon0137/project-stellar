//! CLI 동등성 읽기·dry-run tool (XIX.9.3, ADR-0085 결정 1·8).
//!
//! 각 tool은 `stellar` 명령과 같은 ops 공개 함수를 부르고 결과를 JSON으로만 바꿉니다. 판정·해석을 따로 두지
//! 않습니다. 모두 고정 루트(신뢰 확인한 snapshot의 root)만 읽고, 쓰기 함수(`ApplyMode::Write`, Git index·설정
//! 변경)를 부르지 않습니다. ops 결과의 revision이 세션 snapshot과 다르면 그 사이 저장된 것이므로 `stale`입니다.
//!
//! 오류 문구는 Stellar가 정한 고정 문구이고 프로젝트 문자열을 넣지 않습니다(XIX.9.6).

use stellar_ops::apply::{apply, ApplyError, ApplyMode, MAX_COMMANDS};
use stellar_ops::candidate::session::ConflictSession;
use stellar_ops::compare::{ConflictEntry, SideView};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::css::{copy, paste, CssError};
use stellar_ops::diff::DiffReport;
use stellar_ops::git::{change_id_text, GitError, GitSession, GitStatusReport, StatusRequest};
use stellar_ops::lock::LockPolicy;
use stellar_ops::mcp_merge::{semantic_conflicts, SemanticConflictsError};
use stellar_ops::query::{node_layout, property_path_label, LayoutUnavailable, NodeLayout};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::text_source::source_span;
use stellar_ops::web_frame::{export_web_frame_node, HtmlError};

use super::comparison::{diff_entry, diff_position, opt_value};
use super::{count, fail, id, opt_id, revision, uint, CachedRead, Kind, WithDetail};
use crate::catalog::paging::{self, PageRequest};
use crate::json::JsonValue as J;
use crate::jsonrpc::RpcError;
use crate::proposal::parse_transaction;

/// CSS 선언 블록 최대 byte 수(`stellar css paste` 입력 상한과 같음).
pub const MAX_DECLARATION_BYTES: usize = 16 * 1024;

fn arg_id(args: &J, key: &str) -> Result<StableId, RpcError> {
    args.get(key)
        .and_then(J::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            fail(
                Kind::InvalidArguments,
                "ID 인자는 StableId 문자열이어야 합니다",
            )
        })
}

fn opt_arg_id(args: &J, key: &str) -> Result<Option<StableId>, RpcError> {
    match args.get(key) {
        None => Ok(None),
        Some(_) => arg_id(args, key).map(Some),
    }
}

fn arg_revision(args: &J) -> Result<ProjectRevision, RpcError> {
    args.get("base_revision")
        .and_then(J::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| fail(Kind::InvalidArguments, "base_revision 형식이 틀렸습니다"))
}

fn stale() -> RpcError {
    fail(
        Kind::Stale,
        "요청 base revision이 현재 저장 revision과 다릅니다. 프로젝트 구조를 다시 읽으세요",
    )
}

fn ensure_current(cached: &CachedRead, read: ProjectRevision) -> Result<(), RpcError> {
    if read == cached.snapshot.revision() {
        Ok(())
    } else {
        Err(stale())
    }
}

/// 변경 요약: 종류별 수와 앞쪽 항목(page 상한까지).
fn changes(report: &DiffReport) -> J {
    let shown = report.entries.len().min(paging::MAX_PAGE_ITEMS);
    J::object(vec![
        ("count", count(report.entries.len())),
        (
            "by_kind",
            J::Object(
                report
                    .counts()
                    .into_iter()
                    .map(|(kind, n)| (kind.to_string(), count(n)))
                    .collect(),
            ),
        ),
        (
            "entries",
            J::Array(report.entries[..shown].iter().map(diff_entry).collect()),
        ),
        ("truncated", J::Bool(shown < report.entries.len())),
    ])
}

fn apply_failure(error: &ApplyError) -> RpcError {
    match error {
        ApplyError::BaseRevisionMismatch { .. } | ApplyError::ConcurrentWrite => stale(),
        ApplyError::EmptyTransaction
        | ApplyError::TooManyCommands { .. }
        | ApplyError::InvalidCommand { .. } => {
            fail(Kind::InvalidArguments, "command 형식이 틀렸습니다")
        }
        ApplyError::UnknownCommand { .. } => fail(
            Kind::InvalidArguments,
            "closed enum에 없는 command 종류입니다",
        ),
        ApplyError::ComponentSourceChanged { .. } => fail(
            Kind::Stale,
            "컴포넌트 원본이 없거나 source_hash가 다릅니다. 컴포넌트를 다시 읽으세요",
        ),
        ApplyError::Rejected { .. } | ApplyError::InvalidResult(_) => fail(
            Kind::Rejected,
            "문서 규칙이 command 묶음을 거부했습니다(아무것도 적용하지 않음)",
        ),
        ApplyError::Document(_) => fail(Kind::NotFound, "대상 문서를 읽을 수 없습니다"),
        ApplyError::Open(_) | ApplyError::ComponentSource(_) => {
            fail(Kind::Unavailable, "프로젝트를 읽을 수 없습니다")
        }
        ApplyError::Projection(_) | ApplyError::Lock(_) | ApplyError::Save(_) => {
            fail(Kind::Internal, "dry-run을 완료하지 못했습니다")
        }
    }
}

/// `document.propose_commands`: `stellar apply --dry-run`과 같은 [`apply`] 경로.
pub(super) fn propose_commands(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let request = parse_transaction(args, MAX_COMMANDS).map_err(|e| {
        fail(
            Kind::InvalidArguments,
            if e.code == "unknown-command" {
                "closed enum에 없는 command 종류입니다"
            } else {
                "command 형식이 틀렸습니다"
            },
        )
        .with_detail("code", J::string(e.code))
    })?;
    if request.base_revision != cached.snapshot.revision() {
        return Err(stale());
    }
    let outcome = apply(
        cached.snapshot.root(),
        &request,
        ApplyMode::DryRun,
        &LockPolicy::None,
    )
    .map_err(|e| {
        let failure = apply_failure(&e);
        match e {
            ApplyError::UnknownCommand { index, .. }
            | ApplyError::InvalidCommand { index, .. }
            | ApplyError::Rejected { index, .. }
            | ApplyError::ComponentSourceChanged { index, .. } => {
                failure.with_detail("index", count(index))
            }
            _ => failure,
        }
    })?;
    ensure_current(cached, outcome.base_revision)?;
    Ok(J::object(vec![
        ("mode", J::string("dry-run")),
        ("document", id(outcome.document)),
        ("base_revision", revision(outcome.base_revision)),
        ("command_count", count(outcome.command_count)),
        ("changes", changes(&outcome.diff)),
    ]))
}

fn css_failure(error: &CssError) -> RpcError {
    match error {
        CssError::Open(_) => fail(Kind::Unavailable, "프로젝트를 읽을 수 없습니다"),
        CssError::Document(_) => fail(Kind::NotFound, "대상 문서를 읽을 수 없습니다"),
        CssError::UnknownNode { .. } => fail(Kind::NotFound, "문서에 node가 없습니다"),
        CssError::Paste(_) => fail(Kind::InvalidArguments, "선언 블록 형식이 틀렸습니다"),
        CssError::Ids(_) => fail(Kind::Internal, "새 ID를 만들지 못했습니다"),
        CssError::Apply(e) => apply_failure(e),
    }
}

/// node의 layout longhand 값과 출처(ADR-0082 결정 5). `stellar query --node`의 `node.layout`과 같은 모양입니다.
fn layout_json(layout: &Result<NodeLayout, LayoutUnavailable>) -> J {
    match layout {
        Ok(layout) => J::object(vec![
            ("style_defaults", J::string(layout.style_defaults.clone())),
            ("class", J::string(layout.class)),
            (
                "values",
                J::Array(
                    layout
                        .values
                        .iter()
                        .map(|v| {
                            J::object(vec![
                                ("longhand", J::string(v.longhand)),
                                ("value", J::string(v.value.clone())),
                                ("source", J::string(layout.source_label(v.source))),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        Err(LayoutUnavailable::NoBox) => J::Null,
        Err(error @ LayoutUnavailable::StyleDefaults { code }) => J::object(vec![(
            "error",
            J::object(vec![
                ("code", J::string(*code)),
                ("message", J::string(error.message())),
            ]),
        )]),
    }
}

/// `document.copy_css`: `stellar css copy`와 같은 [`copy`]. `layout`은 세션 snapshot에서 [`node_layout`]으로 푼
/// layout longhand 값과 출처입니다(`stellar query --node`의 `node.layout`과 같음).
pub(super) fn copy_css(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let document = arg_id(args, "document")?;
    let node = arg_id(args, "node")?;
    let copied = copy(cached.snapshot.root(), document, node).map_err(|e| css_failure(&e))?;
    ensure_current(cached, copied.revision)?;
    let layout = node_layout(&cached.snapshot, document, node)
        .map_err(|_| fail(Kind::NotFound, "문서에 node가 없습니다"))?;
    Ok(J::object(vec![
        ("revision", revision(copied.revision)),
        ("document", id(copied.document)),
        ("node", id(copied.node)),
        ("text", J::string(copied.text)),
        (
            "declarations",
            J::Array(
                copied
                    .declarations
                    .iter()
                    .map(|d| {
                        J::object(vec![
                            ("name", J::string(d.name)),
                            ("value", J::string(d.value.clone())),
                            ("stellar_only", J::Bool(d.stellar_only)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("layout", layout_json(&layout)),
    ]))
}

/// `document.propose_css_paste`: `stellar css paste --dry-run`과 같은 [`paste`].
pub(super) fn propose_css_paste(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let base = arg_revision(args)?;
    let document = arg_id(args, "document")?;
    let nodes = args
        .get("nodes")
        .and_then(J::as_array)
        .filter(|items| !items.is_empty() && items.len() <= paging::MAX_PAGE_ITEMS)
        .ok_or_else(|| fail(Kind::InvalidArguments, "nodes는 StableId 배열이어야 합니다"))?
        .iter()
        .map(|item| {
            item.as_str()
                .and_then(|text| text.parse::<StableId>().ok())
                .ok_or_else(|| fail(Kind::InvalidArguments, "nodes는 StableId 배열이어야 합니다"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let text = args
        .get("declarations")
        .and_then(J::as_str)
        .ok_or_else(|| fail(Kind::InvalidArguments, "declarations는 문자열이어야 합니다"))?;
    if text.len() > MAX_DECLARATION_BYTES {
        return Err(fail(Kind::TooLarge, "선언 블록이 16 KiB를 넘습니다"));
    }
    if base != cached.snapshot.revision() {
        return Err(stale());
    }
    let pasted = paste(
        cached.snapshot.root(),
        document,
        &nodes,
        text,
        ApplyMode::DryRun,
        &LockPolicy::None,
    )
    .map_err(|e| css_failure(&e))?;
    ensure_current(cached, pasted.base_revision)?;
    let applied = pasted
        .applied
        .iter()
        .map(|a| {
            J::object(vec![
                ("name", J::string(a.name)),
                ("value", J::string(a.value.clone())),
            ])
        })
        .collect();
    let skipped = pasted
        .skipped
        .iter()
        .map(|s| {
            J::object(vec![
                ("name", J::string(s.name.clone())),
                ("value", J::string(s.value.clone())),
                ("code", J::string(s.code)),
                ("message", J::string(s.message.clone())),
            ])
        })
        .collect();
    let (command_count, changes) = match &pasted.outcome {
        Some(outcome) => (count(outcome.command_count), changes(&outcome.diff)),
        None => (count(0), J::Null),
    };
    Ok(J::object(vec![
        ("mode", J::string("dry-run")),
        ("document", id(document)),
        ("base_revision", revision(pasted.base_revision)),
        ("applied", J::Array(applied)),
        ("skipped", J::Array(skipped)),
        ("command_count", command_count),
        ("changes", changes),
    ]))
}

/// `document.export_web_frame`: `stellar export-web [--frame]`과 같은 [`export_web_frame_node`]. 파일을 쓰지
/// 않습니다. 오류 진단이 있으면 `exported`가 거짓이고 HTML은 `null`입니다.
pub(super) fn export_web_frame(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let report = export_web_frame_node(
        &cached.snapshot,
        arg_id(args, "document")?,
        arg_id(args, "view")?,
        opt_arg_id(args, "frame")?,
    )
    .map_err(|e| match e {
        HtmlError::Unsaved => fail(Kind::NotFound, "저장된 generation이 없습니다"),
        HtmlError::UnsupportedSchema(_) => fail(
            Kind::Unavailable,
            "이 도구가 지원하지 않는 generation schema입니다",
        ),
        HtmlError::Document(_) => fail(Kind::NotFound, "대상 문서를 읽을 수 없습니다"),
        HtmlError::UnknownView(_) => fail(Kind::NotFound, "문서에 View가 없습니다"),
        HtmlError::UnknownFrame(_) => fail(Kind::NotFound, "View에 Frame이 없습니다"),
    })?;
    let fields = vec![
        ("revision", revision(report.revision)),
        ("document", id(report.document)),
        ("view", id(report.view)),
        ("root_frame", id(report.root_frame)),
        ("format", J::string("fixed-free-frame-html")),
        ("exported", J::Bool(report.html.is_some())),
        ("html", report.html.as_ref().map_or(J::Null, J::string)),
        (
            "component_source_hash",
            report
                .component_source_hash
                .map_or(J::Null, |h| J::string(h.to_string())),
        ),
        (
            "diagnostics",
            J::Array(
                report
                    .diagnostics
                    .iter()
                    .map(|d| {
                        J::object(vec![
                            (
                                "severity",
                                J::string(if d.error { "error" } else { "warning" }),
                            ),
                            ("code", J::string(d.code)),
                            ("node", id(d.node)),
                            ("message", J::string(d.message.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    Ok(J::object(fields))
}

/// `document.source_span`: `stellar query <design> --source-span`과 같은 [`source_span`]. 고정 루트의
/// `design` 폴더만 읽습니다.
pub(super) fn node_source_span(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let node = arg_id(args, "node")?;
    let design = cached.snapshot.root().join("design");
    let span = source_span(&design, node)
        .map_err(|_| {
            fail(
                Kind::Unavailable,
                "다중 파일 텍스트 원본을 읽을 수 없습니다",
            )
        })?
        .ok_or_else(|| fail(Kind::NotFound, "텍스트 원본에 node가 없습니다"))?;
    Ok(J::object(vec![
        ("revision", revision(cached.snapshot.revision())),
        ("node", id(node)),
        ("file", J::string(span.file)),
        ("start_line", count(span.start_line as usize)),
        ("end_line", count(span.end_line as usize)),
    ]))
}

fn git_failure(error: &GitError) -> RpcError {
    match error {
        GitError::NotTrusted => fail(Kind::NotTrusted, "프로젝트 신뢰를 확인할 수 없습니다"),
        GitError::StaleCursor => fail(
            Kind::Stale,
            "Git 상태가 바뀌어 cursor를 쓸 수 없습니다. 첫 페이지부터 다시 읽으세요",
        ),
        GitError::InvalidCursor => fail(Kind::InvalidArguments, "cursor 형식이 틀렸습니다"),
        GitError::GitUnavailable
        | GitError::NotWorktreeRoot
        | GitError::Location(_)
        | GitError::Path(_) => fail(
            Kind::Unavailable,
            "프로젝트 루트의 Git 저장소를 쓸 수 없습니다",
        ),
        _ => fail(Kind::Internal, "Git 상태를 읽지 못했습니다"),
    }
}

/// 공용: 세션을 열고 status를 읽습니다.
pub(super) fn read_git_status(
    cached: &CachedRead,
    request: StatusRequest,
) -> Result<GitStatusReport, RpcError> {
    GitSession::open_for_os_user(cached.snapshot.root())
        .and_then(|session| session.status(&request))
        .map_err(|e| git_failure(&e))
}

/// Git 경로 bytes(UTF-8이 아니면 손실 표시).
fn lossy(bytes: &[u8]) -> J {
    J::string(String::from_utf8_lossy(bytes).into_owned())
}

/// `git.status`: `stellar git status`와 같은 [`GitSession::status`]. index를 바꾸지 않습니다.
pub(super) fn git_status(cached: &CachedRead, request: &PageRequest) -> Result<J, RpcError> {
    let limit = request.limit.unwrap_or(paging::DEFAULT_PAGE_ITEMS);
    if !(1..=paging::MAX_PAGE_ITEMS).contains(&limit) {
        return Err(fail(Kind::InvalidArguments, "limit이 page 상한 밖입니다"));
    }
    let report = read_git_status(
        cached,
        StatusRequest {
            cursor: request.cursor.clone(),
            limit,
        },
    )?;
    let repository = &report.repository;
    let counts = &repository.counts;
    let semantic = match &report.semantic {
        Ok(page) => J::object(vec![
            ("staged_total", count(page.staged_total)),
            ("unstaged_total", count(page.unstaged_total)),
            (
                "changes",
                J::Array(
                    page.entries
                        .iter()
                        .map(|entry| {
                            J::object(vec![
                                ("id", J::string(change_id_text(&entry.id))),
                                (
                                    "area",
                                    J::string(match entry.area {
                                        stellar_ops::git::StatusArea::Staged => "staged",
                                        stellar_ops::git::StatusArea::Unstaged => "unstaged",
                                    }),
                                ),
                                ("kind", J::string(entry.kind.name())),
                                ("object", id(entry.object)),
                                ("document", opt_id(entry.document)),
                                (
                                    "property",
                                    entry
                                        .property
                                        .as_ref()
                                        .map_or(J::Null, |p| J::string(property_path_label(p))),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("next_cursor", J::opt_string(page.next_cursor.as_deref())),
        ]),
        Err(_) => J::Null,
    };
    // `stellar git status`와 같이 semantic 상태를 읽지 못하면 `semantic`은 null이고 경고 진단을 둡니다.
    // 원인 문구는 프로젝트에서 온 값일 수 있으므로 data 안에만 둡니다(XIX.9.6).
    let diagnostics = match &report.semantic {
        Ok(_) => Vec::new(),
        Err(reason) => vec![J::object(vec![
            ("severity", J::string("warning")),
            ("code", J::string("semantic-status-unavailable")),
            ("message", J::string(reason.clone())),
        ])],
    };
    Ok(J::object(vec![
        ("revision", revision(cached.snapshot.revision())),
        ("git_revision", J::string(report.revision.to_string())),
        ("head", head_json(&repository.head)),
        (
            "counts",
            J::object(vec![
                ("staged", uint(counts.staged)),
                ("unstaged", uint(counts.unstaged)),
                ("untracked", uint(counts.untracked)),
                ("conflicted", uint(counts.conflicted)),
            ]),
        ),
        (
            "units",
            J::Array(
                report
                    .units
                    .iter()
                    .map(|u| {
                        J::object(vec![
                            ("id", J::string(u.unit.to_string())),
                            ("staged", J::Bool(u.staged)),
                            ("unstaged", J::Bool(u.unstaged)),
                            ("untracked", J::Bool(u.untracked)),
                            ("conflicted", J::Bool(u.conflicted)),
                            ("paths", count(u.paths)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "raw",
            J::Array(
                report
                    .raw
                    .iter()
                    .take(paging::MAX_PAGE_ITEMS)
                    .map(|r| {
                        J::object(vec![
                            ("path", lossy(r.path.as_bytes())),
                            ("xy", J::string(&r.xy)),
                        ])
                    })
                    .collect(),
            ),
        ),
        // raw는 page 상한까지만 싣고 전체 수와 잘림 여부를 함께 줍니다.
        ("raw_total", count(report.raw.len())),
        (
            "raw_truncated",
            J::Bool(report.raw.len() > paging::MAX_PAGE_ITEMS),
        ),
        ("semantic", semantic),
        ("diagnostics", J::Array(diagnostics)),
    ]))
}

fn head_json(head: &stellar_ops::git::HeadState) -> J {
    use stellar_ops::git::HeadState;
    match head {
        HeadState::Branch { name, commit } => J::object(vec![
            ("state", J::string("branch")),
            ("branch", lossy(name)),
            ("commit", J::string(commit.to_string())),
        ]),
        HeadState::Detached { commit } => J::object(vec![
            ("state", J::string("detached")),
            ("branch", J::Null),
            ("commit", J::string(commit.to_string())),
        ]),
        HeadState::Unborn { branch } => J::object(vec![
            ("state", J::string("unborn")),
            ("branch", lossy(branch)),
            ("commit", J::Null),
        ]),
    }
}

fn side_json(side: &SideView) -> J {
    match side {
        SideView::Missing => J::object(vec![("state", J::string("missing"))]),
        SideView::Object(kind) => J::object(vec![
            ("state", J::string("object")),
            ("object_kind", J::string(kind.clone())),
        ]),
        SideView::Position(position) => {
            let mut value = diff_position(Some(position));
            if let J::Object(fields) = &mut value {
                fields.insert(0, ("state".into(), J::string("position")));
            }
            value
        }
        SideView::Value(value) => J::object(vec![
            ("state", J::string("value")),
            ("value", opt_value(value.as_ref())),
        ]),
    }
}

fn conflict_json(conflict: &ConflictEntry) -> Vec<(String, J)> {
    let mut fields: Vec<(String, J)> = vec![
        ("kind".into(), J::string(conflict.kind)),
        ("document".into(), opt_id(conflict.document)),
        ("object".into(), id(conflict.object)),
        ("target".into(), J::string(conflict.target)),
        ("path".into(), J::opt_string(conflict.path.as_deref())),
        ("base".into(), side_json(&conflict.base)),
        ("ours".into(), side_json(&conflict.ours)),
        ("theirs".into(), side_json(&conflict.theirs)),
    ];
    if let Some(component) = conflict.component {
        fields.push(("component".into(), id(component)));
    }
    fields
}

fn session_json(session: &ConflictSession) -> J {
    let conflicts = session.conflicts();
    let units = session
        .units()
        .iter()
        .map(|unit| {
            let status = unit.status();
            let entries = conflicts
                .iter()
                .filter(|c| std::ptr::eq(c.unit, unit))
                .map(|c| {
                    let mut fields = vec![("index".to_string(), count(c.index))];
                    fields.extend(conflict_json(&c.entry()));
                    fields.push((
                        "allowed".into(),
                        J::Array(c.allowed().into_iter().map(J::string).collect()),
                    ));
                    fields.push((
                        "resolution".into(),
                        c.resolution().map_or(J::Null, J::string),
                    ));
                    J::Object(fields)
                })
                .collect();
            J::object(vec![
                ("path", J::string(unit.path().to_string_lossy())),
                ("kind", J::string(unit.kind().name())),
                ("objects", count(unit.objects().len())),
                ("conflicts", J::Array(entries)),
                ("unresolved", count(status.unresolved)),
                ("violations", count(status.violations)),
                ("writable", J::Bool(status.writable())),
            ])
        })
        .collect();
    J::object(vec![
        ("operation", J::string(session.operation().name())),
        ("units", J::Array(units)),
        (
            "raw_conflicts",
            J::Array(
                session
                    .raw_conflicts()
                    .iter()
                    .map(|p| J::string(p.to_string_lossy()))
                    .collect(),
            ),
        ),
        (
            "unsupported",
            J::Array(
                session
                    .unsupported()
                    .iter()
                    .map(|(path, reason)| {
                        J::object(vec![
                            ("path", J::string(path.to_string_lossy())),
                            ("reason", J::string(reason.as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// `git.semantic_conflicts`: `stellar merge-driver resolve`(`--apply` 없음)와 같은 [`ConflictSession`]을
/// 읽기 전용 Git 포트로 엽니다. 해결을 적용하지 않습니다.
pub(super) fn git_semantic_conflicts(cached: &CachedRead) -> Result<J, RpcError> {
    let session = semantic_conflicts(cached.snapshot.root()).map_err(|e| match e {
        SemanticConflictsError::Git(e) => git_failure(&e),
        SemanticConflictsError::Session(_) => {
            fail(Kind::Unavailable, "unmerged index를 읽을 수 없습니다")
        }
    })?;
    let mut body = session_json(&session);
    if let J::Object(fields) = &mut body {
        fields.insert(0, ("revision".into(), revision(cached.snapshot.revision())));
    }
    Ok(body)
}
