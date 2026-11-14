//! semantic snapshot·change canonical 인코딩 (XIV.3.8).
//!
//! diff 결과를 결정론적 bytes로 비교·전달하기 위한 형식입니다(저장 포맷 아님). 모든 정수는
//! little-endian, 길이·개수는 `u32`입니다.
//!
//! | 항목 | 형식 |
//! |---|---|
//! | 속성 경로 | root 16 + key 수 `u32` + key `u32`* + tail tag `u8`(0 없음·1 index·2 key) + (`u32` index \| `u32` 길이 + UTF-8) |
//! | OrderKey | `u32` 길이 + bytes(비어 있거나 `0x00`으로 끝나면 거부) |
//! | 선택 값 | flag `u8`(0/1) + 값 |
//! | 출처 | code `u8` + 문서·컴포넌트·인스턴스 id(종류별) |
//! | 객체 | id 16 + 종류 `u16` + 범위 tag `u8`(+id) + parent(선택 id) + OrderKey(선택) + 속성 수 `u32` + (경로 + 값)* |
//! | 변경 | 종류 code `u8` + 출처 + 본문(객체 \| id + from/to 위치 \| 경로 + 선택 old + 선택 new) |
//!
//! decoder는 신뢰하지 않는 입력을 받습니다: 길이 초과·알 수 없는 tag·비정규 값(비유한 float,
//! 잘못된 OrderKey·TypedValue·UTF-8)·순서 위반·trailing bytes를 구조화 오류로 거부하고
//! panic하지 않습니다. 값에 재귀 구조가 없어 decode는 반복문만 씁니다. 개수 필드로 미리
//! 할당하지 않고 남은 bytes로 상한을 둡니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PathSegment, PropertyKey, PropertyPath};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;

use crate::change::{
    ObjectChange, Position, PositionChange, PropertyChange, Provenance, SemanticChange,
    SemanticChangeKind,
};
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot};
use crate::value::{put_blob, put_len, SemanticValue};

/// canonical bytes 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// 입력이 필드 도중에 끝났습니다.
    Truncated,
    /// 해석 후 남은 bytes가 있습니다.
    TrailingBytes {
        /// 남은 길이.
        remaining: usize,
    },
    /// 형식 식별자(magic)가 다릅니다.
    BadMagic,
    /// 지원하지 않는 형식 version.
    UnsupportedVersion {
        /// 발견된 version.
        version: u16,
    },
    /// 알 수 없는 tag·code.
    UnknownTag {
        /// 필드 이름.
        field: &'static str,
        /// 발견된 값.
        value: u32,
    },
    /// 값이 형식 규칙을 어깁니다(비유한 float, 잘못된 UTF-8·OrderKey·TypedValue 등).
    InvalidValue {
        /// 필드 이름.
        field: &'static str,
    },
    /// canonical 순서(오름차순·중복 없음)를 어깁니다.
    NonCanonicalOrder {
        /// 필드 이름.
        field: &'static str,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("입력이 필드 도중에 끝났습니다"),
            DecodeError::TrailingBytes { remaining } => {
                write!(f, "해석 후 {remaining}바이트가 남았습니다")
            }
            DecodeError::BadMagic => f.write_str("형식 식별자가 다릅니다"),
            DecodeError::UnsupportedVersion { version } => {
                write!(f, "지원하지 않는 형식 version {version}")
            }
            DecodeError::UnknownTag { field, value } => {
                write!(f, "{field}: 알 수 없는 값 {value}")
            }
            DecodeError::InvalidValue { field } => write!(f, "{field}: 형식 규칙 위반"),
            DecodeError::NonCanonicalOrder { field } => {
                write!(f, "{field}: canonical 순서 위반")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// 신뢰하지 않는 bytes 읽기 도구. 모든 읽기는 경계를 검사합니다.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// 입력 처음부터 읽습니다.
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, position: 0 }
    }

    /// 남은 bytes 수.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// 다 읽었는지 확인합니다.
    pub fn finish(&self) -> Result<(), DecodeError> {
        match self.remaining() {
            0 => Ok(()),
            remaining => Err(DecodeError::TrailingBytes { remaining }),
        }
    }

    /// `n` bytes를 읽습니다.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.bytes[self.position..self.position + n];
        self.position += n;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// `u8`.
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    /// `u16` LE.
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    /// `u32` LE.
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// `u64` LE.
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// StableId 16바이트.
    pub fn id(&mut self) -> Result<StableId, DecodeError> {
        Ok(StableId::from_bytes(self.array()?))
    }

    /// 0/1 flag.
    pub fn flag(&mut self, field: &'static str) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(DecodeError::UnknownTag {
                field,
                value: u32::from(other),
            }),
        }
    }

    /// 개수 필드. 항목 하나가 최소 `min_item` bytes이므로 남은 bytes로 상한을 검사합니다.
    pub fn count(&mut self, min_item: usize) -> Result<usize, DecodeError> {
        let count = self.u32()? as usize;
        if count.saturating_mul(min_item.max(1)) > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        Ok(count)
    }

    /// `u32` 길이 + bytes.
    pub fn blob(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// `u32` 길이 + UTF-8.
    pub fn string(&mut self, field: &'static str) -> Result<String, DecodeError> {
        let bytes = self.blob()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::InvalidValue { field })
    }
}

/// 선택 id를 기록합니다.
pub fn put_opt_id(out: &mut Vec<u8>, id: Option<StableId>) {
    match id {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(&id.to_bytes());
        }
    }
}

/// 선택 id를 읽습니다.
pub fn read_opt_id(
    reader: &mut Reader<'_>,
    field: &'static str,
) -> Result<Option<StableId>, DecodeError> {
    if reader.flag(field)? {
        Ok(Some(reader.id()?))
    } else {
        Ok(None)
    }
}

fn put_opt_order_key(out: &mut Vec<u8>, key: Option<&OrderKey>) {
    match key {
        None => out.push(0),
        Some(key) => {
            out.push(1);
            put_blob(out, key.as_bytes());
        }
    }
}

fn read_opt_order_key(reader: &mut Reader<'_>) -> Result<Option<OrderKey>, DecodeError> {
    if !reader.flag("order_key")? {
        return Ok(None);
    }
    let bytes = reader.blob()?.to_vec();
    OrderKey::from_bytes(bytes)
        .map(Some)
        .map_err(|_| DecodeError::InvalidValue { field: "order_key" })
}

/// 속성 경로를 기록합니다(모듈 문서 형식).
pub fn put_path(out: &mut Vec<u8>, path: &PropertyPath) {
    out.extend_from_slice(&path.root().to_bytes());
    put_len(out, path.keys().len());
    for key in path.keys() {
        out.extend_from_slice(&key.0.to_le_bytes());
    }
    match path.tail() {
        None => out.push(0),
        Some(PathSegment::Index(index)) => {
            out.push(1);
            out.extend_from_slice(&index.to_le_bytes());
        }
        Some(PathSegment::Key(key)) => {
            out.push(2);
            put_blob(out, key.as_bytes());
        }
    }
}

/// 속성 경로를 읽습니다.
pub fn read_path(reader: &mut Reader<'_>) -> Result<PropertyPath, DecodeError> {
    let root = reader.id()?;
    let count = reader.count(4)?;
    let mut keys = Vec::new();
    for _ in 0..count {
        keys.push(PropertyKey(reader.u32()?));
    }
    let path = PropertyPath::with_keys(root, keys);
    match reader.u8()? {
        0 => Ok(path),
        1 => Ok(path.with_index(reader.u32()?)),
        2 => Ok(path.with_key_segment(&reader.string("path.tail")?)),
        other => Err(DecodeError::UnknownTag {
            field: "path.tail",
            value: u32::from(other),
        }),
    }
}

/// 값을 읽습니다([`SemanticValue::write_canonical`]의 역).
pub fn read_value(reader: &mut Reader<'_>) -> Result<SemanticValue, DecodeError> {
    let tag = reader.u8()?;
    let value = match tag {
        SemanticValue::TAG_BOOL => SemanticValue::Bool(reader.flag("value.bool")?),
        SemanticValue::TAG_UINT => SemanticValue::Uint(reader.u64()?),
        SemanticValue::TAG_TEXT => SemanticValue::Text(reader.string("value.text")?),
        SemanticValue::TAG_ID => SemanticValue::Id(reader.id()?),
        SemanticValue::TAG_IDS => {
            let count = reader.count(16)?;
            let mut ids = Vec::with_capacity(count);
            for _ in 0..count {
                ids.push(reader.id()?);
            }
            SemanticValue::Ids(ids)
        }
        SemanticValue::TAG_FLOATS => {
            let count = reader.count(4)?;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                let raw: [u8; 4] = reader.array()?;
                let value =
                    FiniteF32::from_le_bytes(raw).map_err(|_| DecodeError::InvalidValue {
                        field: "value.float",
                    })?;
                if value.to_le_bytes() != raw {
                    // -0.0처럼 정규화되는 bit 패턴은 canonical이 아닙니다.
                    return Err(DecodeError::InvalidValue {
                        field: "value.float",
                    });
                }
                values.push(value);
            }
            SemanticValue::Floats(values)
        }
        SemanticValue::TAG_TYPED => {
            let bytes = reader.blob()?;
            let value = TypedValue::from_bytes(bytes).map_err(|_| DecodeError::InvalidValue {
                field: "value.typed",
            })?;
            if value.to_bytes() != bytes {
                return Err(DecodeError::InvalidValue {
                    field: "value.typed",
                });
            }
            SemanticValue::Typed(value)
        }
        SemanticValue::TAG_BYTES => SemanticValue::Bytes(reader.blob()?.to_vec()),
        other => {
            return Err(DecodeError::UnknownTag {
                field: "value.tag",
                value: u32::from(other),
            })
        }
    };
    Ok(value)
}

/// 선택 값을 기록합니다.
pub fn put_opt_value(out: &mut Vec<u8>, value: Option<&SemanticValue>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            value.write_canonical(out);
        }
    }
}

/// 선택 값을 읽습니다.
pub fn read_opt_value(reader: &mut Reader<'_>) -> Result<Option<SemanticValue>, DecodeError> {
    if reader.flag("value.present")? {
        Ok(Some(read_value(reader)?))
    } else {
        Ok(None)
    }
}

fn put_origin(out: &mut Vec<u8>, origin: Origin) {
    match origin {
        Origin::Project => out.push(1),
        Origin::ProjectComponent(component) => {
            out.push(4);
            out.extend_from_slice(&component.to_bytes());
        }
        Origin::Document(document) => {
            out.push(2);
            out.extend_from_slice(&document.to_bytes());
        }
        Origin::Component {
            document,
            component,
        } => {
            out.push(3);
            out.extend_from_slice(&document.to_bytes());
            out.extend_from_slice(&component.to_bytes());
        }
    }
}

fn read_origin(reader: &mut Reader<'_>) -> Result<Origin, DecodeError> {
    match reader.u8()? {
        1 => Ok(Origin::Project),
        4 => Ok(Origin::ProjectComponent(reader.id()?)),
        2 => Ok(Origin::Document(reader.id()?)),
        3 => Ok(Origin::Component {
            document: reader.id()?,
            component: reader.id()?,
        }),
        other => Err(DecodeError::UnknownTag {
            field: "origin",
            value: u32::from(other),
        }),
    }
}

/// 객체를 기록합니다.
pub fn put_object(out: &mut Vec<u8>, object: &SemanticObject) {
    out.extend_from_slice(&object.id().to_bytes());
    out.extend_from_slice(&object.kind().code().to_le_bytes());
    put_origin(out, object.origin());
    put_opt_id(out, object.parent());
    put_opt_order_key(out, object.order_key());
    put_len(out, object.properties().len());
    for (path, value) in object.properties() {
        put_path(out, path);
        value.write_canonical(out);
    }
}

/// 객체를 읽습니다. 속성 경로는 오름차순이고 root가 객체여야 합니다.
pub fn read_object(reader: &mut Reader<'_>) -> Result<SemanticObject, DecodeError> {
    let id = reader.id()?;
    let code = reader.u16()?;
    let kind = ObjectKind::from_code(code).ok_or(DecodeError::UnknownTag {
        field: "object.kind",
        value: u32::from(code),
    })?;
    let origin = read_origin(reader)?;
    let parent = read_opt_id(reader, "object.parent")?;
    let order_key = read_opt_order_key(reader)?;
    let mut object = SemanticObject::new(id, kind, origin).with_position(parent, order_key);
    // 경로 최소 21바이트(root 16 + 개수 4 + tail 1) + 값 최소 2바이트.
    let count = reader.count(23)?;
    let mut previous: Option<PropertyPath> = None;
    for _ in 0..count {
        let path = read_path(reader)?;
        let value = read_value(reader)?;
        if previous.as_ref().is_some_and(|p| *p >= path) {
            return Err(DecodeError::NonCanonicalOrder {
                field: "object.properties",
            });
        }
        previous = Some(path.clone());
        object
            .insert_property(path, value)
            .map_err(|_| DecodeError::InvalidValue {
                field: "object.property_root",
            })?;
    }
    Ok(object)
}

const SNAPSHOT_MAGIC: &[u8; 4] = b"SSNP";
const DIFF_MAGIC: &[u8; 4] = b"SDIF";
const FORMAT_VERSION: u16 = 1;

fn read_header(reader: &mut Reader<'_>, magic: &[u8; 4]) -> Result<(), DecodeError> {
    if reader.take(4)? != magic {
        return Err(DecodeError::BadMagic);
    }
    match reader.u16()? {
        FORMAT_VERSION => Ok(()),
        version => Err(DecodeError::UnsupportedVersion { version }),
    }
}

/// snapshot을 기록합니다(헤더 `SSNP` + version + 객체 수 + 객체 StableId 순).
pub fn put_snapshot(out: &mut Vec<u8>, snapshot: &SemanticSnapshot) {
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_len(out, snapshot.len());
    for object in snapshot.objects() {
        put_object(out, object);
    }
}

/// snapshot을 읽습니다. 객체는 StableId 오름차순·중복 없음이어야 합니다.
pub fn read_snapshot(reader: &mut Reader<'_>) -> Result<SemanticSnapshot, DecodeError> {
    read_header(reader, SNAPSHOT_MAGIC)?;
    // 객체 최소 25바이트(id 16 + 종류 2 + 범위 1 + parent 1 + key 1 + 개수 4).
    let count = reader.count(25)?;
    let mut snapshot = SemanticSnapshot::new();
    let mut previous: Option<StableId> = None;
    for _ in 0..count {
        let object = read_object(reader)?;
        if previous.is_some_and(|p| p >= object.id()) {
            return Err(DecodeError::NonCanonicalOrder {
                field: "snapshot.objects",
            });
        }
        previous = Some(object.id());
        snapshot
            .insert(object)
            .map_err(|_| DecodeError::NonCanonicalOrder {
                field: "snapshot.objects",
            })?;
    }
    Ok(snapshot)
}

/// 출처를 기록합니다.
pub fn put_provenance(out: &mut Vec<u8>, provenance: &Provenance) {
    out.push(provenance.code());
    match provenance {
        Provenance::Project => {}
        Provenance::ProjectComponentDefinition(component) => {
            out.extend_from_slice(&component.to_bytes())
        }
        Provenance::Document { document } => out.extend_from_slice(&document.to_bytes()),
        Provenance::ComponentDefinition {
            document,
            component,
        } => {
            out.extend_from_slice(&document.to_bytes());
            out.extend_from_slice(&component.to_bytes());
        }
        Provenance::InstanceOverride {
            document,
            instance,
            component,
        } => {
            out.extend_from_slice(&document.to_bytes());
            out.extend_from_slice(&instance.to_bytes());
            out.extend_from_slice(&component.to_bytes());
        }
    }
}

/// 출처를 읽습니다.
pub fn read_provenance(reader: &mut Reader<'_>) -> Result<Provenance, DecodeError> {
    match reader.u8()? {
        1 => Ok(Provenance::Project),
        5 => Ok(Provenance::ProjectComponentDefinition(reader.id()?)),
        2 => Ok(Provenance::Document {
            document: reader.id()?,
        }),
        3 => Ok(Provenance::ComponentDefinition {
            document: reader.id()?,
            component: reader.id()?,
        }),
        4 => Ok(Provenance::InstanceOverride {
            document: reader.id()?,
            instance: reader.id()?,
            component: reader.id()?,
        }),
        other => Err(DecodeError::UnknownTag {
            field: "provenance",
            value: u32::from(other),
        }),
    }
}

/// 위치를 기록합니다.
pub fn put_position(out: &mut Vec<u8>, position: &Position) {
    put_opt_id(out, position.parent);
    put_opt_order_key(out, position.order_key.as_ref());
}

/// 위치를 읽습니다.
pub fn read_position(reader: &mut Reader<'_>) -> Result<Position, DecodeError> {
    Ok(Position {
        parent: read_opt_id(reader, "position.parent")?,
        order_key: read_opt_order_key(reader)?,
    })
}

/// 변경 하나를 기록합니다.
pub fn put_change(out: &mut Vec<u8>, change: &SemanticChange) {
    out.push(change.kind().code());
    put_provenance(out, change.provenance());
    match change {
        SemanticChange::AddObject(c)
        | SemanticChange::RemoveObject(c)
        | SemanticChange::AddToken(c)
        | SemanticChange::RemoveToken(c) => put_object(out, &c.object),
        SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
            out.extend_from_slice(&c.object.to_bytes());
            put_position(out, &c.from);
            put_position(out, &c.to);
        }
        SemanticChange::SetProperty(c)
        | SemanticChange::ChangeToken(c)
        | SemanticChange::ComponentInterfaceChange(c)
        | SemanticChange::AssetRefChange(c) => {
            put_path(out, &c.path);
            put_opt_value(out, c.old.as_ref());
            put_opt_value(out, c.new.as_ref());
        }
    }
}

/// 변경 하나의 canonical bytes.
pub fn change_bytes(change: &SemanticChange) -> Vec<u8> {
    let mut out = Vec::new();
    put_change(&mut out, change);
    out
}

/// 변경 하나를 읽습니다.
pub fn read_change(reader: &mut Reader<'_>) -> Result<SemanticChange, DecodeError> {
    let code = reader.u8()?;
    let kind = SemanticChangeKind::from_code(code).ok_or(DecodeError::UnknownTag {
        field: "change.kind",
        value: u32::from(code),
    })?;
    let provenance = read_provenance(reader)?;
    let change = match kind {
        SemanticChangeKind::AddObject
        | SemanticChangeKind::RemoveObject
        | SemanticChangeKind::AddToken
        | SemanticChangeKind::RemoveToken => {
            let change = ObjectChange {
                provenance,
                object: read_object(reader)?,
            };
            match kind {
                SemanticChangeKind::AddObject => SemanticChange::AddObject(change),
                SemanticChangeKind::RemoveObject => SemanticChange::RemoveObject(change),
                SemanticChangeKind::AddToken => SemanticChange::AddToken(change),
                _ => SemanticChange::RemoveToken(change),
            }
        }
        SemanticChangeKind::MoveObject | SemanticChangeKind::ReorderObject => {
            let change = PositionChange {
                provenance,
                object: reader.id()?,
                from: read_position(reader)?,
                to: read_position(reader)?,
            };
            if kind == SemanticChangeKind::MoveObject {
                SemanticChange::MoveObject(change)
            } else {
                SemanticChange::ReorderObject(change)
            }
        }
        SemanticChangeKind::SetProperty
        | SemanticChangeKind::ChangeToken
        | SemanticChangeKind::ComponentInterfaceChange
        | SemanticChangeKind::AssetRefChange => {
            let change = PropertyChange {
                provenance,
                path: read_path(reader)?,
                old: read_opt_value(reader)?,
                new: read_opt_value(reader)?,
            };
            match kind {
                SemanticChangeKind::SetProperty => SemanticChange::SetProperty(change),
                SemanticChangeKind::ChangeToken => SemanticChange::ChangeToken(change),
                SemanticChangeKind::ComponentInterfaceChange => {
                    SemanticChange::ComponentInterfaceChange(change)
                }
                _ => SemanticChange::AssetRefChange(change),
            }
        }
    };
    Ok(change)
}

/// 변경 목록을 기록합니다(개수 `u32` + 변경*).
pub fn put_changes(out: &mut Vec<u8>, changes: &[SemanticChange]) {
    put_len(out, changes.len());
    for change in changes {
        put_change(out, change);
    }
}

/// 변경 목록을 읽습니다.
pub fn read_changes(reader: &mut Reader<'_>) -> Result<Vec<SemanticChange>, DecodeError> {
    // 변경 최소 2바이트(종류 1 + 출처 1) 이상.
    let count = reader.count(2)?;
    let mut changes = Vec::new();
    for _ in 0..count {
        changes.push(read_change(reader)?);
    }
    Ok(changes)
}

/// snapshot canonical bytes.
pub fn snapshot_to_bytes(snapshot: &SemanticSnapshot) -> Vec<u8> {
    let mut out = Vec::new();
    put_snapshot(&mut out, snapshot);
    out
}

/// snapshot canonical bytes를 읽습니다(trailing 거부).
pub fn snapshot_from_bytes(bytes: &[u8]) -> Result<SemanticSnapshot, DecodeError> {
    let mut reader = Reader::new(bytes);
    let snapshot = read_snapshot(&mut reader)?;
    reader.finish()?;
    Ok(snapshot)
}

/// diff canonical bytes(헤더 `SDIF` + version + 변경 목록).
pub(crate) fn diff_to_bytes(changes: &[SemanticChange]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(DIFF_MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_changes(&mut out, changes);
    out
}

/// diff canonical bytes를 읽습니다(trailing 거부, 순서 검사는 호출자).
pub(crate) fn diff_from_bytes(bytes: &[u8]) -> Result<Vec<SemanticChange>, DecodeError> {
    let mut reader = Reader::new(bytes);
    read_header(&mut reader, DIFF_MAGIC)?;
    let changes = read_changes(&mut reader)?;
    reader.finish()?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records};
    use stellar_document::test_support::random_document::random_valid_document;

    #[test]
    fn snapshot_round_trips() {
        for seed in 0..20 {
            let snapshot = build(&random_valid_document(seed).to_records().unwrap());
            let bytes = snapshot_to_bytes(&snapshot);
            assert_eq!(
                snapshot_from_bytes(&bytes).unwrap(),
                snapshot,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn malformed_snapshot_bytes_are_rejected() {
        let bytes = snapshot_to_bytes(&build(&head_records()));
        for cut in 0..bytes.len() {
            assert!(snapshot_from_bytes(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            snapshot_from_bytes(&trailing),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        );
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(snapshot_from_bytes(&magic), Err(DecodeError::BadMagic));
        let mut version = bytes;
        version[4] = 9;
        assert_eq!(
            snapshot_from_bytes(&version),
            Err(DecodeError::UnsupportedVersion { version: 9 })
        );
    }

    #[test]
    fn non_canonical_values_are_rejected() {
        let decode = |bytes: &[u8]| {
            let mut reader = Reader::new(bytes);
            read_value(&mut reader)
        };
        // -0.0 bit 패턴, NaN.
        let negative_zero = [
            &[SemanticValue::TAG_FLOATS, 1, 0, 0, 0][..],
            &(-0.0f32).to_le_bytes(),
        ]
        .concat();
        assert!(decode(&negative_zero).is_err());
        let nan = [
            &[SemanticValue::TAG_FLOATS, 1, 0, 0, 0][..],
            &f32::NAN.to_le_bytes(),
        ]
        .concat();
        assert!(decode(&nan).is_err());
        assert!(decode(&[0xFF]).is_err());
        assert!(decode(&[SemanticValue::TAG_BOOL, 2]).is_err());
        assert!(decode(&[SemanticValue::TAG_TEXT, 1, 0, 0, 0, 0xFF]).is_err());
        // 큰 개수 필드로 과할당하지 않습니다.
        assert_eq!(
            decode(&[SemanticValue::TAG_IDS, 0xFF, 0xFF, 0xFF, 0xFF]),
            Err(DecodeError::Truncated)
        );
    }
}
