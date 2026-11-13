//! `.gitattributes` 해석 결과 읽기 — `git check-attr` (XIII.7.2~).
//!
//! Stellar는 `.gitattributes` 문법(패턴·우선순위·매크로)을 직접 해석하지 않습니다. 경로별 속성 값은
//! 시스템 Git `git check-attr -z --stdin <attr>`로 읽습니다. 입력은 NUL로 끝나는 경로 목록이고, 출력은
//! 경로마다 `<path> NUL <attr> NUL <info> NUL`입니다. `<info>`는 `unspecified`·`unset`·`set` 또는 값입니다.
//!
//! - 경로는 worktree root 기준 Git 경로입니다. cwd를 worktree root로 실행합니다.
//! - 응답 경로·속성 이름이 요청 순서와 다르거나 레코드 수가 맞지 않으면 해석 오류입니다.
//! - 읽기 전용입니다(`.gitattributes`·index를 바꾸지 않음).

use std::path::Path;

use crate::decode::{split_nul_terminated, GitPath};
use crate::ops::{run_ok_with_input, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::snapshot::materialize::validate_snapshot_path;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "check-attr -z --stdin";

/// 한 번의 `check-attr` 실행에 넘기는 최대 경로 수.
pub const CHECK_ATTR_CHUNK: usize = 512;

/// Git 속성 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    /// 지정되지 않음.
    Unspecified,
    /// `-attr`로 해제됨.
    Unset,
    /// 값 없이 설정됨(`attr`).
    Set,
    /// 값(`attr=value`).
    Value(Vec<u8>),
}

impl AttrValue {
    fn from_info(info: &[u8]) -> AttrValue {
        match info {
            b"unspecified" => AttrValue::Unspecified,
            b"unset" => AttrValue::Unset,
            b"set" => AttrValue::Set,
            other => AttrValue::Value(other.to_vec()),
        }
    }

    /// 값이 `value`와 같은지(`attr=value`).
    pub fn is_value(&self, value: &[u8]) -> bool {
        matches!(self, AttrValue::Value(v) if v == value)
    }
}

/// 속성 읽기 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrError {
    /// Git 경로로 쓸 수 없는 경로(Git 실행 전 거부).
    InvalidPath(GitPath),
    /// Git 실행·해석 실패.
    Op(GitOpError),
}

impl std::fmt::Display for AttrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttrError::InvalidPath(p) => {
                write!(f, "속성을 읽을 수 없는 경로: {:?}", p.text().display())
            }
            AttrError::Op(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AttrError {}

impl From<GitOpError> for AttrError {
    fn from(e: GitOpError) -> AttrError {
        AttrError::Op(e)
    }
}

/// `check-attr -z` 출력을 요청 순서대로 해석합니다(속성 하나).
pub fn parse_check_attr_z(
    stdout: &[u8],
    requested: &[GitPath],
    attr: &str,
) -> Result<Vec<(GitPath, AttrValue)>, ParseFailure> {
    let records = split_nul_terminated(stdout)
        .map_err(|e| ParseFailure::new(COMMAND, e.to_string(), stdout))?;
    if records.len() != requested.len().saturating_mul(3) {
        return Err(ParseFailure::new(
            COMMAND,
            "레코드 수가 요청과 다름",
            stdout,
        ));
    }
    let mut out = Vec::with_capacity(requested.len());
    for (path, triple) in requested.iter().zip(records.chunks_exact(3)) {
        if triple[0] != path.as_bytes() {
            return Err(ParseFailure::new(
                COMMAND,
                "응답 경로가 요청과 다름",
                triple[0],
            ));
        }
        if triple[1] != attr.as_bytes() {
            return Err(ParseFailure::new(
                COMMAND,
                "응답 속성이 요청과 다름",
                triple[1],
            ));
        }
        out.push((path.clone(), AttrValue::from_info(triple[2])));
    }
    Ok(out)
}

/// 경로들의 속성 `attr` 값을 읽습니다(worktree root 기준, 읽기 전용).
///
/// 경로는 Git 실행 전에 모두 검증합니다(빈 경로·절대 경로·`..`·`.git`·NUL 거부).
pub fn check_attribute(
    git: &GatedGitRunner,
    worktree_root: &Path,
    attr: &'static str,
    paths: &[GitPath],
) -> Result<Vec<(GitPath, AttrValue)>, AttrError> {
    for path in paths {
        if validate_snapshot_path(path.as_bytes()).is_err() {
            return Err(AttrError::InvalidPath(path.clone()));
        }
    }
    let mut out = Vec::with_capacity(paths.len());
    for chunk in paths.chunks(CHECK_ATTR_CHUNK) {
        let mut input = Vec::new();
        for path in chunk {
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        let output = run_ok_with_input(
            git,
            worktree_root,
            &argv(["check-attr", "-z", "--stdin", attr]),
            &input,
        )?;
        out.extend(parse_check_attr_z(&output.stdout, chunk, attr).map_err(GitOpError::Parse)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> GitPath {
        GitPath::from_bytes(s.as_bytes())
    }

    #[test]
    fn parses_values_in_request_order() {
        let out =
            b"a.bin\0filter\0lfs\0b.png\0filter\0unspecified\0c\0filter\0set\0d\0filter\0unset\0";
        let parsed =
            parse_check_attr_z(out, &[p("a.bin"), p("b.png"), p("c"), p("d")], "filter").unwrap();
        assert_eq!(
            parsed.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            vec![
                AttrValue::Value(b"lfs".to_vec()),
                AttrValue::Unspecified,
                AttrValue::Set,
                AttrValue::Unset,
            ]
        );
        assert!(parsed[0].1.is_value(b"lfs"));
        assert!(!parsed[1].1.is_value(b"lfs"));
        assert!(parse_check_attr_z(b"", &[], "filter").unwrap().is_empty());
    }

    #[test]
    fn malformed_output_is_rejected() {
        let req = [p("a")];
        for bad in [
            &b"a\0filter\0lfs"[..],
            b"a\0filter\0",
            b"b\0filter\0lfs\0",
            b"a\0diff\0lfs\0",
            b"a\0filter\0lfs\0a\0filter\0lfs\0",
            b"",
        ] {
            assert!(
                parse_check_attr_z(bad, &req, "filter").is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn invalid_paths_are_rejected_before_spawn() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        for bad in [&b"a\0b"[..], b"/abs", b"../x", b".git/config", b""] {
            assert_eq!(
                check_attribute(
                    &gate,
                    &std::env::temp_dir(),
                    "filter",
                    &[GitPath::from_bytes(bad)]
                ),
                Err(AttrError::InvalidPath(GitPath::from_bytes(bad)))
            );
        }
        assert_eq!(runner.spawn_count(), 0);
    }
}
