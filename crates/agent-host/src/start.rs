//! 내장 세션 시작 확인, 약관 고지 기록, 신뢰 철회 (XXIX.3.5, ADR-0005 §4.1·§4.5, 아키텍처 31.3, ADR-0101 결정 3).
//!
//! ## 시작 순서
//!
//! 1. [`prepare_start`]: 프로젝트 신뢰 capability(XVI.7.1 `ProjectCapability::Mcp`)가 허용이 아니면 시작하지
//!    않습니다. 판정 자체는 앱 runtime이 `stellar_project::capability::capability_level`로 하고 결과만
//!    [`McpCapability`]로 넘깁니다(판정 이중 경로 없음). 비활성 profile도 시작하지 않습니다. 통과하면 사용자가
//!    대상 프로젝트, 실행할 CLI, 모드를 확인할 [`StartConfirmation`]을 만듭니다. 약관 고지가 필요하면
//!    `notice_required`가 참입니다.
//! 2. 사용자가 고지를 확인하면 앱 UI가 [`TermsNoticeStore::acknowledge`]를 부릅니다.
//! 3. [`confirm_start`]: 사용자가 시작을 확인한 순간 capability와 고지를 **다시** 확인합니다(그 사이 신뢰
//!    철회·profile 검토 결과 변경). 통과하면 [`AgentSession`]을 만들고 이 확인을 첫 연결 승인
//!    [`FirstConnectionApproval`]로 기록합니다(§31.3). 이 기록은 세션에 결합된 bridge의 첫 연결 승인이며
//!    Read 외 권한을 주지 않습니다(XIX.3.3·XIX.3.4).
//! 4. 그 뒤 실행 전 확인(XXIX.2.7 [`crate::preflight`])과 실행기가 이어집니다.
//!
//! ## 약관 고지 기록
//!
//! 각 CLI를 처음 쓰기 전과 profile 약관 검토 결과(검토 일자·결과)가 바뀐 뒤 고지 확인을 요구합니다. 기록은
//! 프로젝트 폴더와 Git 밖, 앱이 정한 OS 사용자 데이터 디렉터리 아래 `agent/terms-notices`에 둡니다.
//! 줄마다 `<cli 종류> <약관 검토 일자> <검토 결과>`뿐이며 계정·경로·내용은 없습니다(XVI.8). 디렉터리는
//! 0700, 파일은 0600이고 symlink를 따라가지 않습니다. 형식이 틀린 파일은 덮어쓰지 않고 오류로
//! 닫습니다(fail closed: 시작 0).
//!
//! ## 신뢰 철회
//!
//! [`on_trust_change`]: 세션 중 capability가 허용이 아니게 되면 즉시 `TrustRevoked`로 XIX.6.4 종료 절차를
//! 시작합니다(자동 rollback 없음, lease·권한·대기 요청 폐기).

use std::fmt;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use crate::approval::{ProjectKey, WindowSessionId};
use crate::profile::LaunchProfile;
use crate::session::{
    AgentSession, AgentSessionId, SessionEffect, SessionEvent, SessionMode, TransitionError,
    UserStartRequest,
};

/// 고지 기록 디렉터리(데이터 디렉터리 아래).
pub const NOTICE_DIR: &str = "agent";

/// 고지 기록 파일 이름.
pub const NOTICE_FILE: &str = "terms-notices";

/// 고지 기록 첫 줄.
pub const NOTICE_HEADER: &str = "stellar-agent-terms-notice 1";

/// 고지 기록 파일 최대 byte 수.
pub const MAX_NOTICE_FILE_BYTES: u64 = 64 * 1024;

/// 프로젝트 신뢰 capability(XVI.7.1 `Mcp`) 판정 결과. 앱 runtime이 capability 표로 만듭니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCapability {
    /// 허용(신뢰된 프로젝트).
    Allowed,
    /// 금지(신뢰되지 않은 프로젝트 또는 판정 불가).
    Denied,
}

/// 시작 거부 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartRefusal {
    /// 프로젝트가 신뢰되지 않음.
    Untrusted,
    /// profile이 활성 목록에 없음.
    ProfileDisabled,
    /// 확인 데이터와 실행할 profile이 다름.
    ProfileMismatch,
    /// 확인 데이터의 대상 프로젝트와 신뢰를 다시 확인한 프로젝트가 다름.
    ProjectMismatch,
    /// 약관 고지를 확인하지 않음(또는 검토 결과가 바뀐 뒤 다시 확인하지 않음).
    NoticeNotAcknowledged,
}

impl StartRefusal {
    /// 고정 코드.
    pub fn code(&self) -> &'static str {
        match self {
            StartRefusal::Untrusted => "untrusted-project",
            StartRefusal::ProfileDisabled => "profile-disabled",
            StartRefusal::ProfileMismatch => "profile-mismatch",
            StartRefusal::ProjectMismatch => "project-mismatch",
            StartRefusal::NoticeNotAcknowledged => "notice-not-acknowledged",
        }
    }
}

/// 사용자가 확인할 시작 데이터.
#[derive(Clone, PartialEq, Eq)]
pub struct StartConfirmation {
    /// 대상 프로젝트.
    pub project: ProjectKey,
    /// 세션을 묶을 창.
    pub window: WindowSessionId,
    /// 실행할 launch profile id.
    pub profile_id: String,
    /// CLI 종류 id.
    pub cli: String,
    /// 사용자에게 보여 주는 CLI 이름(일반 텍스트).
    pub cli_display_name: String,
    /// 사용자가 고른 모드.
    pub mode: SessionMode,
    /// 약관 고지 확인이 필요한지.
    pub notice_required: bool,
}

impl fmt::Debug for StartConfirmation {
    /// 경로는 쓰지 않습니다.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartConfirmation")
            .field("profile_id", &self.profile_id)
            .field("mode", &self.mode)
            .field("notice_required", &self.notice_required)
            .finish()
    }
}

/// 시작 확인을 첫 연결 승인으로 남긴 기록(§31.3, XIX.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstConnectionApproval {
    /// 대상 프로젝트.
    pub project: ProjectKey,
    /// 세션이 묶인 창.
    pub window: WindowSessionId,
    /// 세션.
    pub session: AgentSessionId,
    /// 실행한 profile id.
    pub profile_id: String,
    /// 확인한 모드.
    pub mode: SessionMode,
    /// 확인 시각(ms).
    pub approved_at_ms: u64,
}

/// 시작된 세션.
#[derive(Debug)]
pub struct StartedSession {
    /// 세션(상태 `Starting`, 권한·lease 0).
    pub session: AgentSession,
    /// 첫 연결 승인 기록.
    pub connection: FirstConnectionApproval,
}

/// 시작 확인 데이터를 만듭니다.
pub fn prepare_start(
    project: ProjectKey,
    window: WindowSessionId,
    mcp: McpCapability,
    profile: &LaunchProfile,
    mode: SessionMode,
    notices: &TermsNoticeStore,
) -> Result<StartConfirmation, StartRefusal> {
    if mcp != McpCapability::Allowed {
        return Err(StartRefusal::Untrusted);
    }
    if !profile.enabled {
        return Err(StartRefusal::ProfileDisabled);
    }
    Ok(StartConfirmation {
        project,
        window,
        profile_id: profile.id.clone(),
        cli: profile.cli.clone(),
        cli_display_name: profile.display_name.clone(),
        mode,
        notice_required: notices.needs_notice(profile),
    })
}

/// 사용자가 시작을 확인했습니다. capability·profile·고지를 다시 확인하고 세션을 만듭니다.
pub fn confirm_start(
    confirmation: StartConfirmation,
    mcp: McpCapability,
    profile: &LaunchProfile,
    notices: &TermsNoticeStore,
    session: AgentSessionId,
    now_ms: u64,
    reconnect_grace_ms: u64,
) -> Result<StartedSession, StartRefusal> {
    if mcp != McpCapability::Allowed {
        return Err(StartRefusal::Untrusted);
    }
    if !profile.enabled {
        return Err(StartRefusal::ProfileDisabled);
    }
    if profile.id != confirmation.profile_id || profile.cli != confirmation.cli {
        return Err(StartRefusal::ProfileMismatch);
    }
    if notices.needs_notice(profile) {
        return Err(StartRefusal::NoticeNotAcknowledged);
    }
    let agent = AgentSession::start_by_user(UserStartRequest {
        session,
        mode: confirmation.mode,
        now_ms,
        reconnect_grace_ms,
    });
    Ok(StartedSession {
        session: agent,
        connection: FirstConnectionApproval {
            project: confirmation.project,
            window: confirmation.window,
            session,
            profile_id: confirmation.profile_id,
            mode: confirmation.mode,
            approved_at_ms: now_ms,
        },
    })
}

/// 세션 중 신뢰 capability가 바뀌었습니다. 허용이 아니면 즉시 XIX.6.4 종료 절차를 시작합니다.
pub fn on_trust_change(
    session: &mut AgentSession,
    mcp: McpCapability,
    now_ms: u64,
) -> Result<Vec<SessionEffect>, TransitionError> {
    match mcp {
        McpCapability::Allowed => Ok(Vec::new()),
        McpCapability::Denied => session.apply(SessionEvent::TrustRevoked, now_ms),
    }
}

/// 고지 기록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeError {
    /// 입출력 실패(경로는 담지 않음).
    Io(std::io::ErrorKind),
    /// 기록 파일 형식이 틀림(덮어쓰지 않음).
    Corrupt(&'static str),
    /// 기록 위치가 symlink임.
    Symlink,
}

impl fmt::Display for NoticeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoticeError::Io(kind) => write!(f, "약관 고지 기록 입출력 실패: {kind:?}"),
            NoticeError::Corrupt(why) => write!(f, "약관 고지 기록 손상: {why}"),
            NoticeError::Symlink => f.write_str("약관 고지 기록 위치가 symlink입니다"),
        }
    }
}

impl std::error::Error for NoticeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoticeEntry {
    cli: String,
    terms_date: String,
    terms_result: String,
}

/// OS 사용자 데이터에 두는 약관 고지 확인 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermsNoticeStore {
    dir: PathBuf,
    entries: Vec<NoticeEntry>,
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

fn io_err(e: std::io::Error) -> NoticeError {
    NoticeError::Io(e.kind())
}

fn reject_symlink(path: &Path) -> Result<bool, NoticeError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(NoticeError::Symlink),
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_err(e)),
    }
}

impl TermsNoticeStore {
    /// `data_dir`(앱이 정한 OS 사용자 데이터 디렉터리)의 기록을 엽니다. 파일이 없으면 빈 기록이며 파일을
    /// 만들지 않습니다.
    pub fn open(data_dir: &Path) -> Result<TermsNoticeStore, NoticeError> {
        let dir = data_dir.join(NOTICE_DIR);
        let file = dir.join(NOTICE_FILE);
        let mut entries = Vec::new();
        if reject_symlink(&dir)? && reject_symlink(&file)? {
            let meta = std::fs::metadata(&file).map_err(io_err)?;
            if !meta.is_file() {
                return Err(NoticeError::Corrupt("일반 파일이 아님"));
            }
            if meta.len() > MAX_NOTICE_FILE_BYTES {
                return Err(NoticeError::Corrupt("크기 상한 초과"));
            }
            let text = std::fs::read_to_string(&file).map_err(io_err)?;
            let mut lines = text.lines();
            if lines.next() != Some(NOTICE_HEADER) {
                return Err(NoticeError::Corrupt("첫 줄"));
            }
            for line in lines {
                let parts: Vec<&str> = line.split(' ').collect();
                let [cli, date, result] = parts.as_slice() else {
                    return Err(NoticeError::Corrupt("항목 형식"));
                };
                if !is_token(cli) || !is_token(date) || !is_token(result) {
                    return Err(NoticeError::Corrupt("항목 문자"));
                }
                if entries.iter().any(|e: &NoticeEntry| e.cli == *cli) {
                    return Err(NoticeError::Corrupt("중복 CLI"));
                }
                entries.push(NoticeEntry {
                    cli: cli.to_string(),
                    terms_date: date.to_string(),
                    terms_result: result.to_string(),
                });
            }
        }
        Ok(TermsNoticeStore { dir, entries })
    }

    /// 이 profile로 시작하기 전에 고지 확인이 필요한지(처음 쓰는 CLI, 또는 검토 일자·결과가 바뀜).
    pub fn needs_notice(&self, profile: &LaunchProfile) -> bool {
        !self.entries.iter().any(|e| {
            e.cli == profile.cli
                && e.terms_date == profile.review.terms_date
                && e.terms_result == profile.review.terms_result.name()
        })
    }

    /// 사용자가 고지를 확인했습니다(앱 사용자 UI만 부름). 그 CLI의 기록을 지금 profile 검토 결과로 바꿔
    /// 원자적으로 씁니다.
    pub fn acknowledge(&mut self, profile: &LaunchProfile) -> Result<(), NoticeError> {
        let entry = NoticeEntry {
            cli: profile.cli.clone(),
            terms_date: profile.review.terms_date.clone(),
            terms_result: profile.review.terms_result.name().to_string(),
        };
        if !is_token(&entry.cli) || !is_token(&entry.terms_date) {
            return Err(NoticeError::Corrupt("profile 값 형식"));
        }
        let mut entries = self.entries.clone();
        entries.retain(|e| e.cli != entry.cli);
        entries.push(entry);
        entries.sort_by(|a, b| a.cli.cmp(&b.cli));
        let mut text = String::from(NOTICE_HEADER);
        text.push('\n');
        for e in &entries {
            text.push_str(&format!("{} {} {}\n", e.cli, e.terms_date, e.terms_result));
        }
        if !reject_symlink(&self.dir)? {
            std::fs::create_dir_all(&self.dir).map_err(io_err)?;
        }
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
            .map_err(io_err)?;
        let file = self.dir.join(NOTICE_FILE);
        reject_symlink(&file)?;
        let tmp = self
            .dir
            .join(format!(".{NOTICE_FILE}.{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(io_err)?;
            out.write_all(text.as_bytes()).map_err(io_err)?;
            out.sync_all().map_err(io_err)?;
        }
        std::fs::rename(&tmp, &file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io_err(e)
        })?;
        self.entries = entries;
        Ok(())
    }

    /// 기록 파일 경로.
    pub fn file_path(&self) -> PathBuf {
        self.dir.join(NOTICE_FILE)
    }
}
