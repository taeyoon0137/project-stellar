//! 프로젝트 열기 오류 (XXIV.4.2).
//!
//! 호출자(`stellar` 명령, MCP backend)가 종료 코드·결과 종류로 분류할 수 있도록 원인을 그대로
//! 구분합니다. 메시지에는 경로를 넣지 않습니다(경로는 호출자가 진단 문맥으로만 씁니다).

use std::fmt;
use std::io;

use stellar_project::document_io::load::LoadDocumentError;
use stellar_project::manifest::ManifestError;
use stellar_project::root::RootError;
use stellar_project::yaml::LoaderError;
use stellar_storage::generation::lazy::LazyLoadError;

use crate::core_types::ids::StableId;

/// 프로젝트 열기 실패.
#[derive(Debug)]
pub enum OpenError {
    /// 입력 경로를 읽을 수 없습니다(없음·권한·I/O).
    Path {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 입력 경로가 디렉터리가 아닙니다.
    NotADirectory,
    /// Stellar 프로젝트 root가 아닙니다(`stellar.yaml` 또는 `.git` 없음, IV.2.2).
    NotAProject(RootError),
    /// `stellar.yaml` 읽기 실패.
    ManifestRead {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `stellar.yaml` 제한 YAML 문법 오류(IV.1.2).
    ManifestSyntax(LoaderError),
    /// 매니페스트 스키마 오류(IV.1.1).
    ManifestSchema(ManifestError),
    /// `project.id`가 UUID 형식이 아닙니다(IV.2.3).
    InvalidProjectId {
        /// 매니페스트의 원문 값.
        id: String,
    },
    /// 저장된 generation을 열 수 없습니다(손상·누락·다른 project id, V.4.4).
    Storage(LazyLoadError),
    /// generation의 DocumentManifest chunk를 해석하지 못했습니다(손상, VI.10.2).
    Document(DocumentReadError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Path { kind } => write!(f, "입력 경로를 읽을 수 없습니다: {kind:?}"),
            OpenError::NotADirectory => f.write_str("입력 경로가 디렉터리가 아닙니다"),
            OpenError::NotAProject(e) => write!(f, "Stellar 프로젝트 root가 아닙니다: {e}"),
            OpenError::ManifestRead { kind } => {
                write!(f, "stellar.yaml을 읽을 수 없습니다: {kind:?}")
            }
            OpenError::ManifestSyntax(e) => write!(f, "stellar.yaml YAML 문법 오류: {e}"),
            OpenError::ManifestSchema(e) => write!(f, "stellar.yaml 스키마 오류: {e}"),
            OpenError::InvalidProjectId { id } => {
                write!(f, "project.id가 UUID가 아닙니다: {id:?}")
            }
            OpenError::Storage(e) => write!(f, "저장된 generation을 열 수 없습니다: {e}"),
            OpenError::Document(e) => write!(f, "문서 manifest를 해석할 수 없습니다: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// 저장된 문서 읽기 실패.
#[derive(Debug)]
pub enum DocumentReadError {
    /// 현재 generation에 그 문서가 없습니다.
    UnknownDocument {
        /// 요청 문서.
        document: StableId,
    },
    /// 문서 chunk를 읽거나 해석하지 못했습니다(손상, VI.10.3).
    Load(LoadDocumentError),
    /// 문서 레코드를 만들지 못했습니다(모델 불변식 위반).
    Records(String),
}

impl fmt::Display for DocumentReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentReadError::UnknownDocument { document } => {
                write!(f, "현재 generation에 문서가 없습니다: {document}")
            }
            DocumentReadError::Load(e) => write!(f, "문서를 읽을 수 없습니다: {e}"),
            DocumentReadError::Records(e) => write!(f, "문서 레코드를 만들 수 없습니다: {e}"),
        }
    }
}

impl std::error::Error for DocumentReadError {}
