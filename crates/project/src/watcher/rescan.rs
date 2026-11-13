//! 감시 넘침 뒤 전체 rescan (XIII.2.6).
//!
//! 파일 감시기가 [`FileWatchEvent::Overflow`]를 보고하면 그 사이의 변화를 알 수 없습니다. 이때 상태를
//! clean으로 가정하지 않고 [`WatchState::RescanPending`]으로 두고 [`RescanRequest`]를 내보냅니다.
//! 호출자가 [`OverflowRecovery::run_rescan`]을 실행해야만 다시 [`WatchState::Synced`]가 됩니다.
//!
//! rescan은 두 가지를 다시 검사합니다.
//! - **current pointer**: 조용한 구간을 기다리지 않고 바로 확인합니다([`ChangeCoalescer::rescan_pointer`]).
//!   외부의 새 generation이면 [`ExternalChange::Generation`] 등으로 보고합니다.
//! - **tracked files**: 저장소 영역(`design/`)과 `.git` 밖의 프로젝트 파일 전체를 resolver(XIII.4)로
//!   다시 훑어 내용 hash를 직전 기준과 비교합니다. 비신뢰 프로젝트의 외부 symlink target은 내용을
//!   읽지 않고 target 경로만 비교합니다(XIII.4.5).

use std::collections::BTreeMap;
use std::path::PathBuf;

use stellar_storage::hash::ContentHash;

use super::coalesce::{ChangeCoalescer, ExternalChange};
use super::event::{FileWatchEvent, OverflowReason};
use super::own_save::{current_pointer_watch_path, OwnSaveLedger};
use crate::path_resolver::{PathResolver, ResolveError, TargetKind, WalkIssue};

/// 저장소 영역 밖에서 제외하는 최상위 이름(Git 내부).
const GIT_DIR: &str = ".git";

/// rescan 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescanRequest {
    /// 요청을 만든 넘침 사유.
    pub reason: OverflowReason,
    /// current pointer 재검사(항상 `true`).
    pub current_pointer: bool,
    /// tracked files 재검사(항상 `true`).
    pub tracked_files: bool,
}

/// 감시 동기 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchState {
    /// 감시 event로 변화를 따라가고 있음.
    Synced,
    /// 넘침 뒤 rescan을 기다림(변화를 알 수 없음 — clean이 아님).
    RescanPending(OverflowReason),
}

/// tracked file 하나의 비교 기준.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFingerprint {
    /// 내용 hash.
    Content(ContentHash),
    /// 비신뢰 프로젝트의 외부 target: 내용 대신 target 경로만.
    ExternalTarget(PathBuf),
    /// 읽지 못함(사유).
    Unreadable(String),
}

/// tracked files의 (프로젝트 상대 경로 → 기준) snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackedFiles {
    files: BTreeMap<String, FileFingerprint>,
}

impl TrackedFiles {
    /// 저장소 영역과 `.git` 밖의 모든 파일을 resolver로 훑어 snapshot을 만듭니다.
    ///
    /// 목록을 읽지 못한 하위 디렉터리(권한 없음 등)는 실패로 끝내지 않고 그 디렉터리 경로를
    /// [`FileFingerprint::Unreadable`]로 기록합니다(읽을 수 있게 되거나 사라지면 변화로 나옵니다).
    /// 프로젝트 root 목록을 읽지 못할 때만 `Err`입니다.
    pub fn capture(resolver: &PathResolver) -> Result<TrackedFiles, ResolveError> {
        let pointer = current_pointer_watch_path();
        let storage_dir = pointer
            .relative()
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        let walk = resolver.walk_tree(|path| {
            let top = path.split('/').next().unwrap_or(path);
            top != GIT_DIR && top != storage_dir
        })?;
        let mut files = BTreeMap::new();
        for entry in walk.entries {
            if entry.kind != TargetKind::File {
                continue;
            }
            let fingerprint = match resolver.read(&entry.path) {
                Ok(bytes) => FileFingerprint::Content(ContentHash::compute(&bytes)),
                Err(ResolveError::ExternalTargetNotTrusted { target, .. }) => {
                    FileFingerprint::ExternalTarget(target)
                }
                Err(error) => FileFingerprint::Unreadable(error.to_string()),
            };
            files.insert(entry.path, fingerprint);
        }
        for issue in walk.issues {
            if let WalkIssue::DirectoryUnreadable { path, error } = issue {
                files.insert(path, FileFingerprint::Unreadable(error.to_string()));
            }
        }
        Ok(TrackedFiles { files })
    }

    /// 경로 수.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 경로의 기준.
    pub fn get(&self, path: &str) -> Option<&FileFingerprint> {
        self.files.get(path)
    }

    /// `newer`와 비교해 추가·삭제·변경된 경로(정렬)를 돌려줍니다.
    pub fn changed_paths(&self, newer: &TrackedFiles) -> Vec<String> {
        let mut changed: Vec<String> = Vec::new();
        for (path, fingerprint) in &newer.files {
            if self.files.get(path) != Some(fingerprint) {
                changed.push(path.clone());
            }
        }
        for path in self.files.keys() {
            if !newer.files.contains_key(path) {
                changed.push(path.clone());
            }
        }
        changed.sort();
        changed
    }
}

/// rescan 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescanOutcome {
    /// current pointer 재검사 결과(외부 generation 등).
    pub pointer_changes: Vec<ExternalChange>,
    /// 기준과 달라진 tracked file 경로(정렬).
    pub changed_files: Vec<String>,
}

/// 넘침 감지와 rescan 실행.
#[derive(Debug, Clone)]
pub struct OverflowRecovery {
    state: WatchState,
    baseline: TrackedFiles,
    rescans: u64,
}

impl OverflowRecovery {
    /// 감시 시작 시점의 tracked files 기준으로 만듭니다.
    pub fn new(baseline: TrackedFiles) -> OverflowRecovery {
        OverflowRecovery {
            state: WatchState::Synced,
            baseline,
            rescans: 0,
        }
    }

    /// 현재 상태.
    pub fn state(&self) -> WatchState {
        self.state
    }

    /// 실행한 rescan 수.
    pub fn rescans(&self) -> u64 {
        self.rescans
    }

    /// 감시 event를 봅니다. 넘침이 있으면 상태를 `RescanPending`으로 바꾸고 요청을 돌려줍니다.
    /// 이미 기다리는 중이면 같은 요청을 다시 돌려줍니다(하나로 합침).
    pub fn on_events(&mut self, events: &[FileWatchEvent]) -> Option<RescanRequest> {
        let reason = events.iter().find_map(|event| match event {
            FileWatchEvent::Overflow(reason) => Some(*reason),
            _ => None,
        })?;
        let reason = match self.state {
            WatchState::RescanPending(first) => first,
            WatchState::Synced => reason,
        };
        self.state = WatchState::RescanPending(reason);
        Some(RescanRequest {
            reason,
            current_pointer: true,
            tracked_files: true,
        })
    }

    /// rescan을 실행합니다: current pointer를 바로 확인하고 tracked files를 다시 훑어 기준과
    /// 비교합니다. 모두 성공해야 `Synced`가 되며, 실패하면 `RescanPending`을 유지합니다.
    ///
    /// 실패할 수 있는 tracked files 캡처를 **먼저** 하고, 그다음 current pointer를 확인합니다. pointer
    /// 확인은 마지막 관찰 값을 바꾸므로, 캡처가 실패했을 때 이미 확인한 외부 generation을 결과와 함께
    /// 버리면 재시도에서 `Unchanged`로 보여 다시 보고되지 않습니다. 실패하면 coalescer·기준·상태
    /// 어느 것도 바꾸지 않습니다(rescan 횟수만 셈).
    pub fn run_rescan(
        &mut self,
        coalescer: &mut ChangeCoalescer,
        ledger: &OwnSaveLedger,
        resolver: &PathResolver,
    ) -> Result<RescanOutcome, ResolveError> {
        self.rescans += 1;
        let current = TrackedFiles::capture(resolver)?;
        let pointer_changes = coalescer.rescan_pointer(ledger);
        let changed_files = self.baseline.changed_paths(&current);
        self.baseline = current;
        self.state = WatchState::Synced;
        Ok(RescanOutcome {
            pointer_changes,
            changed_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::TrustStatus;
    use crate::watcher::event::WatchPath;
    use crate::watcher::own_save::tests::{save_generation, TempDir, PROJECT};
    use stellar_storage::save::commit::SavedGenerationState;

    const WINDOW: u64 = 50_000_000;

    struct Fixture {
        dir: TempDir,
        memory: SavedGenerationState,
        ledger: OwnSaveLedger,
        base: stellar_storage::generation::loader::LoadedGeneration,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path.join(".git/objects")).unwrap();
        std::fs::create_dir_all(dir.path.join("assets")).unwrap();
        std::fs::write(dir.path.join("stellar.yaml"), b"schema_version: 1\n").unwrap();
        std::fs::write(dir.path.join("assets/logo.png"), b"png-v1").unwrap();
        std::fs::write(dir.path.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
        let mut memory = SavedGenerationState::new(None);
        let mut ledger = OwnSaveLedger::new();
        let base = save_generation(&dir.path, None, b"v0", &mut memory);
        ledger.record_saved(&memory);
        Fixture {
            dir,
            memory,
            ledger,
            base,
        }
    }

    #[test]
    fn overflow_requests_and_runs_a_full_rescan() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.dir.path).unwrap();
        let baseline = TrackedFiles::capture(&resolver).unwrap();
        // 저장소 영역·.git은 tracked files에서 제외.
        assert_eq!(baseline.len(), 2);
        assert!(baseline.get("stellar.yaml").is_some());
        let mut coalescer =
            ChangeCoalescer::new(&fx.dir.path, PROJECT, Some(fx.base.manifest_hash), WINDOW);
        let mut recovery = OverflowRecovery::new(baseline);

        // 넘침 동안 잃어버린 변화: 외부 generation, stellar.yaml 수정, 에셋 추가·삭제.
        let mut other = SavedGenerationState::new(fx.memory.saved());
        let external = save_generation(&fx.dir.path, Some(&fx.base), b"lost", &mut other);
        std::fs::write(fx.dir.path.join("stellar.yaml"), b"schema_version: 2\n").unwrap();
        std::fs::write(fx.dir.path.join("assets/new.png"), b"n").unwrap();
        std::fs::remove_file(fx.dir.path.join("assets/logo.png")).unwrap();

        // 일반 event만으로는 요청 없음.
        assert_eq!(
            recovery.on_events(&[FileWatchEvent::Modified(WatchPath::project("x").unwrap())]),
            None
        );
        let request = recovery
            .on_events(&[FileWatchEvent::Overflow(OverflowReason::EventsDropped)])
            .unwrap();
        assert_eq!(
            request,
            RescanRequest {
                reason: OverflowReason::EventsDropped,
                current_pointer: true,
                tracked_files: true,
            }
        );
        assert_eq!(
            recovery.state(),
            WatchState::RescanPending(OverflowReason::EventsDropped)
        );

        let outcome = recovery
            .run_rescan(&mut coalescer, &fx.ledger, &resolver)
            .unwrap();
        assert_eq!(recovery.rescans(), 1);
        assert_eq!(recovery.state(), WatchState::Synced);
        assert!(matches!(
            outcome.pointer_changes.as_slice(),
            [ExternalChange::Generation { manifest_hash, .. }] if *manifest_hash == external.manifest_hash
        ));
        assert_eq!(
            outcome.changed_files,
            vec!["assets/logo.png", "assets/new.png", "stellar.yaml"]
        );

        // 다음 rescan은 새 기준과 비교(변화 없음).
        recovery.on_events(&[FileWatchEvent::Overflow(OverflowReason::RescanRequired)]);
        let again = recovery
            .run_rescan(&mut coalescer, &fx.ledger, &resolver)
            .unwrap();
        assert!(again.pointer_changes.is_empty());
        assert!(again.changed_files.is_empty());
    }

    #[test]
    fn state_is_not_marked_clean_without_a_rescan() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.dir.path).unwrap();
        let mut coalescer =
            ChangeCoalescer::new(&fx.dir.path, PROJECT, Some(fx.base.manifest_hash), WINDOW);
        let mut recovery = OverflowRecovery::new(TrackedFiles::capture(&resolver).unwrap());
        recovery.on_events(&[FileWatchEvent::Overflow(OverflowReason::QueueFull)]);
        // 이후 일반 event·coalescer 결과가 비어도 clean으로 돌아가지 않음.
        let later = [FileWatchEvent::Modified(
            WatchPath::project("assets/logo.png").unwrap(),
        )];
        assert_eq!(
            recovery.on_events(&later),
            None,
            "일반 event는 새 요청을 만들지 않음"
        );
        coalescer.push(&later, 0);
        let _ = coalescer.poll(10 * WINDOW, &fx.ledger);
        assert_eq!(
            recovery.state(),
            WatchState::RescanPending(OverflowReason::QueueFull)
        );
        // 두 번째 넘침은 첫 사유로 합쳐짐.
        let merged = recovery
            .on_events(&[FileWatchEvent::Overflow(OverflowReason::RootChanged)])
            .unwrap();
        assert_eq!(merged.reason, OverflowReason::QueueFull);

        // rescan이 실패하면(프로젝트 폴더가 사라짐) 계속 pending.
        let gone = fx.dir.path.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let broken = PathResolver::new(&gone).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(recovery
            .run_rescan(&mut coalescer, &fx.ledger, &broken)
            .is_err());
        assert_eq!(
            recovery.state(),
            WatchState::RescanPending(OverflowReason::QueueFull)
        );
        assert!(recovery
            .run_rescan(&mut coalescer, &fx.ledger, &resolver)
            .is_ok());
        assert_eq!(recovery.state(), WatchState::Synced);
    }

    #[test]
    fn failed_rescan_keeps_the_pointer_change_for_the_retry() {
        // 회귀(검토 지적): 캡처 실패로 rescan이 실패해도, 그 사이 확인했어야 할 외부 generation은
        // 재시도에서 보고되어야 합니다.
        let fx = fixture();
        let resolver = PathResolver::new(&fx.dir.path).unwrap();
        let mut coalescer =
            ChangeCoalescer::new(&fx.dir.path, PROJECT, Some(fx.base.manifest_hash), WINDOW);
        let mut recovery = OverflowRecovery::new(TrackedFiles::capture(&resolver).unwrap());
        let mut other = SavedGenerationState::new(fx.memory.saved());
        let external = save_generation(&fx.dir.path, Some(&fx.base), b"lost", &mut other);
        recovery.on_events(&[FileWatchEvent::Overflow(OverflowReason::EventsDropped)]);

        // 캡처 실패(root를 읽을 수 없음): pointer 관찰 값도 바뀌지 않아야 함.
        let gone = fx.dir.path.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let broken = PathResolver::new(&gone).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(recovery
            .run_rescan(&mut coalescer, &fx.ledger, &broken)
            .is_err());
        assert_eq!(coalescer.current_pointer(), Some(fx.base.manifest_hash));
        assert_eq!(
            recovery.state(),
            WatchState::RescanPending(OverflowReason::EventsDropped)
        );

        let outcome = recovery
            .run_rescan(&mut coalescer, &fx.ledger, &resolver)
            .unwrap();
        assert!(
            matches!(
                outcome.pointer_changes.as_slice(),
                [ExternalChange::Generation { manifest_hash, .. }]
                    if *manifest_hash == external.manifest_hash
            ),
            "{:?}",
            outcome.pointer_changes
        );
        assert_eq!(recovery.state(), WatchState::Synced);
        assert_eq!(recovery.rescans(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_subdirectory_does_not_block_the_rescan() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        let locked = fx.dir.path.join("assets/locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("hidden.png"), b"h").unwrap();
        let resolver = PathResolver::new(&fx.dir.path).unwrap();
        let mut coalescer =
            ChangeCoalescer::new(&fx.dir.path, PROJECT, Some(fx.base.manifest_hash), WINDOW);
        let mut recovery = OverflowRecovery::new(TrackedFiles::capture(&resolver).unwrap());
        assert!(recovery.baseline.get("assets/locked/hidden.png").is_some());

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&locked).is_ok() {
            // 권한 검사가 없는 환경(root)에서는 건너뜀.
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        std::fs::write(fx.dir.path.join("stellar.yaml"), b"schema_version: 2\n").unwrap();
        recovery.on_events(&[FileWatchEvent::Overflow(OverflowReason::QueueFull)]);
        let outcome = recovery.run_rescan(&mut coalescer, &fx.ledger, &resolver);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = outcome.unwrap();
        assert_eq!(recovery.state(), WatchState::Synced);
        // 읽을 수 없게 된 디렉터리와 그 안의 기존 파일, 그리고 다른 변경이 모두 보고됨.
        assert_eq!(
            outcome.changed_files,
            vec!["assets/locked", "assets/locked/hidden.png", "stellar.yaml"]
        );
        assert!(matches!(
            recovery.baseline.get("assets/locked"),
            Some(FileFingerprint::Unreadable(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_rescan_compares_external_targets_without_reading_them() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        let outside = fx
            .dir
            .path
            .join("..")
            .join(format!("stellar-rescan-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("font.otf"), b"secret").unwrap();
        symlink(
            outside.join("font.otf"),
            fx.dir.path.join("assets/font.otf"),
        )
        .unwrap();
        let resolver = PathResolver::new(&fx.dir.path)
            .unwrap()
            .with_trust(TrustStatus::NotTrusted);
        let snapshot = TrackedFiles::capture(&resolver).unwrap();
        assert!(matches!(
            snapshot.get("assets/font.otf"),
            Some(FileFingerprint::ExternalTarget(_))
        ));
        // 외부 내용이 바뀌어도(비신뢰라 읽지 않으므로) 변화로 보지 않음.
        std::fs::write(outside.join("font.otf"), b"changed").unwrap();
        let again = TrackedFiles::capture(&resolver).unwrap();
        assert!(snapshot.changed_paths(&again).is_empty());
        let _ = std::fs::remove_dir_all(&outside);
    }
}
