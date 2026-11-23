//! 소스 inventory 도구(XXIX.6.5·XXIX.6.6). 주석과 `#[cfg(test)]` 뒤 단위 테스트를 뺀 코드 줄에서 낱말을 찾습니다.

use std::path::{Path, PathBuf};

/// 저장소 root.
pub fn repo_dir() -> PathBuf {
    std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

/// `dir` 아래 소스 파일(rs·swift·h·m·c), 정렬 순. `tests` 디렉터리는 뺍니다(제품 경로만).
pub fn source_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "tests" && name != "target" && name != "fixtures" {
                out.extend(source_files(&path));
            }
        } else if path
            .extension()
            .is_some_and(|e| e == "rs" || e == "swift" || e == "h" || e == "m" || e == "c")
        {
            let stem = path.file_stem().unwrap().to_string_lossy();
            if !stem.ends_with("_tests") && stem != "tests" {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 코드 줄(1부터 줄 번호, 주석 제거).
pub fn code_lines(path: &Path) -> Vec<(usize, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .take_while(|line| line.trim() != "#[cfg(test)]")
        .enumerate()
        .map(|(n, line)| {
            let code = match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            };
            (n + 1, code.to_string())
        })
        .filter(|(_, l)| !l.trim().is_empty())
        .collect()
}

/// `files`의 코드 줄에서 `words`가 나온 곳(`경로:줄: 낱말`).
pub fn hits(files: &[PathBuf], words: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        for (n, line) in code_lines(file) {
            for word in words {
                if line.contains(word) {
                    out.push(format!("{}:{n}: {word}", file.display()));
                }
            }
        }
    }
    out
}

/// 여러 base(저장소 기준 상대 경로) 아래 소스 파일.
pub fn sources_in(bases: &[&str]) -> Vec<PathBuf> {
    let root = repo_dir();
    bases
        .iter()
        .flat_map(|b| source_files(&root.join(b)))
        .collect()
}
