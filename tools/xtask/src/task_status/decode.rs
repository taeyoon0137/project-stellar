//! TaskStatus decoder (I.2.5).
//!
//! 제한 YAML 값 트리(`yaml::restricted_loader::YamlValue`)를 `TaskStatusRecord`로 변환합니다.
//! top-level은 `tasks` sequence 하나만 허용하고, unknown key·duplicate key·잘못된 형식을 거부합니다.
//! 상태 규칙과 선행 graph는 검사하지 않습니다(각각 I.2.7, I.2.8).

use crate::task_status::model::{
    TaskState, TaskStatusRecord, VerificationEnvironment, VerificationEvidence,
};
use crate::yaml::restricted_loader::YamlValue;

/// decode 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub code: &'static str,
    pub message: String,
}

impl DecodeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        DecodeError {
            code,
            message: message.into(),
        }
    }
}

const TASK_KEYS: [&str; 10] = [
    "id",
    "title",
    "status",
    "owner",
    "depends_on",
    "required_environments",
    "evidence",
    "commit",
    "report",
    "verified_at",
];
const EVIDENCE_KEYS: [&str; 3] = ["environment", "locator", "artifact_sha256"];

/// top-level 값을 레코드 목록으로 디코드합니다.
pub fn decode_collection(value: &YamlValue) -> Result<Vec<TaskStatusRecord>, DecodeError> {
    let YamlValue::Mapping(pairs) = value else {
        return Err(DecodeError::new(
            "top-level-not-mapping",
            "최상위는 mapping이어야 합니다",
        ));
    };
    let tasks = single_key(pairs, "tasks")?;
    let YamlValue::Sequence(items) = tasks else {
        return Err(DecodeError::new(
            "tasks-not-sequence",
            "`tasks`는 sequence여야 합니다",
        ));
    };
    let mut records = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        records.push(decode_record(item, index)?);
    }
    Ok(records)
}

/// mapping에서 정확히 하나의 지정 key만 허용하고 그 값을 반환합니다.
fn single_key<'a>(
    pairs: &'a [(YamlValue, YamlValue)],
    key: &str,
) -> Result<&'a YamlValue, DecodeError> {
    let mut found: Option<&YamlValue> = None;
    for (k, v) in pairs {
        let name = scalar(k)
            .ok_or_else(|| DecodeError::new("type-mismatch", "최상위 key는 scalar여야 합니다"))?;
        if name != key {
            return Err(DecodeError::new(
                "unknown-top-key",
                format!("허용하지 않는 최상위 key: {name}"),
            ));
        }
        if found.is_some() {
            return Err(DecodeError::new(
                "duplicate-key",
                format!("중복 최상위 key: {name}"),
            ));
        }
        found = Some(v);
    }
    found.ok_or_else(|| DecodeError::new("missing-tasks", "`tasks` key가 없습니다"))
}

fn decode_record(value: &YamlValue, index: usize) -> Result<TaskStatusRecord, DecodeError> {
    let YamlValue::Mapping(pairs) = value else {
        return Err(DecodeError::new(
            "task-not-mapping",
            format!("tasks[{index}]는 mapping이어야 합니다"),
        ));
    };

    let mut seen: Vec<&str> = Vec::new();
    let mut id = String::new();
    let mut title = String::new();
    let mut status: Option<TaskState> = None;
    let mut owner: Option<String> = None;
    let mut depends_on: Vec<String> = Vec::new();
    let mut required_environments: Vec<VerificationEnvironment> = Vec::new();
    let mut evidence: Vec<VerificationEvidence> = Vec::new();
    let mut commit: Option<String> = None;
    let mut report: Option<String> = None;
    let mut verified_at: Option<String> = None;

    for (key_value, field_value) in pairs {
        let key = scalar(key_value).ok_or_else(|| {
            DecodeError::new(
                "type-mismatch",
                format!("tasks[{index}] key는 scalar여야 합니다"),
            )
        })?;
        if !TASK_KEYS.contains(&key) {
            return Err(DecodeError::new(
                "unknown-key",
                format!("tasks[{index}] 알 수 없는 key: {key}"),
            ));
        }
        if seen.contains(&key) {
            return Err(DecodeError::new(
                "duplicate-key",
                format!("tasks[{index}] 중복 key: {key}"),
            ));
        }
        seen.push(key);

        match key {
            "id" => id = required_scalar(field_value, index, "id")?,
            "title" => title = required_scalar(field_value, index, "title")?,
            "status" => {
                let token = required_scalar(field_value, index, "status")?;
                status = Some(TaskState::from_token(&token).ok_or_else(|| {
                    DecodeError::new(
                        "unknown-status",
                        format!("tasks[{index}] 알 수 없는 status: {token}"),
                    )
                })?);
            }
            "owner" => owner = Some(required_scalar(field_value, index, "owner")?),
            "depends_on" => depends_on = scalar_sequence(field_value, index, "depends_on")?,
            "required_environments" => {
                let tokens = scalar_sequence(field_value, index, "required_environments")?;
                for token in tokens {
                    let env = VerificationEnvironment::from_token(&token).ok_or_else(|| {
                        DecodeError::new(
                            "unknown-environment",
                            format!("tasks[{index}] 알 수 없는 환경 코드: {token}"),
                        )
                    })?;
                    required_environments.push(env);
                }
            }
            "evidence" => evidence = decode_evidence_list(field_value, index)?,
            "commit" => commit = Some(required_scalar(field_value, index, "commit")?),
            "report" => report = Some(required_scalar(field_value, index, "report")?),
            "verified_at" => {
                verified_at = Some(required_scalar(field_value, index, "verified_at")?)
            }
            _ => unreachable!("key는 TASK_KEYS로 이미 검증됨"),
        }
    }

    let status = status.ok_or_else(|| {
        DecodeError::new(
            "missing-status",
            format!("tasks[{index}] status가 없습니다"),
        )
    })?;

    let mut record = TaskStatusRecord::new(id, title, status);
    record.owner = owner;
    record.depends_on = depends_on;
    record.required_environments = required_environments;
    record.evidence = evidence;
    record.commit = commit;
    record.report = report;
    record.verified_at = verified_at;
    Ok(record)
}

fn decode_evidence_list(
    value: &YamlValue,
    index: usize,
) -> Result<Vec<VerificationEvidence>, DecodeError> {
    let YamlValue::Sequence(items) = value else {
        return Err(DecodeError::new(
            "type-mismatch",
            format!("tasks[{index}] evidence는 sequence여야 합니다"),
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let YamlValue::Mapping(pairs) = item else {
            return Err(DecodeError::new(
                "type-mismatch",
                format!("tasks[{index}] evidence 항목은 mapping이어야 합니다"),
            ));
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut environment: Option<VerificationEnvironment> = None;
        let mut locator = String::new();
        let mut artifact_sha256: Option<String> = None;
        for (key_value, field_value) in pairs {
            let key = scalar(key_value).ok_or_else(|| {
                DecodeError::new(
                    "type-mismatch",
                    format!("tasks[{index}] evidence key는 scalar"),
                )
            })?;
            if !EVIDENCE_KEYS.contains(&key) {
                return Err(DecodeError::new(
                    "unknown-key",
                    format!("tasks[{index}] evidence 알 수 없는 key: {key}"),
                ));
            }
            if seen.contains(&key) {
                return Err(DecodeError::new(
                    "duplicate-key",
                    format!("tasks[{index}] evidence 중복 key: {key}"),
                ));
            }
            seen.push(key);
            match key {
                "environment" => {
                    let token = required_scalar(field_value, index, "evidence.environment")?;
                    environment =
                        Some(VerificationEnvironment::from_token(&token).ok_or_else(|| {
                            DecodeError::new(
                                "unknown-environment",
                                format!("tasks[{index}] evidence 알 수 없는 환경 코드: {token}"),
                            )
                        })?);
                }
                "locator" => locator = required_scalar(field_value, index, "evidence.locator")?,
                "artifact_sha256" => {
                    artifact_sha256 = Some(required_scalar(
                        field_value,
                        index,
                        "evidence.artifact_sha256",
                    )?);
                }
                _ => unreachable!("evidence key는 이미 검증됨"),
            }
        }
        let environment = environment.ok_or_else(|| {
            DecodeError::new(
                "missing-field",
                format!("tasks[{index}] evidence.environment가 없습니다"),
            )
        })?;
        out.push(VerificationEvidence {
            environment,
            locator,
            artifact_sha256,
        });
    }
    Ok(out)
}

fn scalar(value: &YamlValue) -> Option<&str> {
    match value {
        YamlValue::Scalar(text) => Some(text),
        _ => None,
    }
}

fn required_scalar(value: &YamlValue, index: usize, field: &str) -> Result<String, DecodeError> {
    scalar(value).map(str::to_string).ok_or_else(|| {
        DecodeError::new(
            "type-mismatch",
            format!("tasks[{index}] {field}는 scalar여야 합니다"),
        )
    })
}

fn scalar_sequence(
    value: &YamlValue,
    index: usize,
    field: &str,
) -> Result<Vec<String>, DecodeError> {
    let YamlValue::Sequence(items) = value else {
        return Err(DecodeError::new(
            "type-mismatch",
            format!("tasks[{index}] {field}는 sequence여야 합니다"),
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(scalar(item).map(str::to_string).ok_or_else(|| {
            DecodeError::new(
                "type-mismatch",
                format!("tasks[{index}] {field} 항목은 scalar여야 합니다"),
            )
        })?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::restricted_loader::load_restricted;

    fn decode(input: &str) -> Result<Vec<TaskStatusRecord>, DecodeError> {
        decode_collection(&load_restricted(input).expect("valid yaml"))
    }

    #[test]
    fn decodes_full_record() {
        let records = decode(
            "tasks:\n  - id: I.1.10\n    title: Bootstrap CI\n    status: reported_done\n    owner: agent\n    depends_on: [I.1.9]\n    required_environments: [CI-MAC, CI-WIN]\n    evidence:\n      - environment: CI-MAC\n        locator: https://ci/1\n    report: docs/task-reports/I.1.10.md\n",
        )
        .unwrap();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.id, "I.1.10");
        assert_eq!(r.status, TaskState::ReportedDone);
        assert_eq!(r.owner.as_deref(), Some("agent"));
        assert_eq!(r.depends_on, vec!["I.1.9".to_string()]);
        assert_eq!(r.required_environments.len(), 2);
        assert_eq!(r.evidence.len(), 1);
        assert_eq!(r.evidence[0].environment, VerificationEnvironment::CiMac);
        assert_eq!(r.report.as_deref(), Some("docs/task-reports/I.1.10.md"));
    }

    #[test]
    fn rejects_top_level_not_mapping() {
        assert_eq!(
            decode("- a\n- b\n").unwrap_err().code,
            "top-level-not-mapping"
        );
    }

    #[test]
    fn rejects_unknown_top_key() {
        assert_eq!(
            decode("nope:\n  - 1\n").unwrap_err().code,
            "unknown-top-key"
        );
    }

    #[test]
    fn rejects_tasks_not_sequence() {
        assert_eq!(decode("tasks: 1\n").unwrap_err().code, "tasks-not-sequence");
    }

    #[test]
    fn rejects_task_not_mapping() {
        assert_eq!(
            decode("tasks:\n  - 1\n").unwrap_err().code,
            "task-not-mapping"
        );
    }

    #[test]
    fn rejects_unknown_key() {
        let e = decode("tasks:\n  - id: I.1.1\n    status: verified\n    bogus: x\n").unwrap_err();
        assert_eq!(e.code, "unknown-key");
    }

    #[test]
    fn rejects_duplicate_key() {
        let e = decode("tasks:\n  - id: I.1.1\n    id: I.1.2\n    status: verified\n").unwrap_err();
        assert_eq!(e.code, "duplicate-key");
    }

    #[test]
    fn rejects_type_mismatch() {
        let e =
            decode("tasks:\n  - id: I.1.1\n    title: [a, b]\n    status: verified\n").unwrap_err();
        assert_eq!(e.code, "type-mismatch");
        let e2 =
            decode("tasks:\n  - id: I.1.1\n    status: verified\n    depends_on: x\n").unwrap_err();
        assert_eq!(e2.code, "type-mismatch");
    }

    #[test]
    fn rejects_unknown_status_and_environment() {
        let e = decode("tasks:\n  - id: I.1.1\n    status: implemented\n").unwrap_err();
        assert_eq!(e.code, "unknown-status");
        let e2 = decode(
            "tasks:\n  - id: I.1.1\n    status: verified\n    required_environments: [DEV-WIN]\n",
        )
        .unwrap_err();
        assert_eq!(e2.code, "unknown-environment");
    }
}
