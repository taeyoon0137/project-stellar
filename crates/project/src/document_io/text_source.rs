//! ADR-0059 첫 전환 단위: 단일 page/View 문서를 사람이 읽는 텍스트로 저장합니다.
//! 기본 generation 저장 경로는 바꾸지 않습니다. 미지원 의미는 저장 전에 거부합니다.

use std::fmt;
use std::io;
use std::path::Path;

use stellar_document::model::DocumentModel;
use stellar_storage::hash::ContentHash;
use stellar_storage::source_file;

use crate::yaml::loader::MAX_INPUT_BYTES;

use super::text_codec;

/// 로컬 파생 chunk cache 상태. cache 실패는 원본 열기를 막지 않습니다.
#[derive(Debug)]
pub enum CacheStatus {
    /// 모든 chunk bytes가 원본에서 재생성한 내용과 같음.
    Verified,
    /// 누락·손상 chunk를 원본에서 재생성함.
    Rebuilt,
    /// cache I/O가 불가능함. 원본 문서는 정상적으로 열림.
    Unavailable(io::Error),
}

/// 텍스트 원본 오류. 오류를 성공으로 바꾸거나 기존 원본을 비워서 복구하지 않습니다.
#[derive(Debug)]
pub enum TextSourceError {
    /// 문법·모델 검증 오류.
    Invalid(String),
    /// 아직 텍스트 왕복이 지원되지 않는 의미·필드.
    Unsupported(String),
    /// 열린 뒤 원본이 바뀜. 다시 열고 병합해야 합니다.
    Changed,
    /// 파일 접근 오류.
    Io(io::Error),
}

impl fmt::Display for TextSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "텍스트 원본 검증: {message}"),
            Self::Unsupported(message) => write!(f, "지원하지 않는 텍스트 원본 의미: {message}"),
            Self::Changed => f.write_str("원본이 외부에서 변경되었습니다. 다시 열고 병합하세요"),
            Self::Io(error) => write!(f, "텍스트 원본 I/O: {error}"),
        }
    }
}

impl std::error::Error for TextSourceError {}

impl From<io::Error> for TextSourceError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::WouldBlock {
            Self::Changed
        } else {
            Self::Io(error)
        }
    }
}

/// 디스크 원본과 그 bytes의 hash. 런타임 ID는 파일에 기록하지 않습니다.
#[derive(Debug)]
pub struct OpenedTextSource {
    /// validator를 통과한 편집 가능한 문서.
    pub model: DocumentModel,
    /// 외부 편집 감지에 사용하는 실제 원본 bytes hash.
    pub source_hash: ContentHash,
}

/// 파일을 읽기 전용으로 엽니다. 캐시는 원본 해석에 필요하지 않습니다.
pub fn open(path: &Path) -> Result<OpenedTextSource, TextSourceError> {
    let bytes = source_file::read(path, MAX_INPUT_BYTES as u64)?;
    let text = std::str::from_utf8(&bytes).map_err(|e| TextSourceError::Invalid(e.to_string()))?;
    Ok(OpenedTextSource {
        model: text_codec::decode(text)?,
        source_hash: ContentHash::compute(&bytes),
    })
}

/// 텍스트를 열고 `.stellar/cache/text-source-v1/<source-hash>/`에 chunk cache를 재생성합니다.
/// 첫 단계에서는 매번 텍스트를 검증합니다. 빠른 열기·부분 로딩 최적화는 후속 범위입니다.
/// cache는 기존 ChunkId와 envelope를 사용하며 Git 원본이나 generation을 변경하지 않습니다.
pub fn open_with_cache(
    path: &Path,
    project_root: &Path,
) -> Result<(OpenedTextSource, CacheStatus), TextSourceError> {
    let opened = open(path)?;
    let serialized = super::serialize::serialize_document(&opened.model)
        .map_err(|error| TextSourceError::Invalid(error.to_string()))?;
    let mut status = CacheStatus::Verified;
    for chunk in serialized.chunks() {
        let envelope = stellar_storage::chunk::envelope::ChunkEnvelope::for_payload(
            chunk.kind,
            &chunk.payload,
        );
        let mut bytes = envelope
            .encode()
            .map_err(|error| TextSourceError::Invalid(error.to_string()))?;
        bytes.extend_from_slice(&chunk.payload);
        match source_file::refresh_cache_chunk(
            project_root,
            opened.source_hash,
            chunk.chunk_id,
            &bytes,
        ) {
            Ok(true) => {}
            Ok(false) => status = CacheStatus::Rebuilt,
            Err(error) => {
                status = CacheStatus::Unavailable(error);
                break;
            }
        }
    }
    Ok((opened, status))
}

/// 파일 I/O 없이 텍스트 저장 지원 범위·크기·모델 불변식을 검사합니다.
/// dry-run도 실제 저장과 같은 codec을 통과해야 합니다.
pub fn validate_model(model: &DocumentModel) -> Result<(), TextSourceError> {
    let text = text_codec::encode(model)?;
    if text.len() > MAX_INPUT_BYTES {
        return Err(TextSourceError::Invalid(
            "텍스트 원본 크기 상한 초과".into(),
        ));
    }
    Ok(())
}

/// 기존 파일을 덮어쓰지 않고 새 원본을 만듭니다.
pub fn save_new(path: &Path, model: &DocumentModel) -> Result<ContentHash, TextSourceError> {
    write(path, model, None)
}

/// 열린 원본 hash를 확인한 뒤 수정된 문서를 원자적으로 저장합니다.
/// 미지원 필드가 추가된 원본은 재해석 시 거부하며 절대 덮어쓰지 않습니다.
pub fn save(
    path: &Path,
    model: &DocumentModel,
    expected: ContentHash,
) -> Result<ContentHash, TextSourceError> {
    let current = open(path)?;
    if current.source_hash != expected {
        return Err(TextSourceError::Changed);
    }
    if current.model.document_id() != model.document_id() {
        return Err(TextSourceError::Invalid("문서 StableId 변경".into()));
    }
    let records = |model: &DocumentModel| {
        model
            .to_records()
            .map_err(|error| TextSourceError::Invalid(error.to_string()))
    };
    if records(&current.model)? == records(model)? {
        return Ok(current.source_hash);
    }
    write(path, model, Some(expected))
}

fn write(
    path: &Path,
    model: &DocumentModel,
    expected: Option<ContentHash>,
) -> Result<ContentHash, TextSourceError> {
    let text = text_codec::encode(model)?;
    let hash = ContentHash::compute(text.as_bytes());
    if expected != Some(hash) {
        source_file::write_checked(path, text.as_bytes(), expected, MAX_INPUT_BYTES as u64)?;
    }
    Ok(hash)
}
