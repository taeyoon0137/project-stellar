//! Project Font import와 record (XIII.6.1).
//!
//! 프로젝트 폰트는 일반 에셋처럼 `AssetStore`(XIII.5.2)로 프로젝트 안에 복사하고, 그 content
//! hash를 폰트 파일 hash로 씁니다. [`ProjectFontRecord`]는 에셋 id, 폰트 파일 hash, container
//! 형식, face 수와 face별 이름, 사용자 입력 라이선스 metadata를 기록합니다.
//!
//! # import 순서
//!
//! 1. 원본 header를 probe해 폰트가 아니면 **아무 것도 쓰기 전에** 거부합니다.
//! 2. probe한 형식의 media type이 import 정책(`config/asset-import.yaml`) 허용 목록에 있어야
//!    합니다. 폰트를 binary asset으로 대체 저장하지 않습니다.
//! 3. `AssetStore::import_file`로 streaming hash·복사합니다(dedup·atomic write 포함).
//! 4. 저장된 content 파일을 다시 probe합니다. content 파일은 hash로 검증된 bytes이므로 face 수와
//!    이름은 이 결과를 씁니다. 1과 형식이 다르면(복사 도중 원본 교체) 거부합니다. 이때 이미 쓴
//!    content는 어떤 record도 참조하지 않으므로 asset GC(XIII.5.6)가 회수합니다.
//!
//! # binary 형식 (canonical tagged record, R6)
//!
//! | tag | field | mandatory | payload |
//! |---|---|---|---|
//! | 1 | asset id | 예 | 16 bytes raw |
//! | 2 | font file hash | 예 | 길이 프리픽스 UTF-8 `sha256:<hex>` |
//! | 3 | container 형식 | 예 | `u8` ([`FontFileFormat::tag`]) |
//! | 4 | faces | 예 | `u32` LE 개수 + face마다 nested record(1 family, 2 subfamily, 3 PostScript — 모두 optional 문자열) |
//! | 5 | 라이선스 metadata | 아니오 | nested record(1 이름, 2 URL, 3 저작권, 4 메모 — optional 문자열, 5 라이선스·임베딩 상태 — optional `u8` [`FontLicenseState::tag`], `Unknown`이면 생략). 비어 있으면 생략 |
//!
//! decode는 절단·잔여 bytes·tag 중복/역순·unknown mandatory tag·known tag의 mandatory flag
//! 불일치·필수 field 누락·형식과 맞지 않는 face 수·알 수 없는 라이선스 상태 tag·비canonical 값(빈
//! 문자열, 빈 라이선스 record, 명시적 `Unknown` 상태)을 거부합니다. unknown optional field는
//! 무시합니다(R6 허용 정책). 상태 field를 모르는 이전 reader는 상태를 `Unknown`으로 읽으므로 판정이
//! 생기지 않는 쪽으로만 호환됩니다.
//!
//! # 라이선스·임베딩 상태 (XIII.6.6)
//!
//! import는 호출자가 넘긴 라이선스 입력(상태 포함)을 그대로 기록하고 상태를 스스로 정하지 않습니다.
//! 폰트 파일의 OS/2 `fsType` 선언은 [`ImportedProjectFont::embedding`]으로 함께 돌려주지만 참고
//! 정보이며 상태를 바꾸지 않습니다. 상태 변경은
//! [`ProjectFontRecord::change_license_state`]([`FontLicenseState::transition`] 규칙)로만 합니다.

use std::fmt;
use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use stellar_assets::id::AssetId;
use stellar_assets::media_type::{MediaType, MediaTypeError};
use stellar_assets::policy::{AssetImportPolicy, PolicyViolation};
use stellar_assets::store::{AssetStore, ImportError, ImportedAsset};
use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError, Field};
use stellar_text::font::{FontFaceRef, FontFileHash, FontSource, ParseFontFileHashError};
use stellar_text::registry::{FontFaceKey, FontHandle, FontRegistry, FontRegistryError};

use super::license::{
    validate_license_field, FontLicenseInfo, FontLicenseState, LicenseChangeOrigin, LicenseField,
    LicenseFieldError, LicenseTransitionError,
};
use super::sfnt::{
    read_font_file, validate_face_name, EmbeddingPermissions, FaceNameError, FontFaceNames,
    FontFileFormat, FontFileInfo, FontParseError, MAX_FACES_PER_FILE,
};

const TAG_ASSET_ID: u32 = 1;
const TAG_FILE_HASH: u32 = 2;
const TAG_FORMAT: u32 = 3;
const TAG_FACES: u32 = 4;
const TAG_LICENSE: u32 = 5;

/// 최상위 known field의 (tag, mandatory).
const RECORD_FIELDS: [(u32, bool); 5] = [
    (TAG_ASSET_ID, true),
    (TAG_FILE_HASH, true),
    (TAG_FORMAT, true),
    (TAG_FACES, true),
    (TAG_LICENSE, false),
];

const FACE_FAMILY: u32 = 1;
const FACE_SUBFAMILY: u32 = 2;
const FACE_POSTSCRIPT: u32 = 3;
const FACE_FIELDS: [(u32, bool); 3] = [
    (FACE_FAMILY, false),
    (FACE_SUBFAMILY, false),
    (FACE_POSTSCRIPT, false),
];

/// 라이선스 nested record의 (tag, field).
const LICENSE_TEXT_FIELDS: [(u32, LicenseField); 4] = [
    (1, LicenseField::Name),
    (2, LicenseField::Url),
    (3, LicenseField::Copyright),
    (4, LicenseField::Notes),
];
/// 라이선스 nested record의 상태 field tag (XIII.6.6).
const LICENSE_STATE: u32 = 5;

/// record 생성·인코딩·디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFontRecordError {
    /// canonical 인코딩 실패.
    Encode(EncodeError),
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// 필수 field가 없습니다.
    MissingField {
        /// 누락 tag.
        tag: u32,
    },
    /// known tag의 mandatory flag가 형식과 다릅니다.
    MandatoryFlagMismatch {
        /// 문제 tag.
        tag: u32,
    },
    /// 폰트 파일 hash 문자열이 잘못되었습니다.
    InvalidFileHash(ParseFontFileHashError),
    /// 알 수 없는 container 형식 tag입니다.
    UnknownFormat {
        /// 입력 tag.
        tag: u8,
    },
    /// face 수가 형식에 맞지 않거나 범위(1..=[`MAX_FACES_PER_FILE`]) 밖입니다.
    FaceCount {
        /// container 형식.
        format: FontFileFormat,
        /// face 수.
        count: u64,
    },
    /// face 이름 값이 잘못되었습니다.
    InvalidFaceName(FaceNameError),
    /// 라이선스 field 값이 잘못되었습니다.
    InvalidLicense(LicenseFieldError),
    /// 알 수 없는 라이선스·임베딩 상태 tag입니다.
    UnknownLicenseState {
        /// 입력 tag.
        tag: u8,
    },
    /// canonical이 아닌 표현입니다(같은 값의 다른 bytes).
    NonCanonical {
        /// 위반 규칙.
        reason: &'static str,
    },
}

impl fmt::Display for ProjectFontRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectFontRecordError::Encode(e) => write!(f, "프로젝트 폰트 record 인코딩 실패: {e}"),
            ProjectFontRecordError::Decode(e) => write!(f, "프로젝트 폰트 record 디코딩 실패: {e}"),
            ProjectFontRecordError::MissingField { tag } => {
                write!(f, "프로젝트 폰트 record 필수 field {tag}가 없습니다")
            }
            ProjectFontRecordError::MandatoryFlagMismatch { tag } => {
                write!(
                    f,
                    "프로젝트 폰트 record field {tag}의 mandatory flag가 다릅니다"
                )
            }
            ProjectFontRecordError::InvalidFileHash(e) => {
                write!(f, "프로젝트 폰트 파일 hash가 잘못되었습니다: {e}")
            }
            ProjectFontRecordError::UnknownFormat { tag } => {
                write!(f, "알 수 없는 폰트 container 형식 tag입니다: {tag}")
            }
            ProjectFontRecordError::FaceCount { format, count } => {
                write!(f, "{format} 형식에 맞지 않는 face 수입니다: {count}")
            }
            ProjectFontRecordError::InvalidFaceName(e) => write!(f, "{e}"),
            ProjectFontRecordError::InvalidLicense(e) => write!(f, "{e}"),
            ProjectFontRecordError::UnknownLicenseState { tag } => {
                write!(f, "알 수 없는 폰트 라이선스 상태 tag입니다: {tag}")
            }
            ProjectFontRecordError::NonCanonical { reason } => {
                write!(f, "프로젝트 폰트 record가 canonical이 아닙니다: {reason}")
            }
        }
    }
}

impl std::error::Error for ProjectFontRecordError {}

impl From<EncodeError> for ProjectFontRecordError {
    fn from(e: EncodeError) -> ProjectFontRecordError {
        ProjectFontRecordError::Encode(e)
    }
}

impl From<DecodeError> for ProjectFontRecordError {
    fn from(e: DecodeError) -> ProjectFontRecordError {
        ProjectFontRecordError::Decode(e)
    }
}

/// 프로젝트 폰트 record (XIII.6.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectFontRecord {
    asset_id: AssetId,
    file_hash: FontFileHash,
    format: FontFileFormat,
    faces: Vec<FontFaceNames>,
    license: FontLicenseInfo,
}

impl ProjectFontRecord {
    /// record를 만듭니다. `faces`의 index가 face index이고, 길이가 face 수입니다.
    pub fn new(
        asset_id: AssetId,
        file_hash: FontFileHash,
        format: FontFileFormat,
        faces: Vec<FontFaceNames>,
        license: FontLicenseInfo,
    ) -> Result<ProjectFontRecord, ProjectFontRecordError> {
        check_face_count(format, faces.len() as u64)?;
        Ok(ProjectFontRecord {
            asset_id,
            file_hash,
            format,
            faces,
            license,
        })
    }

    /// 폰트 파일 content를 담은 에셋 id.
    pub fn asset_id(&self) -> AssetId {
        self.asset_id
    }

    /// 폰트 파일 hash(= 에셋 content hash).
    pub fn file_hash(&self) -> FontFileHash {
        self.file_hash
    }

    /// container 형식.
    pub fn format(&self) -> FontFileFormat {
        self.format
    }

    /// face 수.
    pub fn face_count(&self) -> u32 {
        // 생성 경로상 MAX_FACES_PER_FILE 이하입니다.
        u32::try_from(self.faces.len()).unwrap_or(u32::MAX)
    }

    /// face별 이름(index = face index).
    pub fn faces(&self) -> &[FontFaceNames] {
        &self.faces
    }

    /// face 하나의 이름.
    pub fn face_names(&self, face_index: u32) -> Option<&FontFaceNames> {
        self.faces.get(face_index as usize)
    }

    /// face 하나의 참조(source = Project, variation 없음).
    pub fn face_ref(&self, face_index: u32) -> Option<FontFaceRef> {
        (face_index < self.face_count())
            .then(|| FontFaceRef::new(FontSource::Project, self.file_hash, face_index, Vec::new()))
    }

    /// catalog의 asset id/hash와 저장 객체의 실제 bytes를 검증한 뒤 shaping용 기본 face를 엽니다.
    /// 요청한 face의 axes는 style과 합쳐 shaping할 때 적용합니다.
    pub fn load_face(
        &self,
        reference: &FontFaceRef,
        catalog: &stellar_assets::catalog::AssetCatalog,
        store: &AssetStore,
        policy: &AssetImportPolicy,
    ) -> Result<stellar_text::shaping::FontFace, ProjectFaceError> {
        if reference.source() != FontSource::Project
            || reference.file_hash() != self.file_hash
            || reference.face_index() >= self.face_count()
        {
            return Err(ProjectFaceError::ReferenceMismatch);
        }
        let asset = catalog
            .get(&self.asset_id)
            .ok_or(ProjectFaceError::MissingAsset)?;
        if asset.content_hash.digest() != self.file_hash.sha256_digest()
            || asset.media_type.as_str() != self.format.media_type()
        {
            return Err(ProjectFaceError::ReferenceMismatch);
        }
        let bytes = store
            .read_content(
                &asset.content_hash,
                asset.size,
                policy.max_file_size_bytes(),
            )
            .map_err(ProjectFaceError::Store)?
            .ok_or(ProjectFaceError::MissingAsset)?;
        stellar_text::shaping::FontFace::from_bytes(bytes, reference.face_index())
            .map_err(ProjectFaceError::Font)
    }

    /// 모든 face 참조(face index 오름차순).
    pub fn face_refs(&self) -> impl Iterator<Item = FontFaceRef> + '_ {
        (0..self.face_count())
            .map(|i| FontFaceRef::new(FontSource::Project, self.file_hash, i, Vec::new()))
    }

    /// 모든 face를 `FontSource::Project`로 registry에 등록합니다. handle은 face index 순서입니다.
    ///
    /// 같은 hash+index가 이미 있으면 registry가 기존 handle을 돌려줍니다(X.8.2 dedup).
    pub fn register_faces<R: FontRegistry + ?Sized>(
        &self,
        registry: &mut R,
    ) -> Result<Vec<FontHandle>, FontRegistryError> {
        (0..self.face_count())
            .map(|i| registry.register(FontSource::Project, FontFaceKey::new(self.file_hash, i)))
            .collect()
    }

    /// 사용자 입력 라이선스 metadata.
    pub fn license(&self) -> &FontLicenseInfo {
        &self.license
    }

    /// 라이선스 metadata를 바꿉니다(사용자 편집).
    pub fn set_license(&mut self, license: FontLicenseInfo) {
        self.license = license;
    }

    /// 라이선스·임베딩 상태 (XIII.6.6).
    pub fn license_state(&self) -> FontLicenseState {
        self.license.state()
    }

    /// 라이선스·임베딩 상태를 바꿉니다. 자동 경로는 `Unknown`으로만 바꿀 수 있습니다.
    pub fn change_license_state(
        &mut self,
        to: FontLicenseState,
        origin: LicenseChangeOrigin,
    ) -> Result<(), LicenseTransitionError> {
        self.license.change_state(to, origin)
    }

    /// canonical bytes.
    pub fn encode(&self) -> Result<Vec<u8>, ProjectFontRecordError> {
        let mut asset_id = CanonicalEncoder::new();
        self.asset_id.encode_into(&mut asset_id);
        let mut format = CanonicalEncoder::new();
        format.put_u8(self.format.tag());
        let mut faces = CanonicalEncoder::new();
        faces.put_u32(self.face_count());
        for names in &self.faces {
            let fields: Vec<Field> = [
                (FACE_FAMILY, names.family()),
                (FACE_SUBFAMILY, names.subfamily()),
                (FACE_POSTSCRIPT, names.postscript()),
            ]
            .into_iter()
            .filter_map(|(tag, value)| value.map(|v| (tag, v)))
            .map(|(tag, value)| optional_string(tag, value))
            .collect::<Result<_, _>>()?;
            faces.put_fields(&fields)?;
        }
        let mut fields = vec![
            Field {
                tag: TAG_ASSET_ID,
                mandatory: true,
                payload: asset_id.into_bytes(),
            },
            Field {
                tag: TAG_FILE_HASH,
                mandatory: true,
                payload: string_payload(&self.file_hash.to_string())?,
            },
            Field {
                tag: TAG_FORMAT,
                mandatory: true,
                payload: format.into_bytes(),
            },
            Field {
                tag: TAG_FACES,
                mandatory: true,
                payload: faces.into_bytes(),
            },
        ];
        if !self.license.is_empty() {
            fields.push(Field {
                tag: TAG_LICENSE,
                mandatory: false,
                payload: encode_license(&self.license)?,
            });
        }
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(&fields)?;
        Ok(enc.into_bytes())
    }

    /// canonical bytes에서 복원합니다. 끝에 남은 bytes가 있으면 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ProjectFontRecord, ProjectFontRecordError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let fields = take_known_fields(&mut dec, &RECORD_FIELDS)?;
        dec.expect_end()?;

        let mut asset_dec = CanonicalDecoder::new(required(&fields, TAG_ASSET_ID)?);
        let asset_id = AssetId::decode_from(&mut asset_dec)?;
        asset_dec.expect_end()?;

        let hash_text = take_string_payload(required(&fields, TAG_FILE_HASH)?)?;
        let file_hash: FontFileHash = hash_text
            .parse()
            .map_err(ProjectFontRecordError::InvalidFileHash)?;

        let mut format_dec = CanonicalDecoder::new(required(&fields, TAG_FORMAT)?);
        let tag = format_dec.take_u8()?;
        format_dec.expect_end()?;
        let format =
            FontFileFormat::from_tag(tag).ok_or(ProjectFontRecordError::UnknownFormat { tag })?;

        let mut faces_dec = CanonicalDecoder::new(required(&fields, TAG_FACES)?);
        let count = faces_dec.take_u32()?;
        check_face_count(format, u64::from(count))?;
        let mut faces = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let face_fields = take_known_fields(&mut faces_dec, &FACE_FIELDS)?;
            let text = |tag| -> Result<Option<String>, ProjectFontRecordError> {
                find(&face_fields, tag)
                    .map(|payload| {
                        let value = take_string_payload(payload)?;
                        validate_face_name(value)
                            .map_err(ProjectFontRecordError::InvalidFaceName)?;
                        Ok(value.to_string())
                    })
                    .transpose()
            };
            faces.push(
                FontFaceNames::new(
                    text(FACE_FAMILY)?,
                    text(FACE_SUBFAMILY)?,
                    text(FACE_POSTSCRIPT)?,
                )
                .map_err(ProjectFontRecordError::InvalidFaceName)?,
            );
        }
        faces_dec.expect_end()?;

        let license = match find(&fields, TAG_LICENSE) {
            Some(payload) => decode_license(payload)?,
            None => FontLicenseInfo::new(),
        };

        Ok(ProjectFontRecord {
            asset_id,
            file_hash,
            format,
            faces,
            license,
        })
    }
}

fn check_face_count(format: FontFileFormat, count: u64) -> Result<(), ProjectFontRecordError> {
    let valid = if format.is_single_face() {
        count == 1
    } else {
        (1..=u64::from(MAX_FACES_PER_FILE)).contains(&count)
    };
    if valid {
        Ok(())
    } else {
        Err(ProjectFontRecordError::FaceCount { format, count })
    }
}

fn string_payload(value: &str) -> Result<Vec<u8>, EncodeError> {
    let mut enc = CanonicalEncoder::new();
    enc.put_string(value)?;
    Ok(enc.into_bytes())
}

fn optional_string(tag: u32, value: &str) -> Result<Field, EncodeError> {
    Ok(Field {
        tag,
        mandatory: false,
        payload: string_payload(value)?,
    })
}

fn take_string_payload(payload: &[u8]) -> Result<&str, ProjectFontRecordError> {
    let mut dec = CanonicalDecoder::new(payload);
    let value = dec.take_string()?;
    dec.expect_end()?;
    Ok(value)
}

/// known field만 읽고 mandatory flag를 검사합니다. unknown optional field는 무시합니다.
fn take_known_fields(
    dec: &mut CanonicalDecoder<'_>,
    known: &[(u32, bool)],
) -> Result<Vec<Field>, ProjectFontRecordError> {
    let tags: Vec<u32> = known.iter().map(|(tag, _)| *tag).collect();
    let fields = dec.take_fields(&tags)?;
    for field in &fields {
        let expected = known
            .iter()
            .find(|(tag, _)| *tag == field.tag)
            .map(|(_, mandatory)| *mandatory);
        if expected != Some(field.mandatory) {
            return Err(ProjectFontRecordError::MandatoryFlagMismatch { tag: field.tag });
        }
    }
    Ok(fields)
}

fn find(fields: &[Field], tag: u32) -> Option<&[u8]> {
    fields
        .iter()
        .find(|f| f.tag == tag)
        .map(|f| f.payload.as_slice())
}

fn required(fields: &[Field], tag: u32) -> Result<&[u8], ProjectFontRecordError> {
    find(fields, tag).ok_or(ProjectFontRecordError::MissingField { tag })
}

fn encode_license(license: &FontLicenseInfo) -> Result<Vec<u8>, ProjectFontRecordError> {
    let mut fields: Vec<Field> = LICENSE_TEXT_FIELDS
        .iter()
        .filter_map(|(tag, field)| license.get(*field).map(|v| (*tag, v)))
        .map(|(tag, value)| optional_string(tag, value))
        .collect::<Result<_, _>>()?;
    if license.state() != FontLicenseState::Unknown {
        fields.push(Field {
            tag: LICENSE_STATE,
            mandatory: false,
            payload: vec![license.state().tag()],
        });
    }
    let mut enc = CanonicalEncoder::new();
    enc.put_fields(&fields)?;
    Ok(enc.into_bytes())
}

fn decode_license(payload: &[u8]) -> Result<FontLicenseInfo, ProjectFontRecordError> {
    let known: Vec<(u32, bool)> = LICENSE_TEXT_FIELDS
        .iter()
        .map(|(tag, _)| (*tag, false))
        .chain([(LICENSE_STATE, false)])
        .collect();
    let mut dec = CanonicalDecoder::new(payload);
    let fields = take_known_fields(&mut dec, &known)?;
    dec.expect_end()?;
    let mut license = FontLicenseInfo::new();
    for (tag, field) in LICENSE_TEXT_FIELDS {
        if let Some(payload) = find(&fields, tag) {
            let value = take_string_payload(payload)?;
            validate_license_field(field, value).map_err(ProjectFontRecordError::InvalidLicense)?;
            license
                .set(field, Some(value.to_string()))
                .map_err(ProjectFontRecordError::InvalidLicense)?;
        }
    }
    if let Some(payload) = find(&fields, LICENSE_STATE) {
        let mut state_dec = CanonicalDecoder::new(payload);
        let tag = state_dec.take_u8()?;
        state_dec.expect_end()?;
        let state = FontLicenseState::from_tag(tag)
            .ok_or(ProjectFontRecordError::UnknownLicenseState { tag })?;
        if state == FontLicenseState::Unknown {
            return Err(ProjectFontRecordError::NonCanonical {
                reason: "Unknown 라이선스 상태는 field를 생략해야 합니다",
            });
        }
        // 저장된 값은 이전에 기록된 결정을 복원하는 것이므로 새 판정이 아닙니다.
        license
            .change_state(state, LicenseChangeOrigin::User)
            .map_err(|_| ProjectFontRecordError::UnknownLicenseState { tag })?;
    }
    if license.is_empty() {
        return Err(ProjectFontRecordError::NonCanonical {
            reason: "빈 라이선스 record는 field를 생략해야 합니다",
        });
    }
    Ok(license)
}

/// Project Font import 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFontImportError {
    /// 원본을 열거나 읽을 수 없습니다.
    Source {
        /// I/O 오류 종류.
        kind: ErrorKind,
        /// 원인 메시지.
        message: String,
    },
    /// 원본이 일반 파일이 아닙니다.
    NotAFile,
    /// 원본이 지원하는 폰트 container가 아닙니다. 아무 것도 쓰지 않았습니다.
    NotAFont(FontParseError),
    /// media type 생성 실패(발생하지 않아야 함).
    MediaType(MediaTypeError),
    /// 폰트 media type이 import 정책 허용 목록에 없습니다. 아무 것도 쓰지 않았습니다.
    MediaTypeNotAllowed {
        /// probe한 형식의 media type.
        media_type: MediaType,
    },
    /// import 정책 위반.
    Policy(PolicyViolation),
    /// 에셋 import 실패.
    Import(ImportError),
    /// 저장된 content 파일을 다시 probe하지 못했습니다.
    StoredContent(FontParseError),
    /// probe 뒤 복사 전에 원본 형식이 바뀌었습니다.
    SourceChanged {
        /// 처음 probe한 형식.
        probed: FontFileFormat,
        /// 저장된 content의 형식.
        stored: FontFileFormat,
    },
    /// record 생성 실패.
    Record(ProjectFontRecordError),
}

impl fmt::Display for ProjectFontImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectFontImportError::Source { message, .. } => {
                write!(f, "폰트 원본을 읽을 수 없습니다: {message}")
            }
            ProjectFontImportError::NotAFile => f.write_str("폰트 원본이 일반 파일이 아닙니다"),
            ProjectFontImportError::NotAFont(e) => write!(f, "지원하는 폰트 파일이 아닙니다: {e}"),
            ProjectFontImportError::MediaType(e) => write!(f, "{e}"),
            ProjectFontImportError::MediaTypeNotAllowed { media_type } => {
                write!(f, "폰트 media type이 import 정책에 없습니다: {media_type}")
            }
            ProjectFontImportError::Policy(e) => write!(f, "{e}"),
            ProjectFontImportError::Import(e) => write!(f, "{e}"),
            ProjectFontImportError::StoredContent(e) => {
                write!(f, "저장된 폰트 content를 읽을 수 없습니다: {e}")
            }
            ProjectFontImportError::SourceChanged { probed, stored } => write!(
                f,
                "import 도중 폰트 원본이 바뀌었습니다: {probed} → {stored}"
            ),
            ProjectFontImportError::Record(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProjectFontImportError {}

/// 프로젝트 font 해석 오류.
#[derive(Debug)]
pub enum ProjectFaceError {
    /// 출처·hash·face index 또는 media type 불일치.
    ReferenceMismatch,
    /// 등록된 asset이 없거나 검증된 contents가 없습니다.
    MissingAsset,
    /// 저장소에 접근할 수 없습니다.
    Store(ImportError),
    /// bytes가 shaping 가능한 SFNT가 아닙니다.
    Font(stellar_text::shaping::FontLoadError),
}

impl fmt::Display for ProjectFaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReferenceMismatch => f.write_str("프로젝트 글꼴 참조가 일치하지 않습니다"),
            Self::MissingAsset => f.write_str("검증된 프로젝트 글꼴 파일이 없습니다"),
            Self::Store(e) => write!(f, "{e}"),
            Self::Font(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProjectFaceError {}

/// Project Font import 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedProjectFont {
    /// 폰트 record.
    pub font: ProjectFontRecord,
    /// 에셋 import 결과(`AssetCatalog`에 `font.asset_id()`로 넣을 record 포함).
    pub asset: ImportedAsset,
    /// face별 OS/2 `fsType` 임베딩 권한 선언(index = face index). 참고 정보이며 라이선스 상태를
    /// 바꾸지 않습니다(XIII.6.6).
    pub embedding: Vec<Option<EmbeddingPermissions>>,
}

/// 파일 하나를 열어 probe합니다.
fn probe_file(path: &Path) -> Result<Result<FontFileInfo, FontParseError>, std::io::Error> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    Ok(read_font_file(&mut file, metadata.len()))
}

/// 외부 폰트 파일을 Project Font로 import합니다(모듈 문서의 순서).
///
/// `asset_id`는 호출자가 발급한 새 에셋 id입니다. `license`는 사용자 입력을 그대로 기록합니다.
pub fn import_project_font(
    store: &AssetStore,
    policy: &AssetImportPolicy,
    asset_id: AssetId,
    source: &Path,
    license: FontLicenseInfo,
) -> Result<ImportedProjectFont, ProjectFontImportError> {
    let metadata = std::fs::metadata(source).map_err(|e| ProjectFontImportError::Source {
        kind: e.kind(),
        message: e.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(ProjectFontImportError::NotAFile);
    }
    let probed = probe_file(source)
        .map_err(|e| ProjectFontImportError::Source {
            kind: e.kind(),
            message: e.to_string(),
        })?
        .map_err(ProjectFontImportError::NotAFont)?;

    let media_type =
        MediaType::new(probed.format.media_type()).map_err(ProjectFontImportError::MediaType)?;
    let decision = policy
        .decide(&media_type)
        .map_err(ProjectFontImportError::Policy)?;
    if decision.fallback_to_binary {
        return Err(ProjectFontImportError::MediaTypeNotAllowed { media_type });
    }
    let request = policy
        .request(source, &media_type)
        .map_err(ProjectFontImportError::Policy)?;
    let asset = store
        .import_file(&request)
        .map_err(ProjectFontImportError::Import)?;

    let stored = probe_file(&store.content_path(&asset.object_path))
        .map_err(|e| {
            ProjectFontImportError::StoredContent(FontParseError::Io {
                kind: e.kind(),
                message: e.to_string(),
            })
        })?
        .map_err(ProjectFontImportError::StoredContent)?;
    if stored.format != probed.format {
        return Err(ProjectFontImportError::SourceChanged {
            probed: probed.format,
            stored: stored.format,
        });
    }

    let file_hash = FontFileHash::from_sha256_digest(*asset.record.content_hash.digest());
    let embedding = stored.faces.iter().map(|face| face.embedding).collect();
    let faces = stored.faces.into_iter().map(|face| face.names).collect();
    // 라이선스 입력(상태 포함)은 그대로 기록합니다. fsType 선언으로 상태를 정하지 않습니다.
    let font = ProjectFontRecord::new(asset_id, file_hash, stored.format, faces, license)
        .map_err(ProjectFontImportError::Record)?;
    Ok(ImportedProjectFont {
        font,
        asset,
        embedding,
    })
}

#[cfg(test)]
mod tests {

    /// 외부 원본 import에는 기기별 출처 key가 필요합니다(XIII.5.1 보안 수정). 테스트는 매번 새 key를 씁니다.
    fn test_origin_key() -> stellar_assets::origin::OriginKey {
        stellar_assets::origin::OriginKey::generate().expect("출처 key 생성")
    }
    use super::*;
    use crate::fonts::sfnt::EmbeddingLevel;
    use crate::fonts::test_fixtures::{
        family_faces, os2_table, sfnt_face, sfnt_face_with_tables, ttc, TempDir, SIG_TRUETYPE,
    };
    use stellar_assets::store::ImportOutcome;
    use stellar_storage::hash::ContentHash;
    use stellar_text::registry::InMemoryFontRegistry;

    fn shipped_policy() -> AssetImportPolicy {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/asset-import.yaml"
        );
        AssetImportPolicy::from_config_file(Path::new(path)).unwrap()
    }

    fn asset(n: u8) -> AssetId {
        AssetId::from_bytes([n; 16])
    }

    fn license() -> FontLicenseInfo {
        FontLicenseInfo::new()
            .with(LicenseField::Name, "SIL Open Font License 1.1")
            .unwrap()
            .with(LicenseField::Copyright, "Copyright 2026 Fixture Foundry")
            .unwrap()
    }

    fn sample_record() -> ProjectFontRecord {
        let faces = family_faces("Fixture Sans");
        let bytes = ttc(&faces);
        let hash = FontFileHash::from_sha256_digest(*ContentHash::compute(&bytes).digest());
        let names = ["Regular", "Bold"]
            .iter()
            .map(|style| {
                FontFaceNames::new(
                    Some("Fixture Sans".into()),
                    Some((*style).into()),
                    Some(format!("FixtureSans-{style}")),
                )
                .unwrap()
            })
            .chain([FontFaceNames::default()])
            .collect();
        ProjectFontRecord::new(asset(7), hash, FontFileFormat::Collection, names, license())
            .unwrap()
    }

    fn assets_dir_files(root: &Path) -> usize {
        fn walk(dir: &Path, n: &mut usize) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, n);
                    } else {
                        *n += 1;
                    }
                }
            }
        }
        let mut n = 0;
        walk(&root.join("assets"), &mut n);
        n
    }

    #[test]
    fn multi_face_collection_is_imported_and_registered() {
        let project = TempDir::new("project");
        let outside = TempDir::new("source");
        let bytes = ttc(&family_faces("Fixture Sans"));
        let source = outside.path.join("Fixture Sans.ttc");
        std::fs::write(&source, &bytes).unwrap();
        let store = AssetStore::new(&project.path).with_origin_key(test_origin_key());

        let imported =
            import_project_font(&store, &shipped_policy(), asset(1), &source, license()).unwrap();
        let font = &imported.font;
        // font file hash = 에셋 content hash = 원본 bytes의 SHA-256.
        let expected = ContentHash::compute(&bytes);
        assert_eq!(imported.asset.record.content_hash, expected);
        assert_eq!(font.file_hash().sha256_digest(), expected.digest());
        assert_eq!(font.asset_id(), asset(1));
        assert_eq!(font.format(), FontFileFormat::Collection);
        assert_eq!(font.face_count(), 3);
        assert_eq!(
            font.face_names(2).and_then(FontFaceNames::postscript),
            Some("FixtureSans-Italic")
        );
        assert_eq!(font.license(), &license());
        assert_eq!(imported.asset.record.media_type.as_str(), "font/collection");
        assert_eq!(imported.asset.outcome, ImportOutcome::Copied);
        assert!(imported.asset.object_path.file_name().ends_with(".ttc"));
        // 프로젝트 안에 복사된 content가 원본과 같은 bytes입니다.
        assert_eq!(
            std::fs::read(store.content_path(&imported.asset.object_path)).unwrap(),
            bytes
        );

        let mut registry = InMemoryFontRegistry::new();
        let handles = font.register_faces(&mut registry).unwrap();
        assert_eq!(handles.len(), 3);
        for (index, handle) in handles.iter().enumerate() {
            let key = FontFaceKey::new(font.file_hash(), index as u32);
            assert_eq!(registry.lookup(key), Some(*handle));
            let face = registry.face(*handle).unwrap();
            assert!(face.has_source(FontSource::Project));
            assert_eq!(
                registry.resolve(&font.face_ref(index as u32).unwrap()),
                Some(*handle)
            );
        }
        assert_eq!(font.face_refs().count(), 3);
        assert_eq!(font.face_ref(3), None);

        // 같은 파일을 새 asset id로 다시 import하면 content를 재복사하지 않고 같은 face handle.
        let again = import_project_font(
            &store,
            &shipped_policy(),
            asset(2),
            &source,
            FontLicenseInfo::new(),
        )
        .unwrap();
        assert_eq!(again.asset.outcome, ImportOutcome::Deduplicated);
        assert_eq!(again.font.file_hash(), font.file_hash());
        assert_eq!(again.font.register_faces(&mut registry).unwrap(), handles);
        assert_eq!(registry.len(), 3);
        assert_eq!(assets_dir_files(&project.path), 1);
    }

    #[test]
    fn single_face_font_is_imported_with_font_media_type() {
        let project = TempDir::new("project");
        let source = project.path.join("Fixture-Regular.ttf");
        std::fs::write(
            &source,
            sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]),
        )
        .unwrap();
        let store = AssetStore::new(&project.path).with_origin_key(test_origin_key());
        let imported = import_project_font(
            &store,
            &shipped_policy(),
            asset(3),
            &source,
            FontLicenseInfo::new(),
        )
        .unwrap();
        assert_eq!(imported.font.format(), FontFileFormat::TrueType);
        assert_eq!(imported.font.face_count(), 1);
        assert_eq!(imported.asset.record.media_type.as_str(), "font/ttf");
        assert!(imported.font.license().is_empty());
    }

    #[test]
    fn non_fonts_and_disallowed_types_are_rejected_before_writing() {
        let project = TempDir::new("project");
        let store = AssetStore::new(&project.path).with_origin_key(test_origin_key());
        let text = project.path.join("notes.ttf");
        std::fs::write(&text, b"not a font at all").unwrap();
        assert_eq!(
            import_project_font(&store, &shipped_policy(), asset(1), &text, license()),
            Err(ProjectFontImportError::NotAFont(
                FontParseError::UnknownSignature {
                    signature: *b"not "
                }
            ))
        );
        assert!(matches!(
            import_project_font(
                &store,
                &shipped_policy(),
                asset(1),
                &project.path.join("missing.ttf"),
                license()
            ),
            Err(ProjectFontImportError::Source {
                kind: ErrorKind::NotFound,
                ..
            })
        ));
        assert_eq!(
            import_project_font(
                &store,
                &shipped_policy(),
                asset(1),
                &project.path,
                license()
            ),
            Err(ProjectFontImportError::NotAFile)
        );

        // 폰트 type이 허용 목록에 없으면 binary 대체 저장 정책이어도 거부합니다.
        let font = project.path.join("Fixture.ttc");
        std::fs::write(&font, ttc(&family_faces("Fixture Sans"))).unwrap();
        let images_only = AssetImportPolicy::from_config_str(
            "max_file_size_bytes: 1000000\nunknown_media_type: store_as_binary\n\
             media_type: image/png png\n",
        )
        .unwrap();
        assert!(matches!(
            import_project_font(&store, &images_only, asset(1), &font, license()),
            Err(ProjectFontImportError::MediaTypeNotAllowed { .. })
        ));
        let rejecting = AssetImportPolicy::from_config_str(
            "max_file_size_bytes: 1000000\nunknown_media_type: reject\nmedia_type: image/png png\n",
        )
        .unwrap();
        assert!(matches!(
            import_project_font(&store, &rejecting, asset(1), &font, license()),
            Err(ProjectFontImportError::Policy(
                PolicyViolation::MediaTypeNotAllowed { .. }
            ))
        ));
        // 크기 상한은 에셋 import가 강제합니다.
        let tiny = AssetImportPolicy::from_config_str(
            "max_file_size_bytes: 16\nunknown_media_type: reject\nmedia_type: font/collection ttc\n",
        )
        .unwrap();
        assert!(matches!(
            import_project_font(&store, &tiny, asset(1), &font, license()),
            Err(ProjectFontImportError::Import(ImportError::TooLarge { .. }))
        ));
        assert_eq!(assets_dir_files(&project.path), 0);
    }

    #[test]
    fn record_round_trips_canonically() {
        let record = sample_record();
        let bytes = record.encode().unwrap();
        let decoded = ProjectFontRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.encode().unwrap(), bytes);

        // 라이선스가 비면 field 자체를 생략합니다.
        let mut bare = record.clone();
        bare.set_license(FontLicenseInfo::new());
        let bare_bytes = bare.encode().unwrap();
        assert!(bare_bytes.len() < bytes.len());
        assert_eq!(ProjectFontRecord::decode(&bare_bytes).unwrap(), bare);
    }

    #[test]
    fn record_constructor_checks_face_count_for_format() {
        let hash = FontFileHash::from_sha256_digest([1; 32]);
        let two = vec![FontFaceNames::default(); 2];
        for format in [
            FontFileFormat::TrueType,
            FontFileFormat::OpenTypeCff,
            FontFileFormat::Woff,
        ] {
            assert_eq!(
                ProjectFontRecord::new(asset(1), hash, format, two.clone(), FontLicenseInfo::new()),
                Err(ProjectFontRecordError::FaceCount { format, count: 2 })
            );
        }
        for format in FontFileFormat::ALL {
            assert_eq!(
                ProjectFontRecord::new(asset(1), hash, format, vec![], FontLicenseInfo::new()),
                Err(ProjectFontRecordError::FaceCount { format, count: 0 })
            );
        }
        assert!(ProjectFontRecord::new(
            asset(1),
            hash,
            FontFileFormat::Woff2,
            two,
            FontLicenseInfo::new()
        )
        .is_ok());
    }

    /// 테스트용 record bytes를 field 목록으로 직접 만듭니다.
    fn raw_record(fields: &[Field]) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(fields).unwrap();
        enc.into_bytes()
    }

    fn base_fields() -> Vec<Field> {
        let mut asset_id = CanonicalEncoder::new();
        asset(9).encode_into(&mut asset_id);
        let mut faces = CanonicalEncoder::new();
        faces.put_u32(1);
        faces.put_fields(&[]).unwrap();
        vec![
            Field {
                tag: TAG_ASSET_ID,
                mandatory: true,
                payload: asset_id.into_bytes(),
            },
            Field {
                tag: TAG_FILE_HASH,
                mandatory: true,
                payload: string_payload(&FontFileHash::from_sha256_digest([2; 32]).to_string())
                    .unwrap(),
            },
            Field {
                tag: TAG_FORMAT,
                mandatory: true,
                payload: vec![FontFileFormat::TrueType.tag()],
            },
            Field {
                tag: TAG_FACES,
                mandatory: true,
                payload: faces.into_bytes(),
            },
        ]
    }

    fn with_field(tag: u32, mandatory: bool, payload: Vec<u8>) -> Vec<u8> {
        let mut fields: Vec<Field> = base_fields().into_iter().filter(|f| f.tag != tag).collect();
        fields.push(Field {
            tag,
            mandatory,
            payload,
        });
        raw_record(&fields)
    }

    #[test]
    fn decode_rejects_hostile_records() {
        assert!(ProjectFontRecord::decode(&raw_record(&base_fields())).is_ok());

        let bytes = sample_record().encode().unwrap();
        for len in 0..bytes.len() {
            assert!(
                ProjectFontRecord::decode(&bytes[..len]).is_err(),
                "prefix {len}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(ProjectFontRecord::decode(&trailing).is_err());

        // 필수 field 누락.
        let missing: Vec<Field> = base_fields()
            .into_iter()
            .filter(|f| f.tag != TAG_FACES)
            .collect();
        assert_eq!(
            ProjectFontRecord::decode(&raw_record(&missing)),
            Err(ProjectFontRecordError::MissingField { tag: TAG_FACES })
        );
        // unknown mandatory tag는 거부, unknown optional tag는 무시.
        assert_eq!(
            ProjectFontRecord::decode(&with_field(99, true, vec![])),
            Err(ProjectFontRecordError::Decode(
                DecodeError::UnknownMandatoryTag { tag: 99 }
            ))
        );
        assert!(ProjectFontRecord::decode(&with_field(99, false, vec![1, 2, 3])).is_ok());
        // known tag의 mandatory flag 불일치.
        assert_eq!(
            ProjectFontRecord::decode(&with_field(TAG_FORMAT, false, vec![1])),
            Err(ProjectFontRecordError::MandatoryFlagMismatch { tag: TAG_FORMAT })
        );
        // 알 수 없는 형식, 형식과 맞지 않는 face 수, face 수 상한.
        assert_eq!(
            ProjectFontRecord::decode(&with_field(TAG_FORMAT, true, vec![42])),
            Err(ProjectFontRecordError::UnknownFormat { tag: 42 })
        );
        let mut two = CanonicalEncoder::new();
        two.put_u32(2);
        two.put_fields(&[]).unwrap();
        two.put_fields(&[]).unwrap();
        assert_eq!(
            ProjectFontRecord::decode(&with_field(TAG_FACES, true, two.into_bytes())),
            Err(ProjectFontRecordError::FaceCount {
                format: FontFileFormat::TrueType,
                count: 2
            })
        );
        let mut huge = CanonicalEncoder::new();
        huge.put_u32(u32::MAX);
        let mut fields: Vec<Field> = base_fields()
            .into_iter()
            .filter(|f| f.tag != TAG_FORMAT && f.tag != TAG_FACES)
            .collect();
        fields.push(Field {
            tag: TAG_FORMAT,
            mandatory: true,
            payload: vec![FontFileFormat::Collection.tag()],
        });
        fields.push(Field {
            tag: TAG_FACES,
            mandatory: true,
            payload: huge.into_bytes(),
        });
        assert_eq!(
            ProjectFontRecord::decode(&raw_record(&fields)),
            Err(ProjectFontRecordError::FaceCount {
                format: FontFileFormat::Collection,
                count: u64::from(u32::MAX)
            })
        );
        // 잘못된 hash 문자열.
        assert!(matches!(
            ProjectFontRecord::decode(&with_field(
                TAG_FILE_HASH,
                true,
                string_payload("sha256:abc").unwrap()
            )),
            Err(ProjectFontRecordError::InvalidFileHash(_))
        ));
        // face 이름: 빈 문자열은 비canonical.
        let mut empty_name = CanonicalEncoder::new();
        empty_name.put_u32(1);
        empty_name
            .put_fields(&[optional_string(FACE_FAMILY, "").unwrap()])
            .unwrap();
        assert_eq!(
            ProjectFontRecord::decode(&with_field(TAG_FACES, true, empty_name.into_bytes())),
            Err(ProjectFontRecordError::InvalidFaceName(
                FaceNameError::Empty
            ))
        );
        // 빈 라이선스 record는 생략해야 합니다.
        let mut empty_license = CanonicalEncoder::new();
        empty_license.put_fields(&[]).unwrap();
        assert!(matches!(
            ProjectFontRecord::decode(&with_field(TAG_LICENSE, false, empty_license.into_bytes())),
            Err(ProjectFontRecordError::NonCanonical { .. })
        ));
        // 라이선스 field 값 규칙.
        let mut bad_license = CanonicalEncoder::new();
        bad_license
            .put_fields(&[optional_string(1, "OFL\u{1}").unwrap()])
            .unwrap();
        assert_eq!(
            ProjectFontRecord::decode(&with_field(TAG_LICENSE, false, bad_license.into_bytes())),
            Err(ProjectFontRecordError::InvalidLicense(
                LicenseFieldError::ControlCharacter {
                    field: LicenseField::Name
                }
            ))
        );
    }

    #[test]
    fn license_state_is_stored_and_never_determined_automatically() {
        let project = TempDir::new("project");
        let store = AssetStore::new(&project.path).with_origin_key(test_origin_key());
        let names = &family_faces("Fixture Sans")[0];
        // 파일이 "Restricted License"·"Installable"을 선언해도 상태는 Unknown으로 시작합니다.
        for (fs_type, level) in [
            (0x0002, EmbeddingLevel::RestrictedLicense),
            (0x0000, EmbeddingLevel::Installable),
        ] {
            let source = project.path.join(format!("Fixture-{fs_type}.ttf"));
            std::fs::write(
                &source,
                sfnt_face_with_tables(SIG_TRUETYPE, names, &[os2_table(fs_type, 96)]),
            )
            .unwrap();
            let imported = import_project_font(
                &store,
                &shipped_policy(),
                asset(4),
                &source,
                FontLicenseInfo::new(),
            )
            .unwrap();
            assert_eq!(imported.font.license_state(), FontLicenseState::Unknown);
            assert_eq!(imported.embedding.len(), 1);
            assert_eq!(imported.embedding[0].map(|e| e.level()), Some(level));
        }

        // 자동 경로의 판정 시도는 거부되고 상태가 그대로입니다.
        let mut record = sample_record();
        for to in [
            FontLicenseState::Allowed,
            FontLicenseState::Restricted,
            FontLicenseState::UserConfirmed,
        ] {
            assert_eq!(
                record.change_license_state(to, LicenseChangeOrigin::Automatic),
                Err(LicenseTransitionError::AutomaticDetermination {
                    from: FontLicenseState::Unknown,
                    to
                })
            );
            assert_eq!(record.license_state(), FontLicenseState::Unknown);
        }

        // 사용자가 정한 상태는 canonical bytes에 저장되고 그대로 복원됩니다.
        let unknown_bytes = record.encode().unwrap();
        for state in [
            FontLicenseState::Allowed,
            FontLicenseState::Restricted,
            FontLicenseState::UserConfirmed,
        ] {
            record
                .change_license_state(state, LicenseChangeOrigin::User)
                .unwrap();
            let bytes = record.encode().unwrap();
            assert_ne!(bytes, unknown_bytes);
            let decoded = ProjectFontRecord::decode(&bytes).unwrap();
            assert_eq!(decoded.license_state(), state);
            assert_eq!(decoded, record);
            assert_eq!(decoded.encode().unwrap(), bytes);
        }
        // 자동 초기화(Unknown)는 허용되고 상태 field가 다시 생략됩니다.
        record
            .change_license_state(FontLicenseState::Unknown, LicenseChangeOrigin::Automatic)
            .unwrap();
        assert_eq!(record.encode().unwrap(), unknown_bytes);

        // 텍스트 입력 없이 상태만 있는 라이선스도 저장됩니다.
        let mut bare = sample_record();
        bare.set_license(FontLicenseInfo::new());
        bare.change_license_state(FontLicenseState::Restricted, LicenseChangeOrigin::User)
            .unwrap();
        let decoded = ProjectFontRecord::decode(&bare.encode().unwrap()).unwrap();
        assert_eq!(decoded.license_state(), FontLicenseState::Restricted);
        assert_eq!(decoded.license().get(LicenseField::Name), None);
    }

    #[test]
    fn decode_rejects_unknown_or_explicit_unknown_license_state() {
        let license_with_state = |tag: u8| {
            let mut enc = CanonicalEncoder::new();
            enc.put_fields(&[
                optional_string(1, "OFL").unwrap(),
                Field {
                    tag: LICENSE_STATE,
                    mandatory: false,
                    payload: vec![tag],
                },
            ])
            .unwrap();
            with_field(TAG_LICENSE, false, enc.into_bytes())
        };
        for tag in [0u8, 5, 0xFF] {
            assert_eq!(
                ProjectFontRecord::decode(&license_with_state(tag)),
                Err(ProjectFontRecordError::UnknownLicenseState { tag })
            );
        }
        assert!(matches!(
            ProjectFontRecord::decode(&license_with_state(FontLicenseState::Unknown.tag())),
            Err(ProjectFontRecordError::NonCanonical { .. })
        ));
        let decoded =
            ProjectFontRecord::decode(&license_with_state(FontLicenseState::Allowed.tag()))
                .unwrap();
        assert_eq!(decoded.license_state(), FontLicenseState::Allowed);
    }
}
