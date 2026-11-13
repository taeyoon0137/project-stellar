//! Git 출력 디코딩 (XIV.1.5).
//!
//! Git stdout/stderr와 경로는 **원시 바이트가 정본**입니다. 표시용 텍스트는 다음 순서로 만듭니다.
//!
//! 1. **UTF-8 우선**: 유효한 UTF-8이면 그대로 텍스트로 씁니다([`TextEncoding::Utf8`]).
//! 2. **invalid sequence replacement 표시**: 유효하지 않은 바이트열은 U+FFFD로 바꾼 표시용
//!    텍스트를 만들고, 바뀐 sequence 수를 [`TextEncoding::Lossy`]로 구분해 기록합니다.
//! 3. **raw bytes 보존**: 어느 경우에도 원시 바이트를 함께 보존합니다. 표시용 텍스트를 다시
//!    경로·식별자로 쓰지 않습니다.
//!
//! machine parse는 `-z`(NUL 종료 레코드) 또는 안정 형식만 사용합니다. `-z` 없는 경로 출력은
//! `core.quotePath`에 따라 C 스타일로 quote되어 원시 바이트가 아니므로 해석하지 않습니다.

use std::path::PathBuf;

/// 텍스트 디코딩 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// 유효한 UTF-8(표시 텍스트 == 원시 바이트).
    Utf8,
    /// 유효하지 않은 sequence를 U+FFFD로 바꿈(표시 전용).
    Lossy {
        /// 바뀐 invalid sequence 수.
        replaced_sequences: usize,
    },
}

/// 원시 바이트와 표시용 텍스트를 함께 가진 Git 출력 텍스트.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitText {
    raw: Vec<u8>,
    display: String,
    encoding: TextEncoding,
}

impl GitText {
    /// 원시 바이트를 UTF-8 우선으로 디코딩합니다.
    pub fn decode(raw: Vec<u8>) -> GitText {
        match String::from_utf8(raw) {
            Ok(text) => GitText {
                raw: text.as_bytes().to_vec(),
                display: text,
                encoding: TextEncoding::Utf8,
            },
            Err(err) => {
                let raw = err.into_bytes();
                let replaced_sequences = raw
                    .utf8_chunks()
                    .filter(|chunk| !chunk.invalid().is_empty())
                    .count();
                let display = String::from_utf8_lossy(&raw).into_owned();
                GitText {
                    raw,
                    display,
                    encoding: TextEncoding::Lossy { replaced_sequences },
                }
            }
        }
    }

    /// 바이트 slice에서 디코딩합니다.
    pub fn from_bytes(raw: &[u8]) -> GitText {
        GitText::decode(raw.to_vec())
    }

    /// 원시 바이트(정본).
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// 원시 바이트를 꺼냅니다.
    pub fn into_raw(self) -> Vec<u8> {
        self.raw
    }

    /// 유효한 UTF-8일 때만 텍스트를 반환합니다(대체 문자 텍스트는 반환하지 않음).
    pub fn as_utf8(&self) -> Option<&str> {
        match self.encoding {
            TextEncoding::Utf8 => Some(&self.display),
            TextEncoding::Lossy { .. } => None,
        }
    }

    /// 표시용 텍스트(invalid sequence는 U+FFFD).
    pub fn display(&self) -> &str {
        &self.display
    }

    /// 디코딩 상태.
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// 대체 문자가 들어간 표시용 텍스트인지.
    pub fn is_lossy(&self) -> bool {
        matches!(self.encoding, TextEncoding::Lossy { .. })
    }
}

/// `-z` 출력 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZParseError {
    /// 마지막 레코드가 NUL로 끝나지 않음(잘린 출력).
    Unterminated {
        /// 끝나지 않은 레코드의 시작 offset.
        offset: usize,
        /// 끝나지 않은 바이트(보존).
        trailing: Vec<u8>,
    },
}

impl std::fmt::Display for ZParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZParseError::Unterminated { offset, trailing } => write!(
                f,
                "NUL로 끝나지 않은 -z 레코드(offset {offset}, {} bytes)",
                trailing.len()
            ),
        }
    }
}

impl std::error::Error for ZParseError {}

/// `-z` 출력(NUL 종료 레코드)을 레코드 slice로 나눕니다. 빈 입력은 레코드 0개입니다.
///
/// 모든 레코드는 NUL로 끝나야 합니다. 마지막 바이트가 NUL이 아니면 잘린 출력으로 보고
/// [`ZParseError::Unterminated`]를 반환합니다(부분 레코드를 정상 값으로 쓰지 않음).
pub fn split_nul_terminated(bytes: &[u8]) -> Result<Vec<&[u8]>, ZParseError> {
    let mut records = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        match bytes[start..].iter().position(|b| *b == 0) {
            Some(len) => {
                records.push(&bytes[start..start + len]);
                start += len + 1;
            }
            None => {
                return Err(ZParseError::Unterminated {
                    offset: start,
                    trailing: bytes[start..].to_vec(),
                })
            }
        }
    }
    Ok(records)
}

/// Git이 출력한 경로(원시 바이트 보존).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GitPath {
    raw: Vec<u8>,
}

impl GitPath {
    /// 원시 바이트로 경로를 만듭니다.
    pub fn from_bytes(raw: &[u8]) -> GitPath {
        GitPath { raw: raw.to_vec() }
    }

    /// 원시 바이트(정본).
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// UTF-8 우선 디코딩 결과(표시용).
    pub fn text(&self) -> GitText {
        GitText::from_bytes(&self.raw)
    }

    /// 로컬 파일시스템 경로로 변환합니다(unix: 원시 바이트 그대로, Windows: UTF-8일 때만).
    pub fn to_path_buf(&self) -> Option<PathBuf> {
        path_from_git_bytes(&self.raw)
    }
}

/// `-z` 경로 목록 출력을 [`GitPath`] 목록으로 해석합니다.
pub fn parse_nul_paths(bytes: &[u8]) -> Result<Vec<GitPath>, ZParseError> {
    Ok(split_nul_terminated(bytes)?
        .into_iter()
        .map(GitPath::from_bytes)
        .collect())
}

/// Git이 출력한 경로 바이트를 로컬 경로로 변환합니다(unix: 원시 바이트 그대로).
#[cfg(unix)]
pub fn path_from_git_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

/// Git이 출력한 경로 바이트를 로컬 경로로 변환합니다(Windows Git 경로 출력은 UTF-8).
#[cfg(not(unix))]
pub fn path_from_git_bytes(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_is_utf8_and_preserves_raw() {
        let t = GitText::from_bytes("디자인 design.txt\n".as_bytes());
        assert_eq!(t.encoding(), TextEncoding::Utf8);
        assert_eq!(t.as_utf8(), Some("디자인 design.txt\n"));
        assert_eq!(t.display(), "디자인 design.txt\n");
        assert_eq!(t.raw(), "디자인 design.txt\n".as_bytes());
        assert!(!t.is_lossy());
    }

    #[test]
    fn invalid_sequences_are_replaced_counted_and_raw_kept() {
        let raw = b"a\xffb\xfe\xfdc".to_vec();
        let t = GitText::decode(raw.clone());
        // \xff 1개, \xfe·\xfd 각각 1개 → 3 sequence.
        assert_eq!(
            t.encoding(),
            TextEncoding::Lossy {
                replaced_sequences: 3
            }
        );
        assert_eq!(t.display(), "a\u{FFFD}b\u{FFFD}\u{FFFD}c");
        assert_eq!(
            t.as_utf8(),
            None,
            "대체 문자 텍스트를 UTF-8 값으로 주면 안 됨"
        );
        assert_eq!(t.raw(), raw.as_slice());
        assert_eq!(t.into_raw(), raw);

        // 잘린 multi-byte sequence(€ 앞 2바이트)는 1개로 셉니다.
        let t = GitText::from_bytes(b"x\xe2\x82");
        assert_eq!(
            t.encoding(),
            TextEncoding::Lossy {
                replaced_sequences: 1
            }
        );
        assert_eq!(t.display(), "x\u{FFFD}");
    }

    #[test]
    fn nul_terminated_records_split_exactly() {
        assert_eq!(split_nul_terminated(b""), Ok(vec![]));
        assert_eq!(
            split_nul_terminated(b"a\0b c\0"),
            Ok(vec![&b"a"[..], &b"b c"[..]])
        );
        // 개행이 들어간 레코드도 그대로.
        assert_eq!(
            split_nul_terminated(b"line\nbreak\0"),
            Ok(vec![&b"line\nbreak"[..]])
        );
        // 빈 레코드도 레코드입니다.
        assert_eq!(split_nul_terminated(b"\0"), Ok(vec![&b""[..]]));
        assert_eq!(
            split_nul_terminated(b"a\0partial"),
            Err(ZParseError::Unterminated {
                offset: 2,
                trailing: b"partial".to_vec()
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_path_round_trips_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let p = GitPath::from_bytes(b"caf\xe9.txt");
        assert!(p.text().is_lossy());
        assert_eq!(p.text().display(), "caf\u{FFFD}.txt");
        let path = p.to_path_buf().expect("unix는 원시 바이트 경로 허용");
        assert_eq!(path.as_os_str().as_bytes(), b"caf\xe9.txt");
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::command::GitCommandResult;
        use crate::runner::{argv, GitRunner};
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        /// 작업 트리 파일 없이 index에 원시 바이트 경로를 추가합니다.
        ///
        /// macOS APFS는 UTF-8이 아닌 파일 이름을 만들 수 없으므로, blob을 만든 뒤
        /// `update-index --cacheinfo`로 Git 경로만 등록합니다(argv는 원시 바이트 그대로 전달).
        fn add_index_path(runner: &GitRunner, repo: &Path, raw_path: &[u8]) {
            std::fs::write(repo.join("blob-source"), b"contents\n").unwrap();
            let out = git_ok(runner, repo, ["hash-object", "-w", "blob-source"]);
            let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
            let mut cacheinfo = format!("100644,{oid},").into_bytes();
            cacheinfo.extend_from_slice(raw_path);
            let args: Vec<OsString> = vec![
                OsString::from("update-index"),
                OsString::from("--add"),
                OsString::from("--cacheinfo"),
                OsStr::from_bytes(&cacheinfo).to_os_string(),
            ];
            let out = runner.run(&args, repo).unwrap();
            assert!(out.success(), "{}", String::from_utf8_lossy(&out.stderr));
            std::fs::remove_file(repo.join("blob-source")).unwrap();
        }

        #[test]
        fn non_utf8_filename_fixture_is_preserved_through_z_output() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("decode-nonutf8");
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, dir.path());
            let latin1: &[u8] = b"caf\xe9.txt"; // Latin-1 é: UTF-8 아님.
            let korean = "디자인.txt".as_bytes();
            add_index_path(&runner, dir.path(), latin1);
            add_index_path(&runner, dir.path(), korean);
            git_ok(&runner, dir.path(), ["commit", "-q", "-m", "paths"]);

            for args in [
                argv(["ls-files", "-z"]),
                argv(["ls-tree", "-r", "-z", "--name-only", "HEAD"]),
            ] {
                let out = runner.run(&args, dir.path()).unwrap();
                let result = GitCommandResult::from(out).require_success().unwrap();
                let paths = parse_nul_paths(result.stdout()).expect("-z parse");
                let raws: Vec<&[u8]> = paths.iter().map(GitPath::as_bytes).collect();
                // Git 정렬은 바이트 순서: "caf\xe9" < "디"(0xEB…).
                assert_eq!(raws, vec![latin1, korean], "{args:?}");

                let latin = &paths[0];
                assert_eq!(
                    latin.text().encoding(),
                    TextEncoding::Lossy {
                        replaced_sequences: 1
                    }
                );
                assert_eq!(latin.text().display(), "caf\u{FFFD}.txt");
                assert_eq!(
                    latin.to_path_buf().unwrap().as_os_str().as_bytes(),
                    latin1,
                    "원시 바이트 경로 보존"
                );
                assert_eq!(paths[1].text().as_utf8(), Some("디자인.txt"));
            }

            // 대조: -z 없는 출력은 core.quotePath로 quote되어 원시 바이트가 아님 → machine parse 금지.
            let quoted = runner.run(&argv(["ls-files"]), dir.path()).unwrap().stdout;
            assert!(
                quoted.windows(4).any(|w| w == b"\\351"),
                "{:?}",
                String::from_utf8_lossy(&quoted)
            );
            assert!(!quoted.windows(latin1.len()).any(|w| w == latin1));
        }

        #[test]
        fn stderr_is_decoded_utf8_first_with_raw_kept() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("decode-stderr");
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, dir.path());
            let out = runner
                .run(&argv(["rev-parse", "--verify", "no-such-ref"]), dir.path())
                .unwrap();
            let result = GitCommandResult::from(out);
            assert!(!result.succeeded());
            let stderr = result.stderr_text();
            assert_eq!(stderr.encoding(), TextEncoding::Utf8);
            assert_eq!(stderr.raw(), result.stderr());
            assert!(!stderr.display().is_empty());
        }
    }
}
