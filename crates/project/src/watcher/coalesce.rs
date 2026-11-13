//! 외부 변경 event coalescing (XIII.2.5).
//!
//! 저장 한 번은 chunk·manifest·임시 파일·current pointer 등 여러 파일 event를 만듭니다. 이 event들을
//! 하나씩 처리하지 않고 **조용한 구간**(마지막 event 뒤 `quiet_window_nanos`)이 지날 때까지 모은 뒤,
//! root manifest의 **current pointer 변화 단위**로 하나의 [`ExternalChange`]로 바꿉니다.
//!
//! - 저장소 영역(`design/` — chunk·manifest·임시 파일) event만으로는 reload하지 않습니다. 그 영역의
//!   변화는 current pointer가 외부의 새 generation을 가리킬 때만 [`ExternalChange::Generation`] 하나로
//!   나옵니다(자기 저장은 XIII.2.4로 제외).
//! - 새 generation은 manifest와 모든 chunk가 갖춰졌을 때만(V.4.4 검증 통과) 내보냅니다. pointer가
//!   먼저 바뀌고 chunk가 아직 없으면 다음 event를 기다립니다(완성된 snapshot 단위 전환, 기술 아키텍처
//!   22.5). 손상된 generation은 [`ExternalChange::InvalidGeneration`]입니다.
//! - 저장소 밖 파일(`stellar.yaml`, 에셋, 외부 symlink target)의 변화는 조용한 구간마다
//!   [`ExternalChange::Files`] 하나로 묶습니다.
//! - 시간은 단조 nanoseconds로 주입합니다(`MonotonicClock`·`ManualClock`과 같은 단위).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;
use stellar_storage::generation::loader::{verify_generation, LoadError};
use stellar_storage::hash::ContentHash;

use super::event::{FileWatchEvent, WatchPath, WatchRootId};
use super::own_save::{
    current_pointer_watch_path, may_change_current_pointer, CurrentPointerTracker, OwnSaveLedger,
    PointerObservation,
};

/// coalescing 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChange {
    /// 외부 writer가 만든 새 정상 generation(조용한 구간 하나에 하나).
    Generation {
        /// 새 manifest hash.
        manifest_hash: ContentHash,
        /// generation number.
        generation: u64,
        /// 이 변화에 묶인 저장소 영역 경로(정렬, 중복 제거).
        paths: Vec<WatchPath>,
    },
    /// current pointer가 가리키는 generation이 손상됨(자동 reload 금지).
    InvalidGeneration {
        /// manifest hash.
        manifest_hash: ContentHash,
        /// 검증 실패 사유.
        reason: String,
    },
    /// current pointer를 읽을 수 없음(없음·형식 오류·I/O).
    PointerUnavailable(PointerObservation),
    /// 저장소 밖 파일 변화(정렬, 중복 제거).
    Files {
        /// 바뀐 경로.
        paths: Vec<WatchPath>,
    },
}

/// event를 모아 current pointer 변화 단위로 묶는 coalescer.
#[derive(Debug, Clone)]
pub struct ChangeCoalescer {
    root: PathBuf,
    project_id: StableId,
    quiet_window_nanos: u64,
    tracker: CurrentPointerTracker,
    storage_dir: WatchPath,
    pending_storage: BTreeSet<WatchPath>,
    pending_files: BTreeSet<WatchPath>,
    pointer_dirty: bool,
    /// pointer는 바뀌었지만 chunk·manifest가 아직 갖춰지지 않은 외부 generation.
    awaiting: Option<ContentHash>,
    last_event_at: Option<u64>,
}

impl ChangeCoalescer {
    /// 프로젝트 root, project id, 열 때 읽은 current pointer, 조용한 구간 길이로 만듭니다.
    pub fn new(
        root: &Path,
        project_id: StableId,
        initial_pointer: Option<ContentHash>,
        quiet_window_nanos: u64,
    ) -> ChangeCoalescer {
        let pointer = current_pointer_watch_path();
        let storage_dir = parent_of(&pointer);
        ChangeCoalescer {
            root: root.to_path_buf(),
            project_id,
            quiet_window_nanos,
            tracker: CurrentPointerTracker::new(root, initial_pointer),
            storage_dir,
            pending_storage: BTreeSet::new(),
            pending_files: BTreeSet::new(),
            pointer_dirty: false,
            awaiting: None,
            last_event_at: None,
        }
    }

    /// 마지막으로 확인한 current pointer.
    pub fn current_pointer(&self) -> Option<ContentHash> {
        self.tracker.last_seen()
    }

    /// 아직 묶지 않은 event가 있는지.
    pub fn has_pending(&self) -> bool {
        self.last_event_at.is_some() || self.awaiting.is_some()
    }

    /// 감시 event를 받습니다(`now`는 단조 nanoseconds).
    pub fn push(&mut self, events: &[FileWatchEvent], now: u64) {
        if events.is_empty() {
            return;
        }
        for event in events {
            if may_change_current_pointer(event) {
                self.pointer_dirty = true;
            }
            for path in event.paths() {
                if self.is_storage_path(path) {
                    self.pending_storage.insert(path.clone());
                } else {
                    self.pending_files.insert(path.clone());
                }
            }
        }
        self.last_event_at = Some(now);
    }

    /// 조용한 구간이 지났으면 모은 event를 [`ExternalChange`]로 바꿉니다.
    pub fn poll(&mut self, now: u64, ledger: &OwnSaveLedger) -> Vec<ExternalChange> {
        let Some(last) = self.last_event_at else {
            return Vec::new();
        };
        if now.saturating_sub(last) < self.quiet_window_nanos {
            return Vec::new();
        }
        self.last_event_at = None;
        let mut out = Vec::new();
        if self.pointer_dirty || self.awaiting.is_some() {
            self.pointer_dirty = false;
            self.settle_pointer(ledger, &mut out);
        }
        if self.awaiting.is_none() {
            // 기다리는 generation이 없으면 저장소 영역 event는 여기서 끝납니다.
            self.pending_storage.clear();
        }
        if !self.pending_files.is_empty() {
            out.push(ExternalChange::Files {
                paths: std::mem::take(&mut self.pending_files)
                    .into_iter()
                    .collect(),
            });
        }
        out
    }

    /// current pointer를 확인하고 외부의 완성된 generation이면 하나로 내보냅니다.
    fn settle_pointer(&mut self, ledger: &OwnSaveLedger, out: &mut Vec<ExternalChange>) {
        let candidate = match self.tracker.observe(ledger) {
            PointerObservation::External { manifest_hash } => Some(manifest_hash),
            PointerObservation::Unchanged => self.awaiting,
            PointerObservation::OwnSave(_) => None,
            other => {
                self.awaiting = None;
                out.push(ExternalChange::PointerUnavailable(other));
                return;
            }
        };
        let Some(manifest_hash) = candidate else {
            self.awaiting = None;
            return;
        };
        match verify_generation(&self.root, &manifest_hash, self.project_id) {
            Ok(loaded) => {
                self.awaiting = None;
                out.push(ExternalChange::Generation {
                    manifest_hash,
                    generation: loaded.manifest.generation,
                    paths: std::mem::take(&mut self.pending_storage)
                        .into_iter()
                        .collect(),
                });
            }
            // 아직 기록 중일 수 있음: 다음 event 뒤 다시 확인합니다.
            Err(LoadError::MissingManifest { .. } | LoadError::MissingChunk { .. }) => {
                self.awaiting = Some(manifest_hash);
            }
            Err(error) => {
                self.awaiting = None;
                out.push(ExternalChange::InvalidGeneration {
                    manifest_hash,
                    reason: error.to_string(),
                });
            }
        }
    }

    /// 저장소 영역(`design/`) 경로인지.
    fn is_storage_path(&self, path: &WatchPath) -> bool {
        path.root() == WatchRootId::PROJECT
            && path.starts_with(&self.storage_dir)
            && !path.is_root()
    }
}

/// 경로의 부모(root 자신이면 그대로).
fn parent_of(path: &WatchPath) -> WatchPath {
    match path.relative().rsplit_once('/') {
        Some((parent, _)) => {
            WatchPath::new(path.root(), parent).unwrap_or_else(|_| WatchPath::root_of(path.root()))
        }
        None => WatchPath::root_of(path.root()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watcher::own_save::tests::{save_generation, TempDir, PROJECT};
    use stellar_storage::save::commit::SavedGenerationState;

    const MS: u64 = 1_000_000;
    const WINDOW: u64 = 50 * MS;

    fn p(s: &str) -> WatchPath {
        WatchPath::project(s).unwrap()
    }

    /// 저장 한 번이 만드는 실제 순서의 event(chunk·manifest·임시 파일·pointer rename).
    fn save_events(manifest_hash: &ContentHash) -> Vec<Vec<FileWatchEvent>> {
        let manifest = format!(
            "design/generations/{}.starmanifest",
            manifest_hash.digest_hex()
        );
        vec![
            vec![FileWatchEvent::Created(p(
                "design/objects/ab/sha256-ab.starchunk",
            ))],
            vec![FileWatchEvent::Created(p(
                "design/objects/cd/sha256-cd.starchunk",
            ))],
            vec![FileWatchEvent::Created(p(&manifest))],
            vec![FileWatchEvent::Created(p("design/.current.tmp-9-0"))],
            vec![FileWatchEvent::Renamed {
                from: p("design/.current.tmp-9-0"),
                to: p("design/current"),
            }],
        ]
    }

    #[test]
    fn one_external_save_is_one_change_batch() {
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        ledger.record_saved(&memory);
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), WINDOW);

        // 다른 writer의 저장.
        let mut other = SavedGenerationState::new(memory.saved());
        let external = save_generation(&dir.path, Some(&base), b"external", &mut other);
        let mut now = 1_000 * MS;
        for batch in save_events(&external.manifest_hash) {
            coalescer.push(&batch, now);
            now += 2 * MS;
            // 조용한 구간 전에는 아무것도 내보내지 않음(개별 chunk event로 reload 없음).
            assert!(coalescer.poll(now, &ledger).is_empty());
        }
        assert!(coalescer.poll(now + WINDOW / 2, &ledger).is_empty());
        let changes = coalescer.poll(now + WINDOW, &ledger);
        assert_eq!(changes.len(), 1, "{changes:?}");
        match &changes[0] {
            ExternalChange::Generation {
                manifest_hash,
                generation,
                paths,
            } => {
                assert_eq!(*manifest_hash, external.manifest_hash);
                assert_eq!(*generation, 2);
                assert!(paths.contains(&p("design/current")));
                assert_eq!(paths.len(), 5);
            }
            other => panic!("Generation이어야 함: {other:?}"),
        }
        assert!(coalescer.poll(now + 10 * WINDOW, &ledger).is_empty());
        assert!(!coalescer.has_pending());
    }

    #[test]
    fn chunk_only_events_and_own_saves_do_not_reload() {
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        ledger.record_saved(&memory);
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), WINDOW);

        // chunk·manifest event만(예: GC·다른 도구) → 변화 없음.
        coalescer.push(
            &[
                FileWatchEvent::Created(p("design/objects/ef/sha256-ef.starchunk")),
                FileWatchEvent::Removed(p("design/objects/ab/sha256-ab.starchunk")),
            ],
            0,
        );
        assert!(coalescer.poll(WINDOW, &ledger).is_empty());

        // 자기 저장 → 변화 없음.
        let own = save_generation(&dir.path, Some(&base), b"v1", &mut memory);
        ledger.record_saved(&memory);
        for batch in save_events(&own.manifest_hash) {
            coalescer.push(&batch, 2 * WINDOW);
        }
        assert!(coalescer.poll(3 * WINDOW, &ledger).is_empty());
        assert_eq!(coalescer.current_pointer(), Some(own.manifest_hash));
    }

    #[test]
    fn saves_within_one_window_coalesce_and_separate_saves_do_not() {
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), WINDOW);
        let mut other = SavedGenerationState::new(memory.saved());

        // 한 구간 안의 외부 저장 두 번 → 최신 generation 하나.
        let g2 = save_generation(&dir.path, Some(&base), b"a", &mut other);
        coalescer.push(&save_events(&g2.manifest_hash).concat(), 0);
        let g3 = save_generation(&dir.path, Some(&g2), b"b", &mut other);
        coalescer.push(&save_events(&g3.manifest_hash).concat(), 10 * MS);
        let changes = coalescer.poll(10 * MS + WINDOW, &ledger);
        assert!(matches!(
            changes.as_slice(),
            [ExternalChange::Generation { manifest_hash, generation: 3, .. }]
                if *manifest_hash == g3.manifest_hash
        ));

        // 구간을 넘겨 떨어진 저장 → 각각 하나.
        let g4 = save_generation(&dir.path, Some(&g3), b"c", &mut other);
        coalescer.push(&save_events(&g4.manifest_hash).concat(), 100 * MS);
        assert_eq!(coalescer.poll(100 * MS + WINDOW, &ledger).len(), 1);
        let g5 = save_generation(&dir.path, Some(&g4), b"d", &mut other);
        coalescer.push(&save_events(&g5.manifest_hash).concat(), 300 * MS);
        assert_eq!(coalescer.poll(300 * MS + WINDOW, &ledger).len(), 1);
    }

    #[test]
    fn incomplete_generation_waits_for_its_chunks() {
        use stellar_storage::chunk::path::ChunkObjectPath;
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), WINDOW);
        let mut other = SavedGenerationState::new(memory.saved());
        let g2 = save_generation(&dir.path, Some(&base), b"late chunk", &mut other);
        // pointer는 바뀌었지만 chunk가 아직 없는 상태를 만듦(예: Git checkout 도중).
        let chunk = dir.path.join(
            ChunkObjectPath::for_hash(&g2.manifest.chunk_pointers[0].content_hash)
                .to_relative_path(),
        );
        let held = dir.path.join("held.starchunk");
        std::fs::rename(&chunk, &held).unwrap();

        coalescer.push(&[FileWatchEvent::Modified(p("design/current"))], 0);
        assert!(
            coalescer.poll(WINDOW, &ledger).is_empty(),
            "미완성 generation"
        );
        assert!(coalescer.has_pending());

        std::fs::rename(&held, &chunk).unwrap();
        coalescer.push(
            &[FileWatchEvent::Created(p(
                "design/objects/xx/late.starchunk",
            ))],
            2 * WINDOW,
        );
        let changes = coalescer.poll(3 * WINDOW, &ledger);
        assert!(matches!(
            changes.as_slice(),
            [ExternalChange::Generation { manifest_hash, .. }] if *manifest_hash == g2.manifest_hash
        ));
    }

    #[test]
    fn corrupt_generation_and_missing_pointer_are_reported_not_reloaded() {
        use stellar_storage::generation::current::write_current_pointer;
        use stellar_storage::generation::loader::manifest_path;
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), WINDOW);

        // pointer가 내용이 다른 manifest 파일을 가리킴(hash 불일치).
        let mut other = SavedGenerationState::new(memory.saved());
        let g2 = save_generation(&dir.path, Some(&base), b"x", &mut other);
        std::fs::write(manifest_path(&dir.path, &g2.manifest_hash), b"tampered").unwrap();
        write_current_pointer(&dir.path, &g2.manifest_hash).unwrap();
        coalescer.push(&[FileWatchEvent::Modified(p("design/current"))], 0);
        assert!(matches!(
            coalescer.poll(WINDOW, &ledger).as_slice(),
            [ExternalChange::InvalidGeneration { manifest_hash, .. }] if *manifest_hash == g2.manifest_hash
        ));

        std::fs::remove_file(stellar_storage::generation::current::current_pointer_path(
            &dir.path,
        ))
        .unwrap();
        coalescer.push(&[FileWatchEvent::Removed(p("design/current"))], 2 * WINDOW);
        assert_eq!(
            coalescer.poll(3 * WINDOW, &ledger),
            vec![ExternalChange::PointerUnavailable(
                PointerObservation::Missing
            )]
        );
    }

    #[test]
    fn non_storage_files_are_grouped_per_quiet_window() {
        let dir = TempDir::new();
        let ledger = OwnSaveLedger::new();
        let mut coalescer = ChangeCoalescer::new(&dir.path, PROJECT, None, WINDOW);
        let ext = WatchPath::new(WatchRootId(1), "font.otf").unwrap();
        coalescer.push(
            &[
                FileWatchEvent::Modified(p("stellar.yaml")),
                FileWatchEvent::Created(p("assets/a.png")),
            ],
            0,
        );
        coalescer.push(
            &[
                FileWatchEvent::Modified(p("stellar.yaml")),
                FileWatchEvent::Modified(ext.clone()),
                FileWatchEvent::Modified(p("design/objects/ab/x.starchunk")),
            ],
            5 * MS,
        );
        assert_eq!(
            coalescer.poll(5 * MS + WINDOW, &ledger),
            vec![ExternalChange::Files {
                paths: vec![p("assets/a.png"), p("stellar.yaml"), ext],
            }]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn real_external_save_produces_one_batch() {
        use crate::watcher::fsevents::FsEventsWatcher;
        use crate::watcher::ProjectFileWatcher;
        use std::time::{Duration, Instant};

        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        ledger.record_saved(&memory);
        let mut watcher = FsEventsWatcher::start(&dir.path, &[]).unwrap();
        let window = 150 * MS;
        let mut coalescer =
            ChangeCoalescer::new(&dir.path, PROJECT, Some(base.manifest_hash), window);
        let clock = Instant::now();
        let now = || u64::try_from(clock.elapsed().as_nanos()).unwrap_or(u64::MAX);

        let mut other = SavedGenerationState::new(memory.saved());
        let external = save_generation(&dir.path, Some(&base), b"external", &mut other);
        let mut changes = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            watcher.flush();
            let events = watcher.wait(Duration::from_millis(50));
            coalescer.push(&events, now());
            changes.extend(coalescer.poll(now(), &ledger));
            if !changes.is_empty() && !coalescer.has_pending() {
                // 남은 event가 늦게 와도 추가 batch가 없음을 한 구간 더 확인.
                std::thread::sleep(Duration::from_millis(300));
                watcher.flush();
                coalescer.push(&watcher.drain(), now());
                std::thread::sleep(Duration::from_millis(200));
                changes.extend(coalescer.poll(now(), &ledger));
                break;
            }
        }
        let generations: Vec<&ExternalChange> = changes
            .iter()
            .filter(|c| matches!(c, ExternalChange::Generation { .. }))
            .collect();
        assert_eq!(generations.len(), 1, "{changes:?}");
        assert!(matches!(
            generations[0],
            ExternalChange::Generation { manifest_hash, .. } if *manifest_hash == external.manifest_hash
        ));
    }
}
