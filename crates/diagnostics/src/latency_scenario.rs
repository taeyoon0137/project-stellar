//! 성능 게이트 시나리오 목록과 cache setup 판정 (XVI.2.5, XVI.2.6, ADR-0096 결정 3·4·5).
//!
//! - **시나리오**(XVI.2.5): 아키텍처 §38.1의 결정적 성능 게이트 시나리오 13개입니다. 시나리오 하나가
//!   [`InteractionKind`] 하나입니다. 모두 required이고 시나리오마다 판정에 쓰는 sample이
//!   `scenario_min_samples`(performance-budgets, 1,000) 이상이어야 합니다.
//! - **core 경로**: core replay(UI 없이 command → layout → scene → render graph)가 실행할 수 있는 시나리오와 아직
//!   엔진 경로가 문서 → scene에 이어지지 않은 시나리오를 [`CoreReplay`]로 구분합니다. 이어지지 않은 시나리오는
//!   재생 파일을 만들지 않고, release gate에서는 sample 0개라 실패합니다(빼지 않음).
//! - **cache setup**(XVI.2.6): `warm`은 측정 전 setup event로 cache를 채운 상태에서, `cold`는 측정 시작 직전에
//!   cache를 비운 상태에서 측정합니다. 측정 구간의 실제 hit·miss 수([`CacheObservation`])가 선언한 setup과 맞는지
//!   [`check_cache_setup`]이 판정합니다. 맞지 않는 결과는 gate에서 실패합니다.

use std::fmt;

use crate::interaction::{CacheState, InteractionKind};

/// §38.1 성능 게이트 시나리오.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LatencyScenario {
    /// Pan.
    Pan,
    /// Zoom.
    Zoom,
    /// Drag.
    Drag,
    /// Resize.
    Resize,
    /// Rotate.
    Rotate,
    /// Hover.
    Hover,
    /// Select.
    Select,
    /// Text Edit.
    TextEdit,
    /// Token Propagation.
    TokenPropagation,
    /// Component Update(중첩 컴포넌트 갱신).
    ComponentUpdate,
    /// Variant Change(View Variant 변경).
    VariantChange,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
}

/// core replay가 시나리오를 실행할 수 있는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreReplay {
    /// core replay가 실행합니다.
    Supported,
    /// 엔진 경로가 아직 문서 → scene에 이어지지 않았습니다.
    NotConnected {
        /// 빠진 연결.
        missing: &'static str,
    },
}

impl LatencyScenario {
    /// 모든 시나리오(§38.1 순서).
    pub const ALL: [LatencyScenario; 13] = [
        LatencyScenario::Pan,
        LatencyScenario::Zoom,
        LatencyScenario::Drag,
        LatencyScenario::Resize,
        LatencyScenario::Rotate,
        LatencyScenario::Hover,
        LatencyScenario::Select,
        LatencyScenario::TextEdit,
        LatencyScenario::TokenPropagation,
        LatencyScenario::ComponentUpdate,
        LatencyScenario::VariantChange,
        LatencyScenario::Undo,
        LatencyScenario::Redo,
    ];

    /// trace dimension의 상호작용 종류(1:1).
    pub const fn kind(self) -> InteractionKind {
        match self {
            LatencyScenario::Pan => InteractionKind::Pan,
            LatencyScenario::Zoom => InteractionKind::Zoom,
            LatencyScenario::Drag => InteractionKind::Drag,
            LatencyScenario::Resize => InteractionKind::Resize,
            LatencyScenario::Rotate => InteractionKind::Rotate,
            LatencyScenario::Hover => InteractionKind::Hover,
            LatencyScenario::Select => InteractionKind::Select,
            LatencyScenario::TextEdit => InteractionKind::TextEdit,
            LatencyScenario::TokenPropagation => InteractionKind::TokenPropagation,
            LatencyScenario::ComponentUpdate => InteractionKind::ComponentUpdate,
            LatencyScenario::VariantChange => InteractionKind::VariantChange,
            LatencyScenario::Undo => InteractionKind::Undo,
            LatencyScenario::Redo => InteractionKind::Redo,
        }
    }

    /// 재생 파일·CLI 이름(소문자·`-`).
    pub const fn name(self) -> &'static str {
        match self {
            LatencyScenario::Pan => "pan",
            LatencyScenario::Zoom => "zoom",
            LatencyScenario::Drag => "drag",
            LatencyScenario::Resize => "resize",
            LatencyScenario::Rotate => "rotate",
            LatencyScenario::Hover => "hover",
            LatencyScenario::Select => "select",
            LatencyScenario::TextEdit => "text-edit",
            LatencyScenario::TokenPropagation => "token-propagation",
            LatencyScenario::ComponentUpdate => "component-update",
            LatencyScenario::VariantChange => "variant-change",
            LatencyScenario::Undo => "undo",
            LatencyScenario::Redo => "redo",
        }
    }

    /// 이름에서 찾습니다.
    pub fn parse(text: &str) -> Option<LatencyScenario> {
        LatencyScenario::ALL.into_iter().find(|s| s.name() == text)
    }

    /// release gate required 여부. §38.1의 시나리오는 모두 required입니다.
    pub const fn required(self) -> bool {
        true
    }

    /// core replay 실행 가능 여부.
    pub const fn core_replay(self) -> CoreReplay {
        match self {
            LatencyScenario::TextEdit => CoreReplay::NotConnected {
                missing:
                    "Text node의 scene 입력(text layout hook)이 runtime 밖에 공개되지 않았습니다",
            },
            LatencyScenario::TokenPropagation => CoreReplay::NotConnected {
                missing: "binding 평가 결과를 문서 node 값과 scene에 쓰는 경로가 없습니다",
            },
            LatencyScenario::ComponentUpdate => CoreReplay::NotConnected {
                missing: "컴포넌트 정의 편집을 command change set으로 내는 경로가 없습니다",
            },
            _ => CoreReplay::Supported,
        }
    }
}

/// scene build 한 번의 cache 사용량.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheBuild {
    /// 이전 결과를 그대로 쓴 node 수(hit).
    pub hits: u64,
    /// 다시 평가한 node 수(miss).
    pub misses: u64,
    /// 이전 결과 없이 처음부터 만든 build인지.
    pub full: bool,
}

/// 측정 구간에서 관측한 cache 사용량.
///
/// core replay의 cache는 증분 scene build의 node 결과입니다. 이전 결과를 그대로 쓴 node가 hit, 다시 평가한 node가
/// miss입니다. GPU 쪽 cache(glyph atlas, texture)는 macOS full replay(XVI.2.3)가 같은 형식으로 더합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheObservation {
    /// 측정 구간 전체 hit.
    pub hits: u64,
    /// 측정 구간 전체 miss.
    pub misses: u64,
    /// 측정 구간의 처음부터 만든 build 수.
    pub full_builds: u64,
    /// 측정 구간 첫 build(측정 시작 상태). build가 없으면 `None`.
    pub first: Option<CacheBuild>,
}

impl CacheObservation {
    /// build 하나를 더합니다.
    pub fn record(&mut self, build: CacheBuild) {
        self.hits = self.hits.saturating_add(build.hits);
        self.misses = self.misses.saturating_add(build.misses);
        if build.full {
            self.full_builds = self.full_builds.saturating_add(1);
        }
        if self.first.is_none() {
            self.first = Some(build);
        }
    }
}

/// cache setup 불일치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSetupMismatch {
    /// cold인데 측정 구간에 cache 조회가 없어 비운 상태를 확인할 수 없습니다.
    NoLookups,
    /// cold인데 측정 시작 build가 이전 결과를 썼습니다.
    ColdStartHit {
        /// hit 수.
        hits: u64,
    },
    /// cold인데 측정 시작 build가 처음부터 만든 build가 아닙니다.
    ColdStartNotFull,
    /// warm인데 측정 구간에서 처음부터 만든 build가 있습니다.
    WarmFullBuild {
        /// 처음부터 만든 build 수.
        builds: u64,
    },
    /// warm인데 측정 시작 build가 이전 결과를 하나도 쓰지 않았습니다.
    WarmStartMissed {
        /// miss 수.
        misses: u64,
    },
}

impl fmt::Display for CacheSetupMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheSetupMismatch::NoLookups => {
                f.write_str("cold setup인데 측정 구간에 cache 조회가 없습니다")
            }
            CacheSetupMismatch::ColdStartHit { hits } => {
                write!(f, "cold setup인데 측정 시작 build에서 hit {hits}개")
            }
            CacheSetupMismatch::ColdStartNotFull => {
                f.write_str("cold setup인데 측정 시작 build가 처음부터 만든 build가 아닙니다")
            }
            CacheSetupMismatch::WarmFullBuild { builds } => {
                write!(f, "warm setup인데 처음부터 만든 build {builds}개")
            }
            CacheSetupMismatch::WarmStartMissed { misses } => {
                write!(
                    f,
                    "warm setup인데 측정 시작 build가 hit 없이 miss {misses}개"
                )
            }
        }
    }
}

impl std::error::Error for CacheSetupMismatch {}

/// 선언한 cache setup이 실제 hit·miss와 맞는지 판정합니다(XVI.2.6).
///
/// - `cold`: 측정 시작 build가 처음부터 만든 build이고 hit 0, miss 1 이상이어야 합니다(cache가 비어 있었음).
/// - `warm`: 측정 구간에 처음부터 만든 build가 없고, 측정 시작 build가 있으면 hit가 1 이상이어야 합니다(setup이 채운
///   결과를 썼음). 측정 중 편집이 만든 miss는 setup과 무관하므로 셉니다만 판정하지 않습니다.
/// - 측정 구간에 build가 하나도 없으면(예: pan만 하는 warm 측정) warm은 맞고, cold는 확인할 수 없습니다.
pub fn check_cache_setup(
    state: CacheState,
    observed: &CacheObservation,
) -> Result<(), CacheSetupMismatch> {
    match state {
        CacheState::Cold => {
            let first = observed.first.ok_or(CacheSetupMismatch::NoLookups)?;
            if first.hits > 0 {
                Err(CacheSetupMismatch::ColdStartHit { hits: first.hits })
            } else if !first.full {
                Err(CacheSetupMismatch::ColdStartNotFull)
            } else if first.misses == 0 {
                Err(CacheSetupMismatch::NoLookups)
            } else {
                Ok(())
            }
        }
        CacheState::Warm => {
            if observed.full_builds > 0 {
                return Err(CacheSetupMismatch::WarmFullBuild {
                    builds: observed.full_builds,
                });
            }
            match observed.first {
                Some(first) if first.hits == 0 => Err(CacheSetupMismatch::WarmStartMissed {
                    misses: first.misses,
                }),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_the_thirteen_gate_scenarios() {
        // 검증 기준(XVI.2.5): §38.1 시나리오 13개, 이름·종류가 서로 다르고 모두 required.
        let names: std::collections::BTreeSet<&str> =
            LatencyScenario::ALL.iter().map(|s| s.name()).collect();
        let kinds: std::collections::BTreeSet<InteractionKind> =
            LatencyScenario::ALL.iter().map(|s| s.kind()).collect();
        assert_eq!(names.len(), 13);
        assert_eq!(kinds.len(), 13);
        for scenario in LatencyScenario::ALL {
            assert!(scenario.required());
            assert_eq!(LatencyScenario::parse(scenario.name()), Some(scenario));
        }
        assert_eq!(LatencyScenario::parse("spin"), None);
        let not_connected: Vec<&str> = LatencyScenario::ALL
            .iter()
            .filter(|s| s.core_replay() != CoreReplay::Supported)
            .map(|s| s.name())
            .collect();
        assert_eq!(
            not_connected,
            ["text-edit", "token-propagation", "component-update"]
        );
    }

    #[test]
    fn cache_setup_must_match_observed_hits_and_misses() {
        // 검증 기준(XVI.2.6): cache setup이 실제 hit/miss count와 일치.
        let one = |hits, misses, full| {
            let mut observed = CacheObservation::default();
            observed.record(CacheBuild { hits, misses, full });
            observed.record(CacheBuild {
                hits: 90,
                misses: 10,
                full: false,
            });
            observed
        };
        assert_eq!(
            check_cache_setup(CacheState::Cold, &one(0, 500, true)),
            Ok(())
        );
        let cold_twice = one(0, 500, true);
        assert_eq!(cold_twice.hits, 90);
        assert_eq!(cold_twice.misses, 510);
        assert_eq!(cold_twice.full_builds, 1);
        assert_eq!(
            check_cache_setup(CacheState::Cold, &one(3, 497, false)),
            Err(CacheSetupMismatch::ColdStartHit { hits: 3 })
        );
        assert_eq!(
            check_cache_setup(CacheState::Cold, &one(0, 5, false)),
            Err(CacheSetupMismatch::ColdStartNotFull)
        );
        assert_eq!(
            check_cache_setup(CacheState::Cold, &one(0, 0, true)),
            Err(CacheSetupMismatch::NoLookups)
        );
        assert_eq!(
            check_cache_setup(CacheState::Cold, &CacheObservation::default()),
            Err(CacheSetupMismatch::NoLookups)
        );
        assert_eq!(
            check_cache_setup(CacheState::Warm, &one(499, 1, false)),
            Ok(())
        );
        assert_eq!(
            check_cache_setup(CacheState::Warm, &one(0, 500, true)),
            Err(CacheSetupMismatch::WarmFullBuild { builds: 1 })
        );
        assert_eq!(
            check_cache_setup(CacheState::Warm, &one(0, 7, false)),
            Err(CacheSetupMismatch::WarmStartMissed { misses: 7 })
        );
        assert_eq!(
            check_cache_setup(CacheState::Warm, &CacheObservation::default()),
            Ok(())
        );
    }
}
