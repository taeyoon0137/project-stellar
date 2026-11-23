//! `.stellar-plugin` package 열기와 검증 (XXI.1.2, ADR-0097 결정 3).
//!
//! [`PluginPackage::open`]은 package 디렉터리 전체를 `symlink_metadata`로 훑어 다음을 거부합니다.
//!
//! - package root 자신이 symlink이거나 디렉터리가 아님, 이름이 `.stellar-plugin`으로 끝나지 않음
//! - package 안 symlink(대상이 밖이거나 깨졌으면 `symlink-escape`, 안이면 `package-symlink`)
//! - 일반 파일·디렉터리 외 항목, [`PackagePath`] 규칙에 맞지 않는 이름, 항목·크기·깊이 상한 초과
//! - manifest 오류([`ManifestError`]), 지원하지 않는 Host API major
//! - `entry`·`wasm[].path`가 package 안 일반 파일이 아님, WASM magic·version 불일치
//!
//! [`scan_plugins`]는 플러그인 디렉터리 하나의 package를 모두 열고 같은 id를 가진 package를 모두 거부합니다.
//! 오류 문구에 package 원문 값(경로·id)을 넣지 않습니다. 위치는 구조화 필드로만 냅니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use stellar_storage::hash::ContentHash;

use crate::manifest::{ManifestError, PluginId, PluginManifest, MANIFEST_FILE, MAX_MANIFEST_BYTES};
use crate::path::{PackagePath, PathError, MAX_PATH_DEPTH};

/// package 디렉터리 이름 접미사.
pub const PACKAGE_SUFFIX: &str = ".stellar-plugin";

/// package 안 최대 항목(파일·디렉터리) 수.
pub const MAX_ENTRIES: usize = 4096;

/// package 전체 최대 byte 수.
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// 파일 하나 최대 byte 수.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// WASM binary magic과 version 1.
pub const WASM_HEADER: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

/// package 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// package root를 읽을 수 없음.
    Io {
        /// OS 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// root가 디렉터리가 아니거나 symlink.
    NotDirectory,
    /// 이름이 `.stellar-plugin`으로 끝나지 않음.
    BadPackageName,
    /// `plugin.yaml` 없음.
    MissingManifest,
    /// manifest 오류.
    Manifest(ManifestError),
    /// package 밖을 가리키거나 깨진 symlink.
    SymlinkEscape {
        /// package 상대 위치.
        at: String,
    },
    /// package 안을 가리키는 symlink(전면 금지).
    Symlink {
        /// package 상대 위치.
        at: String,
    },
    /// 일반 파일·디렉터리가 아닌 항목.
    SpecialFile {
        /// package 상대 위치.
        at: String,
    },
    /// 경로 규칙에 맞지 않는 이름(UTF-8 아님 포함).
    InvalidName {
        /// 원인.
        error: Option<PathError>,
    },
    /// 항목·크기·깊이 상한 초과.
    TooLarge,
    /// 이 빌드가 제공하지 않는 Host API major.
    UnsupportedHostApi {
        /// 요구 version.
        requested: u32,
        /// 제공 version.
        supported: u32,
    },
    /// `entry`·`wasm[].path`가 package 안 일반 파일이 아님.
    MissingFile {
        /// manifest 필드.
        field: String,
    },
    /// WASM magic·version 불일치.
    InvalidWasm {
        /// manifest 필드.
        field: String,
    },
    /// 같은 플러그인 디렉터리에 같은 id가 둘 이상.
    DuplicateId {
        /// 중복 id.
        id: PluginId,
    },
}

impl PackageError {
    /// 고정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            PackageError::Io { .. } => "package-io",
            PackageError::NotDirectory => "package-not-directory",
            PackageError::BadPackageName => "package-bad-name",
            PackageError::MissingManifest => "manifest-missing",
            PackageError::Manifest(e) => e.code(),
            PackageError::SymlinkEscape { .. } => "symlink-escape",
            PackageError::Symlink { .. } => "package-symlink",
            PackageError::SpecialFile { .. } => "package-special-file",
            PackageError::InvalidName { .. } => "package-invalid-name",
            PackageError::TooLarge => "package-too-large",
            PackageError::UnsupportedHostApi { .. } => "unsupported-host-api",
            PackageError::MissingFile { .. } => "package-missing-file",
            PackageError::InvalidWasm { .. } => "package-invalid-wasm",
            PackageError::DuplicateId { .. } => "duplicate-id",
        }
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Io { kind } => write!(f, "package를 읽을 수 없습니다({kind:?})"),
            PackageError::NotDirectory => {
                f.write_str("package는 symlink가 아닌 디렉터리여야 합니다")
            }
            PackageError::BadPackageName => {
                write!(
                    f,
                    "package 디렉터리 이름은 {PACKAGE_SUFFIX}로 끝나야 합니다"
                )
            }
            PackageError::MissingManifest => f.write_str("plugin.yaml이 없습니다"),
            PackageError::Manifest(e) => write!(f, "{e}"),
            PackageError::SymlinkEscape { .. } => {
                f.write_str("package 밖을 가리키거나 깨진 symlink가 있습니다")
            }
            PackageError::Symlink { .. } => f.write_str("package 안에는 symlink를 둘 수 없습니다"),
            PackageError::SpecialFile { .. } => {
                f.write_str("일반 파일·디렉터리가 아닌 항목이 있습니다")
            }
            PackageError::InvalidName { .. } => {
                f.write_str("경로 규칙에 맞지 않는 이름이 있습니다")
            }
            PackageError::TooLarge => f.write_str("package 항목 수·크기·깊이 상한을 넘습니다"),
            PackageError::UnsupportedHostApi {
                requested,
                supported,
            } => write!(
                f,
                "Host API {requested}은 지원하지 않습니다(이 빌드: {supported})"
            ),
            PackageError::MissingFile { field } => {
                write!(f, "{field}가 package 안 일반 파일이 아닙니다")
            }
            PackageError::InvalidWasm { field } => write!(f, "{field}가 WASM 1 binary가 아닙니다"),
            PackageError::DuplicateId { .. } => {
                f.write_str("같은 플러그인 디렉터리에 같은 id의 package가 둘 이상 있습니다")
            }
        }
    }
}

impl std::error::Error for PackageError {}

fn io(e: std::io::Error) -> PackageError {
    PackageError::Io { kind: e.kind() }
}

/// 검증한 package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPackage {
    root: PathBuf,
    manifest: PluginManifest,
    files: BTreeMap<PackagePath, ContentHash>,
    digest: ContentHash,
}

impl PluginPackage {
    /// package 디렉터리를 열어 검증합니다. `supported_host_api`는 이 빌드의 Host API major입니다.
    pub fn open(root: &Path, supported_host_api: u32) -> Result<PluginPackage, PackageError> {
        let meta = fs::symlink_metadata(root).map_err(io)?;
        if !meta.is_dir() {
            return Err(PackageError::NotDirectory);
        }
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(PackageError::BadPackageName)?;
        if !name.ends_with(PACKAGE_SUFFIX) || name.len() == PACKAGE_SUFFIX.len() {
            return Err(PackageError::BadPackageName);
        }
        let root = fs::canonicalize(root).map_err(io)?;
        let files = walk(&root)?;

        let manifest_path = PackagePath::parse(MANIFEST_FILE).expect("고정 이름");
        if !files.contains_key(&manifest_path) {
            return Err(PackageError::MissingManifest);
        }
        let bytes = read_limited(&root.join(MANIFEST_FILE), MAX_MANIFEST_BYTES as u64)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            PackageError::Manifest(ManifestError::InvalidValue {
                field: MANIFEST_FILE.to_owned(),
                reason: "UTF-8이 아닙니다",
            })
        })?;
        let manifest = PluginManifest::parse(text).map_err(PackageError::Manifest)?;
        if manifest.host_api != supported_host_api {
            return Err(PackageError::UnsupportedHostApi {
                requested: manifest.host_api,
                supported: supported_host_api,
            });
        }
        if !files.contains_key(&manifest.entry) {
            return Err(PackageError::MissingFile {
                field: "entry".to_owned(),
            });
        }
        for (i, module) in manifest.wasm.iter().enumerate() {
            let field = format!("wasm[{i}].path");
            if !files.contains_key(&module.path) {
                return Err(PackageError::MissingFile { field });
            }
            let mut header = [0u8; 8];
            let ok = fs::File::open(root.join(module.path.as_str()))
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut header))
                .is_ok();
            if !ok || header != WASM_HEADER {
                return Err(PackageError::InvalidWasm { field });
            }
        }
        let listing: String = files
            .iter()
            .map(|(path, hash)| format!("{path}\0{hash}\n"))
            .collect();
        let digest = ContentHash::compute(listing.as_bytes());
        Ok(PluginPackage {
            root,
            manifest,
            files,
            digest,
        })
    }

    /// package root(실제 경로).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// manifest.
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// package 파일과 내용 hash(경로 순).
    pub fn files(&self) -> &BTreeMap<PackagePath, ContentHash> {
        &self.files
    }

    /// package digest: (경로, 내용 hash) 목록의 sha256.
    pub fn digest(&self) -> ContentHash {
        self.digest
    }

    /// package 파일이 있으면 package 상대 경로를 돌려줍니다(WebView asset 요청 해석용). 문법은
    /// [`PackagePath::parse`]가 보고, 검증 당시 파일 목록에 있어야 합니다.
    pub fn resolve(&self, request: &str) -> Result<PathBuf, PackageError> {
        let path = PackagePath::parse(request)
            .map_err(|e| PackageError::InvalidName { error: Some(e) })?;
        if !self.files.contains_key(&path) {
            return Err(PackageError::MissingFile {
                field: "request".to_owned(),
            });
        }
        Ok(self.root.join(path.as_str()))
    }
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, PackageError> {
    let meta = fs::symlink_metadata(path).map_err(io)?;
    if meta.len() > limit {
        return Err(PackageError::Manifest(ManifestError::TooLarge));
    }
    fs::read(path).map_err(io)
}

/// package 전체를 훑어 일반 파일의 (상대 경로, hash)를 모읍니다.
fn walk(root: &Path) -> Result<BTreeMap<PackagePath, ContentHash>, PackageError> {
    let mut files = BTreeMap::new();
    let mut entries = 0usize;
    let mut total = 0u64;
    // (실제 경로, package 상대 경로 문자열, 깊이)
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)
            .map_err(io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io)?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            entries += 1;
            if entries > MAX_ENTRIES || depth + 1 > MAX_PATH_DEPTH {
                return Err(PackageError::TooLarge);
            }
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| PackageError::InvalidName { error: None })?;
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = PackagePath::parse(&rel)
                .map_err(|e| PackageError::InvalidName { error: Some(e) })?;
            let full = child.path();
            let meta = fs::symlink_metadata(&full).map_err(io)?;
            let kind = meta.file_type();
            if kind.is_symlink() {
                let inside = fs::canonicalize(&full)
                    .map(|target| target.starts_with(root))
                    .unwrap_or(false);
                return Err(if inside {
                    PackageError::Symlink { at: rel }
                } else {
                    PackageError::SymlinkEscape { at: rel }
                });
            }
            if kind.is_dir() {
                stack.push((full, rel, depth + 1));
            } else if kind.is_file() {
                if meta.len() > MAX_FILE_BYTES {
                    return Err(PackageError::TooLarge);
                }
                total += meta.len();
                if total > MAX_TOTAL_BYTES {
                    return Err(PackageError::TooLarge);
                }
                let bytes = fs::read(&full).map_err(io)?;
                files.insert(path, ContentHash::compute(&bytes));
            } else {
                return Err(PackageError::SpecialFile { at: rel });
            }
        }
    }
    Ok(files)
}

/// 플러그인 디렉터리 훑기 결과 한 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    /// 디렉터리 안 package 이름(`*.stellar-plugin`).
    pub name: String,
    /// 열기 결과.
    pub result: Result<PluginPackage, PackageError>,
}

/// 플러그인 디렉터리 하나의 `*.stellar-plugin` 항목을 모두 열어 이름 순으로 돌려줍니다.
///
/// 같은 id를 가진 package는 모두 [`PackageError::DuplicateId`]로 바꿉니다. `.stellar-plugin`으로 끝나지 않는
/// 항목은 무시합니다.
pub fn scan_plugins(dir: &Path, supported_host_api: u32) -> Result<Vec<ScanEntry>, PackageError> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(io)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(PACKAGE_SUFFIX))
        .collect();
    names.sort();
    let mut entries: Vec<ScanEntry> = names
        .into_iter()
        .map(|name| {
            let result = PluginPackage::open(&dir.join(&name), supported_host_api);
            ScanEntry { name, result }
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for entry in &entries {
        if let Ok(package) = &entry.result {
            if !seen.insert(package.manifest().id.clone()) {
                duplicates.insert(package.manifest().id.clone());
            }
        }
    }
    for entry in &mut entries {
        let duplicate = match &entry.result {
            Ok(package) => duplicates.get(&package.manifest().id).cloned(),
            Err(_) => None,
        };
        if let Some(id) = duplicate {
            entry.result = Err(PackageError::DuplicateId { id });
        }
    }
    Ok(entries)
}
