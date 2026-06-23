//! 레코드 집합 검증 (I.2.7).
//!
//! task ID 중복과 같은 ID의 title mismatch를 검사합니다.
//! 허용 상태(not_started, in_progress, reported_done, verified, blocked, cancelled)는
//! `TaskState` 타입과 decoder(I.2.5)에서 이미 강제되므로 여기서는 ID·title만 검사합니다.
//! 선행 graph(I.2.8)와 증거 규칙(I.2.9)은 다루지 않습니다.

use crate::task_status::model::TaskStatusRecord;

/// 집합 단위 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionError {
    pub code: &'static str,
    pub message: String,
}

impl CollectionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        CollectionError {
            code,
            message: message.into(),
        }
    }
}

/// 레코드 집합을 검증하고 발견된 모든 오류를 반환합니다(빈 Vec = 통과).
pub fn validate_collection(records: &[TaskStatusRecord]) -> Vec<CollectionError> {
    let mut errors = Vec::new();
    // (id, first_title) 누적. 같은 id 재등장 시 중복, title이 다르면 mismatch.
    let mut seen: Vec<(&str, &str)> = Vec::new();
    let mut reported_dup: Vec<&str> = Vec::new();

    for record in records {
        if let Some((_, first_title)) = seen.iter().find(|(id, _)| *id == record.id) {
            if !reported_dup.contains(&record.id.as_str()) {
                errors.push(CollectionError::new(
                    "duplicate-id",
                    format!("중복 task ID: {}", record.id),
                ));
                reported_dup.push(record.id.as_str());
            }
            if *first_title != record.title {
                errors.push(CollectionError::new(
                    "title-mismatch",
                    format!("같은 ID {}의 title 불일치", record.id),
                ));
            }
        } else {
            seen.push((record.id.as_str(), record.title.as_str()));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::decode::decode_collection;
    use crate::task_status::model::TaskState;
    use crate::yaml::restricted_loader::load_restricted;

    fn record(id: &str, title: &str) -> TaskStatusRecord {
        TaskStatusRecord::new(id.to_string(), title.to_string(), TaskState::NotStarted)
    }

    #[test]
    fn unique_collection_passes() {
        let records = [record("I.1.1", "a"), record("I.1.2", "b")];
        assert!(validate_collection(&records).is_empty());
    }

    #[test]
    fn detects_duplicate_id() {
        let records = [record("I.1.1", "a"), record("I.1.1", "a")];
        let codes: Vec<&str> = validate_collection(&records)
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"duplicate-id"));
    }

    #[test]
    fn detects_title_mismatch() {
        let records = [record("I.1.1", "a"), record("I.1.1", "different")];
        let codes: Vec<&str> = validate_collection(&records)
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"title-mismatch"));
    }

    #[test]
    fn unknown_status_rejected_at_decode() {
        // 허용하지 않는 상태는 decoder 경계에서 거부됩니다.
        let value = load_restricted("tasks:\n  - id: I.1.1\n    status: implemented\n").unwrap();
        let err = decode_collection(&value).unwrap_err();
        assert_eq!(err.code, "unknown-status");
    }
}
