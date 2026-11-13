//! 남은 위험 절 없이 통과하는 초기 완료 보고서 목록.
//!
//! 이 보고서들은 `docs/task-reports/TEMPLATE.md`의 `## 남은 위험` 절이 보고서 관례가 되기 전에
//! 계획서 §1.2 규칙 11의 기록(변경 파일 목록, 명령, 검증 환경, 테스트 결과와 남은 경고 수, 작업
//! 지시와 다른 점)만으로 작성되었습니다. 완료된 보고서를 소급 수정하지 않도록 이 목록의 ID만 남은
//! 위험 절 없이 통과시킵니다. 목록은 `1c461c2` 시점의 보고서로 고정하며 새 보고서는 추가하지
//! 않습니다. 목록의 보고서에 남은 위험 절을 추가하면 그 ID를 목록에서 뺍니다(테스트가 확인).

/// 남은 위험 절 없이 통과하는 완료 보고서 WBS ID(WBS 순서).
pub(super) const LEGACY_REPORTS_WITHOUT_RISK_SECTION: &[&str] = &[
    "I.1.1", "I.1.2", "I.1.3", "I.1.4", "I.1.5", "I.1.6", "I.1.7", "I.1.8", "I.1.9", "I.1.10",
    "I.2.1", "I.2.2", "I.2.3", "I.2.4", "I.2.5", "I.2.6", "I.2.7", "I.2.8", "I.2.9", "I.2.10",
    "I.2.11", "I.2.12", "I.2.13", "I.2.14", "I.2.15", "I.2.16", "I.2.17", "I.3.1", "I.3.2",
    "I.3.3", "I.3.4", "I.3.5", "I.4.1", "I.4.2", "I.4.3", "II.1.1", "II.1.2", "II.1.3", "II.1.4",
    "II.1.5", "II.1.6", "II.2.1", "II.2.2", "II.2.3", "II.2.4", "II.2.5", "II.2.6", "II.2.7",
    "II.2.8", "II.3.1", "II.3.2", "II.3.3", "II.3.4", "II.4.1", "II.4.2", "II.4.3", "II.4.4",
    "II.4.5", "II.4.6", "II.5.1", "II.5.2", "II.5.3", "II.5.4", "III.1.1", "III.1.2", "III.1.3",
    "III.1.4", "III.1.5", "III.2.1", "III.2.2", "III.2.3", "III.2.4", "III.3.1", "III.3.2",
    "III.3.3",
];

/// `id` 보고서가 남은 위험 절 없이 통과하는 초기 보고서인지 확인합니다.
pub(super) fn is_legacy_report_without_risk_section(id: &str) -> bool {
    LEGACY_REPORTS_WITHOUT_RISK_SECTION.contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::wbs_id::WbsId;

    #[test]
    fn legacy_ids_are_canonical_and_in_wbs_order() {
        let ids: Vec<WbsId> = LEGACY_REPORTS_WITHOUT_RISK_SECTION
            .iter()
            .map(|text| {
                let id = WbsId::parse(text).unwrap_or_else(|| panic!("WBS ID 아님: {text}"));
                assert_eq!(id.to_string(), *text, "정규 표기가 아닙니다");
                id
            })
            .collect();
        assert!(
            ids.windows(2).all(|pair| pair[0] < pair[1]),
            "WBS 순서가 아니거나 중복이 있습니다"
        );
        assert!(is_legacy_report_without_risk_section("I.1.1"));
        assert!(!is_legacy_report_without_risk_section("VI.9.5"));
    }
}
