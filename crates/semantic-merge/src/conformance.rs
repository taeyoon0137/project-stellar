//! Semantic Diff/Merge conformance corpus runner (XXVIII.2.3, ADR-0081 결정 10).
//!
//! corpus는 폴더 하나이고 사례마다 세 파일입니다(모두 16진수 텍스트, 공백·줄바꿈 무시).
//!
//! | 파일 | 내용 |
//! |---|---|
//! | `<이름>.input.hex` | [`MergeInput`] canonical bytes(base·ours·theirs snapshot) |
//! | `<이름>.diff.hex` | 기대 base → ours [`SemanticDiff`] canonical bytes |
//! | `<이름>.result.hex` | 기대 [`MergeResult`] canonical bytes |
//!
//! 구현은 [`Implementation`]으로 받습니다. 판정은 bytes 단위 일치이고, merge가 오류를 내면 실패입니다(corpus의 모든
//! 사례는 공식 구현이 결과를 내는 입력입니다). 사례 목록은 이름 순서이며 세 파일 중 하나라도 빠진 사례는 corpus 오류입니다.
//! 공식 구현은 [`official`]입니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::snapshot::SemanticSnapshot;

use crate::merge::merge;
use crate::model::MergeInput;

/// conformance 대상 구현.
pub trait Implementation {
    /// base → target semantic diff canonical bytes.
    fn diff(&self, base: &SemanticSnapshot, target: &SemanticSnapshot) -> Vec<u8>;
    /// 3-way merge 결과 canonical bytes. 실패는 설명 문자열입니다.
    fn merge(&self, input: &MergeInput) -> Result<Vec<u8>, String>;
}

/// 공식 구현(이 crate의 semantic-diff·merge 엔진).
#[derive(Debug, Clone, Copy, Default)]
pub struct Official;

impl Implementation for Official {
    fn diff(&self, base: &SemanticSnapshot, target: &SemanticSnapshot) -> Vec<u8> {
        diff(base, target).to_canonical_bytes()
    }

    fn merge(&self, input: &MergeInput) -> Result<Vec<u8>, String> {
        merge(input)
            .map(|result| result.to_canonical_bytes())
            .map_err(|e| e.to_string())
    }
}

/// 공식 구현.
pub fn official() -> Official {
    Official
}

/// corpus 사례 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// 이름(파일 접두사).
    pub name: String,
    /// 입력.
    pub input: MergeInput,
    /// 기대 base → ours diff bytes.
    pub diff: Vec<u8>,
    /// 기대 merge 결과 bytes.
    pub result: Vec<u8>,
}

/// corpus 읽기 오류.
#[derive(Debug)]
pub enum CorpusError {
    /// 폴더·파일 입출력.
    Io(PathBuf, std::io::Error),
    /// 16진수 형식.
    Hex(PathBuf),
    /// 입력 bytes 형식.
    Input(PathBuf, String),
    /// 세 파일 중 일부가 없음.
    Incomplete(String),
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CorpusError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            CorpusError::Hex(path) => write!(f, "{}: 16진수 형식", path.display()),
            CorpusError::Input(path, e) => write!(f, "{}: 입력 형식: {e}", path.display()),
            CorpusError::Incomplete(name) => write!(f, "사례 {name}: 세 파일이 모두 필요합니다"),
        }
    }
}

impl std::error::Error for CorpusError {}

/// bytes를 64자 줄의 16진수 텍스트로 씁니다(corpus 파일 형식).
pub fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + bytes.len() / 32 + 1);
    for chunk in bytes.chunks(32) {
        for byte in chunk {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push('\n');
    }
    out
}

/// 16진수 텍스트(공백·줄바꿈 무시)를 bytes로 읽습니다.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let hex = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(hex, 16).ok()
        })
        .collect()
}

const SUFFIXES: [&str; 3] = [".input.hex", ".diff.hex", ".result.hex"];

/// corpus 폴더를 읽습니다(이름 순서).
pub fn load_corpus(dir: &Path) -> Result<Vec<Case>, CorpusError> {
    let mut files: BTreeMap<String, [Option<PathBuf>; 3]> = BTreeMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| CorpusError::Io(dir.to_path_buf(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| CorpusError::Io(dir.to_path_buf(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        for (slot, suffix) in SUFFIXES.iter().enumerate() {
            if let Some(stem) = name.strip_suffix(suffix) {
                files.entry(stem.to_owned()).or_default()[slot] = Some(entry.path());
            }
        }
    }
    let read = |path: &Path| -> Result<Vec<u8>, CorpusError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| CorpusError::Io(path.to_path_buf(), e))?;
        from_hex(&text).ok_or_else(|| CorpusError::Hex(path.to_path_buf()))
    };
    let mut cases = Vec::new();
    for (name, [input, diff, result]) in files {
        let (Some(input), Some(diff), Some(result)) = (input, diff, result) else {
            return Err(CorpusError::Incomplete(name));
        };
        let bytes = read(&input)?;
        let decoded = MergeInput::from_canonical_bytes(&bytes)
            .map_err(|e| CorpusError::Input(input.clone(), e.to_string()))?;
        cases.push(Case {
            name,
            input: decoded,
            diff: read(&diff)?,
            result: read(&result)?,
        });
    }
    Ok(cases)
}

/// 사례 하나의 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// diff·merge bytes가 모두 기대와 같음.
    Pass,
    /// diff bytes가 다름.
    DiffMismatch,
    /// merge 결과 bytes가 다름.
    ResultMismatch,
    /// merge가 오류를 냄.
    MergeFailed(String),
}

/// corpus 실행 보고.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// 사례별 판정(이름 순서).
    pub verdicts: Vec<(String, Verdict)>,
}

impl Report {
    /// 모든 사례가 통과했는지(사례가 하나 이상일 때).
    pub fn passed(&self) -> bool {
        !self.verdicts.is_empty() && self.verdicts.iter().all(|(_, v)| *v == Verdict::Pass)
    }

    /// 실패한 사례.
    pub fn failures(&self) -> Vec<&(String, Verdict)> {
        self.verdicts
            .iter()
            .filter(|(_, v)| *v != Verdict::Pass)
            .collect()
    }
}

/// 사례 하나를 판정합니다.
pub fn run_case(case: &Case, implementation: &dyn Implementation) -> Verdict {
    if implementation.diff(&case.input.base, &case.input.ours) != case.diff {
        return Verdict::DiffMismatch;
    }
    match implementation.merge(&case.input) {
        Ok(bytes) if bytes == case.result => Verdict::Pass,
        Ok(_) => Verdict::ResultMismatch,
        Err(e) => Verdict::MergeFailed(e),
    }
}

/// 사례 목록을 판정합니다.
pub fn run_cases(cases: &[Case], implementation: &dyn Implementation) -> Report {
    Report {
        verdicts: cases
            .iter()
            .map(|case| (case.name.clone(), run_case(case, implementation)))
            .collect(),
    }
}

/// corpus 폴더를 읽어 판정합니다.
pub fn run_corpus(dir: &Path, implementation: &dyn Implementation) -> Result<Report, CorpusError> {
    Ok(run_cases(&load_corpus(dir)?, implementation))
}

/// 공식 구현으로 사례의 기대 bytes를 만듭니다(corpus 생성용).
pub fn expected_case(name: &str, input: MergeInput) -> Result<Case, String> {
    let official = official();
    let diff = official.diff(&input.base, &input.ours);
    let result = official.merge(&input)?;
    Ok(Case {
        name: name.to_owned(),
        input,
        diff,
        result,
    })
}

/// 사례를 corpus 폴더에 씁니다(세 파일).
pub fn write_case(dir: &Path, case: &Case) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let contents = [
        to_hex(&case.input.to_canonical_bytes()),
        to_hex(&case.diff),
        to_hex(&case.result),
    ];
    for (suffix, text) in SUFFIXES.iter().zip(contents) {
        std::fs::write(dir.join(format!("{}{suffix}", case.name)), text)?;
    }
    Ok(())
}
