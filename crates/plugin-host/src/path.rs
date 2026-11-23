//! package 안 상대 경로 (XXI.1.1, XXI.1.2, ADR-0097 결정 2).
//!
//! manifest의 `entry`·`wasm[].path`와 앞으로 WebView scheme handler가 받을 asset 요청 경로는 모두
//! [`PackagePath::parse`] 하나로만 해석합니다. 해석 규칙은 문법만 봅니다(파일 시스템을 읽지 않음). 실제 파일이
//! package 안 일반 파일인지는 [`crate::package`]가 확인합니다.
//!
//! 거부: 빈 경로, 255 bytes 초과, 절대 경로(`/`로 시작), `\`, NUL·제어 문자, `:`(drive·scheme 혼동),
//! 빈 구성 요소(`a//b`, 끝 `/`), `.`·`..` 구성 요소, 32단계 초과 깊이.

use std::fmt;

/// 경로 최대 byte 수.
pub const MAX_PATH_BYTES: usize = 255;

/// 경로 최대 구성 요소 수.
pub const MAX_PATH_DEPTH: usize = 32;

/// 문법을 검증한 package 상대 경로(구분자 `/`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackagePath(String);

/// 경로 문법 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// 빈 경로.
    Empty,
    /// [`MAX_PATH_BYTES`] 초과.
    TooLong,
    /// `/`로 시작하는 절대 경로.
    Absolute,
    /// `..` 구성 요소(package 밖으로 나가는 경로).
    ParentTraversal,
    /// `.` 구성 요소 또는 빈 구성 요소.
    NonCanonical,
    /// `\`, `:`, NUL·제어 문자.
    ForbiddenCharacter,
    /// [`MAX_PATH_DEPTH`] 초과.
    TooDeep,
}

impl PathError {
    /// 고정 진단 code.
    pub fn code(self) -> &'static str {
        match self {
            PathError::Empty => "path-empty",
            PathError::TooLong => "path-too-long",
            PathError::Absolute => "path-absolute",
            PathError::ParentTraversal => "path-traversal",
            PathError::NonCanonical => "path-non-canonical",
            PathError::ForbiddenCharacter => "path-forbidden-character",
            PathError::TooDeep => "path-too-deep",
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl PackagePath {
    /// 문법을 검사합니다.
    pub fn parse(text: &str) -> Result<PackagePath, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if text
            .chars()
            .any(|c| c == '\\' || c == ':' || c.is_control())
        {
            return Err(PathError::ForbiddenCharacter);
        }
        if text.starts_with('/') {
            return Err(PathError::Absolute);
        }
        let mut depth = 0;
        for component in text.split('/') {
            match component {
                ".." => return Err(PathError::ParentTraversal),
                "" | "." => return Err(PathError::NonCanonical),
                _ => depth += 1,
            }
        }
        if depth > MAX_PATH_DEPTH {
            return Err(PathError::TooDeep);
        }
        Ok(PackagePath(text.to_owned()))
    }

    /// 경로 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `/`로 나눈 구성 요소.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// 마지막 구성 요소의 확장자(소문자 비교용 원문).
    pub fn extension(&self) -> Option<&str> {
        let name = self.0.rsplit('/').next()?;
        let (stem, ext) = name.rsplit_once('.')?;
        (!stem.is_empty()).then_some(ext)
    }
}

impl fmt::Display for PackagePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative_paths() {
        let path = PackagePath::parse("ui/index.html").unwrap();
        assert_eq!(path.components().collect::<Vec<_>>(), ["ui", "index.html"]);
        assert_eq!(path.extension(), Some("html"));
        assert_eq!(PackagePath::parse(".hidden").unwrap().extension(), None);
    }

    #[test]
    fn rejects_escape_and_ambiguous_forms() {
        let cases = [
            ("", PathError::Empty),
            ("/etc/passwd", PathError::Absolute),
            ("../x.html", PathError::ParentTraversal),
            ("ui/../../x.html", PathError::ParentTraversal),
            ("ui/./x.html", PathError::NonCanonical),
            ("ui//x.html", PathError::NonCanonical),
            ("ui/", PathError::NonCanonical),
            ("ui\\x.html", PathError::ForbiddenCharacter),
            ("C:/x.html", PathError::ForbiddenCharacter),
            ("a\u{0}b", PathError::ForbiddenCharacter),
        ];
        for (text, expected) in cases {
            assert_eq!(PackagePath::parse(text), Err(expected), "{text:?}");
        }
        assert_eq!(
            PackagePath::parse("a/".repeat(33).trim_end_matches('/')),
            Err(PathError::TooDeep)
        );
        assert_eq!(
            PackagePath::parse(&"a".repeat(256)),
            Err(PathError::TooLong)
        );
    }
}
