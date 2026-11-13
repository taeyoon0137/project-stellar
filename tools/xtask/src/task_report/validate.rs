//! 작업 보고서 validator (I.2.15).
//!
//! 완료/차단 보고서의 필수 절(heading), WBS 일치, 상태 줄, 검증 환경 증거, 명령·검증 표를
//! 검사합니다.
//!
//! 완료 보고서의 절 구성은 `docs/task-reports/TEMPLATE.md`와 실제 보고서 관례(예:
//! `VI.9.5.md`)를 따릅니다. 계획서 §1.2 규칙 11이 요구하는 기록(변경 파일 목록, 실행한 명령과
//! 결과, 검증 환경 코드, 테스트 결과와 남은 경고 수, 작업 지시와 다른 점)마다 필수 절 하나를
//! 두고, 초기 보고서가 같은 내용에 쓰던 heading 별칭도 그 절로 인정합니다. 차단 보고서는
//! `docs/task-reports/BLOCKED_TEMPLATE.md`의 절 구성을 따릅니다.
//!
//! 코드 펜스(```` ``` ````, `~~~`) 안의 줄은 heading·상태 줄·표로 세지 않습니다. 펜스의 열림·닫힘은
//! CommonMark 규칙(같은 문자, 여는 줄 이상 길이, 닫는 줄에는 정보 문자열 없음)으로 판정합니다.

use crate::task_status::model::VerificationEnvironment;
use crate::task_status::wbs_id::WbsId;

/// 보고서 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Complete,
    Blocked,
}

/// 보고서 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportError {
    pub code: &'static str,
    pub message: String,
}

impl ReportError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ReportError {
            code,
            message: message.into(),
        }
    }
}

/// 보고서 한 절(`## ` heading)의 규칙.
struct SectionRule {
    /// 대표 heading 제목(template과 오류 메시지에 쓰는 이름).
    name: &'static str,
    /// 이 절로 인정하는 heading 제목(대표 제목과 초기 보고서 별칭).
    accepted: &'static [&'static str],
    /// 필수 여부. 선택 절은 중복만 검사합니다.
    required: bool,
}

impl SectionRule {
    /// heading 제목이 이 절에 해당하는지 확인합니다.
    ///
    /// 허용 제목과 같거나, 허용 제목 뒤에 ` (...)` 한정어(예: `` (`DEV-MAC`) ``)가 붙은 경우입니다.
    fn matches(&self, title: &str) -> bool {
        self.accepted.iter().any(|accepted| {
            title == *accepted
                || title
                    .strip_prefix(accepted)
                    .and_then(|rest| rest.strip_prefix(" ("))
                    .is_some_and(|rest| rest.ends_with(')'))
        })
    }
}

/// 완료 보고서 `필수 실행 환경` 절(검증 환경 코드를 담는 절).
const ENVIRONMENT_SECTION: SectionRule = SectionRule {
    name: "필수 실행 환경",
    accepted: &["필수 실행 환경"],
    required: true,
};

/// 완료 보고서 `실행한 명령 / 검증` 절(명령·검증 표를 담는 절).
const COMMANDS_SECTION: SectionRule = SectionRule {
    name: "실행한 명령 / 검증",
    accepted: &[
        "실행한 명령 / 검증",
        "실행한 명령",
        "실행한 검증",
        "실행한 명령과 결정론 검증",
    ],
    required: true,
};

/// 완료 보고서 절 규칙(template 순서).
const COMPLETE_SECTIONS: [SectionRule; 6] = [
    ENVIRONMENT_SECTION,
    SectionRule {
        name: "변경 파일 목록",
        accepted: &["변경 파일 목록"],
        required: true,
    },
    SectionRule {
        name: "구현 요약",
        accepted: &["구현 요약"],
        required: false,
    },
    COMMANDS_SECTION,
    SectionRule {
        name: "차이(작업 지시와 다른 점)",
        accepted: &[
            "차이(작업 지시와 다른 점)",
            "작업 지시와 다른 점",
            "작업 지시와 다른 점 / 후속",
        ],
        required: true,
    },
    SectionRule {
        name: "남은 위험 / 한계",
        accepted: &[
            "남은 위험 / 한계",
            "남은 위험 / 후속",
            "남은 경고 수",
            "남은 경고 수 / 한계",
        ],
        required: true,
    },
];

/// 차단 보고서 절 규칙(`BLOCKED_TEMPLATE.md` 순서).
const BLOCKED_SECTIONS: [SectionRule; 6] = [
    SectionRule {
        name: "WBS",
        accepted: &["WBS"],
        required: true,
    },
    SectionRule {
        name: "실패 선행조건",
        accepted: &["실패 선행조건"],
        required: true,
    },
    SectionRule {
        name: "재현 명령",
        accepted: &["재현 명령"],
        required: true,
    },
    SectionRule {
        name: "필요한 결정과 환경",
        accepted: &["필요한 결정과 환경"],
        required: true,
    },
    SectionRule {
        name: "변경되지 않은 파일",
        accepted: &["변경되지 않은 파일"],
        required: true,
    },
    SectionRule {
        name: "재개 조건",
        accepted: &["재개 조건"],
        required: true,
    },
];

/// 상태 줄 접두어.
const STATUS_PREFIX: &str = "- 상태:";

/// 계획서 상태 표기(§1.1).
const STATUS_MARKERS: [&str; 4] = ["[x]", "[~]", "[!]", "[ ]"];

/// 명령·검증 표 header 첫 열로 허용하는 이름.
const COMMAND_TABLE_FIRST_COLUMNS: [&str; 2] = ["명령", "검사"];

/// 명령·검증 표 header에 하나 이상 있어야 하는 결과 열 이름.
const COMMAND_TABLE_RESULT_COLUMNS: [&str; 2] = ["결과", "종료 코드"];

/// 보고서 내용을 검증합니다(빈 Vec = 통과).
pub fn validate_report(content: &str, kind: ReportKind, expected_id: &str) -> Vec<ReportError> {
    let mut errors = Vec::new();
    let lines = visible_lines(content);
    let sections = split_sections(&lines);

    let rules: &[SectionRule] = match kind {
        ReportKind::Complete => &COMPLETE_SECTIONS,
        ReportKind::Blocked => &BLOCKED_SECTIONS,
    };
    check_sections(&sections, rules, &mut errors);

    if !wbs_matches(&lines, expected_id) {
        errors.push(ReportError::new(
            "missing-wbs",
            format!("WBS: {expected_id} 와 일치하는 항목이 없습니다"),
        ));
    }

    check_status(&lines, kind, &mut errors);

    if kind == ReportKind::Complete {
        let environment_evidence = sections_of(&sections, &ENVIRONMENT_SECTION)
            .any(|section| has_environment_evidence(&section.body));
        if !environment_evidence {
            errors.push(ReportError::new(
                "missing-environment",
                format!(
                    "`## {}` 절에 검증 환경 코드(예: DEV-MAC)가 없습니다",
                    ENVIRONMENT_SECTION.name
                ),
            ));
        }

        let command_table = sections_of(&sections, &COMMANDS_SECTION)
            .any(|section| has_command_table(&section.body));
        if !command_table {
            errors.push(ReportError::new(
                "missing-command-table",
                format!(
                    "`## {}` 절에 `| 명령 | 결과 |` 또는 `| 검사 | 결과 |` 형식의 표가 없습니다",
                    COMMANDS_SECTION.name
                ),
            ));
        }
    }

    errors
}

/// `## ` heading 하나와 그 본문(다음 `#`/`##` heading 전까지의 줄).
struct Section<'a> {
    title: &'a str,
    body: Vec<&'a str>,
}

/// 코드 펜스 안의 줄(펜스 구분 줄 포함)을 빈 줄로 바꾼 줄 목록을 반환합니다.
///
/// 줄을 지우지 않고 비우므로 펜스 앞뒤 줄이 인접한 것처럼 보이지 않습니다. 닫히지 않은 펜스는
/// 문서 끝까지 이어집니다(CommonMark).
fn visible_lines(content: &str) -> Vec<&str> {
    let mut open: Option<Fence> = None;
    content
        .lines()
        .map(|line| match open {
            Some(fence) => {
                if fence.is_closed_by(line) {
                    open = None;
                }
                ""
            }
            None => match Fence::opened_by(line) {
                Some(fence) => {
                    open = Some(fence);
                    ""
                }
                None => line,
            },
        })
        .collect()
}

/// 열린 코드 펜스(펜스 문자와 여는 줄의 펜스 길이).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fence {
    marker: u8,
    len: usize,
}

impl Fence {
    /// 줄이 코드 펜스를 여는지 확인합니다.
    ///
    /// CommonMark 규칙: `` ` `` 또는 `~`가 3개 이상 이어지고, backtick 펜스의 정보 문자열에는
    /// backtick이 없어야 합니다(있으면 인라인 코드). 목록 안 펜스를 위해 들여쓰기는 제한하지
    /// 않습니다.
    fn opened_by(line: &str) -> Option<Fence> {
        let trimmed = line.trim_start();
        let marker = *trimmed.as_bytes().first()?;
        if marker != b'`' && marker != b'~' {
            return None;
        }
        let len = run_length(trimmed, marker);
        if len < 3 {
            return None;
        }
        let info = &trimmed[len..];
        if marker == b'`' && info.contains('`') {
            return None;
        }
        Some(Fence { marker, len })
    }

    /// 줄이 이 펜스를 닫는지 확인합니다.
    ///
    /// CommonMark 규칙: 같은 펜스 문자가 여는 줄 이상 이어지고, 그 뒤에는 공백만 있어야 합니다
    /// (정보 문자열이 있는 줄은 닫는 줄이 아닙니다).
    fn is_closed_by(self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let len = run_length(trimmed, self.marker);
        len >= self.len && trimmed[len..].trim().is_empty()
    }
}

/// 문자열 앞에서 ASCII 문자 `marker`가 이어지는 개수(바이트 수와 같음)를 반환합니다.
fn run_length(text: &str, marker: u8) -> usize {
    text.bytes().take_while(|byte| *byte == marker).count()
}

/// 줄 목록을 `## ` 절 단위로 나눕니다. `# ` heading은 직전 절을 끝냅니다.
fn split_sections<'a>(lines: &[&'a str]) -> Vec<Section<'a>> {
    let mut sections: Vec<Section<'a>> = Vec::new();
    let mut in_section = false;
    for line in lines {
        if let Some(title) = line.strip_prefix("## ") {
            sections.push(Section {
                title: title.trim(),
                body: Vec::new(),
            });
            in_section = true;
        } else if line.starts_with("# ") {
            in_section = false;
        } else if in_section {
            if let Some(section) = sections.last_mut() {
                section.body.push(line);
            }
        }
    }
    sections
}

/// 규칙에 해당하는 절을 순서대로 반환합니다.
fn sections_of<'s, 'a>(
    sections: &'s [Section<'a>],
    rule: &'s SectionRule,
) -> impl Iterator<Item = &'s Section<'a>> + 's {
    sections
        .iter()
        .filter(move |section| rule.matches(section.title))
}

/// 필수 절 누락과 같은 heading 제목의 중복을 검사합니다.
fn check_sections(sections: &[Section<'_>], rules: &[SectionRule], errors: &mut Vec<ReportError>) {
    for rule in rules {
        let matched: Vec<&str> = sections_of(sections, rule)
            .map(|section| section.title)
            .collect();
        if rule.required && matched.is_empty() {
            errors.push(ReportError::new(
                "missing-heading",
                format!("필수 heading 누락: ## {}", rule.name),
            ));
        }
        for (index, title) in matched.iter().enumerate() {
            if matched[..index].contains(title) {
                continue;
            }
            if matched[index + 1..].contains(title) {
                errors.push(ReportError::new(
                    "duplicate-heading",
                    format!("heading 중복: ## {title}"),
                ));
            }
        }
    }
}

/// `WBS: <id>` 형태의 줄에서 expected_id와 일치하는 항목이 있는지 확인합니다.
fn wbs_matches(lines: &[&str], expected_id: &str) -> bool {
    for line in lines {
        if let Some(rest) = line.split("WBS:").nth(1) {
            let token = rest.trim().trim_matches('`').split_whitespace().next();
            if let Some(token) = token {
                if WbsId::parse(token).is_some() && token == expected_id {
                    return true;
                }
            }
        }
    }
    false
}

/// `- 상태:` 줄이 정확히 하나이고, 첫 상태 표기가 보고서 종류와 맞는지 검사합니다.
///
/// 완료 보고서는 `[x]` 또는 `[~]`, 차단 보고서는 `[!]`만 허용합니다.
fn check_status(lines: &[&str], kind: ReportKind, errors: &mut Vec<ReportError>) {
    let values: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.trim_start().strip_prefix(STATUS_PREFIX))
        .collect();
    let Some(value) = values.first() else {
        errors.push(ReportError::new(
            "missing-status",
            format!("상태 줄(`{STATUS_PREFIX} ...`)이 없습니다"),
        ));
        return;
    };
    if values.len() > 1 {
        errors.push(ReportError::new(
            "duplicate-status",
            format!("상태 줄(`{STATUS_PREFIX} ...`)이 {}개입니다", values.len()),
        ));
    }
    let allowed: &[&str] = match kind {
        ReportKind::Complete => &["[x]", "[~]"],
        ReportKind::Blocked => &["[!]"],
    };
    match first_status_marker(value) {
        Some(marker) if allowed.contains(&marker) => {}
        Some(marker) => errors.push(ReportError::new(
            "invalid-status",
            format!(
                "상태 표기 {marker}는 이 보고서 종류에 허용되지 않습니다(허용: {})",
                allowed.join(", ")
            ),
        )),
        None => errors.push(ReportError::new(
            "invalid-status",
            format!("상태 줄에 상태 표기({})가 없습니다", allowed.join(", ")),
        )),
    }
}

/// 상태 값에서 가장 먼저 나오는 상태 표기를 반환합니다.
fn first_status_marker(value: &str) -> Option<&'static str> {
    STATUS_MARKERS
        .into_iter()
        .filter_map(|marker| value.find(marker).map(|position| (position, marker)))
        .min_by_key(|(position, _)| *position)
        .map(|(_, marker)| marker)
}

/// 절 본문에 검증 환경 코드가 하나라도 있는지 확인합니다.
fn has_environment_evidence(body: &[&str]) -> bool {
    body.iter().any(|line| {
        VerificationEnvironment::ALL
            .iter()
            .any(|env| line.contains(env.as_str()))
    })
}

/// 절 본문에 명령·검증 표(header, 구분 줄, 데이터 줄 1개 이상)가 있는지 확인합니다.
fn has_command_table(body: &[&str]) -> bool {
    body.windows(3).any(|rows| {
        let (Some(header), Some(separator), Some(data)) = (
            table_cells(rows[0]),
            table_cells(rows[1]),
            table_cells(rows[2]),
        ) else {
            return false;
        };
        is_command_table_header(&header)
            && separator.len() == header.len()
            && separator.iter().all(|cell| is_separator_cell(cell))
            && data.len() >= 2
            && !data[0].is_empty()
            && !data.iter().all(|cell| is_separator_cell(cell))
    })
}

/// `|`로 시작하는 표 줄의 셀을 잘라 반환합니다. 표 줄이 아니면 `None`.
fn table_cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim().strip_prefix('|')?;
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    Some(inner.split('|').map(str::trim).collect())
}

/// 명령·검증 표 header인지 확인합니다.
fn is_command_table_header(cells: &[&str]) -> bool {
    cells.len() >= 2
        && COMMAND_TABLE_FIRST_COLUMNS.contains(&cells[0])
        && cells[1..]
            .iter()
            .any(|cell| COMMAND_TABLE_RESULT_COLUMNS.contains(cell))
}

/// 표 구분 셀(하이픈 1개 이상, 양끝 `:` 선택 — 예: `---`, `:--`, `--:`, `:-:`)인지 확인합니다.
fn is_separator_cell(cell: &str) -> bool {
    let dashes = cell.strip_prefix(':').unwrap_or(cell);
    let dashes = dashes.strip_suffix(':').unwrap_or(dashes);
    !dashes.is_empty() && dashes.chars().all(|c| c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 보고서 관례(`VI.9.5.md` 구조)를 따르는 완료 보고서.
    fn complete_report() -> String {
        "\
# 작업 보고서: I.1.1 제목

- WBS: I.1.1
- 작업명: 제목
- 상태: 완료 (`[x]`)
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `a.rs` (신규)

## 구현 요약

- 요약.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask` | 10 passed, 0 failed |

| 검사 | 결과 |
|---|---|
| fixture | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
"
        .to_string()
    }

    /// 초기 보고서 관례(별칭 heading)를 따르는 완료 보고서.
    fn legacy_complete_report() -> String {
        "\
# 작업 보고서: I.2.15 제목

- WBS: I.2.15
- 작업명: 제목
- 상태: 완료 (`[x]`)
- 선행: I.2.13, I.2.14 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `a.rs` (신규)

## 승인 내용

- 작업별 본문 절.

## 실행한 명령

| 명령 | 결과 | 종료 코드 |
|:---|---:|:---:|
| `cargo check -p xtask --locked` | Finished | 0 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점 / 후속

- 없음.
"
        .to_string()
    }

    fn blocked_report() -> String {
        "\
# 차단 보고서: I.2.4 제목

## WBS

- WBS: I.2.4
- 작업명: 제목
- 상태: `[!]`
- 선행: I.2.3

## 실패 선행조건

- x

## 재현 명령

- y

## 필요한 결정과 환경

- CI-WIN

## 변경되지 않은 파일

- z

## 재개 조건

- w
"
        .to_string()
    }

    fn codes(content: &str, kind: ReportKind, id: &str) -> Vec<&'static str> {
        validate_report(content, kind, id)
            .iter()
            .map(|e| e.code)
            .collect()
    }

    #[test]
    fn valid_complete_report_passes() {
        assert!(validate_report(&complete_report(), ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn legacy_heading_aliases_pass() {
        assert_eq!(
            validate_report(&legacy_complete_report(), ReportKind::Complete, "I.2.15"),
            Vec::new()
        );
        let older = legacy_complete_report()
            .replace(
                "## 실행한 명령\n",
                "## 실행한 명령 (`git check-attr` fixture)\n",
            )
            .replace("## 남은 경고 수\n", "## 남은 경고 수 / 한계\n")
            .replace(
                "## 작업 지시와 다른 점 / 후속\n",
                "## 작업 지시와 다른 점\n",
            );
        assert!(validate_report(&older, ReportKind::Complete, "I.2.15").is_empty());
    }

    #[test]
    fn missing_heading_fails() {
        // 필수 절을 하나씩 지우면 각각 `missing-heading`이 됩니다(별칭이 남지 않도록 대표 제목만 사용).
        for heading in [
            "## 필수 실행 환경\n",
            "## 변경 파일 목록\n",
            "## 실행한 명령 / 검증 (`DEV-MAC`)\n",
            "## 차이(작업 지시와 다른 점)\n",
            "## 남은 위험 / 한계\n",
        ] {
            let content = complete_report().replace(heading, "");
            assert!(
                codes(&content, ReportKind::Complete, "I.1.1").contains(&"missing-heading"),
                "{heading} 제거가 실패하지 않았습니다"
            );
        }
    }

    #[test]
    fn optional_summary_section_may_be_absent() {
        let content = complete_report().replace("## 구현 요약\n\n- 요약.\n\n", "");
        assert!(validate_report(&content, ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn unknown_alias_does_not_satisfy_required_section() {
        // 초기 template의 `## 차이`·`## 남은 위험`은 현재 관례가 아니므로 인정하지 않습니다.
        let content = complete_report()
            .replace("## 차이(작업 지시와 다른 점)\n", "## 차이\n")
            .replace("## 남은 위험 / 한계\n", "## 남은 위험\n");
        let found = codes(&content, ReportKind::Complete, "I.1.1");
        assert_eq!(found, vec!["missing-heading", "missing-heading"]);
    }

    #[test]
    fn duplicate_heading_fails() {
        let content = format!("{}\n## 변경 파일 목록\n\n- b.rs\n", complete_report());
        assert_eq!(
            codes(&content, ReportKind::Complete, "I.1.1"),
            vec!["duplicate-heading"]
        );
        let content = format!("{}\n## 구현 요약\n\n- 다시.\n", complete_report());
        assert_eq!(
            codes(&content, ReportKind::Complete, "I.1.1"),
            vec!["duplicate-heading"]
        );
    }

    #[test]
    fn command_sections_with_distinct_qualifiers_are_not_duplicates() {
        let content = format!(
            "{}\n## 실행한 명령 / 검증 (`CI-MAC`)\n\n| 명령 | 결과 |\n|---|---|\n| `cargo test` | 통과 |\n",
            complete_report()
        );
        assert!(validate_report(&content, ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn headings_inside_code_fence_are_ignored() {
        let content = complete_report().replace(
            "## 필수 실행 환경\n\n- `DEV-MAC`.\n",
            "```text\n## 필수 실행 환경\n\n- `DEV-MAC`.\n```\n",
        );
        let found = codes(&content, ReportKind::Complete, "I.1.1");
        assert!(found.contains(&"missing-heading"));
        assert!(found.contains(&"missing-environment"));
    }

    #[test]
    fn fence_closes_only_on_matching_bare_fence() {
        // CommonMark: 닫는 줄은 여는 줄과 같은 문자로 같거나 더 길고, 정보 문자열이 없어야 합니다.
        // 펜스 안의 정보 문자열 있는 줄·더 짧은 펜스·다른 문자 펜스는 펜스를 닫지 않습니다.
        let section = "## 필수 실행 환경\n\n- `DEV-MAC`.\n";
        for fenced in [
            "```text\n```rust\n## 필수 실행 환경\n\n- `DEV-MAC`.\n```\n",
            "````text\n```\n## 필수 실행 환경\n\n- `DEV-MAC`.\n````\n",
            "~~~\n```\n## 필수 실행 환경\n\n- `DEV-MAC`.\n~~~\n",
            "```\n```` x\n## 필수 실행 환경\n\n- `DEV-MAC`.\n```\n",
        ] {
            let content = complete_report().replace(section, fenced);
            let found = codes(&content, ReportKind::Complete, "I.1.1");
            assert!(found.contains(&"missing-heading"), "{fenced:?}: {found:?}");
            assert!(
                found.contains(&"missing-environment"),
                "{fenced:?}: {found:?}"
            );
        }

        // 더 긴 닫는 줄은 펜스를 닫고, 그 뒤 절은 다시 보입니다.
        let content =
            complete_report().replace(section, &format!("```\n본문\n````` \t\n{section}"));
        assert!(validate_report(&content, ReportKind::Complete, "I.1.1").is_empty());

        // backtick이 정보 문자열에 있는 줄(인라인 코드)은 펜스를 열지 않습니다.
        let content = complete_report().replace(
            "- 작업명: 제목\n",
            "- 작업명: 제목\n```inline``` 코드 설명\n",
        );
        assert!(validate_report(&content, ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn missing_wbs_fails() {
        let content = complete_report().replace("- WBS: I.1.1", "- WBS:");
        assert_eq!(
            codes(&content, ReportKind::Complete, "I.1.1"),
            vec!["missing-wbs"]
        );
        // 다른 ID를 가리키는 보고서도 실패합니다.
        assert_eq!(
            codes(&complete_report(), ReportKind::Complete, "I.1.2"),
            vec!["missing-wbs"]
        );
    }

    #[test]
    fn missing_environment_evidence_fails() {
        let content = complete_report().replace("- `DEV-MAC`.", "- 로컬.");
        assert_eq!(
            codes(&content, ReportKind::Complete, "I.1.1"),
            vec!["missing-environment"]
        );
    }

    #[test]
    fn environment_code_outside_environment_section_does_not_count() {
        // 명령 절 heading에 `DEV-MAC`이 있어도 `필수 실행 환경` 절이 비면 실패합니다.
        let content = complete_report().replace("- `DEV-MAC`.", "- 미정.");
        assert!(content.contains("(`DEV-MAC`)"));
        assert_eq!(
            codes(&content, ReportKind::Complete, "I.1.1"),
            vec!["missing-environment"]
        );
    }

    #[test]
    fn status_line_rules() {
        let partial = complete_report().replace("완료 (`[x]`)", "부분 완료 (`[~]`) — CI 대기");
        assert!(validate_report(&partial, ReportKind::Complete, "I.1.1").is_empty());

        let missing = complete_report().replace("- 상태: 완료 (`[x]`)\n", "");
        assert_eq!(
            codes(&missing, ReportKind::Complete, "I.1.1"),
            vec!["missing-status"]
        );

        let duplicate =
            complete_report().replace("- 선행: 없음\n", "- 선행: 없음\n- 상태: 완료 (`[x]`)\n");
        assert_eq!(
            codes(&duplicate, ReportKind::Complete, "I.1.1"),
            vec!["duplicate-status"]
        );

        for status in ["미착수 (`[ ]`)", "차단 (`[!]`)", "완료"] {
            let content = complete_report().replace("완료 (`[x]`)", status);
            assert_eq!(
                codes(&content, ReportKind::Complete, "I.1.1"),
                vec!["invalid-status"],
                "{status}"
            );
        }

        // 첫 상태 표기가 기준입니다(뒤의 선행 상태 표기는 무시).
        let first_wins = complete_report().replace("완료 (`[x]`)", "`[~]` (선행 I.1.0 `[x]`)");
        assert!(validate_report(&first_wins, ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn command_table_rules() {
        let table = "| 명령 | 결과 |\n|---|---|\n| `cargo test -p xtask` | 10 passed, 0 failed |\n\n| 검사 | 결과 |\n|---|---|\n| fixture | 통과 |\n";
        let replaced = |with: &str| complete_report().replace(table, with);

        let bullets = replaced("- `cargo test -p xtask` → 종료 코드 0\n");
        let no_separator = replaced("| 명령 | 결과 |\n| `cargo test` | 통과 |\n");
        let header_only = replaced("| 명령 | 결과 |\n|---|---|\n\n- 본문.\n");
        let unknown_header = replaced("| 항목 | 값 |\n|---|---|\n| a | b |\n");
        let no_result_column = replaced("| 명령 | 비고 |\n|---|---|\n| `cargo test` | - |\n");
        let column_mismatch = replaced("| 명령 | 결과 |\n|---|\n| `cargo test` | 통과 |\n");
        let empty_first_cell = replaced("| 명령 | 결과 |\n|---|---|\n|  | 통과 |\n");
        let fenced =
            replaced("```text\n| 명령 | 결과 |\n|---|---|\n| `cargo test` | 통과 |\n```\n");
        for (name, content) in [
            ("bullets", bullets),
            ("no_separator", no_separator),
            ("header_only", header_only),
            ("unknown_header", unknown_header),
            ("no_result_column", no_result_column),
            ("column_mismatch", column_mismatch),
            ("empty_first_cell", empty_first_cell),
            ("fenced", fenced),
        ] {
            assert_eq!(
                codes(&content, ReportKind::Complete, "I.1.1"),
                vec!["missing-command-table"],
                "{name}"
            );
        }

        // 다른 절에 있는 표는 명령 절의 표로 세지 않습니다.
        let moved = replaced("- 표는 아래 절에 있습니다.\n").replace(
            "## 구현 요약\n\n- 요약.\n",
            &format!("## 구현 요약\n\n{table}"),
        );
        assert_eq!(
            codes(&moved, ReportKind::Complete, "I.1.1"),
            vec!["missing-command-table"]
        );

        // 검사 표 하나만 있어도 통과합니다.
        let checks_only = replaced("| 검사 | 결과 |\n|:--|--:|\n| fixture | 통과 |\n");
        assert!(validate_report(&checks_only, ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn valid_blocked_report_passes() {
        assert!(validate_report(&blocked_report(), ReportKind::Blocked, "I.2.4").is_empty());
    }

    #[test]
    fn blocked_report_rules() {
        let content = blocked_report().replace("## 재개 조건\n", "");
        assert_eq!(
            codes(&content, ReportKind::Blocked, "I.2.4"),
            vec!["missing-heading"]
        );
        let content = blocked_report().replace("`[!]`", "`[x]`");
        assert_eq!(
            codes(&content, ReportKind::Blocked, "I.2.4"),
            vec!["invalid-status"]
        );
        // 차단 보고서는 명령 표·환경 절을 요구하지 않지만, 완료 보고서로 검사하면 실패합니다.
        assert!(!codes(&blocked_report(), ReportKind::Complete, "I.2.4").is_empty());
    }

    #[test]
    fn report_templates_satisfy_validator() {
        // template과 validator가 같은 절 구성을 유지하는지 고정합니다.
        let complete = include_str!("../../../../docs/task-reports/TEMPLATE.md");
        assert_eq!(
            validate_report(
                &complete.replace("<WBS-ID>", "I.1.1"),
                ReportKind::Complete,
                "I.1.1"
            ),
            Vec::new()
        );
        // 선택 절을 포함한 모든 절이 대표 제목으로 정확히 한 번씩 있어야 합니다.
        let lines = visible_lines(complete);
        let sections = split_sections(&lines);
        for rule in &COMPLETE_SECTIONS {
            let titles: Vec<&str> = sections_of(&sections, rule)
                .map(|section| section.title)
                .collect();
            assert_eq!(titles.len(), 1, "TEMPLATE.md의 ## {} 개수", rule.name);
            assert!(
                titles[0].starts_with(rule.name),
                "TEMPLATE.md는 별칭이 아닌 대표 제목 ## {}을 써야 합니다",
                rule.name
            );
        }
        assert_eq!(sections.len(), COMPLETE_SECTIONS.len());

        let blocked = include_str!("../../../../docs/task-reports/BLOCKED_TEMPLATE.md");
        assert_eq!(
            validate_report(
                &blocked.replace("<WBS-ID>", "I.2.4"),
                ReportKind::Blocked,
                "I.2.4"
            ),
            Vec::new()
        );
    }
}
