//! 작업 보고서 validator (I.2.15).
//!
//! # 검사 항목
//!
//! - 필수 절: 완료 보고서는 `docs/task-reports/TEMPLATE.md`(I.2.13)가 고정한 8개 절(WBS, 변경 파일,
//!   명령과 종료 코드, 환경과 증거, 테스트, 성능, 차이, 남은 위험), 차단 보고서는
//!   `docs/task-reports/BLOCKED_TEMPLATE.md`(I.2.14)의 6개 절이 있어야 합니다. 같은 `## ` 제목은
//!   두 번 나올 수 없습니다.
//! - WBS 일치: 머리 목록 또는 `## WBS` 절의 `WBS: <ID>`가 파일 이름의 ID와 같아야 합니다.
//! - 상태 줄: `- 상태:` 줄이 하나이고, 첫 상태 표기가 완료 보고서는 `[x]`·`[~]`, 차단 보고서는
//!   `[!]`입니다.
//! - 검증 환경: 완료 보고서의 환경 절에 검증 환경 코드(`DEV-MAC` 등)가 있어야 합니다.
//! - 명령 표: 완료 보고서의 명령 절에 `| 명령 | … |` 또는 `| 검사 | … |` 표(header·구분 줄·데이터
//!   줄)가 있고, header에 `결과` 또는 `종료 코드` 열이 있어야 합니다.
//! - placeholder: 목록 항목·`- 키: 값`의 값·표 셀이 template placeholder(`<WBS-ID>`,
//!   `` `<command>` `` 등) 하나뿐이면 안 됩니다. template을 복사만 한 보고서는 이 검사로 실패합니다.
//!
//! # 완료 보고서 절 별칭
//!
//! template 제목 외에, 실제 보고서 관례(예: `VI.9.5.md`)와 초기 보고서가 같은 기록에 쓰는 heading을
//! 절마다 별칭으로 인정합니다(전체 목록은 `COMPLETE_SECTIONS`). heading 뒤의 ` (...)` 한정어(예:
//! `` (`DEV-MAC`) ``)는 무시합니다.
//!
//! - WBS: `## WBS` 절 대신 제목 바로 아래 머리 목록의 `- WBS:` 줄.
//! - 변경 파일 `변경 파일 목록`, 환경과 증거 `필수 실행 환경`, 차이 `차이(작업 지시와 다른 점)`·
//!   `작업 지시와 다른 점`, 남은 위험 `남은 위험 / 한계`·`남은 위험 / 후속`·`남은 경고 수 / 한계`.
//! - 명령과 종료 코드: `실행한 명령 / 검증`·`실행한 명령`·`실행한 검증`·`실행한 명령과 결정론 검증`.
//! - 테스트·성능: `테스트 결과`·`남은 경고 수`(계획서 §1.2 규칙 11의 "테스트 결과와 남은 경고 수"),
//!   `성능 수치`. 명령과 검증 결과를 한 절에 담는 관례(`실행한 명령 / 검증`·`실행한 검증`·
//!   `실행한 명령과 결정론 검증`)를 쓰면 그 절이 테스트·성능 기록을 겸합니다.
//!
//! `남은 경고 수`는 경고 수 기록이므로 남은 위험 절로 인정하지 않습니다. 남은 위험 절이 관례가 되기
//! 전에 작성되어 그 절이 없는 초기 보고서는 `task_report::legacy`에 고정한 ID만 통과합니다.
//!
//! # 검사하지 않는 것
//!
//! 계획서 §1.2 규칙 11과 I.2.15 작업 행이 요구하는 기록 중 다음은 내용 형식을 검사하지 않습니다.
//! 기존 보고서 다수가 이 형식을 따르지 않아(예: `결과` 열만 있는 명령 표, `비0`·`—` 같은 종료 코드
//! 표기) 소급 강제하지 않았습니다.
//!
//! - 명령별 종료 코드: `결과` 열만 있는 명령 표도 통과하고, 종료 코드 값의 형식은 보지 않습니다.
//! - CI·LAB·REF 실행 URL과 artifact locator 형식, 성능 원시 결과 경로, 테스트 수·경고 수 값.
//!
//! 코드 펜스(```` ``` ````, `~~~`) 안의 줄은 heading·상태 줄·표로 세지 않습니다. 펜스의 열림·닫힘은
//! CommonMark 규칙(같은 문자, 여는 줄 이상 길이, 닫는 줄에는 정보 문자열 없음)으로 판정합니다.

use std::collections::HashSet;

use super::legacy::is_legacy_report_without_risk_section;
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
    /// template heading 제목(오류 메시지에 쓰는 이름).
    name: &'static str,
    /// 이 절로 인정하는 heading 제목(template 제목과 보고서 관례 별칭).
    accepted: &'static [&'static str],
    /// heading이 없을 때 이 절을 대신하는 기록.
    fallback: Fallback,
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

/// heading이 없을 때 절을 대신하는 기록.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fallback {
    /// 대신하는 기록이 없습니다(heading 필수).
    None,
    /// 첫 `## ` heading 앞 머리 목록의 `- WBS:` 줄(보고서 관례).
    PreambleWbs,
    /// 명령과 검증 결과를 함께 담는 명령 절(`COMBINED_COMMANDS_SECTION`).
    CombinedCommands,
    /// 남은 위험 절이 관례가 되기 전의 초기 보고서(`task_report::legacy`에 고정한 ID).
    LegacyReport,
}

/// 완료 보고서 `WBS` 절(WBS·작업명·상태·선행 머리 목록).
const WBS_SECTION: SectionRule = SectionRule {
    name: "WBS",
    accepted: &["WBS"],
    fallback: Fallback::PreambleWbs,
};

/// 완료 보고서 `명령과 종료 코드` 절(명령·검증 표를 담는 절).
const COMMANDS_SECTION: SectionRule = SectionRule {
    name: "명령과 종료 코드",
    accepted: &[
        "명령과 종료 코드",
        "실행한 명령 / 검증",
        "실행한 명령",
        "실행한 검증",
        "실행한 명령과 결정론 검증",
    ],
    fallback: Fallback::None,
};

/// 명령과 검증 결과(테스트·성능 기록 포함)를 한 절에 담는 보고서 관례의 명령 절.
const COMBINED_COMMANDS_SECTION: SectionRule = SectionRule {
    name: "실행한 명령 / 검증",
    accepted: &[
        "실행한 명령 / 검증",
        "실행한 검증",
        "실행한 명령과 결정론 검증",
    ],
    fallback: Fallback::None,
};

/// 완료 보고서 `환경과 증거` 절(검증 환경 코드를 담는 절).
const ENVIRONMENT_SECTION: SectionRule = SectionRule {
    name: "환경과 증거",
    accepted: &["환경과 증거", "필수 실행 환경"],
    fallback: Fallback::None,
};

/// 완료 보고서 `남은 위험` 절. `남은 경고 수`(경고 수 기록)는 이 절로 인정하지 않습니다.
const RISK_SECTION: SectionRule = SectionRule {
    name: "남은 위험",
    accepted: &[
        "남은 위험",
        "남은 위험 / 한계",
        "남은 위험 / 후속",
        "남은 경고 수 / 한계",
    ],
    fallback: Fallback::LegacyReport,
};

/// 완료 보고서 절 규칙(`TEMPLATE.md` 순서, I.2.13).
const COMPLETE_SECTIONS: [SectionRule; 8] = [
    WBS_SECTION,
    SectionRule {
        name: "변경 파일",
        accepted: &["변경 파일", "변경 파일 목록"],
        fallback: Fallback::None,
    },
    COMMANDS_SECTION,
    ENVIRONMENT_SECTION,
    SectionRule {
        name: "테스트",
        accepted: &[
            "테스트",
            "테스트 결과",
            "남은 경고 수",
            "남은 경고 수 / 한계",
        ],
        fallback: Fallback::CombinedCommands,
    },
    SectionRule {
        name: "성능",
        accepted: &["성능", "성능 수치"],
        fallback: Fallback::CombinedCommands,
    },
    SectionRule {
        name: "차이",
        accepted: &[
            "차이",
            "차이(작업 지시와 다른 점)",
            "작업 지시와 다른 점",
            "작업 지시와 다른 점 / 후속",
        ],
        fallback: Fallback::None,
    },
    RISK_SECTION,
];

/// 차단 보고서 절 규칙(`BLOCKED_TEMPLATE.md` 순서, I.2.14).
const BLOCKED_SECTIONS: [SectionRule; 6] = [
    SectionRule {
        name: "WBS",
        accepted: &["WBS"],
        fallback: Fallback::None,
    },
    SectionRule {
        name: "실패 선행조건",
        accepted: &["실패 선행조건"],
        fallback: Fallback::None,
    },
    SectionRule {
        name: "재현 명령",
        accepted: &["재현 명령"],
        fallback: Fallback::None,
    },
    SectionRule {
        name: "필요한 결정과 환경",
        accepted: &["필요한 결정과 환경"],
        fallback: Fallback::None,
    },
    SectionRule {
        name: "변경되지 않은 파일",
        accepted: &["변경되지 않은 파일"],
        fallback: Fallback::None,
    },
    SectionRule {
        name: "재개 조건",
        accepted: &["재개 조건"],
        fallback: Fallback::None,
    },
];

/// WBS 머리 목록 줄 접두어.
const WBS_PREFIX: &str = "- WBS:";

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
    let report = ParsedReport::parse(&lines);

    let rules: &[SectionRule] = match kind {
        ReportKind::Complete => &COMPLETE_SECTIONS,
        ReportKind::Blocked => &BLOCKED_SECTIONS,
    };
    check_sections(&report, rules, expected_id, &mut errors);

    if !wbs_matches(report.wbs_lines(), expected_id) {
        errors.push(ReportError::new(
            "missing-wbs",
            format!("머리 목록 또는 `## WBS` 절에 WBS: {expected_id} 와 일치하는 항목이 없습니다"),
        ));
    }

    check_status(&lines, kind, &mut errors);
    check_placeholders(&lines, &mut errors);

    if kind == ReportKind::Complete {
        let environment_evidence = report
            .sections_of(&ENVIRONMENT_SECTION)
            .any(|section| has_environment_evidence(&section.body));
        if !environment_evidence {
            errors.push(ReportError::new(
                "missing-environment",
                format!(
                    "환경 절(## {})에 검증 환경 코드(예: DEV-MAC)가 없습니다",
                    ENVIRONMENT_SECTION.accepted.join(" | ")
                ),
            ));
        }

        let command_table = report
            .sections_of(&COMMANDS_SECTION)
            .any(|section| has_command_table(&section.body));
        if !command_table {
            errors.push(ReportError::new(
                "missing-command-table",
                format!(
                    "명령 절(## {})에 `| 명령 | 결과 |`·`| 명령 | 종료 코드 |`·`| 검사 | 결과 |` 형식의 표가 없습니다",
                    COMMANDS_SECTION.accepted.join(" | ")
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

/// 머리 목록과 `## ` 절로 나눈 보고서.
struct ParsedReport<'a> {
    /// 첫 `## ` heading 앞의 줄(`# ` 제목 줄 제외). 보고서 관례의 WBS 머리 목록이 여기 있습니다.
    preamble: Vec<&'a str>,
    sections: Vec<Section<'a>>,
}

impl<'a> ParsedReport<'a> {
    /// 줄 목록을 머리 목록과 `## ` 절 단위로 나눕니다. `# ` heading은 직전 절을 끝냅니다.
    fn parse(lines: &[&'a str]) -> Self {
        let mut preamble = Vec::new();
        let mut sections: Vec<Section<'a>> = Vec::new();
        let mut in_section = false;
        for &line in lines {
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
            } else if sections.is_empty() {
                preamble.push(line);
            }
        }
        ParsedReport { preamble, sections }
    }

    /// 규칙에 해당하는 절을 순서대로 반환합니다.
    fn sections_of<'s>(&'s self, rule: &'s SectionRule) -> impl Iterator<Item = &'s Section<'a>> {
        self.sections
            .iter()
            .filter(move |section| rule.matches(section.title))
    }

    /// 머리 목록과 `## WBS` 절 본문의 줄(WBS 기록이 있을 수 있는 자리)을 반환합니다.
    fn wbs_lines(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.preamble.iter().copied().chain(
            self.sections_of(&WBS_SECTION)
                .flat_map(|section| section.body.iter().copied()),
        )
    }

    /// 머리 목록에 `- WBS:` 줄이 있는지 확인합니다.
    fn has_preamble_wbs(&self) -> bool {
        self.preamble
            .iter()
            .any(|line| line.trim_start().starts_with(WBS_PREFIX))
    }
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

/// 필수 절 누락과 같은 `## ` 제목의 중복을 검사합니다.
///
/// 절은 허용 heading이 있거나, heading 대신 인정하는 기록(`Fallback`)이 있으면 있는 것으로 봅니다.
fn check_sections(
    report: &ParsedReport<'_>,
    rules: &[SectionRule],
    expected_id: &str,
    errors: &mut Vec<ReportError>,
) {
    for rule in rules {
        let present = report.sections_of(rule).next().is_some()
            || match rule.fallback {
                Fallback::None => false,
                Fallback::PreambleWbs => report.has_preamble_wbs(),
                Fallback::CombinedCommands => report
                    .sections_of(&COMBINED_COMMANDS_SECTION)
                    .next()
                    .is_some(),
                Fallback::LegacyReport => is_legacy_report_without_risk_section(expected_id),
            };
        if !present {
            errors.push(ReportError::new(
                "missing-heading",
                format!(
                    "필수 heading 누락: ## {} (허용 제목: {})",
                    rule.name,
                    rule.accepted.join(", ")
                ),
            ));
        }
    }

    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    for section in &report.sections {
        if !seen.insert(section.title) && reported.insert(section.title) {
            errors.push(ReportError::new(
                "duplicate-heading",
                format!("heading 중복: ## {}", section.title),
            ));
        }
    }
}

/// `WBS: <id>` 형태의 줄에서 expected_id와 일치하는 항목이 있는지 확인합니다.
fn wbs_matches<'a>(lines: impl IntoIterator<Item = &'a str>, expected_id: &str) -> bool {
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

/// template placeholder가 그대로 남은 줄을 검사합니다(코드 펜스 밖).
///
/// 목록 항목 전체, `- 키: 값` 목록 항목의 값, 표 셀이 placeholder 하나뿐이면 채우지 않은 것으로
/// 봅니다. 문장이나 인라인 코드 속 `<...>` 표기(예: `` `inspect <경로>` ``)는 세지 않습니다. 오류는
/// 하나로 모으고 해당 줄 번호를 모두 적습니다.
fn check_placeholders(lines: &[&str], errors: &mut Vec<ReportError>) {
    let unfilled: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| has_placeholder_value(line))
        .map(|(index, _)| (index + 1).to_string())
        .collect();
    if !unfilled.is_empty() {
        errors.push(ReportError::new(
            "unfilled-placeholder",
            format!(
                "채우지 않은 template placeholder가 있습니다: 줄 {}",
                unfilled.join(", ")
            ),
        ));
    }
}

/// 줄의 목록 항목 값 또는 표 셀 중 placeholder 하나뿐인 값이 있는지 확인합니다.
fn has_placeholder_value(line: &str) -> bool {
    if let Some(cells) = table_cells(line) {
        return cells.iter().any(|cell| is_placeholder(cell));
    }
    let trimmed = line.trim_start();
    let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    else {
        return false;
    };
    if is_placeholder(item) {
        return true;
    }
    // `- 키: 값` 형식이면 값만 봅니다. 키에 `<`가 있으면 placeholder 안의 `:`이므로 나누지 않습니다.
    item.split_once(':')
        .is_some_and(|(key, value)| !key.contains('<') && is_placeholder(value))
}

/// 값이 template placeholder 하나뿐인지(예: `<WBS-ID>`, `` `<command>` ``) 확인합니다.
///
/// 앞뒤 공백·끝 마침표·감싼 backtick을 걷어낸 값이 `<`로 시작해 `>`로 끝나고, 안쪽이 비어 있지 않으며
/// `<`·`>`를 더 담지 않아야 합니다. autolink(`<https://…>`, `<a@b.c>`)와 HTML 주석·선언(`<!…>`)은
/// placeholder가 아닙니다.
fn is_placeholder(value: &str) -> bool {
    let value = value.trim();
    let value = value.strip_suffix('.').unwrap_or(value);
    let value = value
        .strip_prefix('`')
        .and_then(|inner| inner.strip_suffix('`'))
        .unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    !inner.trim().is_empty()
        && !inner.contains(['<', '>'])
        && !inner.starts_with('!')
        && !is_autolink(inner)
}

/// `<`·`>` 안쪽이 CommonMark autolink(URI 또는 email)인지 확인합니다.
///
/// URI: 글자로 시작하는 2~32자 scheme(글자·숫자·`+`·`.`·`-`) 뒤에 `:`와 공백 없는 나머지.
/// email: 공백 없이 `@`를 담은 값.
fn is_autolink(inner: &str) -> bool {
    if inner.contains(char::is_whitespace) {
        return false;
    }
    if inner.contains('@') {
        return true;
    }
    inner.split_once(':').is_some_and(|(scheme, _)| {
        (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    })
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
    use crate::task_report::legacy::LEGACY_REPORTS_WITHOUT_RISK_SECTION;

    /// fixture 보고서 ID(초기 보고서 고정 목록에 없는 ID).
    const ID: &str = "VI.9.5";

    /// I.2.13이 고정한 완료 보고서 template 절 제목(template 순서).
    const I_2_13_TEMPLATE_HEADINGS: [&str; 8] = [
        "WBS",
        "변경 파일",
        "명령과 종료 코드",
        "환경과 증거",
        "테스트",
        "성능",
        "차이",
        "남은 위험",
    ];

    /// 실제 보고서 관례(`VI.9.5.md` 구조)를 따르는 완료 보고서.
    fn complete_report() -> String {
        "\
# 작업 보고서: VI.9.5 제목

- WBS: VI.9.5
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

    /// `TEMPLATE.md`(I.2.13)의 8개 절 제목을 그대로 쓰는 완료 보고서.
    fn template_report() -> String {
        "\
# 작업 보고서: VI.9.5 제목

## WBS

- WBS: VI.9.5
- 작업명: 제목
- 상태: 완료 (`[x]`)
- 선행: 없음

## 변경 파일

- `a.rs` (신규)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p xtask` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(로컬 검증)

## 테스트

- 10 passed, 0 failed, 경고 0

## 성능

- 해당 없음

## 차이

- 없음.

## 남은 위험

- 없음.
"
        .to_string()
    }

    /// 초기 보고서 관례(별칭 heading, 남은 위험 절 없음)를 따르는 완료 보고서.
    ///
    /// I.2.15는 초기 보고서 고정 목록에 있는 ID입니다.
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

## 성능 수치

- 해당 없음

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

    /// 저장소의 작업 보고서 디렉터리.
    fn reports_dir() -> std::path::PathBuf {
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/task-reports"
        ))
        .to_path_buf()
    }

    #[test]
    fn valid_complete_report_passes() {
        assert_eq!(
            validate_report(&complete_report(), ReportKind::Complete, ID),
            Vec::new()
        );
    }

    #[test]
    fn template_layout_report_passes() {
        assert_eq!(
            validate_report(&template_report(), ReportKind::Complete, ID),
            Vec::new()
        );
    }

    #[test]
    fn template_layout_requires_each_heading() {
        // I.2.15 검증: template 절 heading을 하나씩 지운 fixture가 실패합니다.
        for heading in [
            "## 변경 파일\n",
            "## 명령과 종료 코드\n",
            "## 환경과 증거\n",
            "## 테스트\n",
            "## 성능\n",
            "## 차이\n",
            "## 남은 위험\n",
        ] {
            let content = template_report().replace(heading, "");
            assert!(
                codes(&content, ReportKind::Complete, ID).contains(&"missing-heading"),
                "{heading} 제거가 실패하지 않았습니다"
            );
        }

        // `## WBS` heading만 지우면 WBS 머리 목록이 제목 바로 아래에 남아 보고서 관례와 같아집니다.
        let content = template_report().replace("## WBS\n\n", "");
        assert_eq!(
            validate_report(&content, ReportKind::Complete, ID),
            Vec::new()
        );
        // WBS 줄까지 지우면 WBS 절과 WBS 일치가 모두 실패합니다.
        let content = template_report().replace("## WBS\n\n- WBS: VI.9.5\n", "");
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-heading", "missing-wbs"]
        );
    }

    #[test]
    fn established_layout_requires_each_heading() {
        // 보고서 관례의 필수 절을 하나씩 지우면 각각 `missing-heading`이 됩니다.
        for heading in [
            "## 필수 실행 환경\n",
            "## 변경 파일 목록\n",
            "## 실행한 명령 / 검증 (`DEV-MAC`)\n",
            "## 차이(작업 지시와 다른 점)\n",
            "## 남은 위험 / 한계\n",
        ] {
            let content = complete_report().replace(heading, "");
            assert!(
                codes(&content, ReportKind::Complete, ID).contains(&"missing-heading"),
                "{heading} 제거가 실패하지 않았습니다"
            );
        }

        // 머리 목록의 WBS 줄을 지우면 WBS 절이 없는 것으로 봅니다.
        let content = complete_report().replace("- WBS: VI.9.5\n", "");
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-heading", "missing-wbs"]
        );
    }

    #[test]
    fn optional_summary_section_may_be_absent() {
        let content = complete_report().replace("## 구현 요약\n\n- 요약.\n\n", "");
        assert!(validate_report(&content, ReportKind::Complete, ID).is_empty());
    }

    #[test]
    fn combined_commands_section_covers_tests_and_performance() {
        // 명령만 담는 `## 실행한 명령`이면 테스트·성능 절이 따로 있어야 합니다.
        let commands_only =
            complete_report().replace("## 실행한 명령 / 검증 (`DEV-MAC`)\n", "## 실행한 명령\n");
        let errors = validate_report(&commands_only, ReportKind::Complete, ID);
        assert_eq!(
            errors.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec!["missing-heading", "missing-heading"]
        );
        assert!(errors[0].message.contains("## 테스트 "), "{errors:?}");
        assert!(errors[1].message.contains("## 성능 "), "{errors:?}");

        let with_records = commands_only.replace(
            "## 차이(작업 지시와 다른 점)\n",
            "## 테스트 결과\n\n- 10 passed\n\n## 성능 수치\n\n- 해당 없음\n\n## 차이(작업 지시와 다른 점)\n",
        );
        assert!(validate_report(&with_records, ReportKind::Complete, ID).is_empty());

        // template의 `## 명령과 종료 코드`도 테스트·성능 기록을 겸하지 않습니다.
        let template_commands = template_report().replace(
            "## 테스트\n\n- 10 passed, 0 failed, 경고 0\n\n## 성능\n\n- 해당 없음\n\n",
            "",
        );
        assert_eq!(
            codes(&template_commands, ReportKind::Complete, ID),
            vec!["missing-heading", "missing-heading"]
        );
    }

    #[test]
    fn warning_count_is_not_a_risk_section() {
        // `남은 경고 수`는 경고 수 기록(테스트 절 별칭)이므로 남은 위험 절을 대신하지 못합니다.
        let content = complete_report().replace("## 남은 위험 / 한계\n", "## 남은 경고 수\n");
        let errors = validate_report(&content, ReportKind::Complete, ID);
        assert_eq!(
            errors.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec!["missing-heading"]
        );
        assert!(errors[0].message.contains("## 남은 위험 "), "{errors:?}");

        // 한계를 함께 적는 `남은 경고 수 / 한계`는 남은 위험 절로 인정합니다.
        let content =
            complete_report().replace("## 남은 위험 / 한계\n", "## 남은 경고 수 / 한계\n");
        assert!(validate_report(&content, ReportKind::Complete, ID).is_empty());
    }

    #[test]
    fn legacy_layout_passes_only_for_frozen_reports() {
        // 고정 목록에 있는 초기 보고서는 별칭 heading과 남은 위험 절 없는 구성으로 통과합니다.
        assert_eq!(
            validate_report(&legacy_complete_report(), ReportKind::Complete, "I.2.15"),
            Vec::new()
        );
        let older = legacy_complete_report()
            .replace(
                "## 실행한 명령\n",
                "## 실행한 명령 (`git check-attr` fixture)\n",
            )
            .replace(
                "## 남은 경고 수\n",
                "## 테스트 결과\n\n- 통과\n\n## 남은 경고 수\n",
            )
            .replace(
                "## 작업 지시와 다른 점 / 후속\n",
                "## 작업 지시와 다른 점\n",
            );
        assert!(validate_report(&older, ReportKind::Complete, "I.2.15").is_empty());

        // 같은 구성의 새 보고서는 남은 위험 절이 없어 실패합니다.
        let new_report = legacy_complete_report().replace("I.2.15", ID);
        let errors = validate_report(&new_report, ReportKind::Complete, ID);
        assert_eq!(
            errors.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec!["missing-heading"]
        );
        assert!(errors[0].message.contains("## 남은 위험 "), "{errors:?}");
    }

    #[test]
    fn unknown_heading_does_not_satisfy_required_section() {
        let content = complete_report()
            .replace("## 차이(작업 지시와 다른 점)\n", "## 비고\n")
            .replace("## 남은 위험 / 한계\n", "## 후속\n");
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-heading", "missing-heading"]
        );
    }

    #[test]
    fn duplicate_heading_fails() {
        let content = format!("{}\n## 변경 파일 목록\n\n- b.rs\n", complete_report());
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["duplicate-heading"]
        );
        let content = format!("{}\n## 구현 요약\n\n- 다시.\n", complete_report());
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["duplicate-heading"]
        );
        // 같은 제목이 세 번 나와도 오류는 하나입니다.
        let content = format!("{}\n## 비고\n\n## 비고\n\n## 비고\n", complete_report());
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["duplicate-heading"]
        );
    }

    #[test]
    fn command_sections_with_distinct_qualifiers_are_not_duplicates() {
        let content = format!(
            "{}\n## 실행한 명령 / 검증 (`CI-MAC`)\n\n| 명령 | 결과 |\n|---|---|\n| `cargo test` | 통과 |\n",
            complete_report()
        );
        assert!(validate_report(&content, ReportKind::Complete, ID).is_empty());
    }

    #[test]
    fn headings_inside_code_fence_are_ignored() {
        let content = complete_report().replace(
            "## 필수 실행 환경\n\n- `DEV-MAC`.\n",
            "```text\n## 필수 실행 환경\n\n- `DEV-MAC`.\n```\n",
        );
        let found = codes(&content, ReportKind::Complete, ID);
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
            let found = codes(&content, ReportKind::Complete, ID);
            assert!(found.contains(&"missing-heading"), "{fenced:?}: {found:?}");
            assert!(
                found.contains(&"missing-environment"),
                "{fenced:?}: {found:?}"
            );
        }

        // 더 긴 닫는 줄은 펜스를 닫고, 그 뒤 절은 다시 보입니다.
        let content =
            complete_report().replace(section, &format!("```\n본문\n````` \t\n{section}"));
        assert!(validate_report(&content, ReportKind::Complete, ID).is_empty());

        // backtick이 정보 문자열에 있는 줄(인라인 코드)은 펜스를 열지 않습니다.
        let content = complete_report().replace(
            "- 작업명: 제목\n",
            "- 작업명: 제목\n```inline``` 코드 설명\n",
        );
        assert!(validate_report(&content, ReportKind::Complete, ID).is_empty());
    }

    #[test]
    fn missing_wbs_fails() {
        let content = complete_report().replace("- WBS: VI.9.5", "- WBS:");
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-wbs"]
        );
        // 다른 ID를 가리키는 보고서도 실패합니다.
        assert_eq!(
            codes(&complete_report(), ReportKind::Complete, "VI.9.6"),
            vec!["missing-wbs"]
        );
    }

    #[test]
    fn wbs_outside_wbs_record_does_not_count() {
        // 머리 목록·`## WBS` 절 밖의 `WBS:` 줄은 WBS 기록으로 세지 않습니다.
        let content = complete_report().replace("- WBS: VI.9.5\n", "").replace(
            "## 차이(작업 지시와 다른 점)\n\n- 없음.\n",
            "## 차이(작업 지시와 다른 점)\n\n- WBS: VI.9.5\n",
        );
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-heading", "missing-wbs"]
        );
    }

    #[test]
    fn missing_environment_evidence_fails() {
        let content = complete_report().replace("- `DEV-MAC`.", "- 로컬.");
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-environment"]
        );
    }

    #[test]
    fn environment_code_outside_environment_section_does_not_count() {
        // 명령 절 heading에 `DEV-MAC`이 있어도 환경 절이 비면 실패합니다.
        let content = complete_report().replace("- `DEV-MAC`.", "- 미정.");
        assert!(content.contains("(`DEV-MAC`)"));
        assert_eq!(
            codes(&content, ReportKind::Complete, ID),
            vec!["missing-environment"]
        );
    }

    #[test]
    fn status_line_rules() {
        let partial = complete_report().replace("완료 (`[x]`)", "부분 완료 (`[~]`) — CI 대기");
        assert!(validate_report(&partial, ReportKind::Complete, ID).is_empty());

        let missing = complete_report().replace("- 상태: 완료 (`[x]`)\n", "");
        assert_eq!(
            codes(&missing, ReportKind::Complete, ID),
            vec!["missing-status"]
        );

        let duplicate =
            complete_report().replace("- 선행: 없음\n", "- 선행: 없음\n- 상태: 완료 (`[x]`)\n");
        assert_eq!(
            codes(&duplicate, ReportKind::Complete, ID),
            vec!["duplicate-status"]
        );

        for status in ["미착수 (`[ ]`)", "차단 (`[!]`)", "완료"] {
            let content = complete_report().replace("완료 (`[x]`)", status);
            assert_eq!(
                codes(&content, ReportKind::Complete, ID),
                vec!["invalid-status"],
                "{status}"
            );
        }

        // 첫 상태 표기가 기준입니다(뒤의 선행 상태 표기는 무시).
        let first_wins = complete_report().replace("완료 (`[x]`)", "`[~]` (선행 I.1.0 `[x]`)");
        assert!(validate_report(&first_wins, ReportKind::Complete, ID).is_empty());
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
                codes(&content, ReportKind::Complete, ID),
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
            codes(&moved, ReportKind::Complete, ID),
            vec!["missing-command-table"]
        );

        // 검사 표 하나만 있어도 통과합니다.
        let checks_only = replaced("| 검사 | 결과 |\n|:--|--:|\n| fixture | 통과 |\n");
        assert!(validate_report(&checks_only, ReportKind::Complete, ID).is_empty());
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
        // 차단 보고서는 `## WBS` 절이 필수입니다(머리 목록으로 대신하지 않음).
        let content = blocked_report().replace("## WBS\n\n", "");
        assert_eq!(
            codes(&content, ReportKind::Blocked, "I.2.4"),
            vec!["missing-heading"]
        );
        // 차단 보고서는 명령 표·환경 절을 요구하지 않지만, 완료 보고서로 검사하면 실패합니다.
        assert!(!codes(&blocked_report(), ReportKind::Complete, "I.2.4").is_empty());
    }

    #[test]
    fn report_template_fixes_i_2_13_sections() {
        // I.2.13 검증: TEMPLATE.md에 template 절 heading이 정확히 한 번씩, 이 순서로 있습니다.
        let template = include_str!("../../../../docs/task-reports/TEMPLATE.md");
        let lines = visible_lines(template);
        let titles: Vec<&str> = ParsedReport::parse(&lines)
            .sections
            .iter()
            .map(|section| section.title)
            .collect();
        assert_eq!(titles, I_2_13_TEMPLATE_HEADINGS);
        // validator 규칙의 대표 제목도 template 제목과 같습니다.
        let names: Vec<&str> = COMPLETE_SECTIONS.iter().map(|rule| rule.name).collect();
        assert_eq!(names, I_2_13_TEMPLATE_HEADINGS);
        // 절 구성은 통과하고, 채우지 않은 placeholder만 실패 원인으로 남습니다.
        assert_eq!(
            codes(&template.replace("<WBS-ID>", ID), ReportKind::Complete, ID),
            vec!["unfilled-placeholder"]
        );

        // I.2.14 차단 보고서 template도 validator 규칙과 같은 절 구성입니다.
        let blocked = include_str!("../../../../docs/task-reports/BLOCKED_TEMPLATE.md");
        let lines = visible_lines(blocked);
        let titles: Vec<&str> = ParsedReport::parse(&lines)
            .sections
            .iter()
            .map(|section| section.title)
            .collect();
        let names: Vec<&str> = BLOCKED_SECTIONS.iter().map(|rule| rule.name).collect();
        assert_eq!(titles, names);
        assert_eq!(
            codes(
                &blocked.replace("<WBS-ID>", "I.2.4"),
                ReportKind::Blocked,
                "I.2.4"
            ),
            vec!["unfilled-placeholder"]
        );
    }

    #[test]
    fn unfilled_template_placeholders_fail() {
        // 목록 항목, `키: 값` 목록 항목의 값, 표 셀이 template placeholder 하나뿐이면 실패합니다.
        for (from, to) in [
            (
                "## 차이(작업 지시와 다른 점)\n\n- 없음.\n",
                "## 차이(작업 지시와 다른 점)\n\n- <작업 지시와 다른 점 또는 없음>\n",
            ),
            ("- 작업명: 제목\n", "- 작업명: <작업 내용 한 줄>\n"),
            (
                "- 상태: 완료 (`[x]`)\n",
                "- 상태: <완료 (`[x]`) | 부분 완료 (`[~]`) — 미완료 사유>\n",
            ),
            (
                "- `DEV-MAC`.\n",
                "- 필수 실행 환경: <환경 코드: DEV-MAC | CI-MAC>\n",
            ),
            ("| fixture | 통과 |", "| fixture | <통과 또는 실패> |"),
            (
                "| `cargo test -p xtask` | 10 passed, 0 failed |",
                "| `<command>` | 10 passed, 0 failed |",
            ),
        ] {
            let content = complete_report().replace(from, to);
            assert_ne!(content, complete_report(), "{from:?}");
            let errors = validate_report(&content, ReportKind::Complete, ID);
            assert_eq!(
                errors.iter().map(|e| e.code).collect::<Vec<_>>(),
                vec!["unfilled-placeholder"],
                "{to:?}"
            );
        }

        // 오류는 하나이고 placeholder가 남은 줄 번호를 모두 담습니다.
        let content = complete_report()
            .replace("- 작업명: 제목\n", "- 작업명: <제목>\n")
            .replace("- 요약.\n", "- <요약>\n");
        let errors = validate_report(&content, ReportKind::Complete, ID);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("줄 4, 18"), "{errors:?}");

        // 문장·인라인 코드 속 `<...>` 표기, autolink, 코드 펜스 안의 placeholder는 실패하지 않습니다.
        for summary in [
            "- `inspect <경로>` 명령을 추가했습니다.\n",
            "- 커밋 제목 `Task(<WBS-ID>): <제목>` 형식.\n",
            "- `Vec<u8>`\n",
            "- <https://example.com/runs/1>\n",
            "- <mailto:dev@example.com>\n",
            "- 범위 <경로> 안\n",
            "```text\n- <제목>\n| <a> | <b> |\n```\n",
        ] {
            let content = complete_report().replace("- 요약.\n", summary);
            assert!(
                validate_report(&content, ReportKind::Complete, ID).is_empty(),
                "{summary:?}"
            );
        }
    }

    #[test]
    fn repository_reports_pass() {
        // 저장소의 모든 완료·차단 보고서(template 제외)가 validator를 통과합니다.
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(reports_dir())
            .expect("docs/task-reports 디렉터리")
            .map(|entry| entry.expect("디렉터리 항목").path())
            .collect();
        paths.sort();
        let mut checked = 0usize;
        let mut failures = Vec::new();
        for path in paths {
            let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(stem) = file.strip_suffix(".md") else {
                continue;
            };
            if stem == "TEMPLATE" || stem == "BLOCKED_TEMPLATE" {
                continue;
            }
            let (kind, id) = match stem.strip_suffix("-blocked") {
                Some(id) => (ReportKind::Blocked, id),
                None => (ReportKind::Complete, stem),
            };
            let content = std::fs::read_to_string(&path).expect("보고서 읽기");
            let errors = validate_report(&content, kind, id);
            if !errors.is_empty() {
                let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
                failures.push(format!("{file}: {messages:?}"));
            }
            checked += 1;
        }
        assert!(checked > 0, "검사한 보고서가 없습니다");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn legacy_reports_lack_risk_section() {
        // 고정 목록은 남은 위험 절이 실제로 없는 초기 보고서만 담습니다.
        for id in LEGACY_REPORTS_WITHOUT_RISK_SECTION {
            let path = reports_dir().join(format!("{id}.md"));
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let lines = visible_lines(&content);
            let report = ParsedReport::parse(&lines);
            assert!(
                report.sections_of(&RISK_SECTION).next().is_none(),
                "{id}는 남은 위험 절이 있으므로 고정 목록에서 뺍니다"
            );
        }
    }
}
