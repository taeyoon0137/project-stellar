//! 문서 변경 시 영향받는 진단만 다시 검사합니다 (XXV.2.3).
//!
//! 진단은 (문서, 생산자) scope 단위로 계산·교체합니다. 각 scope는 계산할 때 [`Footprint`]를 함께 남깁니다. footprint는
//! "이 scope의 결과가 무엇을 읽었는가"입니다.
//!
//! - `nodes`: 결과가 직접 읽은 영속 객체(node·modifier). 이 객체가 바뀌거나 삽입·삭제되면 영향받습니다. 없는 대상을
//!   가리키는 참조의 원래 ID도 넣습니다(되살아나면 진단이 사라져야 하므로).
//! - `keys`: 어느 객체든 이 속성 key가 바뀌면 영향받습니다(새 참조가 생길 수 있는 속성). 경로에 key가 없는 변경(modifier
//!   삽입·삭제처럼 객체 전체가 바뀐 경우)은 key 관심이 있는 scope 모두에 영향입니다.
//! - `inserts`: 어느 node든 새로 삽입되면 영향받습니다(새 node의 payload가 새 참조를 가질 수 있음).
//!
//! [`IntegrityEngine::apply_changes`]는 command 결과 [`ChangeSet`](VII.1.4)을 각 scope footprint와 대조합니다.
//!
//! - 영향받은 **내장 checker** scope([`DocumentChecker`], 문서 레코드만 읽음)는 그 자리에서 다시 계산합니다. 필요한
//!   경우에만 문서 레코드를 만듭니다. 영향받은 scope가 없으면 레코드도 만들지 않고 checker 실행도 0입니다.
//! - 영향받은 **외부 생산자** scope(font registry·접근성 tree·Target Profile·binding 저장소·library cache처럼 문서 밖 상태도
//!   읽는 생산자, [`IntegrityEngine::submit`]으로 받음)는 stale로 표시해 보고합니다. 호출자가 그 생산자를 다시 실행해
//!   `submit`하면 stale이 풀립니다. 그 사이 기존 진단은 남습니다.
//!
//! 영향 판정은 보수적입니다: footprint에 걸린 변경이 결과를 바꾸지 않을 수는 있지만, footprint에 걸리지 않은 변경이
//! 결과를 바꾸면 안 됩니다. 내장 checker는 무작위 command 열에서 증분 결과가 전체 재검사와 같은지 test합니다.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};

use stellar_commands::change::{node_keys, ChangeSet};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_document::model::{DocumentModel, ModelError};
use stellar_document::records::DocumentRecords;

use crate::diagnostic::Diagnostic;
use crate::index::DiagnosticIndex;

/// 생산자 이름(진단 key의 origin). 내장·변환 생산자 이름은 [`origins`]에 있습니다.
pub type SourceId = &'static str;

/// 생산자 이름 목록.
pub mod origins {
    use super::SourceId;
    /// 문서 레코드의 미해결 node 참조(내장 checker).
    pub const REFERENCES: SourceId = "references";
    /// Image asset 누락(내장 checker, asset store 조회 필요).
    pub const ASSETS: SourceId = "assets";
    /// OrphanedBinding.
    pub const BINDINGS: SourceId = "bindings";
    /// 미해결 token 참조(token registry 등 호출자가 모은 참조).
    pub const TOKENS: SourceId = "tokens";
    /// missing font.
    pub const FONTS: SourceId = "fonts";
    /// 접근성.
    pub const ACCESSIBILITY: SourceId = "accessibility";
    /// 사용할 수 없는 library(프로젝트 단위).
    pub const LIBRARIES: SourceId = "libraries";
    /// 사용할 수 없는 library의 resource를 가리키는 문서 참조.
    pub const LIBRARY_REFS: SourceId = "library-refs";
    /// Target Profile capability.
    pub const TARGET_PROFILES: SourceId = "target-profiles";
    /// schema migration.
    pub const MIGRATION: SourceId = "migration";
}

/// 속성 key 관심.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyInterest {
    /// key 변경과 무관(watched node 변경만 봄).
    #[default]
    Nothing,
    /// 이 key들(또는 key 없는 객체 전체 변경).
    Keys(BTreeSet<PropertyKey>),
    /// 편집 전용 공통 필드(잠금·표시 이름)를 뺀 모든 key.
    Any,
}

impl KeyInterest {
    fn matches(&self, path: &PropertyPath) -> bool {
        match self {
            KeyInterest::Nothing => false,
            KeyInterest::Keys(keys) => {
                path.keys().is_empty() || path.keys().iter().any(|k| keys.contains(k))
            }
            KeyInterest::Any => {
                !matches!(path.keys(), [k] if *k == node_keys::LOCKED || *k == node_keys::DISPLAY_NAME)
            }
        }
    }
}

/// scope 결과가 읽은 것(모듈 문서 참조).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Footprint {
    /// 직접 읽은 영속 객체.
    pub nodes: BTreeSet<StableId>,
    /// 속성 key 관심.
    pub keys: KeyInterest,
    /// node 삽입에 반응하는지.
    pub inserts: bool,
}

impl Footprint {
    /// 문서 변경과 무관한 scope(프로젝트 단위 생산자).
    pub fn independent() -> Footprint {
        Footprint::default()
    }

    /// 객체 목록만 보는 footprint.
    pub fn nodes(nodes: impl IntoIterator<Item = StableId>) -> Footprint {
        Footprint {
            nodes: nodes.into_iter().collect(),
            ..Footprint::default()
        }
    }

    /// 변경이 이 footprint에 걸리는지.
    pub fn affected_by(&self, changes: &ChangeSet) -> bool {
        if self.inserts && !changes.inserted_nodes().is_empty() {
            return true;
        }
        let watched = |id: &StableId| self.nodes.contains(id);
        changes.changed_nodes().iter().any(watched)
            || changes.removed_nodes().iter().any(watched)
            || changes.inserted_nodes().iter().any(watched)
            || changes
                .changed_properties()
                .iter()
                .any(|path| watched(&path.root()) || self.keys.matches(path))
    }
}

/// 생산자 결과 한 묶음.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contribution {
    /// 진단.
    pub diagnostics: Vec<Diagnostic>,
    /// 읽은 것.
    pub footprint: Footprint,
}

/// 내장 checker가 문서 밖에서 받는 조회. 없으면 그 checker는 실행하지 않고 scope를 비웁니다.
#[derive(Clone, Copy, Default)]
pub struct CheckContext<'a> {
    /// asset store 조회(`true` = 있음).
    pub asset_exists: Option<&'a dyn Fn(StableId) -> bool>,
}

impl std::fmt::Debug for CheckContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckContext")
            .field("asset_exists", &self.asset_exists.is_some())
            .finish()
    }
}

/// 문서 레코드(와 [`CheckContext`])만 읽는 내장 checker.
pub trait DocumentChecker: Send + Sync {
    /// 생산자 이름.
    fn origin(&self) -> SourceId;
    /// 문서 하나를 검사합니다. 실행할 수 없으면(문맥 없음) `None`.
    fn check(
        &self,
        document: StableId,
        records: &DocumentRecords,
        context: &CheckContext<'_>,
    ) -> Option<Contribution>;
}

/// scope 식별자: (문서, 생산자).
pub type ScopeKey = (Option<StableId>, String);

#[derive(Debug, Clone)]
struct ScopeState {
    footprint: Footprint,
    built_in: bool,
    stale: bool,
}

/// 증분 재검사 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecheckReport {
    /// 다시 계산한 내장 checker.
    pub rechecked: Vec<SourceId>,
    /// 새로 stale이 된 외부 생산자 scope.
    pub stale: Vec<ScopeKey>,
    /// index가 바뀌었는지.
    pub changed: bool,
}

/// 진단 index와 scope footprint를 함께 관리하는 증분 엔진.
pub struct IntegrityEngine {
    index: DiagnosticIndex,
    checkers: Vec<Box<dyn DocumentChecker>>,
    scopes: BTreeMap<ScopeKey, ScopeState>,
    runs: u64,
}

impl std::fmt::Debug for IntegrityEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegrityEngine")
            .field("diagnostics", &self.index.len())
            .field("scopes", &self.scopes.len())
            .field("runs", &self.runs)
            .finish()
    }
}

impl Default for IntegrityEngine {
    fn default() -> Self {
        IntegrityEngine::new()
    }
}

impl IntegrityEngine {
    /// 내장 checker(미해결 참조, Image asset)를 가진 엔진.
    pub fn new() -> IntegrityEngine {
        IntegrityEngine::with_checkers(crate::sources::built_in_checkers())
    }

    /// 주어진 내장 checker를 가진 엔진.
    pub fn with_checkers(checkers: Vec<Box<dyn DocumentChecker>>) -> IntegrityEngine {
        IntegrityEngine {
            index: DiagnosticIndex::new(),
            checkers,
            scopes: BTreeMap::new(),
            runs: 0,
        }
    }

    /// 공통 진단 index.
    pub fn index(&self) -> &DiagnosticIndex {
        &self.index
    }

    /// 지금까지 내장 checker를 실행한 횟수(증분 검증용 계측).
    pub fn check_runs(&self) -> u64 {
        self.runs
    }

    /// stale인 외부 scope.
    pub fn stale_scopes(&self) -> Vec<ScopeKey> {
        self.scopes
            .iter()
            .filter(|(_, s)| s.stale)
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn run(
        &mut self,
        index: usize,
        document: StableId,
        records: &DocumentRecords,
        context: &CheckContext<'_>,
    ) -> bool {
        self.runs += 1;
        let checker = &self.checkers[index];
        let origin = checker.origin();
        let key: ScopeKey = (Some(document), origin.to_string());
        match checker.check(document, records, context) {
            Some(contribution) => {
                self.scopes.insert(
                    key,
                    ScopeState {
                        footprint: contribution.footprint,
                        built_in: true,
                        stale: false,
                    },
                );
                self.index
                    .replace_scope(Some(document), origin, contribution.diagnostics)
            }
            None => {
                self.scopes.remove(&key);
                self.index.replace_scope(Some(document), origin, Vec::new())
            }
        }
    }

    /// 문서 하나를 내장 checker 전부로 검사합니다(처음 열 때·문맥이 바뀔 때).
    pub fn check_document(
        &mut self,
        document: StableId,
        records: &DocumentRecords,
        context: &CheckContext<'_>,
    ) -> bool {
        let mut changed = false;
        for index in 0..self.checkers.len() {
            changed |= self.run(index, document, records, context);
        }
        changed
    }

    /// 문서 하나의 내장 checker 하나만 다시 검사합니다(예: asset store가 바뀜).
    pub fn recheck_origin(
        &mut self,
        document: StableId,
        origin: &str,
        records: &DocumentRecords,
        context: &CheckContext<'_>,
    ) -> bool {
        match self.checkers.iter().position(|c| c.origin() == origin) {
            Some(index) => self.run(index, document, records, context),
            None => false,
        }
    }

    /// command 결과로 영향받는 scope만 다시 검사하거나 stale로 표시합니다. 문서 레코드는 다시 계산할 내장 checker가
    /// 있을 때만 `model`에서 만듭니다.
    pub fn apply_changes(
        &mut self,
        document: StableId,
        changes: &ChangeSet,
        model: &DocumentModel,
        context: &CheckContext<'_>,
    ) -> Result<RecheckReport, ModelError> {
        let mut report = RecheckReport::default();
        if changes.is_empty() {
            return Ok(report);
        }
        for (key, scope) in self.scopes.iter_mut() {
            if key.0 == Some(document)
                && !scope.built_in
                && !scope.stale
                && scope.footprint.affected_by(changes)
            {
                scope.stale = true;
                report.stale.push(key.clone());
            }
        }
        let affected: Vec<usize> = (0..self.checkers.len())
            .filter(|&i| {
                let key: ScopeKey = (Some(document), self.checkers[i].origin().to_string());
                // 아직 검사한 적 없는 scope는 열 때 check_document로 계산하므로 여기서 새로 만들지 않습니다.
                self.scopes
                    .get(&key)
                    .is_some_and(|s| s.footprint.affected_by(changes))
            })
            .collect();
        if affected.is_empty() {
            return Ok(report);
        }
        let records = OnceCell::new();
        for index in affected {
            let records = match records.get() {
                Some(r) => r,
                None => {
                    let mut built = model.to_records()?;
                    built.sort_canonical();
                    records.get_or_init(|| built)
                }
            };
            report.rechecked.push(self.checkers[index].origin());
            report.changed |= self.run(index, document, records, context);
        }
        Ok(report)
    }

    /// 외부 생산자 결과를 scope 단위로 넣습니다(문서 단위면 `document`, 프로젝트 단위면 `None`). stale이 풀립니다.
    pub fn submit(
        &mut self,
        document: Option<StableId>,
        origin: &str,
        contribution: Contribution,
    ) -> bool {
        self.scopes.insert(
            (document, origin.to_string()),
            ScopeState {
                footprint: contribution.footprint,
                built_in: false,
                stale: false,
            },
        );
        self.index
            .replace_scope(document, origin, contribution.diagnostics)
    }

    /// 문서를 닫거나 지웠을 때 그 문서의 scope와 진단을 모두 뺍니다.
    pub fn remove_document(&mut self, document: StableId) -> bool {
        let keys: Vec<ScopeKey> = self
            .scopes
            .keys()
            .filter(|k| k.0 == Some(document))
            .cloned()
            .collect();
        let mut changed = false;
        for key in keys {
            self.scopes.remove(&key);
            changed |= self.index.replace_scope(key.0, &key.1, Vec::new());
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn key_interest_matches_whole_entity_and_skips_editing_fields() {
        let keys = KeyInterest::Keys([PropertyKey(0x0103)].into());
        assert!(keys.matches(&PropertyPath::new(sid(1))));
        assert!(keys.matches(&PropertyPath::with_keys(sid(1), vec![PropertyKey(0x0103)])));
        assert!(!keys.matches(&PropertyPath::with_keys(sid(1), vec![PropertyKey(0x0104)])));
        assert!(!KeyInterest::Any.matches(&PropertyPath::with_keys(
            sid(1),
            vec![node_keys::DISPLAY_NAME]
        )));
        assert!(
            KeyInterest::Any.matches(&PropertyPath::with_keys(sid(1), vec![node_keys::VISIBLE]))
        );
        assert!(!KeyInterest::Nothing.matches(&PropertyPath::new(sid(1))));
    }
}
