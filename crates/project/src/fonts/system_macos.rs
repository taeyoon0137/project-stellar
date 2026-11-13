//! macOS CoreText 시스템 폰트 locator (XIII.6.2).
//!
//! 새 crate 의존 없이 CoreText·CoreFoundation framework의 C 함수만 선언해 씁니다
//! (`watcher::fsevents`와 같은 방식).
//!
//! - [`CoreTextFontLocator::font_files`]: `CTFontManagerCopyAvailableFontURLs`가 돌려준 URL을 파일
//!   시스템 경로로 바꿉니다. 파일 URL이 아니거나 경로로 바꿀 수 없는 항목은 건너뜁니다.
//! - [`CoreTextFontLocator::locate_postscript_name`]: PostScript 이름으로 CoreText가 고른 font의
//!   파일 경로를 찾습니다. CoreText는 이름이 없으면 다른 font로 대체하므로, 고른 font의 PostScript
//!   이름이 요청과 정확히 같을 때만 경로를 돌려줍니다(임의 대체 금지, X.8.3).
//!
//! hash·face 계산과 registry 등록은 플랫폼 독립 [`crate::fonts::system`]이 합니다. CoreText의
//! face 순서나 이름을 face 정체성에 쓰지 않습니다(정체성은 파일 hash + face index).

use std::ffi::{c_char, c_void, CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use super::system::{SystemFontLocateError, SystemFontLocator};

type CFTypeRef = *const c_void;
type CFArrayRef = *const c_void;
type CFStringRef = *const c_void;
type CFURLRef = *const c_void;
type CTFontRef = *const c_void;
type CFIndex = isize;
type CFTypeID = usize;
type Boolean = u8;
type CGFloat = f64;

/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;
/// 파일 경로 buffer 크기(macOS `PATH_MAX` 1024보다 넉넉하게).
const PATH_BUFFER_LEN: usize = 4096;
/// PostScript 이름 buffer 크기(OpenType 규칙상 63자 이하).
const NAME_BUFFER_LEN: usize = 512;
/// 조회할 PostScript 이름 최대 길이(byte).
const MAX_QUERY_NAME_BYTES: usize = 255;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFURLGetTypeID() -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFURLGetFileSystemRepresentation(
        url: CFURLRef,
        resolve_against_base: Boolean,
        buffer: *mut u8,
        max_buf_len: CFIndex,
    ) -> Boolean;
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: u32,
        is_external_representation: Boolean,
    ) -> CFStringRef;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> Boolean;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    static kCTFontURLAttribute: CFStringRef;
    fn CTFontManagerCopyAvailableFontURLs() -> CFArrayRef;
    fn CTFontCreateWithName(name: CFStringRef, size: CGFloat, matrix: *const c_void) -> CTFontRef;
    fn CTFontCopyPostScriptName(font: CTFontRef) -> CFStringRef;
    fn CTFontCopyAttribute(font: CTFontRef, attribute: CFStringRef) -> CFTypeRef;
}

/// Create·Copy 규칙으로 받은 CF 객체(소유권 보유, drop 시 `CFRelease`).
struct Owned(CFTypeRef);

impl Owned {
    /// null이면 `None`.
    fn new(cf: CFTypeRef) -> Option<Owned> {
        (!cf.is_null()).then_some(Owned(cf))
    }

    fn get(&self) -> CFTypeRef {
        self.0
    }

    fn has_type(&self, type_id: CFTypeID) -> bool {
        // SAFETY: self.0은 null이 아닌 유효한 CF 객체입니다(생성 시 확인, drop 전까지 소유).
        unsafe { CFGetTypeID(self.0) == type_id }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: Create·Copy 규칙으로 받은 참조를 정확히 한 번 해제합니다.
        unsafe { CFRelease(self.0) };
    }
}

/// 파일 URL을 경로로 바꿉니다. 파일 URL이 아니거나 buffer에 맞지 않으면 `None`.
///
/// `url`은 호출자가 유효한 CFURL임을 확인한 값이어야 합니다.
fn url_to_path(url: CFURLRef) -> Option<PathBuf> {
    let mut buffer = vec![0u8; PATH_BUFFER_LEN];
    // SAFETY: url은 유효한 CFURL이고, buffer는 PATH_BUFFER_LEN bytes 쓰기 가능합니다.
    let ok = unsafe {
        CFURLGetFileSystemRepresentation(url, 1, buffer.as_mut_ptr(), PATH_BUFFER_LEN as CFIndex)
    };
    if ok == 0 {
        return None;
    }
    let len = buffer.iter().position(|b| *b == 0)?;
    (len > 0).then(|| PathBuf::from(OsStr::from_bytes(&buffer[..len])))
}

/// CFString을 UTF-8 문자열로 바꿉니다. buffer를 넘으면 `None`.
fn cf_string(string: &Owned) -> Option<String> {
    // SAFETY: CFStringGetTypeID는 인자 없는 순수 조회입니다.
    if !string.has_type(unsafe { CFStringGetTypeID() }) {
        return None;
    }
    let mut buffer = vec![0 as c_char; NAME_BUFFER_LEN];
    // SAFETY: string은 유효한 CFString이고, buffer는 NAME_BUFFER_LEN bytes 쓰기 가능합니다.
    let ok = unsafe {
        CFStringGetCString(
            string.get(),
            buffer.as_mut_ptr(),
            NAME_BUFFER_LEN as CFIndex,
            UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: 성공 시 CFStringGetCString은 buffer 안에 NUL로 끝나는 문자열을 씁니다.
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    text.to_str().ok().map(str::to_string)
}

/// macOS CoreText 시스템 폰트 locator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoreTextFontLocator;

impl CoreTextFontLocator {
    /// locator를 만듭니다.
    pub fn new() -> CoreTextFontLocator {
        CoreTextFontLocator
    }

    /// PostScript 이름이 정확히 같은 설치 font의 파일 경로. 없으면 `None`(대체 font 반환 금지).
    pub fn locate_postscript_name(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty() || name.len() > MAX_QUERY_NAME_BYTES || name.contains('\0') {
            return None;
        }
        // SAFETY: name은 유효한 UTF-8 slice이고 길이는 MAX_QUERY_NAME_BYTES 이하입니다.
        let query = Owned::new(unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                name.as_ptr(),
                name.len() as CFIndex,
                UTF8,
                0,
            )
        })?;
        // SAFETY: query는 유효한 CFString이고 matrix는 null(단위 행렬)입니다.
        let font =
            Owned::new(unsafe { CTFontCreateWithName(query.get(), 12.0, std::ptr::null()) })?;
        // SAFETY: font는 유효한 CTFont입니다.
        let resolved = Owned::new(unsafe { CTFontCopyPostScriptName(font.get()) })?;
        if cf_string(&resolved).as_deref() != Some(name) {
            return None;
        }
        // SAFETY: font는 유효한 CTFont이고 kCTFontURLAttribute는 CoreText가 제공하는 상수입니다.
        let url = Owned::new(unsafe { CTFontCopyAttribute(font.get(), kCTFontURLAttribute) })?;
        // SAFETY: CFURLGetTypeID는 인자 없는 순수 조회입니다.
        if !url.has_type(unsafe { CFURLGetTypeID() }) {
            return None;
        }
        url_to_path(url.get())
    }
}

impl SystemFontLocator for CoreTextFontLocator {
    fn font_files(&self) -> Result<Vec<PathBuf>, SystemFontLocateError> {
        // SAFETY: 인자 없는 CoreText 조회이며 결과는 Copy 규칙(소유권 이전)입니다.
        let urls =
            Owned::new(unsafe { CTFontManagerCopyAvailableFontURLs() }).ok_or_else(|| {
                SystemFontLocateError::new(
                    "CTFontManagerCopyAvailableFontURLs가 null을 반환했습니다",
                )
            })?;
        // SAFETY: urls는 유효한 CFArray입니다.
        let count = unsafe { CFArrayGetCount(urls.get()) }.max(0);
        // SAFETY: CFURLGetTypeID는 인자 없는 순수 조회입니다.
        let url_type = unsafe { CFURLGetTypeID() };
        let mut paths = Vec::new();
        for index in 0..count {
            // SAFETY: index는 0..count 범위이고, 반환값은 배열이 소유한(빌린) 참조입니다.
            let value = unsafe { CFArrayGetValueAtIndex(urls.get(), index) };
            // SAFETY: value는 배열 원소이므로 null이 아니면 유효한 CF 객체입니다.
            if value.is_null() || unsafe { CFGetTypeID(value) } != url_type {
                continue;
            }
            if let Some(path) = url_to_path(value) {
                paths.push(path);
            }
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::system::{SystemFontDiscovery, SystemFontLimits};
    use stellar_storage::hash::ContentHash;
    use stellar_text::font::FontSource;
    use stellar_text::registry::{FontRegistry, InMemoryFontRegistry};

    /// macOS 기본 설치 face 후보(PostScript 이름).
    const KNOWN_FACES: [&str; 4] = ["Helvetica", "Menlo-Regular", "Geneva", "Courier"];

    fn known_face() -> (&'static str, PathBuf) {
        let locator = CoreTextFontLocator::new();
        KNOWN_FACES
            .iter()
            .find_map(|name| locator.locate_postscript_name(name).map(|p| (*name, p)))
            .expect("알려진 macOS system face가 하나 이상 있어야 합니다")
    }

    #[test]
    fn core_text_lists_absolute_font_files() {
        let files = CoreTextFontLocator::new().font_files().unwrap();
        assert!(!files.is_empty());
        assert!(files.iter().all(|p| p.is_absolute()));
    }

    #[test]
    fn known_system_face_is_registered_from_its_original_file() {
        let (name, path) = known_face();
        // CoreText 가용 목록에 같은 파일이 있습니다.
        let canonical = path.canonicalize().unwrap();
        let listed = CoreTextFontLocator::new().font_files().unwrap();
        assert!(listed
            .iter()
            .any(|p| p.canonicalize().ok().as_ref() == Some(&canonical)));

        let before = std::fs::metadata(&path).unwrap();
        let mut registry = InMemoryFontRegistry::new();
        let mut discovery = SystemFontDiscovery::new(SystemFontLimits::default());
        let catalog = discovery.register_files([path.clone()], &mut registry);
        assert!(catalog.skipped().is_empty(), "{:?}", catalog.skipped());
        let face = catalog
            .faces_with_postscript_name(name)
            .next()
            .expect("PostScript 이름으로 등록된 face");
        // 원래 위치를 그대로 가리키고, file hash는 실제 bytes의 SHA-256입니다.
        assert_eq!(face.path(), path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            face.key().file_hash().sha256_digest(),
            ContentHash::compute(&bytes).digest()
        );
        assert_eq!(registry.lookup(face.key()), Some(face.handle()));
        let registered = registry.face(face.handle()).unwrap();
        assert!(registered.has_source(FontSource::System));
        assert_eq!(registry.resolve(&face.face_ref()), Some(face.handle()));
        // 시스템 파일은 바뀌지 않았습니다(읽기만 함).
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().ok(), after.modified().ok());
    }

    #[test]
    fn unknown_or_invalid_names_are_not_substituted() {
        let locator = CoreTextFontLocator::new();
        assert_eq!(
            locator.locate_postscript_name("StellarNoSuchFace-Regular"),
            None
        );
        assert_eq!(locator.locate_postscript_name(""), None);
        assert_eq!(locator.locate_postscript_name("bad\0name"), None);
        assert_eq!(locator.locate_postscript_name(&"x".repeat(300)), None);
    }

    /// 전체 시스템 font scan(수동 실행: `cargo test -p stellar-project --release -- --ignored full_system_scan --nocapture`).
    #[test]
    #[ignore = "모든 시스템 폰트를 hash하므로 수동 실행"]
    fn full_system_scan() {
        let started = std::time::Instant::now();
        let mut registry = InMemoryFontRegistry::new();
        let mut discovery = SystemFontDiscovery::new(SystemFontLimits::default());
        let catalog = discovery
            .discover(&CoreTextFontLocator::new(), &mut registry)
            .unwrap();
        let first = started.elapsed();
        let again = discovery
            .discover(&CoreTextFontLocator::new(), &mut registry)
            .unwrap();
        println!(
            "files_hashed={} faces={} unique={} skipped={} over_limit={} first={:?} cached={:?}",
            discovery.files_hashed(),
            catalog.faces().len(),
            registry.len(),
            catalog.skipped().len(),
            catalog.files_over_limit(),
            first,
            started.elapsed() - first,
        );
        for skipped in catalog.skipped() {
            println!("skipped {:?}: {}", skipped.path, skipped.reason);
        }
        assert!(!catalog.faces().is_empty());
        assert_eq!(again.faces().len(), catalog.faces().len());
    }
}
