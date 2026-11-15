//! 한 컴포넌트 정의와 Free Frame tree를 담는 opt-in YAML 원본.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use stellar_core_types::{
    color::Color,
    ids::{IdRemapTable, StableId},
    order_key::OrderKey,
    reference::{ComponentRef, NodeRef},
    value::Size2,
};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::frame::{Clipping, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_storage::{hash::ContentHash, source_file};

use super::text_codec::{
    field, fields, invalid, mapping, node, parse, parse_node, scalar, text, Grammar,
};
use super::text_source::TextSourceError;
use crate::folder_name::{collision_key, validate_folder_name};
use crate::yaml::{load_restricted, loader::MAX_INPUT_BYTES, write_canonical_checked, YamlValue};

pub use super::text_codec::color_source_value;

pub(crate) const EXT: &str = ".stellar.yaml";
pub(crate) const MAX_COMPONENT_FILES: usize = 4096;
pub(crate) const MAX_PROJECT_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

/// 컴포넌트 조회에서 사용하는 Frame 속성. 읽기 전용 v1·v2 원본도 v3의 한 속성 한 줄 표현으로 제공합니다.
pub fn frame_source_properties(frame: &FramePayload) -> YamlValue {
    super::text_codec::frame(frame)
}

/// 기본 앱이 선택된 원본 한 개에 수행하는 root Frame 편집.
#[derive(Clone, Copy)]
pub enum ComponentRootEdit {
    /// 논리 픽셀 크기.
    Size(Size2),
    /// 원본 색 공간과 비연관 RGBA. None이면 배경 없음.
    Background(Option<Color>),
}

/// 빈 공개 인터페이스와 Free Frame tree의 편집 원본.
#[derive(Debug)]
pub struct ComponentSource {
    definition: ComponentDefinition,
    root: NodeRecord,
    children: BTreeMap<StableId, NodeRecord>,
}

fn check_frame(payload: &NodePayload) -> Result<(), TextSourceError> {
    let NodePayload::Frame(frame) = payload else {
        return Err(TextSourceError::Unsupported(
            "컴포넌트는 Free Frame만 지원합니다".into(),
        ));
    };
    if frame.layout_mode.is_some() || matches!(frame.clipping, Clipping::Path(_)) {
        return Err(TextSourceError::Unsupported(
            "외부 layout 또는 Path 참조".into(),
        ));
    }
    Ok(())
}

impl ComponentSource {
    /// 새 정의와 독립된 root Frame을 만듭니다. ID는 표시 이름과 독립적입니다.
    pub fn new(id: StableId, name: String, root: StableId) -> Result<Self, TextSourceError> {
        let value = Self {
            definition: ComponentDefinition::new(id, name, NodeRef::new(root)),
            children: BTreeMap::new(),
            root: NodeRecord {
                id: root,
                parent: None,
                order_key: OrderKey::between(None, None),
                visible: true,
                locked: false,
                display_name: "Root".into(),
                payload: NodePayload::Frame(FramePayload::default()),
            },
        };
        value.definition_tree()?;
        Ok(value)
    }

    /// 현재 문서의 선택 subtree를 새 ID로 복제합니다. root의 기존 부모만 떼고 로컬 affine은 보존합니다.
    pub fn from_selection(
        model: &DocumentModel,
        view: StableId,
        selected: StableId,
        name: String,
    ) -> Result<Self, TextSourceError> {
        validate_folder_name(&name).map_err(invalid)?;
        let root = model
            .definition()
            .pages
            .iter()
            .flat_map(|p| &p.views)
            .find(|v| v.id == view)
            .ok_or_else(|| invalid("활성 View가 없습니다"))?
            .root_frame;
        let mut cursor = model
            .runtime_id(selected)
            .ok_or_else(|| invalid("선택한 Frame이 없습니다"))?;
        loop {
            let fields = model
                .columns()
                .get(cursor)
                .ok_or_else(|| invalid("선택 경로가 없습니다"))?;
            if fields.stable_id == root {
                break;
            }
            cursor = fields
                .parent
                .ok_or_else(|| invalid("다른 View에 속한 선택입니다"))?;
        }

        // 모든 Frame은 YAML에 적어도 UUID 문자열 하나와 표시 이름을 씁니다.
        // 확실히 파일 상한을 넘는 subtree는 레코드 또는 전체 형제 배열을 복제하기 전에 중단합니다.
        const ID_BYTES: usize = 36;
        let too_large = || invalid("컴포넌트 원본이 파일 크기 상한을 넘습니다");
        let mut remaining = MAX_INPUT_BYTES;
        let mut pending = vec![selected];
        let mut records = Vec::new();
        while let Some(id) = pending.pop() {
            check_frame(
                model
                    .payload(id)
                    .ok_or_else(|| invalid("선택한 Frame이 없습니다"))?,
            )?;
            if model.modifier_stack(id).is_some() {
                return Err(TextSourceError::Unsupported(
                    "컴포넌트 원본은 modifier stack을 지원하지 않습니다".into(),
                ));
            }
            let record = model
                .record(id)
                .ok_or_else(|| invalid("선택한 Frame이 없습니다"))?;
            remaining = remaining
                .checked_sub(ID_BYTES + record.display_name.len())
                .ok_or_else(too_large)?;
            let handle = model
                .runtime_id(id)
                .ok_or_else(|| invalid("선택한 Frame이 없습니다"))?;
            let children = model.columns().child_count(handle).unwrap_or(0);
            if children + pending.len() > remaining / ID_BYTES {
                return Err(too_large());
            }
            pending.extend(model.children_in_order(id));
            records.push(record);
        }
        let ids = crate::new_ids::new_ids_vec(records.len() + 1)?;
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        if unique.len() != ids.len() || ids.iter().any(|id| model.contains_persistent_id(*id)) {
            return Err(invalid("새 컴포넌트 식별자 충돌"));
        }
        let mut remap = IdRemapTable::new();
        for (record, &id) in records.iter().zip(&ids[1..]) {
            remap.insert(record.id, id);
        }
        for record in &mut records {
            record.parent = if record.id == selected {
                None
            } else {
                Some(
                    record
                        .parent
                        .and_then(|p| remap.mapped(p))
                        .ok_or_else(|| invalid("잘못된 subtree 부모"))?,
                )
            };
            record.id = remap.remap(record.id);
        }
        let mut records = records.into_iter();
        let root = records.next().ok_or_else(|| invalid("선택이 비었습니다"))?;
        Ok(Self {
            definition: ComponentDefinition::new(ids[0], name, NodeRef::new(root.id)),
            root,
            children: records.map(|r| (r.id, r)).collect(),
        })
    }

    /// 실제 문서 도메인 정의.
    pub fn definition(&self) -> &ComponentDefinition {
        &self.definition
    }

    /// root Frame 레코드.
    pub fn root(&self) -> &NodeRecord {
        &self.root
    }

    /// 정의 tree에 속한 Frame 수(root 포함).
    pub fn node_count(&self) -> usize {
        self.children.len() + 1
    }

    fn record_mut(&mut self, id: StableId) -> Result<&mut NodeRecord, TextSourceError> {
        if self.root.id == id {
            Ok(&mut self.root)
        } else {
            self.children
                .get_mut(&id)
                .ok_or_else(|| invalid("같은 컴포넌트에 Frame이 필요합니다"))
        }
    }

    fn ensure_unlocked(&self, id: StableId) -> Result<(), TextSourceError> {
        let mut current = Some(id);
        while let Some(id) = current {
            let record = if id == self.root.id {
                &self.root
            } else {
                self.children
                    .get(&id)
                    .ok_or_else(|| invalid("같은 컴포넌트에 Frame이 필요합니다"))?
            };
            if record.locked {
                return Err(invalid("대상 또는 조상 Frame이 잠겨 있습니다"));
            }
            current = record.parent;
        }
        Ok(())
    }

    fn edit_frame(
        &mut self,
        id: StableId,
        edit: ComponentRootEdit,
    ) -> Result<bool, TextSourceError> {
        self.ensure_unlocked(id)?;
        let NodePayload::Frame(frame) = &mut self.record_mut(id)?.payload else {
            unreachable!("validated Frame");
        };
        match edit {
            ComponentRootEdit::Size(size) if size != frame.size => frame.size = size,
            ComponentRootEdit::Background(fill) if fill != frame.background => {
                frame.background = fill
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// 직렬화 순서(StableId)와 부모·자식 순서를 분리합니다. 순환·끊긴 부모도 거부합니다.
    fn definition_tree(&self) -> Result<DefinitionTree, TextSourceError> {
        self.filename()?;
        if self.definition.id == StableId::NIL
            || self.root.id != self.definition.root.id()
            || self.root.parent.is_some()
            || self.children.contains_key(&self.root.id)
        {
            return Err(invalid(
                "component/root ID 또는 root parent가 잘못되었습니다",
            ));
        }
        let mut by_parent: BTreeMap<StableId, Vec<&NodeRecord>> = BTreeMap::new();
        for record in self.children.values() {
            let parent = record
                .parent
                .ok_or_else(|| invalid("자식 Frame의 parent가 필요합니다"))?;
            by_parent.entry(parent).or_default().push(record);
        }
        let mut tree = DefinitionTree::new(self.definition.reference());
        let mut pending = vec![&self.root];
        while let Some(record) = pending.pop() {
            if record.id == StableId::NIL || record.id == self.definition.id {
                return Err(invalid(
                    "component와 Frame에는 서로 다른 StableId가 필요합니다",
                ));
            }
            check_frame(&record.payload)?;
            tree.insert(DefinitionNode {
                id: record.id,
                kind: record.kind(),
                parent: record.parent,
                order_key: record.order_key.clone(),
                visible: record.visible,
                locked: record.locked,
                display_name: record.display_name.clone(),
                payload: encode_payload(&record.payload),
            })
            .map_err(invalid)?;
            if let Some(children) = by_parent.remove(&record.id) {
                pending.extend(children);
            }
        }
        if tree.len() != self.node_count() {
            return Err(invalid("root에 연결되지 않은 Frame 또는 순환 parent입니다"));
        }
        Ok(tree)
    }

    /// 표시 이름과 전체 StableId의 SHA-256 앞 12자리(hex)로 만든 파일명.
    pub fn filename(&self) -> Result<String, TextSourceError> {
        let name = self.definition.display_name();
        validate_folder_name(name).map_err(invalid)?;
        let digest = ContentHash::compute(&self.definition.id.to_bytes()).digest_hex();
        let filename = format!("{name}-{}{EXT}", &digest[..12]);
        if filename.len() > 255 {
            return Err(invalid("UTF-8 파일명이 255 bytes를 넘습니다"));
        }
        Ok(filename)
    }

    fn add_to(&self, library: &mut ComponentLibrary) -> Result<(), TextSourceError> {
        library
            .insert(self.definition.clone(), self.definition_tree()?)
            .map_err(invalid)
    }

    /// 검증한 정의를 유일한 쓰기 버전 v3의 canonical YAML로 직렬화합니다(ADR-0061 결정 4).
    pub fn to_yaml(&self) -> Result<String, TextSourceError> {
        let mut fields = vec![
            (scalar("format"), scalar("stellar-component")),
            (scalar("schema_version"), scalar(3)),
            (
                scalar("component"),
                mapping([
                    ("id", scalar(self.definition.id)),
                    ("name", scalar(self.definition.display_name())),
                    ("root", scalar(self.definition.root.id())),
                ]),
            ),
            (scalar("root_node"), node(&self.root)?),
        ];
        if !self.children.is_empty() {
            fields.push((
                scalar("children"),
                YamlValue::Sequence(self.children.values().map(node).collect::<Result<_, _>>()?),
            ));
        }
        write_canonical_checked(&YamlValue::Mapping(fields)).map_err(invalid)
    }

    fn decode(src: &str) -> Result<Self, TextSourceError> {
        let v = load_restricted(src).map_err(invalid)?;
        if text(field(&v, "format")?)? != "stellar-component" {
            return Err(TextSourceError::Unsupported("컴포넌트 원본 형식".into()));
        }
        // v1·v2는 커밋된 기존 원본의 읽기 전용 문법입니다. 편집하면 v3로 다시 씁니다.
        let version = text(field(&v, "schema_version")?)?;
        let grammar = match version {
            "1" | "2" => Grammar::Legacy,
            "3" => Grammar::Current,
            _ => return Err(TextSourceError::Unsupported("컴포넌트 원본 버전".into())),
        };
        let mut children = BTreeMap::new();
        if version == "2" || (version == "3" && v.get("children").is_some()) {
            fields(
                &v,
                &[
                    "format",
                    "schema_version",
                    "component",
                    "root_node",
                    "children",
                ],
            )?;
            let records = field(&v, "children")?
                .as_sequence()
                .filter(|records| !records.is_empty())
                .ok_or_else(|| invalid("children에 자식 Frame이 필요합니다"))?;
            for record in records {
                let record = parse_node(record, grammar)?;
                if children.insert(record.id, record).is_some() {
                    return Err(invalid("중복 자식 Frame ID"));
                }
            }
        } else {
            fields(&v, &["format", "schema_version", "component", "root_node"])?;
        }
        let component = field(&v, "component")?;
        fields(component, &["id", "name", "root"])?;
        let value = Self {
            definition: ComponentDefinition::new(
                parse(field(component, "id")?)?,
                text(field(component, "name")?)?.to_owned(),
                NodeRef::new(parse(field(component, "root")?)?),
            ),
            root: parse_node(field(&v, "root_node")?, grammar)?,
            children,
        };
        Ok(value)
    }
}

/// 정확히 하나의 기존 파일. 파일명과 내용에서 같은 StableId·이름을 확인합니다.
pub fn open(path: &Path) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let (source, hash) = read_source(path)?;
    source.definition_tree()?;
    Ok((source, hash))
}

fn read_source(path: &Path) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let bytes = source_file::read(path, MAX_INPUT_BYTES as u64)?;
    decode_named(path, &bytes)
}

fn decode_named(
    path: &Path,
    bytes: &[u8],
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let source = ComponentSource::decode(std::str::from_utf8(bytes).map_err(invalid)?)?;
    if path.file_name() != Some(source.filename()?.as_ref()) {
        return Err(invalid(
            "파일명이 정의의 이름·StableId 접미사와 일치하지 않습니다",
        ));
    }
    Ok((source, ContentHash::compute(bytes)))
}

/// 기존 원본을 대체하지 않고 지정 폴더에 새 컴포넌트를 기록합니다.
pub fn create(
    dir: &Path,
    source: &ComponentSource,
) -> Result<(PathBuf, ContentHash), TextSourceError> {
    let (library, files) = read_library(dir)?;
    create_in(dir, source, library, files, None)
}

/// 고정한 프로젝트 폴더 안에서만 전체 원본을 검사하고 새 파일을 작성합니다.
pub fn create_in_project(
    root: &Path,
    dir: &Path,
    source: &ComponentSource,
) -> Result<(PathBuf, ContentHash), TextSourceError> {
    create_in_project_with_budget(root, dir, source, MAX_PROJECT_SOURCE_BYTES)
}

fn create_in_project_with_budget(
    root: &Path,
    dir: &Path,
    source: &ComponentSource,
    limit: u64,
) -> Result<(PathBuf, ContentHash), TextSourceError> {
    let canonical = dir.canonicalize()?;
    if !canonical.starts_with(root)
        || canonical == root
        || root.join("design/components").canonicalize()? != canonical
    {
        return Err(invalid("프로젝트의 컴포넌트 원본 폴더가 바뀌었습니다"));
    }
    let (library, files, remaining) = project_library(root, limit)?;
    create_in(&canonical, source, library, files, Some((root, remaining)))
}

fn create_in(
    dir: &Path,
    source: &ComponentSource,
    mut library: ComponentLibrary,
    files: ComponentFiles,
    project: Option<(&Path, u64)>,
) -> Result<(PathBuf, ContentHash), TextSourceError> {
    let filename = source.filename()?;
    let path = dir.join(&filename);
    if files.len() == MAX_COMPONENT_FILES {
        return Err(invalid("컴포넌트 파일 수 상한 초과"));
    }
    let key = collision_key(&filename);
    if files.values().any(|(p, _)| {
        p.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| collision_key(name) == key)
    }) {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
    }
    source.add_to(&mut library)?;
    let encoded = source.to_yaml()?;
    if let Some((root, remaining)) = project {
        if root.join("design/components").canonicalize()? != dir {
            return Err(TextSourceError::Changed);
        }
        source_file::write_new_resolved(
            root,
            &path,
            encoded.as_bytes(),
            remaining.min(MAX_INPUT_BYTES as u64),
        )?;
    } else {
        source_file::write_checked(&path, encoded.as_bytes(), None, MAX_INPUT_BYTES as u64)?;
    }
    Ok((path, ContentHash::compute(encoded.as_bytes())))
}

/// 같은 라이브러리 내 ID를 확인하고 지정한 부모의 마지막 자식으로 Frame을 추가합니다.
pub fn add_frame(
    path: &Path,
    expected: ContentHash,
    parent: StableId,
    id: StableId,
    name: String,
    size: Size2,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let (mut source, hash) = open(path)?;
    if hash != expected {
        return Err(TextSourceError::Changed);
    }
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (library, _) = read_library(folder)?;
    let entry = library
        .get(source.definition.reference())
        .ok_or_else(|| invalid("컴포넌트 정의 누락"))?;
    let tree = entry.tree();
    if id == StableId::NIL
        || library.owner_of(id).is_some()
        || library.get(ComponentRef::new(id)).is_some()
    {
        return Err(invalid("이미 사용 중이거나 비어 있는 Frame ID"));
    }
    if !tree.contains(parent) {
        return Err(invalid("같은 컴포넌트에 parent Frame이 필요합니다"));
    }
    let last = tree.children(parent).last().and_then(|id| tree.node(*id));
    let record = NodeRecord {
        id,
        parent: Some(parent),
        order_key: OrderKey::between(last.map(|n| &n.order_key), None),
        visible: true,
        locked: false,
        display_name: name,
        payload: NodePayload::Frame(FramePayload {
            size,
            ..FramePayload::default()
        }),
    };
    source.children.insert(id, record);
    source.definition_tree()?;
    save_changed(path, hash, source)
}

fn save_changed(
    path: &Path,
    hash: ContentHash,
    source: ComponentSource,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let encoded = source.to_yaml()?;
    source_file::write_checked(path, encoded.as_bytes(), Some(hash), MAX_INPUT_BYTES as u64)?;
    Ok((source, ContentHash::compute(encoded.as_bytes())))
}

fn open_expected(
    path: &Path,
    expected: ContentHash,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let (source, hash) = open(path)?;
    if hash != expected {
        return Err(TextSourceError::Changed);
    }
    Ok((source, hash))
}

/// 기존 hash로 단일 파일의 Frame 크기를 조건부 저장합니다.
pub fn resize(
    path: &Path,
    expected: ContentHash,
    size: Size2,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    edit_root(path, expected, ComponentRootEdit::Size(size))
}

/// 단일 원본의 루트에 앱과 같은 잠금·무변경 판정을 적용합니다.
pub fn edit_root(
    path: &Path,
    expected: ContentHash,
    edit: ComponentRootEdit,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let (source, hash) = open_expected(path, expected)?;
    let id = source.root.id;
    edit_record(path, source, hash, id, edit)
}

fn edit_record(
    path: &Path,
    mut source: ComponentSource,
    hash: ContentHash,
    id: StableId,
    edit: ComponentRootEdit,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    if !source.edit_frame(id, edit)? {
        return Ok((source, hash));
    }
    save_changed(path, hash, source)
}

/// 조립할 때 읽은 각 원본 경로와 실제 bytes의 hash. 키는 전체 component ID입니다.
pub type ComponentFiles = BTreeMap<StableId, (PathBuf, ContentHash)>;

/// 같은 폴더의 컴포넌트 정의를 기존 library로 재조립합니다. 참조는 전체 StableId를 사용합니다.
pub fn read_library(dir: &Path) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
    assemble_library(dir, read_source)
}

/// 고정 프로젝트의 opt-in 텍스트 라이브러리. 폴더 부재는 빈 집합입니다. 명시적 폴더 편집과
/// 달리 프로젝트 밖 경로는 열지 않으며, 한 요청의 전체 YAML 읽기를 64 MiB로 제한합니다.
pub fn read_project_library(
    root: &Path,
) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
    let (library, files, _) = project_library(root, MAX_PROJECT_SOURCE_BYTES)?;
    Ok((library, files))
}

fn project_library(
    root: &Path,
    limit: u64,
) -> Result<(ComponentLibrary, ComponentFiles, u64), TextSourceError> {
    let Some(paths) = project_source_paths(root)? else {
        return Ok((ComponentLibrary::new(), BTreeMap::new(), limit));
    };
    let mut remaining = limit;
    // 읽기·예산 차감·조립은 정렬된 파일 순서대로 합니다. 앞 파일의 ID 충돌이 뒤 파일의 오류보다 먼저 보고됩니다.
    let (library, files) = assemble(paths.files.into_iter().map(|path| {
        let bytes = read_project_source(&paths.root, &path, &mut remaining)?;
        let (source, hash) = decode_named(&path, &bytes)?;
        Ok((path, source, hash))
    }))?;
    Ok((library, files, remaining))
}

/// 프로젝트 루트 내부 실제 경로에서 읽은 파일 이름과 bytes hash의 결정적 집합 hash.
/// HEAD 비교 페이지에서 YAML을 다시 파싱하지 않고 현재 원본을 재확인합니다.
pub fn project_source_hash(root: &Path) -> Result<ContentHash, TextSourceError> {
    Ok(project_source_index(root)?.hash)
}

/// 실제 경계 안에서 bytes를 읽어 만든 원본 파일 목록. 읽기 요청 하나에만 유효합니다.
pub struct ProjectSourceIndex {
    root: PathBuf,
    paths: BTreeMap<std::ffi::OsString, (PathBuf, ContentHash)>,
    hash: ContentHash,
}

impl ProjectSourceIndex {
    /// 관측한 파일 이름과 bytes 집합의 결정적 hash.
    pub fn hash(&self) -> ContentHash {
        self.hash
    }

    /// 이 목록을 관측한 실제 프로젝트 루트.
    pub fn project_root(&self) -> &Path {
        &self.root
    }

    /// 비교를 위해 관측한 (파일명, bytes hash) 목록.
    pub fn entries(&self) -> &BTreeMap<std::ffi::OsString, (PathBuf, ContentHash)> {
        &self.paths
    }
    /// 앞서 읽은 파일 하나를 같은 경계·상한·hash로 재검증하여 독립된 정의를 조립합니다.
    pub fn library_for(&self, name: &std::ffi::OsStr) -> Result<ComponentLibrary, TextSourceError> {
        let (path, expected) = self
            .paths
            .get(name)
            .ok_or_else(|| invalid("관측한 컴포넌트 원본이 필요합니다"))?;
        let mut remaining = MAX_INPUT_BYTES as u64;
        let bytes = read_project_source(&self.root, path, &mut remaining)?;
        let (source, hash) = decode_named(path, &bytes)?;
        if &hash != expected {
            return Err(TextSourceError::Changed);
        }
        let mut library = ComponentLibrary::new();
        source.add_to(&mut library)?;
        Ok(library)
    }
}

/// 원본 전체의 바이트 상한을 확인하면서 개별 파일 hash도 보존합니다.
pub fn project_source_index(root: &Path) -> Result<ProjectSourceIndex, TextSourceError> {
    let Some(paths) = project_source_paths(root)? else {
        return Ok(ProjectSourceIndex {
            root: root.into(),
            paths: BTreeMap::new(),
            hash: ContentHash::compute(&[]),
        });
    };
    let mut remaining = MAX_PROJECT_SOURCE_BYTES;
    let mut entries = BTreeMap::new();
    for path in paths.files {
        let bytes = read_project_source(&paths.root, &path, &mut remaining)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("컴포넌트 파일 이름이 필요합니다"))?
            .to_owned();
        entries.insert(name, (path, ContentHash::compute(&bytes)));
    }
    let hash = source_set_hash(entries.values().cloned())?;
    Ok(ProjectSourceIndex {
        root: paths.root,
        paths: entries,
        hash,
    })
}

/// 이미 검증한 라이브러리의 원본 파일 집합 hash.
pub fn source_set_hash(
    hashes: impl IntoIterator<Item = (PathBuf, ContentHash)>,
) -> Result<ContentHash, TextSourceError> {
    let mut files: Vec<_> = hashes.into_iter().collect();
    files.sort_unstable_by(|(a, _), (b, _)| a.file_name().cmp(&b.file_name()));
    let mut input = Vec::new();
    for (path, hash) in files {
        let name = path
            .file_name()
            .ok_or_else(|| invalid("컴포넌트 파일 이름이 필요합니다"))?
            .as_encoded_bytes();
        input.extend_from_slice(&(name.len() as u64).to_le_bytes());
        input.extend_from_slice(name);
        input.extend_from_slice(hash.digest());
    }
    Ok(ContentHash::compute(&input))
}

struct ProjectSourcePaths {
    root: PathBuf,
    files: Vec<PathBuf>,
}

fn project_source_paths(root: &Path) -> Result<Option<ProjectSourcePaths>, TextSourceError> {
    use crate::path_resolver::{PathResolver, ResolveError, TargetKind, TargetLocation};
    let resolver = PathResolver::new(root)?;
    let folder = match resolver.resolve("design/components") {
        Ok(target)
            if target.location == TargetLocation::Internal
                && target.kind == TargetKind::Directory =>
        {
            target
        }
        Ok(_) => return Err(invalid("프로젝트 안의 컴포넌트 폴더가 필요합니다")),
        Err(ResolveError::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => return Ok(None),
        Err(_) => return Err(invalid("컴포넌트 폴더를 해석할 수 없습니다")),
    };
    Ok(Some(ProjectSourcePaths {
        root: resolver.canonical_root().to_owned(),
        files: source_paths(&folder.target)?,
    }))
}

fn read_project_source(
    root: &Path,
    path: &Path,
    remaining: &mut u64,
) -> Result<Vec<u8>, TextSourceError> {
    let target = path.canonicalize()?;
    if !target.starts_with(root) {
        return Err(invalid("프로젝트 밖의 컴포넌트 원본입니다"));
    }
    let bytes = source_file::read_resolved(&target, (*remaining).min(MAX_INPUT_BYTES as u64))?;
    *remaining -= bytes.len() as u64;
    Ok(bytes)
}

fn assemble_library(
    dir: &Path,
    mut read: impl FnMut(&Path) -> Result<(ComponentSource, ContentHash), TextSourceError>,
) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
    assemble(source_paths(dir)?.into_iter().map(|path| {
        let (source, hash) = read(&path)?;
        Ok((path, source, hash))
    }))
}

fn source_paths(dir: &Path) -> Result<Vec<PathBuf>, TextSourceError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .as_encoded_bytes()
            .ends_with(EXT.as_bytes())
        {
            if paths.len() == MAX_COMPONENT_FILES {
                return Err(invalid("컴포넌트 파일 수 상한 초과"));
            }
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn library_from_blobs(
    sources: Vec<(PathBuf, Vec<u8>)>,
) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
    assemble(sources.into_iter().map(|(path, bytes)| {
        let (source, hash) = decode_named(&path, &bytes)?;
        Ok((path, source, hash))
    }))
}

fn assemble(
    sources: impl IntoIterator<Item = Result<(PathBuf, ComponentSource, ContentHash), TextSourceError>>,
) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
    let mut lib = ComponentLibrary::new();
    let mut index = BTreeMap::new();
    let mut filenames = BTreeSet::new();
    for item in sources {
        let (path, source, hash) = item?;
        if !filenames.insert(collision_key(&source.filename()?)) {
            return Err(invalid("대소문자 무시 파일명 충돌"));
        }
        source.add_to(&mut lib)?;
        index.insert(source.definition.id, (path, hash));
    }
    Ok((lib, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Transform2D;
    use stellar_document::frame::{Border, CornerRadius};

    #[test]
    fn project_read_preserves_budget_and_first_error() {
        let temp = crate::saved_project::tests::TempDir::new();
        let root = temp.path.canonicalize().unwrap();
        let dir = root.join("design/components");
        fs::create_dir_all(&dir).unwrap();
        let id = |n| StableId::new_v7(1, 0, n);
        let mut length = 0;
        let mut files = Vec::new();
        for n in 0..40 {
            let source =
                ComponentSource::new(id(2 * n + 1), format!("Card {n:02}"), id(2 * n + 2)).unwrap();
            let bytes = source.to_yaml().unwrap();
            let path = dir.join(source.filename().unwrap());
            fs::write(&path, &bytes).unwrap();
            length += bytes.len() as u64;
            files.push(path);
        }
        let (_, index, remaining) = project_library(&root, length).unwrap();
        assert_eq!((index.len(), remaining), (40, 0));
        let result = project_library(&root, length - 1);
        assert!(
            matches!(result, Err(TextSourceError::Io(ref e)) if e.kind() == std::io::ErrorKind::FileTooLarge)
        );

        // 파일 간 ID 충돌이 뒤쪽 파일의 구문 오류·읽기 상한보다 먼저 보고되어야 합니다.
        let duplicate = ComponentSource::new(id(5), "Card 02".into(), id(2)).unwrap();
        fs::write(&files[2], duplicate.to_yaml().unwrap()).unwrap();
        let collision = read_library(&dir).unwrap_err().to_string();
        fs::write(&files[3], b"invalid").unwrap();
        fs::write(&files[4], vec![0; MAX_INPUT_BYTES + 1]).unwrap();
        assert_eq!(
            project_library(&root, MAX_PROJECT_SOURCE_BYTES)
                .unwrap_err()
                .to_string(),
            collision
        );
    }

    #[test]
    fn project_creation_charges_existing_bytes_before_writing() {
        let temp = crate::saved_project::tests::TempDir::new();
        let root = temp.path.canonicalize().unwrap();
        let dir = root.join("design/components");
        std::fs::create_dir_all(&dir).unwrap();
        let make = |n| {
            ComponentSource::new(
                StableId::new_v7(1, 0, n),
                "Card".into(),
                StableId::new_v7(1, 0, n + 1),
            )
            .unwrap()
        };
        let first = make(10);
        let second = make(20);
        let (file, hash) = create_in_project(&root, &dir, &first).unwrap();
        let first_len = std::fs::metadata(&file).unwrap().len();
        let exact = first_len + second.to_yaml().unwrap().len() as u64;
        let next = dir.join(second.filename().unwrap());
        let result = create_in_project_with_budget(&root, &dir, &second, exact - 1);
        assert!(
            matches!(result,Err(TextSourceError::Io(ref io)) if io.kind()==std::io::ErrorKind::FileTooLarge)
        );
        assert!(!next.exists());
        assert_eq!(open(&file).unwrap().1, hash);
        assert_eq!(read_project_library(&root).unwrap().1.len(), 1);
        assert_eq!(
            create_in_project_with_budget(&root, &dir, &second, exact)
                .unwrap()
                .0,
            next
        );
        assert_eq!(read_project_library(&root).unwrap().1.len(), 2);
    }

    #[test]
    fn locked_root_rejects_fill_even_if_unchanged() {
        let temp = crate::saved_project::tests::TempDir::new();
        let root = temp.path.canonicalize().unwrap();
        let dir = root.join("design/components");
        std::fs::create_dir_all(&dir).unwrap();
        let id = |n| StableId::new_v7(1, 0, n);
        let mut component = ComponentSource::new(id(1), "Card".into(), id(2)).unwrap();
        component.root.locked = true;
        let (file, hash) = create_in_project(&root, &dir, &component).unwrap();
        let original = std::fs::read(&file).unwrap();
        let NodePayload::Frame(frame) = &component.root.payload else {
            unreachable!()
        };
        let result = edit_root(&file, hash, ComponentRootEdit::Background(frame.background));
        assert!(matches!(result, Err(TextSourceError::Invalid(_))));
        assert_eq!(std::fs::read(file).unwrap(), original);
    }

    #[test]
    fn frame_and_component_library_bytes_survive_a_source_round_trip() {
        let mut source = ComponentSource::new(
            StableId::new_v7(10, 0, 1),
            "버튼".into(),
            StableId::new_v7(10, 0, 2),
        )
        .unwrap();
        source.root.visible = false;
        source.root.locked = true;
        source.root.display_name = "영역 #1".into();
        let NodePayload::Frame(frame) = &mut source.root.payload else {
            unreachable!()
        };
        frame.size = Size2::new(92.25, 17.75).unwrap();
        frame.transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 2.5, -3.25).unwrap();
        frame.corner_radius = CornerRadius::new(1.0, 2.25, 3.5, 4.75).unwrap();
        frame.clipping = Clipping::Disabled;
        let color = Color::new(ColorSpace::DisplayP3, 0.7, 0.2, 0.3, 0.5).unwrap();
        frame.background = Some(color);
        frame.border = Some(Border::new(color, 1.25).unwrap());
        // root의 색·geometry와 같은 payload를 가진 손자. StableId 정렬에서 부모보다 앞에 둡니다.
        let id = |n| StableId::new_v7(10, 0, n);
        for (child, parent, order, name) in [
            (id(9), source.root.id, 0x40, "그룹"),
            (id(7), source.root.id, 0x80, "다음 형제"),
            (id(3), id(9), 0x80, "손자 #1"),
        ] {
            source.children.insert(
                child,
                NodeRecord {
                    id: child,
                    parent: Some(parent),
                    order_key: OrderKey::from_bytes(vec![order]).unwrap(),
                    display_name: name.into(),
                    ..source.root.clone()
                },
            );
        }
        // 실제 문서에 배치한 뒤 선택을 복제하고 yaml을 재열어 ID·부모 외의 전 필드를 비교합니다.
        use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
        let mut def = DocumentDefinition::new(id(20), "Document".into());
        let mut page = PageDefinition::new(id(21), "Page".into());
        page.add_view(ViewDefinition::new(id(22), "View".into(), id(23)))
            .unwrap();
        def.add_page(page).unwrap();
        let mut model = DocumentModel::new(def);
        model
            .insert_node(NodeRecord {
                id: id(23),
                ..source.root.clone()
            })
            .unwrap();
        model
            .insert_node(NodeRecord {
                parent: Some(id(23)),
                ..source.root.clone()
            })
            .unwrap();
        for child in [id(9), id(3), id(7)] {
            model.insert_node(source.children[&child].clone()).unwrap();
        }
        let before = model.to_records().unwrap();
        let copied =
            ComponentSource::from_selection(&model, id(22), source.root.id, "Copy".into()).unwrap();
        let copied = ComponentSource::decode(&copied.to_yaml().unwrap()).unwrap();
        copied.definition_tree().unwrap();
        let copied_id = |n: StableId| {
            if n == source.root.id {
                return copied.root.id;
            }
            let name = &source.children[&n].display_name;
            copied
                .children
                .values()
                .find(|r| r.display_name == *name)
                .unwrap()
                .id
        };
        assert_eq!(copied.node_count(), source.node_count());
        assert!(!model.contains_persistent_id(copied.definition.id));
        for old in std::iter::once(&source.root).chain(source.children.values()) {
            let new = copied_id(old.id);
            assert!(!model.contains_persistent_id(new));
            let record = copied.children.get(&new).unwrap_or(&copied.root);
            assert_eq!(
                *record,
                NodeRecord {
                    id: new,
                    parent: old.parent.map(copied_id),
                    ..old.clone()
                }
            );
        }
        assert_eq!(model.to_records().unwrap(), before);
        let text = source.to_yaml().unwrap();
        let reopened = ComponentSource::decode(&text).unwrap();
        let disconnected = text.replace(
            &format!("parent: \"{}\"", id(9)),
            &format!("parent: \"{}\"", id(3)),
        );
        assert_ne!(disconnected, text);
        assert!(ComponentSource::decode(&disconnected)
            .unwrap()
            .definition_tree()
            .is_err());
        let mut duplicate_order = ComponentSource::decode(&text).unwrap();
        duplicate_order.children.get_mut(&id(7)).unwrap().order_key =
            OrderKey::from_bytes(vec![0x40]).unwrap();
        assert!(duplicate_order.definition_tree().is_err());
        assert_eq!(reopened.to_yaml().unwrap(), text);
        let mut before = ComponentLibrary::new();
        let mut after = ComponentLibrary::new();
        source.add_to(&mut before).unwrap();
        reopened.add_to(&mut after).unwrap();
        assert_eq!(before.encode(), after.encode());

        // 같은 정의를 참조하는 두 실제 scene 인스턴스가 고유한 가상 주소로 같은 tree를 사용합니다.
        use stellar_document::component::{
            expansion::{ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads},
            instance::ComponentInstancePayload,
        };
        use stellar_document::node_columns::{NodeColumns, NodeFields};
        use stellar_document::node_kind::NodeKind;
        let mut columns = NodeColumns::new();
        let mut payloads = ScenePayloads::new();
        for instance in [id(20), id(21)] {
            columns.insert(NodeFields {
                stable_id: instance,
                kind: NodeKind::ComponentInstance,
                parent: None,
                order_key: OrderKey::between(None, None),
                visible: true,
                locked: false,
                display_name: "Instance".into(),
            });
            payloads.insert(
                instance,
                ComponentInstancePayload::new(source.definition.reference()).encode(),
            );
        }
        let scene = SceneIndex::build(&columns, &payloads);
        for instance in [id(20), id(21)] {
            let expansion = InstanceExpansion::new(&after, &scene, instance).unwrap();
            let at = |node| ExpandedNodeId::virtual_node(vec![instance], node);
            assert_eq!(
                expansion.children(&expansion.root()).unwrap(),
                [at(id(9)), at(id(7))]
            );
            assert_eq!(expansion.children(&at(id(9))).unwrap(), [at(id(3))]);
            let leaf = expansion.node(&at(id(3))).unwrap();
            assert_eq!(leaf.display_name(), "손자 #1");
            assert_eq!(
                leaf.payload(),
                Some(encode_payload(&source.root.payload).as_slice())
            );
        }
    }
}
