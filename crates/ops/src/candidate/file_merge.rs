//! 텍스트 의미 파일 하나의 3-way semantic merge (XXIV.2.1·XXIV.2.2, ADR-0081 결정 8).
//!
//! Git merge driver는 한 파일의 base·ours·theirs bytes만 받습니다. 세 bytes를 같은 codec으로 snapshot에 올려 엔진으로
//! 병합하고(conflict 분류 XIV.10, 결과 검증 XIV.10.8), conflict가 없고 candidate 검증(XIV.12.3)과 codec 재해석 검증을
//! 통과할 때만 결과 bytes를 돌려줍니다. 아니면 conflict 목록이나 오류를 돌려주며 어떤 파일도 쓰지 않습니다.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use stellar_semantic_merge::candidate::{CandidateBlocked, CandidateSnapshot};
use stellar_semantic_merge::model::MergeInput;

use super::component_file::{
    decode_component_file, encode_component_candidate, is_component_source_path, ComponentFileError,
};
use crate::compare::ConflictEntry;

/// 파일 병합 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum FileMergeOutcome {
    /// 병합 결과 bytes(검증 통과).
    Merged(Vec<u8>),
    /// 병합 결과 정의가 없음(양쪽이 삭제 등). 파일을 지워야 합니다.
    Deleted,
    /// 자동 병합하지 못한 conflict(엔진 순서). 결과 bytes는 없습니다.
    Conflicts(Vec<ConflictEntry>),
}

/// 파일 병합 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum FileMergeError {
    /// 지원하지 않는 파일(경로 형식).
    UnsupportedPath,
    /// 한쪽 bytes를 읽을 수 없음.
    Decode {
        /// base·ours·theirs.
        side: &'static str,
        /// 원인.
        error: ComponentFileError,
    },
    /// 엔진 실패.
    Merge(String),
    /// conflict는 없지만 candidate 검증을 통과하지 못함(새 snapshot 위반).
    Invalid(String),
    /// 결과를 원본 bytes로 쓸 수 없음.
    Encode(ComponentFileError),
}

impl fmt::Display for FileMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileMergeError::UnsupportedPath => {
                f.write_str("semantic merge가 지원하지 않는 파일입니다(컴포넌트 원본만 지원)")
            }
            FileMergeError::Decode { side, error } => write!(f, "{side}: {error}"),
            FileMergeError::Merge(e) => write!(f, "semantic merge 실패: {e}"),
            FileMergeError::Invalid(e) => write!(f, "병합 결과 검증 실패: {e}"),
            FileMergeError::Encode(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FileMergeError {}

fn blocked(error: &CandidateBlocked) -> FileMergeError {
    FileMergeError::Invalid(format!("{error}: {:?}", error.violations))
}

/// 경로 `path`(저장소 기준)의 세 bytes를 병합합니다. 빈 bytes는 그쪽에 파일이 없음입니다.
pub fn merge_semantic_file(
    path: &Path,
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
) -> Result<FileMergeOutcome, FileMergeError> {
    if !is_component_source_path(path) {
        return Err(FileMergeError::UnsupportedPath);
    }
    let name = path.file_name().unwrap_or(OsStr::new(""));
    let decode = |side, bytes| {
        decode_component_file(name, bytes).map_err(|error| FileMergeError::Decode { side, error })
    };
    let input = MergeInput {
        base: decode("base", base)?,
        ours: decode("ours", ours)?,
        theirs: decode("theirs", theirs)?,
    };
    let candidate =
        CandidateSnapshot::build(input).map_err(|e| FileMergeError::Merge(e.to_string()))?;
    if candidate.unresolved_count() > 0 {
        return Ok(FileMergeOutcome::Conflicts(
            candidate
                .conflicts()
                .iter()
                .map(ConflictEntry::from)
                .collect(),
        ));
    }
    let validated = candidate.validate().map_err(|e| blocked(&e))?;
    match encode_component_candidate(validated, name).map_err(FileMergeError::Encode)? {
        Some(bytes) => Ok(FileMergeOutcome::Merged(bytes)),
        None => Ok(FileMergeOutcome::Deleted),
    }
}

#[cfg(test)]
pub(crate) mod tests;
