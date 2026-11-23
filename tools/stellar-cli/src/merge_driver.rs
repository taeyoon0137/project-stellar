//! `stellar merge-driver` — Git custom merge driver (XXIV.2, ADR-0081 결정 8).
//!
//! Git이 `merge.stellar.driver = stellar merge-driver %O %A %B %P`로 부릅니다(`%O` base, `%A` ours이자 결과,
//! `%B` theirs 임시 파일, `%P` 저장소 기준 경로). 텍스트 의미 파일(지금은 컴포넌트 원본)의 세 bytes를 semantic
//! snapshot으로 올려 병합하고:
//!
//! - conflict가 없고 검증을 통과하면 `%A`에 결과 bytes를 쓰고 0으로 끝납니다.
//! - conflict가 있거나 지원하지 않는 파일·읽기·검증 실패면 `%A`를 바꾸지 않고 nonzero로 끝납니다. Git은 그 경로를
//!   conflict(unmerged)로 남깁니다.
//!
//! driver는 Git을 실행하지 않고 Git config·`.gitattributes`도 바꾸지 않습니다.

use std::path::Path;

use stellar_ops::candidate::file_merge::{merge_semantic_file, FileMergeError, FileMergeOutcome};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::project_cmds::conflict_json;

/// 한 입력 파일의 크기 상한(컴포넌트 원본 상한 1 MiB와 같음). 넘으면 읽지 않고 실패합니다.
const MAX_SIDE_BYTES: u64 = 1024 * 1024;

/// `merge-driver` 명령.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    drive(args)
}

fn read_side(path: &Path, side: &str) -> Result<Vec<u8>, CliError> {
    let fail = |message: String| CliError::new(ExitStatus::NoInput, "merge-input", message);
    let length = std::fs::metadata(path)
        .map_err(|e| fail(format!("{side} 파일을 읽을 수 없습니다: {e}")))?
        .len();
    if length > MAX_SIDE_BYTES {
        return Err(CliError::new(
            ExitStatus::DataErr,
            "merge-input-too-large",
            format!("{side} 파일이 {MAX_SIDE_BYTES} bytes를 넘습니다"),
        ));
    }
    std::fs::read(path).map_err(|e| fail(format!("{side} 파일을 읽을 수 없습니다: {e}")))
}

/// driver 실행(`%O %A %B %P`).
fn drive(args: &[String]) -> Result<Success, CliError> {
    let [base, ours, theirs, path] = args else {
        return Err(CliError::usage(
            "merge-driver는 <%O> <%A> <%B> <%P> 네 인자를 받습니다",
        ));
    };
    let relative = Path::new(path);
    let base_bytes = read_side(Path::new(base), "base(%O)")?;
    let ours_bytes = read_side(Path::new(ours), "ours(%A)")?;
    let theirs_bytes = read_side(Path::new(theirs), "theirs(%B)")?;
    let path_json = JsonValue::string(path.as_str());
    match merge_semantic_file(relative, &base_bytes, &ours_bytes, &theirs_bytes) {
        Ok(FileMergeOutcome::Merged(bytes)) => {
            std::fs::write(ours, &bytes).map_err(|e| {
                CliError::new(
                    ExitStatus::IoErr,
                    "merge-write",
                    format!("병합 결과를 %A에 쓸 수 없습니다: {e}"),
                )
            })?;
            Ok(Success {
                result: JsonValue::object(vec![
                    ("path", path_json),
                    ("outcome", JsonValue::string("merged")),
                    ("bytes", JsonValue::int(bytes.len() as i64)),
                ]),
                diagnostics: Vec::new(),
            })
        }
        Ok(FileMergeOutcome::Deleted) => Err(CliError::new(
            ExitStatus::Failure,
            "merge-deleted",
            "병합 결과에 정의가 없어 driver가 표현할 수 없습니다. 파일을 직접 지우고 git add 하세요",
        )
        .with_context(path.clone())),
        Ok(FileMergeOutcome::Conflicts(conflicts)) => Err(CliError::new(
            ExitStatus::Failure,
            "unresolved-conflicts",
            format!(
                "자동 병합할 수 없는 semantic conflict {}건이 있습니다. %A는 바꾸지 않았습니다",
                conflicts.len()
            ),
        )
        .with_result(JsonValue::object(vec![
            ("path", path_json),
            ("outcome", JsonValue::string("conflicts")),
            (
                "conflicts",
                JsonValue::Array(conflicts.iter().map(conflict_json).collect()),
            ),
        ]))
        .with_context(path.clone())),
        Err(FileMergeError::UnsupportedPath) => Err(CliError::new(
            ExitStatus::Unavailable,
            "unsupported-file",
            FileMergeError::UnsupportedPath.to_string(),
        )
        .with_context(path.clone())),
        Err(error) => Err(CliError::new(
            ExitStatus::DataErr,
            "merge-failed",
            error.to_string(),
        )
        .with_context(path.clone())),
    }
}
