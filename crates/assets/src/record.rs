//! `AssetRecord { content_hash, media_type, size, source_metadata }` (XIII.5.1).
//!
//! 한 에셋의 내용 주소(ADR-0004 content hash)와 해석 정보를 담습니다. 같은 content hash를
//! 여러 AssetId가 참조할 수 있으므로(XIII.5.3) AssetId는 record 밖의 key로 둡니다.
//!
//! # binary 형식 (canonical tagged record, R6)
//!
//! `u32` LE field 수 + tag 오름차순 field(`u32` LE tag, `u8` flags, 길이 프리픽스 payload).
//! payload는 각 값의 canonical 인코딩이며, 디코딩 시 payload 끝까지 정확히 소비해야 합니다.
//!
//! | tag | field | mandatory | payload |
//! |---|---|---|---|
//! | 1 | content hash | 예 | 길이 프리픽스 UTF-8 (`sha256:<hex>`, ADR-0004) |
//! | 2 | media type | 예 | 길이 프리픽스 UTF-8 (소문자 `type/subtype`) |
//! | 3 | size | 예 | `u64` LE (byte) |
//! | 4 | source metadata | 아니오 | 아래 nested record. `None`이면 field 생략 |
//!
//! source metadata nested record:
//!
//! | tag | field | mandatory | payload |
//! |---|---|---|---|
//! | 1 | location kind | 예 | `u8` (1 = 프로젝트 내부, 2 = 프로젝트 외부) |
//! | 2 | path | 예 | 길이 프리픽스 UTF-8 — 내부: project root 기준 `/` 상대 경로, 외부: 파일 이름만 |
//! | 3 | origin fingerprint | 예(외부 전용) | 길이 프리픽스 UTF-8 (`sha256:<hex>`). 내부 kind에는 금지 |
//!
//! # 원본 경로 기록 정책(privacy·이식성)
//!
//! record는 Git에 commit되므로 import한 사용자의 **절대 로컬 경로를 그대로 기록하지
//! 않습니다**(아키텍처 29.1 "clone·worktree·프로젝트 이동에서 재현 가능").
//! - 원본이 프로젝트 안에 있으면 project root 기준 상대 경로(`/` 구분)만 기록합니다.
//! - 프로젝트 밖이면 파일 이름과 비가역 출처 지문(domain 구분 prefix + 해석된 절대 경로의
//!   SHA-256)만 기록합니다. 지문에서 경로를 복원할 수 없으며, `Update from Source`
//!   (XIII.5.5)가 호출자가 제시한 경로가 같은 출처인지 확인하는 데만 씁니다.
//!
//! # 엄격한 디코딩
//!
//! 절단·끝 잔여 bytes·tag 중복/역순·정의되지 않은 flags·unknown mandatory tag·필수 field
//! 누락·known tag의 mandatory flag 불일치·비canonical media type은 모두 거부합니다.
//!
//! # 전방 호환(unknown field) 정책 — R6 "보존"
//!
//! - unknown **mandatory** tag: 거부합니다(storage decoder와 같은 `UnknownMandatoryTag`).
//! - unknown **optional** tag: 조용히 버리지 않고 **보존**합니다([`PreservedFields`]).
//!   재기록 시 known field와 함께 tag 오름차순으로 payload를 그대로 다시 씁니다. 따라서
//!   미래 버전이 쓴 record를 이 버전이 decode → encode해도 bytes가 같습니다(결정론 유지).
//!   AssetRecord(최상위)와 SourceMetadata(nested) 모두 같은 정책입니다.
//! - 보존 field는 decode로만 생깁니다. 내용이 바뀌는 연산(import·Update from Source)은
//!   [`AssetRecord::new`]로 새 record를 만들며 이전 record의 보존 field를 이어받지 않습니다
//!   (다른 content에 대한 미래 field 값을 잘못 옮기지 않기 위함).
//! - known field의 정의되지 않은 **값**(예: source location kind 3)은 보존 대상이 아니며
//!   명시 오류로 거부합니다.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError, Field};
use stellar_storage::hash::{ContentHash, ParseContentHashError};

use crate::media_type::{MediaType, MediaTypeError};

/// AssetRecord field tag.
const TAG_CONTENT_HASH: u32 = 1;
const TAG_MEDIA_TYPE: u32 = 2;
const TAG_SIZE: u32 = 3;
const TAG_SOURCE_METADATA: u32 = 4;

/// SourceMetadata field tag.
const TAG_SOURCE_KIND: u32 = 1;
const TAG_SOURCE_PATH: u32 = 2;
const TAG_SOURCE_ORIGIN: u32 = 3;

/// SourceMetadata location kind 값.
const SOURCE_KIND_PROJECT: u8 = 1;
const SOURCE_KIND_EXTERNAL: u8 = 2;

/// 외부 출처 지문의 domain 구분 prefix(다른 용도의 SHA-256 값과 섞이지 않게 합니다).
const EXTERNAL_ORIGIN_DOMAIN: &[u8] = b"stellar.asset.source-origin.v1\0";

/// known field의 (tag, mandatory flag).
const ASSET_RECORD_FIELDS: [(u32, bool); 4] = [
    (TAG_CONTENT_HASH, true),
    (TAG_MEDIA_TYPE, true),
    (TAG_SIZE, true),
    (TAG_SOURCE_METADATA, false),
];
const SOURCE_METADATA_FIELDS: [(u32, bool); 3] = [
    (TAG_SOURCE_KIND, true),
    (TAG_SOURCE_PATH, true),
    (TAG_SOURCE_ORIGIN, true),
];

/// field flags bit0: mandatory (storage canonical wire 형식과 같은 값, R6).
const FIELD_FLAG_MANDATORY: u8 = 0b0000_0001;

/// 보존한 unknown optional field 하나(R6 보존 정책).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreservedField {
    tag: u32,
    payload: Vec<u8>,
}

impl PreservedField {
    /// field tag.
    pub fn tag(&self) -> u32 {
        self.tag
    }

    /// 해석하지 않은 payload bytes(입력 그대로).
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// record가 해석하지 못한 optional field 목록(tag 오름차순, 중복 없음).
///
/// decode로만 채워집니다. 공개 생성자는 빈 목록([`Default`])뿐이므로 known tag와 충돌하는
/// 보존 field를 만들 수 없습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PreservedFields(Vec<PreservedField>);

impl PreservedFields {
    /// 보존 field가 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 보존 field 수.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// tag 오름차순 순회.
    pub fn iter(&self) -> std::slice::Iter<'_, PreservedField> {
        self.0.iter()
    }

    /// 재기록용 optional field.
    fn to_fields(&self) -> impl Iterator<Item = Field> + '_ {
        self.0.iter().map(|f| Field {
            tag: f.tag,
            mandatory: false,
            payload: f.payload.clone(),
        })
    }
}

/// 오류가 발생한 record 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    /// [`AssetRecord`] 최상위 record.
    AssetRecord,
    /// [`SourceMetadata`] nested record.
    SourceMetadata,
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordKind::AssetRecord => f.write_str("AssetRecord"),
            RecordKind::SourceMetadata => f.write_str("SourceMetadata"),
        }
    }
}

/// source metadata 형식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMetadataError {
    /// 경로(또는 파일 이름)가 비어 있습니다.
    EmptyPath,
    /// 경로에 NUL 문자가 있습니다.
    ContainsNul,
    /// 프로젝트 상대 경로가 `/`로 시작합니다(절대 경로 기록 금지).
    AbsolutePath,
    /// 프로젝트 상대 경로에 빈 구성 요소·`.`·`..`가 있습니다.
    InvalidComponent {
        /// 위반 구성 요소.
        component: String,
    },
    /// 경로에 `\`가 있습니다(내부 구분자는 `/`만 사용, 아키텍처 19.2).
    Backslash,
    /// 외부 원본 파일 이름에 구분자가 있거나 `.`·`..`입니다.
    InvalidFileName {
        /// 위반 파일 이름.
        name: String,
    },
    /// 경로를 UTF-8로 표현할 수 없습니다(기록하지 않음).
    NonUtf8Path,
    /// 해석된 경로 입력이 절대 경로가 아닙니다.
    NotAbsolute,
    /// 해석된 경로 입력에 `.`·`..` 구성 요소가 남아 있습니다.
    NotNormalized,
    /// 경로 해석(canonicalize)에 실패했습니다.
    Resolve {
        /// I/O 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for SourceMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceMetadataError::EmptyPath => f.write_str("source 경로가 비어 있습니다"),
            SourceMetadataError::ContainsNul => f.write_str("source 경로에 NUL 문자가 있습니다"),
            SourceMetadataError::AbsolutePath => {
                f.write_str("source 프로젝트 경로는 상대 경로여야 합니다")
            }
            SourceMetadataError::InvalidComponent { component } => write!(
                f,
                "source 프로젝트 경로에 허용되지 않는 구성 요소가 있습니다: {component:?}"
            ),
            SourceMetadataError::Backslash => f.write_str("source 경로 구분자는 '/'만 허용됩니다"),
            SourceMetadataError::InvalidFileName { name } => {
                write!(f, "source 파일 이름이 잘못되었습니다: {name:?}")
            }
            SourceMetadataError::NonUtf8Path => {
                f.write_str("source 경로를 UTF-8로 표현할 수 없습니다")
            }
            SourceMetadataError::NotAbsolute => {
                f.write_str("해석된 source·project 경로는 절대 경로여야 합니다")
            }
            SourceMetadataError::NotNormalized => {
                f.write_str("해석된 source·project 경로에 '.'·'..'가 남아 있습니다")
            }
            SourceMetadataError::Resolve { kind } => {
                write!(f, "source·project 경로를 해석할 수 없습니다: {kind}")
            }
        }
    }
}

impl std::error::Error for SourceMetadataError {}

/// AssetRecord 인코딩·디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRecordError {
    /// canonical 디코딩 실패(절단·잔여 bytes·tag 중복/역순·unknown mandatory tag 등).
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
    /// 필수 field가 없습니다.
    MissingField {
        /// field가 빠진 record.
        record: RecordKind,
        /// 누락된 tag.
        tag: u32,
    },
    /// known tag의 mandatory flag가 형식 정의와 다릅니다(비canonical).
    FlagMismatch {
        /// 위반이 발견된 record.
        record: RecordKind,
        /// 위반 tag.
        tag: u32,
        /// 입력에 기록된 mandatory flag.
        mandatory: bool,
    },
    /// content hash 문자열이 ADR-0004 형식이 아닙니다.
    InvalidContentHash(ParseContentHashError),
    /// media type 형식이 잘못되었습니다.
    InvalidMediaType(MediaTypeError),
    /// media type이 canonical(소문자) 형태가 아닙니다.
    NonCanonicalMediaType {
        /// 입력에 기록된 문자열.
        found: String,
    },
    /// source metadata 값이 잘못되었습니다.
    InvalidSourceMetadata(SourceMetadataError),
    /// source metadata location kind 값이 정의되지 않았습니다.
    InvalidSourceKind {
        /// 입력에 기록된 값.
        value: u8,
    },
    /// 현재 값 조합에서 금지된 field가 있습니다(예: 내부 kind의 origin fingerprint).
    UnexpectedField {
        /// 위반이 발견된 record.
        record: RecordKind,
        /// 위반 tag.
        tag: u32,
    },
}

impl fmt::Display for AssetRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetRecordError::Decode(e) => write!(f, "asset record 디코딩 실패: {e}"),
            AssetRecordError::Encode(e) => write!(f, "asset record 인코딩 실패: {e}"),
            AssetRecordError::MissingField { record, tag } => {
                write!(f, "{record}의 필수 field(tag {tag})가 없습니다")
            }
            AssetRecordError::FlagMismatch {
                record,
                tag,
                mandatory,
            } => write!(
                f,
                "{record} field(tag {tag})의 mandatory flag({mandatory})가 형식 정의와 다릅니다"
            ),
            AssetRecordError::InvalidContentHash(e) => {
                write!(f, "content hash 형식이 잘못되었습니다: {e}")
            }
            AssetRecordError::InvalidMediaType(e) => {
                write!(f, "media type 형식이 잘못되었습니다: {e}")
            }
            AssetRecordError::NonCanonicalMediaType { found } => {
                write!(
                    f,
                    "media type이 canonical(소문자) 형태가 아닙니다: {found:?}"
                )
            }
            AssetRecordError::InvalidSourceMetadata(e) => {
                write!(f, "source metadata가 잘못되었습니다: {e}")
            }
            AssetRecordError::InvalidSourceKind { value } => {
                write!(
                    f,
                    "source metadata location kind 값이 잘못되었습니다: {value}"
                )
            }
            AssetRecordError::UnexpectedField { record, tag } => {
                write!(f, "{record}에 허용되지 않는 field(tag {tag})가 있습니다")
            }
        }
    }
}

impl std::error::Error for AssetRecordError {}

impl From<DecodeError> for AssetRecordError {
    fn from(e: DecodeError) -> AssetRecordError {
        AssetRecordError::Decode(e)
    }
}

impl From<EncodeError> for AssetRecordError {
    fn from(e: EncodeError) -> AssetRecordError {
        AssetRecordError::Encode(e)
    }
}

/// 원본 위치(경로 기록 정책은 모듈 문서 참조).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceLocation {
    /// 프로젝트 안의 원본 — project root 기준 `/` 구분 상대 경로.
    Project {
        /// 상대 경로(예: `images/logo.png`).
        relative_path: String,
    },
    /// 프로젝트 밖의 원본 — 파일 이름과 비가역 출처 지문만 기록합니다.
    External {
        /// 원본 파일 이름(디렉터리 없음).
        file_name: String,
        /// [`external_origin_fingerprint`] 값.
        origin: ContentHash,
    },
}

/// 외부 원본 출처 정보(optional).
///
/// import 당시 원본 위치를 기록할 뿐이며, 외부 파일 변경을 자동 반영하지 않습니다
/// (`Update from Source`는 명시 실행, XIII.5.5). 절대 로컬 경로는 기록하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceMetadata {
    location: SourceLocation,
    preserved: PreservedFields,
}

impl SourceMetadata {
    /// 프로젝트 내부 원본(project root 기준 `/` 상대 경로)으로 만듭니다.
    ///
    /// 빈 경로·NUL·`\`·선행 `/`·빈 구성 요소·`.`·`..`를 거부합니다.
    pub fn project(
        relative_path: impl Into<String>,
    ) -> Result<SourceMetadata, SourceMetadataError> {
        let relative_path = relative_path.into();
        validate_relative_path(&relative_path)?;
        Ok(SourceMetadata {
            location: SourceLocation::Project { relative_path },
            preserved: PreservedFields::default(),
        })
    }

    /// 프로젝트 외부 원본(파일 이름 + 출처 지문)으로 만듭니다.
    pub fn external(
        file_name: impl Into<String>,
        origin: ContentHash,
    ) -> Result<SourceMetadata, SourceMetadataError> {
        let file_name = file_name.into();
        validate_file_name(&file_name)?;
        Ok(SourceMetadata {
            location: SourceLocation::External { file_name, origin },
            preserved: PreservedFields::default(),
        })
    }

    /// 이미 해석된(절대·정규화) project root와 원본 경로로 만듭니다(파일 I/O 없음).
    ///
    /// 원본이 root 아래면 상대 경로를, 아니면 파일 이름과 출처 지문을 기록합니다.
    pub fn from_resolved_paths(
        project_root: &Path,
        source: &Path,
    ) -> Result<SourceMetadata, SourceMetadataError> {
        for path in [project_root, source] {
            if !path.is_absolute() {
                return Err(SourceMetadataError::NotAbsolute);
            }
            if path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
            {
                return Err(SourceMetadataError::NotNormalized);
            }
        }
        match source.strip_prefix(project_root) {
            Ok(relative) => {
                let mut parts: Vec<&str> = Vec::new();
                for component in relative.components() {
                    let Component::Normal(part) = component else {
                        return Err(SourceMetadataError::NotNormalized);
                    };
                    parts.push(part.to_str().ok_or(SourceMetadataError::NonUtf8Path)?);
                }
                SourceMetadata::project(parts.join("/"))
            }
            Err(_) => {
                let name = source.file_name().ok_or(SourceMetadataError::EmptyPath)?;
                let name = name.to_str().ok_or(SourceMetadataError::NonUtf8Path)?;
                SourceMetadata::external(name, external_origin_fingerprint(source))
            }
        }
    }

    /// project root와 원본 경로를 canonicalize(symlink 해석)한 뒤
    /// [`SourceMetadata::from_resolved_paths`]로 만듭니다.
    pub fn from_import_path(
        project_root: &Path,
        source: &Path,
    ) -> Result<SourceMetadata, SourceMetadataError> {
        let resolve = |p: &Path| {
            std::fs::canonicalize(p).map_err(|e| SourceMetadataError::Resolve { kind: e.kind() })
        };
        SourceMetadata::from_resolved_paths(&resolve(project_root)?, &resolve(source)?)
    }

    /// 원본 위치.
    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    /// 원본 파일 이름(표시용). 내부 경로는 마지막 구성 요소입니다.
    pub fn file_name(&self) -> &str {
        match &self.location {
            SourceLocation::Project { relative_path } => relative_path
                .rsplit('/')
                .next()
                .unwrap_or(relative_path.as_str()),
            SourceLocation::External { file_name, .. } => file_name,
        }
    }

    /// 내부 원본이면 project root 기준 실제 경로를 만듭니다. 외부면 `None`.
    pub fn project_path(&self, project_root: &Path) -> Option<PathBuf> {
        match &self.location {
            SourceLocation::Project { relative_path } => {
                let mut path = project_root.to_path_buf();
                for part in relative_path.split('/') {
                    path.push(part);
                }
                Some(path)
            }
            SourceLocation::External { .. } => None,
        }
    }

    /// 해석된 절대 경로가 기록된 외부 출처와 같은지(지문 비교). 내부 원본은 `false`.
    pub fn matches_external_origin(&self, resolved_source: &Path) -> bool {
        match &self.location {
            SourceLocation::External { origin, .. } => {
                *origin == external_origin_fingerprint(resolved_source)
            }
            SourceLocation::Project { .. } => false,
        }
    }

    /// nested record payload bytes.
    fn encode_payload(&self) -> Result<Vec<u8>, AssetRecordError> {
        let (kind, path, origin) = match &self.location {
            SourceLocation::Project { relative_path } => {
                (SOURCE_KIND_PROJECT, relative_path.as_str(), None)
            }
            SourceLocation::External { file_name, origin } => {
                (SOURCE_KIND_EXTERNAL, file_name.as_str(), Some(origin))
            }
        };
        let mut fields = vec![
            Field {
                tag: TAG_SOURCE_KIND,
                mandatory: true,
                payload: vec![kind],
            },
            Field {
                tag: TAG_SOURCE_PATH,
                mandatory: true,
                payload: string_payload(path)?,
            },
        ];
        if let Some(origin) = origin {
            fields.push(Field {
                tag: TAG_SOURCE_ORIGIN,
                mandatory: true,
                payload: string_payload(&origin.to_string())?,
            });
        }
        fields.extend(self.preserved.to_fields());
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(&fields)?;
        Ok(enc.into_bytes())
    }

    /// decode 시 보존한 unknown optional field(R6 보존 정책).
    pub fn preserved_fields(&self) -> &PreservedFields {
        &self.preserved
    }

    /// nested record payload를 끝까지 엄격히 디코딩합니다.
    fn decode_payload(payload: &[u8]) -> Result<SourceMetadata, AssetRecordError> {
        let mut dec = CanonicalDecoder::new(payload);
        let (fields, preserved) = take_record_fields(
            &mut dec,
            RecordKind::SourceMetadata,
            &SOURCE_METADATA_FIELDS,
        )?;
        dec.expect_end()?;
        let mut source = SourceMetadata::from_known_fields(&fields)?;
        source.preserved = preserved;
        Ok(source)
    }

    /// known field 값을 해석·검증합니다.
    fn from_known_fields(fields: &[Field]) -> Result<SourceMetadata, AssetRecordError> {
        let kind = RecordKind::SourceMetadata;

        let mut kind_dec = CanonicalDecoder::new(required_payload(fields, kind, TAG_SOURCE_KIND)?);
        let location_kind = kind_dec.take_u8()?;
        kind_dec.expect_end()?;
        let path = take_string_payload(required_payload(fields, kind, TAG_SOURCE_PATH)?)?;
        let origin = find_payload(fields, TAG_SOURCE_ORIGIN);

        match location_kind {
            SOURCE_KIND_PROJECT => {
                if origin.is_some() {
                    return Err(AssetRecordError::UnexpectedField {
                        record: kind,
                        tag: TAG_SOURCE_ORIGIN,
                    });
                }
                SourceMetadata::project(path).map_err(AssetRecordError::InvalidSourceMetadata)
            }
            SOURCE_KIND_EXTERNAL => {
                let origin = origin.ok_or(AssetRecordError::MissingField {
                    record: kind,
                    tag: TAG_SOURCE_ORIGIN,
                })?;
                let origin: ContentHash = take_string_payload(origin)?
                    .parse()
                    .map_err(AssetRecordError::InvalidContentHash)?;
                SourceMetadata::external(path, origin)
                    .map_err(AssetRecordError::InvalidSourceMetadata)
            }
            value => Err(AssetRecordError::InvalidSourceKind { value }),
        }
    }
}

/// 해석된 절대 원본 경로의 비가역 출처 지문.
///
/// `SHA-256(domain prefix ‖ 경로의 OS encoded bytes)`입니다. 경로를 복원할 수 없지만,
/// 후보 경로를 알면 같은지 확인할 수 있습니다(같은 출처 판정 전용 — 비밀 값이 아닙니다).
pub fn external_origin_fingerprint(resolved_source: &Path) -> ContentHash {
    let mut input = EXTERNAL_ORIGIN_DOMAIN.to_vec();
    input.extend_from_slice(resolved_source.as_os_str().as_encoded_bytes());
    ContentHash::compute(&input)
}

fn check_common(text: &str) -> Result<(), SourceMetadataError> {
    if text.is_empty() {
        return Err(SourceMetadataError::EmptyPath);
    }
    if text.contains('\0') {
        return Err(SourceMetadataError::ContainsNul);
    }
    if text.contains('\\') {
        return Err(SourceMetadataError::Backslash);
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), SourceMetadataError> {
    check_common(path)?;
    if path.starts_with('/') {
        return Err(SourceMetadataError::AbsolutePath);
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(SourceMetadataError::InvalidComponent {
                component: component.to_string(),
            });
        }
    }
    Ok(())
}

fn validate_file_name(name: &str) -> Result<(), SourceMetadataError> {
    check_common(name)?;
    if name.contains('/') || name == "." || name == ".." {
        return Err(SourceMetadataError::InvalidFileName {
            name: name.to_string(),
        });
    }
    Ok(())
}

/// 에셋 record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetRecord {
    /// 에셋 content hash (ADR-0004). `assets/` 저장 경로의 주소입니다.
    pub content_hash: ContentHash,
    /// media type.
    pub media_type: MediaType,
    /// content 크기(byte).
    pub size: u64,
    /// 외부 원본 출처 정보(optional).
    pub source_metadata: Option<SourceMetadata>,
    /// decode 시 보존한 unknown optional field(R6 보존 정책, decode로만 채워짐).
    preserved: PreservedFields,
}

impl AssetRecord {
    /// 보존 field 없이 새 record를 만듭니다(import·Update from Source 등 새 content).
    pub fn new(
        content_hash: ContentHash,
        media_type: MediaType,
        size: u64,
        source_metadata: Option<SourceMetadata>,
    ) -> AssetRecord {
        AssetRecord {
            content_hash,
            media_type,
            size,
            source_metadata,
            preserved: PreservedFields::default(),
        }
    }

    /// decode 시 보존한 unknown optional field(R6 보존 정책).
    pub fn preserved_fields(&self) -> &PreservedFields {
        &self.preserved
    }

    /// encoder 현재 위치에 record를 canonical 인코딩합니다.
    ///
    /// 보존 field는 known field와 함께 tag 오름차순으로 payload 그대로 다시 씁니다.
    pub fn encode_into(&self, enc: &mut CanonicalEncoder) -> Result<(), AssetRecordError> {
        let mut size = CanonicalEncoder::new();
        size.put_u64(self.size);
        let mut fields = vec![
            Field {
                tag: TAG_CONTENT_HASH,
                mandatory: true,
                payload: string_payload(&self.content_hash.to_string())?,
            },
            Field {
                tag: TAG_MEDIA_TYPE,
                mandatory: true,
                payload: string_payload(self.media_type.as_str())?,
            },
            Field {
                tag: TAG_SIZE,
                mandatory: true,
                payload: size.into_bytes(),
            },
        ];
        if let Some(source) = &self.source_metadata {
            fields.push(Field {
                tag: TAG_SOURCE_METADATA,
                mandatory: false,
                payload: source.encode_payload()?,
            });
        }
        fields.extend(self.preserved.to_fields());
        enc.put_fields(&fields)?;
        Ok(())
    }

    /// 단독 bytes로 인코딩합니다. 같은 record는 항상 같은 bytes를 만듭니다.
    pub fn encode(&self) -> Result<Vec<u8>, AssetRecordError> {
        let mut enc = CanonicalEncoder::new();
        self.encode_into(&mut enc)?;
        Ok(enc.into_bytes())
    }

    /// decoder 현재 위치에서 record 하나를 엄격히 디코딩합니다.
    ///
    /// record 뒤에 다른 값이 이어질 수 있으므로 끝 검사는 호출자가 합니다
    /// (단독 bytes는 [`AssetRecord::decode`]).
    pub fn decode_from(dec: &mut CanonicalDecoder<'_>) -> Result<AssetRecord, AssetRecordError> {
        let kind = RecordKind::AssetRecord;
        let (fields, preserved) = take_record_fields(dec, kind, &ASSET_RECORD_FIELDS)?;

        let hash_text = take_string_payload(required_payload(&fields, kind, TAG_CONTENT_HASH)?)?;
        let content_hash: ContentHash = hash_text
            .parse()
            .map_err(AssetRecordError::InvalidContentHash)?;

        let media_text = take_string_payload(required_payload(&fields, kind, TAG_MEDIA_TYPE)?)?;
        let media_type = MediaType::new(media_text).map_err(AssetRecordError::InvalidMediaType)?;
        if media_type.as_str() != media_text {
            return Err(AssetRecordError::NonCanonicalMediaType {
                found: media_text.to_string(),
            });
        }

        let mut size_dec = CanonicalDecoder::new(required_payload(&fields, kind, TAG_SIZE)?);
        let size = size_dec.take_u64()?;
        size_dec.expect_end()?;

        let source_metadata = match find_payload(&fields, TAG_SOURCE_METADATA) {
            Some(payload) => Some(SourceMetadata::decode_payload(payload)?),
            None => None,
        };

        Ok(AssetRecord {
            content_hash,
            media_type,
            size,
            source_metadata,
            preserved,
        })
    }

    /// 단독 bytes에서 record를 디코딩합니다. 끝에 남은 bytes가 있으면 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<AssetRecord, AssetRecordError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let record = AssetRecord::decode_from(&mut dec)?;
        dec.expect_end()?;
        Ok(record)
    }
}

/// 문자열 값의 canonical payload(`u32` LE 길이 프리픽스 + UTF-8).
fn string_payload(value: &str) -> Result<Vec<u8>, EncodeError> {
    let mut enc = CanonicalEncoder::new();
    enc.put_string(value)?;
    Ok(enc.into_bytes())
}

/// 문자열 payload를 끝까지 정확히 읽습니다.
fn take_string_payload(payload: &[u8]) -> Result<&str, DecodeError> {
    let mut dec = CanonicalDecoder::new(payload);
    let value = dec.take_string()?;
    dec.expect_end()?;
    Ok(value)
}

/// record를 읽어 known field와 보존할 unknown optional field로 나눕니다(R6 보존 정책).
///
/// wire 형식·구조 검사는 storage [`CanonicalDecoder::take_fields`]와 같습니다(같은 순서로
/// `InvalidFlags`·`DuplicateTag`·`MisorderedTag`·`UnknownMandatoryTag`를 냅니다). storage
/// decoder는 unknown optional field를 버리므로 보존을 위해 같은 규칙을 여기서 읽습니다
/// (`record_reader_agrees_with_storage_take_fields` 테스트로 일치를 고정). 구조 검사를
/// 통과한 뒤 known tag의 mandatory flag가 형식 정의와 같은지 검사합니다.
fn take_record_fields(
    dec: &mut CanonicalDecoder<'_>,
    record: RecordKind,
    specs: &[(u32, bool)],
) -> Result<(Vec<Field>, PreservedFields), AssetRecordError> {
    let count = dec.take_u32()?;
    let mut known: Vec<Field> = Vec::new();
    let mut preserved: Vec<PreservedField> = Vec::new();
    let mut last_tag: Option<u32> = None;
    for _ in 0..count {
        let tag = dec.take_u32()?;
        let flags = dec.take_u8()?;
        if flags & !FIELD_FLAG_MANDATORY != 0 {
            return Err(DecodeError::InvalidFlags { value: flags }.into());
        }
        let mandatory = flags & FIELD_FLAG_MANDATORY != 0;
        let payload = dec.take_bytes()?.to_vec();
        if let Some(last) = last_tag {
            match tag.cmp(&last) {
                std::cmp::Ordering::Equal => return Err(DecodeError::DuplicateTag { tag }.into()),
                std::cmp::Ordering::Less => return Err(DecodeError::MisorderedTag { tag }.into()),
                std::cmp::Ordering::Greater => {}
            }
        }
        last_tag = Some(tag);
        if specs.iter().any(|(known_tag, _)| *known_tag == tag) {
            known.push(Field {
                tag,
                mandatory,
                payload,
            });
        } else if mandatory {
            return Err(DecodeError::UnknownMandatoryTag { tag }.into());
        } else {
            preserved.push(PreservedField { tag, payload });
        }
    }
    for field in &known {
        let expected = specs
            .iter()
            .find(|(tag, _)| *tag == field.tag)
            .map(|(_, mandatory)| *mandatory);
        if expected != Some(field.mandatory) {
            return Err(AssetRecordError::FlagMismatch {
                record,
                tag: field.tag,
                mandatory: field.mandatory,
            });
        }
    }
    Ok((known, PreservedFields(preserved)))
}

fn find_payload(fields: &[Field], tag: u32) -> Option<&[u8]> {
    fields
        .iter()
        .find(|f| f.tag == tag)
        .map(|f| f.payload.as_slice())
}

fn required_payload(
    fields: &[Field],
    record: RecordKind,
    tag: u32,
) -> Result<&[u8], AssetRecordError> {
    find_payload(fields, tag).ok_or(AssetRecordError::MissingField { record, tag })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_source(path: &str) -> SourceMetadata {
        SourceMetadata::project(path).unwrap()
    }

    fn record(source: Option<&str>) -> AssetRecord {
        AssetRecord::new(
            ContentHash::compute(b"asset payload"),
            MediaType::new("image/png").unwrap(),
            1234,
            source.map(project_source),
        )
    }

    fn field(tag: u32, mandatory: bool, payload: Vec<u8>) -> Field {
        Field {
            tag,
            mandatory,
            payload,
        }
    }

    fn u64_payload(value: u64) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_u64(value);
        enc.into_bytes()
    }

    /// 정상 record의 field 목록(테스트에서 일부를 바꿔 손상 입력을 만듭니다).
    fn base_fields() -> Vec<Field> {
        vec![
            field(
                TAG_CONTENT_HASH,
                true,
                string_payload(&ContentHash::compute(b"asset payload").to_string()).unwrap(),
            ),
            field(TAG_MEDIA_TYPE, true, string_payload("image/png").unwrap()),
            field(TAG_SIZE, true, u64_payload(1234)),
        ]
    }

    fn encode_fields(fields: &[Field]) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(fields).unwrap();
        enc.into_bytes()
    }

    /// 정렬·중복 검사 없이 field를 입력 순서 그대로 기록합니다(손상 입력 생성용).
    fn raw_record(fields: &[(u32, u8, Vec<u8>)]) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_u32(fields.len() as u32);
        for (tag, flags, payload) in fields {
            enc.put_u32(*tag);
            enc.put_u8(*flags);
            enc.put_bytes(payload).unwrap();
        }
        enc.into_bytes()
    }

    /// source metadata nested payload를 field 목록으로 직접 만듭니다.
    fn source_payload(fields: &[Field]) -> Vec<u8> {
        encode_fields(fields)
    }

    fn decode_with_source(source: Vec<u8>) -> Result<AssetRecord, AssetRecordError> {
        let mut fields = base_fields();
        fields.push(field(TAG_SOURCE_METADATA, false, source));
        AssetRecord::decode(&encode_fields(&fields))
    }

    #[test]
    fn round_trip_without_source_metadata() {
        let original = record(None);
        let bytes = original.encode().unwrap();
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.encode().unwrap(),
            bytes,
            "canonical 재기록 동일 bytes"
        );
    }

    #[test]
    fn round_trip_with_project_source_metadata() {
        let original = record(Some("에셋/로고 최종.png"));
        let bytes = original.encode().unwrap();
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        let source = decoded.source_metadata.as_ref().unwrap();
        assert_eq!(
            source.location(),
            &SourceLocation::Project {
                relative_path: "에셋/로고 최종.png".to_string()
            }
        );
        assert_eq!(source.file_name(), "로고 최종.png");
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn round_trip_with_external_source_metadata() {
        let origin = external_origin_fingerprint(Path::new("/Users/someone/Desktop/logo.png"));
        let mut original = record(None);
        original.source_metadata = Some(SourceMetadata::external("logo.png", origin).unwrap());
        let bytes = original.encode().unwrap();
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.source_metadata.unwrap().file_name(), "logo.png");
        assert_eq!(original.encode().unwrap(), bytes);
    }

    #[test]
    fn size_boundaries_round_trip() {
        for size in [0, 1, u32::MAX as u64 + 1, u64::MAX] {
            let mut original = record(None);
            original.size = size;
            let decoded = AssetRecord::decode(&original.encode().unwrap()).unwrap();
            assert_eq!(decoded.size, size);
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn records_in_sequence_round_trip() {
        let records: Vec<AssetRecord> = (0..5u64)
            .map(|i| {
                AssetRecord::new(
                    ContentHash::compute(format!("asset-{i}").as_bytes()),
                    MediaType::new("font/woff2").unwrap(),
                    i * 1000,
                    (i % 2 == 0)
                        .then(|| SourceMetadata::project(format!("fonts/{i}.woff2")).unwrap()),
                )
            })
            .collect();
        let mut enc = CanonicalEncoder::new();
        for r in &records {
            r.encode_into(&mut enc).unwrap();
        }
        let bytes = enc.into_bytes();
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded: Vec<AssetRecord> = (0..5)
            .map(|_| AssetRecord::decode_from(&mut dec).unwrap())
            .collect();
        dec.expect_end().unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn encoding_matches_documented_layout() {
        // golden: tag 오름차순 tagged record, 정수는 LE, payload는 값의 canonical 인코딩.
        let hash = ContentHash::compute(b"asset payload").to_string();
        assert_eq!(hash.len(), 71);
        let mut expected: Vec<u8> = vec![0x03, 0x00, 0x00, 0x00]; // field 수 3
        expected.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]); // tag 1, mandatory
        expected.extend_from_slice(&[0x4b, 0x00, 0x00, 0x00]); // payload 75 bytes
        expected.extend_from_slice(&[0x47, 0x00, 0x00, 0x00]); // 문자열 71 bytes
        expected.extend_from_slice(hash.as_bytes());
        expected.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x01]); // tag 2, mandatory
        expected.extend_from_slice(&[0x0d, 0x00, 0x00, 0x00]); // payload 13 bytes
        expected.extend_from_slice(&[0x09, 0x00, 0x00, 0x00]); // 문자열 9 bytes
        expected.extend_from_slice(b"image/png");
        expected.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x01]); // tag 3, mandatory
        expected.extend_from_slice(&[0x08, 0x00, 0x00, 0x00]); // payload 8 bytes
        expected.extend_from_slice(&[0xd2, 0x04, 0, 0, 0, 0, 0, 0]); // 1234 LE
        assert_eq!(record(None).encode().unwrap(), expected);

        // source metadata(내부 경로)는 optional(flags 0) tag 4로 끝에 붙습니다.
        let with_source = record(Some("a.png")).encode().unwrap();
        let mut expected_source = expected.clone();
        expected_source[0] = 0x04; // field 수 4
        expected_source.extend_from_slice(&[0x04, 0x00, 0x00, 0x00, 0x00]); // tag 4, optional
        expected_source.extend_from_slice(&[0x20, 0x00, 0x00, 0x00]); // payload 32 bytes
        expected_source.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]); // nested field 수 2
        expected_source.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]); // tag 1, mandatory
        expected_source.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]); // payload 1 byte: 내부
        expected_source.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x01]); // tag 2, mandatory
        expected_source.extend_from_slice(&[0x09, 0x00, 0x00, 0x00]); // payload 9 bytes
        expected_source.extend_from_slice(&[0x05, 0x00, 0x00, 0x00]); // 문자열 5 bytes
        expected_source.extend_from_slice(b"a.png");
        assert_eq!(with_source, expected_source);

        // 외부 원본: kind 2 + 파일 이름 + tag 3 지문(71자 문자열).
        let origin = external_origin_fingerprint(Path::new("/elsewhere/a.png"));
        let mut external = record(None);
        external.source_metadata = Some(SourceMetadata::external("a.png", origin).unwrap());
        let bytes = external.encode().unwrap();
        let mut expected_external = expected.clone();
        expected_external[0] = 0x04;
        expected_external.extend_from_slice(&[0x04, 0x00, 0x00, 0x00, 0x00]);
        expected_external.extend_from_slice(&[0x74, 0x00, 0x00, 0x00]); // payload 116 bytes
        expected_external.extend_from_slice(&[0x03, 0x00, 0x00, 0x00]); // nested field 수 3
        expected_external.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]);
        expected_external.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x02]); // kind 2: 외부
        expected_external.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x01]);
        expected_external.extend_from_slice(&[0x09, 0x00, 0x00, 0x00]);
        expected_external.extend_from_slice(&[0x05, 0x00, 0x00, 0x00]);
        expected_external.extend_from_slice(b"a.png");
        expected_external.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x01]); // tag 3
        expected_external.extend_from_slice(&[0x4b, 0x00, 0x00, 0x00]); // payload 75 bytes
        expected_external.extend_from_slice(&[0x47, 0x00, 0x00, 0x00]); // 문자열 71 bytes
        expected_external.extend_from_slice(origin.to_string().as_bytes());
        assert_eq!(bytes, expected_external);
    }

    #[test]
    fn truncated_input_is_rejected_at_every_length() {
        let bytes = record(Some("src/logo.png")).encode().unwrap();
        for cut in 0..bytes.len() {
            let err = AssetRecord::decode(&bytes[..cut]).unwrap_err();
            assert!(
                matches!(err, AssetRecordError::Decode(_)),
                "절단 길이 {cut}: {err:?}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = record(None).encode().unwrap();
        bytes.push(0x00);
        assert_eq!(
            AssetRecord::decode(&bytes).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn unknown_mandatory_tag_is_rejected() {
        let mut fields = base_fields();
        fields.push(field(9, true, vec![0xaa]));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnknownMandatoryTag { tag: 9 })
        );
    }

    #[test]
    fn unknown_optional_tags_are_preserved_and_reemitted() {
        // 미래 버전이 known field 앞·사이·뒤에 optional field를 추가한 record.
        let mut fields = base_fields();
        fields.push(field(0, false, vec![0x01, 0x02]));
        fields.push(field(
            TAG_SOURCE_METADATA,
            false,
            project_source("a.png").encode_payload().unwrap(),
        ));
        fields.push(field(5, false, Vec::new()));
        fields.push(field(9, false, vec![0xaa]));
        let bytes = encode_fields(&fields);

        let decoded = AssetRecord::decode(&bytes).unwrap();
        // known field 해석은 보존 field와 무관합니다.
        assert_eq!(decoded.content_hash, record(None).content_hash);
        assert_eq!(decoded.size, 1234);
        assert_eq!(decoded.source_metadata, Some(project_source("a.png")));
        // 조용히 버리지 않습니다: tag·payload를 순서대로 보존합니다.
        let preserved: Vec<(u32, Vec<u8>)> = decoded
            .preserved_fields()
            .iter()
            .map(|f| (f.tag(), f.payload().to_vec()))
            .collect();
        assert_eq!(
            preserved,
            vec![(0, vec![0x01, 0x02]), (5, Vec::new()), (9, vec![0xaa])]
        );
        assert_eq!(decoded.preserved_fields().len(), 3);
        // 재기록은 입력과 같은 bytes(결정론), 다시 decode해도 같은 값입니다.
        let reencoded = decoded.encode().unwrap();
        assert_eq!(reencoded, bytes);
        assert_eq!(AssetRecord::decode(&reencoded).unwrap(), decoded);
        // 보존 field가 있는 record는 같은 known 값의 새 record와 구분됩니다.
        assert_ne!(decoded, record(Some("a.png")));
        assert!(record(Some("a.png")).preserved_fields().is_empty());
    }

    #[test]
    fn nested_unknown_optional_tags_are_preserved() {
        let source = source_payload(&[
            field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_PROJECT]),
            field(TAG_SOURCE_PATH, true, string_payload("a.png").unwrap()),
            field(7, false, vec![0x07]),
        ]);
        let mut fields = base_fields();
        fields.push(field(TAG_SOURCE_METADATA, false, source));
        let bytes = encode_fields(&fields);

        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert!(decoded.preserved_fields().is_empty());
        let nested = decoded.source_metadata.as_ref().unwrap();
        assert_eq!(nested.location(), project_source("a.png").location());
        let tags: Vec<u32> = nested.preserved_fields().iter().map(|f| f.tag()).collect();
        assert_eq!(tags, vec![7]);
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn new_records_do_not_inherit_preserved_fields() {
        let mut fields = base_fields();
        fields.push(field(9, false, vec![0xaa]));
        let decoded = AssetRecord::decode(&encode_fields(&fields)).unwrap();
        // 새 content로 다시 만든 record(import·Update from Source 경로)는 보존 field가 없습니다.
        let fresh = AssetRecord::new(
            ContentHash::compute(b"new content"),
            decoded.media_type.clone(),
            11,
            decoded.source_metadata.clone(),
        );
        assert!(fresh.preserved_fields().is_empty());
        assert_eq!(fresh.encode().unwrap()[0], 0x03, "known field 3개만 기록");
    }

    #[test]
    fn record_reader_agrees_with_storage_take_fields() {
        // 보존 reader는 storage decoder와 같은 구조 규칙·오류를 냅니다. known field 결과도 같습니다.
        let fields = base_fields();
        let entry = |i: usize, flags: u8| (fields[i].tag, flags, fields[i].payload.clone());
        let mut inputs = vec![
            encode_fields(&fields),
            raw_record(&[entry(0, 1), entry(0, 1), entry(1, 1), entry(2, 1)]),
            raw_record(&[entry(1, 1), entry(0, 1), entry(2, 1)]),
            raw_record(&[entry(0, 0x80), entry(1, 1), entry(2, 1)]),
            raw_record(&[entry(0, 1), (8, 1, vec![]), entry(1, 1)]),
            raw_record(&[(0, 0, vec![1]), entry(0, 1), entry(1, 1), (6, 0, vec![])]),
        ];
        let full = encode_fields(&fields);
        inputs.extend((0..full.len()).map(|cut| full[..cut].to_vec()));

        let tags: Vec<u32> = ASSET_RECORD_FIELDS.iter().map(|(t, _)| *t).collect();
        for input in inputs {
            let storage = CanonicalDecoder::new(&input).take_fields(&tags);
            let ours = take_record_fields(
                &mut CanonicalDecoder::new(&input),
                RecordKind::AssetRecord,
                &ASSET_RECORD_FIELDS,
            );
            match (storage, ours) {
                (Ok(expected), Ok((known, _))) => assert_eq!(known, expected),
                (Err(expected), Err(err)) => {
                    assert_eq!(err, AssetRecordError::Decode(expected), "{input:02x?}")
                }
                (storage, ours) => panic!("불일치: storage {storage:?}, ours {ours:?}"),
            }
        }
    }

    #[test]
    fn nested_unknown_mandatory_tag_is_rejected() {
        let source = source_payload(&[
            field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_PROJECT]),
            field(TAG_SOURCE_PATH, true, string_payload("a.png").unwrap()),
            field(5, true, Vec::new()),
        ]);
        assert_eq!(
            decode_with_source(source).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnknownMandatoryTag { tag: 5 })
        );
    }

    #[test]
    fn missing_mandatory_fields_are_rejected() {
        for missing in [TAG_CONTENT_HASH, TAG_MEDIA_TYPE, TAG_SIZE] {
            let fields: Vec<Field> = base_fields()
                .into_iter()
                .filter(|f| f.tag != missing)
                .collect();
            assert_eq!(
                AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
                AssetRecordError::MissingField {
                    record: RecordKind::AssetRecord,
                    tag: missing
                }
            );
        }

        let kind = |value: u8| field(TAG_SOURCE_KIND, true, vec![value]);
        let path = |text: &str| field(TAG_SOURCE_PATH, true, string_payload(text).unwrap());
        for (fields, tag) in [
            (vec![], TAG_SOURCE_KIND),
            (vec![path("a.png")], TAG_SOURCE_KIND),
            (vec![kind(SOURCE_KIND_PROJECT)], TAG_SOURCE_PATH),
            // 외부 kind는 출처 지문이 필수입니다.
            (
                vec![kind(SOURCE_KIND_EXTERNAL), path("a.png")],
                TAG_SOURCE_ORIGIN,
            ),
        ] {
            assert_eq!(
                decode_with_source(source_payload(&fields)).unwrap_err(),
                AssetRecordError::MissingField {
                    record: RecordKind::SourceMetadata,
                    tag
                }
            );
        }
    }

    #[test]
    fn source_kind_and_origin_combinations_are_validated() {
        let origin = external_origin_fingerprint(Path::new("/x/a.png")).to_string();
        let origin_field = field(TAG_SOURCE_ORIGIN, true, string_payload(&origin).unwrap());
        let path = field(TAG_SOURCE_PATH, true, string_payload("a.png").unwrap());

        // 내부 kind에 지문이 있으면 거부합니다(절대 경로 유출 경로 차단).
        let source = source_payload(&[
            field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_PROJECT]),
            path.clone(),
            origin_field.clone(),
        ]);
        assert_eq!(
            decode_with_source(source).unwrap_err(),
            AssetRecordError::UnexpectedField {
                record: RecordKind::SourceMetadata,
                tag: TAG_SOURCE_ORIGIN
            }
        );

        // 정의되지 않은 kind 값.
        for value in [0u8, 3, 0xff] {
            let source = source_payload(&[field(TAG_SOURCE_KIND, true, vec![value]), path.clone()]);
            assert_eq!(
                decode_with_source(source).unwrap_err(),
                AssetRecordError::InvalidSourceKind { value }
            );
        }

        // kind payload 폭 위반.
        let source = source_payload(&[
            field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_PROJECT, 0]),
            path.clone(),
        ]);
        assert_eq!(
            decode_with_source(source).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );

        // 지문 문자열이 hash 형식이 아님.
        let source = source_payload(&[
            field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_EXTERNAL]),
            path,
            field(
                TAG_SOURCE_ORIGIN,
                true,
                string_payload("sha256:00").unwrap(),
            ),
        ]);
        assert_eq!(
            decode_with_source(source).unwrap_err(),
            AssetRecordError::InvalidContentHash(ParseContentHashError::InvalidLength {
                expected: 64,
                actual: 2
            })
        );
    }

    #[test]
    fn flag_mismatch_on_known_tag_is_rejected() {
        let mut fields = base_fields();
        fields[2].mandatory = false;
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::FlagMismatch {
                record: RecordKind::AssetRecord,
                tag: TAG_SIZE,
                mandatory: false
            }
        );

        let mut fields = base_fields();
        let source = project_source("a.png");
        fields.push(field(
            TAG_SOURCE_METADATA,
            true,
            source.encode_payload().unwrap(),
        ));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::FlagMismatch {
                record: RecordKind::AssetRecord,
                tag: TAG_SOURCE_METADATA,
                mandatory: true
            }
        );
    }

    #[test]
    fn duplicate_misordered_and_bad_flag_tags_are_rejected() {
        let fields = base_fields();
        let entry = |i: usize, flags: u8| (fields[i].tag, flags, fields[i].payload.clone());

        let duplicate = raw_record(&[entry(0, 1), entry(0, 1), entry(1, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&duplicate).unwrap_err(),
            AssetRecordError::Decode(DecodeError::DuplicateTag { tag: 1 })
        );

        let misordered = raw_record(&[entry(1, 1), entry(0, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&misordered).unwrap_err(),
            AssetRecordError::Decode(DecodeError::MisorderedTag { tag: 1 })
        );

        let bad_flags = raw_record(&[entry(0, 0x03), entry(1, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&bad_flags).unwrap_err(),
            AssetRecordError::Decode(DecodeError::InvalidFlags { value: 0x03 })
        );
    }

    #[test]
    fn invalid_content_hash_is_rejected() {
        let mut fields = base_fields();
        fields[0].payload = string_payload("sha256:abcd").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::InvalidContentHash(ParseContentHashError::InvalidLength {
                expected: 64,
                actual: 4
            })
        );
    }

    #[test]
    fn invalid_and_non_canonical_media_type_are_rejected() {
        let mut fields = base_fields();
        fields[1].payload = string_payload("imagepng").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::InvalidMediaType(MediaTypeError::MissingSeparator)
        );

        fields[1].payload = string_payload("Image/PNG").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::NonCanonicalMediaType {
                found: "Image/PNG".to_string()
            }
        );
    }

    #[test]
    fn field_payload_must_be_consumed_exactly() {
        // size payload 9 bytes(잔여 1) / 7 bytes(부족).
        let mut fields = base_fields();
        let mut long = u64_payload(1234);
        long.push(0);
        fields[2].payload = long;
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
        fields[2].payload = u64_payload(1234)[..7].to_vec();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnexpectedEof {
                needed: 8,
                remaining: 7
            })
        );

        // 문자열 payload 뒤 잔여 bytes.
        let mut fields = base_fields();
        fields[1].payload.push(b'x');
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );

        // nested source record 뒤 잔여 bytes.
        let mut source = project_source("a.png").encode_payload().unwrap();
        source.push(0);
        assert_eq!(
            decode_with_source(source).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn invalid_project_paths_are_rejected() {
        for (path, expected) in [
            ("", SourceMetadataError::EmptyPath),
            ("a\0b.png", SourceMetadataError::ContainsNul),
            ("/Users/someone/a.png", SourceMetadataError::AbsolutePath),
            ("dir\\a.png", SourceMetadataError::Backslash),
            (
                "../a.png",
                SourceMetadataError::InvalidComponent {
                    component: "..".to_string(),
                },
            ),
            (
                "a/./b.png",
                SourceMetadataError::InvalidComponent {
                    component: ".".to_string(),
                },
            ),
            (
                "a//b.png",
                SourceMetadataError::InvalidComponent {
                    component: String::new(),
                },
            ),
            (
                "a/",
                SourceMetadataError::InvalidComponent {
                    component: String::new(),
                },
            ),
        ] {
            assert_eq!(
                SourceMetadata::project(path).unwrap_err(),
                expected,
                "{path:?}"
            );
            // 디코딩 경로에서도 같은 규칙으로 거부합니다(절대 경로가 기록된 입력 포함).
            let source = source_payload(&[
                field(TAG_SOURCE_KIND, true, vec![SOURCE_KIND_PROJECT]),
                field(TAG_SOURCE_PATH, true, string_payload(path).unwrap()),
            ]);
            assert_eq!(
                decode_with_source(source).unwrap_err(),
                AssetRecordError::InvalidSourceMetadata(expected),
                "{path:?}"
            );
        }
    }

    #[test]
    fn invalid_external_file_names_are_rejected() {
        let origin = external_origin_fingerprint(Path::new("/x/a.png"));
        for (name, expected) in [
            ("", SourceMetadataError::EmptyPath),
            ("a\0.png", SourceMetadataError::ContainsNul),
            ("a\\b.png", SourceMetadataError::Backslash),
            (
                "dir/a.png",
                SourceMetadataError::InvalidFileName {
                    name: "dir/a.png".to_string(),
                },
            ),
            (
                "/Users/someone/a.png",
                SourceMetadataError::InvalidFileName {
                    name: "/Users/someone/a.png".to_string(),
                },
            ),
            (
                "..",
                SourceMetadataError::InvalidFileName {
                    name: "..".to_string(),
                },
            ),
        ] {
            assert_eq!(
                SourceMetadata::external(name, origin).unwrap_err(),
                expected,
                "{name:?}"
            );
        }
    }

    #[test]
    fn resolved_paths_inside_project_become_relative() {
        let root = Path::new("/work/project");
        let source =
            SourceMetadata::from_resolved_paths(root, Path::new("/work/project/images/로고.png"))
                .unwrap();
        assert_eq!(
            source.location(),
            &SourceLocation::Project {
                relative_path: "images/로고.png".to_string()
            }
        );
        assert_eq!(
            source.project_path(root).unwrap(),
            Path::new("/work/project/images/로고.png")
        );
        assert!(!source.matches_external_origin(Path::new("/work/project/images/로고.png")));

        // root 자체는 원본 파일이 될 수 없습니다.
        assert_eq!(
            SourceMetadata::from_resolved_paths(root, root).unwrap_err(),
            SourceMetadataError::EmptyPath
        );
        // 접두 문자열만 같은 형제 디렉터리는 외부입니다(구성 요소 단위 비교).
        let sibling =
            SourceMetadata::from_resolved_paths(root, Path::new("/work/project-old/a.png"))
                .unwrap();
        assert!(matches!(
            sibling.location(),
            SourceLocation::External { .. }
        ));
    }

    #[test]
    fn resolved_paths_outside_project_keep_only_file_name_and_fingerprint() {
        let root = Path::new("/work/project");
        let outside = Path::new("/Users/someone/Desktop/브랜드/logo.png");
        let source = SourceMetadata::from_resolved_paths(root, outside).unwrap();
        assert_eq!(
            source.location(),
            &SourceLocation::External {
                file_name: "logo.png".to_string(),
                origin: external_origin_fingerprint(outside),
            }
        );
        assert!(source.matches_external_origin(outside));
        assert!(!source.matches_external_origin(Path::new("/Users/other/Desktop/브랜드/logo.png")));
        assert_eq!(source.project_path(root), None);

        // 기록 bytes에 디렉터리 경로가 남지 않습니다(privacy).
        let mut rec = record(None);
        rec.source_metadata = Some(source);
        let bytes = rec.encode().unwrap();
        for leaked in ["/Users", "someone", "Desktop", "브랜드"] {
            assert!(
                !bytes.windows(leaked.len()).any(|w| w == leaked.as_bytes()),
                "{leaked:?}가 기록됨"
            );
        }
        assert!(bytes.windows(8).any(|w| w == b"logo.png"));
    }

    #[test]
    fn fingerprint_is_domain_separated_and_path_sensitive() {
        let a = Path::new("/a/logo.png");
        assert_eq!(
            external_origin_fingerprint(a),
            external_origin_fingerprint(a)
        );
        assert_ne!(
            external_origin_fingerprint(a),
            external_origin_fingerprint(Path::new("/b/logo.png"))
        );
        // 경로 bytes의 단순 SHA-256과 다릅니다(domain prefix).
        assert_ne!(
            external_origin_fingerprint(a),
            ContentHash::compute(b"/a/logo.png")
        );
    }

    #[test]
    fn unresolved_inputs_are_rejected() {
        let root = Path::new("/work/project");
        for (root, source, expected) in [
            (
                Path::new("work/project"),
                Path::new("/work/project/a.png"),
                SourceMetadataError::NotAbsolute,
            ),
            (root, Path::new("a.png"), SourceMetadataError::NotAbsolute),
            (
                root,
                Path::new("/work/project/../secret/a.png"),
                SourceMetadataError::NotNormalized,
            ),
            (
                Path::new("/work/other/../project"),
                Path::new("/work/project/a.png"),
                SourceMetadataError::NotNormalized,
            ),
        ] {
            assert_eq!(
                SourceMetadata::from_resolved_paths(root, source).unwrap_err(),
                expected,
                "{root:?} {source:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_not_recorded() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let root = Path::new("/work/project");
        let inside = Path::new("/work/project").join(OsStr::from_bytes(b"\xff.png"));
        let outside = Path::new("/elsewhere").join(OsStr::from_bytes(b"\xfe.png"));
        for source in [inside, outside] {
            assert_eq!(
                SourceMetadata::from_resolved_paths(root, &source).unwrap_err(),
                SourceMetadataError::NonUtf8Path
            );
        }
    }

    #[test]
    fn import_path_resolution_uses_canonical_paths() {
        let base =
            std::env::temp_dir().join(format!("stellar-assets-source-test-{}", std::process::id()));
        let project = base.join("project");
        let outside = base.join("outside");
        std::fs::create_dir_all(project.join("images")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(project.join("images").join("a.png"), b"a").unwrap();
        std::fs::write(outside.join("b.png"), b"b").unwrap();

        // `..`가 섞인 비정규 입력도 canonicalize 후 판정합니다.
        let inside = SourceMetadata::from_import_path(
            &project,
            &project
                .join("images")
                .join("..")
                .join("images")
                .join("a.png"),
        )
        .unwrap();
        assert_eq!(
            inside.location(),
            &SourceLocation::Project {
                relative_path: "images/a.png".to_string()
            }
        );

        let external = SourceMetadata::from_import_path(&project, &outside.join("b.png")).unwrap();
        let resolved = std::fs::canonicalize(outside.join("b.png")).unwrap();
        assert!(external.matches_external_origin(&resolved));
        assert_eq!(external.file_name(), "b.png");

        assert_eq!(
            SourceMetadata::from_import_path(&project, &outside.join("missing.png")).unwrap_err(),
            SourceMetadataError::Resolve {
                kind: std::io::ErrorKind::NotFound
            }
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn same_content_hash_can_back_distinct_records() {
        // dedup(XIII.5.3) 전제: record는 AssetId를 포함하지 않고 content hash만 공유합니다.
        let a = record(Some("a.png"));
        let b = record(Some("copy/a.png"));
        assert_eq!(a.content_hash, b.content_hash);
        assert_ne!(a.encode().unwrap(), b.encode().unwrap());
    }

    #[test]
    fn error_messages_are_descriptive() {
        let err = AssetRecordError::MissingField {
            record: RecordKind::SourceMetadata,
            tag: 1,
        };
        assert_eq!(
            err.to_string(),
            "SourceMetadata의 필수 field(tag 1)가 없습니다"
        );
        let err = AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 2 });
        assert!(err.to_string().contains("2 bytes"));
        assert!(SourceMetadataError::AbsolutePath
            .to_string()
            .contains("상대 경로"));
    }
}
