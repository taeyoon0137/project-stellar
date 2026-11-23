//! 생성 파일 집합과 고정 formatting (XXII.5.1, ADR-0106 결정 8).
//!
//! - 파일은 상대 경로 byte 오름차순으로 둡니다([`GeneratedFiles`]는 `BTreeMap`).
//! - 내용은 UTF-8, LF, 끝 줄바꿈 하나입니다. 빈 내용은 파일을 만들지 않습니다.
//! - 파일마다 첫 줄에 생성 marker 주석을 씁니다(형식별 주석 문법). 사용자 파일과 구별하고, 생성 코드를 다시 가져오지
//!   않는다는 경고를 담습니다(XXII.5.3 문서 경고와 같은 문구).
//! - 경로는 `/`로 나눈 상대 경로이고 `..`·빈 조각·절대 경로를 받지 않습니다.
//! - Web(HTML·CSS·JavaScript), Apple(Swift), Android(Kotlin) 출력이 모두 이 집합을 씁니다(ADR-0121 결정 5). 파일
//!   이름은 adapter가 검사한 ASCII 식별자(타입 접두어, Kotlin package 조각)로만 만듭니다.

use std::collections::BTreeMap;

/// 생성 marker 문구.
pub const GENERATED_NOTICE: &str =
    "stellar:generated 이 파일은 Stellar가 생성합니다. 직접 고친 내용은 다음 생성 때 사라지고 Stellar로 가져오지 않습니다.";

/// 파일 형식(주석 문법).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// HTML(`<!-- -->`).
    Html,
    /// CSS(`/* */`).
    Css,
    /// JavaScript(`//`).
    JavaScript,
    /// Swift(`//`).
    Swift,
    /// Kotlin(`//`).
    Kotlin,
}

impl FileKind {
    /// 경로 확장자의 형식.
    pub fn of(path: &str) -> Option<FileKind> {
        Some(match path.rsplit('.').next()? {
            "html" => FileKind::Html,
            "css" => FileKind::Css,
            "js" | "mjs" => FileKind::JavaScript,
            "swift" => FileKind::Swift,
            "kt" => FileKind::Kotlin,
            _ => return None,
        })
    }

    /// marker 주석 한 줄.
    pub fn marker(self) -> String {
        match self {
            FileKind::Html => format!("<!-- {GENERATED_NOTICE} -->"),
            FileKind::Css => format!("/* {GENERATED_NOTICE} */"),
            FileKind::JavaScript | FileKind::Swift | FileKind::Kotlin => {
                format!("// {GENERATED_NOTICE}")
            }
        }
    }
}

/// 파일 집합 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputError {
    /// 경로가 상대 경로 규칙을 어깁니다.
    InvalidPath(String),
    /// 형식을 모르는 확장자입니다.
    UnknownKind(String),
    /// 같은 경로가 두 번 있습니다.
    DuplicatePath(String),
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputError::InvalidPath(p) => write!(f, "생성 파일 경로가 잘못되었습니다: {p}"),
            OutputError::UnknownKind(p) => write!(f, "생성 파일 형식을 모릅니다: {p}"),
            OutputError::DuplicatePath(p) => write!(f, "생성 파일 경로가 두 번 있습니다: {p}"),
        }
    }
}

impl std::error::Error for OutputError {}

/// 상대 경로 규칙: 비지 않은 조각을 `/`로 이은 경로, `.`·`..`·`\`·NUL 없음, 절대 경로 아님.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// 내용 formatting: CRLF·CR을 LF로, 끝 줄바꿈 하나. 줄 안 공백은 바꾸지 않습니다(HTML 텍스트 보존).
pub fn normalize(content: &str) -> String {
    let unified = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = unified.trim_end_matches('\n').to_string();
    out.push('\n');
    out
}

/// 생성 파일 집합(경로 byte 오름차순).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneratedFiles {
    files: BTreeMap<String, String>,
}

impl GeneratedFiles {
    /// 빈 집합.
    pub fn new() -> GeneratedFiles {
        GeneratedFiles::default()
    }

    /// 파일을 더합니다. 내용이 비면(공백만이면) 만들지 않습니다. marker 줄을 앞에 붙이고 formatting합니다.
    pub fn insert(&mut self, path: &str, content: &str) -> Result<(), OutputError> {
        if !valid_path(path) {
            return Err(OutputError::InvalidPath(path.to_string()));
        }
        let kind = FileKind::of(path).ok_or_else(|| OutputError::UnknownKind(path.to_string()))?;
        if content.trim().is_empty() {
            return Ok(());
        }
        if self.files.contains_key(path) {
            return Err(OutputError::DuplicatePath(path.to_string()));
        }
        let body = normalize(content);
        let text = match kind {
            // HTML은 doctype이 첫 줄이어야 하므로 marker를 그 다음에 둡니다.
            FileKind::Html if body.starts_with("<!doctype html>") => body.replacen(
                "<!doctype html>\n",
                &format!("<!doctype html>\n{}\n", kind.marker()),
                1,
            ),
            _ => format!("{}\n{body}", kind.marker()),
        };
        self.files.insert(path.to_string(), text);
        Ok(())
    }

    /// (경로, 내용) 목록(경로 순).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.files.iter().map(|(p, c)| (p.as_str(), c.as_str()))
    }

    /// 경로의 내용.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// 파일 수.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// 빈 집합인지.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 전체 집합의 결정론 직렬화(golden·byte 비교용): 파일마다 `=== <경로>` 줄과 내용.
    pub fn manifest_text(&self) -> String {
        let mut out = String::new();
        for (path, content) in &self.files {
            out.push_str("=== ");
            out.push_str(path);
            out.push('\n');
            out.push_str(content);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_formatting_and_order_are_fixed() {
        let mut files = GeneratedFiles::new();
        files
            .insert("styles.css", "a {\r\n  b: c;\r\n}\n\n\n")
            .expect("css");
        files
            .insert("index.html", "<!doctype html>\n<p>x</p>")
            .expect("html");
        files.insert("empty.js", "  \n").expect("empty");
        let paths: Vec<&str> = files.iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["index.html", "styles.css"]);
        assert_eq!(
            files.get("styles.css"),
            Some(format!("{}\na {{\n  b: c;\n}}\n", FileKind::Css.marker()).as_str())
        );
        assert!(files
            .get("index.html")
            .unwrap()
            .starts_with("<!doctype html>\n<!-- stellar:generated"));
        for bad in [
            "",
            "/abs.css",
            "../x.css",
            "a//b.css",
            "a/./b.css",
            "a\\b.css",
        ] {
            assert!(files.insert(bad, "x").is_err(), "{bad}");
        }
        files
            .insert("Sources/StellarView.swift", "import SwiftUI\r\n")
            .expect("swift");
        assert_eq!(
            files.get("Sources/StellarView.swift"),
            Some(format!("{}\nimport SwiftUI\n", FileKind::Swift.marker()).as_str())
        );
        assert_eq!(FileKind::of("a/B.kt"), Some(FileKind::Kotlin));
        assert!(FileKind::Kotlin
            .marker()
            .starts_with("// stellar:generated"));
        assert_eq!(
            files.insert("data.bin", "x"),
            Err(OutputError::UnknownKind("data.bin".into()))
        );
        assert_eq!(
            files.insert("styles.css", "x"),
            Err(OutputError::DuplicatePath("styles.css".into()))
        );
    }
}
