//! 계획서 본문 WBS ID 참조 추출 (XXIX.1.3).
//!
//! task 행의 구현 지시·검증 셀과 Phase 완료 조건 문단에 나온 WBS ID 형식 토큰을 모읍니다.
//! `sync-plan`은 이 참조가 계획서에 정의된 ID인지 검사합니다.
//!
//! - 토큰은 ASCII 영숫자와 `.`로 이뤄진 최대 연속 구간입니다. 앞뒤의 `.`(문장 끝 마침표)를 떼고
//!   [`WbsId::parse`]가 받아들이는 것만 참조로 셉니다. 그래서 `1.1.1`(Roman Phase 아님),
//!   `I.1.1.1`(네 단계), `I.01.1`(선행 0), `XIX.9`(두 단계), `v0.11.0` 같은 표기는 세지 않습니다.
//! - 범위 표기 `A~B`는 `~`가 토큰 경계이므로 양 끝 ID만 각각 참조로 셉니다. 중간 ID는 펼치지 않습니다.
//! - 선행 셀은 ledger depends_on 검사가 맡으므로 여기서 세지 않습니다. fenced code block, 개정 기록과
//!   요약 표 같은 그 밖의 본문, 계획서 밖 문서는 검사하지 않습니다.

use crate::task_status::wbs_id::WbsId;

/// 참조가 나온 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefLocation {
    /// task 행(행 ID)의 구현 지시·검증 셀.
    TaskRow(String),
    /// Phase 완료 조건 문단.
    PhaseCompletion,
}

/// 본문의 WBS ID 참조 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdReference {
    /// 1부터 세는 계획서 행 번호.
    pub line: usize,
    pub id: String,
    pub location: RefLocation,
}

/// 계획서에서 검사 대상 위치의 WBS ID 참조를 모읍니다(행 순서).
pub fn extract_id_references(markdown: &str) -> Vec<IdReference> {
    let mut refs = Vec::new();
    let mut in_fence = false;
    let mut in_completion = false;

    for (index, raw) in markdown.lines().enumerate() {
        let line_no = index + 1;
        if raw.trim_start().starts_with("```") {
            in_fence = !in_fence;
            in_completion = false;
            continue;
        }
        if in_fence {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            in_completion = false;
            continue;
        }
        if line.starts_with('|') {
            in_completion = false;
            if let Some((row_id, body)) = task_row_body(line) {
                for id in wbs_id_tokens(&body) {
                    refs.push(IdReference {
                        line: line_no,
                        id,
                        location: RefLocation::TaskRow(row_id.clone()),
                    });
                }
            }
            continue;
        }
        if is_completion_start(line) {
            in_completion = true;
        }
        if in_completion {
            for id in wbs_id_tokens(line) {
                refs.push(IdReference {
                    line: line_no,
                    id,
                    location: RefLocation::PhaseCompletion,
                });
            }
        }
    }
    refs
}

/// `**Phase <N> 완료 조건:**`, `**Phase <N> 추가 완료 조건:**` 문단의 시작인지 판정합니다.
fn is_completion_start(line: &str) -> bool {
    line.strip_prefix("**Phase ").is_some_and(|rest| {
        rest.split_once("**")
            .is_some_and(|(label, _)| label.contains("완료 조건"))
    })
}

/// task 정의 행이면 (행 ID, 상태와 선행 사이 셀 본문)을 반환합니다.
///
/// 행 판정은 `extract_wbs`와 같습니다: 셀 5개 이상, 첫 셀이 WBS ID, 둘째 셀이 상태 marker.
fn task_row_body(line: &str) -> Option<(String, String)> {
    let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
    if cells.len() < 5 {
        return None;
    }
    let id = cells[0].trim_matches('`').trim();
    WbsId::parse(id)?;
    let marker = cells[1].trim().trim_matches('`').trim();
    if !matches!(marker, "[x]" | "[~]" | "[ ]" | "[!]") {
        return None;
    }
    // 셀 안의 `|`로 나뉜 경우에도 구현 지시·검증 본문 전체를 덮도록 상태와 선행 사이를 합칩니다.
    let body = cells[2..cells.len() - 1].join(" | ");
    Some((id.to_string(), body))
}

/// 텍스트에서 WBS ID 형식 토큰을 순서대로 찾습니다.
pub fn wbs_id_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .map(|word| word.trim_matches('.'))
        .filter(|word| WbsId::parse(word).is_some())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(refs: &[IdReference]) -> Vec<&str> {
        refs.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn tokens_follow_wbs_id_format_only() {
        let text = "정상 `I.1.1`, XXVIII.3.3; 실패 1.1.1, I.0.1, I.1, I.1.1.1, I.01.1, IIII.1.1, \
                    v0.11.0, XIX.9, docs/task-reports/I.2.1.md, Task(I.4.1), XIX.6.4절차, 끝 II.1.2.";
        assert_eq!(
            wbs_id_tokens(text),
            ["I.1.1", "XXVIII.3.3", "I.4.1", "XIX.6.4", "II.1.2"]
        );
    }

    #[test]
    fn ranges_contribute_both_ends_only() {
        assert_eq!(
            wbs_id_tokens("XIX.6.7~XIX.6.10, `I.1.1`~`I.1.5`, XIX.1~XIX.3"),
            ["XIX.6.7", "XIX.6.10", "I.1.1", "I.1.5"]
        );
    }

    #[test]
    fn extracts_instruction_verification_and_completion_only() {
        let md = "\
- 개정 기록: I.9.1을 고쳤습니다.

| ID | 상태 | 구현 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | I.1.2와 맞춥니다 | I.1.3 fixture | I.9.2 |
| `I.1.1`~`I.1.5` | `[x]` 검증 완료 | I.9.3 요약 |

```text
| I.1.4 | `[ ]` | I.9.4 예시 | ok | 없음 |
```

**Phase I 완료 조건:** I.1.6 통과.
이어지는 줄의 I.1.7도 문단입니다.

본문 I.9.5는 검사하지 않습니다.
**Phase I 추가 완료 조건:** I.1.8~I.1.9.
";
        let refs = extract_id_references(md);
        assert_eq!(
            ids(&refs),
            ["I.1.2", "I.1.3", "I.1.6", "I.1.7", "I.1.8", "I.1.9"]
        );
        assert_eq!(refs[0].location, RefLocation::TaskRow("I.1.1".to_string()));
        assert_eq!(refs[0].line, 5);
        assert_eq!(refs[2].location, RefLocation::PhaseCompletion);
        assert_eq!(refs[2].line, 12);
    }
}
