//! 같은 원본 digest의 snapshot cache 재사용 (ADR-0138 결정 1).
//!
//! semantic Git은 Git tree(HEAD·index·후보·merge stage·HEAD 비교)를 비공개 임시 폴더에 materialize하고, 그 안에서
//! 텍스트 원본을 해석해 chunk cache를 만든 뒤 저장소 loader로 엽니다([`super::prepare_snapshot`], ADR-0122 결정 5).
//! 원본 해석과 직렬화·chunk 쓰기는 문서 크기에 비례하고, stage 한 번에도 index·작업 트리·HEAD를 따로 엽니다. 그런데
//! 그 결과는 입력에만 달린 값입니다: cache가 없는 snapshot에서 원본으로 만든 generation은 (프로젝트 id, 원본 파일
//! 집합 digest, 제품 schema 번호, 실행 중인 build)가 같으면 bytes까지 같습니다. 그래서 그 넷을 key로 한 번 만든
//! generation을 보관하고, 같은 key의 snapshot은 원본을 해석하지 않고 그 generation 파일을 옮겨 놓습니다.
//!
//! - **위치**: 사용자 임시 폴더 아래 `stellar-snapshot-cache/`(materialize한 snapshot과 같은 volume이라 hard link로
//!   옮김). 폴더는 `0700`으로 만들고, symlink가 아닌 실제 폴더이며 group·other 비트가 없고 이 사용자가 만든 항목과
//!   소유자가 같을 때만 씁니다. 아니면 재사용하지 않습니다(원본에서 만듦).
//! - **항목**: `<key 64자 hex>/` 하나가 작은 프로젝트 root처럼 cache 배치(`.stellar/cache/chunks/`, ADR-0122 결정 1)를
//!   가집니다: current pointer, manifest, manifest가 가리키는 chunk object만. 항목은 비공개 임시 폴더에 다 쓴 뒤
//!   rename으로 공개하므로 반쯤 쓴 항목은 보이지 않습니다. 같은 key를 두 프로세스가 동시에 공개하면 먼저 끝난 쪽이
//!   남습니다(내용이 같음).
//! - **꺼내기**: 항목의 manifest를 검증하고([`verify_generation`]) chunk마다 envelope·hash를 다시 확인한 뒤
//!   ([`ChunkStore::read_chunk`]) snapshot에 hard link(다른 volume이면 임시 이름 복사 뒤 rename)로 놓고, current
//!   pointer와 stamp(원본 digest ↔ manifest)를 씁니다. 그래서 그 snapshot의 열기 계획은 `Cache`이고 저장소 loader가
//!   평소처럼 엽니다. 확인이 하나라도 실패하면 꺼내지 않고 원본에서 만듭니다(현재 pointer를 쓰기 전에 실패하므로
//!   snapshot에는 의미가 남지 않음).
//! - **수명**: 마지막 사용 시각(항목 current pointer 파일의 수정 시각, 꺼낼 때 갱신) 기준으로 최근 [`MAX_ENTRIES`]개만
//!   남기고, 보존 정책의 유예 시간(`config/storage-gc.yaml` `grace_period_seconds`)보다 오래 쓰지 않은 항목은 지웁니다
//!   ([`SnapshotStore::prune`]). 공개할 때와 앱 runtime의 유휴 GC가 부릅니다. 사용자 작업 트리의 cache·Git과는 무관합니다.
//!
//! 사용자 작업 트리의 cache는 이 store에 넣지 않습니다. 작업 트리 cache는 앱·CLI가 이어 쓴 generation이라 원본만으로
//! 다시 만든 결과와 chunk 구성이 같다는 보장이 없고(이미지 metadata 등), 다른 writer와 동시에 바뀔 수 있습니다.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use stellar_core_types::ids::StableId;
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::generation::current::{read_current_pointer, write_current_pointer};
use stellar_storage::generation::loader::{manifest_path, verify_generation};
use stellar_storage::hash::ContentHash;

use crate::document_io::session::{write_stamp, Stamp};
use crate::private_temp::{remove_private_tree, sweep_stale_private_temps, PrivateTempDir};

/// 사용자 임시 폴더 아래 store 폴더 이름.
pub const STORE_DIR_NAME: &str = "stellar-snapshot-cache";

/// 남기는 최근 항목 수. HEAD·index·작업 트리·병합 세 side가 한 작업에서 함께 쓰이고, 작업 트리는 편집마다 바뀌므로
/// 그보다 넉넉한 값입니다(ADR-0138 결정 1).
pub const MAX_ENTRIES: usize = 16;

/// 공개 중인 항목을 쓰는 비공개 임시 폴더 접두사.
const PUBLISH_PREFIX: &str = "publish";

/// key 형식 첫 bytes.
const KEY_MAGIC: &[u8] = b"stellar-snapshot-cache/1";

/// snapshot cache key: (프로젝트 id, 원본 digest, 제품 schema 번호, 실행 중인 build).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotKey(ContentHash);

impl SnapshotKey {
    /// 이 build의 key. build 식별자를 얻지 못하면 `None`(재사용하지 않음)입니다.
    pub fn new(project: StableId, source: ContentHash) -> Option<SnapshotKey> {
        Some(SnapshotKey::with_build(
            project,
            source,
            build_fingerprint()?,
        ))
    }

    /// build 식별자를 직접 준 key(시험용).
    pub fn with_build(project: StableId, source: ContentHash, build: [u8; 32]) -> SnapshotKey {
        let mut input = KEY_MAGIC.to_vec();
        input.extend_from_slice(&build);
        input.extend_from_slice(&crate::migration::PRODUCT_SCHEMA_VERSION.to_le_bytes());
        input.extend_from_slice(project.to_string().as_bytes());
        input.push(0);
        input.extend_from_slice(source.digest());
        SnapshotKey(ContentHash::compute(&input))
    }

    fn dir_name(&self) -> String {
        self.0.digest_hex()
    }
}

/// 실행 중인 build 식별자: 실행 파일 경로·크기·수정 시각의 hash. 직렬화 code가 바뀐 build가 옛 항목을 쓰지 않게
/// 합니다(crate version은 개발 중 바뀌지 않으므로 쓰지 않음).
fn build_fingerprint() -> Option<[u8; 32]> {
    static FINGERPRINT: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    *FINGERPRINT.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        let mut input = exe.to_string_lossy().into_owned().into_bytes();
        input.push(0);
        input.extend_from_slice(&meta.len().to_le_bytes());
        input.extend_from_slice(&modified.as_nanos().to_le_bytes());
        Some(*ContentHash::compute(&input).digest())
    })
}

/// 같은 원본 digest의 snapshot generation 보관소(모듈 문서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStore {
    root: PathBuf,
}

/// [`SnapshotStore::prune`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// 남긴 항목 수.
    pub kept: usize,
    /// 지운 항목 수.
    pub removed: usize,
}

#[cfg(unix)]
fn private_bits(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private_bits(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn same_owner(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.uid() == b.uid()
}

#[cfg(not(unix))]
fn same_owner(_a: &std::fs::Metadata, _b: &std::fs::Metadata) -> bool {
    true
}

fn create_private(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(crate::private_temp::PRIVATE_DIR_MODE);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

impl SnapshotStore {
    /// 이 사용자의 기본 store(사용자 임시 폴더 아래 [`STORE_DIR_NAME`]). 비공개 조건을 만족하지 않으면 `None`입니다.
    /// 프로세스마다 한 번 확인합니다.
    pub fn user() -> Option<SnapshotStore> {
        static USER: OnceLock<Option<SnapshotStore>> = OnceLock::new();
        USER.get_or_init(|| SnapshotStore::at(&std::env::temp_dir().join(STORE_DIR_NAME)).ok())
            .clone()
    }

    /// `root`의 store를 만들거나 엽니다. 폴더가 symlink이거나 group·other 권한이 있거나, 이 사용자가 그 안에 만든
    /// 항목과 소유자가 다르면 거부합니다.
    pub fn at(root: &Path) -> io::Result<SnapshotStore> {
        create_private(root)?;
        let meta = std::fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.file_type().is_symlink() || !private_bits(&meta) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "snapshot cache 폴더가 비공개가 아닙니다",
            ));
        }
        let probe = PrivateTempDir::create_in(root, "probe")
            .map_err(|error| io::Error::other(error.to_string()))?;
        let owned = same_owner(&meta, &std::fs::symlink_metadata(probe.path())?);
        probe.cleanup()?;
        if !owned {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "snapshot cache 폴더 소유자가 다릅니다",
            ));
        }
        Ok(SnapshotStore {
            root: root.to_path_buf(),
        })
    }

    /// store 폴더.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn entry(&self, key: &SnapshotKey) -> PathBuf {
        self.root.join(key.dir_name())
    }

    /// `key` 항목을 snapshot `target`(cache가 없는 materialize한 폴더)에 놓고 stamp를 `source`와 짝지어 씁니다.
    /// 항목이 없거나 확인에 실패하면 `None`이고, 그때 `target`의 current pointer·stamp는 쓰지 않습니다. 돌려주는 값은
    /// (generation 번호, manifest hash)입니다.
    pub fn restore(
        &self,
        key: &SnapshotKey,
        target: &Path,
        project: StableId,
        source: ContentHash,
    ) -> Option<(u64, ContentHash)> {
        let entry = self.entry(key);
        let meta = std::fs::symlink_metadata(&entry).ok()?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return None;
        }
        let manifest = read_current_pointer(&entry).ok()?;
        let loaded = verify_generation(&entry, &manifest, project).ok()?;
        let chunks = ChunkStore::new(&entry);
        for pointer in &loaded.manifest.chunk_pointers {
            chunks
                .read_chunk(&pointer.content_hash, pointer.kind)
                .ok()?;
        }
        for pointer in &loaded.manifest.chunk_pointers {
            let relative = ChunkObjectPath::for_hash(&pointer.content_hash).to_relative_path();
            place(&entry.join(&relative), &target.join(&relative)).ok()?;
        }
        place(
            &manifest_path(&entry, &manifest),
            &manifest_path(target, &manifest),
        )
        .ok()?;
        write_current_pointer(target, &manifest).ok()?;
        write_stamp(target, &Stamp { source, manifest }).ok()?;
        touch(&stellar_storage::generation::current::current_pointer_path(
            &entry,
        ));
        Some((loaded.manifest.generation, manifest))
    }

    /// 원본에서 막 만든 snapshot `from`의 generation(`manifest`)을 `key` 항목으로 공개합니다. 실패는 무시해도 됩니다
    /// (다음 snapshot이 원본에서 다시 만듦). 이미 같은 key가 있으면 그대로 둡니다. 공개한 뒤 [`SnapshotStore::prune`]을
    /// 부릅니다.
    pub fn publish(
        &self,
        key: &SnapshotKey,
        from: &Path,
        project: StableId,
        manifest: ContentHash,
        now: SystemTime,
    ) -> io::Result<()> {
        let entry = self.entry(key);
        if std::fs::symlink_metadata(&entry).is_ok() {
            return Ok(());
        }
        let loaded = verify_generation(from, &manifest, project)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let temp = PrivateTempDir::create_in(&self.root, PUBLISH_PREFIX)
            .map_err(|error| io::Error::other(error.to_string()))?;
        for pointer in &loaded.manifest.chunk_pointers {
            let relative = ChunkObjectPath::for_hash(&pointer.content_hash).to_relative_path();
            place(&from.join(&relative), &temp.path().join(&relative))?;
        }
        place(
            &manifest_path(from, &manifest),
            &manifest_path(temp.path(), &manifest),
        )?;
        write_current_pointer(temp.path(), &manifest)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match std::fs::rename(temp.path(), &entry) {
            Ok(()) => {
                temp.keep();
            }
            // 다른 프로세스가 같은 key를 먼저 공개했습니다(내용이 같음). temp는 drop이 지웁니다.
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error),
        }
        self.prune(now, super::default_retention().grace_period_seconds);
        Ok(())
    }

    /// 최근 [`MAX_ENTRIES`]개만 남기고, `max_idle_seconds`보다 오래 쓰지 않은 항목과 비정상 종료로 남은 공개 중 폴더를
    /// 지웁니다(모듈 문서 "수명"). 지우지 못한 항목은 다음 실행으로 넘깁니다.
    pub fn prune(&self, now: SystemTime, max_idle_seconds: u64) -> PruneReport {
        let _ = sweep_stale_private_temps(&self.root, PUBLISH_PREFIX);
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return PruneReport::default();
        };
        let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let path = entry.path();
            let used = std::fs::metadata(
                stellar_storage::generation::current::current_pointer_path(&path),
            )
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
            found.push((used, path));
        }
        // 최근 사용 순(같은 시각이면 이름 순).
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let max_idle = Duration::from_secs(max_idle_seconds);
        let mut report = PruneReport::default();
        for (index, (used, path)) in found.into_iter().enumerate() {
            let stale = now.duration_since(used).is_ok_and(|idle| idle > max_idle);
            if index < MAX_ENTRIES && !stale {
                report.kept += 1;
            } else if remove_private_tree(&path).is_ok() {
                report.removed += 1;
            } else {
                report.kept += 1;
            }
        }
        report
    }
}

/// 내용 주소 파일 `from`을 `to`에 놓습니다: 이미 있으면 그대로(같은 hash 주소는 같은 내용), 없으면 hard link, 다른
/// volume이면 임시 이름으로 복사한 뒤 rename합니다(반쯤 쓴 파일이 최종 이름에 보이지 않음).
fn place(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(from, to) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => {}
    }
    let name = to
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = to.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::copy(from, &temp)?;
    std::fs::rename(&temp, to).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// 마지막 사용 시각을 지금으로 바꿉니다(실패는 무시: 수명 계산만 늦어짐).
fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}
