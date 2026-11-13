//! 폰트 face registry (X.8.2).
//!
//! - [`FontFaceKey`]: face 정체성. 폰트 파일 콘텐츠 해시 + face index입니다. 같은 bytes의 파일은
//!   출처(Project·System·External)나 경로와 무관하게 같은 face로 봅니다. variation axes는 face를
//!   바꾸지 않는 instance 설정이라 key에 넣지 않습니다.
//! - [`FontHandle`]: registry가 발급하는 런타임 handle입니다. 저장하지 않으며 같은 registry
//!   안에서만 의미가 있습니다.
//! - [`FontRegistry`]: face 등록·조회 인터페이스입니다. 같은 key를 다시 등록하면 기존 handle을
//!   돌려줍니다(dedup). 플랫폼 font discovery(Phase XIII)는 이 인터페이스로 face를 등록합니다.
//! - [`InMemoryFontRegistry`]: 결정론적 기본 구현입니다. handle은 최초 등록 순서대로 0부터
//!   발급되고, 조회 경로는 `BTreeMap`이라 실행마다 같은 결과를 냅니다.
//!
//! registry는 폰트 bytes를 읽거나 hash를 계산하지 않습니다. 호출자가 계산한 key만 다룹니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::font::{FontFaceRef, FontFileHash, FontSource};

/// face 정체성: 폰트 파일 콘텐츠 해시 + face index (X.8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFaceKey {
    file_hash: FontFileHash,
    face_index: u32,
}

impl FontFaceKey {
    /// file hash와 face index로 key를 만듭니다.
    pub const fn new(file_hash: FontFileHash, face_index: u32) -> FontFaceKey {
        FontFaceKey {
            file_hash,
            face_index,
        }
    }

    /// 폰트 파일 콘텐츠 해시.
    pub const fn file_hash(&self) -> FontFileHash {
        self.file_hash
    }

    /// 파일 안의 face index.
    pub const fn face_index(&self) -> u32 {
        self.face_index
    }
}

impl From<&FontFaceRef> for FontFaceKey {
    fn from(face: &FontFaceRef) -> FontFaceKey {
        FontFaceKey::new(face.file_hash(), face.face_index())
    }
}

impl fmt::Display for FontFaceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.file_hash, self.face_index)
    }
}

/// registry가 발급한 런타임 face handle. 저장하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontHandle(u32);

impl FontHandle {
    /// raw 값(진단·테스트용).
    pub const fn raw(&self) -> u32 {
        self.0
    }
}

/// 등록된 face 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFace {
    key: FontFaceKey,
    sources: BTreeSet<FontSource>,
}

impl RegisteredFace {
    /// face 정체성.
    pub fn key(&self) -> FontFaceKey {
        self.key
    }

    /// 이 face를 제공한다고 등록된 출처(tag 오름차순).
    pub fn sources(&self) -> impl Iterator<Item = FontSource> + '_ {
        self.sources.iter().copied()
    }

    /// `source`가 이 face를 제공하는지.
    pub fn has_source(&self, source: FontSource) -> bool {
        self.sources.contains(&source)
    }
}

/// registry 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRegistryError {
    /// 더 이상 새 handle을 발급할 수 없습니다.
    CapacityExceeded {
        /// registry가 담을 수 있는 최대 face 수.
        max_faces: u32,
    },
}

impl fmt::Display for FontRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontRegistryError::CapacityExceeded { max_faces } => {
                write!(f, "font registry가 가득 찼습니다(최대 {max_faces}개 face)")
            }
        }
    }
}

impl std::error::Error for FontRegistryError {}

/// 폰트 face 등록·조회 인터페이스 (X.8.2).
///
/// 구현은 같은 [`FontFaceKey`]에 대해 항상 같은 [`FontHandle`]을 돌려줘야 합니다. 출처가 달라도
/// 파일 hash와 face index가 같으면 같은 face이며, 추가 출처만 기록합니다.
pub trait FontRegistry {
    /// `source`가 제공하는 face `key`를 등록하고 handle을 돌려줍니다. 이미 있으면 기존 handle입니다.
    fn register(
        &mut self,
        source: FontSource,
        key: FontFaceKey,
    ) -> Result<FontHandle, FontRegistryError>;

    /// key에 해당하는 handle. 등록되지 않았으면 `None`.
    fn lookup(&self, key: FontFaceKey) -> Option<FontHandle>;

    /// handle에 해당하는 face. 이 registry가 발급하지 않은 handle이면 `None`.
    fn face(&self, handle: FontHandle) -> Option<&RegisteredFace>;

    /// 등록된 고유 face 수.
    fn len(&self) -> usize;

    /// 등록된 face가 없는지.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// face 참조가 가리키는 등록 face의 handle. variation axes와 요청 출처는 face 정체성에 영향을
    /// 주지 않습니다.
    fn resolve(&self, face: &FontFaceRef) -> Option<FontHandle> {
        self.lookup(FontFaceKey::from(face))
    }
}

/// 결정론적 메모리 registry (X.8.2 기본 구현).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryFontRegistry {
    faces: Vec<RegisteredFace>,
    index: BTreeMap<FontFaceKey, FontHandle>,
    max_faces: u32,
}

impl Default for InMemoryFontRegistry {
    fn default() -> InMemoryFontRegistry {
        InMemoryFontRegistry::new()
    }
}

impl InMemoryFontRegistry {
    /// 빈 registry를 만듭니다. handle 공간(`u32`) 전체를 사용할 수 있습니다.
    pub fn new() -> InMemoryFontRegistry {
        InMemoryFontRegistry::with_max_faces(u32::MAX)
    }

    /// 최대 face 수를 제한한 빈 registry를 만듭니다.
    pub fn with_max_faces(max_faces: u32) -> InMemoryFontRegistry {
        InMemoryFontRegistry {
            faces: Vec::new(),
            index: BTreeMap::new(),
            max_faces,
        }
    }

    /// 등록된 face를 handle 순서(최초 등록 순서)로 순회합니다.
    pub fn iter(&self) -> impl Iterator<Item = (FontHandle, &RegisteredFace)> + '_ {
        self.faces
            .iter()
            .enumerate()
            .map(|(i, face)| (FontHandle(i as u32), face))
    }
}

impl FontRegistry for InMemoryFontRegistry {
    fn register(
        &mut self,
        source: FontSource,
        key: FontFaceKey,
    ) -> Result<FontHandle, FontRegistryError> {
        if let Some(&handle) = self.index.get(&key) {
            self.faces[handle.0 as usize].sources.insert(source);
            return Ok(handle);
        }
        let next = u32::try_from(self.faces.len())
            .ok()
            .filter(|n| *n < self.max_faces)
            .ok_or(FontRegistryError::CapacityExceeded {
                max_faces: self.max_faces,
            })?;
        let handle = FontHandle(next);
        self.faces.push(RegisteredFace {
            key,
            sources: BTreeSet::from([source]),
        });
        self.index.insert(key, handle);
        Ok(handle)
    }

    fn lookup(&self, key: FontFaceKey) -> Option<FontHandle> {
        self.index.get(&key).copied()
    }

    fn face(&self, handle: FontHandle) -> Option<&RegisteredFace> {
        self.faces.get(handle.0 as usize)
    }

    fn len(&self) -> usize {
        self.faces.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::tests::{axis, hash};

    fn key(fill: u8, index: u32) -> FontFaceKey {
        FontFaceKey::new(hash(fill), index)
    }

    #[test]
    fn duplicate_register_returns_same_handle() {
        let mut registry = InMemoryFontRegistry::new();
        let first = registry
            .register(FontSource::Project, key(1, 0))
            .expect("등록");
        let again = registry
            .register(FontSource::Project, key(1, 0))
            .expect("재등록");
        assert_eq!(first, again);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn same_hash_and_index_from_other_source_dedups_and_records_source() {
        let mut registry = InMemoryFontRegistry::new();
        let project = registry
            .register(FontSource::Project, key(7, 2))
            .expect("등록");
        let system = registry
            .register(FontSource::System, key(7, 2))
            .expect("등록");
        assert_eq!(project, system);
        assert_eq!(registry.len(), 1);
        let face = registry.face(project).expect("발급한 handle");
        assert_eq!(face.key(), key(7, 2));
        assert_eq!(
            face.sources().collect::<Vec<_>>(),
            vec![FontSource::Project, FontSource::System]
        );
        assert!(face.has_source(FontSource::System));
        assert!(!face.has_source(FontSource::External));
    }

    #[test]
    fn different_index_or_hash_gets_distinct_handles_in_registration_order() {
        let mut registry = InMemoryFontRegistry::new();
        let a = registry.register(FontSource::System, key(3, 0)).expect("a");
        let b = registry.register(FontSource::System, key(3, 1)).expect("b");
        let c = registry
            .register(FontSource::External, key(4, 0))
            .expect("c");
        assert_eq!((a.raw(), b.raw(), c.raw()), (0, 1, 2));
        assert_eq!(registry.len(), 3);
        let order: Vec<(u32, FontFaceKey)> = registry
            .iter()
            .map(|(handle, face)| (handle.raw(), face.key()))
            .collect();
        assert_eq!(order, vec![(0, key(3, 0)), (1, key(3, 1)), (2, key(4, 0))]);
    }

    #[test]
    fn resolve_ignores_variations_and_requested_source() {
        let mut registry = InMemoryFontRegistry::new();
        let handle = registry
            .register(FontSource::System, key(5, 0))
            .expect("등록");
        let bold = FontFaceRef::new(FontSource::Project, hash(5), 0, vec![axis("wght", 700.0)]);
        let light = FontFaceRef::new(FontSource::System, hash(5), 0, vec![axis("wght", 300.0)]);
        assert_eq!(registry.resolve(&bold), Some(handle));
        assert_eq!(registry.resolve(&light), Some(handle));
        let other_face = FontFaceRef::new(FontSource::System, hash(5), 1, vec![]);
        assert_eq!(registry.resolve(&other_face), None);
        assert_eq!(FontFaceKey::from(&bold), key(5, 0));
    }

    #[test]
    fn lookup_and_face_for_unknown_entries() {
        let mut registry = InMemoryFontRegistry::default();
        assert!(registry.is_empty());
        assert_eq!(registry.lookup(key(1, 0)), None);
        assert_eq!(registry.face(FontHandle(0)), None);
        let handle = registry
            .register(FontSource::Project, key(1, 0))
            .expect("등록");
        assert_eq!(registry.lookup(key(1, 0)), Some(handle));
        assert!(!registry.is_empty());
        assert_eq!(registry.face(FontHandle(1)), None);
    }

    #[test]
    fn works_through_trait_object() {
        fn register_twice(registry: &mut dyn FontRegistry) -> (FontHandle, FontHandle) {
            let a = registry
                .register(FontSource::Project, key(9, 0))
                .expect("a");
            let b = registry
                .register(FontSource::External, key(9, 0))
                .expect("b");
            (a, b)
        }
        let mut registry = InMemoryFontRegistry::new();
        let (a, b) = register_twice(&mut registry);
        assert_eq!(a, b);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn capacity_limit_rejects_new_faces_but_keeps_dedup() {
        let mut registry = InMemoryFontRegistry::with_max_faces(1);
        let handle = registry
            .register(FontSource::Project, key(1, 0))
            .expect("등록");
        assert_eq!(
            registry.register(FontSource::Project, key(2, 0)),
            Err(FontRegistryError::CapacityExceeded { max_faces: 1 })
        );
        // 이미 있는 face 재등록은 용량과 무관하게 같은 handle.
        assert_eq!(registry.register(FontSource::System, key(1, 0)), Ok(handle));
        assert_eq!(registry.len(), 1);
        assert!(InMemoryFontRegistry::with_max_faces(0)
            .register(FontSource::Project, key(1, 0))
            .is_err());
        assert!(FontRegistryError::CapacityExceeded { max_faces: 1 }
            .to_string()
            .contains("최대 1개"));
    }

    #[test]
    fn key_display_includes_hash_and_index() {
        assert_eq!(
            key(0, 3).to_string(),
            format!("sha256:{}#3", "0".repeat(64))
        );
    }
}
