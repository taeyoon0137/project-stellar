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

use std::path::{Path, PathBuf};

use stellar_ops::candidate::file_merge::{merge_semantic_file, FileMergeError, FileMergeOutcome};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
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

/// conflict 기록 폴더(프로젝트 루트 기준, Git 밖).
pub const MERGE_RECORD_DIR: &str = ".stellar/merge";

/// `%P`(현재 폴더 = 저장소 최상위 기준)를 담은 Stellar 프로젝트 루트(`stellar.yaml`이 있는 가장 가까운 조상).
fn project_root_of(relative: &Path) -> Option<PathBuf> {
    let mut current = relative.parent();
    while let Some(dir) = current {
        let candidate = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if candidate.join("stellar.yaml").is_file() {
            return Some(candidate.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// 기록 파일 이름: 프로젝트 기준 경로의 `/`를 `__`로 바꾼 이름 + `.json`.
fn record_name(relative: &Path) -> String {
    let text: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    format!("{}.json", text.join("__"))
}

/// conflict 목록을 프로젝트의 `.stellar/merge/`에 JSON으로 남깁니다(ADR-0081 결정 8). 폴더 안 `.gitignore`(`*`)로
/// Git 밖에 둡니다. 쓰기는 임시 파일 뒤 rename입니다.
fn record_conflicts(relative: &Path, value: &JsonValue) -> std::io::Result<PathBuf> {
    let root = project_root_of(relative).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "stellar.yaml을 찾지 못했습니다",
        )
    })?;
    let within = relative.strip_prefix(&root).unwrap_or(relative);
    let dir = root.join(MERGE_RECORD_DIR);
    std::fs::create_dir_all(&dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")?;
    }
    let target = dir.join(record_name(within));
    let temp = dir.join(format!(
        ".{}.tmp-{}",
        record_name(within),
        std::process::id()
    ));
    let mut text = crate::json::write(value);
    text.push('\n');
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &target)?;
    Ok(target)
}

/// 같은 경로의 이전 conflict 기록을 지웁니다(병합 성공 시).
fn clear_record(relative: &Path) {
    if let Some(root) = project_root_of(relative) {
        let within = relative.strip_prefix(&root).unwrap_or(relative);
        let _ = std::fs::remove_file(root.join(MERGE_RECORD_DIR).join(record_name(within)));
    }
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
            clear_record(relative);
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
        Ok(FileMergeOutcome::Conflicts(conflicts)) => {
            let mut fields = vec![
                ("path", path_json),
                ("outcome", JsonValue::string("conflicts")),
                (
                    "conflicts",
                    JsonValue::Array(conflicts.iter().map(conflict_json).collect()),
                ),
            ];
            // 기록 실패는 driver 결과(nonzero, %A 미변경)를 바꾸지 않고 결과에 남깁니다.
            let record = record_conflicts(relative, &JsonValue::object(fields.clone()));
            fields.push((
                "record",
                match &record {
                    Ok(path) => JsonValue::string(path.to_string_lossy()),
                    Err(_) => JsonValue::Null,
                },
            ));
            let mut error = CliError::new(
                ExitStatus::Failure,
                "unresolved-conflicts",
                format!(
                    "자동 병합할 수 없는 semantic conflict {}건이 있습니다. %A는 바꾸지 않았고 Git은 이 경로를 \
                     unmerged로 남깁니다. 앱이나 `stellar merge-driver resolve`로 해결하세요",
                    conflicts.len()
                ),
            )
            .with_result(JsonValue::object(fields))
            .with_context(path.clone());
            if let Err(e) = record {
                error = error.with_diagnostic(Diagnostic {
                    severity: Severity::Warning,
                    code: "merge-record",
                    message: format!("conflict 기록을 쓰지 못했습니다: {e}"),
                });
            }
            Err(error)
        }
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
