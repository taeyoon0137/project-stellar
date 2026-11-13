//! Stellar 자신의 저장이 만든 파일 event 식별 (XIII.2.4).
//!
//! 저장(V.5)은 chunk·manifest 파일을 쓰고 마지막에 `design/current`를 새 manifest hash로 바꿉니다.
//! 파일 감시기는 이 변화도 똑같이 보고하므로, 그대로 외부 변경으로 처리하면 "저장 → 감시 event →
//! reload → …" 고리가 생깁니다. 이를 막기 위해:
//!
//! - [`OwnSaveLedger`]: 이 session이 pointer 전환까지 성공한 저장의 (generation, manifest hash)를
//!   최근 [`OWN_SAVE_HISTORY`]개 기록합니다. 가장 최근 기록([`OwnSaveLedger::latest`])이 창의
//!   현재 saved generation입니다.
//! - [`CurrentPointerTracker`]: 감시 event가 current pointer를 바꿨을 수 있으면
//!   ([`may_change_current_pointer`]) 디스크의 pointer를 다시 읽어 마지막으로 본 값과 비교하고,
//!   새 hash가 **가장 최근 자기 저장**이면 [`PointerObservation::OwnSave`], 아니면
//!   [`PointerObservation::External`]로 판정합니다.
//!
//! 판정은 파일 이름·시각이 아니라 content-addressed manifest hash(ADR-0004)로 하므로, 다른 프로세스가
//! 창의 현재 saved generation과 같은 hash의 manifest를 만들었다면 내용이 같아 다시 읽을 필요도
//! 없습니다. 반대로 외부 도구(git checkout·reset·stash 등)가 pointer를 이 session이 **예전에**
//! 저장한 generation으로 되돌리면, 디스크는 창의 현재 saved generation과 다르므로 외부 변경입니다
//! (예전 자기 저장 기록은 자기 저장 판정에 쓰지 않음).

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

use stellar_storage::generation::current::{
    current_pointer_path, read_current_pointer, CurrentPointerError,
};
use stellar_storage::hash::ContentHash;
use stellar_storage::save::commit::SavedGenerationState;

use super::event::{FileWatchEvent, WatchPath, WatchRootId};

/// 기억하는 최근 자기 저장 수.
pub const OWN_SAVE_HISTORY: usize = 64;

/// 이 session이 기록한 저장 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnSave {
    /// generation number.
    pub generation: u64,
    /// manifest content hash(current pointer에 기록된 값).
    pub manifest_hash: ContentHash,
}

/// 최근 자기 저장 기록.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnSaveLedger {
    recent: VecDeque<OwnSave>,
}

impl OwnSaveLedger {
    /// 빈 기록.
    pub fn new() -> OwnSaveLedger {
        OwnSaveLedger::default()
    }

    /// pointer 전환에 성공한 저장을 기록합니다. 이 기록이 가장 최근 자기 저장이 되며, 오래된
    /// 기록부터 버립니다.
    ///
    /// 같은 manifest hash가 이미 있으면 그 기록을 가장 최근으로 옮깁니다(같은 hash로 다시 전환한
    /// 저장도 현재 saved generation입니다). 처음 기록한 generation 번호는 유지합니다.
    pub fn record(&mut self, generation: u64, manifest_hash: ContentHash) {
        let save = match self
            .recent
            .iter()
            .position(|save| save.manifest_hash == manifest_hash)
            .and_then(|index| self.recent.remove(index))
        {
            Some(existing) => existing,
            None => OwnSave {
                generation,
                manifest_hash,
            },
        };
        if self.recent.len() == OWN_SAVE_HISTORY {
            self.recent.pop_front();
        }
        self.recent.push_back(save);
    }

    /// 저장 commit(V.5.4) 뒤의 in-memory saved generation을 기록합니다.
    ///
    /// 창의 in-memory saved generation이 바뀔 때마다(자기 저장 commit, 외부 generation reload 반영)
    /// 호출해 [`OwnSaveLedger::latest`]가 창의 현재 saved generation을 가리키게 합니다.
    pub fn record_saved(&mut self, state: &SavedGenerationState) {
        if let Some((generation, hash)) = state.saved() {
            self.record(generation, hash);
        }
    }

    /// manifest hash가 최근 기록(가장 최근이 아니어도) 중 자기 저장인지 찾습니다.
    ///
    /// pointer 판정에는 쓰지 않습니다. 예전 자기 저장으로 되돌아간 pointer는 외부 변경이기 때문입니다
    /// ([`OwnSaveLedger::latest`] 참고).
    pub fn find(&self, manifest_hash: &ContentHash) -> Option<OwnSave> {
        self.recent
            .iter()
            .find(|save| save.manifest_hash == *manifest_hash)
            .copied()
    }

    /// 가장 최근 자기 저장(창의 현재 saved generation). 기록이 없으면 `None`.
    pub fn latest(&self) -> Option<OwnSave> {
        self.recent.back().copied()
    }

    /// 기록 수.
    pub fn len(&self) -> usize {
        self.recent.len()
    }

    /// 기록이 없는지.
    pub fn is_empty(&self) -> bool {
        self.recent.is_empty()
    }
}

/// current pointer 관찰 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerObservation {
    /// 마지막으로 본 값과 같음(다시 읽을 필요 없음).
    Unchanged,
    /// 이 session의 가장 최근 저장이 만든 값.
    OwnSave(OwnSave),
    /// 다른 writer(외부 에디터·Git·다른 창 등)가 만든 새 값. 이 session이 예전에 저장한
    /// generation으로 되돌아간 값도 포함합니다.
    External {
        /// 새 manifest hash.
        manifest_hash: ContentHash,
    },
    /// pointer 파일이 없어짐.
    Missing,
    /// pointer 파일 형식이 잘못됨(손상 또는 기록 도중).
    Invalid {
        /// 원인.
        message: String,
    },
    /// pointer 파일을 읽지 못함.
    Unreadable {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

/// current pointer의 마지막 관찰 값을 추적합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentPointerTracker {
    root: PathBuf,
    last_seen: Option<ContentHash>,
}

impl CurrentPointerTracker {
    /// 프로젝트 root와 열 때 읽은 pointer 값으로 만듭니다.
    pub fn new(root: &Path, initial: Option<ContentHash>) -> CurrentPointerTracker {
        CurrentPointerTracker {
            root: root.to_path_buf(),
            last_seen: initial,
        }
    }

    /// 마지막으로 본 pointer 값.
    pub fn last_seen(&self) -> Option<ContentHash> {
        self.last_seen
    }

    /// 디스크의 pointer를 다시 읽어 판정합니다. 새 값이면 마지막 관찰 값을 갱신합니다.
    ///
    /// 새 값이 ledger의 **가장 최근** 자기 저장일 때만 [`PointerObservation::OwnSave`]입니다. 예전
    /// 자기 저장 hash로 되돌아간 값은 창이 보여 주는 generation과 다르므로
    /// [`PointerObservation::External`]입니다.
    pub fn observe(&mut self, ledger: &OwnSaveLedger) -> PointerObservation {
        match read_current_pointer(&self.root) {
            Ok(hash) if self.last_seen == Some(hash) => PointerObservation::Unchanged,
            Ok(hash) => {
                self.last_seen = Some(hash);
                match ledger.latest() {
                    Some(save) if save.manifest_hash == hash => PointerObservation::OwnSave(save),
                    _ => PointerObservation::External {
                        manifest_hash: hash,
                    },
                }
            }
            Err(CurrentPointerError::Missing { .. }) if self.last_seen.is_none() => {
                PointerObservation::Unchanged
            }
            Err(CurrentPointerError::Missing { .. }) => {
                self.last_seen = None;
                PointerObservation::Missing
            }
            Err(CurrentPointerError::InvalidFormat { message }) => {
                PointerObservation::Invalid { message }
            }
            Err(CurrentPointerError::Io { source, .. }) => PointerObservation::Unreadable {
                kind: source.kind(),
            },
        }
    }
}

/// 프로젝트 root 기준 current pointer 경로(`design/current`).
pub fn current_pointer_watch_path() -> WatchPath {
    let relative = current_pointer_path(Path::new(""))
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    // storage의 고정 상수 경로라 규칙 위반이 없지만, 만약 실패하면 root 자신으로 대신합니다.
    WatchPath::new(WatchRootId::PROJECT, relative)
        .unwrap_or_else(|_| WatchPath::root_of(WatchRootId::PROJECT))
}

/// event가 current pointer를 바꿨을 수 있는지. 넘침은 알 수 없으므로 `true`입니다.
pub fn may_change_current_pointer(event: &FileWatchEvent) -> bool {
    let pointer = current_pointer_watch_path();
    match event {
        FileWatchEvent::Overflow(_) => true,
        other => other
            .paths()
            .iter()
            .any(|path| pointer.starts_with(path) || **path == pointer),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::watcher::event::OverflowReason;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;
    use stellar_storage::chunk::envelope::ChunkKindCode;
    use stellar_storage::chunk::id::ChunkId;
    use stellar_storage::chunk::planner::LogicalChunkState;
    use stellar_storage::generation::loader::{load_current_generation, LoadedGeneration};
    use stellar_storage::save::cancel::{run_save, CancelToken};
    use stellar_storage::save::plan::build_save_plan;

    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }
    impl TempDir {
        pub(crate) fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-own-save-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(crate) const PROJECT: StableId = StableId::from_bytes([1; 16]);

    /// 실제 저장 파이프라인(V.5)으로 문서 chunk 하나를 가진 새 generation을 기록합니다.
    pub(crate) fn save_generation(
        root: &Path,
        base: Option<&LoadedGeneration>,
        payload: &[u8],
        memory: &mut SavedGenerationState,
    ) -> LoadedGeneration {
        let chunk = LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(StableId::new_v7(1, 1, 1)),
            kind: ChunkKindCode(0x8000_0001),
            schema_version: 1,
            payload: payload.to_vec(),
        };
        let plan =
            build_save_plan(base, PROJECT, 1, &[chunk], vec![StableId::new_v7(2, 2, 2)]).unwrap();
        run_save(root, &plan, memory, &CancelToken::new()).unwrap();
        load_current_generation(root, PROJECT).unwrap()
    }

    #[test]
    fn ledger_identifies_own_saves_and_evicts_old_ones() {
        let mut ledger = OwnSaveLedger::new();
        let hashes: Vec<ContentHash> = (0..OWN_SAVE_HISTORY as u64 + 1)
            .map(|i| ContentHash::compute(&i.to_le_bytes()))
            .collect();
        for (i, hash) in hashes.iter().enumerate() {
            ledger.record(i as u64 + 1, *hash);
        }
        assert_eq!(
            ledger.latest().map(|s| s.manifest_hash),
            Some(hashes[OWN_SAVE_HISTORY])
        );
        // 중복 기록은 새 항목을 만들지 않고 가장 최근으로 옮김(처음 generation 유지).
        ledger.record(999, hashes[5]);
        assert_eq!(ledger.len(), OWN_SAVE_HISTORY);
        assert_eq!(ledger.find(&hashes[0]), None, "가장 오래된 기록 제거");
        assert_eq!(ledger.find(&hashes[5]).map(|s| s.generation), Some(6));
        assert_eq!(
            ledger.latest(),
            Some(OwnSave {
                generation: 6,
                manifest_hash: hashes[5]
            })
        );
        let mut state = SavedGenerationState::new(Some((70, hashes[0])));
        ledger.record_saved(&state);
        assert!(ledger.find(&hashes[0]).is_some());
        state = SavedGenerationState::new(None);
        ledger.record_saved(&state);
        assert_eq!(ledger.len(), OWN_SAVE_HISTORY);
    }

    #[test]
    fn tracker_classifies_own_and_external_generations() {
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let mut tracker = CurrentPointerTracker::new(&dir.path, None);
        assert_eq!(tracker.observe(&ledger), PointerObservation::Unchanged);

        let g1 = save_generation(&dir.path, None, b"v1", &mut memory);
        ledger.record_saved(&memory);
        assert_eq!(
            tracker.observe(&ledger),
            PointerObservation::OwnSave(OwnSave {
                generation: 1,
                manifest_hash: g1.manifest_hash
            })
        );
        assert_eq!(tracker.observe(&ledger), PointerObservation::Unchanged);

        // 다른 writer(자기 기록에 없음)의 저장.
        let mut other = SavedGenerationState::new(memory.saved());
        let g2 = save_generation(&dir.path, Some(&g1), b"external", &mut other);
        assert_eq!(
            tracker.observe(&ledger),
            PointerObservation::External {
                manifest_hash: g2.manifest_hash
            }
        );
        assert_eq!(tracker.last_seen(), Some(g2.manifest_hash));

        // 손상·삭제.
        let pointer = current_pointer_path(&dir.path);
        std::fs::write(&pointer, b"garbage").unwrap();
        assert!(matches!(
            tracker.observe(&ledger),
            PointerObservation::Invalid { .. }
        ));
        std::fs::remove_file(&pointer).unwrap();
        assert_eq!(tracker.observe(&ledger), PointerObservation::Missing);
    }

    #[test]
    fn pointer_reverted_to_an_older_own_save_is_external() {
        use stellar_storage::generation::current::write_current_pointer;
        // 회귀(검토 지적): 외부 도구(git checkout·reset·stash)가 pointer를 이 session이 예전에
        // 저장한 generation으로 되돌리면 창(B)과 디스크(A)가 달라지므로 외부 변경이어야 합니다.
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let a = save_generation(&dir.path, None, b"A", &mut memory);
        ledger.record_saved(&memory);
        let b = save_generation(&dir.path, Some(&a), b"B", &mut memory);
        ledger.record_saved(&memory);
        let mut tracker = CurrentPointerTracker::new(&dir.path, Some(b.manifest_hash));

        write_current_pointer(&dir.path, &a.manifest_hash).unwrap();
        assert_eq!(
            tracker.observe(&ledger),
            PointerObservation::External {
                manifest_hash: a.manifest_hash
            }
        );
        // 예전 자기 저장 기록 자체는 남아 있음(판정에만 쓰지 않음).
        assert!(ledger.find(&a.manifest_hash).is_some());

        // 창이 A를 reload해 saved generation으로 반영한 뒤 pointer가 B로 돌아가면 그것도 외부 변경.
        let adopted = SavedGenerationState::new(Some((a.manifest.generation, a.manifest_hash)));
        ledger.record_saved(&adopted);
        write_current_pointer(&dir.path, &b.manifest_hash).unwrap();
        assert_eq!(
            tracker.observe(&ledger),
            PointerObservation::External {
                manifest_hash: b.manifest_hash
            }
        );

        // 창의 현재 saved generation과 같은 값은 자기 저장.
        let c = save_generation(&dir.path, Some(&b), b"C", &mut memory);
        ledger.record_saved(&memory);
        assert!(matches!(
            tracker.observe(&ledger),
            PointerObservation::OwnSave(save) if save.manifest_hash == c.manifest_hash
        ));
    }

    #[test]
    fn pointer_relevance_of_events() {
        let p = |s: &str| WatchPath::project(s).unwrap();
        assert_eq!(current_pointer_watch_path(), p("design/current"));
        for event in [
            FileWatchEvent::Modified(p("design/current")),
            FileWatchEvent::Renamed {
                from: p("design/.current.tmp-1-0"),
                to: p("design/current"),
            },
            FileWatchEvent::Removed(p("design")),
            FileWatchEvent::Modified(p("")),
            FileWatchEvent::Overflow(OverflowReason::EventsDropped),
        ] {
            assert!(may_change_current_pointer(&event), "{event:?}");
        }
        for event in [
            FileWatchEvent::Created(p("design/objects/ab/sha256-ab.starchunk")),
            FileWatchEvent::Created(p("design/generations/sha256-cd.starmanifest")),
            FileWatchEvent::Created(p("design/.current.tmp-1-0")),
            FileWatchEvent::Modified(p("design/currently")),
            FileWatchEvent::Modified(WatchPath::new(WatchRootId(1), "design/current").unwrap()),
        ] {
            assert!(!may_change_current_pointer(&event), "{event:?}");
        }
    }

    /// 감시 event를 받아 current pointer가 외부에서 바뀌었을 때만 reload하는 최소 소비자.
    #[cfg(target_os = "macos")]
    struct ReloadCounter {
        tracker: CurrentPointerTracker,
        reloads: usize,
        own: usize,
    }

    #[cfg(target_os = "macos")]
    impl ReloadCounter {
        fn feed(&mut self, events: &[FileWatchEvent], ledger: &OwnSaveLedger) {
            if !events.iter().any(may_change_current_pointer) {
                return;
            }
            match self.tracker.observe(ledger) {
                PointerObservation::External { .. } => self.reloads += 1,
                PointerObservation::OwnSave(_) => self.own += 1,
                _ => {}
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn autosave_events_cause_no_reload_loop() {
        use crate::watcher::fsevents::FsEventsWatcher;
        use crate::watcher::ProjectFileWatcher;
        use std::time::{Duration, Instant};

        let dir = TempDir::new();
        let root = dir.path.clone();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let mut base = save_generation(&root, None, b"v0", &mut memory);
        ledger.record_saved(&memory);

        let mut watcher = FsEventsWatcher::start(&root, &[]).unwrap();
        let mut consumer = ReloadCounter {
            tracker: CurrentPointerTracker::new(&root, Some(base.manifest_hash)),
            reloads: 0,
            own: 0,
        };
        let pointer = current_pointer_watch_path();
        let mut wait_for_pointer_event = |consumer: &mut ReloadCounter, ledger: &OwnSaveLedger| {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut seen_pointer = false;
            while Instant::now() < deadline && !seen_pointer {
                watcher.flush();
                let events = watcher.wait(Duration::from_millis(200));
                seen_pointer = events
                    .iter()
                    .any(|e| e.paths().iter().any(|p| **p == pointer));
                consumer.feed(&events, ledger);
            }
            assert!(seen_pointer, "current pointer event 미수신");
        };

        // autosave 5회: 매번 자기 저장으로 식별되어 reload 0.
        for i in 1..=5u8 {
            base = save_generation(&root, Some(&base), &[i; 64], &mut memory);
            ledger.record_saved(&memory);
            wait_for_pointer_event(&mut consumer, &ledger);
        }
        assert_eq!(consumer.reloads, 0, "autosave 후 reload loop");
        assert!(consumer.own >= 1);

        // 다른 writer의 저장은 외부 변경으로 정확히 한 번 reload.
        let mut other = SavedGenerationState::new(memory.saved());
        save_generation(&root, Some(&base), b"external", &mut other);
        wait_for_pointer_event(&mut consumer, &ledger);
        assert_eq!(consumer.reloads, 1);
    }
}
