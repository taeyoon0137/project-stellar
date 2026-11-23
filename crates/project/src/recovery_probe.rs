//! 복구 스냅샷 최신성 probe (V.9.3).
//!
//! 스냅샷은 **정상 저장 세대보다 최신일 때만** 복구를 제안합니다:
//!
//! - 스냅샷의 base manifest hash가 현재 current 세대와 일치하고 미저장
//!   revision(> 0)이 있으면 제안.
//! - base가 current와 다르면(스냅샷 이후 정상 저장이 진행됨) 오래된 스냅샷 —
//!   제안하지 않음.
//! - revision 0(미저장 변경 없음)이면 복구할 것이 없음 — 제안하지 않음.
//! - payload를 읽지 못하는 스냅샷은 제안 대상에서 제외하고 사유만 보고.
//! - 정상 저장·닫기·폐기로 지우려다 남은 스냅샷(폐기 표식의 hash와 bytes가 같음)은
//!   제안하지 않음(XIII.8.7, [`discard_snapshot`](crate::recovery_shutdown::discard_snapshot)).
//!
//! 저장된 스냅샷 하나의 판정은 [`judge_stored_snapshot`] 하나로 합니다. 이 crate의 probe와 앱 runtime의
//! 복구 제안이 같은 함수를 씁니다. 목록은 최근에 기록한 스냅샷부터입니다
//! ([`RecoverySnapshotStore::list_sessions_newest_first`]).

use std::fmt;
use std::path::Path;

use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::hash::ContentHash;

use stellar_core_types::ids::WindowSessionId;

use crate::identity::ProjectIdentity;
use crate::recovery::{RecoverySnapshotStore, RecoveryStoreError};
use crate::recovery_payload::RecoveryPayload;
use crate::recovery_shutdown::is_discarded;

/// 스냅샷 하나의 최신성 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotFreshness {
    /// 정상 저장 세대보다 최신 — 복구 제안.
    Fresh {
        /// 스냅샷 revision.
        revision: u64,
    },
    /// 오래됨 — 제안하지 않음.
    Stale {
        /// 사유.
        reason: StaleReason,
    },
}

/// 스냅샷이 오래된 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// 미저장 변경이 없습니다(revision 0).
    NoUnsavedChanges,
    /// 스냅샷 이후 정상 저장이 진행되어 base 세대가 다릅니다.
    BaseGenerationMoved {
        /// 스냅샷의 base.
        snapshot_base: Option<ContentHash>,
        /// 현재 current.
        current: Option<ContentHash>,
    },
    /// payload를 해석할 수 없습니다.
    UnreadablePayload,
    /// 정상 저장·닫기·폐기로 지우려다 삭제에 실패해 남은 스냅샷입니다.
    Discarded,
}

impl fmt::Display for StaleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StaleReason::NoUnsavedChanges => f.write_str("미저장 변경 없음"),
            StaleReason::BaseGenerationMoved { .. } => {
                f.write_str("스냅샷 이후 정상 저장이 진행됨")
            }
            StaleReason::UnreadablePayload => f.write_str("payload 해석 불가"),
            StaleReason::Discarded => f.write_str("폐기했지만 삭제하지 못한 스냅샷"),
        }
    }
}

/// probe 오류.
#[derive(Debug)]
pub enum RecoveryProbeError {
    /// 스냅샷 저장소 I/O 실패.
    Store(RecoveryStoreError),
    /// current pointer 읽기 실패(형식 위반·I/O).
    CurrentPointer(CurrentPointerError),
}

impl fmt::Display for RecoveryProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryProbeError::Store(e) => write!(f, "복구 저장소 오류: {e}"),
            RecoveryProbeError::CurrentPointer(e) => write!(f, "current pointer 오류: {e}"),
        }
    }
}

impl std::error::Error for RecoveryProbeError {}

/// 복구 제안 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryProposal {
    /// window session(파일 이름의 UUID 문자열).
    pub window_session: String,
    /// 스냅샷 revision.
    pub revision: u64,
    /// 판정한 스냅샷 파일 bytes의 hash. 적용할 때 같은 bytes인지 다시 확인하는 데 씁니다.
    pub hash: ContentHash,
}

/// 저장된 스냅샷 하나의 판정 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredSnapshot {
    /// 제안 대상입니다.
    Fresh {
        /// 해석한 payload.
        payload: RecoveryPayload,
        /// 스냅샷 파일 bytes의 hash.
        hash: ContentHash,
    },
    /// 제안하지 않습니다.
    Stale {
        /// 사유.
        reason: StaleReason,
    },
}

/// 저장소에서 읽은 스냅샷 bytes 하나를 판정합니다. 순서: 폐기 표식 → payload 해석 → [`assess_snapshot`].
pub fn judge_stored_snapshot(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
    current: Option<ContentHash>,
    bytes: &[u8],
) -> StoredSnapshot {
    if is_discarded(store, identity, window_session, bytes) {
        return StoredSnapshot::Stale {
            reason: StaleReason::Discarded,
        };
    }
    let Ok(payload) = RecoveryPayload::decode(bytes) else {
        return StoredSnapshot::Stale {
            reason: StaleReason::UnreadablePayload,
        };
    };
    match assess_snapshot(current, &payload) {
        SnapshotFreshness::Fresh { .. } => StoredSnapshot::Fresh {
            payload,
            hash: ContentHash::compute(bytes),
        },
        SnapshotFreshness::Stale { reason } => StoredSnapshot::Stale { reason },
    }
}

/// 스냅샷 payload 하나의 최신성을 판정합니다(순수 함수).
pub fn assess_snapshot(
    current: Option<ContentHash>,
    payload: &RecoveryPayload,
) -> SnapshotFreshness {
    if payload.base_manifest_hash != current {
        return SnapshotFreshness::Stale {
            reason: StaleReason::BaseGenerationMoved {
                snapshot_base: payload.base_manifest_hash,
                current,
            },
        };
    }
    if payload.revision == 0 {
        return SnapshotFreshness::Stale {
            reason: StaleReason::NoUnsavedChanges,
        };
    }
    SnapshotFreshness::Fresh {
        revision: payload.revision,
    }
}

/// 프로젝트의 모든 스냅샷을 검사해 **최신 스냅샷만** 복구 제안 목록으로
/// 반환합니다. 오래된 스냅샷은 목록에 포함하지 않습니다. 목록은 최근에 기록한 스냅샷부터입니다.
pub fn probe_recovery_snapshots(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    project_root: &Path,
) -> Result<Vec<RecoveryProposal>, RecoveryProbeError> {
    // 현재 정상 저장 세대(없으면 None — 아직 저장 이력 없음).
    let current = match read_current_pointer(project_root) {
        Ok(hash) => Some(hash),
        Err(CurrentPointerError::Missing { .. }) => None,
        Err(e) => return Err(RecoveryProbeError::CurrentPointer(e)),
    };

    let mut proposals = Vec::new();
    for session in store
        .list_sessions_newest_first(identity)
        .map_err(RecoveryProbeError::Store)?
    {
        let Some(id) = crate::identity::parse_project_id(&session) else {
            continue; // 형식이 아닌 파일 이름은 무시.
        };
        let window = WindowSessionId::from_stable_id(id);
        let Some(bytes) = store
            .read_snapshot(identity, window)
            .map_err(RecoveryProbeError::Store)?
        else {
            continue;
        };
        if let StoredSnapshot::Fresh { payload, hash } =
            judge_stored_snapshot(store, identity, window, current, &bytes)
        {
            proposals.push(RecoveryProposal {
                window_session: session,
                revision: payload.revision,
                hash,
            });
        }
    }
    Ok(proposals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery_payload::RecoveryPayload;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;
    use stellar_storage::generation::current::write_current_pointer;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(prefix: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn payload(base: Option<ContentHash>, revision: u64) -> RecoveryPayload {
        RecoveryPayload {
            base_manifest_hash: base,
            revision,
            unsaved_chunks: vec![],
            open_tabs: vec![],
            active_tab: 0,
            selection: vec![],
            target_profile: stable(4_000),
        }
    }

    #[test]
    fn assess_matrix() {
        let current = Some(ContentHash::compute(b"gen-2"));
        let old = Some(ContentHash::compute(b"gen-1"));

        // 최신: base 일치 + 미저장 revision.
        assert_eq!(
            assess_snapshot(current, &payload(current, 7)),
            SnapshotFreshness::Fresh { revision: 7 }
        );
        // base가 과거 세대 → 제안 안 함.
        assert!(matches!(
            assess_snapshot(current, &payload(old, 7)),
            SnapshotFreshness::Stale {
                reason: StaleReason::BaseGenerationMoved { .. }
            }
        ));
        // revision 0 → 제안 안 함.
        assert_eq!(
            assess_snapshot(current, &payload(current, 0)),
            SnapshotFreshness::Stale {
                reason: StaleReason::NoUnsavedChanges
            }
        );
        // 저장 이력 없는 프로젝트 + base None → 제안.
        assert_eq!(
            assess_snapshot(None, &payload(None, 1)),
            SnapshotFreshness::Fresh { revision: 1 }
        );
        // 저장 이력이 생겼는데 base None 스냅샷 → 제안 안 함.
        assert!(matches!(
            assess_snapshot(current, &payload(None, 1)),
            SnapshotFreshness::Stale { .. }
        ));
    }

    #[test]
    fn stale_snapshots_are_not_proposed() {
        // 검증 기준(V.9.3): 오래된 snapshot은 제안하지 않습니다.
        let project = TempDir::new("stellar-probe-project");
        let app_data = TempDir::new("stellar-probe-appdata");
        let identity = ProjectIdentity::new(stable(1), &project.path).unwrap();
        let store = RecoverySnapshotStore::new(&app_data.path);

        // 프로젝트의 current는 gen-2를 가리킵니다.
        let current_hash = ContentHash::compute(b"gen-2-manifest");
        write_current_pointer(&project.path, &current_hash).unwrap();

        // 스냅샷 3개: 최신(base=current, rev 5), 과거 base, revision 0.
        let fresh_session = WindowSessionId::from_stable_id(stable(100));
        store
            .write_snapshot(
                &identity,
                fresh_session,
                &payload(Some(current_hash), 5).encode().unwrap(),
            )
            .unwrap();
        store
            .write_snapshot(
                &identity,
                WindowSessionId::from_stable_id(stable(200)),
                &payload(Some(ContentHash::compute(b"gen-1-manifest")), 9)
                    .encode()
                    .unwrap(),
            )
            .unwrap();
        store
            .write_snapshot(
                &identity,
                WindowSessionId::from_stable_id(stable(300)),
                &payload(Some(current_hash), 0).encode().unwrap(),
            )
            .unwrap();

        let proposals = probe_recovery_snapshots(&store, &identity, &project.path).unwrap();
        assert_eq!(proposals.len(), 1, "{proposals:?}");
        assert_eq!(proposals[0].window_session, fresh_session.to_string());
        assert_eq!(proposals[0].revision, 5);
    }

    #[test]
    fn unreadable_snapshot_is_skipped() {
        let project = TempDir::new("stellar-probe-project");
        let app_data = TempDir::new("stellar-probe-appdata");
        let identity = ProjectIdentity::new(stable(1), &project.path).unwrap();
        let store = RecoverySnapshotStore::new(&app_data.path);
        store
            .write_snapshot(
                &identity,
                WindowSessionId::from_stable_id(stable(100)),
                b"garbage-not-a-payload",
            )
            .unwrap();
        let proposals = probe_recovery_snapshots(&store, &identity, &project.path).unwrap();
        assert!(proposals.is_empty());
    }

    #[test]
    fn no_snapshots_means_no_proposals() {
        let project = TempDir::new("stellar-probe-project");
        let app_data = TempDir::new("stellar-probe-appdata");
        let identity = ProjectIdentity::new(stable(1), &project.path).unwrap();
        let store = RecoverySnapshotStore::new(&app_data.path);
        assert!(probe_recovery_snapshots(&store, &identity, &project.path)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn newest_snapshot_comes_first_and_discarded_leftovers_are_stale() {
        // XIII.8.3: 최신 스냅샷부터 제안합니다. XIII.8.7: 폐기 표식과 bytes가 같은 스냅샷은 제안하지 않습니다.
        let project = TempDir::new("stellar-probe-project");
        let app_data = TempDir::new("stellar-probe-appdata");
        let identity = ProjectIdentity::new(stable(1), &project.path).unwrap();
        let store = RecoverySnapshotStore::new(&app_data.path);
        let current = ContentHash::compute(b"gen-2-manifest");
        write_current_pointer(&project.path, &current).unwrap();
        let base =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        let older_name = WindowSessionId::from_stable_id(stable(900));
        let newer_name = WindowSessionId::from_stable_id(stable(100));
        for (window, revision, offset) in [(older_name, 2, 50), (newer_name, 3, 10)] {
            store
                .write_snapshot(
                    &identity,
                    window,
                    &payload(Some(current), revision).encode().unwrap(),
                )
                .unwrap();
            std::fs::File::options()
                .write(true)
                .open(store.snapshot_path(&identity, window))
                .unwrap()
                .set_modified(base + std::time::Duration::from_secs(offset))
                .unwrap();
        }
        let proposals = probe_recovery_snapshots(&store, &identity, &project.path).unwrap();
        let order: Vec<u64> = proposals.iter().map(|p| p.revision).collect();
        assert_eq!(order, [2, 3], "마지막에 기록한 창 900이 먼저");
        let bytes = store.read_snapshot(&identity, older_name).unwrap().unwrap();
        assert_eq!(proposals[0].hash, ContentHash::compute(&bytes));

        // 삭제가 실패해 남은 스냅샷: 표식이 같은 bytes를 가리키면 stale입니다.
        std::fs::write(
            store.discarded_marker_path(&identity, older_name),
            ContentHash::compute(&bytes).to_string(),
        )
        .unwrap();
        assert_eq!(
            judge_stored_snapshot(&store, &identity, older_name, Some(current), &bytes),
            StoredSnapshot::Stale {
                reason: StaleReason::Discarded
            }
        );
        let proposals = probe_recovery_snapshots(&store, &identity, &project.path).unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].window_session, newer_name.to_string());
        // 해석 불가 payload도 같은 판정 함수의 결과입니다.
        assert_eq!(
            judge_stored_snapshot(&store, &identity, newer_name, Some(current), b"garbage"),
            StoredSnapshot::Stale {
                reason: StaleReason::UnreadablePayload
            }
        );
    }
}
