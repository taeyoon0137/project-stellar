//! 시스템 폰트 discovery (XIII.6.2).
//!
//! 플랫폼 adapter([`SystemFontLocator`], macOS는 CoreText)가 설치된 폰트 **파일 경로**를 알려 주면,
//! 이 모듈이 파일마다 streaming hash와 container probe([`crate::fonts::sfnt`])를 계산해 face마다
//! `FontSource::System`, `FontFaceKey(hash, index)`로 [`FontRegistry`]에 등록합니다.
//!
//! - **복사하지 않음**: 시스템 폰트는 원래 위치를 읽기만 합니다. 프로젝트 root·`AssetStore`를 받지
//!   않으므로 프로젝트에 파일을 쓸 수 없습니다. 재현성이 필요하면 사용자가 Project Font로
//!   import합니다(XIII.6.1).
//! - **face 정체성**: 경로가 아니라 hash+index입니다. 같은 bytes가 두 경로에 있으면 같은 face로
//!   dedup됩니다(X.8.2). 경로는 catalog의 위치 정보일 뿐입니다.
//! - **결정론**: 경로를 정렬·중복 제거한 순서로 처리하므로 locator가 돌려준 순서와 무관하게 같은
//!   catalog를 만듭니다.
//! - **일관성**: 한 파일의 hash와 probe는 같은 열린 handle로 읽고, 읽기 전후 크기·수정 시각이
//!   다르면 그 파일을 건너뜁니다([`SystemFontSkipReason::ChangedWhileReading`]).
//! - **비용 제한**: 파일 수([`SystemFontLimits::max_files`])와 파일 크기
//!   ([`SystemFontLimits::max_file_bytes`])를 제한합니다. 크기·수정 시각이 같은 파일은 이전 결과를
//!   다시 쓰고 재hash하지 않습니다(메모리 cache, 마지막 discovery에 나온 경로만 보관).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{File, Metadata};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use stellar_assets::hash_stream::{hash_reader_with, StreamHashError, STREAM_BUFFER_SIZE};
use stellar_text::font::{FontFaceRef, FontFileHash, FontSource};
use stellar_text::registry::{FontFaceKey, FontHandle, FontRegistry, FontRegistryError};

use super::sfnt::{read_font_file, FontFaceNames, FontFileFormat, FontFileInfo, FontParseError};

/// 기본 최대 시스템 폰트 파일 수.
pub const DEFAULT_MAX_SYSTEM_FONT_FILES: usize = 16_384;

/// 기본 최대 시스템 폰트 파일 크기(byte).
pub const DEFAULT_MAX_SYSTEM_FONT_FILE_BYTES: u64 = 1 << 30;

/// 플랫폼 font 목록 조회 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFontLocateError {
    message: String,
}

impl SystemFontLocateError {
    /// 원인 메시지로 만듭니다.
    pub fn new(message: impl Into<String>) -> SystemFontLocateError {
        SystemFontLocateError {
            message: message.into(),
        }
    }

    /// 원인 메시지.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SystemFontLocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "시스템 폰트 목록을 조회할 수 없습니다: {}", self.message)
    }
}

impl std::error::Error for SystemFontLocateError {}

/// 설치된 시스템 폰트 파일 경로를 알려 주는 플랫폼 adapter.
pub trait SystemFontLocator {
    /// 사용 가능한 폰트 파일의 절대 경로 목록. 순서·중복은 상관없습니다.
    fn font_files(&self) -> Result<Vec<PathBuf>, SystemFontLocateError>;
}

/// discovery 비용 상한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFontLimits {
    /// 처리할 최대 파일 수(정렬 순서로 앞에서부터).
    pub max_files: usize,
    /// 처리할 최대 파일 크기(byte, 이 값까지 허용).
    pub max_file_bytes: u64,
}

impl Default for SystemFontLimits {
    fn default() -> SystemFontLimits {
        SystemFontLimits {
            max_files: DEFAULT_MAX_SYSTEM_FONT_FILES,
            max_file_bytes: DEFAULT_MAX_SYSTEM_FONT_FILE_BYTES,
        }
    }
}

/// registry에 등록한 시스템 face 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFontFace {
    path: PathBuf,
    format: FontFileFormat,
    key: FontFaceKey,
    handle: FontHandle,
    names: FontFaceNames,
}

impl SystemFontFace {
    /// 폰트 파일의 원래 위치(복사본 아님).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// container 형식.
    pub fn format(&self) -> FontFileFormat {
        self.format
    }

    /// face 정체성(hash+index).
    pub fn key(&self) -> FontFaceKey {
        self.key
    }

    /// registry handle.
    pub fn handle(&self) -> FontHandle {
        self.handle
    }

    /// face 이름.
    pub fn names(&self) -> &FontFaceNames {
        &self.names
    }

    /// 문서에 기록할 face 참조(source = System, variation 없음).
    pub fn face_ref(&self) -> FontFaceRef {
        FontFaceRef::new(
            FontSource::System,
            self.key.file_hash(),
            self.key.face_index(),
            Vec::new(),
        )
    }
}

/// 파일을 건너뛴 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemFontSkipReason {
    /// 절대 경로가 아닙니다.
    RelativePath,
    /// 열기·읽기 실패.
    Io {
        /// I/O 오류 종류.
        kind: ErrorKind,
        /// 원인 메시지.
        message: String,
    },
    /// 일반 파일이 아닙니다.
    NotAFile,
    /// [`SystemFontLimits::max_file_bytes`]를 넘습니다.
    TooLarge {
        /// 파일 크기(byte).
        size: u64,
        /// 허용 최대 크기(byte).
        max: u64,
    },
    /// 지원하는 폰트 container가 아닙니다(예: `.dfont`).
    NotAFont(FontParseError),
    /// 읽는 도중 파일이 바뀌었습니다.
    ChangedWhileReading,
    /// registry 등록 실패. 이 파일의 face는 catalog에 넣지 않습니다.
    Registry(FontRegistryError),
}

impl fmt::Display for SystemFontSkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SystemFontSkipReason::RelativePath => f.write_str("절대 경로가 아닙니다"),
            SystemFontSkipReason::Io { message, .. } => write!(f, "읽을 수 없습니다: {message}"),
            SystemFontSkipReason::NotAFile => f.write_str("일반 파일이 아닙니다"),
            SystemFontSkipReason::TooLarge { size, max } => {
                write!(f, "파일이 너무 큽니다: {size} bytes > {max} bytes")
            }
            SystemFontSkipReason::NotAFont(e) => write!(f, "{e}"),
            SystemFontSkipReason::ChangedWhileReading => {
                f.write_str("읽는 도중 파일이 바뀌었습니다")
            }
            SystemFontSkipReason::Registry(e) => write!(f, "{e}"),
        }
    }
}

/// 건너뛴 파일 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedSystemFont {
    /// 파일 경로.
    pub path: PathBuf,
    /// 이유.
    pub reason: SystemFontSkipReason,
}

/// discovery 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemFontCatalog {
    faces: Vec<SystemFontFace>,
    skipped: Vec<SkippedSystemFont>,
    files_over_limit: usize,
}

impl SystemFontCatalog {
    /// 등록한 face(경로 오름차순, 같은 파일 안은 face index 오름차순).
    pub fn faces(&self) -> &[SystemFontFace] {
        &self.faces
    }

    /// 건너뛴 파일(경로 오름차순).
    pub fn skipped(&self) -> &[SkippedSystemFont] {
        &self.skipped
    }

    /// [`SystemFontLimits::max_files`]를 넘어 처리하지 않은 파일 수.
    pub fn files_over_limit(&self) -> usize {
        self.files_over_limit
    }

    /// PostScript 이름이 정확히 같은 face.
    pub fn faces_with_postscript_name<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a SystemFontFace> + 'a {
        self.faces
            .iter()
            .filter(move |face| face.names.postscript() == Some(name))
    }

    /// family 이름이 정확히 같은 face.
    pub fn faces_with_family<'a>(
        &'a self,
        family: &'a str,
    ) -> impl Iterator<Item = &'a SystemFontFace> + 'a {
        self.faces
            .iter()
            .filter(move |face| face.names.family() == Some(family))
    }
}

/// 재hash 판단용 파일 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(metadata: &Metadata) -> FileStamp {
        FileStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedFile {
    stamp: FileStamp,
    hash: FontFileHash,
    info: FontFileInfo,
}

/// 시스템 폰트 discovery(파일 결과 cache 포함).
#[derive(Debug, Clone)]
pub struct SystemFontDiscovery {
    limits: SystemFontLimits,
    cache: BTreeMap<PathBuf, CachedFile>,
    files_hashed: u64,
}

impl SystemFontDiscovery {
    /// 비용 상한으로 만듭니다(파일 I/O 없음).
    pub fn new(limits: SystemFontLimits) -> SystemFontDiscovery {
        SystemFontDiscovery {
            limits,
            cache: BTreeMap::new(),
            files_hashed: 0,
        }
    }

    /// 지금까지 실제로 hash한 파일 수(cache 적중 제외).
    pub fn files_hashed(&self) -> u64 {
        self.files_hashed
    }

    /// locator의 모든 폰트 파일을 registry에 등록합니다.
    pub fn discover<L: SystemFontLocator + ?Sized, R: FontRegistry + ?Sized>(
        &mut self,
        locator: &L,
        registry: &mut R,
    ) -> Result<SystemFontCatalog, SystemFontLocateError> {
        let paths = locator.font_files()?;
        Ok(self.register_files(paths, registry))
    }

    /// 주어진 폰트 파일들을 registry에 등록합니다.
    pub fn register_files<R: FontRegistry + ?Sized>(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        registry: &mut R,
    ) -> SystemFontCatalog {
        let paths: BTreeSet<PathBuf> = paths.into_iter().collect();
        let mut catalog = SystemFontCatalog {
            files_over_limit: paths.len().saturating_sub(self.limits.max_files),
            ..SystemFontCatalog::default()
        };
        let mut previous = std::mem::take(&mut self.cache);
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        for path in paths.into_iter().take(self.limits.max_files) {
            let cached = previous.remove(&path);
            let file = match self.load(&path, cached, &mut buffer) {
                Ok(file) => file,
                Err(reason) => {
                    catalog.skipped.push(SkippedSystemFont { path, reason });
                    continue;
                }
            };
            let registered: Result<Vec<FontHandle>, FontRegistryError> =
                (0..file.info.face_count())
                    .map(|i| registry.register(FontSource::System, FontFaceKey::new(file.hash, i)))
                    .collect();
            match registered {
                Ok(handles) => {
                    for (index, (handle, face)) in
                        handles.into_iter().zip(&file.info.faces).enumerate()
                    {
                        catalog.faces.push(SystemFontFace {
                            path: path.clone(),
                            format: file.info.format,
                            // index < face_count <= MAX_FACES_PER_FILE.
                            key: FontFaceKey::new(file.hash, index as u32),
                            handle,
                            names: face.names.clone(),
                        });
                    }
                }
                Err(e) => catalog.skipped.push(SkippedSystemFont {
                    path: path.clone(),
                    reason: SystemFontSkipReason::Registry(e),
                }),
            }
            self.cache.insert(path, file);
        }
        catalog
    }

    /// 파일 하나의 hash·probe(cache 적중이면 재사용).
    fn load(
        &mut self,
        path: &Path,
        cached: Option<CachedFile>,
        buffer: &mut [u8],
    ) -> Result<CachedFile, SystemFontSkipReason> {
        if !path.is_absolute() {
            return Err(SystemFontSkipReason::RelativePath);
        }
        let io = |e: std::io::Error| SystemFontSkipReason::Io {
            kind: e.kind(),
            message: e.to_string(),
        };
        let mut file = File::open(path).map_err(io)?;
        let before = file.metadata().map_err(io)?;
        if !before.is_file() {
            return Err(SystemFontSkipReason::NotAFile);
        }
        if before.len() > self.limits.max_file_bytes {
            return Err(SystemFontSkipReason::TooLarge {
                size: before.len(),
                max: self.limits.max_file_bytes,
            });
        }
        let stamp = FileStamp::of(&before);
        if let Some(cached) = cached.filter(|c| c.stamp == stamp) {
            return Ok(cached);
        }
        let digest = hash_reader_with(&mut file, buffer, |_| Ok(())).map_err(|e| match e {
            StreamHashError::Io(e) => io(e),
            // ContentHash 생성 실패는 일어나지 않아야 하지만 패닉 대신 건너뜁니다.
            StreamHashError::Hash(e) => SystemFontSkipReason::Io {
                kind: ErrorKind::InvalidData,
                message: e.to_string(),
            },
        })?;
        if digest.size != stamp.len {
            return Err(SystemFontSkipReason::ChangedWhileReading);
        }
        let info = read_font_file(&mut file, stamp.len).map_err(SystemFontSkipReason::NotAFont)?;
        let after = file.metadata().map_err(io)?;
        if FileStamp::of(&after) != stamp {
            return Err(SystemFontSkipReason::ChangedWhileReading);
        }
        self.files_hashed += 1;
        Ok(CachedFile {
            stamp,
            hash: FontFileHash::from_sha256_digest(*digest.hash.digest()),
            info,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::test_fixtures::{family_faces, sfnt_face, ttc, TempDir, SIG_TRUETYPE};
    use stellar_storage::hash::ContentHash;
    use stellar_text::registry::InMemoryFontRegistry;

    struct ListLocator(Result<Vec<PathBuf>, SystemFontLocateError>);

    impl SystemFontLocator for ListLocator {
        fn font_files(&self) -> Result<Vec<PathBuf>, SystemFontLocateError> {
            self.0.clone()
        }
    }

    fn hash_of(bytes: &[u8]) -> FontFileHash {
        FontFileHash::from_sha256_digest(*ContentHash::compute(bytes).digest())
    }

    #[test]
    fn system_files_are_registered_in_place_as_system_faces() {
        let dir = TempDir::new("system");
        let ttf = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Mono")[0]);
        let collection = ttc(&family_faces("Fixture Sans"));
        let ttf_path = dir.path.join("FixtureMono.ttf");
        let ttc_path = dir.path.join("FixtureSans.ttc");
        std::fs::write(&ttf_path, &ttf).unwrap();
        std::fs::write(&ttc_path, &collection).unwrap();

        let mut discovery = SystemFontDiscovery::new(SystemFontLimits::default());
        let mut registry = InMemoryFontRegistry::new();
        // locator 순서·중복과 무관합니다.
        let locator = ListLocator(Ok(vec![
            ttc_path.clone(),
            ttf_path.clone(),
            ttc_path.clone(),
        ]));
        let catalog = discovery.discover(&locator, &mut registry).unwrap();
        assert!(catalog.skipped().is_empty());
        assert_eq!(catalog.faces().len(), 4);
        assert_eq!(registry.len(), 4);
        // 경로 오름차순(FixtureMono < FixtureSans), 파일 안은 face index 순.
        assert_eq!(catalog.faces()[0].path(), ttf_path);
        assert_eq!(
            catalog.faces()[1].key(),
            FontFaceKey::new(hash_of(&collection), 0)
        );
        assert_eq!(
            catalog.faces()[3].key(),
            FontFaceKey::new(hash_of(&collection), 2)
        );
        for face in catalog.faces() {
            assert_eq!(registry.lookup(face.key()), Some(face.handle()));
            let registered = registry.face(face.handle()).unwrap();
            assert!(registered.has_source(FontSource::System));
            assert!(!registered.has_source(FontSource::Project));
            assert_eq!(face.face_ref().source(), FontSource::System);
            assert_eq!(registry.resolve(&face.face_ref()), Some(face.handle()));
        }
        let bold: Vec<_> = catalog
            .faces_with_postscript_name("FixtureSans-Bold")
            .collect();
        assert_eq!(bold.len(), 1);
        assert_eq!(bold[0].format(), FontFileFormat::Collection);
        assert_eq!(bold[0].names().subfamily(), Some("Bold"));
        assert_eq!(catalog.faces_with_family("Fixture Sans").count(), 3);
        // 원래 파일만 읽었고 디렉터리에 새 파일이 없습니다(복사 없음).
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 2);
        assert_eq!(std::fs::read(&ttc_path).unwrap(), collection);
    }

    #[test]
    fn same_bytes_at_two_paths_share_one_face() {
        let dir = TempDir::new("system");
        let bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        let a = dir.path.join("a.ttf");
        let b = dir.path.join("b.ttf");
        std::fs::write(&a, &bytes).unwrap();
        std::fs::write(&b, &bytes).unwrap();
        let mut registry = InMemoryFontRegistry::new();
        let catalog = SystemFontDiscovery::new(SystemFontLimits::default())
            .register_files([a, b], &mut registry);
        assert_eq!(catalog.faces().len(), 2);
        assert_eq!(catalog.faces()[0].handle(), catalog.faces()[1].handle());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn unusable_files_are_skipped_with_reasons() {
        let dir = TempDir::new("system");
        let font = dir.path.join("ok.ttf");
        std::fs::write(
            &font,
            sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]),
        )
        .unwrap();
        let text = dir.path.join("readme.txt");
        std::fs::write(&text, b"not a font").unwrap();
        let big = dir.path.join("big.ttc");
        std::fs::write(&big, ttc(&family_faces("Fixture Sans"))).unwrap();
        let sub = dir.path.join("subdir");
        std::fs::create_dir(&sub).unwrap();

        let limits = SystemFontLimits {
            max_files: 16,
            max_file_bytes: std::fs::metadata(&font).unwrap().len(),
        };
        let mut registry = InMemoryFontRegistry::new();
        let catalog = SystemFontDiscovery::new(limits).register_files(
            [
                font.clone(),
                text.clone(),
                big.clone(),
                sub.clone(),
                dir.path.join("missing.ttf"),
                PathBuf::from("relative.ttf"),
            ],
            &mut registry,
        );
        assert_eq!(catalog.faces().len(), 1);
        let reasons: BTreeMap<PathBuf, &SystemFontSkipReason> = catalog
            .skipped()
            .iter()
            .map(|s| (s.path.clone(), &s.reason))
            .collect();
        assert_eq!(reasons.len(), 5);
        assert!(matches!(
            reasons[&big],
            SystemFontSkipReason::TooLarge { .. }
        ));
        assert!(matches!(
            reasons[&text],
            SystemFontSkipReason::NotAFont(FontParseError::UnknownSignature { .. })
        ));
        assert_eq!(reasons[&sub], &SystemFontSkipReason::NotAFile);
        assert!(matches!(
            reasons[&dir.path.join("missing.ttf")],
            SystemFontSkipReason::Io {
                kind: ErrorKind::NotFound,
                ..
            }
        ));
        assert_eq!(
            reasons[&PathBuf::from("relative.ttf")],
            &SystemFontSkipReason::RelativePath
        );
    }

    #[test]
    fn file_limit_and_registry_capacity_are_enforced() {
        let dir = TempDir::new("system");
        let a = dir.path.join("a.ttc");
        let b = dir.path.join("b.ttf");
        std::fs::write(&a, ttc(&family_faces("Fixture Sans"))).unwrap();
        std::fs::write(
            &b,
            sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Mono")[0]),
        )
        .unwrap();

        let limits = SystemFontLimits {
            max_files: 1,
            ..SystemFontLimits::default()
        };
        let mut registry = InMemoryFontRegistry::new();
        let catalog =
            SystemFontDiscovery::new(limits).register_files([a.clone(), b.clone()], &mut registry);
        assert_eq!(catalog.files_over_limit(), 1);
        assert_eq!(catalog.faces().len(), 3);

        // 3 face 파일이 registry 상한(2)을 넘으면 그 파일 face는 catalog에 넣지 않습니다.
        let mut small = InMemoryFontRegistry::with_max_faces(2);
        let catalog = SystemFontDiscovery::new(SystemFontLimits::default())
            .register_files([a.clone()], &mut small);
        assert!(catalog.faces().is_empty());
        assert!(matches!(
            catalog.skipped()[0].reason,
            SystemFontSkipReason::Registry(FontRegistryError::CapacityExceeded { max_faces: 2 })
        ));
    }

    #[test]
    fn unchanged_files_are_not_rehashed() {
        let dir = TempDir::new("system");
        let path = dir.path.join("a.ttf");
        let first = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        std::fs::write(&path, &first).unwrap();
        let mut discovery = SystemFontDiscovery::new(SystemFontLimits::default());
        let mut registry = InMemoryFontRegistry::new();

        let catalog = discovery.register_files([path.clone()], &mut registry);
        assert_eq!(discovery.files_hashed(), 1);
        let again = discovery.register_files([path.clone()], &mut registry);
        assert_eq!(discovery.files_hashed(), 1);
        assert_eq!(again, catalog);

        // 내용(길이)이 바뀌면 다시 hash하고 다른 face가 됩니다.
        let mut second = first.clone();
        second.extend_from_slice(&[0; 4]);
        std::fs::write(&path, &second).unwrap();
        let changed = discovery.register_files([path.clone()], &mut registry);
        assert_eq!(discovery.files_hashed(), 2);
        assert_eq!(changed.faces()[0].key().file_hash(), hash_of(&second));
        assert_ne!(changed.faces()[0].handle(), catalog.faces()[0].handle());
    }

    #[test]
    fn locator_failure_is_reported() {
        let locator = ListLocator(Err(SystemFontLocateError::new("CoreText unavailable")));
        let mut registry = InMemoryFontRegistry::new();
        let err = SystemFontDiscovery::new(SystemFontLimits::default())
            .discover(&locator, &mut registry)
            .unwrap_err();
        assert_eq!(err.message(), "CoreText unavailable");
        assert!(err.to_string().contains("CoreText unavailable"));
        assert!(registry.is_empty());
    }
}
