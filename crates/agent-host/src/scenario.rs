//! 내장 에이전트 E2E 시나리오 정의와 loader (XXIX.6.1, ADR-0101 결정 6).
//!
//! 테스트 대역(XXIX.2.2) 기반 E2E 시나리오를 공통 fixture(`crates/agent-host/tests/fixtures/e2e/*.json`, 형식
//! [`SCENARIO_FORMAT`])로 정의합니다. 같은 정의를 두 실행기가 씁니다.
//!
//! - **headless 실행기**(`tests/e2e_scenarios.rs`): 테스트 대역 CLI 프로세스, [`crate::session::AgentSession`],
//!   [`crate::approval::ApprovalStore`], [`crate::assembly::ChatAssembly`]로 세션 수준 단계와 확인을 실행합니다.
//!   bridge 결합은 단계(`bridge-bound` 등)로 흉내 냅니다.
//! - **앱 실행기**(XXIX.6.2, macOS 앱): 같은 단계에 더해 문서·history가 필요한 `app` 단계(Undo, 되돌리기,
//!   유지)와 `check.app` 확인을 실행합니다. headless 실행기는 `app` 단계를 건너뛰고 기록만 합니다.
//!
//! 형식은 닫혀 있습니다. 모르는 key·단계·확인 이름, 시각 역행, 첫 단계가 `start`가 아님, 열린 적 없는 요청
//! 번호에 대한 응답은 load에서 거부합니다. 같은 입력은 항상 같은 정규 표기([`Scenario::to_golden`])입니다.
//!
//! | 단계(`do`) | 인자 | 뜻 |
//! |---|---|---|
//! | `start` | - | 사용자가 시작 확인(신뢰된 프로젝트, 고지 확인됨) |
//! | `launch` | `prompt` | 실행 전 확인 뒤 테스트 대역 세션 실행, 사용자 문장 전달 |
//! | `bridge-bound`·`bridge-disconnected`·`tick` | - | bridge 결합·끊김, 시각 진행 |
//! | `grant` | `permission` | 앱이 권한 승인 |
//! | `lease` | `lease` | 앱이 lease 획득 |
//! | `open-scope` | `targets` | 범위 승인 요청 |
//! | `open-batch` | `proposal` | 적용 전 batch 승인 요청(제안 대기) |
//! | `answer` | `request`(연 순서 1부터), `answer` | 사용자 응답 |
//! | `apply` | `batch`, `history`, `proposal`(없으면 바로 적용) | batch 적용과 출처 기록 |
//! | `complete` | - | CompleteAgentEdit |
//! | `stop` | - | 사용자 중지 |
//! | `release-lease` | - | 사용자의 lease 해제 |
//! | `await-output` | `until`(`decode-failed`·`turn-completed`·`closed`) | 대역 출력 수신 대기와 신호 처리 |
//! | `app` | `action`(`undo`·`revert-batch`·`revert-session`·`keep`), `batch` | 앱 실행기 전용 |
//! | `check` | 아래 확인 | 그 시점 상태 확인 |
//!
//! `check` 확인: `state`, `leases`, `permissions`, `pending_requests`, `pending_proposals`, `applied`,
//! `effects`(지금까지 낸 세션 효과 이름, 처음 나온 순서로 중복 없이 정확히 일치), `chat`(채팅 item 종류가 모두 포함), `outcomes`(승인 결과
//! 코드 순서), `app`(앱 실행기 전용 확인 이름, [`APP_EXPECTATIONS`]).

use std::fmt;

use stellar_agent_protocol::permission::Permission;

use crate::json::{self, JsonLimits, JsonValue};
use crate::session::SessionMode;

/// fixture 형식 이름.
pub const SCENARIO_FORMAT: &str = "stellar-agent-e2e-scenario/1";

/// 앱 실행기 전용 확인 이름(닫힌 목록).
pub const APP_EXPECTATIONS: [&str; 9] = [
    "history-entries-1",
    "undo-restores-baseline",
    "applied-batches-kept",
    "provenance-shown",
    "provenance-matches-index",
    "revert-batch-baseline",
    "revert-session-baseline",
    "no-auto-rollback",
    "default-view-chat",
];

/// 세션 효과 이름(닫힌 목록, [`crate::session::SessionEffect::kind`]).
pub const EFFECT_NAMES: [&str; 12] = [
    "terminate",
    "interrupted-entry",
    "undo-entry",
    "release-leases",
    "revoke",
    "revoke-some",
    "reject-requests",
    "discard-proposals",
    "close-ui",
    "end-binding",
    "cleanup",
    "diagnostic",
];

/// 채팅 item 종류(닫힌 목록, [`crate::chat::AgentChatItem::kind`]).
pub const CHAT_KINDS: [&str; 7] = [
    "user-prompt",
    "assistant-text",
    "tool-activity",
    "session-started",
    "turn-completed",
    "cli-error",
    "decode-stopped",
];

/// 앱 단계 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// 창 Undo 한 번.
    Undo,
    /// 변경 단위 되돌리기(XIX.6.8).
    RevertBatch,
    /// 세션 단위 되돌리기(XIX.6.9).
    RevertSession,
    /// 출처 표시 유지(XIX.6.10).
    Keep,
}

impl AppAction {
    fn parse(text: &str) -> Option<AppAction> {
        match text {
            "undo" => Some(AppAction::Undo),
            "revert-batch" => Some(AppAction::RevertBatch),
            "revert-session" => Some(AppAction::RevertSession),
            "keep" => Some(AppAction::Keep),
            _ => None,
        }
    }

    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            AppAction::Undo => "undo",
            AppAction::RevertBatch => "revert-batch",
            AppAction::RevertSession => "revert-session",
            AppAction::Keep => "keep",
        }
    }
}

/// 출력 대기 조건.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputWait {
    /// 디코드 실패 신호.
    DecodeFailed,
    /// CLI가 turn 끝을 보고한 채팅 item.
    TurnCompleted,
    /// stdout·stderr 닫힘.
    Closed,
}

/// 그 시점 확인(`None`은 확인하지 않음).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    /// 상태 코드([`crate::session::SessionState::code`]).
    pub state: Option<String>,
    /// lease 수.
    pub leases: Option<u32>,
    /// 보유 권한(정의 순서).
    pub permissions: Option<Vec<Permission>>,
    /// 대기 요청 수.
    pub pending_requests: Option<u32>,
    /// 대기 제안 수.
    pub pending_proposals: Option<u32>,
    /// 적용 batch 수.
    pub applied: Option<u32>,
    /// 지금까지 낸 효과 이름(처음 나온 순서, 중복 없음, 정확히 일치). 키가 없으면 확인하지 않습니다.
    pub effects: Option<Vec<String>>,
    /// 채팅 목록에 포함돼야 하는 item 종류.
    pub chat: Vec<String>,
    /// 지금까지 확정된 승인 결과 코드(순서대로).
    pub outcomes: Option<Vec<String>>,
    /// 앱 실행기 전용 확인.
    pub app: Vec<String>,
}

/// 단계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 시작 확인.
    Start,
    /// 대역 실행.
    Launch {
        /// 사용자 문장.
        prompt: String,
    },
    /// bridge 결합.
    BridgeBound,
    /// bridge 끊김.
    BridgeDisconnected,
    /// 시각 진행.
    Tick,
    /// 권한 승인.
    Grant(Permission),
    /// lease 획득.
    Lease(u64),
    /// 범위 승인 요청.
    OpenScope {
        /// 대상 수.
        targets: u32,
    },
    /// batch 승인 요청.
    OpenBatch {
        /// 제안.
        proposal: u64,
    },
    /// 사용자 응답.
    Answer {
        /// 연 순서(1부터).
        request: u32,
        /// 승인 여부.
        approve: bool,
    },
    /// batch 적용.
    Apply {
        /// batch 순번.
        batch: u32,
        /// history sequence.
        history: u64,
        /// 승인된 제안(없으면 바로 적용).
        proposal: Option<u64>,
    },
    /// 정상 완료.
    Complete,
    /// 사용자 중지.
    Stop,
    /// 사용자의 lease 해제.
    ReleaseLease,
    /// 출력 대기.
    AwaitOutput(OutputWait),
    /// 앱 실행기 전용 단계.
    App {
        /// 동작.
        action: AppAction,
        /// 대상 batch(변경 단위 되돌리기).
        batch: Option<u32>,
    },
    /// 확인.
    Check(Box<Check>),
}

/// 시각이 붙은 단계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// 단조 시각(ms).
    pub at_ms: u64,
    /// 단계.
    pub action: Action,
}

/// 시나리오 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    /// id.
    pub id: String,
    /// 설명.
    pub title: String,
    /// 테스트 대역 profile 종류(`claude`·`codex`).
    pub profile: String,
    /// 모드.
    pub mode: SessionMode,
    /// 테스트 대역 `[session]` 절 명령 줄(`outfile`은 fixture 디렉터리 기준 상대 경로).
    pub double: Vec<String>,
    /// 단계.
    pub steps: Vec<Step>,
}

/// load 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioError {
    /// 고정 코드.
    pub code: &'static str,
    /// 설명.
    pub detail: String,
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E2E 시나리오 오류({}): {}", self.code, self.detail)
    }
}

impl std::error::Error for ScenarioError {}

fn err(code: &'static str, detail: impl Into<String>) -> ScenarioError {
    ScenarioError {
        code,
        detail: detail.into(),
    }
}

fn keys(value: &JsonValue, allowed: &[&str], at: &str) -> Result<(), ScenarioError> {
    let pairs = value
        .as_object()
        .ok_or_else(|| err("not-object", format!("{at}는 object여야 합니다")))?;
    for (key, _) in pairs {
        if !allowed.contains(&key.as_str()) {
            return Err(err("unknown-key", format!("{at}의 모르는 key `{key}`")));
        }
    }
    Ok(())
}

fn string(value: &JsonValue, key: &str, at: &str) -> Result<String, ScenarioError> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| err("missing-key", format!("{at}.{key} 문자열이 없습니다")))
}

fn uint(value: &JsonValue, key: &str, at: &str) -> Result<u64, ScenarioError> {
    value
        .get(key)
        .and_then(JsonValue::as_i64)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| err("missing-key", format!("{at}.{key} 0 이상 정수가 없습니다")))
}

fn opt_uint(value: &JsonValue, key: &str, at: &str) -> Result<Option<u64>, ScenarioError> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(_) => uint(value, key, at).map(Some),
    }
}

fn small(n: u64, at: &str) -> Result<u32, ScenarioError> {
    u32::try_from(n).map_err(|_| err("range", format!("{at} 값이 너무 큽니다")))
}

fn strings(value: &JsonValue, key: &str, at: &str) -> Result<Option<Vec<String>>, ScenarioError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_array()
            .ok_or_else(|| err("type", format!("{at}.{key}는 배열이어야 합니다")))?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| err("type", format!("{at}.{key} 항목은 문자열이어야 합니다")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
    }
}

fn closed(list: Vec<String>, allowed: &[&str], at: &str) -> Result<Vec<String>, ScenarioError> {
    for item in &list {
        if !allowed.contains(&item.as_str()) {
            return Err(err("unknown-name", format!("{at}의 모르는 이름 `{item}`")));
        }
    }
    Ok(list)
}

fn mode(name: &str) -> Option<SessionMode> {
    SessionMode::ALL.into_iter().find(|m| m.name() == name)
}

fn parse_check(v: &JsonValue, at: &str) -> Result<Check, ScenarioError> {
    keys(
        v,
        &[
            "at",
            "do",
            "state",
            "leases",
            "permissions",
            "pending_requests",
            "pending_proposals",
            "applied",
            "effects",
            "chat",
            "outcomes",
            "app",
        ],
        at,
    )?;
    let count = |key: &str| -> Result<Option<u32>, ScenarioError> {
        opt_uint(v, key, at)?.map(|n| small(n, at)).transpose()
    };
    let permissions = strings(v, "permissions", at)?
        .map(|names| {
            names
                .iter()
                .map(|n| {
                    Permission::from_name(n)
                        .ok_or_else(|| err("unknown-name", format!("{at}의 모르는 권한 `{n}`")))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let state = match v.get("state") {
        None => None,
        Some(_) => Some(string(v, "state", at)?),
    };
    if let Some(state) = &state {
        let (head, tail) = state.split_once(':').unwrap_or((state, ""));
        let ok = match head {
            "starting" | "running" | "reconnecting" => tail.is_empty(),
            "ending" | "ended" => !tail.is_empty(),
            _ => false,
        };
        if !ok {
            return Err(err("unknown-name", format!("{at}의 모르는 상태 `{state}`")));
        }
    }
    let outcomes = strings(v, "outcomes", at)?;
    if let Some(list) = &outcomes {
        for code in list {
            let ok = code == "approved"
                || code.strip_prefix("denied:").is_some_and(|d| {
                    ["user", "expired", "window-closed", "session-ended"].contains(&d)
                });
            if !ok {
                return Err(err("unknown-name", format!("{at}의 모르는 결과 `{code}`")));
            }
        }
    }
    Ok(Check {
        state,
        leases: count("leases")?,
        permissions,
        pending_requests: count("pending_requests")?,
        pending_proposals: count("pending_proposals")?,
        applied: count("applied")?,
        effects: strings(v, "effects", at)?
            .map(|list| closed(list, &EFFECT_NAMES, at))
            .transpose()?,
        chat: closed(strings(v, "chat", at)?.unwrap_or_default(), &CHAT_KINDS, at)?,
        outcomes,
        app: closed(
            strings(v, "app", at)?.unwrap_or_default(),
            &APP_EXPECTATIONS,
            at,
        )?,
    })
}

fn parse_step(v: &JsonValue, at: &str, opened: &mut u32) -> Result<Step, ScenarioError> {
    let at_ms = uint(v, "at", at)?;
    let action_name = string(v, "do", at)?;
    let only = |extra: &[&str]| -> Result<(), ScenarioError> {
        let mut allowed = vec!["at", "do"];
        allowed.extend_from_slice(extra);
        keys(v, &allowed, at)
    };
    let action = match action_name.as_str() {
        "start" => {
            only(&[])?;
            Action::Start
        }
        "launch" => {
            only(&["prompt"])?;
            Action::Launch {
                prompt: string(v, "prompt", at)?,
            }
        }
        "bridge-bound" => {
            only(&[])?;
            Action::BridgeBound
        }
        "bridge-disconnected" => {
            only(&[])?;
            Action::BridgeDisconnected
        }
        "tick" => {
            only(&[])?;
            Action::Tick
        }
        "grant" => {
            only(&["permission"])?;
            let name = string(v, "permission", at)?;
            Action::Grant(
                Permission::from_name(&name)
                    .ok_or_else(|| err("unknown-name", format!("{at}의 모르는 권한 `{name}`")))?,
            )
        }
        "lease" => {
            only(&["lease"])?;
            Action::Lease(uint(v, "lease", at)?)
        }
        "open-scope" => {
            only(&["targets"])?;
            *opened += 1;
            Action::OpenScope {
                targets: small(uint(v, "targets", at)?, at)?,
            }
        }
        "open-batch" => {
            only(&["proposal"])?;
            *opened += 1;
            Action::OpenBatch {
                proposal: uint(v, "proposal", at)?,
            }
        }
        "answer" => {
            only(&["request", "answer"])?;
            let request = small(uint(v, "request", at)?, at)?;
            if request == 0 || request > *opened {
                return Err(err(
                    "unknown-request",
                    format!("{at}: 열린 적 없는 요청 {request}"),
                ));
            }
            let approve = match string(v, "answer", at)?.as_str() {
                "approve" => true,
                "deny" => false,
                other => return Err(err("unknown-name", format!("{at}의 모르는 응답 `{other}`"))),
            };
            Action::Answer { request, approve }
        }
        "apply" => {
            only(&["batch", "history", "proposal"])?;
            Action::Apply {
                batch: small(uint(v, "batch", at)?, at)?,
                history: uint(v, "history", at)?,
                proposal: opt_uint(v, "proposal", at)?,
            }
        }
        "complete" => {
            only(&[])?;
            Action::Complete
        }
        "stop" => {
            only(&[])?;
            Action::Stop
        }
        "release-lease" => {
            only(&[])?;
            Action::ReleaseLease
        }
        "await-output" => {
            only(&["until"])?;
            Action::AwaitOutput(match string(v, "until", at)?.as_str() {
                "decode-failed" => OutputWait::DecodeFailed,
                "turn-completed" => OutputWait::TurnCompleted,
                "closed" => OutputWait::Closed,
                other => return Err(err("unknown-name", format!("{at}의 모르는 대기 `{other}`"))),
            })
        }
        "app" => {
            only(&["action", "batch"])?;
            let name = string(v, "action", at)?;
            Action::App {
                action: AppAction::parse(&name).ok_or_else(|| {
                    err("unknown-name", format!("{at}의 모르는 앱 동작 `{name}`"))
                })?,
                batch: opt_uint(v, "batch", at)?
                    .map(|n| small(n, at))
                    .transpose()?,
            }
        }
        "check" => Action::Check(Box::new(parse_check(v, at)?)),
        other => return Err(err("unknown-step", format!("{at}의 모르는 단계 `{other}`"))),
    };
    Ok(Step { at_ms, action })
}

impl Scenario {
    /// fixture 원문을 읽습니다.
    pub fn parse(text: &str) -> Result<Scenario, ScenarioError> {
        let value =
            json::parse(text, JsonLimits::default()).map_err(|e| err("json", e.to_string()))?;
        keys(
            &value,
            &[
                "format", "id", "title", "profile", "mode", "double", "steps",
            ],
            "scenario",
        )?;
        if string(&value, "format", "scenario")? != SCENARIO_FORMAT {
            return Err(err("format", "형식 이름이 다릅니다"));
        }
        let id = string(&value, "id", "scenario")?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(err("id", "id 형식이 잘못되었습니다"));
        }
        let profile = string(&value, "profile", "scenario")?;
        if profile != "claude" && profile != "codex" {
            return Err(err("unknown-name", "profile은 claude·codex입니다"));
        }
        let mode_name = string(&value, "mode", "scenario")?;
        let mode = mode(&mode_name).ok_or_else(|| err("unknown-name", "모르는 모드"))?;
        let double = strings(&value, "double", "scenario")?
            .ok_or_else(|| err("missing-key", "double이 없습니다"))?;
        for line in &double {
            if line.contains('\n') || line.starts_with('[') {
                return Err(err("double", "double 줄은 [session] 절 명령 한 줄씩입니다"));
            }
        }
        let steps_value = value
            .get("steps")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| err("missing-key", "steps 배열이 없습니다"))?;
        let mut steps = Vec::new();
        let mut opened = 0u32;
        let mut last = 0u64;
        for (n, v) in steps_value.iter().enumerate() {
            let at = format!("steps[{n}]");
            let step = parse_step(v, &at, &mut opened)?;
            if step.at_ms < last {
                return Err(err("time", format!("{at}: 시각이 거꾸로 갑니다")));
            }
            last = step.at_ms;
            steps.push(step);
        }
        if steps.first().map(|s| &s.action) != Some(&Action::Start) {
            return Err(err("first-step", "첫 단계는 start여야 합니다"));
        }
        Ok(Scenario {
            id,
            title: string(&value, "title", "scenario")?,
            profile,
            mode,
            double,
            steps,
        })
    }

    /// 앱 실행기 전용 확인 이름 전체(나온 순서, 중복 제거).
    pub fn app_expectations(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for step in &self.steps {
            if let Action::Check(check) = &step.action {
                for name in &check.app {
                    if !out.contains(&name.as_str()) {
                        out.push(name);
                    }
                }
            }
        }
        out
    }

    /// 정규 표기(golden). 단계마다 한 줄입니다.
    pub fn to_golden(&self) -> String {
        let mut out = format!(
            "scenario {} profile={} mode={}\n",
            self.id,
            self.profile,
            self.mode.name()
        );
        for line in &self.double {
            out.push_str(&format!("  double {line}\n"));
        }
        for step in &self.steps {
            out.push_str(&format!(
                "  {:>6} {}\n",
                step.at_ms,
                action_golden(&step.action)
            ));
        }
        out
    }
}

fn action_golden(action: &Action) -> String {
    match action {
        Action::Start => "start".into(),
        Action::Launch { prompt } => format!("launch prompt={prompt:?}"),
        Action::BridgeBound => "bridge-bound".into(),
        Action::BridgeDisconnected => "bridge-disconnected".into(),
        Action::Tick => "tick".into(),
        Action::Grant(p) => format!("grant {}", p.name()),
        Action::Lease(l) => format!("lease {l}"),
        Action::OpenScope { targets } => format!("open-scope targets={targets}"),
        Action::OpenBatch { proposal } => format!("open-batch proposal={proposal}"),
        Action::Answer { request, approve } => format!(
            "answer request={request} {}",
            if *approve { "approve" } else { "deny" }
        ),
        Action::Apply {
            batch,
            history,
            proposal,
        } => format!(
            "apply batch={batch} history={history} proposal={}",
            proposal.map_or("-".to_string(), |p| p.to_string())
        ),
        Action::Complete => "complete".into(),
        Action::Stop => "stop".into(),
        Action::ReleaseLease => "release-lease".into(),
        Action::AwaitOutput(w) => format!(
            "await-output {}",
            match w {
                OutputWait::DecodeFailed => "decode-failed",
                OutputWait::TurnCompleted => "turn-completed",
                OutputWait::Closed => "closed",
            }
        ),
        Action::App { action, batch } => match batch {
            Some(b) => format!("app {} batch={b}", action.name()),
            None => format!("app {}", action.name()),
        },
        Action::Check(c) => {
            let mut parts = vec!["check".to_string()];
            if let Some(s) = &c.state {
                parts.push(format!("state={s}"));
            }
            let mut num = |name: &str, v: Option<u32>| {
                if let Some(v) = v {
                    parts.push(format!("{name}={v}"));
                }
            };
            num("leases", c.leases);
            num("pending_requests", c.pending_requests);
            num("pending_proposals", c.pending_proposals);
            num("applied", c.applied);
            if let Some(p) = &c.permissions {
                let names: Vec<&str> = p.iter().map(|p| p.name()).collect();
                parts.push(format!("permissions=[{}]", names.join(",")));
            }
            if let Some(e) = &c.effects {
                parts.push(format!("effects=[{}]", e.join(",")));
            }
            for (name, list) in [("chat", &c.chat), ("app", &c.app)] {
                if !list.is_empty() {
                    parts.push(format!("{name}=[{}]", list.join(",")));
                }
            }
            if let Some(o) = &c.outcomes {
                parts.push(format!("outcomes=[{}]", o.join(",")));
            }
            parts.join(" ")
        }
    }
}
