//! 한 컴포넌트와 그 정의 tree의 root Frame을 담는 opt-in YAML 원본.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use stellar_core_types::{ids::StableId, order_key::OrderKey, reference::NodeRef, value::Size2};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::frame::{Clipping, FramePayload};
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_storage::{hash::ContentHash, source_file};

use super::text_codec::{field, fields, invalid, mapping, node, parse, parse_node, scalar, text};
use super::text_source::TextSourceError;
use crate::folder_name::{collision_key, validate_folder_name};
use crate::yaml::{load_restricted, loader::MAX_INPUT_BYTES, write_canonical_checked};

const EXT: &str = ".stellar.yaml";
const MAX_COMPONENT_FILES: usize = 4096;

/// 빈 공개 인터페이스와 root Frame으로 제한된 편집 원본.
#[derive(Debug)]
pub struct ComponentSource {
    definition: ComponentDefinition,
    root: NodeRecord,
}

impl ComponentSource {
    /// 새 정의와 독립된 root Frame을 만듭니다. ID는 표시 이름과 독립적입니다.
    pub fn new(id: StableId, name: String, root: StableId) -> Result<Self, TextSourceError> {
        let value = Self {
            definition: ComponentDefinition::new(id, name, NodeRef::new(root)),
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
        value.validate()?;
        Ok(value)
    }

    /// 실제 문서 도메인 정의.
    pub fn definition(&self) -> &ComponentDefinition {
        &self.definition
    }

    /// root Frame 레코드.
    pub fn root(&self) -> &NodeRecord {
        &self.root
    }

    /// root Frame의 크기만 변경합니다.
    fn set_size(&mut self, size: Size2) {
        let NodePayload::Frame(frame) = &mut self.root.payload else {
            unreachable!("validated Frame")
        };
        frame.size = size;
    }

    fn validate(&self) -> Result<(), TextSourceError> {
        if self.definition.id == StableId::NIL
            || self.root.id == StableId::NIL
            || self.definition.id == self.root.id
            || self.root.id != self.definition.root.id()
            || self.root.parent.is_some()
        {
            return Err(invalid(
                "component/root ID 또는 root parent가 잘못되었습니다",
            ));
        }
        let NodePayload::Frame(frame) = &self.root.payload else {
            return Err(TextSourceError::Unsupported(
                "root는 Frame이어야 합니다".into(),
            ));
        };
        if frame.layout_mode.is_some() || matches!(frame.clipping, Clipping::Path(_)) {
            return Err(TextSourceError::Unsupported(
                "외부 layout 또는 Path 참조".into(),
            ));
        }
        self.filename()?;
        Ok(())
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
        let mut tree = DefinitionTree::new(self.definition.reference());
        tree.insert(DefinitionNode {
            id: self.root.id,
            kind: self.root.kind(),
            parent: self.root.parent,
            order_key: self.root.order_key.clone(),
            visible: self.root.visible,
            locked: self.root.locked,
            display_name: self.root.display_name.clone(),
            payload: encode_payload(&self.root.payload),
        })
        .map_err(invalid)?;
        library
            .insert(self.definition.clone(), tree)
            .map_err(invalid)
    }

    fn encode(&self) -> Result<String, TextSourceError> {
        self.validate()?;
        write_canonical_checked(&mapping([
            ("format", scalar("stellar-component")),
            ("schema_version", scalar(1)),
            (
                "component",
                mapping([
                    ("id", scalar(self.definition.id)),
                    ("name", scalar(self.definition.display_name())),
                    ("root", scalar(self.definition.root.id())),
                ]),
            ),
            ("root_node", node(&self.root)?),
        ]))
        .map_err(invalid)
    }

    fn decode(src: &str) -> Result<Self, TextSourceError> {
        let v = load_restricted(src).map_err(invalid)?;
        fields(&v, &["format", "schema_version", "component", "root_node"])?;
        if text(field(&v, "format")?)? != "stellar-component"
            || text(field(&v, "schema_version")?)? != "1"
        {
            return Err(TextSourceError::Unsupported(
                "컴포넌트 원본 형식·버전".into(),
            ));
        }
        let component = field(&v, "component")?;
        fields(component, &["id", "name", "root"])?;
        let value = Self {
            definition: ComponentDefinition::new(
                parse(field(component, "id")?)?,
                text(field(component, "name")?)?.to_owned(),
                NodeRef::new(parse(field(component, "root")?)?),
            ),
            root: parse_node(field(&v, "root_node")?)?,
        };
        value.validate()?;
        Ok(value)
    }
}

/// 정확히 하나의 기존 파일. 파일명과 내용에서 같은 StableId·이름을 확인합니다.
pub fn open(path: &Path) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let bytes = source_file::read(path, MAX_INPUT_BYTES as u64)?;
    let source = ComponentSource::decode(std::str::from_utf8(&bytes).map_err(invalid)?)?;
    if path.file_name() != Some(source.filename()?.as_ref()) {
        return Err(invalid(
            "파일명이 정의의 이름·StableId 접미사와 일치하지 않습니다",
        ));
    }
    Ok((source, ContentHash::compute(&bytes)))
}

/// 기존 원본을 대체하지 않고 지정 폴더에 새 컴포넌트를 기록합니다.
pub fn create(
    dir: &Path,
    source: &ComponentSource,
) -> Result<(PathBuf, ContentHash), TextSourceError> {
    let path = dir.join(source.filename()?);
    let (mut library, files) = read_library(dir)?;
    if files.len() == MAX_COMPONENT_FILES {
        return Err(invalid("컴포넌트 파일 수 상한 초과"));
    }
    let key = collision_key(&source.filename()?);
    if files.values().any(|(p, _)| {
        p.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| collision_key(name) == key)
    }) {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
    }
    source.add_to(&mut library)?;
    let encoded = source.encode()?;
    source_file::write_checked(&path, encoded.as_bytes(), None, MAX_INPUT_BYTES as u64)?;
    Ok((path, ContentHash::compute(encoded.as_bytes())))
}

/// 기존 hash로 단일 파일의 Frame 크기를 조건부 저장합니다.
pub fn resize(
    path: &Path,
    expected: ContentHash,
    size: Size2,
) -> Result<(ComponentSource, ContentHash), TextSourceError> {
    let (mut source, hash) = open(path)?;
    if hash != expected {
        return Err(TextSourceError::Changed);
    }
    if matches!(&source.root.payload, NodePayload::Frame(frame) if frame.size == size) {
        return Ok((source, hash));
    }
    source.set_size(size);
    let encoded = source.encode()?;
    source_file::write_checked(path, encoded.as_bytes(), Some(hash), MAX_INPUT_BYTES as u64)?;
    Ok((source, ContentHash::compute(encoded.as_bytes())))
}

/// 조립할 때 읽은 각 원본 경로와 실제 bytes의 hash. 키는 전체 component ID입니다.
pub type ComponentFiles = BTreeMap<StableId, (PathBuf, ContentHash)>;

/// 같은 폴더의 컴포넌트 정의를 기존 library로 재조립합니다. 참조는 전체 StableId를 사용합니다.
pub fn read_library(dir: &Path) -> Result<(ComponentLibrary, ComponentFiles), TextSourceError> {
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
    let mut lib = ComponentLibrary::new();
    let mut index = BTreeMap::new();
    let mut filenames = BTreeSet::new();
    for path in paths {
        let (source, hash) = open(&path)?;
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
        let text = source.encode().unwrap();
        let reopened = ComponentSource::decode(&text).unwrap();
        assert_eq!(reopened.encode().unwrap(), text);
        let mut before = ComponentLibrary::new();
        let mut after = ComponentLibrary::new();
        source.add_to(&mut before).unwrap();
        reopened.add_to(&mut after).unwrap();
        assert_eq!(before.encode(), after.encode());
    }
}
