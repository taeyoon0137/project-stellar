//! TaskStatus 데이터 모델 (I.2.1).
//!
//! 데이터 모델만 정의합니다. YAML 파싱과 파일 I/O는 별도 모듈에서 처리하며,
//! 이 모듈은 표준 라이브러리 외 어떤 것도 import 하지 않습니다.

/// 작업 상태. `planning/task-status.yaml`의 `status` 값과 1:1 대응합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    NotStarted,
    InProgress,
    ReportedDone,
    Verified,
    Blocked,
    Cancelled,
}

impl TaskState {
    /// 허용되는 모든 상태.
    pub const ALL: [TaskState; 6] = [
        TaskState::NotStarted,
        TaskState::InProgress,
        TaskState::ReportedDone,
        TaskState::Verified,
        TaskState::Blocked,
        TaskState::Cancelled,
    ];

    /// 직렬화 토큰.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::NotStarted => "not_started",
            TaskState::InProgress => "in_progress",
            TaskState::ReportedDone => "reported_done",
            TaskState::Verified => "verified",
            TaskState::Blocked => "blocked",
            TaskState::Cancelled => "cancelled",
        }
    }

    /// 토큰을 상태로 변환합니다. 알 수 없는 토큰은 `None`.
    pub fn from_token(token: &str) -> Option<TaskState> {
        TaskState::ALL
            .into_iter()
            .find(|state| state.as_str() == token)
    }
}

/// 검증 실행 환경 코드. `config/verification-environments.yaml`과 1:1 대응합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationEnvironment {
    DevMac,
    CiMac,
    CiWin,
    LabWin,
    RefMac,
    RefWin,
}

impl VerificationEnvironment {
    /// 허용되는 모든 환경 코드.
    pub const ALL: [VerificationEnvironment; 6] = [
        VerificationEnvironment::DevMac,
        VerificationEnvironment::CiMac,
        VerificationEnvironment::CiWin,
        VerificationEnvironment::LabWin,
        VerificationEnvironment::RefMac,
        VerificationEnvironment::RefWin,
    ];

    /// 직렬화 토큰(계약 코드).
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationEnvironment::DevMac => "DEV-MAC",
            VerificationEnvironment::CiMac => "CI-MAC",
            VerificationEnvironment::CiWin => "CI-WIN",
            VerificationEnvironment::LabWin => "LAB-WIN",
            VerificationEnvironment::RefMac => "REF-MAC",
            VerificationEnvironment::RefWin => "REF-WIN",
        }
    }

    /// 토큰을 환경 코드로 변환합니다. 알 수 없는 토큰은 `None`.
    pub fn from_token(token: &str) -> Option<VerificationEnvironment> {
        VerificationEnvironment::ALL
            .into_iter()
            .find(|env| env.as_str() == token)
    }
}

/// 한 환경에서의 검증 증거.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationEvidence {
    /// 증거가 속한 환경.
    pub environment: VerificationEnvironment,
    /// CI check URL, runner ID 또는 artifact locator.
    pub locator: String,
    /// artifact의 SHA-256(있으면).
    pub artifact_sha256: Option<String>,
}

impl VerificationEvidence {
    pub fn new(environment: VerificationEnvironment, locator: String) -> Self {
        VerificationEvidence {
            environment,
            locator,
            artifact_sha256: None,
        }
    }
}

/// 단일 작업 레코드의 raw 필드 구조.
///
/// 필드 형식·상태 규칙·그래프 검증은 별도 validator 모듈에서 수행합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatusRecord {
    /// WBS ID 문자열(형식 검증은 `wbs_id` 모듈에서).
    pub id: String,
    /// 작업 제목.
    pub title: String,
    /// 상태.
    pub status: TaskState,
    /// 담당자(없을 수 있음).
    pub owner: Option<String>,
    /// 선행 작업 WBS ID 목록.
    pub depends_on: Vec<String>,
    /// 필수 검증 환경 코드 목록.
    pub required_environments: Vec<VerificationEnvironment>,
    /// 환경별 검증 증거 목록.
    pub evidence: Vec<VerificationEvidence>,
    /// 최종 커밋 SHA(없을 수 있음).
    pub commit: Option<String>,
    /// 보고서 상대 경로(없을 수 있음).
    pub report: Option<String>,
    /// 검증 완료 시각(없을 수 있음).
    pub verified_at: Option<String>,
}

impl TaskStatusRecord {
    /// 필수 핵심 필드로 레코드를 생성하고 나머지는 빈 기본값으로 둡니다.
    pub fn new(id: String, title: String, status: TaskState) -> Self {
        TaskStatusRecord {
            id,
            title,
            status,
            owner: None,
            depends_on: Vec::new(),
            required_environments: Vec::new(),
            evidence: Vec::new(),
            commit: None,
            report: None,
            verified_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_token_round_trip() {
        for state in TaskState::ALL {
            assert_eq!(TaskState::from_token(state.as_str()), Some(state));
        }
        assert_eq!(TaskState::from_token("implemented"), None);
        assert_eq!(TaskState::from_token(""), None);
    }

    #[test]
    fn verification_environment_token_round_trip() {
        for env in VerificationEnvironment::ALL {
            assert_eq!(VerificationEnvironment::from_token(env.as_str()), Some(env));
        }
        assert_eq!(VerificationEnvironment::from_token("dev-mac"), None);
        assert_eq!(VerificationEnvironment::from_token("CI"), None);
    }

    #[test]
    fn record_new_sets_empty_defaults() {
        let record = TaskStatusRecord::new(
            "I.1.1".to_string(),
            "저장소 기준선 검증".to_string(),
            TaskState::Verified,
        );
        assert_eq!(record.id, "I.1.1");
        assert_eq!(record.status, TaskState::Verified);
        assert!(record.owner.is_none());
        assert!(record.depends_on.is_empty());
        assert!(record.required_environments.is_empty());
        assert!(record.evidence.is_empty());
        assert!(record.commit.is_none());
        assert!(record.report.is_none());
        assert!(record.verified_at.is_none());
    }

    #[test]
    fn record_accepts_full_fields() {
        let mut record = TaskStatusRecord::new(
            "I.1.10".to_string(),
            "Bootstrap CI".to_string(),
            TaskState::ReportedDone,
        );
        record.owner = Some("agent".to_string());
        record.depends_on = vec!["I.1.9".to_string()];
        record.required_environments = vec![
            VerificationEnvironment::CiMac,
            VerificationEnvironment::CiWin,
        ];
        record.evidence = vec![VerificationEvidence::new(
            VerificationEnvironment::CiMac,
            "https://ci.example/run/1".to_string(),
        )];
        record.report = Some("docs/task-reports/I.1.10.md".to_string());

        assert_eq!(record.required_environments.len(), 2);
        assert_eq!(
            record.evidence[0].environment,
            VerificationEnvironment::CiMac
        );
        assert_eq!(record.depends_on, vec!["I.1.9".to_string()]);
    }
}
