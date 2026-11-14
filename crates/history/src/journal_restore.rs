//! 앱 재실행 후 history 복원과 journal 봉인 (XXV.1.3, 로드맵).
//!
//! [`JournalStore::restore_window`]는 창의 active journal을 읽어 **현재 저장 세대와 journal base가 같을
//! 때만** undo·redo stack과 다음 sequence를 복원합니다. 복원할 수 없는 journal은 지우지 않고 `sealed/`
//! 아래 별도 session 파일로 옮깁니다(봉인). 봉인된 journal은 자동으로 다시 복원하지 않으므로 이후 기록되는
//! history와 섞이지 않습니다.
//!
//! | active journal | 결과 |
//! |---|---|
//! | 없음 | 빈 history([`RestoreOutcome::Fresh`]) |
//! | base == 현재 세대, 편집 가능 | 복원([`RestoreOutcome::Restored`]) |
//! | base ≠ 현재 세대(generation 번호 또는 manifest hash가 다름) | 봉인 [`SealReason::BaseMismatch`] |
//! | 읽기 전용(최소 writer version이 이 writer보다 높음) | 봉인 [`SealReason::ReadOnly`] — 복원 후 이어 기록할 수 없음 |
//! | 읽을 수 없음(형식·버전·손상) | 봉인 [`SealReason::Unreadable`] — 새 reader가 읽을 수 있게 보존 |
//! | 파일 이름과 창 id가 다름 | 봉인 [`SealReason::WindowMismatch`] |
//!
//! 실행 중 checkout·merge·rebase 등으로 기준점이 바뀌면 복원과 별개로 active journal을 봉인합니다
//! ([`SealReason::BaseChanged`], XXV.1.4 `journal_base_change`).
//!
//! 봉인 파일: `sealed/<window>.<순번 6자리>.<사유>.stlhjnl`. 같은 디렉터리 안 rename이라 내용 bytes가 그대로
//! 보존되고, 프로젝트 내부 정책에서도 guard `.gitignore`(`*`)가 하위 디렉터리를 함께 제외합니다.

use std::path::PathBuf;

use stellar_commands::id::WindowSessionId;
use stellar_core_types::ids::StableId;

use crate::grouping::HistoryGroupingPolicy;
use crate::history::History;
use crate::journal::{HistoryJournal, JournalAccess, JournalBase, JOURNAL_SCHEMA_VERSION};
use crate::journal_base_change::BaseChangeKind;
use crate::journal_store::{JournalStore, JournalStoreError, JOURNAL_EXTENSION};
use crate::window::WindowHistory;

/// 봉인 디렉터리 이름(journal 디렉터리 아래).
pub const SEALED_DIR: &str = "sealed";

/// journal을 봉인한 사유. 파일 이름 token은 저장 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SealReason {
    /// journal base가 현재 저장 세대와 다릅니다.
    BaseMismatch,
    /// 이 writer가 이어 기록할 수 없습니다(V.8.1 읽기 전용).
    ReadOnly,
    /// 이 reader가 읽을 수 없습니다(형식·버전·손상).
    Unreadable,
    /// 파일 이름과 journal 안 창 id가 다릅니다.
    WindowMismatch,
    /// 실행 중 기준점이 바뀌었습니다(checkout·merge·rebase 등, XXV.1.4).
    BaseChanged(BaseChangeKind),
}

impl SealReason {
    /// 모든 사유.
    pub const ALL: [SealReason; 12] = [
        SealReason::BaseMismatch,
        SealReason::ReadOnly,
        SealReason::Unreadable,
        SealReason::WindowMismatch,
        SealReason::BaseChanged(BaseChangeKind::Checkout),
        SealReason::BaseChanged(BaseChangeKind::Merge),
        SealReason::BaseChanged(BaseChangeKind::Rebase),
        SealReason::BaseChanged(BaseChangeKind::CherryPick),
        SealReason::BaseChanged(BaseChangeKind::Revert),
        SealReason::BaseChanged(BaseChangeKind::Reset),
        SealReason::BaseChanged(BaseChangeKind::ApplyMailbox),
        SealReason::BaseChanged(BaseChangeKind::Other),
    ];

    /// 파일 이름 token.
    pub fn token(self) -> &'static str {
        match self {
            SealReason::BaseMismatch => "base-mismatch",
            SealReason::ReadOnly => "read-only",
            SealReason::Unreadable => "unreadable",
            SealReason::WindowMismatch => "window-mismatch",
            SealReason::BaseChanged(kind) => kind.token(),
        }
    }

    /// token에서 사유를 찾습니다.
    pub fn from_token(token: &str) -> Option<SealReason> {
        SealReason::ALL.into_iter().find(|r| r.token() == token)
    }
}

/// 봉인된 journal session 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedJournal {
    /// 파일 경로.
    pub path: PathBuf,
    /// 원래 창.
    pub window: WindowSessionId,
    /// 창별 봉인 순번(1부터).
    pub index: u32,
    /// 봉인 사유.
    pub reason: SealReason,
}

/// 복원 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// active journal이 없어 빈 history입니다.
    Fresh,
    /// 복원했습니다.
    Restored {
        /// undo 항목 수.
        undo: usize,
        /// redo 항목 수.
        redo: usize,
    },
    /// 복원하지 않고 봉인했습니다. history는 비어 있습니다.
    Sealed(SealedJournal),
}

/// 복원한 창 history와 결과.
#[derive(Debug, Clone)]
pub struct RestoredWindow {
    /// 창 history(복원하지 않았으면 빈 history).
    pub history: WindowHistory,
    /// 결과.
    pub outcome: RestoreOutcome,
}

pub(crate) fn parse_sealed_name(name: &str) -> Option<(WindowSessionId, u32, SealReason)> {
    let stem = name.strip_suffix(&format!(".{JOURNAL_EXTENSION}"))?;
    let mut parts = stem.split('.');
    let window = parts.next()?.parse::<StableId>().ok()?;
    let index = parts.next()?;
    let reason = SealReason::from_token(parts.next()?)?;
    if parts.next().is_some() || index.len() != 6 {
        return None;
    }
    Some((
        WindowSessionId::from_stable_id(window),
        index.parse().ok()?,
        reason,
    ))
}

impl JournalStore {
    /// 봉인 디렉터리.
    pub fn sealed_dir(&self) -> PathBuf {
        self.dir().join(SEALED_DIR)
    }

    /// 봉인된 journal 목록(창, 순번 순).
    pub fn list_sealed(&self) -> Result<Vec<SealedJournal>, JournalStoreError> {
        let entries = match std::fs::read_dir(self.sealed_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if let Some((window, index, reason)) = parse_sealed_name(&name) {
                out.push(SealedJournal {
                    path: entry.path(),
                    window,
                    index,
                    reason,
                });
            }
        }
        out.sort_by_key(|s| (s.window, s.index));
        Ok(out)
    }

    /// 창의 active journal을 봉인 session으로 옮깁니다. active journal이 없으면 `None`입니다.
    pub fn seal(
        &self,
        window: WindowSessionId,
        reason: SealReason,
    ) -> Result<Option<SealedJournal>, JournalStoreError> {
        self.check_untracked()?;
        let active = self.journal_path(window);
        if !active.try_exists()? {
            return Ok(None);
        }
        std::fs::create_dir_all(self.sealed_dir())?;
        let index = self
            .list_sealed()?
            .iter()
            .filter(|s| s.window == window)
            .map(|s| s.index)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|i| *i <= 999_999)
            .ok_or(JournalStoreError::Io {
                kind: std::io::ErrorKind::StorageFull,
            })?;
        let path = self.sealed_dir().join(format!(
            "{window}.{index:06}.{}.{JOURNAL_EXTENSION}",
            reason.token()
        ));
        std::fs::rename(&active, &path)?;
        Ok(Some(SealedJournal {
            path,
            window,
            index,
            reason,
        }))
    }

    /// 봉인된 journal을 읽습니다(보존 확인·진단용). 읽을 수 없는 사유로 봉인된 파일은 오류일 수 있습니다.
    pub fn read_sealed(&self, sealed: &SealedJournal) -> Result<HistoryJournal, JournalStoreError> {
        Ok(HistoryJournal::decode(&std::fs::read(&sealed.path)?)?)
    }

    /// 앱 재실행 후 창 history를 복원합니다. `current`는 지금 연 문서의 저장 세대(storage current pointer가
    /// 가리키는 세대)입니다. 복원하지 못하면 active journal을 봉인하고 빈 history를 돌려줍니다.
    pub fn restore_window(
        &self,
        window: WindowSessionId,
        current: JournalBase,
        policy: HistoryGroupingPolicy,
    ) -> Result<RestoredWindow, JournalStoreError> {
        let fresh = |outcome| RestoredWindow {
            history: WindowHistory::with_policy(window, policy),
            outcome,
        };
        let bytes = match std::fs::read(self.journal_path(window)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(fresh(RestoreOutcome::Fresh))
            }
            Err(e) => return Err(e.into()),
        };
        let reason = match HistoryJournal::decode(&bytes) {
            Err(_) => SealReason::Unreadable,
            Ok(journal) if journal.header.window != window => SealReason::WindowMismatch,
            Ok(journal)
                if journal.header.access(JOURNAL_SCHEMA_VERSION)? != JournalAccess::Editable =>
            {
                SealReason::ReadOnly
            }
            Ok(journal) if journal.header.base != current => SealReason::BaseMismatch,
            Ok(journal) => {
                let outcome = RestoreOutcome::Restored {
                    undo: journal.undo.len(),
                    redo: journal.redo.len(),
                };
                let history =
                    History::from_parts(policy, journal.undo, journal.redo, journal.next_sequence);
                return Ok(RestoredWindow {
                    history: WindowHistory::from_history(window, history),
                    outcome,
                });
            }
        };
        let sealed = self.seal(window, reason)?.ok_or(JournalStoreError::Io {
            kind: std::io::ErrorKind::NotFound,
        })?;
        Ok(fresh(RestoreOutcome::Sealed(sealed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::journal::tests::base;
    use crate::journal_store::JournalLocation;
    use crate::test_fs::TempDir;
    use stellar_commands::command::Command;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_commands::selection::{Selection, SetSelection};
    use stellar_commands::state::DocumentState;
    use stellar_core_types::revision::StorageGeneration;

    fn snapshot(doc: &DocumentState) -> String {
        model_snapshot_json(doc.model()).unwrap()
    }

    /// 저장된 세대를 다시 여는 것과 같은 새 문서 상태(레코드 왕복, revision 0).
    fn reopen(doc: &DocumentState) -> DocumentState {
        let (model, _) = DocumentModel::from_records(doc.model().to_records().unwrap()).unwrap();
        DocumentState::new(model)
    }

    fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(sid(700 + n))
    }

    /// 편집 3개 + 선택 1개를 기록하고 마지막 편집을 undo한 창과 그 문서.
    fn edited_session(
        window: WindowSessionId,
    ) -> (WindowHistory, DocumentState, Vec<String>, String) {
        let mut history = WindowHistory::new(window);
        let mut doc = small_state();
        let mut states = vec![snapshot(&doc)];
        for (i, name) in ["e1", "e2", "e3"].iter().enumerate() {
            let t = tx(&doc, 1_000 + i as u64, Command::rename_node(sid(11), *name));
            history.record_document(&mut doc, &t, None).unwrap();
            states.push(snapshot(&doc));
            if i == 0 {
                history
                    .record_selection(&doc, &SetSelection::new(Selection::new([sid(12)], None)))
                    .unwrap();
            }
        }
        let final_state = snapshot(&doc);
        history.undo(&mut doc).unwrap().unwrap();
        (history, doc, states, final_state)
    }

    #[test]
    fn same_base_restores_undo_redo_after_relaunch() {
        // 검증(XXV.1.3): 현재 세대 = journal base일 때만 복원. 복원한 stack으로 undo·redo가 이어집니다.
        let project = TempDir::new("restore-project");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let w = window(1);
        let (history, doc, states, final_state) = edited_session(w);
        let next = history.history().next_sequence();
        store.record_save_point(&history, base(5)).unwrap();
        // 앱 종료: 메모리 history 폐기. 재실행: 저장된 세대(같은 문서 내용)를 새로 엽니다.
        drop(history);
        let mut doc = reopen(&doc);
        assert_eq!(snapshot(&doc), states[2]);
        let restored = store
            .restore_window(w, base(5), HistoryGroupingPolicy::default())
            .unwrap();
        assert_eq!(
            restored.outcome,
            RestoreOutcome::Restored { undo: 3, redo: 1 }
        );
        let mut history = restored.history;
        assert_eq!(history.window(), w);
        assert_eq!(history.history().next_sequence(), next);
        assert!(
            history.view().selection().is_empty(),
            "현재 선택은 복원하지 않음"
        );
        assert!(store.list_sealed().unwrap().is_empty());
        // redo(e3) → 종료 전 최종 상태.
        history.redo(&mut doc).unwrap().unwrap();
        assert_eq!(snapshot(&doc), final_state);
        // 전체 undo → 최초 문서. 선택 항목도 같은 stack에서 되돌아갑니다.
        while history.undo(&mut doc).unwrap().is_some() {}
        assert_eq!(snapshot(&doc), states[0]);
        // 전체 redo → 최종 상태.
        while history.redo(&mut doc).unwrap().is_some() {}
        assert_eq!(snapshot(&doc), final_state);
        // 새 편집은 이어지는 sequence를 받습니다(단조성 유지).
        let t = tx(&doc, 2_000, Command::rename_node(sid(12), "after"));
        history.record_document(&mut doc, &t, None).unwrap();
        assert_eq!(
            history.history().entries().last().unwrap().sequence.get(),
            next
        );
        // journal이 없는 창은 빈 history.
        let fresh = store
            .restore_window(window(2), base(5), HistoryGroupingPolicy::default())
            .unwrap();
        assert_eq!(fresh.outcome, RestoreOutcome::Fresh);
        assert!(fresh.history.history().is_empty());
    }

    #[test]
    fn mismatched_base_is_sealed_and_not_mixed() {
        // 검증 기준(XXV.1.3): mismatched base 봉인.
        let app = TempDir::new("restore-app");
        let project = TempDir::new("restore-project-2");
        let store = JournalStore::open(
            project.path(),
            JournalLocation::AppData {
                app_data_dir: app.path().to_path_buf(),
                project_key: crate::journal_store::JournalProjectKey::new("k1").unwrap(),
            },
        )
        .unwrap();
        let w = window(3);
        let (history, doc, _, _) = edited_session(w);
        let path = store.record_save_point(&history, base(5)).unwrap();
        let original = std::fs::read(&path).unwrap();
        // generation 번호가 다르거나, 번호는 같고 manifest hash만 다르면 다른 base입니다.
        let same_number_other_hash = JournalBase::new(StorageGeneration(5), [9; 32]);
        for (round, current) in [base(6), same_number_other_hash].into_iter().enumerate() {
            let mut doc = reopen(&doc);
            let before = snapshot(&doc);
            let restored = store
                .restore_window(w, current, HistoryGroupingPolicy::default())
                .unwrap();
            let RestoreOutcome::Sealed(sealed) = &restored.outcome else {
                panic!("봉인되어야 합니다: {:?}", restored.outcome);
            };
            assert_eq!(sealed.reason, SealReason::BaseMismatch);
            assert_eq!(sealed.index, round as u32 + 1);
            assert_eq!(sealed.window, w);
            // 봉인 session은 원래 bytes 그대로 보존되고 active journal은 없습니다.
            assert_eq!(std::fs::read(&sealed.path).unwrap(), original);
            assert_eq!(store.read_sealed(sealed).unwrap().header.base, base(5));
            assert!(!store.journal_path(w).exists());
            // history는 비어 있어 undo가 문서를 바꾸지 않습니다(혼합 없음).
            let mut fresh = restored.history;
            assert!(fresh.history().is_empty());
            assert_eq!(fresh.undo(&mut doc).unwrap(), None);
            assert_eq!(snapshot(&doc), before);
            // 같은 창에서 새로 기록한 journal에는 봉인된 항목이 없습니다.
            let t = tx(&doc, 3_000, Command::rename_node(sid(13), "new"));
            fresh.record_document(&mut doc, &t, None).unwrap();
            store.record_save_point(&fresh, current).unwrap();
            let new_journal = store.read(w).unwrap().unwrap();
            assert_eq!(new_journal.undo.len(), 1);
            assert_eq!(new_journal.header.base, current);
            // 다음 round를 위해 원래 journal을 active로 되돌립니다.
            std::fs::write(&path, &original).unwrap();
        }
        let sealed = store.list_sealed().unwrap();
        assert_eq!(sealed.len(), 2);
        assert!(sealed.iter().all(|s| s.reason == SealReason::BaseMismatch));
        // 봉인 session을 다시 restore하지 않습니다: 같은 base로 열어도 active(원래 journal)만 봅니다.
        let restored = store
            .restore_window(w, base(5), HistoryGroupingPolicy::default())
            .unwrap();
        assert_eq!(
            restored.outcome,
            RestoreOutcome::Restored { undo: 3, redo: 1 }
        );
        assert_eq!(store.list_sealed().unwrap().len(), 2);
    }

    #[test]
    fn unreadable_read_only_and_foreign_journals_are_sealed() {
        let project = TempDir::new("restore-project-3");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let (history, _, _, _) = edited_session(window(4));
        let path = store.record_save_point(&history, base(5)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let policy = HistoryGroupingPolicy::default();
        let w = window(4);

        // 손상.
        std::fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        let outcome = store.restore_window(w, base(5), policy).unwrap().outcome;
        assert!(
            matches!(&outcome, RestoreOutcome::Sealed(s) if s.reason == SealReason::Unreadable)
        );
        // 미래 writer(최소 writer 2)가 쓴 journal: 같은 base라도 이어 기록할 수 없어 봉인.
        let mut future = bytes.clone();
        future[12..16].copy_from_slice(&3u32.to_le_bytes());
        future[20..24].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&path, &future).unwrap();
        let outcome = store.restore_window(w, base(5), policy).unwrap().outcome;
        assert!(matches!(&outcome, RestoreOutcome::Sealed(s) if s.reason == SealReason::ReadOnly));
        // 다른 창의 journal이 이 창 이름으로 있음.
        std::fs::write(store.journal_path(window(5)), &bytes).unwrap();
        let outcome = store
            .restore_window(window(5), base(5), policy)
            .unwrap()
            .outcome;
        assert!(
            matches!(&outcome, RestoreOutcome::Sealed(s) if s.reason == SealReason::WindowMismatch)
        );
        let sealed = store.list_sealed().unwrap();
        assert_eq!(sealed.len(), 3);
        // 봉인 파일 이름 형식과 내용 보존.
        let read_only = sealed
            .iter()
            .find(|s| s.reason == SealReason::ReadOnly)
            .unwrap();
        assert_eq!(std::fs::read(&read_only.path).unwrap(), future);
        assert!(read_only
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".000002.read-only.stlhjnl"));
        // 봉인 디렉터리도 guard 아래(프로젝트 내부 정책)입니다.
        assert!(read_only.path.starts_with(store.dir()));
        // active journal 없는 봉인은 None.
        assert_eq!(
            store.seal(window(9), SealReason::BaseMismatch).unwrap(),
            None
        );
        for reason in SealReason::ALL {
            assert_eq!(SealReason::from_token(reason.token()), Some(reason));
        }
        assert_eq!(parse_sealed_name("x.000001.base-mismatch.stlhjnl"), None);
    }
}
