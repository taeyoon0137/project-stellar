//! chunk object 경로 생성 (V.2.3).
//!
//! content hash에서 `design/objects/<prefix>/<hash>.starchunk` 상대 경로를 만듭니다.
//! 입력은 [`ContentHash`] 타입으로만 받아 path traversal 문자열이 타입상 불가능합니다.
//! 경로 문자는 소문자 16진수·알고리즘 이름(`[a-z0-9]`)·`-`·`.`뿐이라 모든 플랫폼에서
//! 안전합니다(Windows 금지 문자 `:`는 파일명에서 `-`로 대체).

use std::fmt;
use std::path::PathBuf;

use crate::hash::ContentHash;

/// chunk object 디렉터리(프로젝트 root 기준) 고정 구성 요소.
const OBJECTS_DIR: [&str; 2] = ["design", "objects"];

/// chunk object 파일 확장자.
const CHUNK_EXTENSION: &str = "starchunk";

/// content hash로부터 만들어진 chunk object 상대 경로.
///
/// [`ContentHash`]에서만 생성할 수 있습니다. 임의 문자열 입력 경로가 없으므로
/// path traversal(`..`, 구분자 삽입)이 타입상 불가능합니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkObjectPath {
    /// shard prefix — digest 소문자 hex 앞 2자.
    prefix: String,
    /// 파일 이름 — `<algorithm>-<hex64>.starchunk`.
    file_name: String,
}

impl ChunkObjectPath {
    /// content hash에서 경로를 만듭니다.
    pub fn for_hash(hash: &ContentHash) -> ChunkObjectPath {
        let hex = hash.digest_hex();
        ChunkObjectPath {
            prefix: hex[..2].to_string(),
            file_name: format!("{}-{}.{}", hash.algorithm().name(), hex, CHUNK_EXTENSION),
        }
    }

    /// shard prefix(digest hex 앞 2자).
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// 파일 이름.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// 프로젝트 root 기준 상대 경로(`design/objects/<prefix>/<file>`)를
    /// 플랫폼 구분자로 만듭니다.
    pub fn to_relative_path(&self) -> PathBuf {
        let mut path = PathBuf::new();
        for dir in OBJECTS_DIR {
            path.push(dir);
        }
        path.push(&self.prefix);
        path.push(&self.file_name);
        path
    }
}

impl fmt::Display for ChunkObjectPath {
    /// `/` 구분자의 정규 표기(로그·manifest 표시용).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            OBJECTS_DIR[0], OBJECTS_DIR[1], self.prefix, self.file_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_path_for_known_hash() {
        // sha256("test") = 9f86d081... (FIPS vector).
        let hash = ContentHash::compute(b"test");
        let path = ChunkObjectPath::for_hash(&hash);
        assert_eq!(
            path.to_string(),
            "design/objects/9f/sha256-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08.starchunk"
        );
        assert_eq!(path.prefix(), "9f");
    }

    #[test]
    fn all_hashes_make_fixed_length_safe_paths() {
        // 검증 기준(V.2.3): 모든 hash가 고정 길이 안전 경로를 만듭니다.
        let expected_len = ChunkObjectPath::for_hash(&ContentHash::compute(b""))
            .to_string()
            .len();
        for i in 0..1_000u32 {
            let hash = ContentHash::compute(&i.to_le_bytes());
            let path = ChunkObjectPath::for_hash(&hash);
            let text = path.to_string();
            assert_eq!(text.len(), expected_len, "고정 길이 위반: {text}");
            // 경로 구성 요소는 [a-z0-9.-]와 구분자 '/'만 포함해야 합니다.
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "/.-".contains(c)),
                "안전하지 않은 문자: {text}"
            );
            assert!(!text.contains(".."), "traversal 흔적: {text}");
            assert_eq!(&hash.digest_hex()[..2], path.prefix());
        }
    }

    #[test]
    fn relative_path_has_expected_components() {
        let hash = ContentHash::compute(b"component check");
        let path = ChunkObjectPath::for_hash(&hash).to_relative_path();
        let components: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(components.len(), 4);
        assert_eq!(components[0], "design");
        assert_eq!(components[1], "objects");
        assert_eq!(components[2].len(), 2);
        assert!(components[3].starts_with("sha256-"));
        assert!(components[3].ends_with(".starchunk"));
    }

    #[test]
    fn windows_forbidden_characters_are_absent() {
        let hash = ContentHash::compute(b"windows");
        let text = ChunkObjectPath::for_hash(&hash).to_string();
        for forbidden in ['<', '>', ':', '"', '\\', '|', '?', '*'] {
            assert!(
                !text.contains(forbidden),
                "Windows 금지 문자 {forbidden:?} 포함: {text}"
            );
        }
    }
}
