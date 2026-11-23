//! 로컬 chunk cache 관리: 옛 배치 이동, 원본에서 다시 만들기, GC, Git index 정리 (V.10.7, ADR-0122).
//!
//! 텍스트 원본(`design/**/*.stellar.yaml`)이 Git 원본이고 chunk generation은 Git이 추적하지 않는 로컬 cache
//! (`.stellar/cache/chunks/`, [`stellar_storage::location`])입니다. 이 모듈은 cache의 수명을 다루는 한 곳입니다.
//!
//! | 함수 | 하는 일 | 부르는 곳 |
//! |---|---|---|
//! | [`migrate_layout`] | 옛 배치(`design/current`·`generations/`·`objects/`)를 cache 위치로 옮김 | 열기 관찰(`document_io::session::observe`), Git snapshot, `stellar cache migrate` |
//! | [`rebuild_from_text`] | 원본과 짝이 아닌 cache를 원본에서 다시 만듦 | clone 직후·Git pull 뒤 `stellar cache rebuild`, materialize한 Git snapshot |
//! | [`collect_garbage`] | 보존 정책 밖 manifest·chunk를 지움 | `stellar cache gc`, `stellar cache migrate` |
//! | [`untrack_legacy`] | Git index에서 옛 배치 경로를 뺌(작업 트리·HEAD는 그대로) | semantic Git stage·unstage·commit, `stellar cache migrate` |
//! | [`prepare_snapshot`] | materialize한 Git snapshot(HEAD·index·후보·merge stage)의 cache를 원본에서 만듦 | semantic Git, HEAD 비교 |
//! | [`text_snapshot`] | 작업 트리 원본만 비공개 사본으로 복사하고 cache를 만듦 | semantic Git stage(작업 트리 쪽) |
//!
//! Git이 담는 것은 원본뿐이라, Git tree를 materialize한 snapshot에는 cache가 없습니다. generation을 여는 저장소
//! loader(`saved_project`)를 그대로 쓰려고 snapshot 안에서 원본으로 cache를 만듭니다([`prepare_snapshot`]). 옛
//! commit(원본 없이 옛 배치만 있음)도 같은 함수가 snapshot 안에서 옛 배치를 옮겨 엽니다.
//!
//! cache는 언제든 지우고 원본에서 다시 만들 수 있습니다(ADR-0059 결정 3). 그래서 GC는 Git과 무관하게 cache만
//! 지우고, 원본이 없는 옛 chunk 프로젝트의 Git index는 건드리지 않습니다(그 경우 Git에 남은 옛 cache가 유일한
//! 원본이기 때문).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use stellar_core_types::ids::StableId;
use stellar_document::records::NodePayload;
use stellar_git_porcelain::runner::argv;
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_ASSET_METADATA;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::gc::mark::mark_reachable;
use stellar_storage::gc::retention::{retained_generations, RetentionInput, RetentionPolicy};
use stellar_storage::gc::sweep::sweep_unreferenced;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::generation::loader::{load_current_generation, verify_generation};
use stellar_storage::hash::ContentHash;
use stellar_storage::location::{move_legacy_layout, LegacyMove, LegacyMoveError};
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

pub use stellar_storage::location::{
    cache_dir, is_cache_path, is_legacy_cache_path, is_local_cache_path, CACHE_RELATIVE,
    LEGACY_PATHS,
};

use crate::document_io::serialize::serialize_document;
use crate::document_io::session::{
    decode_source, observe, read_source, write_stamp, OpenPlan, SourceLibrary, Stamp, STAMP_PATH,
};
use crate::identity::parse_project_id;
use crate::manifest::Manifest;
use crate::root::MANIFEST_FILE;
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// 저장소 GC 보존 정책 원본(`config/storage-gc.yaml`, V.7.1). 수치를 코드에 두지 않고 config 파일을 읽습니다.
const RETENTION_CONFIG: &str = include_str!("../../../config/storage-gc.yaml");

/// 제품 기본 보존 정책(`config/storage-gc.yaml`).
pub fn default_retention() -> RetentionPolicy {
    RetentionPolicy::from_config_str(RETENTION_CONFIG)
        .expect("config/storage-gc.yaml은 저장소 시험(retention)이 검증함")
}

/// cache 관리 실패.
#[derive(Debug)]
pub enum CacheError {
    /// 옛 배치를 옮기지 못했습니다.
    Legacy(LegacyMoveError),
    /// `stellar.yaml`을 읽거나 해석할 수 없습니다.
    Manifest(String),
    /// 텍스트 원본을 읽거나 해석할 수 없습니다.
    Source(String),
    /// cache를 쓰거나 지울 수 없습니다.
    Write(String),
    /// Git을 실행할 수 없거나 실패했습니다.
    Git(String),
    /// 작업 트리에 텍스트 원본 없이 옛 배치만 있습니다. 옮기면 Git에 그 프로젝트의 원본이 남지 않으므로 옮기지
    /// 않습니다([`migrate_working_tree`]).
    LegacyWithoutSource,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Legacy(error) => write!(f, "{error}"),
            CacheError::Manifest(error) => write!(f, "stellar.yaml: {error}"),
            CacheError::Source(error) => write!(f, "텍스트 원본: {error}"),
            CacheError::Write(error) => write!(f, "chunk cache: {error}"),
            CacheError::Git(error) => write!(f, "Git: {error}"),
            CacheError::LegacyWithoutSource => f.write_str(
                "텍스트 원본(design/*.stellar.yaml) 없이 예전 저장 배치(design/current)만 있는 프로젝트입니다. \
                 저장 cache를 Git 밖으로 옮기면 Git에 원본이 남지 않으므로 옮기지 않았습니다. 이 프로젝트는 먼저 \
                 텍스트 원본으로 바꿔야 합니다",
            ),
        }
    }
}

impl std::error::Error for CacheError {}

/// [`migrate_layout`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutMigration {
    /// 옛 배치 이동 결과.
    pub moved: LegacyMove,
    /// 옛 current pointer가 cache pointer를 바꿔 원본·cache 짝(stamp)을 지웠는지.
    pub stamp_cleared: bool,
}

fn stamp_path(root: &Path) -> PathBuf {
    STAMP_PATH
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// 옛 배치를 `.stellar/cache/chunks/`로 옮깁니다([`move_legacy_layout`]). 옛 pointer가 cache pointer를 바꿨으면
/// 그 cache가 어느 원본과 짝인지 알 수 없으므로 stamp를 지웁니다(다음 열기는 원본이 있으면 원본에서 읽음). 옛 배치가
/// 없으면 아무것도 하지 않습니다. Git index는 건드리지 않습니다([`untrack_legacy`]).
pub fn migrate_layout(root: &Path) -> Result<LayoutMigration, CacheError> {
    let moved = move_legacy_layout(root).map_err(CacheError::Legacy)?;
    let mut stamp_cleared = false;
    if moved.current_replaced {
        match std::fs::remove_file(stamp_path(root)) {
            Ok(()) => stamp_cleared = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CacheError::Write(error.to_string())),
        }
    }
    Ok(LayoutMigration {
        moved,
        stamp_cleared,
    })
}

/// 작업 트리용 [`migrate_layout`]: 텍스트 원본이 없는데 옛 배치가 있으면 옮기지 않고
/// [`CacheError::LegacyWithoutSource`]입니다(ADR-0122 개정 2026-11-24). 옛 배치는 그 프로젝트의 유일한 Git 원본일 수
/// 있어, 옮긴 뒤 `git commit -a` 한 번으로 원본이 Git에서 사라지는 것을 막습니다. Git snapshot을 푼 임시 폴더는
/// [`migrate_layout`]을 그대로 씁니다(작업 트리가 아님).
pub fn migrate_working_tree(root: &Path) -> Result<LayoutMigration, CacheError> {
    let legacy = ["current", "generations", "objects"]
        .iter()
        .any(|name| std::fs::symlink_metadata(root.join("design").join(name)).is_ok());
    if legacy {
        let text = crate::document_io::session::read_source(root)
            .map_err(|e| CacheError::Source(e.to_string()))?;
        if text.is_none() {
            return Err(CacheError::LegacyWithoutSource);
        }
    }
    migrate_layout(root)
}

/// `stellar.yaml`의 project id.
pub fn project_id(root: &Path) -> Result<StableId, CacheError> {
    let bytes =
        stellar_storage::source_file::read(&root.join(MANIFEST_FILE), MAX_INPUT_BYTES as u64)
            .map_err(|error| CacheError::Manifest(error.to_string()))?;
    let text =
        std::str::from_utf8(&bytes).map_err(|error| CacheError::Manifest(error.to_string()))?;
    let value = load_restricted(text).map_err(|error| CacheError::Manifest(error.to_string()))?;
    let manifest =
        Manifest::from_yaml(&value).map_err(|error| CacheError::Manifest(error.to_string()))?;
    parse_project_id(&manifest.project.id)
        .ok_or_else(|| CacheError::Manifest(format!("project id 형식: {}", manifest.project.id)))
}

/// [`rebuild_from_text`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rebuild {
    /// cache가 원본과 짝입니다(또는 원본 이후 cache만 쓴 writer가 있어 cache를 보존함). 쓰지 않았습니다.
    Fresh,
    /// 원본에서 cache generation을 새로 쓰고 stamp를 맞췄습니다.
    Rebuilt {
        /// 새 generation 번호.
        generation: u64,
        /// 새 manifest hash.
        manifest: ContentHash,
    },
    /// 텍스트 원본이 없습니다(옛 chunk 프로젝트 또는 빈 폴더). 쓰지 않았습니다.
    NoTextSource,
}

/// 원본과 짝이 아닌 cache를 텍스트 원본에서 다시 만듭니다(clone 직후 cache가 없을 때, Git pull·checkout 뒤).
///
/// 열기 계획([`OpenPlan`])이 [`OpenPlan::FromText`]일 때만 씁니다. 원본 문서를 chunk로 직렬화하고 문서가 참조하는
/// 이미지 record를 asset metadata chunk로 더해 새 generation을 쓴 뒤(옛 current가 있으면 그 다음 번호) stamp를
/// 씁니다. 원본이 Git 원본이라 옛 cache chunk는 이어 쓰지 않습니다. 교차 프로세스 저장 잠금은 호출자가 잡습니다
/// (materialize한 비공개 snapshot은 잠금 대상이 아님). 원본 파일은 쓰지 않습니다.
pub fn rebuild_from_text(root: &Path) -> Result<Rebuild, CacheError> {
    let disk = observe(root).map_err(|error| CacheError::Source(error.to_string()))?;
    match disk.plan {
        OpenPlan::Cache | OpenPlan::Forward => return Ok(Rebuild::Fresh),
        OpenPlan::Migrate | OpenPlan::Empty => return Ok(Rebuild::NoTextSource),
        OpenPlan::FromText => {}
    }
    let files = disk.text.expect("FromText 계획에는 원본이 있음");
    let project_id = project_id(root)?;
    let (model, library) = SourceLibrary::split(
        decode_source(&files).map_err(|error| CacheError::Source(error.to_string()))?,
    );
    let serialized =
        serialize_document(&model).map_err(|error| CacheError::Source(error.to_string()))?;
    let mut states = serialized.to_dirty_states();
    let referenced: BTreeSet<StableId> = model
        .to_records()
        .map_err(|error| CacheError::Source(error.to_string()))?
        .nodes
        .iter()
        .filter_map(|node| match &node.payload {
            NodePayload::Image(image) => Some(image.asset.id()),
            _ => None,
        })
        .collect();
    let images: BTreeMap<_, _> = library
        .images
        .iter()
        .filter(|(id, _)| referenced.contains(*id))
        .collect();
    for (id, record) in images {
        states.push(LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(*id),
            kind: KIND_ASSET_METADATA,
            schema_version: 1,
            payload: record
                .encode()
                .map_err(|error| CacheError::Source(format!("이미지 record: {error:?}")))?,
        });
    }
    // 옛 cache가 같은 프로젝트의 정상 세대면 다음 번호로 잇습니다(GC 보존 순서). 아니면 1세대부터입니다.
    let base = disk
        .current
        .and_then(|_| load_current_generation(root, project_id).ok());
    let plan = build_save_plan(
        base.as_ref(),
        project_id,
        crate::migration::PRODUCT_SCHEMA_VERSION,
        &states,
        vec![model.document_id()],
    )
    .map_err(|error| CacheError::Write(error.to_string()))?;
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|base| (base.manifest.generation, base.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new())
        .map_err(|error| CacheError::Write(error.to_string()))?;
    let (generation, manifest) = memory
        .saved()
        .ok_or_else(|| CacheError::Write("저장 결과 generation이 없습니다".into()))?;
    write_stamp(
        root,
        &Stamp {
            source: files.base.digest(),
            manifest,
        },
    )
    .map_err(|error| CacheError::Write(error.to_string()))?;
    Ok(Rebuild::Rebuilt {
        generation,
        manifest,
    })
}

/// materialize한 Git snapshot(비공개 임시 폴더)의 cache를 준비합니다: 옛 배치를 옮기고([`migrate_layout`]), 원본이
/// 있으면 원본에서 cache를 만듭니다([`rebuild_from_text`]). 원본도 옛 배치도 없으면 아무것도 하지 않습니다(저장소
/// loader가 pointer 누락을 보고). snapshot은 이 프로세스만 쓰므로 잠금을 잡지 않습니다. 사용자 작업 트리에는 쓰지
/// 마십시오(작업 트리는 [`text_snapshot`]).
pub fn prepare_snapshot(root: &Path) -> Result<Rebuild, CacheError> {
    migrate_layout(root)?;
    rebuild_from_text(root)
}

/// 쓸 수 없는 Git 사본(library cache entry처럼 commit 하나를 읽기 전용으로 materialize한 폴더)에 옛 배치만 있으면, 옛
/// 배치 파일을 비공개 임시 폴더로 복사하고 그 안에서 cache 위치로 옮긴 사본을 돌려줍니다. 새 위치에 cache가 있거나 옛
/// 배치가 없으면 `None`(그 폴더를 그대로 읽음)입니다. ADR-0122 이전에 commit된 library(텍스트 원본이 아직 담지 못하는
/// Property·Slot 정의의 component library chunk)를 읽는 호환 경로이고, 원래 폴더는 바꾸지 않습니다.
pub fn legacy_view(root: &Path) -> Result<Option<crate::private_temp::PrivateTempDir>, CacheError> {
    let legacy_current = root
        .join(stellar_storage::location::LEGACY_DIR)
        .join(stellar_storage::location::CURRENT_FILE);
    if stellar_storage::generation::current::current_pointer_path(root).exists()
        || !legacy_current.is_file()
    {
        return Ok(None);
    }
    let temp = crate::private_temp::PrivateTempDir::create_in(
        &std::env::temp_dir(),
        "stellar-legacy-view",
    )
    .map_err(|error| CacheError::Write(error.to_string()))?;
    fn copy(from: &Path, to: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(from)?;
        if meta.is_dir() {
            std::fs::create_dir_all(to)?;
            for entry in std::fs::read_dir(from)? {
                let entry = entry?;
                copy(&entry.path(), &to.join(entry.file_name()))?;
            }
        } else if meta.is_file() {
            std::fs::copy(from, to)?;
        }
        Ok(())
    }
    for relative in LEGACY_PATHS {
        let from = root.join(relative);
        if std::fs::symlink_metadata(&from).is_ok() {
            let to = temp.path().join(relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| CacheError::Write(e.to_string()))?;
            }
            copy(&from, &to).map_err(|e| CacheError::Write(e.to_string()))?;
        }
    }
    migrate_layout(temp.path())?;
    Ok(Some(temp))
}

/// 작업 트리 `root`의 `stellar.yaml`과 텍스트 원본만 `parent` 아래 비공개 임시 폴더로 복사하고 그 안에 cache를
/// 만듭니다. 작업 트리의 cache가 원본과 짝인지와 관계없이 원본(Git 원본)의 의미로 엽니다. 원본이 없으면 `None`입니다.
/// 작업 트리는 읽기만 합니다.
pub fn text_snapshot(
    root: &Path,
    parent: &Path,
) -> Result<Option<crate::private_temp::PrivateTempDir>, CacheError> {
    let Some(files) = read_source(root).map_err(|error| CacheError::Source(error.to_string()))?
    else {
        return Ok(None);
    };
    let temp = crate::private_temp::PrivateTempDir::create_in(parent, "stellar-text-snapshot")
        .map_err(|error| CacheError::Write(error.to_string()))?;
    let write = |relative: &Path, bytes: &[u8]| -> Result<(), CacheError> {
        let path = temp.path().join(relative);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| CacheError::Write(error.to_string()))?;
        }
        std::fs::write(&path, bytes).map_err(|error| CacheError::Write(error.to_string()))
    };
    let manifest =
        stellar_storage::source_file::read(&root.join(MANIFEST_FILE), MAX_INPUT_BYTES as u64)
            .map_err(|error| CacheError::Manifest(error.to_string()))?;
    write(Path::new(MANIFEST_FILE), &manifest)?;
    let design = Path::new(crate::document_io::session::DESIGN_DIR);
    for (relative, text) in &files.files {
        write(&design.join(relative), text.as_bytes())?;
    }
    rebuild_from_text(temp.path())?;
    Ok(Some(temp))
}

/// [`collect_garbage`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// 보존한 manifest(current·최근 정상 세대·stamp 짝).
    pub retained: Vec<ContentHash>,
    /// 지운 파일 수.
    pub deleted: usize,
    /// 보존 세대가 참조해 남긴 파일 수.
    pub kept_reachable: usize,
    /// 유예 시간 안이라 남긴 파일 수.
    pub kept_in_grace: usize,
    /// 지우지 못해 다음 실행으로 넘긴 경로.
    pub carry_over: Vec<PathBuf>,
}

/// cache의 정상 세대 목록(번호, manifest hash). 읽을 수 없거나 다른 프로젝트 세대는 뺍니다.
fn valid_generations(root: &Path, project_id: StableId) -> Vec<(u64, ContentHash)> {
    let dir = cache_dir(root).join(stellar_storage::location::GENERATIONS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(hash) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".starmanifest"))
            .and_then(|stem| stem.split_once('-'))
            .and_then(|(algo, hex)| format!("{algo}:{hex}").parse::<ContentHash>().ok())
        else {
            continue;
        };
        if let Ok(loaded) = verify_generation(root, &hash, project_id) {
            out.push((loaded.manifest.generation, hash));
        }
    }
    out.sort();
    out
}

/// 보존 정책 밖의 cache manifest·chunk를 지웁니다(V.7, ADR-0122 결정 4).
///
/// 보존: current 세대, 정상 세대 가운데 최근 `keep_recent_generations`개, stamp가 가리키는 세대. mark(V.7.2)한
/// 집합에 없는 파일만 유예 시간(`grace_period_seconds`)이 지난 뒤 지웁니다. cache가 Git 밖이라 지워도 Git 추적
/// 파일은 바뀌지 않습니다. 보존 세대 manifest를 읽을 수 없으면 아무것도 지우지 않습니다(과잉 삭제 방지). 교차
/// 프로세스 저장 잠금은 호출자가 잡습니다.
pub fn collect_garbage(
    root: &Path,
    policy: &RetentionPolicy,
    now: SystemTime,
) -> Result<GcReport, CacheError> {
    let project_id = project_id(root)?;
    let valid = valid_generations(root, project_id);
    let current = read_current_pointer(root).ok();
    let current_generation = current.and_then(|hash| {
        valid
            .iter()
            .find(|(_, valid)| *valid == hash)
            .map(|(number, _)| *number)
    });
    let numbers = retained_generations(
        policy,
        &RetentionInput {
            current: current_generation,
            valid_generations: valid.iter().map(|(number, _)| *number).collect(),
            recovery_generation: None,
        },
    );
    let mut retained: BTreeSet<ContentHash> = valid
        .iter()
        .filter(|(number, _)| numbers.contains(number))
        .map(|(_, hash)| *hash)
        .collect();
    if let Some(stamp) = crate::document_io::session::read_stamp(root) {
        if valid.iter().any(|(_, hash)| *hash == stamp.manifest) {
            retained.insert(stamp.manifest);
        }
    }
    // 손상된 current도 진단 자료로 보존합니다(retention 정책과 같음). mark는 읽을 수 있는 세대만 합니다.
    let retained: Vec<ContentHash> = retained.into_iter().collect();
    let mut reachable =
        mark_reachable(root, &retained).map_err(|error| CacheError::Write(error.to_string()))?;
    if let Some(current) = current {
        reachable.insert(current);
    }
    let outcome = sweep_unreferenced(root, &reachable, policy, now);
    Ok(GcReport {
        retained,
        deleted: outcome.deleted.len(),
        kept_reachable: outcome.kept_reachable,
        kept_in_grace: outcome.kept_in_grace,
        carry_over: outcome.carry_over,
    })
}

/// Git index에서 옛 배치 경로(`design/current`·`design/generations/`·`design/objects/`)를 뺍니다
/// (`git rm -r --cached`). 작업 트리와 HEAD는 바꾸지 않으므로 다음 commit이 옛 cache를 Git에서 지웁니다.
///
/// 작업 트리에 텍스트 원본이 있을 때만 뺍니다. 원본이 없으면 Git에 남은 옛 cache가 그 프로젝트의 유일한 Git
/// 원본이라 그대로 둡니다(빈 목록). 돌려주는 값은 index에서 뺀 경로(프로젝트 root 기준 `/` 경로)입니다. `root`가
/// 저장소 안 하위 폴더여도 Git pathspec은 `root` 기준으로 해석됩니다.
pub fn untrack_legacy(git: &GatedGitRunner, root: &Path) -> Result<Vec<String>, CacheError> {
    let run = |args: Vec<&str>| -> Result<Vec<u8>, CacheError> {
        let output = git
            .run(&argv(args), root)
            .map_err(|error| CacheError::Git(error.to_string()))?;
        if !output.success() {
            return Err(CacheError::Git(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(output.stdout)
    };
    let mut list = vec!["ls-files", "-z", "--cached", "--"];
    list.extend(LEGACY_PATHS);
    let listed = run(list)?;
    if listed.is_empty() {
        return Ok(Vec::new());
    }
    match read_source(root) {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(Vec::new()),
        Err(error) => return Err(CacheError::Source(error.to_string())),
    }
    let mut remove = vec!["rm", "-r", "-q", "--cached", "--ignore-unmatch", "--"];
    remove.extend(LEGACY_PATHS);
    run(remove)?;
    let mut paths: Vec<String> = listed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retention_reads_the_repository_config() {
        let policy = default_retention();
        assert!(policy.keep_recent_generations >= 1);
    }
}

#[cfg(test)]
mod working_tree_tests {
    use super::*;

    /// 텍스트 원본 없이 옛 배치만 있으면 옮기지 않고 거부합니다. 옛 파일은 작업 트리에 그대로 남습니다.
    #[test]
    fn legacy_layout_without_a_text_source_is_left_in_place() {
        let dir = std::env::temp_dir().join(format!("stellar-legacy-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("design/generations")).unwrap();
        std::fs::write(dir.join("design/current"), b"legacy pointer").unwrap();
        let error = migrate_working_tree(&dir).unwrap_err();
        assert!(matches!(error, CacheError::LegacyWithoutSource), "{error}");
        assert!(
            dir.join("design/current").is_file(),
            "옛 pointer가 그대로 남음"
        );
        assert!(dir.join("design/generations").is_dir());
        assert!(
            !dir.join(".stellar/cache/chunks").exists(),
            "cache로 옮기지 않음"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
