//! 생성 파일 쓰기: 임시 디렉터리 생성 후 교체, 또는 generated marker 구간만 갱신 (XXII.5.2, ADR-0106 결정 9).
//!
//! 사용자 source를 임의로 덮어쓰지 않습니다.
//!
//! 1. **디렉터리 교체**([`replace_directory`]): 출력 디렉터리가 없거나 비었거나, 생성 manifest([`MANIFEST`])가 있고
//!    그 목록 밖 파일이 없을 때만 씁니다. 같은 부모 아래 임시 디렉터리에 전부 쓰고 sync한 뒤 rename으로 교체합니다.
//!    기존 디렉터리는 먼저 백업 이름으로 rename하고, 교체가 실패하면 되돌립니다. 목록 밖 사용자 파일이나 symlink가
//!    있으면 아무것도 바꾸지 않고 거부합니다.
//! 2. **marker 구간 갱신**([`update_marked_regions`]): 이미 있는 파일 안의 `stellar:generated:begin <이름>`과
//!    `stellar:generated:end <이름>` 주석 줄 사이만 바꿉니다. marker가 없거나 짝이 맞지 않거나 겹치면 거부합니다. 구간
//!    밖 byte는 그대로이고, 같은 디렉터리의 임시 파일에 쓴 뒤 rename으로 교체합니다.
//!
//! marker 판정은 사용자 파일을 보존하기 위한 위치 판정이며 생성 코드의 의미를 해석하지 않습니다(XXII.1.3).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_codegen::output::{normalize, valid_path, GeneratedFiles, GENERATED_NOTICE};

/// 생성 manifest 파일 이름.
pub const MANIFEST: &str = ".stellar-generated";

/// marker 시작 낱말.
pub const BEGIN: &str = "stellar:generated:begin";
/// marker 끝 낱말.
pub const END: &str = "stellar:generated:end";

/// 쓰기 오류. 오류가 나면 출력 위치는 바뀌지 않습니다.
#[derive(Debug)]
pub enum WriteError {
    /// 파일 시스템 오류.
    Io(std::io::Error),
    /// 출력 경로가 디렉터리가 아닙니다.
    NotADirectory(PathBuf),
    /// symlink는 따라가지 않습니다.
    Symlink(PathBuf),
    /// 생성 manifest 밖 사용자 파일이 있습니다(상대 경로, 정렬).
    UserFiles(Vec<String>),
    /// manifest가 잘못되었습니다.
    InvalidManifest(PathBuf),
    /// 요청한 marker 구간이 없습니다.
    MarkerMissing(String),
    /// marker 짝이 맞지 않거나 겹치거나 두 번 있습니다.
    MarkerMismatch(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Io(e) => write!(f, "{e}"),
            WriteError::NotADirectory(p) => {
                write!(f, "출력 경로가 디렉터리가 아닙니다: {}", p.display())
            }
            WriteError::Symlink(p) => write!(f, "symlink는 쓰지 않습니다: {}", p.display()),
            WriteError::UserFiles(files) => write!(
                f,
                "생성 manifest 밖 파일 {}개가 있어 덮어쓰지 않습니다: {}",
                files.len(),
                files.join(", ")
            ),
            WriteError::InvalidManifest(p) => {
                write!(f, "생성 manifest가 잘못되었습니다: {}", p.display())
            }
            WriteError::MarkerMissing(name) => {
                write!(f, "generated marker 구간이 없습니다: {name}")
            }
            WriteError::MarkerMismatch(name) => {
                write!(f, "generated marker 짝이 맞지 않습니다: {name}")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(error: std::io::Error) -> WriteError {
        WriteError::Io(error)
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn sibling(dir: &Path, tag: &str) -> Result<PathBuf, WriteError> {
    let parent = dir
        .parent()
        .ok_or_else(|| WriteError::NotADirectory(dir.to_path_buf()))?;
    let name = dir
        .file_name()
        .ok_or_else(|| WriteError::NotADirectory(dir.to_path_buf()))?
        .to_string_lossy();
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{name}.stellar-{tag}-{}-{unique}",
        std::process::id()
    )))
}

/// 디렉터리 안 파일(상대 경로, 정렬). 빈 하위 디렉터리는 `<경로>/`로 넣습니다(생성 manifest는 파일만 담으므로 빈
/// 디렉터리는 사용자 항목이고, 교체로 지우지 않습니다). symlink는 오류입니다.
fn list_files(root: &Path) -> Result<Vec<String>, WriteError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir != root && fs::read_dir(&dir)?.next().is_none() {
            let relative = dir
                .strip_prefix(root)
                .map_err(|_| WriteError::NotADirectory(dir.clone()))?;
            out.push(format!(
                "{}/",
                relative.to_string_lossy().replace('\\', "/")
            ));
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                return Err(WriteError::Symlink(path));
            }
            if kind.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| WriteError::NotADirectory(path.clone()))?;
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn read_manifest(path: &Path) -> Result<BTreeSet<String>, WriteError> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    if lines.next() != Some(&format!("# {GENERATED_NOTICE}")[..]) {
        return Err(WriteError::InvalidManifest(path.to_path_buf()));
    }
    let mut out = BTreeSet::new();
    for line in lines {
        if !valid_path(line) {
            return Err(WriteError::InvalidManifest(path.to_path_buf()));
        }
        out.insert(line.to_string());
    }
    Ok(out)
}

fn write_synced(path: &Path, content: &[u8]) -> Result<(), WriteError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

/// 쓰기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReport {
    /// 쓴 파일(상대 경로, 정렬).
    pub written: Vec<String>,
    /// 이전 생성에서 있었지만 이번에 없어진 파일.
    pub removed: Vec<String>,
}

/// 출력 디렉터리를 생성 파일 집합으로 교체합니다(모듈 문서 1).
pub fn replace_directory(
    out_dir: &Path,
    files: &GeneratedFiles,
) -> Result<WriteReport, WriteError> {
    let mut previous: BTreeSet<String> = BTreeSet::new();
    let exists = match fs::symlink_metadata(out_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(WriteError::Symlink(out_dir.to_path_buf()))
        }
        Ok(meta) if !meta.is_dir() => return Err(WriteError::NotADirectory(out_dir.to_path_buf())),
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if exists {
        let present = list_files(out_dir)?;
        let manifest = out_dir.join(MANIFEST);
        if !present.is_empty() {
            if !manifest.exists() {
                return Err(WriteError::UserFiles(present));
            }
            previous = read_manifest(&manifest)?;
            let user: Vec<String> = present
                .into_iter()
                .filter(|p| p != MANIFEST && !previous.contains(p))
                .collect();
            if !user.is_empty() {
                return Err(WriteError::UserFiles(user));
            }
        }
    }
    // 1. 임시 디렉터리에 전부 씁니다.
    let temp = sibling(out_dir, "tmp")?;
    fs::create_dir(&temp)?;
    let result = (|| -> Result<Vec<String>, WriteError> {
        let mut written = Vec::new();
        for (path, content) in files.iter() {
            write_synced(&temp.join(path), content.as_bytes())?;
            written.push(path.to_string());
        }
        let mut manifest = format!("# {GENERATED_NOTICE}\n");
        for path in &written {
            manifest.push_str(path);
            manifest.push('\n');
        }
        write_synced(&temp.join(MANIFEST), manifest.as_bytes())?;
        fs::File::open(&temp)?.sync_all()?;
        Ok(written)
    })();
    let written = match result {
        Ok(written) => written,
        Err(error) => {
            let _ = fs::remove_dir_all(&temp);
            return Err(error);
        }
    };
    // 2. rename으로 교체합니다(기존은 백업 이름으로 옮겼다가 지움, 실패하면 되돌림).
    if exists {
        let backup = sibling(out_dir, "old")?;
        fs::rename(out_dir, &backup)?;
        if let Err(error) = fs::rename(&temp, out_dir) {
            let _ = fs::rename(&backup, out_dir);
            let _ = fs::remove_dir_all(&temp);
            return Err(error.into());
        }
        fs::remove_dir_all(&backup)?;
    } else {
        if let Some(parent) = out_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(error) = fs::rename(&temp, out_dir) {
            let _ = fs::remove_dir_all(&temp);
            return Err(error.into());
        }
    }
    let now: BTreeSet<&String> = written.iter().collect();
    Ok(WriteReport {
        removed: previous.into_iter().filter(|p| !now.contains(p)).collect(),
        written,
    })
}

/// 줄의 marker(시작이면 `true`, 이름).
fn marker(line: &str) -> Option<(bool, String)> {
    let (begin, rest) = if let Some(at) = line.find(BEGIN) {
        (true, &line[at + BEGIN.len()..])
    } else if let Some(at) = line.find(END) {
        (false, &line[at + END.len()..])
    } else {
        return None;
    };
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    Some((begin, name))
}

/// 파일 안 marker 구간을 바꿉니다(모듈 문서 2). `regions`는 (구간 이름 → 새 내용)입니다.
pub fn update_marked_regions(
    file: &Path,
    regions: &BTreeMap<String, String>,
) -> Result<(), WriteError> {
    let meta = fs::symlink_metadata(file)?;
    if meta.file_type().is_symlink() {
        return Err(WriteError::Symlink(file.to_path_buf()));
    }
    let original = fs::read_to_string(file)?;
    // 줄 단위(줄 끝 포함)로 나눠 구간 밖 byte를 그대로 둡니다.
    let lines: Vec<&str> = original.split_inclusive('\n').collect();
    let mut spans: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut open: Option<(String, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let Some((begin, name)) = marker(line) else {
            continue;
        };
        match (begin, &open) {
            (true, None) => {
                if spans.contains_key(&name) || name.is_empty() {
                    return Err(WriteError::MarkerMismatch(name));
                }
                open = Some((name, index));
            }
            (true, Some((current, _))) => return Err(WriteError::MarkerMismatch(current.clone())),
            (false, Some((current, start))) if *current == name => {
                spans.insert(name, (*start, index));
                open = None;
            }
            (false, _) => return Err(WriteError::MarkerMismatch(name)),
        }
    }
    if let Some((name, _)) = open {
        return Err(WriteError::MarkerMismatch(name));
    }
    for name in regions.keys() {
        if !spans.contains_key(name) {
            return Err(WriteError::MarkerMissing(name.clone()));
        }
    }
    let mut out = String::with_capacity(original.len());
    let mut index = 0;
    while index < lines.len() {
        out.push_str(lines[index]);
        let replaced = spans
            .iter()
            .find(|(name, (start, _))| *start == index && regions.contains_key(*name));
        if let Some((name, (_, end))) = replaced {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&normalize(&regions[name]));
            index = *end;
            continue;
        }
        index += 1;
    }
    let temp = sibling(file, "tmp")?;
    // 사용자 파일의 권한(실행 bit 등)을 그대로 둡니다. 교체 전에 실패하면 임시 파일만 지웁니다.
    let replaced = write_synced(&temp, out.as_bytes())
        .and_then(|()| fs::set_permissions(&temp, meta.permissions()).map_err(WriteError::from))
        .and_then(|()| fs::rename(&temp, file).map_err(WriteError::from));
    if let Err(error) = replaced {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
