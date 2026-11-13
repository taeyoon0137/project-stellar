//! asset content GC (XIII.5.6).
//!
//! generation/chunk GC(storage V.7)와 **별도의** mark-sweep입니다. `assets/` 아래 content
//! 파일만 다루고 `design/` 아래는 건드리지 않습니다.
//!
//! - **mark**: 현재 project가 참조하는 [`AssetRecord`]들의 content hash 집합입니다
//!   ([`mark_live_content`]). Git tracked history(과거 commit)는 고려하지 않고 **current
//!   project refs만** 다룹니다. 호출자는 열린 문서의 현재 참조와 함께, undo history가 아직
//!   되돌릴 수 있는 record(XIII.5.5 `AssetRecordUpdate::previous`)도 mark 입력에 넣어야
//!   합니다 — 그렇지 않으면 undo 후 content가 없을 수 있습니다.
//! - **sweep**: `assets/<prefix>/`의 canonical content 파일 중 mark에 없는 파일을 grace
//!   period가 지난 뒤에만 삭제합니다([`sweep_unreferenced_assets`]). 같은 content를 여러
//!   AssetId가 공유해도 hash 하나만 live이면 삭제하지 않습니다(live shared content 보호).
//!   강제 종료로 남은 temp는 writer(XIII.5.4)가 만드는 이름
//!   `.<이 shard의 canonical content 이름>.tmp-<pid>-<n>`과 **정확히** 같을 때만 grace 이후
//!   정리합니다(`.tmp-`를 포함한 다른 숨김 파일은 사용자 파일로 보고 둠). 그 밖의 파일·
//!   디렉터리·symlink는 절대 삭제하지 않습니다(사용자 파일 보호).
//! - **symlink 디렉터리**: `assets/` 자체나 shard 이름(소문자 hex 2자)의 항목이 symlink면
//!   따라가지 않고 아무 것도 삭제하지 않은 채 [`AssetGcError::SymlinkedAssetsDirectory`]를
//!   돌려줍니다(commit된 symlink가 삭제를 프로젝트 밖으로 돌리는 것을 막음). 목록 수집이
//!   삭제보다 먼저 끝나므로 오류 시 삭제된 파일은 없습니다.
//! - **시작 조건**: storage V.7.4 scheduler가 발급한 [`GcGuard`](idle + 저장 없음)를 요구합니다.
//!   import·`Update from Source` 호출자는 `GcScheduler::note_activity`(또는 저장 guard)로
//!   GC와 겹치지 않게 해야 합니다 — dedup import는 기존 파일 mtime을 바꾸지 않으므로 grace
//!   period만으로는 보호되지 않습니다.
//! - 삭제 실패는 오류가 아니라 결과에 기록하고 다음 실행으로 이월합니다(idempotent).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use stellar_storage::gc::scheduler::GcGuard;
use stellar_storage::hash::ContentHash;

use crate::path::{assets_dir_state, AssetObjectPath, AssetsDirState, ASSETS_DIR};
use crate::record::AssetRecord;
use crate::store::parse_temp_file_name;

/// 현재 project 참조에서 live content hash 집합을 만듭니다(mark).
pub fn mark_live_content<'a>(
    records: impl IntoIterator<Item = &'a AssetRecord>,
) -> BTreeSet<ContentHash> {
    records.into_iter().map(|r| r.content_hash).collect()
}

/// sweep 결과.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetSweepOutcome {
    /// 삭제된 unreferenced content 파일.
    pub deleted: Vec<PathBuf>,
    /// 삭제된 stale temp 파일(강제 종료 잔여).
    pub deleted_temp: Vec<PathBuf>,
    /// mark되어 보존된 content 파일 수.
    pub kept_live: usize,
    /// unmark 또는 temp지만 grace period 이내라 보존된 파일 수.
    pub kept_in_grace: usize,
    /// content·temp 규칙에 맞지 않아 건드리지 않은 항목 수(사용자 파일·symlink 등).
    pub ignored: usize,
    /// 삭제·mtime 조회 실패 — 다음 실행으로 이월.
    pub carry_over: Vec<PathBuf>,
}

/// sweep을 시작할 수 없는 오류(아무 것도 삭제하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetGcError {
    /// `assets/` 또는 shard 디렉터리가 symlink입니다(따라가지 않음).
    SymlinkedAssetsDirectory {
        /// symlink인 경로.
        path: PathBuf,
    },
}

impl fmt::Display for AssetGcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetGcError::SymlinkedAssetsDirectory { path } => write!(
                f,
                "에셋 디렉터리가 symlink라 GC를 하지 않습니다: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for AssetGcError {}

/// `assets/` 항목 분류.
enum Entry {
    Content(ContentHash),
    Temp,
    Other,
}

/// shard 디렉터리 이름(소문자 hex 2자)인지.
fn is_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// XIII.5.4 writer가 이 shard의 content에 대해 만드는 temp 이름
/// (`.<canonical content 이름>.tmp-<pid>-<n>`)과 **정확히** 같은지.
fn is_temp_name(prefix: &str, name: &str) -> bool {
    parse_temp_file_name(prefix, name).is_some()
}

fn classify(prefix: &str, name: &str) -> Entry {
    if let Some((_, hash)) = AssetObjectPath::parse(prefix, name) {
        Entry::Content(hash)
    } else if is_temp_name(prefix, name) {
        Entry::Temp
    } else {
        Entry::Other
    }
}

/// 수집한 `(shard 이름, 파일 이름, 경로)` 목록.
type CollectedFiles = Vec<(String, String, PathBuf)>;

/// `assets/<prefix>/<file>` 일반 파일 목록(symlink·하위 디렉터리 제외)과 무시 항목 수.
///
/// `assets/`나 shard 이름 항목이 symlink면 따라가지 않고 오류입니다.
fn collect(assets: &Path, outcome: &mut AssetSweepOutcome) -> Result<CollectedFiles, AssetGcError> {
    let mut files = Vec::new();
    let symlinked = |path: PathBuf| AssetGcError::SymlinkedAssetsDirectory { path };
    match assets_dir_state(assets) {
        Ok(AssetsDirState::Directory) => {}
        Ok(AssetsDirState::Symlink) => return Err(symlinked(assets.to_path_buf())),
        // 없거나 디렉터리가 아니거나 조회 실패: 다룰 content가 없습니다.
        Ok(AssetsDirState::Missing | AssetsDirState::NotDirectory) | Err(_) => return Ok(files),
    }
    let Ok(shards) = std::fs::read_dir(assets) else {
        return Ok(files);
    };
    for shard in shards.flatten() {
        let shard_name = shard.file_name().to_string_lossy().into_owned();
        // DirEntry::file_type은 symlink를 따라가지 않습니다.
        let file_type = shard.file_type().ok();
        if is_shard_name(&shard_name) && file_type.is_some_and(|t| t.is_symlink()) {
            return Err(symlinked(shard.path()));
        }
        let is_dir = file_type.is_some_and(|t| t.is_dir());
        if !is_dir || !is_shard_name(&shard_name) {
            outcome.ignored += 1;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(shard.path()) else {
            outcome.carry_over.push(shard.path());
            continue;
        };
        for entry in entries.flatten() {
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            match entry.file_name().to_str() {
                Some(name) if is_file => {
                    files.push((shard_name.clone(), name.to_string(), entry.path()));
                }
                _ => outcome.ignored += 1,
            }
        }
    }
    files.sort_by(|a, b| a.2.cmp(&b.2));
    Ok(files)
}

/// unreferenced asset content를 sweep합니다.
///
/// - `live`: mark 집합. 포함된 hash의 content 파일은 절대 삭제하지 않습니다.
/// - `grace`: 이 시간보다 최근에 수정된 파일은 보존합니다(진행 중 import 보호).
/// - `now`: 현재 시각(테스트 주입용).
/// - `_gc`: V.7.4 scheduler의 GC guard — idle + 저장 없음 조건의 증거입니다.
///
/// `assets/`나 shard 디렉터리가 symlink면 아무 것도 삭제하지 않고
/// [`AssetGcError::SymlinkedAssetsDirectory`]입니다.
pub fn sweep_unreferenced_assets(
    project_root: &Path,
    live: &BTreeSet<ContentHash>,
    grace: Duration,
    now: SystemTime,
    _gc: &GcGuard<'_>,
) -> Result<AssetSweepOutcome, AssetGcError> {
    let mut outcome = AssetSweepOutcome::default();
    let files = collect(&project_root.join(ASSETS_DIR), &mut outcome)?;
    for (prefix, name, path) in files {
        let temp = match classify(&prefix, &name) {
            Entry::Content(hash) if live.contains(&hash) => {
                outcome.kept_live += 1;
                continue;
            }
            Entry::Content(_) => false,
            Entry::Temp => true,
            Entry::Other => {
                outcome.ignored += 1;
                continue;
            }
        };
        // grace 판정: mtime을 읽지 못하면 보수적으로 보존(이월).
        let old_enough = match path.symlink_metadata().and_then(|m| m.modified()) {
            Ok(mtime) => now
                .duration_since(mtime)
                .map(|age| age >= grace)
                .unwrap_or(false),
            Err(_) => {
                outcome.carry_over.push(path);
                continue;
            }
        };
        if !old_enough {
            outcome.kept_in_grace += 1;
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) if temp => outcome.deleted_temp.push(path),
            Ok(()) => outcome.deleted.push(path),
            Err(_) => outcome.carry_over.push(path),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::AssetCatalog;
    use crate::id::AssetId;
    use crate::media_type::MediaType;
    use crate::store::test_support::{files_under, test_store, TempDir};
    use crate::store::{AssetStore, ImportRequest};
    use std::time::Instant;
    use stellar_storage::gc::scheduler::GcScheduler;

    const GRACE: Duration = Duration::from_secs(3600);

    struct Fixture {
        _dir: TempDir,
        project: PathBuf,
        outside: PathBuf,
        store: AssetStore,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new("asset-gc");
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("assets")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let store = test_store(&project);
        Fixture {
            _dir: dir,
            project,
            outside,
            store,
        }
    }

    fn import(fx: &Fixture, name: &str, bytes: &[u8]) -> (AssetRecord, PathBuf) {
        let source = fx.outside.join(name);
        std::fs::write(&source, bytes).unwrap();
        let imported = fx
            .store
            .import_file(&ImportRequest::new(
                &source,
                MediaType::new("image/png").unwrap(),
            ))
            .unwrap();
        let path = fx.store.content_path(&imported.object_path);
        (imported.record, path)
    }

    fn id(n: u8) -> AssetId {
        AssetId::from_bytes([n; 16])
    }

    /// idle 조건을 만족한 scheduler에서 GC guard를 받습니다.
    fn idle_scheduler() -> (GcScheduler, Instant) {
        let start = Instant::now();
        (GcScheduler::new(Duration::from_secs(1), start), start)
    }

    fn later() -> SystemTime {
        SystemTime::now() + 2 * GRACE
    }

    #[test]
    fn live_shared_content_is_never_deleted() {
        let fx = fixture();
        let (shared_a, shared_path) = import(&fx, "a.png", b"shared");
        let (shared_b, _) = import(&fx, "copy-of-a.png", b"shared");
        let (single, single_path) = import(&fx, "b.png", b"single");
        let (orphan, orphan_path) = import(&fx, "c.png", b"orphan");
        assert_eq!(shared_a.content_hash, shared_b.content_hash);

        let mut catalog = AssetCatalog::new();
        catalog.insert(id(1), shared_a).unwrap();
        catalog.insert(id(2), shared_b).unwrap();
        catalog.insert(id(3), single).unwrap();
        catalog.insert(id(4), orphan).unwrap();
        // 공유 content의 한 AssetId와 orphan을 project에서 제거합니다.
        catalog.remove(&id(1)).unwrap();
        catalog.remove(&id(4)).unwrap();

        let live = mark_live_content(catalog.iter().map(|(_, r)| r));
        assert_eq!(live.len(), 2);
        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&fx.project, &live, GRACE, later(), &guard).unwrap();

        assert_eq!(outcome.deleted, vec![orphan_path.clone()]);
        assert_eq!(outcome.kept_live, 2);
        assert!(shared_path.exists(), "남은 AssetId가 공유 content를 참조");
        assert!(single_path.exists());
        assert!(!orphan_path.exists());
        assert_eq!(std::fs::read(&shared_path).unwrap(), b"shared");

        // 다시 실행해도 안전합니다(idempotent).
        let again = sweep_unreferenced_assets(&fx.project, &live, GRACE, later(), &guard).unwrap();
        assert!(again.deleted.is_empty());
        assert_eq!(again.kept_live, 2);
    }

    #[test]
    fn unreferenced_content_within_grace_is_kept() {
        let fx = fixture();
        let (_, path) = import(&fx, "fresh.png", b"just imported, not saved yet");
        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome = sweep_unreferenced_assets(
            &fx.project,
            &BTreeSet::new(),
            GRACE,
            SystemTime::now(),
            &guard,
        )
        .unwrap();
        assert_eq!(outcome.kept_in_grace, 1);
        assert!(outcome.deleted.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn stale_temp_is_cleaned_and_foreign_files_are_ignored() {
        let fx = fixture();
        let (record, content) = import(&fx, "a.png", b"kept");
        let shard = content.parent().unwrap().to_path_buf();
        let temp = shard.join(format!(
            ".{}.tmp-1-0",
            content.file_name().unwrap().to_str().unwrap()
        ));
        std::fs::write(&temp, b"partial").unwrap();
        // 사용자 파일·비shard 디렉터리·shard 안 하위 디렉터리·비canonical 이름.
        let assets = fx.project.join("assets");
        std::fs::write(assets.join("README.md"), b"notes").unwrap();
        std::fs::create_dir_all(assets.join("icons")).unwrap();
        std::fs::write(assets.join("icons").join("x.png"), b"x").unwrap();
        std::fs::create_dir_all(shard.join("nested")).unwrap();
        let upper = shard.join(
            content
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace(".png", ".PNG"),
        );
        std::fs::write(&upper, b"kept").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&content, shard.join("link.png")).unwrap();

        let live = mark_live_content([&record]);
        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&fx.project, &live, GRACE, later(), &guard).unwrap();

        assert_eq!(outcome.deleted_temp, vec![temp.clone()]);
        assert!(outcome.deleted.is_empty());
        assert!(!temp.exists());
        for kept in [
            &content,
            &upper,
            &assets.join("README.md"),
            &assets.join("icons").join("x.png"),
        ] {
            assert!(kept.exists(), "{kept:?}");
        }
        assert!(outcome.ignored >= 4, "{outcome:?}");

        // grace 이내 temp는 진행 중 import일 수 있으므로 보존합니다.
        std::fs::write(&temp, b"in progress").unwrap();
        let fresh = sweep_unreferenced_assets(&fx.project, &live, GRACE, SystemTime::now(), &guard)
            .unwrap();
        assert!(fresh.deleted_temp.is_empty());
        assert!(temp.exists());
    }

    #[test]
    fn sweep_requires_scheduler_guard_and_skips_design_objects() {
        let fx = fixture();
        let (_, content) = import(&fx, "a.png", b"orphan");
        // chunk GC 영역(design/objects)의 파일은 asset GC가 건드리지 않습니다.
        let chunk_dir = fx.project.join("design").join("objects").join("ab");
        std::fs::create_dir_all(&chunk_dir).unwrap();
        let chunk = chunk_dir.join(format!("sha256-{}.starchunk", "ab".repeat(32)));
        std::fs::write(&chunk, b"chunk").unwrap();

        let (scheduler, start) = idle_scheduler();
        // 저장 중에는 guard를 받을 수 없습니다(V.7.4).
        {
            let _save = scheduler.begin_save(start);
            assert!(scheduler
                .try_begin_gc(start + Duration::from_secs(2))
                .is_none());
        }
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&fx.project, &BTreeSet::new(), GRACE, later(), &guard)
                .unwrap();
        assert_eq!(outcome.deleted, vec![content]);
        assert!(chunk.exists());
        assert_eq!(
            files_under(&fx.project.join("assets")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn missing_assets_directory_is_a_noop() {
        let dir = TempDir::new("asset-gc-empty");
        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&dir.path, &BTreeSet::new(), GRACE, later(), &guard).unwrap();
        assert_eq!(outcome, AssetSweepOutcome::default());
    }

    /// commit된 symlink(`assets/` 또는 shard)가 삭제를 프로젝트 밖으로 돌리는 fixture.
    #[cfg(unix)]
    #[test]
    fn symlinked_assets_root_or_shard_is_never_followed() {
        let fx = fixture();
        // 프로젝트 밖에 canonical 이름의 unreferenced 파일(따라가면 grace 이후 삭제될 대상).
        let hash = ContentHash::compute(b"victim");
        let name = AssetObjectPath::new(&hash, &crate::path::SafeExtension::new("png").unwrap());
        let victim_root = fx.outside.join("victim-root");
        let victim_shard = victim_root.join(name.prefix());
        std::fs::create_dir_all(&victim_shard).unwrap();
        let victim = victim_shard.join(name.file_name());
        std::fs::write(&victim, b"victim").unwrap();

        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let empty = BTreeSet::new();

        // 1) shard 이름 항목이 symlink.
        let (_, content) = import(&fx, "kept.png", b"orphan inside");
        let shard_link = fx.project.join("assets").join(name.prefix());
        assert_ne!(content.parent().unwrap(), shard_link, "fixture: 다른 shard");
        std::os::unix::fs::symlink(&victim_shard, &shard_link).unwrap();
        assert_eq!(
            sweep_unreferenced_assets(&fx.project, &empty, GRACE, later(), &guard).unwrap_err(),
            AssetGcError::SymlinkedAssetsDirectory {
                path: shard_link.clone()
            }
        );
        assert!(victim.exists(), "프로젝트 밖 파일 보존");
        assert!(content.exists(), "오류 시 아무 것도 삭제하지 않음");
        std::fs::remove_file(&shard_link).unwrap();
        // shard 이름이 아닌 symlink는 기존처럼 무시합니다(사용자 파일).
        std::os::unix::fs::symlink(&victim_shard, fx.project.join("assets").join("linked"))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&fx.project, &empty, GRACE, later(), &guard).unwrap();
        assert_eq!(outcome.deleted, vec![content.clone()]);
        assert!(victim.exists());

        // 2) `assets/` 자체가 symlink.
        std::fs::remove_dir_all(fx.project.join("assets")).unwrap();
        std::os::unix::fs::symlink(&victim_root, fx.project.join("assets")).unwrap();
        let err =
            sweep_unreferenced_assets(&fx.project, &empty, GRACE, later(), &guard).unwrap_err();
        assert_eq!(
            err,
            AssetGcError::SymlinkedAssetsDirectory {
                path: fx.project.join("assets")
            }
        );
        assert!(err.to_string().contains("symlink"));
        assert!(victim.exists(), "프로젝트 밖 파일 보존");
    }

    #[test]
    fn shard_and_temp_name_rules() {
        assert!(is_shard_name("0f"));
        for bad in ["0F", "g0", "abc", "a", ""] {
            assert!(!is_shard_name(bad), "{bad}");
        }
        let hash = ContentHash::compute(b"temp rule");
        let content = AssetObjectPath::new(&hash, &crate::path::SafeExtension::new("png").unwrap());
        let prefix = content.prefix();
        let file = content.file_name();
        assert!(is_temp_name(prefix, &format!(".{file}.tmp-12-3")));
        assert!(is_temp_name(prefix, &format!(".{file}.tmp-0-0")));
        for not_temp in [
            file.to_string(),
            ".DS_Store".to_string(),
            ".DS_Store.tmp-1-2".to_string(),
            ".notes.tmp-1-1".to_string(),
            format!(".{file}.tmp-12"),
            format!(".{file}.tmp-12-"),
            format!(".{file}.tmp--3"),
            format!(".{file}.tmp-a-3"),
            format!(".{file}.tmp-01-3"),
            format!(".{file}.tmp-12-03"),
            format!(".{file}.tmp-1-2-3"),
            format!(".{file}.tmp-+1-2"),
            format!(".{file}.tmp-4294967296-1"), // pid는 u32
            format!("{file}.tmp-1-2"),
            format!(".{}.tmp-1-2", file.replace(".png", ".PNG")),
            format!(".{file}.bak.tmp-1-2"),
        ] {
            assert!(!is_temp_name(prefix, &not_temp), "{not_temp}");
        }
        // 다른 shard의 content 이름이면 이 shard의 temp가 아닙니다.
        let other = if prefix == "00" { "01" } else { "00" };
        assert!(!is_temp_name(other, &format!(".{file}.tmp-12-3")));
    }

    /// GC는 writer temp 형식과 정확히 같은 숨김 파일만 정리합니다.
    #[test]
    fn temp_cleanup_matches_writer_pattern_exactly() {
        let fx = fixture();
        let (record, content) = import(&fx, "a.png", b"live");
        let shard = content.parent().unwrap().to_path_buf();
        let file = content.file_name().unwrap().to_str().unwrap().to_string();
        let writer_temp = crate::store::temp_path_for(&content);
        std::fs::write(&writer_temp, b"partial").unwrap();
        let lookalikes: Vec<PathBuf> = [
            ".DS_Store.tmp-1-2".to_string(),
            ".notes.tmp-1-1".to_string(),
            format!(".{file}.tmp-12"),
            format!(".{file}.tmp-01-3"),
            format!(".{file}.bak.tmp-1-2"),
        ]
        .iter()
        .map(|name| shard.join(name))
        .collect();
        for path in &lookalikes {
            std::fs::write(path, b"user file").unwrap();
        }

        let live = mark_live_content([&record]);
        let (scheduler, start) = idle_scheduler();
        let guard = scheduler
            .try_begin_gc(start + Duration::from_secs(2))
            .unwrap();
        let outcome =
            sweep_unreferenced_assets(&fx.project, &live, GRACE, later(), &guard).unwrap();
        assert_eq!(outcome.deleted_temp, vec![writer_temp.clone()]);
        assert!(!writer_temp.exists());
        for path in &lookalikes {
            assert!(path.exists(), "{path:?}");
        }
        assert_eq!(outcome.ignored, lookalikes.len());
        assert!(content.exists());
    }
}
