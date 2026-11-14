//! journal retention 크기·나이 정책과 compaction (XXV.1.5, 로드맵).
//!
//! history journal 디렉터리가 끝없이 커지지 않도록 세 경계를 둡니다([`JournalRetention`]).
//!
//! - **journal 하나**: 기록할 때마다(`JournalStore::write`) 항목 수·bytes 상한을 넘으면 현재 위치에서 가장 먼
//!   항목부터 버립니다([`HistoryJournal::encode_bounded`]). 남은 항목은 현재 위치 앞뒤로 연속이라 그대로
//!   undo·redo할 수 있습니다. 메모리 history는 줄이지 않습니다(세션 안 undo는 그대로).
//! - **나이**: 봉인 session은 봉인 시각(파일 수정 시각, 봉인 때 갱신)부터 `max_sealed_age`, 열려 있지 않은
//!   창의 active journal은 마지막 기록부터 `max_active_age`가 지나면 지웁니다. 크래시로 남은 임시 파일은
//!   `temp_grace`가 지나면 지웁니다.
//! - **개수·전체 bytes**: 봉인 session 수가 `max_sealed_sessions`, 디렉터리 journal bytes 합(active +
//!   봉인)이 `max_total_bytes`를 넘으면 가장 오래된 봉인 session부터, 그다음 열려 있지 않은 창의 가장
//!   오래된 active journal을 지웁니다. **열린 창의 active journal은 지우지 않습니다**(그 창의 다음 저장
//!   시점 기록을 덮어쓰지 않기 위함). 그래서 정리 뒤 전체 bytes는 `max(max_total_bytes, 열린 창 수 ×
//!   max_journal_bytes)` 이하입니다.
//!
//! 정리 시점: [`JournalStore::apply_retention`]을 프로젝트를 열 때(그리고 caller가 원할 때) 부릅니다. 봉인
//! 경로(XXV.1.3·XXV.1.4)는 봉인할 때마다 봉인 session 경계를 바로 적용합니다(방금 봉인한 session은 유지,
//! best-effort). journal 디렉터리의 다른 파일(guard `.gitignore`, 사용자가 둔 파일)은 세지도 지우지도
//! 않습니다. 나이는 파일 수정 시각(wall clock)으로 재며, 미래 시각은 나이 0으로 봅니다.
//!
//! 기본값은 잠정치입니다. 실제 사용 데이터·디스크 사용량 측정으로 조정합니다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use stellar_commands::id::WindowSessionId;

use crate::journal::{JournalAccess, JournalCompaction, JOURNAL_EMPTY_LEN, JOURNAL_SCHEMA_VERSION};
use crate::journal_restore::SealedJournal;
use crate::journal_store::{JournalStore, JournalStoreError, TEMP_MARKER};

/// 하루.
pub const DAY: Duration = Duration::from_secs(86_400);
/// 기본 journal 하나 상한 bytes(32 MiB).
pub const DEFAULT_MAX_JOURNAL_BYTES: u64 = 32 << 20;
/// 기본 journal 하나 항목 상한.
pub const DEFAULT_MAX_JOURNAL_ENTRIES: usize = 10_000;
/// 기본 디렉터리 전체 상한 bytes(256 MiB).
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 256 << 20;
/// 기본 봉인 session 수 상한.
pub const DEFAULT_MAX_SEALED_SESSIONS: usize = 64;

/// journal retention 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRetention {
    /// journal 하나의 최대 bytes. 넘으면 기록 시 compaction합니다.
    pub max_journal_bytes: u64,
    /// journal 하나의 최대 항목 수(undo + redo).
    pub max_journal_entries: usize,
    /// 디렉터리 journal bytes 합(active + 봉인)의 상한.
    pub max_total_bytes: u64,
    /// 봉인 session 수 상한.
    pub max_sealed_sessions: usize,
    /// 봉인 session 최대 나이(봉인 시각부터).
    pub max_sealed_age: Duration,
    /// 열려 있지 않은 창 active journal의 최대 나이(마지막 기록부터).
    pub max_active_age: Duration,
    /// 남은 임시 파일을 지우기 전 유예(다른 기록이 진행 중일 수 있음).
    pub temp_grace: Duration,
}

impl Default for JournalRetention {
    fn default() -> JournalRetention {
        JournalRetention {
            max_journal_bytes: DEFAULT_MAX_JOURNAL_BYTES,
            max_journal_entries: DEFAULT_MAX_JOURNAL_ENTRIES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_sealed_sessions: DEFAULT_MAX_SEALED_SESSIONS,
            max_sealed_age: 30 * DAY,
            max_active_age: 90 * DAY,
            temp_grace: Duration::from_secs(3_600),
        }
    }
}

impl JournalRetention {
    /// 정책 값이 서로 맞는지 확인합니다.
    pub fn validate(&self) -> Result<(), JournalStoreError> {
        let invalid = |reason| Err(JournalStoreError::InvalidRetention { reason });
        if self.max_journal_bytes < JOURNAL_EMPTY_LEN as u64 {
            return invalid("journal 하나 상한이 빈 journal 길이보다 작습니다");
        }
        if self.max_total_bytes < self.max_journal_bytes {
            return invalid("전체 상한이 journal 하나 상한보다 작습니다");
        }
        Ok(())
    }
}

/// 정리 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionReport {
    /// 지운 임시 파일 수.
    pub removed_temp: usize,
    /// 지운 active journal의 창(창 id 순).
    pub removed_active: Vec<WindowSessionId>,
    /// 지운 봉인 session(창, 순번 순).
    pub removed_sealed: Vec<SealedJournal>,
    /// compaction한 active journal(창 id 순).
    pub compacted: Vec<(WindowSessionId, JournalCompaction)>,
    /// 정리 뒤 journal bytes 합(active + 봉인).
    pub total_bytes: u64,
    /// 정리 뒤 봉인 session 수.
    pub sealed_sessions: usize,
    /// 정리 뒤 모든 경계 안인지. 열린 창 journal만으로 전체 상한을 넘으면 `false`입니다.
    pub within_budget: bool,
}

#[derive(Debug, Clone, Copy)]
struct FileInfo {
    len: u64,
    modified: SystemTime,
}

fn file_info(path: &Path, now: SystemTime) -> Result<FileInfo, JournalStoreError> {
    let metadata = std::fs::metadata(path)?;
    Ok(FileInfo {
        len: metadata.len(),
        // 수정 시각을 줄 수 없는 파일시스템은 지금(나이 0)으로 봅니다.
        modified: metadata.modified().unwrap_or(now),
    })
}

fn age(now: SystemTime, modified: SystemTime) -> Duration {
    now.duration_since(modified).unwrap_or(Duration::ZERO)
}

/// 파일을 지웁니다. 이미 없으면 성공입니다.
fn remove_file(path: &Path) -> Result<(), JournalStoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// 봉인 session 정리 결과(내부).
struct SealedPrune {
    removed: Vec<SealedJournal>,
    remaining: usize,
    remaining_bytes: u64,
}

impl JournalStore {
    /// `dir` 안에서 `temp_grace`보다 오래된 journal 임시 파일(`.<name>.tmp-…`)을 지웁니다.
    fn remove_stale_temp(&self, dir: &Path, now: SystemTime) -> Result<usize, JournalStoreError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut removed = 0;
        for entry in entries {
            let entry = entry?;
            let is_temp = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name.contains(TEMP_MARKER));
            if !is_temp || !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if age(now, file_info(&path, now)?.modified) > self.retention().temp_grace {
                remove_file(&path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// active journal bytes 합.
    fn active_bytes(&self) -> Result<u64, JournalStoreError> {
        let mut total = 0;
        for window in self.list_active()? {
            total += std::fs::metadata(self.journal_path(window))?.len();
        }
        Ok(total)
    }

    /// 봉인 session에 나이·개수·전체 bytes 경계를 적용합니다. `keep`(방금 봉인한 파일)은 지우지 않고,
    /// `other_bytes`는 전체 bytes 상한 계산에 더할 active journal bytes입니다.
    fn prune_sealed(
        &self,
        now: SystemTime,
        keep: Option<&Path>,
        other_bytes: u64,
    ) -> Result<SealedPrune, JournalStoreError> {
        let retention = self.retention().clone();
        let mut removed = Vec::new();
        let mut sealed = Vec::new();
        for journal in self.list_sealed()? {
            let info = file_info(&journal.path, now)?;
            let kept = keep == Some(journal.path.as_path());
            if !kept && age(now, info.modified) > retention.max_sealed_age {
                remove_file(&journal.path)?;
                removed.push(journal);
            } else {
                sealed.push((journal, info, kept));
            }
        }
        // 가장 오래된 것부터(같은 시각이면 창·순번 순).
        sealed.sort_by(|(a, ai, _), (b, bi, _)| {
            (ai.modified, a.window, a.index).cmp(&(bi.modified, b.window, b.index))
        });
        let mut bytes: u64 = sealed.iter().map(|(_, info, _)| info.len).sum();
        while sealed.len() > retention.max_sealed_sessions
            || other_bytes + bytes > retention.max_total_bytes
        {
            let Some(pos) = sealed.iter().position(|(_, _, kept)| !kept) else {
                break;
            };
            let (journal, info, _) = sealed.remove(pos);
            remove_file(&journal.path)?;
            bytes -= info.len;
            removed.push(journal);
        }
        Ok(SealedPrune {
            removed,
            remaining: sealed.len(),
            remaining_bytes: bytes,
        })
    }

    /// 봉인 직후 봉인 session 경계를 적용합니다(방금 봉인한 `path`는 유지). 실패해도 봉인은 끝났으므로
    /// 무시하고, 다음 [`JournalStore::apply_retention`]이 다시 정리합니다.
    pub(crate) fn prune_after_seal(&self, path: &Path) {
        let _ = self
            .active_bytes()
            .and_then(|active| self.prune_sealed(SystemTime::now(), Some(path), active));
    }

    /// 상한을 넘는 active journal을 다시 기록해 줄입니다. 읽을 수 없거나 이 writer가 이어 기록할 수 없으면
    /// 그대로 둡니다(복원 때 봉인됨, XXV.1.3). compaction은 새 기록이 아니므로 나이 기준인 수정 시각을
    /// `modified`(원래 값)로 되돌립니다(best-effort).
    fn compact_active(
        &self,
        window: WindowSessionId,
        modified: SystemTime,
    ) -> Result<Option<JournalCompaction>, JournalStoreError> {
        let Ok(Some(journal)) = self.read(window) else {
            return Ok(None);
        };
        if journal.header.access(JOURNAL_SCHEMA_VERSION) != Ok(JournalAccess::Editable) {
            return Ok(None);
        }
        let (path, compaction) = self.write_reporting(&journal)?;
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(modified));
        Ok(Some(compaction))
    }

    /// retention 정책을 적용합니다(모듈 설명 참고). `now`는 현재 wall clock, `open_windows`는 지금 열려 있는
    /// 창입니다(그 active journal은 지우지 않음). journal 기록과 같은 thread에서 부릅니다.
    pub fn apply_retention(
        &self,
        now: SystemTime,
        open_windows: &[WindowSessionId],
    ) -> Result<RetentionReport, JournalStoreError> {
        self.check_untracked()?;
        let retention = self.retention().clone();
        let open: BTreeSet<WindowSessionId> = open_windows.iter().copied().collect();
        let mut report = RetentionReport::default();
        for dir in [self.dir().to_path_buf(), self.sealed_dir()] {
            report.removed_temp += self.remove_stale_temp(&dir, now)?;
        }

        // active: 나이 초과(열린 창 제외) 삭제, 상한 초과 compaction.
        let mut active: Vec<(WindowSessionId, PathBuf, FileInfo)> = Vec::new();
        for window in self.list_active()? {
            let path = self.journal_path(window);
            let mut info = file_info(&path, now)?;
            if !open.contains(&window) && age(now, info.modified) > retention.max_active_age {
                remove_file(&path)?;
                report.removed_active.push(window);
                continue;
            }
            if info.len > retention.max_journal_bytes {
                if let Some(compaction) = self.compact_active(window, info.modified)? {
                    report.compacted.push((window, compaction));
                    info = file_info(&path, now)?;
                }
            }
            active.push((window, path, info));
        }
        let mut active_bytes: u64 = active.iter().map(|(_, _, info)| info.len).sum();

        // 봉인 session: 나이·개수·전체 bytes.
        let pruned = self.prune_sealed(now, None, active_bytes)?;
        report.removed_sealed = pruned.removed;

        // 그래도 넘치면 열려 있지 않은 창의 가장 오래된 active journal부터.
        active.sort_by_key(|(window, _, info)| (info.modified, *window));
        while active_bytes + pruned.remaining_bytes > retention.max_total_bytes {
            let Some(pos) = active
                .iter()
                .position(|(window, _, _)| !open.contains(window))
            else {
                break;
            };
            let (window, path, info) = active.remove(pos);
            remove_file(&path)?;
            active_bytes -= info.len;
            report.removed_active.push(window);
        }

        report.removed_active.sort();
        report
            .removed_sealed
            .sort_by_key(|journal| (journal.window, journal.index));
        report.total_bytes = active_bytes + pruned.remaining_bytes;
        report.sealed_sessions = pruned.remaining;
        report.within_budget = report.total_bytes <= retention.max_total_bytes
            && pruned.remaining <= retention.max_sealed_sessions;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grouping::HistoryGroupingPolicy;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::journal::tests::base;
    use crate::journal::HistoryJournal;
    use crate::journal_base_change::BaseChangeKind;
    use crate::journal_restore::{RestoreOutcome, SealReason};
    use crate::journal_store::JournalLocation;
    use crate::test_fs::TempDir;
    use crate::window::WindowHistory;
    use stellar_commands::command::Command;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_commands::state::DocumentState;

    fn snapshot(doc: &DocumentState) -> String {
        model_snapshot_json(doc.model()).unwrap()
    }

    fn reopen(doc: &DocumentState) -> DocumentState {
        let (model, _) = DocumentModel::from_records(doc.model().to_records().unwrap()).unwrap();
        DocumentState::new(model)
    }

    fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(sid(900 + n))
    }

    /// 편집 `edits`개 뒤 `undone`개를 undo한 창, 문서, 각 편집 뒤 snapshot(0 = 최초).
    fn session(
        w: WindowSessionId,
        edits: usize,
        undone: usize,
    ) -> (WindowHistory, DocumentState, Vec<String>) {
        let mut history = WindowHistory::new(w);
        let mut doc = small_state();
        let mut states = vec![snapshot(&doc)];
        for i in 0..edits {
            let node = 11 + (i as u64 % 3);
            let t = tx(
                &doc,
                10_000 + i as u64,
                Command::rename_node(sid(node), format!("편집 {i}")),
            );
            history.record_document(&mut doc, &t, None).unwrap();
            states.push(snapshot(&doc));
        }
        for _ in 0..undone {
            history.undo(&mut doc).unwrap().unwrap();
        }
        (history, doc, states)
    }

    fn set_age(path: &Path, now: SystemTime, age: Duration) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
    }

    fn small(retention: JournalRetention) -> JournalRetention {
        retention.validate().unwrap();
        retention
    }

    fn journal_bytes_on_disk(store: &JournalStore) -> u64 {
        let active: u64 = store
            .list_active()
            .unwrap()
            .into_iter()
            .map(|w| std::fs::metadata(store.journal_path(w)).unwrap().len())
            .sum();
        let sealed: u64 = store
            .list_sealed()
            .unwrap()
            .iter()
            .map(|s| std::fs::metadata(&s.path).unwrap().len())
            .sum();
        active + sealed
    }

    #[test]
    fn compaction_keeps_entries_nearest_the_current_position() {
        // 편집 40개 중 10개 undo: undo 30, redo 10. 항목 상한 12 → 현재 위치 앞뒤 6개씩.
        let w = window(1);
        let (history, doc, states) = session(w, 40, 10);
        let full = HistoryJournal::capture(w, base(5), history.history());
        let (bytes, compaction) = full.encode_bounded(u64::MAX, 12);
        assert_eq!(
            compaction,
            JournalCompaction {
                dropped_undo: 24,
                dropped_redo: 4,
                encoded_len: bytes.len() as u64,
            }
        );
        let bounded = HistoryJournal::decode(&bytes).unwrap();
        assert_eq!((bounded.undo.len(), bounded.redo.len()), (6, 6));
        assert_eq!(bounded.undo[..], full.undo[24..]);
        assert_eq!(bounded.redo[..], full.redo[4..]);
        assert_eq!(bounded.next_sequence, full.next_sequence);
        let mut compacted = full.clone();
        assert_eq!(compacted.compact(u64::MAX, 12), compaction);
        assert_eq!(compacted, bounded);
        assert_eq!(
            compacted.encode(),
            bytes,
            "in-place와 bounded encoding 동일"
        );

        // bytes 상한도 같은 규칙: 상한 이하, 실제 길이와 계산 길이 일치.
        let limit = (full.encode().len() / 3) as u64;
        let (bytes, compaction) = full.encode_bounded(limit, usize::MAX);
        assert!(bytes.len() as u64 <= limit && compaction.is_compacted());
        assert_eq!(compaction.encoded_len, bytes.len() as u64);
        HistoryJournal::decode(&bytes).unwrap();
        // 항목 하나보다 작은 상한이면 빈 journal(header만)입니다.
        let (bytes, compaction) = full.encode_bounded(JOURNAL_EMPTY_LEN as u64, usize::MAX);
        assert_eq!(bytes.len(), JOURNAL_EMPTY_LEN);
        assert_eq!((compaction.dropped_undo, compaction.dropped_redo), (30, 10));
        let empty = HistoryJournal::decode(&bytes).unwrap();
        assert!(empty.undo.is_empty() && empty.redo.is_empty());
        // 상한 안이면 그대로입니다.
        let (bytes, compaction) = full.encode_bounded(u64::MAX, usize::MAX);
        assert_eq!(bytes, full.encode());
        assert!(!compaction.is_compacted());

        // 저장소가 기록할 때 compaction하고, 복원한 history는 남은 범위에서 정확히 undo·redo합니다.
        let project = TempDir::new("retention-compact");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal)
            .unwrap()
            .with_retention(small(JournalRetention {
                max_journal_entries: 12,
                ..JournalRetention::default()
            }))
            .unwrap();
        let (_, written) = store
            .write_reporting(&HistoryJournal::capture(w, base(5), history.history()))
            .unwrap();
        assert_eq!(written, compaction_of(&full, 12));
        let mut reopened = reopen(&doc);
        assert_eq!(snapshot(&reopened), states[30]);
        let restored = store
            .restore_window(w, base(5), HistoryGroupingPolicy::default())
            .unwrap();
        assert_eq!(
            restored.outcome,
            RestoreOutcome::Restored { undo: 6, redo: 6 }
        );
        let mut restored = restored.history;
        while restored.undo(&mut reopened).unwrap().is_some() {}
        assert_eq!(snapshot(&reopened), states[24]);
        while restored.redo(&mut reopened).unwrap().is_some() {}
        assert_eq!(snapshot(&reopened), states[36]);
        // 메모리 history는 줄지 않습니다.
        assert_eq!(history.history().len(), 30);
        assert_eq!(history.history().redo_len(), 10);
    }

    fn compaction_of(journal: &HistoryJournal, max_entries: usize) -> JournalCompaction {
        journal.encode_bounded(u64::MAX, max_entries).1
    }

    #[test]
    fn retention_removes_expired_and_over_budget_files_but_keeps_open_windows() {
        let project = TempDir::new("retention-sweep");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let now = SystemTime::now();
        let open = window(10);
        let old_closed = window(11);
        let fresh_closed = window(12);
        let big_closed = window(13);
        for (w, edits) in [
            (open, 3),
            (old_closed, 3),
            (fresh_closed, 3),
            (big_closed, 40),
        ] {
            store
                .record_save_point(&session(w, edits, 0).0, base(5))
                .unwrap();
        }
        // 봉인 session 5개(창 20~24), 나이 1·2·3·4일과 40일.
        for (i, days) in [1u32, 2, 3, 4, 40].into_iter().enumerate() {
            let w = window(20 + i as u64);
            store
                .record_save_point(&session(w, 2, 0).0, base(5))
                .unwrap();
            let sealed = store.seal(w, SealReason::BaseMismatch).unwrap().unwrap();
            set_age(&sealed.path, now, DAY * days);
        }
        set_age(&store.journal_path(open), now, DAY * 200);
        set_age(&store.journal_path(old_closed), now, DAY * 100);
        set_age(&store.journal_path(fresh_closed), now, DAY);
        set_age(&store.journal_path(big_closed), now, DAY * 2);
        // 크래시 잔여 임시 파일(오래됨·방금)과 무관한 파일.
        let stale_temp = store.dir().join(".x.stlhjnl.tmp-1-0");
        let live_temp = store.dir().join(".y.stlhjnl.tmp-1-1");
        std::fs::write(&stale_temp, b"partial").unwrap();
        std::fs::write(&live_temp, b"partial").unwrap();
        set_age(&stale_temp, now, Duration::from_secs(7_200));
        std::fs::write(store.dir().join("notes.txt"), vec![0u8; 4096]).unwrap();

        let big_len = std::fs::metadata(store.journal_path(big_closed))
            .unwrap()
            .len();
        let one_sealed = std::fs::metadata(&store.list_sealed().unwrap()[0].path)
            .unwrap()
            .len();
        let retention = small(JournalRetention {
            max_journal_bytes: big_len / 2,
            max_sealed_sessions: 3,
            ..JournalRetention::default()
        });
        let store = store.with_retention(retention.clone()).unwrap();
        let report = store.apply_retention(now, &[open]).unwrap();
        assert_eq!(report.removed_temp, 1);
        assert!(!stale_temp.exists() && live_temp.exists());
        assert_eq!(
            report.removed_active,
            vec![old_closed],
            "열린 창은 200일이어도 유지"
        );
        assert_eq!(report.compacted.len(), 1);
        assert_eq!(report.compacted[0].0, big_closed);
        let big_meta = std::fs::metadata(store.journal_path(big_closed)).unwrap();
        assert!(big_meta.len() <= big_len / 2);
        assert_eq!(
            big_meta.modified().unwrap(),
            now - DAY * 2,
            "compaction은 나이를 바꾸지 않음"
        );
        // 40일 봉인(나이) + 개수 상한 3을 넘는 가장 오래된 4일 봉인.
        let removed: Vec<_> = report.removed_sealed.iter().map(|s| s.window).collect();
        assert_eq!(removed, vec![window(23), window(24)]);
        assert_eq!(report.sealed_sessions, 3);
        assert_eq!(report.total_bytes, journal_bytes_on_disk(&store));
        assert!(report.within_budget);
        assert!(store.dir().join("notes.txt").exists(), "무관한 파일 유지");
        assert_eq!(
            store.list_active().unwrap(),
            vec![open, fresh_closed, big_closed]
        );
        // 다시 적용해도 바뀌지 않습니다.
        let again = store.apply_retention(now, &[open]).unwrap();
        assert_eq!(
            (
                again.removed_temp,
                again.removed_active.len(),
                again.removed_sealed.len()
            ),
            (0, 0, 0)
        );
        assert!(again.compacted.is_empty());

        // 전체 상한: 봉인 session부터, 그다음 열려 있지 않은 창의 가장 오래된 active journal.
        let open_len = std::fs::metadata(store.journal_path(open)).unwrap().len();
        let fresh_len = std::fs::metadata(store.journal_path(fresh_closed))
            .unwrap()
            .len();
        let big_now = std::fs::metadata(store.journal_path(big_closed))
            .unwrap()
            .len();
        let store = store
            .with_retention(small(JournalRetention {
                max_journal_bytes: big_now,
                max_total_bytes: big_now + fresh_len + one_sealed / 2,
                ..retention
            }))
            .unwrap();
        let report = store.apply_retention(now, &[open]).unwrap();
        assert_eq!(report.removed_sealed.len(), 3);
        assert_eq!(
            report.removed_active,
            vec![big_closed],
            "2일 > 1일: 더 오래된 쪽"
        );
        assert_eq!(report.total_bytes, open_len + fresh_len);
        assert!(report.within_budget);
        // 열린 창 journal만으로 상한을 넘으면 지우지 않고 budget 밖으로 보고합니다.
        let store = store
            .with_retention(small(JournalRetention {
                max_journal_bytes: JOURNAL_EMPTY_LEN as u64,
                max_total_bytes: JOURNAL_EMPTY_LEN as u64,
                ..JournalRetention::default()
            }))
            .unwrap();
        let report = store.apply_retention(now, &[open, fresh_closed]).unwrap();
        assert!(report.removed_active.is_empty());
        assert_eq!(
            report.compacted.len(),
            2,
            "상한을 넘는 열린 창 journal도 compaction"
        );
        assert!(!report.within_budget);
        assert_eq!(store.list_active().unwrap(), vec![open, fresh_closed]);
    }

    #[test]
    fn repeated_sessions_stay_within_bounded_storage() {
        // 검증 기준(XXV.1.5): bounded storage. 기준점 변경·재실행 불일치로 봉인이 계속 쌓여도 경계를 지킵니다.
        let project = TempDir::new("retention-bounded");
        let retention = small(JournalRetention {
            max_journal_bytes: 16 << 10,
            max_journal_entries: 20,
            max_total_bytes: 48 << 10,
            max_sealed_sessions: 4,
            ..JournalRetention::default()
        });
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal)
            .unwrap()
            .with_retention(retention.clone())
            .unwrap();
        let w = window(30);
        let now = SystemTime::now();
        for round in 0..30u32 {
            let (mut history, _, _) = session(w, 25 + round as usize % 7, round as usize % 3);
            store.record_save_point(&history, base(5)).unwrap();
            let journal = store.read(w).unwrap().unwrap();
            assert!(
                journal.undo.len() + journal.redo.len() <= 20,
                "round {round}"
            );
            assert!(
                std::fs::metadata(store.journal_path(w)).unwrap().len()
                    <= retention.max_journal_bytes
            );
            store
                .seal_for_base_change(BaseChangeKind::Checkout, [&mut history])
                .unwrap();
            // 봉인 직후 경계가 바로 적용됩니다(방금 봉인한 session은 유지).
            let sealed = store.list_sealed().unwrap();
            assert!(
                sealed.len() <= retention.max_sealed_sessions,
                "round {round}"
            );
            assert!(journal_bytes_on_disk(&store) <= retention.max_total_bytes);
            assert!(sealed
                .iter()
                .any(|s| s.reason == SealReason::BaseChanged(BaseChangeKind::Checkout)));
            // 봉인 시각이 나이 기준입니다(수정 시각 갱신).
            let newest = sealed.iter().max_by_key(|s| s.index).unwrap();
            let modified = std::fs::metadata(&newest.path).unwrap().modified().unwrap();
            assert!(age(SystemTime::now(), modified) < DAY);
        }
        // 31일 뒤 정리하면 봉인 session이 모두 나이로 사라집니다.
        let report = store.apply_retention(now + 31 * DAY, &[]).unwrap();
        assert_eq!(report.sealed_sessions, 0);
        assert_eq!(report.total_bytes, 0);
        assert!(store.list_sealed().unwrap().is_empty());

        // 모순 정책은 거부합니다.
        for bad in [
            JournalRetention {
                max_journal_bytes: 10,
                ..JournalRetention::default()
            },
            JournalRetention {
                max_total_bytes: DEFAULT_MAX_JOURNAL_BYTES - 1,
                ..JournalRetention::default()
            },
        ] {
            assert!(matches!(
                store.clone().with_retention(bad),
                Err(JournalStoreError::InvalidRetention { .. })
            ));
        }
        assert_eq!(
            JournalStore::open(project.path(), JournalLocation::ProjectLocal)
                .unwrap()
                .retention(),
            &JournalRetention::default()
        );
    }
}
