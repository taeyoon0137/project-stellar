//! 플러그인 전용 data namespace 디렉터리 (XXI.6.2, ADR-0097 결정 10).
//!
//! 플러그인 파일은 프로젝트 루트의 `plugins/<plugin-id>/` 아래에만 둡니다. core 문서 schema·`design/`에 플러그인
//! 데이터를 넣지 않습니다. 경로는 [`PackagePath`] 규칙으로만 해석하고 symlink를 따라가지 않습니다(중간 디렉터리
//! 포함). 쓰기는 같은 디렉터리의 임시 파일에 쓴 뒤 rename합니다.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::manifest::PluginId;
use crate::path::{PackagePath, PathError};

/// 프로젝트 루트 아래 플러그인 data 최상위 디렉터리 이름.
pub const PLUGIN_DATA_DIR: &str = "plugins";

/// 파일 하나 최대 byte 수.
pub const MAX_DATA_FILE_BYTES: usize = 4 * 1024 * 1024;

/// 목록 최대 파일 수.
pub const MAX_LISTED_FILES: usize = 4096;

/// 임시 파일 접두사(목록에서 제외).
const TEMP_PREFIX: &str = ".stellar-plugin-tmp-";

/// data 파일 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginDataError {
    /// 경로 문법 오류.
    Path(PathError),
    /// 파일 없음.
    NotFound,
    /// 크기 상한 초과.
    TooLarge,
    /// UTF-8 텍스트가 아님.
    NotText,
    /// 경로 중간·끝이 symlink.
    Symlink,
    /// 그 밖의 OS 오류.
    Io(std::io::ErrorKind),
}

fn io(e: std::io::Error) -> PluginDataError {
    if e.kind() == std::io::ErrorKind::NotFound {
        PluginDataError::NotFound
    } else {
        PluginDataError::Io(e.kind())
    }
}

/// 한 플러그인의 data 디렉터리.
#[derive(Debug, Clone)]
pub struct PluginData {
    project_root: PathBuf,
    dir: PathBuf,
}

impl PluginData {
    /// `project_root/plugins/<id>`를 가리킵니다(만들지 않음).
    pub fn new(project_root: &Path, id: &PluginId) -> PluginData {
        PluginData {
            project_root: project_root.to_path_buf(),
            dir: project_root.join(PLUGIN_DATA_DIR).join(id.as_str()),
        }
    }

    /// data 디렉터리 경로.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 프로젝트 루트에서 `path`까지 모든 구성 요소가 symlink가 아닌지 봅니다(없는 구성 요소는 통과).
    fn no_symlink(&self, path: &Path) -> Result<(), PluginDataError> {
        let rel = path
            .strip_prefix(&self.project_root)
            .map_err(|_| PluginDataError::Symlink)?;
        let mut current = self.project_root.clone();
        for component in rel.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => return Err(PluginDataError::Symlink),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(io(e)),
            }
        }
        Ok(())
    }

    fn resolve(&self, request: &str) -> Result<(PackagePath, PathBuf), PluginDataError> {
        let path = PackagePath::parse(request).map_err(PluginDataError::Path)?;
        if path.components().any(|c| c.starts_with(TEMP_PREFIX)) {
            return Err(PluginDataError::Path(PathError::ForbiddenCharacter));
        }
        let full = self.dir.join(path.as_str());
        self.no_symlink(&full)?;
        Ok((path, full))
    }

    /// UTF-8 텍스트 파일을 읽습니다. (정규 경로, 내용).
    pub fn read(&self, request: &str) -> Result<(String, String), PluginDataError> {
        let (path, full) = self.resolve(request)?;
        let meta = fs::symlink_metadata(&full).map_err(io)?;
        if !meta.is_file() {
            return Err(PluginDataError::NotFound);
        }
        if meta.len() > MAX_DATA_FILE_BYTES as u64 {
            return Err(PluginDataError::TooLarge);
        }
        let bytes = fs::read(&full).map_err(io)?;
        let text = String::from_utf8(bytes).map_err(|_| PluginDataError::NotText)?;
        Ok((path.as_str().to_owned(), text))
    }

    /// UTF-8 텍스트 파일을 씁니다(임시 파일 뒤 rename). (정규 경로, byte 수).
    pub fn write(&self, request: &str, text: &str) -> Result<(String, usize), PluginDataError> {
        if text.len() > MAX_DATA_FILE_BYTES {
            return Err(PluginDataError::TooLarge);
        }
        let (path, full) = self.resolve(request)?;
        let parent = full.parent().expect("data 디렉터리 아래");
        fs::create_dir_all(parent).map_err(io)?;
        // 만든 뒤 다시 확인합니다(생성 사이 symlink 교체 방지).
        self.no_symlink(&full)?;
        if fs::symlink_metadata(&full).is_ok_and(|m| !m.is_file()) {
            return Err(PluginDataError::Symlink);
        }
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let temp = parent.join(format!(
            "{TEMP_PREFIX}{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &full)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(io(e));
        }
        Ok((path.as_str().to_owned(), text.len()))
    }

    /// data 디렉터리의 파일 목록(경로 순). 디렉터리가 없으면 빈 목록입니다. symlink는 건너뜁니다.
    pub fn list(&self) -> Result<Vec<String>, PluginDataError> {
        match self.no_symlink(&self.dir) {
            Ok(()) => {}
            Err(e) => return Err(e),
        }
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut stack = vec![(self.dir.clone(), String::new())];
        while let Some((dir, prefix)) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(io)? {
                let entry = entry.map_err(io)?;
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if name.starts_with(TEMP_PREFIX) {
                    continue;
                }
                let rel = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                let kind = entry.file_type().map_err(io)?;
                if kind.is_dir() {
                    stack.push((entry.path(), rel));
                } else if kind.is_file() && PackagePath::parse(&rel).is_ok() {
                    out.push(rel);
                    if out.len() > MAX_LISTED_FILES {
                        return Err(PluginDataError::TooLarge);
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}
