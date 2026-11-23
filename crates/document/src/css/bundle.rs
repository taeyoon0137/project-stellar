//! 묶음 key와 longhand key 대응 (IX.11.22, ADR-0024 결정 14, ADR-0075 결정 1).
//!
//! 값 여러 개를 담던 합성 key(Frame `size` 0x0101, Frame `corner_radius` 0x0106, Shadow offset 0x1092)는
//! longhand key로 나눴습니다. 옛 key는 **재사용하지 않는 묶음 key**로 registry에 남아 이전 `SetProperty`·
//! `SetModifierProperty` bytes와 history journal을 그대로 decode·실행합니다(결과는 longhand 전체를 설정한 것과
//! 같음). 새 command, 속성 패널 묶음 편집, semantic snapshot, merge는 longhand key를 씁니다.
//!
//! 저장된 참조(ValueBinding target, View Variant·Component variant override property patch)가 묶음 key를
//! 가리키면 변환하지 않고 [`bundle_key_reference`] 진단을 냅니다. 그 참조가 longhand 하나를 뜻하는지 전체를
//! 뜻하는지 저장 bytes만으로는 정할 수 없어서입니다.

use std::fmt;

use stellar_core_types::property::{PropertyKey, PropertyPath};

use crate::frame_metadata::FRAME_PROPERTIES;
use crate::modifier_metadata::MODIFIER_PROPERTIES;

/// 묶음 key 진단 코드.
pub const BUNDLE_KEY_REFERENCE: &str = "bundle-key-reference";

/// 묶음 key 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleKey {
    /// 묶음 key.
    pub key: PropertyKey,
    /// registry 이름.
    pub name: &'static str,
    /// 나뉜 longhand key(값 순서: Frame 크기는 width, height, 모서리는 TL, TR, BR, BL, offset은 x, y).
    pub longhands: &'static [PropertyKey],
}

/// 모든 묶음 key(Frame registry 다음 modifier registry, key 순).
pub fn bundle_keys() -> Vec<BundleKey> {
    let frame = FRAME_PROPERTIES
        .iter()
        .filter(|m| m.is_bundle())
        .map(|m| BundleKey {
            key: m.key,
            name: m.name,
            longhands: m.longhands,
        });
    let modifier = MODIFIER_PROPERTIES
        .iter()
        .filter(|m| m.is_bundle())
        .map(|m| BundleKey {
            key: m.key,
            name: m.name,
            longhands: m.longhands,
        });
    frame.chain(modifier).collect()
}

/// `key`가 묶음 key이면 그 longhand key.
pub fn bundle_longhands(key: PropertyKey) -> Option<&'static [PropertyKey]> {
    FRAME_PROPERTIES
        .iter()
        .find(|m| m.key == key && m.is_bundle())
        .map(|m| m.longhands)
        .or_else(|| {
            MODIFIER_PROPERTIES
                .iter()
                .find(|m| m.key == key && m.is_bundle())
                .map(|m| m.longhands)
        })
}

/// 저장된 참조가 묶음 key를 가리킨다는 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleKeyReference {
    /// 참조 경로.
    pub path: PropertyPath,
    /// 경로 안의 묶음 key.
    pub key: PropertyKey,
    /// 대신 써야 할 longhand key.
    pub longhands: &'static [PropertyKey],
}

impl BundleKeyReference {
    /// 진단 코드.
    pub fn code(&self) -> &'static str {
        BUNDLE_KEY_REFERENCE
    }
}

impl fmt::Display for BundleKeyReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let longhands: Vec<String> = self
            .longhands
            .iter()
            .map(|k| format!("0x{:04x}", k.0))
            .collect();
        write!(
            f,
            "{BUNDLE_KEY_REFERENCE}: 0x{:04x}는 묶음 key입니다. longhand key({})로 다시 지정하세요(자동 변환하지 않음)",
            self.key.0,
            longhands.join(", ")
        )
    }
}

/// `path`의 key 중 묶음 key가 있으면 진단을 돌려줍니다. 경로를 바꾸지 않습니다.
pub fn bundle_key_reference(path: &PropertyPath) -> Option<BundleKeyReference> {
    path.keys().iter().find_map(|key| {
        bundle_longhands(*key).map(|longhands| BundleKeyReference {
            path: path.clone(),
            key: *key,
            longhands,
        })
    })
}

/// 여러 저장 참조 경로를 검사합니다(입력 순서대로).
pub fn bundle_key_references<'a>(
    paths: impl IntoIterator<Item = &'a PropertyPath>,
) -> Vec<BundleKeyReference> {
    paths.into_iter().filter_map(bundle_key_reference).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::keys as frame_keys;
    use crate::modifier_metadata::keys as modifier_keys;
    use stellar_core_types::ids::StableId;

    #[test]
    fn bundle_keys_are_the_three_legacy_composites() {
        let keys: Vec<(u32, usize)> = bundle_keys()
            .iter()
            .map(|b| (b.key.0, b.longhands.len()))
            .collect();
        assert_eq!(keys, vec![(0x0101, 2), (0x0106, 4), (0x1092, 2)]);
        assert_eq!(
            bundle_longhands(frame_keys::SIZE),
            Some(&[frame_keys::WIDTH, frame_keys::HEIGHT][..])
        );
        assert_eq!(bundle_longhands(frame_keys::WIDTH), None);
        assert_eq!(
            bundle_longhands(modifier_keys::SHADOW_OFFSET),
            Some(&modifier_keys::SHADOW_OFFSET_LONGHANDS[..])
        );
    }

    #[test]
    fn stored_references_to_bundle_keys_are_diagnosed_not_rewritten() {
        // 검증 기준(IX.11.22): 묶음 key 참조 진단 fixture.
        let node = StableId::new_v7(7, 0, 1);
        let modifier = StableId::new_v7(7, 0, 2);
        // ValueBinding target(Frame size), Variant property patch(모서리 묶음), modifier offset 묶음.
        let binding_target = PropertyPath::with_keys(node, vec![frame_keys::SIZE]);
        let variant_patch = PropertyPath::with_keys(node, vec![frame_keys::CORNER_RADIUS]);
        let shadow = PropertyPath::with_keys(modifier, vec![modifier_keys::SHADOW_OFFSET]);
        let longhand = PropertyPath::with_keys(node, vec![frame_keys::BORDER_TOP_LEFT_RADIUS]);
        let found = bundle_key_references([&binding_target, &longhand, &variant_patch, &shadow]);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].path, binding_target, "경로는 그대로");
        assert_eq!(found[0].code(), "bundle-key-reference");
        assert_eq!(found[1].key, frame_keys::CORNER_RADIUS);
        assert_eq!(found[1].longhands, &frame_keys::BORDER_RADIUS_LONGHANDS);
        assert_eq!(found[2].key, modifier_keys::SHADOW_OFFSET);
        assert_eq!(
            found[0].to_string(),
            "bundle-key-reference: 0x0101는 묶음 key입니다. longhand key(0x0108, 0x0109)로 다시 지정하세요(자동 변환하지 않음)"
        );
        assert!(bundle_key_reference(&longhand).is_none());
    }
}
