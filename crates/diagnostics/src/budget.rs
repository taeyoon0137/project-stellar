//! 상호작용 성능 예산 reader (XVI.4.1, ADR-0084 결정 10).
//!
//! `config/performance-budgets.yaml`의 `interaction:` 절을 읽습니다. 값은 아키텍처 §6.2~§6.4를 그대로 옮긴
//! 정수 µs입니다. 다른 최상위 절(color, storage, autosave)은 건너뛰고 이 절만 엄격하게 검사합니다.
//!
//! - 절은 정확히 한 번, 항목은 공백 2칸 들여쓰기 `key: value` 한 줄씩입니다(tab·중첩 금지).
//! - 알 수 없는 key·중복 key·누락 key를 거부합니다. 값은 10진 0 이상 정수입니다(부호·밑줄·소수점·단위·선행 0 거부).
//! - 단계 목표 합 = `total_target_us`, 단계 상한 합 = `total_limit_us`, `p50 ≤ p95 ≤ p99 ≤ controllable_max`를
//!   검사합니다.
//!
//! # T0~T5 구간 대응
//!
//! §6.2의 9개 단계 행을 T 구간에 이렇게 묶습니다([`InteractionBudget::span_target_us`]).
//!
//! | 구간 | §6.2 행 |
//! |---|---|
//! | T0 → T1 | 입력 전달 및 이벤트 병합 + Hit Test 및 인터랙션 해석 |
//! | T1 → T2 | Command 생성 및 문서 변경 |
//! | T2 → T3 | 의존성·토큰·Binding 재평가 + 증분 레이아웃 |
//! | T3 → T4 | Scene Snapshot 갱신 + Render Graph 및 명령 인코딩 |
//! | T4 → T5 | GPU 제출 및 Present 예약 |
//!
//! 스케줄링·메모리 여유 행은 구간에 넣지 않고 합계에만 들어갑니다.

use std::fmt;

use crate::interaction::StageSpan;

/// 예산 파일에서 상호작용 항목을 담는 최상위 절 이름.
pub const INTERACTION_BUDGET_SECTION: &str = "interaction";

/// 단계 행(§6.2 순서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BudgetStage {
    /// 입력 전달 및 이벤트 병합.
    Input,
    /// Hit Test 및 인터랙션 해석.
    HitTest,
    /// Command 생성 및 문서 변경.
    Command,
    /// 의존성·토큰·Binding 재평가.
    Binding,
    /// 증분 레이아웃.
    Layout,
    /// Scene Snapshot 갱신.
    Scene,
    /// Render Graph 및 명령 인코딩.
    RenderGraph,
    /// GPU 제출 및 Present 예약.
    Present,
    /// 스케줄링·메모리 여유.
    Scheduling,
}

impl BudgetStage {
    /// 모든 행(§6.2 순서).
    pub const ALL: [BudgetStage; 9] = [
        BudgetStage::Input,
        BudgetStage::HitTest,
        BudgetStage::Command,
        BudgetStage::Binding,
        BudgetStage::Layout,
        BudgetStage::Scene,
        BudgetStage::RenderGraph,
        BudgetStage::Present,
        BudgetStage::Scheduling,
    ];

    /// yaml key 접두사.
    pub const fn key(self) -> &'static str {
        match self {
            BudgetStage::Input => "input",
            BudgetStage::HitTest => "hit_test",
            BudgetStage::Command => "command",
            BudgetStage::Binding => "binding",
            BudgetStage::Layout => "layout",
            BudgetStage::Scene => "scene",
            BudgetStage::RenderGraph => "render_graph",
            BudgetStage::Present => "present",
            BudgetStage::Scheduling => "scheduling",
        }
    }

    /// 아키텍처 §6.2 표의 행 이름.
    pub const fn architecture_label(self) -> &'static str {
        match self {
            BudgetStage::Input => "입력 전달 및 이벤트 병합",
            BudgetStage::HitTest => "Hit Test 및 인터랙션 해석",
            BudgetStage::Command => "Command 생성 및 문서 변경",
            BudgetStage::Binding => "의존성·토큰·Binding 재평가",
            BudgetStage::Layout => "증분 레이아웃",
            BudgetStage::Scene => "Scene Snapshot 갱신",
            BudgetStage::RenderGraph => "Render Graph 및 명령 인코딩",
            BudgetStage::Present => "GPU 제출 및 Present 예약",
            BudgetStage::Scheduling => "스케줄링·메모리 여유",
        }
    }

    /// 이 행을 담는 T 구간(스케줄링 여유는 없음).
    pub const fn span(self) -> Option<StageSpan> {
        match self {
            BudgetStage::Input | BudgetStage::HitTest => Some(StageSpan::Input),
            BudgetStage::Command => Some(StageSpan::Command),
            BudgetStage::Binding | BudgetStage::Layout => Some(StageSpan::Layout),
            BudgetStage::Scene | BudgetStage::RenderGraph => Some(StageSpan::Scene),
            BudgetStage::Present => Some(StageSpan::Present),
            BudgetStage::Scheduling => None,
        }
    }
}

/// 상호작용 성능 예산(µs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionBudget {
    /// 단계별 (목표, 상한), [`BudgetStage::ALL`] 순서.
    pub stages: [(u64, u64); 9],
    /// 합계 목표.
    pub total_target_us: u64,
    /// 합계 상한.
    pub total_limit_us: u64,
    /// p50 판정 기준.
    pub p50_us: u64,
    /// p95 판정 기준.
    pub p95_us: u64,
    /// p99 판정 기준.
    pub p99_us: u64,
    /// 통제 가능한 단일 인터랙션 상한.
    pub controllable_max_us: u64,
    /// 인터랙티브 GPU 실행 시간 p99.
    pub gpu_p99_us: u64,
    /// Present 대기열 깊이 상한(frame).
    pub present_queue_depth_max: u64,
    /// UI·문서 스레드 단일 점유 상한.
    pub thread_occupancy_max_us: u64,
    /// Hover·선택·핸들 표시 목표(§6.4).
    pub hover_target_us: u64,
    /// Hover·선택·핸들 표시 상한(§6.4).
    pub hover_limit_us: u64,
    /// release gate가 시나리오마다 요구하는 최소 판정 sample 수(XVI.2.5, ADR-0096 결정 6).
    pub scenario_min_samples: u64,
}

/// 단계 외 key(순서 고정).
const SCALAR_KEYS: [&str; 12] = [
    "total_target_us",
    "total_limit_us",
    "p50_us",
    "p95_us",
    "p99_us",
    "controllable_max_us",
    "gpu_p99_us",
    "present_queue_depth_max",
    "thread_occupancy_max_us",
    "hover_target_us",
    "hover_limit_us",
    "scenario_min_samples",
];

impl InteractionBudget {
    /// 행의 (목표, 상한).
    pub const fn stage(&self, stage: BudgetStage) -> (u64, u64) {
        self.stages[stage as usize]
    }

    /// T 구간의 목표 합(µs).
    pub fn span_target_us(&self, span: StageSpan) -> u64 {
        BudgetStage::ALL
            .iter()
            .filter(|stage| stage.span() == Some(span))
            .map(|stage| self.stage(*stage).0)
            .sum()
    }

    /// T 구간의 상한 합(µs).
    pub fn span_limit_us(&self, span: StageSpan) -> u64 {
        BudgetStage::ALL
            .iter()
            .filter(|stage| stage.span() == Some(span))
            .map(|stage| self.stage(*stage).1)
            .sum()
    }

    /// 파일 텍스트에서 읽습니다.
    pub fn parse(text: &str) -> Result<InteractionBudget, BudgetError> {
        let mut keys: Vec<String> = Vec::new();
        for stage in BudgetStage::ALL {
            keys.push(format!("{}_target_us", stage.key()));
            keys.push(format!("{}_limit_us", stage.key()));
        }
        keys.extend(SCALAR_KEYS.iter().map(|k| (*k).to_owned()));
        let values = parse_section(text, INTERACTION_BUDGET_SECTION, &keys)?;
        let mut stages = [(0u64, 0u64); 9];
        for (i, slot) in stages.iter_mut().enumerate() {
            *slot = (values[2 * i], values[2 * i + 1]);
        }
        let s = &values[18..];
        let budget = InteractionBudget {
            stages,
            total_target_us: s[0],
            total_limit_us: s[1],
            p50_us: s[2],
            p95_us: s[3],
            p99_us: s[4],
            controllable_max_us: s[5],
            gpu_p99_us: s[6],
            present_queue_depth_max: s[7],
            thread_occupancy_max_us: s[8],
            hover_target_us: s[9],
            hover_limit_us: s[10],
            scenario_min_samples: s[11],
        };
        budget.check()?;
        Ok(budget)
    }

    fn check(&self) -> Result<(), BudgetError> {
        let targets: u64 = self.stages.iter().map(|s| s.0).sum();
        let limits: u64 = self.stages.iter().map(|s| s.1).sum();
        if targets != self.total_target_us {
            return Err(BudgetError::Inconsistent {
                reason: "단계 목표 합이 total_target_us와 다릅니다",
            });
        }
        if limits != self.total_limit_us {
            return Err(BudgetError::Inconsistent {
                reason: "단계 상한 합이 total_limit_us와 다릅니다",
            });
        }
        if self.stages.iter().any(|(target, limit)| target > limit) {
            return Err(BudgetError::Inconsistent {
                reason: "단계 목표가 상한보다 큽니다",
            });
        }
        if !(self.p50_us <= self.p95_us
            && self.p95_us <= self.p99_us
            && self.p99_us <= self.controllable_max_us)
        {
            return Err(BudgetError::Inconsistent {
                reason: "p50 ≤ p95 ≤ p99 ≤ controllable_max가 아닙니다",
            });
        }
        if self.scenario_min_samples == 0 {
            return Err(BudgetError::Inconsistent {
                reason: "scenario_min_samples는 1 이상입니다",
            });
        }
        if self.hover_target_us > self.hover_limit_us {
            return Err(BudgetError::Inconsistent {
                reason: "hover 목표가 상한보다 큽니다",
            });
        }
        Ok(())
    }
}

/// 예산 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// 절이 없습니다.
    MissingSection,
    /// 절이 두 번 이상 나타납니다.
    DuplicateSection {
        /// 줄 번호(1부터).
        line: usize,
    },
    /// 필수 key가 없습니다.
    MissingKey {
        /// key.
        key: String,
    },
    /// 같은 key가 두 번 나타납니다.
    DuplicateKey {
        /// key.
        key: String,
        /// 줄 번호.
        line: usize,
    },
    /// 알 수 없는 key.
    UnknownKey {
        /// key.
        key: String,
        /// 줄 번호.
        line: usize,
    },
    /// 값이 10진 0 이상 정수가 아닙니다.
    InvalidValue {
        /// key.
        key: String,
        /// 원본.
        value: String,
    },
    /// 줄 형식 오류.
    MalformedLine {
        /// 줄 번호.
        line: usize,
    },
    /// 값 사이 관계가 맞지 않습니다.
    Inconsistent {
        /// 이유.
        reason: &'static str,
    },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BudgetError::MissingSection => write!(
                f,
                "performance-budgets에 {INTERACTION_BUDGET_SECTION}: 절이 없습니다"
            ),
            BudgetError::DuplicateSection { line } => write!(
                f,
                "performance-budgets {line}행: {INTERACTION_BUDGET_SECTION}: 절이 중복됩니다"
            ),
            BudgetError::MissingKey { key } => write!(f, "{key} 값이 없습니다"),
            BudgetError::DuplicateKey { key, line } => write!(f, "{line}행: {key}가 중복됩니다"),
            BudgetError::UnknownKey { key, line } => write!(f, "{line}행: 알 수 없는 key {key}"),
            BudgetError::InvalidValue { key, value } => {
                write!(f, "{key} 값 {value}는 10진 0 이상 정수가 아닙니다")
            }
            BudgetError::MalformedLine { line } => write!(f, "{line}행 형식이 잘못되었습니다"),
            BudgetError::Inconsistent { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for BudgetError {}

/// 줄 끝 주석을 지운 내용.
fn strip_comment(line: &str) -> &str {
    match line.find(" #") {
        Some(index) => &line[..index],
        None => line,
    }
}

fn parse_value(key: &str, value: &str) -> Result<u64, BudgetError> {
    let invalid = || BudgetError::InvalidValue {
        key: key.to_owned(),
        value: value.to_owned(),
    };
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(invalid());
    }
    value.parse::<u64>().map_err(|_| invalid())
}

/// `section:` 절에서 `keys` 순서의 값을 읽습니다.
fn parse_section(text: &str, section: &str, keys: &[String]) -> Result<Vec<u64>, BudgetError> {
    let header = format!("{section}:");
    let mut values: Vec<Option<u64>> = vec![None; keys.len()];
    let mut inside = false;
    let mut found = false;
    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let content = strip_comment(raw).trim_end();
        if content.trim_start().starts_with('#') || content.trim().is_empty() {
            continue;
        }
        if !content.starts_with(' ') && !content.starts_with('\t') {
            // 최상위 줄.
            inside = content == header;
            if inside {
                if found {
                    return Err(BudgetError::DuplicateSection { line });
                }
                found = true;
            } else if content.starts_with(&header) {
                return Err(BudgetError::MalformedLine { line });
            }
            continue;
        }
        if !inside {
            continue;
        }
        let Some(entry) = content.strip_prefix("  ") else {
            return Err(BudgetError::MalformedLine { line });
        };
        if entry.starts_with(' ') || entry.contains('\t') {
            return Err(BudgetError::MalformedLine { line });
        }
        let Some((key, value)) = entry.split_once(": ") else {
            return Err(BudgetError::MalformedLine { line });
        };
        let Some(slot) = keys.iter().position(|k| k == key) else {
            return Err(BudgetError::UnknownKey {
                key: key.to_owned(),
                line,
            });
        };
        if values[slot].is_some() {
            return Err(BudgetError::DuplicateKey {
                key: key.to_owned(),
                line,
            });
        }
        values[slot] = Some(parse_value(key, value.trim())?);
    }
    if !found {
        return Err(BudgetError::MissingSection);
    }
    values
        .into_iter()
        .zip(keys)
        .map(|(value, key)| value.ok_or(BudgetError::MissingKey { key: key.clone() }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGETS: &str = include_str!("../../../config/performance-budgets.yaml");
    const ARCHITECTURE: &str =
        include_str!("../../../docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md");

    /// `"0.15ms"`·`"5ms 이하"` → µs.
    fn ms_to_us(text: &str) -> u64 {
        let number = text
            .trim()
            .trim_end_matches(" 이하")
            .trim_end_matches("ms")
            .trim();
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        let mut fraction = fraction.to_owned();
        while fraction.len() < 3 {
            fraction.push('0');
        }
        whole.parse::<u64>().unwrap() * 1000 + fraction.parse::<u64>().unwrap()
    }

    /// 문서 표 행 `| 이름 | 값 | 값 |`의 칸.
    fn table_row<'a>(doc: &'a str, label: &str) -> Vec<&'a str> {
        let prefix = format!("| {label} |");
        let line = doc
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("아키텍처 문서에 {label} 행이 없습니다"));
        line.trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().trim_matches('*'))
            .collect()
    }

    #[test]
    fn config_matches_architecture_section_six() {
        // 검증 기준(XVI.4.1): 문서 6장 수치와 자동 비교해 불일치 실패.
        let budget = InteractionBudget::parse(BUDGETS).unwrap();
        for stage in BudgetStage::ALL {
            let row = table_row(ARCHITECTURE, stage.architecture_label());
            assert_eq!(
                budget.stage(stage),
                (ms_to_us(row[1]), ms_to_us(row[2])),
                "{}",
                stage.key()
            );
        }
        let total = table_row(ARCHITECTURE, "**합계**");
        assert_eq!(budget.total_target_us, ms_to_us(total[1]));
        assert_eq!(budget.total_limit_us, ms_to_us(total[2]));
        assert_eq!(budget.p50_us, ms_to_us(table_row(ARCHITECTURE, "p50")[1]));
        assert_eq!(budget.p95_us, ms_to_us(table_row(ARCHITECTURE, "p95")[1]));
        assert_eq!(budget.p99_us, ms_to_us(table_row(ARCHITECTURE, "p99")[1]));
        assert_eq!(
            budget.controllable_max_us,
            ms_to_us(table_row(ARCHITECTURE, "통제 가능한 단일 인터랙션 상한")[1])
        );
        let gpu = table_row(ARCHITECTURE, "인터랙티브 GPU 실행 시간")[1];
        assert_eq!(budget.gpu_p99_us, ms_to_us(gpu.trim_start_matches("p99 ")));
        let queue = table_row(ARCHITECTURE, "Present 대기열 깊이")[1];
        assert_eq!(queue, "최대 1프레임");
        assert_eq!(budget.present_queue_depth_max, 1);
        let occupancy = table_row(ARCHITECTURE, "UI 또는 문서 스레드의 단일 점유 구간")[1];
        assert_eq!(
            budget.thread_occupancy_max_us,
            ms_to_us(occupancy.trim_start_matches("최대 "))
        );
        // §6.4 Hover·선택·핸들 표시: 목표 5ms, 상한 8ms.
        assert!(ARCHITECTURE.contains("- Hover, 선택, 핸들 표시: 목표 5ms, 상한 8ms"));
        assert_eq!(
            (budget.hover_target_us, budget.hover_limit_us),
            (5_000, 8_000)
        );
        assert!(
            ARCHITECTURE.contains("- Pan, Zoom, Drag, Resize, Rotate: Present 예약까지 10ms 이하")
        );
    }

    #[test]
    fn spans_group_stage_rows() {
        let budget = InteractionBudget::parse(BUDGETS).unwrap();
        assert_eq!(budget.span_target_us(StageSpan::Input), 500);
        assert_eq!(budget.span_limit_us(StageSpan::Input), 1_050);
        assert_eq!(budget.span_target_us(StageSpan::Layout), 1_600);
        assert_eq!(budget.span_target_us(StageSpan::Scene), 1_750);
        assert_eq!(budget.span_limit_us(StageSpan::Present), 500);
    }

    #[test]
    fn rejects_malformed_sections() {
        let good = BUDGETS;
        assert_eq!(
            InteractionBudget::parse("storage:\n  a: 1\n"),
            Err(BudgetError::MissingSection)
        );
        let dup = format!("{good}\ninteraction:\n  p50_us: 1\n");
        assert!(matches!(
            InteractionBudget::parse(&dup),
            Err(BudgetError::DuplicateSection { .. })
        ));
        let bad = good.replace("p95_us: 7000", "p95_us: 7_000");
        assert!(matches!(
            InteractionBudget::parse(&bad),
            Err(BudgetError::InvalidValue { .. })
        ));
        let unknown = good.replace("p95_us: 7000", "p95_us: 7000\n  p90_us: 6000");
        assert!(matches!(
            InteractionBudget::parse(&unknown),
            Err(BudgetError::UnknownKey { .. })
        ));
        let missing = good.replace("  p95_us: 7000\n", "");
        assert_eq!(
            InteractionBudget::parse(&missing),
            Err(BudgetError::MissingKey {
                key: "p95_us".to_owned()
            })
        );
        let zero = good.replace("scenario_min_samples: 1000", "scenario_min_samples: 0");
        assert!(matches!(
            InteractionBudget::parse(&zero),
            Err(BudgetError::Inconsistent { .. })
        ));
        assert_eq!(
            InteractionBudget::parse(good).unwrap().scenario_min_samples,
            1_000
        );
        let sum = good.replace("total_target_us: 5950", "total_target_us: 6000");
        assert!(matches!(
            InteractionBudget::parse(&sum),
            Err(BudgetError::Inconsistent { .. })
        ));
    }
}
