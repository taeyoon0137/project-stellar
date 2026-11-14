//! checkout·merge·rebase 등 기준점 변경 시 journal 봉인 (XXV.1.4, 로드맵).
//!
//! 앱이 실행 중일 때 Git 작업(checkout·merge·rebase·cherry-pick·revert·reset·am 등)으로 프로젝트의 저장
//! 기준점이 바뀌면, 그 전 기준점에서 기록한 journal은 새 기준점의 history와 섞이면 안 됩니다.
//! [`JournalStore::seal_for_base_change`]는 두 가지를 함께 합니다.
//!
//! 1. **메모리 창 history에 새 journal session 시작**: 각 열린 창의 journal 하한을 지금의 다음 sequence로
//!    올리고 열린 grouping을 닫습니다([`History::journal_stacks`]). 이후 저장 시점 기록에는 기준점 변경
//!    뒤 항목만 들어갑니다. 메모리 undo·redo stack은 그대로라 같은 실행 세션에서는 기준점 변경을 적용한
//!    synthetic command도 계속 undo할 수 있습니다(기술 아키텍처 §22.5).
//! 2. **active journal 봉인**: 프로젝트 journal 디렉터리의 active journal을 **열려 있지 않은 창 것까지
//!    모두** `sealed/<window>.<순번>.<작업 token>.stlhjnl`로 옮깁니다(bytes 보존, 자동 복원 없음, XXV.1.3과
//!    같은 봉인 경로). 그래서 같은 base로 되돌아와도(A→B→A) 봉인 전 session을 다시 복원하지 않습니다.
//!
//! 순서는 1 → 2입니다. 봉인 중 I/O가 실패해도 메모리 session은 이미 나뉘어 이후 기록이 섞이지 않으며,
//! 창 없이 다시 부르면 남은 active journal만 봉인합니다.
//!
//! **호출 시점 계약**: caller(project·runtime)는 Git 작업 뒤 창 문서를 새 기준점 snapshot으로 바꾼 직후
//! 부릅니다(synthetic command로 적용했다면 그 항목을 기록한 뒤). 무엇이 기준점 변경인지(HEAD commit 변경,
//! `RepositoryStatus` 진행 중 작업 XIV.2.2 등)는 git-porcelain을 쓰는 caller가 판정해 [`BaseChangeKind`]로
//! 넘깁니다(history는 git-porcelain에 의존하지 않음). 마지막 저장 시점 뒤의 미저장 메모리 항목은 journal에
//! 기록되지 않았으므로 봉인 session에도 없습니다.

use stellar_commands::id::WindowSessionId;
use stellar_core_types::ids::StableId;

use crate::entry::HistoryError;
use crate::journal_restore::{SealReason, SealedJournal};
use crate::journal_store::{JournalStore, JournalStoreError, JOURNAL_EXTENSION};
use crate::window::WindowHistory;

/// 기준점을 바꾼 작업 종류. 봉인 파일 이름 token은 저장 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BaseChangeKind {
    /// branch·commit checkout(switch 포함).
    Checkout,
    /// merge(fast-forward 포함).
    Merge,
    /// rebase.
    Rebase,
    /// cherry-pick.
    CherryPick,
    /// revert.
    Revert,
    /// reset(HEAD 이동).
    Reset,
    /// `git am`.
    ApplyMailbox,
    /// 그 밖의 외부 기준점 교체(stash 적용·외부 도구 등).
    Other,
}

impl BaseChangeKind {
    /// 모든 종류.
    pub const ALL: [BaseChangeKind; 8] = [
        BaseChangeKind::Checkout,
        BaseChangeKind::Merge,
        BaseChangeKind::Rebase,
        BaseChangeKind::CherryPick,
        BaseChangeKind::Revert,
        BaseChangeKind::Reset,
        BaseChangeKind::ApplyMailbox,
        BaseChangeKind::Other,
    ];

    /// 봉인 파일 이름 token.
    pub fn token(self) -> &'static str {
        match self {
            BaseChangeKind::Checkout => "checkout",
            BaseChangeKind::Merge => "merge",
            BaseChangeKind::Rebase => "rebase",
            BaseChangeKind::CherryPick => "cherry-pick",
            BaseChangeKind::Revert => "revert",
            BaseChangeKind::Reset => "reset",
            BaseChangeKind::ApplyMailbox => "apply-mailbox",
            BaseChangeKind::Other => "base-changed",
        }
    }
}

/// 기준점 변경 처리 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseChangeOutcome {
    /// 새 journal session을 시작한 창과 그 journal 하한(창 id 순).
    pub sessions: Vec<(WindowSessionId, u64)>,
    /// 봉인한 active journal(창 id 순). 열려 있지 않은 창의 journal도 포함합니다.
    pub sealed: Vec<SealedJournal>,
}

fn session_error(error: HistoryError) -> JournalStoreError {
    match error {
        HistoryError::Replaying => JournalStoreError::Replaying,
        _ => JournalStoreError::PreviewActive,
    }
}

/// active journal 파일 이름(`<window>.stlhjnl`)에서 창 id를 읽습니다. 정확히 같은 형식으로 다시 쓴 이름일
/// 때만 인정합니다(임시 파일·guard·다른 파일 제외).
pub(crate) fn parse_active_name(name: &str) -> Option<WindowSessionId> {
    let stem = name.strip_suffix(&format!(".{JOURNAL_EXTENSION}"))?;
    let window = WindowSessionId::from_stable_id(stem.parse::<StableId>().ok()?);
    (format!("{window}.{JOURNAL_EXTENSION}") == name).then_some(window)
}

impl JournalStore {
    /// active journal이 있는 창 목록(창 id 순). 임시 파일·guard·봉인 디렉터리·형식이 아닌 이름은 제외합니다.
    pub fn list_active(&self) -> Result<Vec<WindowSessionId>, JournalStoreError> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.dir())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Some(window) = entry.file_name().to_str().and_then(parse_active_name) {
                out.push(window);
            }
        }
        out.sort();
        Ok(out)
    }

    /// 기준점 변경(checkout·merge·rebase 등) 뒤 history 혼합을 막습니다(모듈 설명 참고): 열린 창마다 새
    /// journal session을 시작하고, active journal을 모두 `kind` 사유로 봉인합니다.
    ///
    /// preview·undo 재생 중인 창이 하나라도 있으면 아무것도 바꾸지 않고 거부합니다(caller가 preview를
    /// 취소한 뒤 다시 부름). Git 추적 금지 검사도 먼저 합니다.
    pub fn seal_for_base_change<'a, I>(
        &self,
        kind: BaseChangeKind,
        windows: I,
    ) -> Result<BaseChangeOutcome, JournalStoreError>
    where
        I: IntoIterator<Item = &'a mut WindowHistory>,
    {
        let mut windows: Vec<&'a mut WindowHistory> = windows.into_iter().collect();
        self.check_untracked()?;
        for window in &windows {
            window
                .history()
                .check_journal_session()
                .map_err(session_error)?;
        }
        let mut sessions = Vec::with_capacity(windows.len());
        for window in &mut windows {
            let floor = window
                .history_mut()
                .begin_journal_session()
                .map_err(session_error)?;
            sessions.push((window.window(), floor));
        }
        sessions.sort();
        let mut sealed = Vec::new();
        for window in self.list_active()? {
            if let Some(journal) = self.seal(window, SealReason::BaseChanged(kind))? {
                sealed.push(journal);
            }
        }
        Ok(BaseChangeOutcome { sessions, sealed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{GroupingKey, TextInputKind};
    use crate::grouping::HistoryGroupingPolicy;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::journal::tests::base;
    use crate::journal_restore::RestoreOutcome;
    use crate::journal_store::JournalLocation;
    use crate::test_fs::TempDir;
    use stellar_commands::command::Command;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_commands::state::DocumentState;
    use stellar_core_types::clock::ManualClock;

    fn snapshot(doc: &DocumentState) -> String {
        model_snapshot_json(doc.model()).unwrap()
    }

    fn reopen(doc: &DocumentState) -> DocumentState {
        let (model, _) = DocumentModel::from_records(doc.model().to_records().unwrap()).unwrap();
        DocumentState::new(model)
    }

    fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(sid(800 + n))
    }

    fn rename(
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        id: u64,
        node: u64,
        name: &str,
    ) {
        let t = tx(doc, id, Command::rename_node(sid(node), name));
        history.record_document(doc, &t, None).unwrap();
    }

    fn policy() -> HistoryGroupingPolicy {
        HistoryGroupingPolicy::default()
    }

    #[test]
    fn checkout_seals_journals_and_later_journals_do_not_mix() {
        // 검증 기준(XXV.1.4): history 혼합 없음.
        let project = TempDir::new("base-change-project");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let w1 = window(1);
        let w2 = window(2);
        // 기준점 A에서 편집 2개를 저장 시점에 기록합니다. 닫힌 창 w2의 journal도 디렉터리에 있습니다.
        let mut doc = small_state();
        let original = snapshot(&doc);
        let mut history = WindowHistory::new(w1);
        rename(&mut history, &mut doc, 1_000, 11, "a1");
        rename(&mut history, &mut doc, 1_001, 12, "a2");
        let a_journal = store.record_save_point(&history, base(5)).unwrap();
        let a_bytes = std::fs::read(&a_journal).unwrap();
        let mut closed = WindowHistory::new(w2);
        let mut closed_doc = small_state();
        rename(&mut closed, &mut closed_doc, 1_100, 13, "closed");
        let closed_bytes =
            std::fs::read(store.record_save_point(&closed, base(5)).unwrap()).unwrap();
        drop(closed);
        // 무관한 파일은 건드리지 않습니다.
        std::fs::write(store.dir().join("notes.txt"), b"x").unwrap();
        assert_eq!(store.list_active().unwrap(), vec![w1, w2]);

        // checkout: 창 문서를 B snapshot으로 바꾸는 synthetic command를 기록한 직후 부릅니다.
        rename(&mut history, &mut doc, 1_002, 11, "B-branch");
        let after_checkout = snapshot(&doc);
        let floor_expected = history.history().next_sequence();
        let outcome = store
            .seal_for_base_change(BaseChangeKind::Checkout, [&mut history])
            .unwrap();
        assert_eq!(outcome.sessions, vec![(w1, floor_expected)]);
        assert_eq!(history.history().journal_floor(), floor_expected);
        assert_eq!(outcome.sealed.len(), 2);
        for (sealed, (w, bytes)) in outcome
            .sealed
            .iter()
            .zip([(w1, &a_bytes), (w2, &closed_bytes)])
        {
            assert_eq!(sealed.window, w);
            assert_eq!(sealed.index, 1);
            assert_eq!(
                sealed.reason,
                SealReason::BaseChanged(BaseChangeKind::Checkout)
            );
            assert!(sealed
                .path
                .to_str()
                .unwrap()
                .ends_with(".000001.checkout.stlhjnl"));
            assert_eq!(&std::fs::read(&sealed.path).unwrap(), bytes, "bytes 보존");
        }
        assert!(store.list_active().unwrap().is_empty());
        assert!(store.dir().join("notes.txt").exists());
        assert_eq!(store.list_sealed().unwrap(), outcome.sealed);

        // 기준점 B에서 새 편집 뒤 저장: journal에는 기준점 변경 뒤 항목만 있습니다.
        rename(&mut history, &mut doc, 1_003, 12, "b1");
        let b_state = snapshot(&doc);
        store.record_save_point(&history, base(6)).unwrap();
        let journal = store.read(w1).unwrap().unwrap();
        assert_eq!(journal.undo.len(), 1);
        assert!(journal.redo.is_empty());
        assert!(journal
            .undo
            .iter()
            .all(|e| e.sequence.get() >= floor_expected));
        assert_eq!(journal.next_sequence, history.history().next_sequence());
        // 메모리 history는 그대로라 같은 세션에서는 checkout synthetic command까지 undo할 수 있습니다(§22.5).
        assert_eq!(history.history().len(), 4);

        // 재실행 후 B로 복원: undo는 기준점 변경 직후 상태에서 멈춥니다(A의 항목 없음).
        let mut reopened = reopen(&doc);
        let restored = store.restore_window(w1, base(6), policy()).unwrap();
        assert_eq!(
            restored.outcome,
            RestoreOutcome::Restored { undo: 1, redo: 0 }
        );
        let mut restored = restored.history;
        restored.undo(&mut reopened).unwrap().unwrap();
        assert_eq!(snapshot(&reopened), after_checkout);
        assert_eq!(restored.undo(&mut reopened).unwrap(), None);
        restored.redo(&mut reopened).unwrap().unwrap();
        assert_eq!(snapshot(&reopened), b_state);

        // B → A로 다시 checkout(같은 base 5로 돌아옴): B journal도 봉인되고, 이전 봉인 session은 복원되지
        // 않습니다.
        let outcome = store
            .seal_for_base_change(BaseChangeKind::Checkout, [&mut history])
            .unwrap();
        assert_eq!(outcome.sealed.len(), 1);
        assert_eq!(outcome.sealed[0].index, 2);
        let back = store.restore_window(w1, base(5), policy()).unwrap();
        assert_eq!(back.outcome, RestoreOutcome::Fresh);
        assert_eq!(store.list_sealed().unwrap().len(), 3);
        // 메모리 history 전체 undo는 같은 세션에서 최초 문서로 돌아갑니다.
        while history.undo(&mut doc).unwrap().is_some() {}
        assert_eq!(snapshot(&doc), original);
    }

    #[test]
    fn groups_close_and_redo_before_floor_is_not_journaled() {
        let project = TempDir::new("base-change-project-2");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let w = window(3);
        let mut doc = small_state();
        let mut history = WindowHistory::new(w);
        let clock = ManualClock::new(0);
        let key = GroupingKey::TextInput {
            node: sid(13),
            kind: TextInputKind::Insert,
            caret_before: 0,
            caret_after: 1,
        };
        let typed =
            |doc: &DocumentState, id, text: &str| tx(doc, id, Command::rename_node(sid(13), text));
        let t = typed(&doc, 2_000, "a");
        history.record_grouped(&mut doc, &t, key, &clock).unwrap();
        let before_rebase = snapshot(&doc);
        let outcome = store
            .seal_for_base_change(BaseChangeKind::Rebase, [&mut history])
            .unwrap();
        assert!(outcome.sealed.is_empty(), "active journal 없음");
        let floor = outcome.sessions[0].1;
        // 같은 key·같은 시각 입력도 기준점 변경 전 항목에 합쳐지지 않습니다.
        let key2 = GroupingKey::TextInput {
            node: sid(13),
            kind: TextInputKind::Insert,
            caret_before: 1,
            caret_after: 2,
        };
        let t = typed(&doc, 2_001, "ab");
        history.record_grouped(&mut doc, &t, key2, &clock).unwrap();
        assert_eq!(history.history().len(), 2, "새 항목");
        let after_new = snapshot(&doc);
        let (undo, redo) = history.history().journal_stacks();
        assert_eq!((undo.len(), redo.len()), (1, 0));
        assert_eq!(undo[0].sequence.get(), floor);

        // 하한 이전까지 undo: 위치가 기준점 변경 전이라 journal은 비어 있습니다.
        history.undo(&mut doc).unwrap().unwrap();
        history.undo(&mut doc).unwrap().unwrap();
        let (undo, redo) = history.history().journal_stacks();
        assert!(undo.is_empty() && redo.is_empty());
        store.record_save_point(&history, base(7)).unwrap();
        let journal = store.read(w).unwrap().unwrap();
        assert!(journal.undo.is_empty() && journal.redo.is_empty());
        // 하한 위치로 redo: 하한 뒤 항목은 redo로 기록됩니다.
        history.redo(&mut doc).unwrap().unwrap();
        assert_eq!(snapshot(&doc), before_rebase);
        store.record_save_point(&history, base(8)).unwrap();
        let mut reopened = reopen(&doc);
        let restored = store.restore_window(w, base(8), policy()).unwrap();
        assert_eq!(
            restored.outcome,
            RestoreOutcome::Restored { undo: 0, redo: 1 }
        );
        let mut restored = restored.history;
        restored.redo(&mut reopened).unwrap().unwrap();
        assert_eq!(snapshot(&reopened), after_new);
        assert!(restored.undo(&mut reopened).unwrap().is_some());
        assert_eq!(snapshot(&reopened), before_rebase);
        assert_eq!(restored.undo(&mut reopened).unwrap(), None);
    }

    #[test]
    fn preview_or_replay_refuses_without_side_effects_and_retry_seals_rest() {
        let project = TempDir::new("base-change-project-3");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let mut doc = small_state();
        let mut idle = WindowHistory::new(window(4));
        rename(&mut idle, &mut doc, 3_000, 11, "x");
        store.record_save_point(&idle, base(5)).unwrap();
        let mut dragging = WindowHistory::new(window(5));
        let mut drag_doc = small_state();
        dragging.begin_pointer_gesture(1).unwrap();
        let t = tx(&drag_doc, 3_001, Command::set_visibility(sid(12), false));
        dragging.update_preview(&mut drag_doc, &t).unwrap();
        // preview 중인 창이 있으면 다른 창도 바꾸지 않고, 봉인도 하지 않습니다.
        assert_eq!(
            store.seal_for_base_change(BaseChangeKind::Merge, [&mut idle, &mut dragging]),
            Err(JournalStoreError::PreviewActive)
        );
        assert_eq!(idle.history().journal_floor(), 0);
        assert_eq!(dragging.history().journal_floor(), 0);
        assert_eq!(store.list_active().unwrap(), vec![window(4)]);
        assert!(store.list_sealed().unwrap().is_empty());
        // undo 재생 hook 안에서는 session을 바꿀 수 없습니다.
        idle.undo_with(&mut doc, |history, _, _| {
            assert_eq!(
                history.check_journal_session(),
                Err(HistoryError::Replaying)
            );
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            session_error(HistoryError::Replaying),
            JournalStoreError::Replaying
        );
        // preview를 취소하면 진행됩니다. 창 없이 다시 부르면 남은 active journal만 봉인합니다(재시도).
        dragging.cancel_preview(&mut drag_doc).unwrap();
        let outcome = store
            .seal_for_base_change(BaseChangeKind::Merge, [&mut idle, &mut dragging])
            .unwrap();
        assert_eq!(outcome.sessions.len(), 2);
        assert_eq!(outcome.sealed.len(), 1);
        let retry = store
            .seal_for_base_change(BaseChangeKind::Merge, std::iter::empty())
            .unwrap();
        assert!(retry.sessions.is_empty() && retry.sealed.is_empty());
        // token은 모두 다르고 봉인 이름 parser가 되읽습니다.
        let tokens: std::collections::BTreeSet<_> =
            SealReason::ALL.iter().map(|r| r.token()).collect();
        assert_eq!(tokens.len(), SealReason::ALL.len());
        for kind in BaseChangeKind::ALL {
            let reason = SealReason::BaseChanged(kind);
            assert_eq!(SealReason::from_token(reason.token()), Some(reason));
            let name = format!("{}.000003.{}.stlhjnl", window(1), kind.token());
            assert_eq!(
                crate::journal_restore::parse_sealed_name(&name),
                Some((window(1), 3, reason))
            );
        }
        // active 이름 parser: 정확한 형식만.
        assert_eq!(
            parse_active_name(&format!("{}.stlhjnl", window(1))),
            Some(window(1))
        );
        for bad in [
            ".tmp.stlhjnl".to_string(),
            format!(".{}.stlhjnl.tmp-1-0", window(1)),
            format!("{}.stlhjnl", window(1)).to_uppercase(),
            "notes.txt".to_string(),
        ] {
            assert_eq!(parse_active_name(&bad), None, "{bad}");
        }
    }
}
