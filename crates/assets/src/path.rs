//! 에셋 content 파일 경로 (XIII.5.2).
//!
//! content hash와 안전한 확장자에서 `assets/<prefix>/<algorithm>-<hex64>.<safe-ext>` 상대
//! 경로를 만듭니다. chunk object 경로(V.2.3 `design/objects/<prefix>/sha256-<hex>.starchunk`)와
//! 같은 규칙입니다.
//! - `<prefix>`: digest 소문자 hex 앞 2자(shard).
//! - `<hash>`: ADR-0004 문자열의 `:`를 `-`로 바꾼 형태(Windows 금지 문자 회피).
//! - `<safe-ext>`: `[a-z0-9]` 1~16자. 원본 확장자가 이 규칙을 벗어나면 `bin`.
//!
//! 입력은 [`ContentHash`]와 검증된 [`SafeExtension`]으로만 받으므로 path traversal
//! (`..`, 구분자 삽입)이 타입상 불가능하고, 모든 경로 문자는 모든 플랫폼에서 안전합니다.

use std::fmt;
use std::path::{Path, PathBuf};

use stellar_storage::hash::ContentHash;

/// 에셋 content 디렉터리(프로젝트 root 기준).
pub const ASSETS_DIR: &str = "assets";

/// 안전한 확장자 최대 길이.
pub const MAX_EXTENSION_LEN: usize = 16;

/// unknown·비안전 확장자의 대체 값.
pub const BINARY_EXTENSION: &str = "bin";

/// `assets/` 또는 shard 디렉터리 상태(symlink를 따라가지 않고 판정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetsDirState {
    /// 없습니다.
    Missing,
    /// 실제 디렉터리입니다.
    Directory,
    /// symlink입니다(대상과 무관하게 따라가지 않음 — Git으로 commit된 symlink가 쓰기·삭제를
    /// 프로젝트 밖으로 돌리지 못하게 함).
    Symlink,
    /// 디렉터리가 아닌 다른 항목입니다.
    NotDirectory,
}

/// `lstat`으로 디렉터리 상태를 봅니다.
pub(crate) fn assets_dir_state(path: &Path) -> std::io::Result<AssetsDirState> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(AssetsDirState::Symlink),
        Ok(metadata) if metadata.is_dir() => Ok(AssetsDirState::Directory),
        Ok(_) => Ok(AssetsDirState::NotDirectory),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AssetsDirState::Missing),
        Err(e) => Err(e),
    }
}

/// 확장자 형식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionError {
    /// 비어 있습니다.
    Empty,
    /// 최대 길이를 초과합니다.
    TooLong {
        /// 실제 길이(byte).
        len: usize,
    },
    /// `[A-Za-z0-9]` 밖의 문자입니다.
    InvalidCharacter {
        /// 위반 문자.
        found: char,
    },
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtensionError::Empty => f.write_str("확장자가 비어 있습니다"),
            ExtensionError::TooLong { len } => {
                write!(f, "확장자 길이가 {MAX_EXTENSION_LEN}자를 초과합니다: {len}")
            }
            ExtensionError::InvalidCharacter { found } => {
                write!(f, "확장자에는 영문자·숫자만 허용됩니다: {found:?}")
            }
        }
    }
}

impl std::error::Error for ExtensionError {}

/// 검증된 안전 확장자(`[a-z0-9]` 1~16자, ASCII 소문자 canonical).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeExtension(String);

impl SafeExtension {
    /// 확장자 문자열을 검증하고 ASCII 소문자로 정규화합니다(앞의 `.`는 받지 않습니다).
    pub fn new(text: &str) -> Result<SafeExtension, ExtensionError> {
        if text.is_empty() {
            return Err(ExtensionError::Empty);
        }
        if let Some(found) = text.chars().find(|c| !c.is_ascii_alphanumeric()) {
            return Err(ExtensionError::InvalidCharacter { found });
        }
        if text.len() > MAX_EXTENSION_LEN {
            return Err(ExtensionError::TooLong { len: text.len() });
        }
        Ok(SafeExtension(text.to_ascii_lowercase()))
    }

    /// 대체 확장자 `bin`.
    pub fn binary() -> SafeExtension {
        SafeExtension(BINARY_EXTENSION.to_string())
    }

    /// 원본 경로의 확장자를 안전 확장자로 정규화합니다. 없거나 비안전하면 `bin`.
    pub fn from_source_path(path: &Path) -> SafeExtension {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| SafeExtension::new(ext).ok())
            .unwrap_or_else(SafeExtension::binary)
    }

    /// canonical 문자열(소문자).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// content hash로부터 만들어진 에셋 content 파일 상대 경로.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetObjectPath {
    /// shard prefix — digest 소문자 hex 앞 2자.
    prefix: String,
    /// 파일 이름 — `<algorithm>-<hex64>.<safe-ext>`.
    file_name: String,
}

impl AssetObjectPath {
    /// content hash와 확장자에서 경로를 만듭니다.
    pub fn new(hash: &ContentHash, extension: &SafeExtension) -> AssetObjectPath {
        let hex = hash.digest_hex();
        AssetObjectPath {
            prefix: hex[..2].to_string(),
            file_name: format!("{}-{}.{}", hash.algorithm().name(), hex, extension),
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

    /// 프로젝트 root 기준 shard 디렉터리(`assets/<prefix>`)를 플랫폼 구분자로 만듭니다.
    pub fn relative_dir(&self) -> PathBuf {
        Path::new(ASSETS_DIR).join(&self.prefix)
    }

    /// content hash가 들어갈 shard 디렉터리(`assets/<prefix>`, 확장자 무관).
    pub fn shard_dir_for(hash: &ContentHash) -> PathBuf {
        Path::new(ASSETS_DIR).join(&hash.digest_hex()[..2])
    }

    /// 프로젝트 root 기준 상대 경로(`assets/<prefix>/<file>`)를 플랫폼 구분자로 만듭니다.
    pub fn to_relative_path(&self) -> PathBuf {
        self.relative_dir().join(&self.file_name)
    }

    /// shard 디렉터리 이름과 파일 이름이 content 파일 규칙에 **정확히**(canonical) 맞으면
    /// 경로와 content hash를 복원합니다(XIII.5.3 dedup 조회·XIII.5.6 GC 용).
    ///
    /// temp 파일(`.`로 시작), 대문자 확장자, prefix 불일치, 알 수 없는 알고리즘 등은 `None`.
    pub fn parse(prefix: &str, file_name: &str) -> Option<(AssetObjectPath, ContentHash)> {
        let (stem, ext) = file_name.rsplit_once('.')?;
        let extension = SafeExtension::new(ext).ok()?;
        let (algorithm, hex) = stem.split_once('-')?;
        let hash: ContentHash = format!("{algorithm}:{hex}").parse().ok()?;
        let path = AssetObjectPath::new(&hash, &extension);
        (path.prefix == prefix && path.file_name == file_name).then_some((path, hash))
    }

    /// 같은 content hash인지(확장자 무관).
    pub fn has_hash(&self, hash: &ContentHash) -> bool {
        AssetObjectPath::parse(&self.prefix, &self.file_name).is_some_and(|(_, h)| h == *hash)
    }
}

impl fmt::Display for AssetObjectPath {
    /// `/` 구분자의 정규 표기(로그·진단 표시용).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{ASSETS_DIR}/{}/{}", self.prefix, self.file_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_path_for_known_hash() {
        // SHA-256("abc")
        let hash = ContentHash::compute(b"abc");
        let path = AssetObjectPath::new(&hash, &SafeExtension::new("PNG").unwrap());
        assert_eq!(path.prefix(), "ba");
        assert_eq!(
            path.file_name(),
            "sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.png"
        );
        assert_eq!(
            path.to_string(),
            "assets/ba/sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.png"
        );
        assert_eq!(
            path.to_relative_path(),
            Path::new("assets").join("ba").join(path.file_name())
        );
        assert_eq!(path.relative_dir(), Path::new("assets").join("ba"));
    }

    #[test]
    fn every_path_has_fixed_safe_shape() {
        for i in 0..64u32 {
            let hash = ContentHash::compute(&i.to_le_bytes());
            let path = AssetObjectPath::new(&hash, &SafeExtension::new("woff2").unwrap());
            assert_eq!(path.prefix().len(), 2);
            assert_eq!(
                path.file_name().len(),
                "sha256-".len() + 64 + ".woff2".len()
            );
            assert!(path
                .to_string()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "/-.".contains(c)));
        }
    }

    #[test]
    fn extension_validation() {
        assert_eq!(SafeExtension::new("Svg").unwrap().as_str(), "svg");
        assert_eq!(SafeExtension::new("").unwrap_err(), ExtensionError::Empty);
        assert_eq!(
            SafeExtension::new("tar.gz").unwrap_err(),
            ExtensionError::InvalidCharacter { found: '.' }
        );
        for bad in ["a/b", "..", "a\\b", "é", "a b", "a:b"] {
            assert!(SafeExtension::new(bad).is_err(), "{bad:?}");
        }
        let max = "a".repeat(MAX_EXTENSION_LEN);
        assert!(SafeExtension::new(&max).is_ok());
        assert_eq!(
            SafeExtension::new(&format!("{max}a")).unwrap_err(),
            ExtensionError::TooLong { len: 17 }
        );
    }

    #[test]
    fn source_extension_is_sanitized_or_binary() {
        for (path, expected) in [
            ("/x/logo.PNG", "png"),
            ("/x/font.woff2", "woff2"),
            ("/x/archive.tar.gz", "gz"),
            ("/x/noext", "bin"),
            ("/x/.hidden", "bin"),
            ("/x/weird.p n g", "bin"),
            ("/x/로고.이미지", "bin"),
            ("/x/long.abcdefghijklmnopq", "bin"),
        ] {
            assert_eq!(
                SafeExtension::from_source_path(Path::new(path)).as_str(),
                expected,
                "{path}"
            );
        }
    }

    #[test]
    fn parse_round_trips_canonical_names_only() {
        let hash = ContentHash::compute(b"abc");
        let path = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        assert_eq!(
            AssetObjectPath::parse(path.prefix(), path.file_name()),
            Some((path.clone(), hash))
        );
        assert!(path.has_hash(&hash));
        assert!(!path.has_hash(&ContentHash::compute(b"abd")));

        let hex = hash.digest_hex();
        for (prefix, name) in [
            ("bb", path.file_name().to_string()),         // prefix 불일치
            ("ba", format!("sha256-{hex}.PNG")),          // 비canonical 확장자
            ("ba", format!("sha256-{hex}")),              // 확장자 없음
            ("ba", format!(".sha256-{hex}.png.tmp-1-0")), // temp 파일
            ("ba", format!("md5-{hex}.png")),             // 알 수 없는 알고리즘
            ("ba", format!("sha256-{}.png", &hex[..63])), // 짧은 digest
            ("ba", format!("sha256-{}.png", hex.to_uppercase())), // 대문자 hex
            ("ba", "readme.txt".to_string()),
        ] {
            assert_eq!(AssetObjectPath::parse(prefix, &name), None, "{name}");
        }
    }
}
