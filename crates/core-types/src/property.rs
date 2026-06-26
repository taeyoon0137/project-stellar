//! PropertyKey와 이름 registry (III.2.1).
//!
//! 속성은 문자열이 아니라 고정 numeric `PropertyKey`로 식별합니다. 사람이 읽는 이름은
//! 별도 [`PropertyNameRegistry`]에 둡니다. 따라서 이름을 바꿔도 numeric key는 유지됩니다.

use std::collections::HashMap;

use crate::ids::StableId;

/// 고정 numeric 속성 키. 문자열이 아닌 안정적인 식별자입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PropertyKey(
    /// numeric key 값.
    pub u32,
);

/// `PropertyKey` ↔ 사람이 읽는 이름 registry.
///
/// 이름과 키를 분리해 두므로, 이름을 변경해도 키는 바뀌지 않습니다. 이름→키 역조회도 제공합니다.
#[derive(Debug, Default)]
pub struct PropertyNameRegistry {
    name_by_key: HashMap<PropertyKey, String>,
    key_by_name: HashMap<String, PropertyKey>,
}

impl PropertyNameRegistry {
    /// 빈 registry를 만듭니다.
    pub fn new() -> PropertyNameRegistry {
        PropertyNameRegistry {
            name_by_key: HashMap::new(),
            key_by_name: HashMap::new(),
        }
    }

    /// 키에 이름을 등록합니다. 키에 이미 이름이 있으면 갱신합니다(키는 유지).
    pub fn register(&mut self, key: PropertyKey, name: &str) {
        if let Some(old) = self.name_by_key.get(&key) {
            let old = old.clone();
            self.key_by_name.remove(&old);
        }
        self.name_by_key.insert(key, name.to_string());
        self.key_by_name.insert(name.to_string(), key);
    }

    /// 키의 이름을 변경합니다. 키는 유지됩니다. 키가 등록돼 있지 않으면 `false`.
    pub fn rename(&mut self, key: PropertyKey, new_name: &str) -> bool {
        if !self.name_by_key.contains_key(&key) {
            return false;
        }
        self.register(key, new_name);
        true
    }

    /// 키의 현재 이름.
    pub fn name_of(&self, key: PropertyKey) -> Option<&str> {
        self.name_by_key.get(&key).map(String::as_str)
    }

    /// 이름으로 키를 역조회합니다.
    pub fn key_of(&self, name: &str) -> Option<PropertyKey> {
        self.key_by_name.get(name).copied()
    }
}

/// `PropertyPath`의 마지막 선택 segment: 배열 index 또는 map key(III.2.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSegment {
    /// 배열 index.
    Index(u32),
    /// map/dictionary key.
    Key(String),
}

/// 속성 경로: 루트 `StableId` + `PropertyKey` 시퀀스 + 선택적 index/key segment(III.2.2).
///
/// canonical 비교·정렬이 가능하며(`Eq`/`Ord` 파생), [`PropertyPath::to_canonical_bytes`]는
/// 동일 의미 경로에 대해 항상 동일한 바이트열을 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PropertyPath {
    root: StableId,
    keys: Vec<PropertyKey>,
    tail: Option<PathSegment>,
}

impl PropertyPath {
    /// 루트만 가진 경로를 만듭니다.
    pub fn new(root: StableId) -> PropertyPath {
        PropertyPath {
            root,
            keys: Vec::new(),
            tail: None,
        }
    }

    /// 루트와 키 시퀀스로 경로를 만듭니다.
    pub fn with_keys(root: StableId, keys: Vec<PropertyKey>) -> PropertyPath {
        PropertyPath {
            root,
            keys,
            tail: None,
        }
    }

    /// 키를 끝에 추가합니다(빌더).
    pub fn push(mut self, key: PropertyKey) -> PropertyPath {
        self.keys.push(key);
        self
    }

    /// 마지막 segment를 배열 index로 지정합니다(빌더).
    pub fn with_index(mut self, index: u32) -> PropertyPath {
        self.tail = Some(PathSegment::Index(index));
        self
    }

    /// 마지막 segment를 map key로 지정합니다(빌더).
    pub fn with_key_segment(mut self, key: &str) -> PropertyPath {
        self.tail = Some(PathSegment::Key(key.to_string()));
        self
    }

    /// 루트 식별자.
    pub fn root(&self) -> StableId {
        self.root
    }

    /// 키 시퀀스.
    pub fn keys(&self) -> &[PropertyKey] {
        &self.keys
    }

    /// 마지막 선택 segment.
    pub fn tail(&self) -> Option<&PathSegment> {
        self.tail.as_ref()
    }

    /// canonical 바이트 직렬화. 동일 의미 경로는 동일 바이트열을 만듭니다.
    ///
    /// 레이아웃: root(16) ++ keys_len(u32 BE) ++ keys(각 u32 BE) ++ tail.
    /// tail은 discriminant(0=none,1=index,2=key) ++ payload(index: u32 BE, key: len u32 BE ++ utf8).
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.root.to_bytes());
        out.extend_from_slice(&(self.keys.len() as u32).to_be_bytes());
        for key in &self.keys {
            out.extend_from_slice(&key.0.to_be_bytes());
        }
        match &self.tail {
            None => out.push(0),
            Some(PathSegment::Index(index)) => {
                out.push(1);
                out.extend_from_slice(&index.to_be_bytes());
            }
            Some(PathSegment::Key(key)) => {
                out.push(2);
                out.extend_from_slice(&(key.len() as u32).to_be_bytes());
                out.extend_from_slice(key.as_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_key_survives_rename() {
        let mut registry = PropertyNameRegistry::new();
        let key = PropertyKey(5);
        registry.register(key, "opacity");

        assert_eq!(registry.name_of(key), Some("opacity"));
        assert_eq!(registry.key_of("opacity"), Some(key));

        // 이름 변경 후에도 numeric key는 그대로 5.
        assert!(registry.rename(key, "alpha"));
        assert_eq!(registry.name_of(key), Some("alpha"));
        assert_eq!(registry.key_of("alpha"), Some(PropertyKey(5)));
        // 옛 이름은 더 이상 역조회되지 않음.
        assert_eq!(registry.key_of("opacity"), None);
        // 키 값 자체(5)는 불변.
        assert_eq!(key.0, 5);
    }

    #[test]
    fn same_meaning_path_same_bytes_and_eq() {
        let root = StableId::new_v7(42, 0, 7);
        // 같은 경로를 두 가지 빌더 경로로 구성.
        let a = PropertyPath::with_keys(root, vec![PropertyKey(1), PropertyKey(2)]).with_index(3);
        let b = PropertyPath::new(root)
            .push(PropertyKey(1))
            .push(PropertyKey(2))
            .with_index(3);
        assert_eq!(a, b);
        assert_eq!(a.to_canonical_bytes(), b.to_canonical_bytes());
    }

    #[test]
    fn different_paths_differ_in_bytes_and_order() {
        let root = StableId::new_v7(42, 0, 7);
        let base = PropertyPath::with_keys(root, vec![PropertyKey(1)]);
        let longer = PropertyPath::with_keys(root, vec![PropertyKey(1), PropertyKey(2)]);
        let indexed = PropertyPath::with_keys(root, vec![PropertyKey(1)]).with_index(0);
        let keyed = PropertyPath::with_keys(root, vec![PropertyKey(1)]).with_key_segment("x");

        // 서로 다른 경로는 바이트가 다름.
        assert_ne!(base.to_canonical_bytes(), longer.to_canonical_bytes());
        assert_ne!(base.to_canonical_bytes(), indexed.to_canonical_bytes());
        assert_ne!(indexed.to_canonical_bytes(), keyed.to_canonical_bytes());

        // canonical 정렬 가능(총 순서).
        let mut paths = [keyed.clone(), base.clone(), longer.clone(), indexed.clone()];
        paths.sort();
        // 정렬은 결정적이어야 함(두 번 정렬해도 같음).
        let mut again = paths.clone();
        again.sort();
        assert_eq!(paths, again);
    }

    #[test]
    fn accessors_return_components() {
        let root = StableId::new_v7(1, 0, 1);
        let path = PropertyPath::with_keys(root, vec![PropertyKey(9)]).with_key_segment("k");
        assert_eq!(path.root(), root);
        assert_eq!(path.keys(), &[PropertyKey(9)]);
        assert_eq!(path.tail(), Some(&PathSegment::Key("k".to_string())));
    }

    #[test]
    fn rename_unknown_key_fails() {
        let mut registry = PropertyNameRegistry::new();
        assert!(!registry.rename(PropertyKey(99), "x"));
    }

    #[test]
    fn register_overwrites_name_keeps_key() {
        let mut registry = PropertyNameRegistry::new();
        registry.register(PropertyKey(1), "fill");
        registry.register(PropertyKey(1), "fillColor");
        assert_eq!(registry.name_of(PropertyKey(1)), Some("fillColor"));
        assert_eq!(registry.key_of("fillColor"), Some(PropertyKey(1)));
        assert_eq!(registry.key_of("fill"), None);
    }
}
