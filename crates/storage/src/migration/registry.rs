//! migration step registry (V.8.2).
//!
//! `MigrationStep { from, to, migrate }`를 등록하고, 문서 스키마 버전에서 목표
//! 버전까지 **연속 경로**를 계획합니다. 경로가 없으면(gap) 거부하고, 등록
//! 집합에 cycle이 있으면 계획 시점에 탐지해 거부합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::chunk::envelope::ChunkKindCode;
use crate::chunk::id::ChunkId;

/// 세대 migration 중 보존하거나 변환할 logical chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationChunk {
    /// 논리 id.
    pub chunk_id: ChunkId,
    /// chunk kind.
    pub kind: ChunkKindCode,
    /// 이 payload의 스키마 버전.
    pub schema_version: u32,
    /// canonical payload.
    pub payload: Vec<u8>,
}

/// migration이 변환하는 in-memory 문서 모델.
///
/// 저장 계층 관점의 세대 내용입니다: 스키마 버전 + logical chunk payload들 +
/// root document 목록. migration step은 이 모델을 다음 버전으로 변환합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDocument {
    /// 현재 스키마 버전.
    pub schema_version: u32,
    /// 세대 스키마와 독립적인 payload 스키마를 가진 logical chunk들.
    pub chunks: Vec<MigrationChunk>,
    /// root document StableId들.
    pub root_document_ids: Vec<StableId>,
}

/// step 실행 오류(각 step 구현이 반환).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStepError {
    /// 사람이 읽는 원인.
    pub message: String,
}

impl fmt::Display for MigrationStepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration step 실패: {}", self.message)
    }
}

impl std::error::Error for MigrationStepError {}

/// migration 변환 함수.
pub type MigrateFn = Box<dyn Fn(&mut MigrationDocument) -> Result<(), MigrationStepError>>;

/// 한 단계의 스키마 migration.
pub struct MigrationStep {
    /// 출발 스키마 버전.
    pub from: u32,
    /// 도착 스키마 버전.
    pub to: u32,
    /// 변환 함수. `schema_version` 갱신은 registry가 수행합니다.
    pub migrate: MigrateFn,
}

impl fmt::Debug for MigrationStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationStep")
            .field("from", &self.from)
            .field("to", &self.to)
            .finish_non_exhaustive()
    }
}

/// registry 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRegistryError {
    /// 같은 출발 버전의 step이 이미 있습니다.
    DuplicateStep {
        /// 중복 출발 버전.
        from: u32,
    },
    /// 출발과 도착이 같거나 역방향입니다.
    InvalidStep {
        /// 출발.
        from: u32,
        /// 도착.
        to: u32,
    },
    /// from에서 to로 가는 연속 경로가 없습니다.
    NoPath {
        /// 경로가 끊긴 버전.
        stuck_at: u32,
        /// 목표 버전.
        target: u32,
    },
    /// step chain에 cycle이 있습니다.
    Cycle {
        /// cycle이 탐지된 버전.
        at: u32,
    },
    /// step 실행 실패.
    Step(MigrationStepError),
}

impl fmt::Display for MigrationRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationRegistryError::DuplicateStep { from } => {
                write!(f, "출발 버전 {from}의 step이 중복됩니다")
            }
            MigrationRegistryError::InvalidStep { from, to } => {
                write!(f, "잘못된 step입니다: {from} → {to} (전진만 허용)")
            }
            MigrationRegistryError::NoPath { stuck_at, target } => {
                write!(
                    f,
                    "버전 {stuck_at}에서 {target}로 가는 step이 없습니다(gap)"
                )
            }
            MigrationRegistryError::Cycle { at } => {
                write!(f, "migration chain에 cycle이 있습니다: 버전 {at}")
            }
            MigrationRegistryError::Step(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MigrationRegistryError {}

/// migration step registry.
#[derive(Debug, Default)]
pub struct MigrationRegistry {
    /// 출발 버전 → step. 출발 버전당 step 하나만 허용합니다.
    steps: BTreeMap<u32, MigrationStep>,
}

impl MigrationRegistry {
    /// 빈 registry.
    pub fn new() -> MigrationRegistry {
        MigrationRegistry::default()
    }

    /// step을 등록합니다. 역방향·제자리·중복 출발 버전은 거부합니다.
    pub fn register(&mut self, step: MigrationStep) -> Result<(), MigrationRegistryError> {
        if step.to <= step.from {
            return Err(MigrationRegistryError::InvalidStep {
                from: step.from,
                to: step.to,
            });
        }
        if self.steps.contains_key(&step.from) {
            return Err(MigrationRegistryError::DuplicateStep { from: step.from });
        }
        self.steps.insert(step.from, step);
        Ok(())
    }

    /// from에서 to까지의 연속 step 경로를 계획합니다.
    ///
    /// 경로가 끊기면 `NoPath`, 같은 버전을 다시 방문하면 `Cycle`, 목표를
    /// 지나치면 `NoPath`(그 버전에서 목표로 갈 수 없음)입니다.
    pub fn plan_path(
        &self,
        from: u32,
        to: u32,
    ) -> Result<Vec<&MigrationStep>, MigrationRegistryError> {
        let mut path = Vec::new();
        let mut visited = std::collections::BTreeSet::new();
        let mut current = from;
        while current != to {
            if !visited.insert(current) {
                return Err(MigrationRegistryError::Cycle { at: current });
            }
            let step = self
                .steps
                .get(&current)
                .ok_or(MigrationRegistryError::NoPath {
                    stuck_at: current,
                    target: to,
                })?;
            if step.to > to {
                return Err(MigrationRegistryError::NoPath {
                    stuck_at: current,
                    target: to,
                });
            }
            path.push(step);
            current = step.to;
        }
        Ok(path)
    }

    /// 문서를 목표 버전까지 변환합니다(경로 계획 + 순차 실행).
    ///
    /// 각 step 실행 후 registry가 `schema_version`을 step.to로 갱신합니다.
    pub fn migrate_to(
        &self,
        document: &mut MigrationDocument,
        target: u32,
    ) -> Result<(), MigrationRegistryError> {
        let path = self.plan_path(document.schema_version, target)?;
        for step in path {
            (step.migrate)(document).map_err(MigrationRegistryError::Step)?;
            document.schema_version = step.to;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_step(from: u32, to: u32) -> MigrationStep {
        MigrationStep {
            from,
            to,
            migrate: Box::new(|_| Ok(())),
        }
    }

    fn document(version: u32) -> MigrationDocument {
        MigrationDocument {
            schema_version: version,
            chunks: vec![],
            root_document_ids: vec![],
        }
    }

    #[test]
    fn contiguous_path_is_planned_in_order() {
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 2)).unwrap();
        registry.register(noop_step(2, 3)).unwrap();
        registry.register(noop_step(3, 4)).unwrap();
        let path = registry.plan_path(1, 4).unwrap();
        let hops: Vec<(u32, u32)> = path.iter().map(|s| (s.from, s.to)).collect();
        assert_eq!(hops, vec![(1, 2), (2, 3), (3, 4)]);
        // 제자리 경로는 빈 경로.
        assert!(registry.plan_path(2, 2).unwrap().is_empty());
    }

    #[test]
    fn gap_is_rejected() {
        // 검증 기준(V.8.2): gap fixture 실패.
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 2)).unwrap();
        registry.register(noop_step(3, 4)).unwrap(); // 2→3 없음.
        assert_eq!(
            registry.plan_path(1, 4).unwrap_err(),
            MigrationRegistryError::NoPath {
                stuck_at: 2,
                target: 4
            }
        );
    }

    #[test]
    fn cycle_is_rejected() {
        // 검증 기준(V.8.2): cycle fixture 실패.
        // register가 역방향을 거부하므로 cycle은 "목표를 지나치는" 다중 단계
        // 조합으로도 만들 수 없지만, 방문 집합 방어는 plan_path에 있습니다.
        // 역방향 step 등록 자체가 거부되는지 확인합니다.
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 2)).unwrap();
        assert_eq!(
            registry.register(noop_step(2, 1)).unwrap_err(),
            MigrationRegistryError::InvalidStep { from: 2, to: 1 }
        );
        assert_eq!(
            registry.register(noop_step(5, 5)).unwrap_err(),
            MigrationRegistryError::InvalidStep { from: 5, to: 5 }
        );
    }

    #[test]
    fn duplicate_step_is_rejected() {
        // 검증 기준(V.8.2): duplicate step fixture 실패.
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 2)).unwrap();
        assert_eq!(
            registry.register(noop_step(1, 3)).unwrap_err(),
            MigrationRegistryError::DuplicateStep { from: 1 }
        );
    }

    #[test]
    fn overshooting_step_is_no_path() {
        // 1→5 step만 있는데 목표가 3이면 경로 없음.
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 5)).unwrap();
        assert_eq!(
            registry.plan_path(1, 3).unwrap_err(),
            MigrationRegistryError::NoPath {
                stuck_at: 1,
                target: 3
            }
        );
    }

    #[test]
    fn migrate_to_applies_steps_and_updates_version() {
        let mut registry = MigrationRegistry::new();
        registry.register(noop_step(1, 2)).unwrap();
        registry
            .register(MigrationStep {
                from: 2,
                to: 3,
                migrate: Box::new(|doc| {
                    doc.root_document_ids.push(StableId::new_v7(9, 0, 0));
                    Ok(())
                }),
            })
            .unwrap();

        let mut doc = document(1);
        registry.migrate_to(&mut doc, 3).unwrap();
        assert_eq!(doc.schema_version, 3);
        assert_eq!(doc.root_document_ids.len(), 1);
    }

    #[test]
    fn failing_step_stops_migration() {
        let mut registry = MigrationRegistry::new();
        registry
            .register(MigrationStep {
                from: 1,
                to: 2,
                migrate: Box::new(|_| {
                    Err(MigrationStepError {
                        message: "의도된 실패".to_string(),
                    })
                }),
            })
            .unwrap();
        let mut doc = document(1);
        let err = registry.migrate_to(&mut doc, 2).unwrap_err();
        assert!(matches!(err, MigrationRegistryError::Step(_)));
        // 실패 시 버전 미갱신.
        assert_eq!(doc.schema_version, 1);
    }
}
