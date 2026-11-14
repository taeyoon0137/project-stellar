//! 컴포넌트 CLI는 같은 원본 폴더를 변경하는 협조적 writer와 저장 잠금을 공유합니다.
use std::path::{Path, PathBuf};

use crate::apply::ApplyError;
use crate::core_types::{ids::StableId, value::Size2};
use crate::lock::{default_lock_dir, SaveLock};
use crate::text_source::TextSourceOperationError as Error;
use stellar_document::record_codec::decode_payload;
use stellar_document::records::NodePayload;
use stellar_project::document_io::component_source::{self, ComponentSource};
use stellar_storage::hash::ContentHash;

/// CLI에 노출하는 component/root의 실제 식별자와 저장된 Frame 크기.
pub struct ComponentInfo {
    /// 전체 component ID.
    pub id: StableId,
    /// DefinitionTree root ID.
    pub root: StableId,
    /// 컴포넌트 이름.
    pub name: String,
    /// root Frame의 논리 픽셀 크기.
    pub size: Size2,
    /// root를 포함한 실제 Frame 수.
    pub node_count: usize,
    /// 원본 경로와 bytes hash.
    pub file: PathBuf,
    /// 다음 쓰기에 사용할 실제 bytes hash.
    pub hash: ContentHash,
}

fn lock(dir: &Path) -> Result<SaveLock, Error> {
    // 라이브러리 디렉터리 자체에 프로젝트 ID가 없으므로 canonical 디렉터리를 잠금 identity로 씁니다.
    SaveLock::try_acquire(
        &default_lock_dir().map_err(ApplyError::Lock)?,
        StableId::NIL,
        dir,
    )
    .map_err(|e| ApplyError::Lock(e).into())
}

fn info(source: &ComponentSource, file: PathBuf, hash: ContentHash) -> ComponentInfo {
    let NodePayload::Frame(frame) = &source.root().payload else {
        unreachable!("validated root Frame")
    };
    ComponentInfo {
        id: source.definition().id,
        root: source.root().id,
        node_count: source.node_count(),
        name: source.definition().display_name().into(),
        size: frame.size,
        file,
        hash,
    }
}

/// 독립 원본 한 개를 새로 만들며 같은 폴더의 중복 component/root ID를 먼저 검사합니다.
pub fn create(
    dir: &Path,
    name: String,
    id: StableId,
    root: StableId,
) -> Result<ComponentInfo, Error> {
    let source = ComponentSource::new(id, name, root)?;
    let _guard = lock(dir)?;
    let (file, hash) = component_source::create(dir, &source)?;
    Ok(info(&source, file, hash))
}

/// 원본 파일만 읽어 ComponentLibrary를 조립하고 ID 순서로 정의를 반환합니다.
pub fn check(dir: &Path) -> Result<Vec<ComponentInfo>, Error> {
    let (library, mut index) = component_source::read_library(dir)?;
    let mut result = Vec::new();
    for entry in library.iter() {
        let definition = entry.definition();
        let root = entry
            .tree()
            .node(definition.root.id())
            .expect("validated tree");
        let NodePayload::Frame(frame) =
            decode_payload(root.kind, &root.payload).map_err(|e| Error::Invalid(e.to_string()))?
        else {
            unreachable!("validated root Frame")
        };
        let (file, hash) = index.remove(&definition.id).expect("loaded source");
        result.push(ComponentInfo {
            id: definition.id,
            root: root.id,
            node_count: entry.tree().len(),
            name: definition.display_name().into(),
            size: frame.size,
            file,
            hash,
        });
    }
    Ok(result)
}

/// 파일을 열 때 관측한 hash가 일치하면 root Frame 크기를 바꿉니다.
pub fn resize(file: &Path, expected: &str, size: Size2) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::resize(file, hash, size)?;
    Ok(info(&source, file.into(), hash))
}

/// 컴포넌트 한 개의 지정 부모에 Free Frame을 추가합니다.
pub fn add_frame(
    file: &Path,
    expected: &str,
    parent: StableId,
    id: StableId,
    name: String,
    size: Size2,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::add_frame(file, hash, parent, id, name, size)?;
    Ok(info(&source, file.into(), hash))
}

fn lock_file(file: &Path, expected: &str) -> Result<(SaveLock, ContentHash), Error> {
    let hash = expected
        .parse()
        .map_err(|_| Error::Invalid("원본 hash 형식".into()))?;
    let dir = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok((lock(dir)?, hash))
}
