//! 컴포넌트 원본 파일 하나의 bytes ↔ semantic snapshot (XXIV.2.1, ADR-0081 결정 8).
//!
//! Git merge driver와 conflict 세션은 Git 객체에서 온 base·ours·theirs bytes를 받습니다. 이 모듈은 그 bytes를
//! project의 컴포넌트 원본 codec(제한 YAML loader, 파일명 StableId 접미사 검사, 정의 tree 검사)으로 읽어 semantic
//! snapshot으로 올리고, 병합 결과 snapshot을 다시 원본 bytes로 씁니다.
//!
//! - 읽기는 bytes를 비공개 임시 폴더에 원래 파일명으로 둔 뒤 codec의 폴더 읽기로 엽니다. 빈 bytes는 파일이 없는
//!   쪽(그 정의 없음)입니다.
//! - 쓰기는 컴포넌트 원본의 유일한 쓰기 버전 v4(ADR-0077 결정 2)입니다. 레코드 역투영은 semantic-diff
//!   `component_records`이고, bytes는 project codec의 공개 encoder(`ComponentSource::from_records`·`to_text`)가 씁니다.
//!   merge 쪽에 따로 encoder를 두지 않습니다. 쓴 bytes는 반드시 codec으로 다시 읽어 같은 snapshot인지 확인하고, 다르면
//!   실패합니다(형식이 바뀌어도 조용히 잘못된 파일을 쓰지 않음).

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use stellar_project::document_io::component_source::{read_library, ComponentSource};
use stellar_project::private_temp::PrivateTempDir;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::records::component_records;
use stellar_semantic_diff::snapshot::{ObjectKind, SemanticSnapshot};
use stellar_semantic_merge::candidate::ValidatedCandidate;

use crate::core_types::ids::StableId;

/// 원본 파일 이름 접미사.
pub const COMPONENT_SOURCE_EXT: &str = ".stellar.yaml";

/// merge driver가 맡는 경로 pattern(프로젝트 루트 기준, `.gitattributes` 형식).
pub const COMPONENT_SOURCE_PATTERN: &str = "design/components/*.stellar.yaml";

/// 경로가 프로젝트의 컴포넌트 원본 파일(`…/design/components/<이름>.stellar.yaml`)인지.
pub fn is_component_source_path(path: &Path) -> bool {
    let name_ok = path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            name.len() > COMPONENT_SOURCE_EXT.len() && name.ends_with(COMPONENT_SOURCE_EXT)
        });
    let mut parents = path.parent().into_iter().flat_map(Path::components).rev();
    let folder = parents.next().map(|c| c.as_os_str() == "components");
    let design = parents.next().map(|c| c.as_os_str() == "design");
    name_ok && folder == Some(true) && design == Some(true)
}

/// 원본 파일 codec 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentFileError {
    /// 읽기 실패(문법·파일명·정의 검사, 크기 상한 등).
    Decode(String),
    /// 병합 결과를 원본으로 쓸 수 없음(지원하지 않는 의미, 여러 정의 등).
    Unsupported(String),
    /// 쓴 bytes를 다시 읽은 snapshot이 병합 결과와 다름.
    RoundTrip,
    /// 임시 폴더 입출력 실패.
    Io(String),
}

impl fmt::Display for ComponentFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentFileError::Decode(e) => write!(f, "컴포넌트 원본을 읽을 수 없습니다: {e}"),
            ComponentFileError::Unsupported(e) => {
                write!(f, "컴포넌트 원본으로 쓸 수 없습니다: {e}")
            }
            ComponentFileError::RoundTrip => {
                f.write_str("쓴 컴포넌트 원본을 다시 읽은 결과가 병합 결과와 다릅니다")
            }
            ComponentFileError::Io(e) => write!(f, "임시 폴더 입출력 실패: {e}"),
        }
    }
}

impl std::error::Error for ComponentFileError {}

fn io(error: impl fmt::Display) -> ComponentFileError {
    ComponentFileError::Io(error.to_string())
}

/// 원본 bytes 하나를 snapshot으로 올립니다. 빈 bytes는 빈 snapshot(파일 없음)입니다. `file_name`은 저장소의 원래
/// 파일명이며 codec이 StableId 접미사를 확인합니다.
pub fn decode_component_file(
    file_name: &OsStr,
    bytes: &[u8],
) -> Result<SemanticSnapshot, ComponentFileError> {
    if bytes.is_empty() {
        return Ok(SemanticSnapshot::new());
    }
    if Path::new(file_name).components().count() != 1 {
        return Err(ComponentFileError::Decode("파일 이름이 필요합니다".into()));
    }
    let dir = PrivateTempDir::create_in(&std::env::temp_dir(), "stellar-merge-file").map_err(io)?;
    std::fs::write(dir.path().join(file_name), bytes).map_err(io)?;
    let (library, files) =
        read_library(dir.path()).map_err(|e| ComponentFileError::Decode(e.to_string()))?;
    if files.len() != 1 {
        return Err(ComponentFileError::Decode(
            "정의가 정확히 하나여야 합니다".into(),
        ));
    }
    let mut builder = SnapshotBuilder::new();
    builder
        .add_project_component_library(&library)
        .map_err(|e| ComponentFileError::Decode(e.to_string()))?;
    drop(dir);
    Ok(builder.finish())
}

/// snapshot의 컴포넌트 정의 하나를 원본 bytes로 씁니다. 정의가 없으면 `None`(파일 삭제)입니다.
fn encode_snapshot(snapshot: &SemanticSnapshot) -> Result<Option<Vec<u8>>, ComponentFileError> {
    let components: Vec<StableId> = snapshot
        .objects()
        .filter(|o| o.kind() == ObjectKind::Component)
        .map(|o| o.id())
        .collect();
    let component = match components.as_slice() {
        [] if snapshot.is_empty() => return Ok(None),
        [one] => *one,
        [] => {
            return Err(ComponentFileError::Unsupported(
                "정의 없이 남은 정의 tree 객체".into(),
            ))
        }
        _ => {
            return Err(ComponentFileError::Unsupported(
                "한 파일에 정의가 여럿".into(),
            ))
        }
    };
    let projection = component_records(snapshot, component)
        .map_err(|e| ComponentFileError::Unsupported(e.to_string()))?;
    let text = ComponentSource::from_records(projection.definition, projection.nodes)
        .and_then(|source| source.to_text())
        .map_err(|e| ComponentFileError::Unsupported(e.to_string()))?;
    Ok(Some(text.into_bytes()))
}

/// 검증한 candidate(XXIII.2.4)를 원본 파일 bytes로 씁니다. 정의가 없으면 `None`(파일 삭제)입니다.
///
/// 쓴 bytes는 `file_name`으로 다시 읽어 candidate snapshot과 같은지 확인합니다.
pub fn encode_component_candidate(
    validated: ValidatedCandidate<'_>,
    file_name: &OsStr,
) -> Result<Option<Vec<u8>>, ComponentFileError> {
    let snapshot = validated.snapshot();
    let Some(bytes) = encode_snapshot(snapshot)? else {
        return Ok(None);
    };
    if &decode_component_file(file_name, &bytes)? != snapshot {
        return Err(ComponentFileError::RoundTrip);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_source_paths() {
        for (path, expected) in [
            ("design/components/Card-0123456789ab.stellar.yaml", true),
            ("sub/project/design/components/a.stellar.yaml", true),
            ("design/components/.stellar.yaml", false),
            ("design/components/x.yaml", false),
            ("components/x.stellar.yaml", false),
            ("design/other/x.stellar.yaml", false),
            ("design/current", false),
        ] {
            assert_eq!(
                is_component_source_path(Path::new(path)),
                expected,
                "{path}"
            );
        }
    }
}
