//! XXII.1.3 검증: CodegenIR은 단방향입니다. 생성 source를 IR이나 문서로 되읽는 parser·import API가 없는지
//! 공개 API inventory로 확인합니다(기술 아키텍처 §17.3, ADR-0025 결정 4.5, ADR-0106 결정 9).
//!
//! 검사 범위는 `stellar-codegen`의 모든 source와 ops의 코드 생성 module(`crates/ops/src/codegen*`,
//! `crates/ops/src/export_web*`)입니다.

use std::path::{Path, PathBuf};

/// 공개 item 이름에 들어가면 되읽기 API로 보는 낱말.
const IMPORT_WORDS: &[&str] = &[
    "parse",
    "import",
    "decode",
    "deserialize",
    "from_html",
    "from_css",
    "from_js",
    "from_swift",
    "from_kotlin",
    "from_source",
    "from_generated",
    "read_generated",
    "round_trip",
    "reverse",
    "ingest",
];

/// 되읽기에 쓰이는 trait 구현.
const IMPORT_IMPLS: &[&str] = &[
    "impl FromStr",
    "impl std::str::FromStr",
    "impl TryFrom<&str>",
    "impl TryFrom<String>",
    "impl<'de> Deserialize",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let ops = root.join("../ops/src");
    let mut ops_files = Vec::new();
    rust_files(&ops, &mut ops_files);
    files.extend(ops_files.into_iter().filter(|path| {
        let relative = path.strip_prefix(&ops).unwrap_or(path);
        let first = relative
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        first.starts_with("codegen") || first.starts_with("export_web")
    }));
    files
}

/// `pub fn`·`pub trait`·`pub struct`·`pub enum`·`pub mod` 이름(test module 밖).
fn public_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("#[cfg(test)]") {
            // 파일 끝의 test module은 공개 API가 아닙니다.
            break;
        }
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        let rest = rest
            .trim_start_matches("const ")
            .trim_start_matches("async ");
        for keyword in ["fn ", "trait ", "struct ", "enum ", "mod ", "type "] {
            if let Some(name) = rest.strip_prefix(keyword) {
                let name: String = name
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                items.push(name.to_lowercase());
            }
        }
    }
    items
}

#[test]
fn api_inventory_has_no_generated_source_import() {
    let files = sources();
    assert!(
        files.len() >= 20,
        "codegen source를 찾지 못했습니다: {}",
        files.len()
    );
    let mut items = 0;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("source");
        for name in public_items(&text) {
            items += 1;
            for word in IMPORT_WORDS {
                assert!(
                    !name.contains(word),
                    "{}: 공개 item `{name}`이 되읽기 API처럼 보입니다({word})",
                    path.display()
                );
            }
        }
        for pattern in IMPORT_IMPLS {
            assert!(
                !text.contains(pattern),
                "{}: 되읽기 trait 구현 `{pattern}`",
                path.display()
            );
        }
    }
    assert!(items > 100, "공개 item inventory가 너무 작습니다: {items}");
}

#[test]
fn inventory_detects_an_import_api() {
    // 검사기가 실제로 잡는지 확인합니다.
    let names = public_items("pub fn parse_generated_html(text: &str) {}\npub struct Ir;\n");
    assert!(names.iter().any(|n| n.contains("parse")));
    let names = public_items("#[cfg(test)]\nmod tests {\n pub fn parse() {}\n}\n");
    assert!(names.is_empty());
}
