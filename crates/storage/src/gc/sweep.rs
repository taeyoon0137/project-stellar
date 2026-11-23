//! unreferenced 파일 sweep (V.7.3).
//!
//! mark 집합(V.7.2)에 없는 chunk·manifest 파일을 **grace period가 지난 뒤에만**
//! 삭제합니다. 방금 기록됐지만 아직 current로 연결되지 않은 파일(진행 중인
//! 저장의 산출물)을 지우지 않기 위한 유예입니다.
//!
//! 파일 삭제 실패는 오류가 아니라 결과에 기록만 하고 **다음 실행으로
//! 이월**합니다 — sweep은 언제 다시 실행해도 안전합니다(idempotent).
//!
//! # 비신뢰 프로젝트 폴더 (보안 감사 STOR-2·3)
//!
//! - 수집은 고정 구조만 봅니다: `design/objects/<shard>/<algo>-<hex>.starchunk`(shard 이름은 그
//!   hash의 hex 앞 2자와 같아야 함, V.2.3)와 `design/generations/<algo>-<hex>.starmanifest`(V.4.4).
//!   항목 종류는 `DirEntry::file_type`(따라가지 않음)으로만 판정하고 symlink는 따라가지도 지우지도
//!   않습니다. 그 밖의 하위 디렉터리는 무시합니다. 그래서 commit된 symlink 순환(`objects/a -> .`)이
//!   있어도 방문 수는 실제 항목 수에 비례합니다.
//! - `design`·`design/objects`·`design/generations` 자체나 shard 이름(소문자 hex 2자) 항목이 symlink면
//!   **아무 것도 지우지 않고** 그 경로를 [`SweepOutcome::carry_over`]에 기록합니다(commit된 symlink가
//!   삭제를 프로젝트 밖으로 돌리는 것을 막음). 목록 수집이 삭제보다 먼저 끝나므로 이때 삭제된 파일은
//!   없습니다. 공개 API(반환 타입)를 바꾸지 않기 위해 오류 대신 이월로 보고합니다.
//! - 각 삭제 직전에 파일과 그 위 디렉터리들을 다시 `lstat`해 수집 때와 같은 항목(`(dev, ino)`)인지
//!   확인하고, 다르면 지우지 않고 이월합니다(crate 내부 `fs_guard`). 확인 직후와 삭제 syscall 사이의
//!   짧은 창은 남습니다(디렉터리 fd 기준 삭제가 std에 없음).

use std::collections::BTreeSet;
use std::fs::DirEntry;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::chunk::path::ChunkObjectPath;
use crate::fs_guard::{same_dir, FileIdentity};
use crate::hash::ContentHash;

use super::retention::RetentionPolicy;

/// sweep 결과.
#[derive(Debug, Default)]
pub struct SweepOutcome {
    /// 삭제된 파일 경로.
    pub deleted: Vec<PathBuf>,
    /// mark되어 보존된 파일 수.
    pub kept_reachable: usize,
    /// unmark지만 grace period 이내라 보존된 파일 수.
    pub kept_in_grace: usize,
    /// 삭제 실패 — 다음 실행으로 이월. 저장소 디렉터리가 symlink라 아무 것도 지우지 않은
    /// 경우에는 그 symlink 경로가 들어갑니다.
    pub carry_over: Vec<PathBuf>,
}

/// chunk 파일 확장자(점 포함).
const CHUNK_SUFFIX: &str = ".starchunk";

/// manifest 파일 확장자(점 포함).
const MANIFEST_SUFFIX: &str = ".starmanifest";

/// 파일 이름(`<algo>-<hex>.<ext>`)에서 hash를 복원합니다. 형식이 아니면 `None`.
fn hash_from_file_name(name: &str, extension: &str) -> Option<ContentHash> {
    let stem = name.strip_suffix(extension)?;
    let (algo, hex) = stem.split_once('-')?;
    format!("{algo}:{hex}").parse().ok()
}

/// chunk shard 디렉터리 이름인지(소문자 hex 2자, V.2.3).
fn is_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 수집한 삭제 후보 파일(수집 때 `lstat` identity 포함).
struct Collected {
    /// 파일 이름에서 복원한 hash.
    hash: ContentHash,
    /// 파일 경로.
    path: PathBuf,
    /// 파일 identity.
    identity: FileIdentity,
    /// 이 파일 위 디렉터리들의 [`Collection::dirs`] 색인(`design`, 영역, shard).
    parents: Vec<usize>,
}

/// 수집 결과.
#[derive(Default)]
struct Collection {
    /// 수집 때 확인한 실제 디렉터리와 identity.
    dirs: Vec<(PathBuf, FileIdentity)>,
    /// 삭제 후보 파일(경로 순).
    files: Vec<Collected>,
}

/// 정규 파일이고 이름이 content-addressed 형식이면 hash와 identity. `shard`가 있으면 hash의 정규
/// shard(V.2.3)와 같을 때만입니다.
fn candidate(
    entry: &DirEntry,
    extension: &str,
    shard: Option<&str>,
) -> Option<(ContentHash, FileIdentity)> {
    // DirEntry::file_type·metadata는 symlink를 따라가지 않습니다.
    if !entry.file_type().ok()?.is_file() {
        return None;
    }
    let name = entry.file_name();
    let hash = hash_from_file_name(name.to_str()?, extension)?;
    if shard.is_some_and(|shard| ChunkObjectPath::for_hash(&hash).prefix() != shard) {
        return None;
    }
    let metadata = entry.metadata().ok()?;
    Some((hash, FileIdentity::of(&metadata)))
}

/// 경로가 실제 디렉터리면 identity, 없거나 디렉터리가 아니면 `None`, symlink면 `Err(경로)`.
fn real_dir(path: &Path) -> Result<Option<FileIdentity>, PathBuf> {
    match FileIdentity::lstat(path) {
        Ok((metadata, _)) if metadata.file_type().is_symlink() => Err(path.to_path_buf()),
        Ok((metadata, identity)) if metadata.is_dir() => Ok(Some(identity)),
        Ok(_) | Err(_) => Ok(None),
    }
}

/// 고정 구조의 chunk·manifest 파일을 수집합니다(symlink는 따라가지 않음).
///
/// 저장소 디렉터리나 shard 이름 항목이 symlink면 `Err(그 경로)`입니다. 디렉터리를 읽지 못하면 그
/// 경로를 `outcome.carry_over`에 남기고 건너뜁니다.
fn collect(root: &Path, outcome: &mut SweepOutcome) -> Result<Collection, PathBuf> {
    let mut collection = Collection::default();
    // cache 폴더(`.stellar/cache/chunks`, ADR-0122)의 부모 구성 요소도 symlink면 아무것도 지우지 않습니다.
    let mut parent = root.to_path_buf();
    for component in &crate::location::CACHE_DIR[..crate::location::CACHE_DIR.len() - 1] {
        parent.push(component);
        if real_dir(&parent)?.is_none() {
            return Ok(collection);
        }
    }
    let design = crate::location::cache_dir(root);
    let Some(design_identity) = real_dir(&design)? else {
        return Ok(collection);
    };
    collection.dirs.push((design.clone(), design_identity));

    for (area_name, extension, sharded) in [
        ("objects", CHUNK_SUFFIX, true),
        ("generations", MANIFEST_SUFFIX, false),
    ] {
        let area = design.join(area_name);
        let Some(area_identity) = real_dir(&area)? else {
            continue;
        };
        let area_index = collection.dirs.len();
        collection.dirs.push((area.clone(), area_identity));
        let Ok(entries) = std::fs::read_dir(&area) else {
            outcome.carry_over.push(area);
            continue;
        };
        for entry in entries.flatten() {
            if !sharded {
                if let Some((hash, identity)) = candidate(&entry, extension, None) {
                    collection.files.push(Collected {
                        hash,
                        path: entry.path(),
                        identity,
                        parents: vec![0, area_index],
                    });
                }
                continue;
            }
            // shard 이름이 아닌 항목(다른 파일·디렉터리·symlink 순환 포함)은 보지 않습니다.
            let name = entry.file_name();
            let Some(shard_name) = name.to_str().filter(|n| is_shard_name(n)) else {
                continue;
            };
            let shard = entry.path();
            let Some(shard_identity) = real_dir(&shard)? else {
                continue;
            };
            let shard_index = collection.dirs.len();
            collection.dirs.push((shard.clone(), shard_identity));
            let Ok(files) = std::fs::read_dir(&shard) else {
                outcome.carry_over.push(shard);
                continue;
            };
            for file in files.flatten() {
                if let Some((hash, identity)) = candidate(&file, extension, Some(shard_name)) {
                    collection.files.push(Collected {
                        hash,
                        path: file.path(),
                        identity,
                        parents: vec![0, area_index, shard_index],
                    });
                }
            }
        }
    }
    collection.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(collection)
}

/// unreferenced chunk·manifest를 sweep합니다.
///
/// - `reachable`: mark 집합(V.7.2). 포함된 파일은 절대 삭제하지 않습니다.
/// - `now`: 현재 시각(테스트 주입용). 파일 mtime + grace period가 `now` 이후면
///   유예 중으로 보존합니다.
///
/// 저장소 디렉터리(`design`·`design/objects`·`design/generations`·shard)가 symlink면 아무 것도
/// 삭제하지 않고 그 경로를 [`SweepOutcome::carry_over`]에 기록합니다(모듈 문서).
pub fn sweep_unreferenced(
    root: &Path,
    reachable: &BTreeSet<ContentHash>,
    policy: &RetentionPolicy,
    now: SystemTime,
) -> SweepOutcome {
    sweep_observed(root, reachable, policy, now, &mut |_| {})
}

/// [`sweep_unreferenced`]에 삭제 판정 직전 hook을 주입한 버전(교체 경합 테스트용).
fn sweep_observed(
    root: &Path,
    reachable: &BTreeSet<ContentHash>,
    policy: &RetentionPolicy,
    now: SystemTime,
    before_delete_check: &mut dyn FnMut(&Path),
) -> SweepOutcome {
    let grace = Duration::from_secs(policy.grace_period_seconds);
    let mut outcome = SweepOutcome::default();
    let collection = match collect(root, &mut outcome) {
        Ok(collection) => collection,
        Err(symlinked) => {
            // 삭제 전에 수집을 끝내므로 아직 아무 것도 지우지 않았습니다.
            outcome.carry_over.push(symlinked);
            return outcome;
        }
    };

    for file in collection.files {
        if reachable.contains(&file.hash) {
            outcome.kept_reachable += 1;
            continue;
        }
        before_delete_check(&file.path);
        // 수집 뒤 같은 이름에 다른 항목(symlink·다른 파일)이 들어왔으면 지우지 않습니다.
        let metadata = match FileIdentity::lstat(&file.path) {
            Ok((metadata, identity))
                if metadata.file_type().is_file() && identity == file.identity =>
            {
                metadata
            }
            _ => {
                outcome.carry_over.push(file.path);
                continue;
            }
        };
        // grace period 판정: mtime을 읽지 못하면 보수적으로 보존(이월).
        let old_enough = match metadata.modified() {
            Ok(mtime) => now
                .duration_since(mtime)
                .map(|age| age >= grace)
                .unwrap_or(false),
            Err(_) => {
                outcome.carry_over.push(file.path);
                continue;
            }
        };
        if !old_enough {
            outcome.kept_in_grace += 1;
            continue;
        }
        // 삭제 직전 디렉터리 확인: 위 디렉터리들이 수집 때와 같은 실제 디렉터리여야 합니다.
        let parents_unchanged = file.parents.iter().all(|&i| {
            let (path, identity) = &collection.dirs[i];
            same_dir(path, *identity)
        });
        if !parents_unchanged {
            outcome.carry_over.push(file.path);
            continue;
        }
        match std::fs::remove_file(&file.path) {
            Ok(()) => outcome.deleted.push(file.path),
            // 삭제 실패는 다음 실행으로 이월합니다.
            Err(_) => outcome.carry_over.push(file.path),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::path::ChunkObjectPath;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::gc::mark::mark_reachable;
    use crate::generation::loader::write_manifest_file;
    use crate::generation::manifest::GenerationManifest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-sweep-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn policy(grace_seconds: u64) -> RetentionPolicy {
        RetentionPolicy {
            keep_recent_generations: 2,
            grace_period_seconds: grace_seconds,
        }
    }

    fn write_chunk(dir: &TempDir, payload: &[u8]) -> ContentHash {
        let store = ChunkStore::new(&dir.path);
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) =
            store.write_chunk(KIND, payload).unwrap();
        hash
    }

    /// grace period(1시간)를 지난 시점.
    fn after_grace() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3_600 + 60)
    }

    #[test]
    fn active_chunks_are_kept_and_orphans_are_deleted() {
        // 검증 기준(V.7.3): active chunk 미삭제, orphan 삭제 fixture.
        let dir = TempDir::new();
        let active = write_chunk(&dir, b"active-chunk");
        let orphan = write_chunk(&dir, b"orphan-chunk");
        let manifest_hash = write_manifest_file(
            &dir.path,
            &GenerationManifest {
                generation: 1,
                parent_generation: None,
                project_id: stable(42),
                schema_version: 1,
                chunk_pointers: vec![ChunkPointer {
                    chunk_id: ChunkId::from_stable_id(stable(100)),
                    content_hash: active,
                    kind: KIND,
                    schema_version: 1,
                }],
                root_document_ids: vec![],
            },
        )
        .unwrap();

        let reachable = mark_reachable(&dir.path, &[manifest_hash]).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &reachable, &policy(3_600), after_grace());

        let active_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&active).to_relative_path());
        let orphan_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&orphan).to_relative_path());
        assert!(active_path.is_file(), "active chunk는 삭제 금지");
        assert!(!orphan_path.exists(), "orphan chunk는 삭제");
        assert_eq!(outcome.deleted.len(), 1);
        assert_eq!(outcome.kept_reachable, 2, "active chunk + 보존 manifest");
        assert!(outcome.carry_over.is_empty());
    }

    #[test]
    fn orphan_inside_grace_period_is_kept() {
        let dir = TempDir::new();
        write_chunk(&dir, b"fresh-orphan");
        let reachable = BTreeSet::new();
        // 방금 만든 파일 + 1시간 유예 → 아직 삭제 금지.
        let outcome = sweep_unreferenced(&dir.path, &reachable, &policy(3_600), SystemTime::now());
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.kept_in_grace, 1);
    }

    #[test]
    fn zero_grace_deletes_immediately() {
        let dir = TempDir::new();
        write_chunk(&dir, b"orphan-now");
        let outcome =
            sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), SystemTime::now());
        assert_eq!(outcome.deleted.len(), 1);
    }

    #[test]
    fn unrelated_files_are_untouched() {
        let dir = TempDir::new();
        let objects = crate::location::cache_dir(&dir.path)
            .join("objects")
            .join("aa");
        std::fs::create_dir_all(&objects).unwrap();
        let stray = objects.join("notes.txt");
        std::fs::write(&stray, b"not a chunk").unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert!(stray.is_file(), "형식이 아닌 파일은 건드리지 않음");
        assert!(outcome.deleted.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn deletion_failure_is_carried_over_to_next_run() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let orphan = write_chunk(&dir, b"protected-orphan");
        let orphan_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&orphan).to_relative_path());
        let shard = orphan_path.parent().unwrap().to_path_buf();

        // shard 디렉터리를 읽기 전용으로 → remove_file 실패.
        std::fs::set_permissions(&shard, std::fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert_eq!(outcome.carry_over.len(), 1, "삭제 실패는 이월");
        assert!(orphan_path.is_file());

        // 권한 복구 후 다음 실행에서 삭제.
        std::fs::set_permissions(&shard, std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert_eq!(outcome.deleted.len(), 1);
        assert!(!orphan_path.exists());
    }

    fn chunk_path(root: &Path, hash: &ContentHash) -> PathBuf {
        root.join(ChunkObjectPath::for_hash(hash).to_relative_path())
    }

    /// `other` 프로젝트에 chunk 하나와 manifest 하나를 만듭니다(프로젝트 밖 희생 파일).
    fn victim_project(other: &Path) -> (PathBuf, PathBuf) {
        let store = ChunkStore::new(other);
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) =
            store.write_chunk(KIND, b"victim-chunk").unwrap();
        let manifest_hash = write_manifest_file(
            other,
            &GenerationManifest {
                generation: 9,
                parent_generation: None,
                project_id: stable(9),
                schema_version: 1,
                chunk_pointers: vec![],
                root_document_ids: vec![],
            },
        )
        .unwrap();
        (
            chunk_path(other, &hash),
            crate::generation::loader::manifest_path(other, &manifest_hash),
        )
    }

    /// 보안 감사 STOR-2: 저장소 디렉터리나 shard가 symlink면 따라가지 않고 아무 것도 지우지 않습니다.
    /// 프로젝트 밖(다른 프로젝트)의 content-addressed 파일은 그대로 남습니다.
    #[cfg(unix)]
    #[test]
    fn symlinked_storage_directories_delete_nothing() {
        let victim_shard = |chunk: &Path| {
            chunk
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        };
        let cases: [&[&str]; 6] = [
            &[".stellar"],
            &[".stellar", "cache"],
            &[".stellar", "cache", "chunks"],
            &[".stellar", "cache", "chunks", "objects"],
            &[".stellar", "cache", "chunks", "generations"],
            &[".stellar", "cache", "chunks", "objects", "<shard>"],
        ];
        for case in cases {
            let dir = TempDir::new();
            let project = dir.path.join("project");
            let other = dir.path.join("other");
            let (victim_chunk, victim_manifest) = victim_project(&other);
            // 프로젝트 안의 진짜 orphan도 하나 둡니다(전부 아니면 전무: 이것도 지우지 않음).
            let own_orphan = {
                let store = ChunkStore::new(&project);
                let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) =
                    store.write_chunk(KIND, b"own-orphan").unwrap();
                chunk_path(&project, &hash)
            };
            let shard = victim_shard(&victim_chunk);
            let components: Vec<&str> = case
                .iter()
                .map(|c| if *c == "<shard>" { shard.as_str() } else { c })
                .collect();
            let mut link = project.clone();
            let mut target = other.clone();
            for component in &components {
                link.push(component);
                target.push(component);
            }
            if link.exists() {
                std::fs::remove_dir_all(&link).unwrap();
            }
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();

            let outcome = sweep_unreferenced(&project, &BTreeSet::new(), &policy(0), after_grace());
            assert!(outcome.deleted.is_empty(), "{components:?}: {outcome:?}");
            assert_eq!(outcome.carry_over, vec![link.clone()], "{components:?}");
            assert!(victim_chunk.is_file(), "{components:?}: 밖의 chunk 보존");
            assert!(
                victim_manifest.is_file(),
                "{components:?}: 밖의 manifest 보존"
            );
            if own_orphan.exists() {
                assert!(own_orphan.is_file());
            }
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }

    /// 보안 감사 STOR-3: symlink 순환을 따라가지 않으므로 방문 수가 실제 파일 수에 비례하고 같은
    /// 파일을 중복 수집하지 않습니다(작업량 계수). 이전에는 순환 하나에 32배, 둘이면 2^32 수준이었습니다.
    #[cfg(unix)]
    #[test]
    fn symlink_loops_are_not_traversed() {
        use crate::fs_guard::test_support::bounded;

        let dir = TempDir::new();
        let mut reachable = BTreeSet::new();
        for i in 0..20u32 {
            reachable.insert(write_chunk(&dir, format!("live-{i}").as_bytes()));
        }
        let orphan = write_chunk(&dir, b"orphan");
        let objects = crate::location::cache_dir(&dir.path).join("objects");
        for name in ["a", "b", "loop"] {
            std::os::unix::fs::symlink(".", objects.join(name)).unwrap();
        }
        // shard 안의 순환·하위 디렉터리도 보지 않습니다.
        let shard = chunk_path(&dir.path, &orphan)
            .parent()
            .unwrap()
            .to_path_buf();
        std::os::unix::fs::symlink("..", shard.join("up")).unwrap();
        std::fs::create_dir(shard.join("nested")).unwrap();
        std::fs::write(
            shard
                .join("nested")
                .join(ChunkObjectPath::for_hash(&orphan).file_name()),
            b"nested copy",
        )
        .unwrap();

        let root = dir.path.clone();
        let reachable_for_sweep = reachable.clone();
        let outcome = bounded("symlink 순환 sweep", move || {
            sweep_unreferenced(&root, &reachable_for_sweep, &policy(0), after_grace())
        });
        assert_eq!(outcome.kept_reachable, 20, "중복 수집 없음: {outcome:?}");
        assert_eq!(outcome.deleted, vec![chunk_path(&dir.path, &orphan)]);
        assert!(outcome.carry_over.is_empty(), "{outcome:?}");
        for name in ["a", "b", "loop"] {
            assert!(std::fs::symlink_metadata(objects.join(name))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert!(
            shard.join("nested").is_dir(),
            "하위 디렉터리 내용은 건드리지 않음"
        );
    }

    /// shard 안의 chunk 이름 symlink, 다른 shard에 놓인 chunk 이름 파일, generations 하위 디렉터리의
    /// manifest 이름 파일은 삭제 후보가 아닙니다(고정 구조 밖).
    #[cfg(unix)]
    #[test]
    fn symlinks_and_misplaced_names_are_never_deleted() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let victim = outside.path.join("victim");
        std::fs::write(&victim, b"USER DATA").unwrap();
        let orphan = write_chunk(&dir, b"orphan-for-names");
        let real = chunk_path(&dir.path, &orphan);
        // 같은 이름을 symlink로 바꿉니다.
        std::fs::remove_file(&real).unwrap();
        std::os::unix::fs::symlink(&victim, &real).unwrap();
        // 정규 shard가 아닌 shard에 둔 chunk 이름 파일.
        let wrong_prefix = if ChunkObjectPath::for_hash(&orphan).prefix() == "00" {
            "01"
        } else {
            "00"
        };
        let misplaced_dir = crate::location::cache_dir(&dir.path)
            .join("objects")
            .join(wrong_prefix);
        std::fs::create_dir_all(&misplaced_dir).unwrap();
        let misplaced = misplaced_dir.join(ChunkObjectPath::for_hash(&orphan).file_name());
        std::fs::write(&misplaced, b"misplaced").unwrap();
        // generations 하위 디렉터리의 manifest 이름 파일.
        let nested = crate::location::cache_dir(&dir.path)
            .join("generations")
            .join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let nested_manifest = nested.join(format!(
            "sha256-{}.starmanifest",
            ContentHash::compute(b"m").digest_hex()
        ));
        std::fs::write(&nested_manifest, b"nested").unwrap();

        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert!(outcome.deleted.is_empty(), "{outcome:?}");
        assert!(std::fs::symlink_metadata(&real)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&victim).unwrap(), b"USER DATA");
        assert!(misplaced.is_file());
        assert!(nested_manifest.is_file());
    }

    /// 수집 뒤 삭제 직전에 파일이 다른 파일로 바뀌거나 shard가 symlink로 바뀌면 지우지 않고 이월합니다.
    #[cfg(unix)]
    #[test]
    fn entries_replaced_after_collection_are_not_deleted() {
        // (a) 같은 이름의 다른 파일(새 inode).
        let dir = TempDir::new();
        let orphan = write_chunk(&dir, b"replaced-file");
        let path = chunk_path(&dir.path, &orphan);
        let outcome = sweep_observed(
            &dir.path,
            &BTreeSet::new(),
            &policy(0),
            after_grace(),
            &mut |p| {
                let keep = p.with_extension("kept");
                std::fs::rename(p, keep).unwrap();
                std::fs::write(p, b"replacement").unwrap();
            },
        );
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.carry_over, vec![path.clone()]);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");

        // (b) shard 디렉터리를 밖으로 옮기고 그 자리에 symlink(파일 identity는 같음).
        let dir = TempDir::new();
        let outside = TempDir::new();
        let orphan = write_chunk(&dir, b"replaced-shard");
        let path = chunk_path(&dir.path, &orphan);
        let shard = path.parent().unwrap().to_path_buf();
        let moved = outside.path.join("shard");
        let outcome = sweep_observed(
            &dir.path,
            &BTreeSet::new(),
            &policy(0),
            after_grace(),
            &mut |_| {
                std::fs::rename(&shard, &moved).unwrap();
                std::os::unix::fs::symlink(&moved, &shard).unwrap();
            },
        );
        assert!(outcome.deleted.is_empty(), "{outcome:?}");
        assert_eq!(outcome.carry_over, vec![path.clone()]);
        assert!(
            moved
                .join(ChunkObjectPath::for_hash(&orphan).file_name())
                .is_file(),
            "밖으로 옮겨진 파일 보존"
        );
    }
}
