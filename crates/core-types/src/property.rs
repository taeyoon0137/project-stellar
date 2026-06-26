//! PropertyKey와 이름 registry (III.2.1).
//!
//! 속성은 문자열이 아니라 고정 numeric `PropertyKey`로 식별합니다. 사람이 읽는 이름은
//! 별도 [`PropertyNameRegistry`]에 둡니다. 따라서 이름을 바꿔도 numeric key는 유지됩니다.

use std::collections::HashMap;

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
