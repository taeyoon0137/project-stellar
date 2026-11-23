//! 쓰기 v4 파일 키의 이름과 순서(ADR-0077 결정 2).
//!
//! 영속 속성의 파일 키는 document 속성 registry(`frame_metadata`·`payload_metadata`·`modifier_metadata`)의
//! `name`이고, 줄 순서는 registry 순서입니다. 이 모듈은 registry를 조회만 하며 이름을 따로 적지 않습니다.
//! registry가 이름을 CSS longhand로 고치거나(IX.11.2) 묶음 key를 longhand로 나누면(IX.11.22) 파일 키도
//! 그대로 따라갑니다.
//!
//! - 내부 모델이 아직 묶음 값인 Frame `border`(색과 두께 하나)는 파일에서 CSS longhand 열두 줄로 씁니다.
//!   그 이름은 CSS 명세의 longhand 이름([`BORDER_LONGHANDS`])이며 registry에 항목이 생기면 registry 이름이
//!   같아야 합니다(테스트가 검사).
//! - registry에 없는 모델 필드(도형 경계 원점, Path 명령, Image asset·crop, Text 본문, 인스턴스 참조)는 codec
//!   고유 키를 씁니다.

use stellar_core_types::property::PropertyKey;
use stellar_document::frame::keys as frame_keys;
use stellar_document::frame::FramePayload;
use stellar_document::frame_metadata::{frame_field_keys, FRAME_PROPERTIES};
use stellar_document::modifier::ModifierKind;
use stellar_document::modifier_metadata::{properties_of, ModifierPropertyMetadata};
use stellar_document::payload_metadata::{
    image_property, shape_kind_keys, shape_property, IMAGE_PROPERTIES,
};
use stellar_document::shape::ShapeKind;

/// 네 변의 border CSS longhand. 변 순서는 CSS의 top·right·bottom·left입니다.
pub(crate) const BORDER_SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// Frame `border` 한 값이 나뉘는 CSS longhand 열두 개(변마다 width·style·color).
pub(crate) const BORDER_LONGHANDS: [&str; 12] = [
    "border-top-width",
    "border-top-style",
    "border-top-color",
    "border-right-width",
    "border-right-style",
    "border-right-color",
    "border-bottom-width",
    "border-bottom-style",
    "border-bottom-color",
    "border-left-width",
    "border-left-style",
    "border-left-color",
];

/// Frame 파일 줄 하나가 담는 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameLine {
    /// registry key 하나의 값 전체.
    Key(PropertyKey),
    /// 묶음 `border`의 longhand 하나(`BORDER_LONGHANDS`의 index).
    Border(usize),
}

/// Frame 파일 줄: (파일 키, 값). registry 순서이며 payload에 없는 묶음 key(옛 `size`·`corner-radius`)는 건너뜁니다.
pub(crate) fn frame_lines() -> Vec<(&'static str, FrameLine)> {
    let fields = frame_field_keys(&FramePayload::default());
    let mut lines = Vec::new();
    for metadata in &FRAME_PROPERTIES {
        if !fields.contains(&metadata.key) {
            continue;
        }
        if metadata.key == frame_keys::BORDER {
            lines.extend(
                BORDER_LONGHANDS
                    .iter()
                    .enumerate()
                    .map(|(index, name)| (*name, FrameLine::Border(index))),
            );
        } else {
            lines.push((metadata.name, FrameLine::Key(metadata.key)));
        }
    }
    lines
}

/// 도형 종류의 registry 키(registry 순서).
pub(crate) fn shape_lines(kind: ShapeKind) -> Vec<(&'static str, PropertyKey)> {
    shape_kind_keys(kind)
        .iter()
        .map(|key| {
            let metadata =
                shape_property(kind, *key).expect("shape_kind_keys는 registry 항목입니다");
            (metadata.name, *key)
        })
        .collect()
}

/// Image registry 키(registry 순서).
pub(crate) fn image_lines() -> Vec<(&'static str, PropertyKey)> {
    IMAGE_PROPERTIES
        .iter()
        .map(|metadata| {
            debug_assert!(image_property(metadata.key).is_some());
            (metadata.name, metadata.key)
        })
        .collect()
}

/// modifier 종류의 registry 항목(공통 항목 포함, registry 순서). 묶음 key(옛 `shadow-offset`)는 decode 전용이라
/// 파일 줄이 아니며, 나뉜 longhand(`shadow-offset-x`·`shadow-offset-y`)를 씁니다(ADR-0075).
pub(crate) fn modifier_lines(kind: ModifierKind) -> Vec<&'static ModifierPropertyMetadata> {
    properties_of(kind)
        .into_iter()
        .filter(|metadata| !metadata.is_bundle())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn frame_lines_cover_every_payload_field_once_in_registry_order() {
        let lines = frame_lines();
        let names: BTreeSet<_> = lines.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), lines.len(), "키 중복");
        // payload 필드는 모두 한 번씩 나타나고, border는 longhand 열두 줄입니다.
        let fields = frame_field_keys(&FramePayload::default());
        for key in fields {
            let count = lines
                .iter()
                .filter(|(_, line)| *line == FrameLine::Key(key))
                .count();
            let expected = usize::from(key != frame_keys::BORDER);
            assert_eq!(count, expected, "{key:?}");
        }
        assert_eq!(
            lines
                .iter()
                .filter(|(_, line)| matches!(line, FrameLine::Border(_)))
                .count(),
            12
        );
        // 이름은 registry에서 읽은 값입니다.
        for (name, line) in &lines {
            if let FrameLine::Key(key) = line {
                assert_eq!(
                    stellar_document::frame_metadata::frame_property(*key)
                        .unwrap()
                        .name,
                    *name
                );
            }
        }
    }

    #[test]
    fn border_longhands_do_not_collide_with_other_registry_names() {
        // registry가 border longhand를 항목으로 가지게 되면 그 이름은 CSS longhand 이름과 같아야 하고,
        // 다른 key가 같은 이름을 쓰면 안 됩니다.
        for metadata in &FRAME_PROPERTIES {
            if BORDER_LONGHANDS.contains(&metadata.name) {
                assert_ne!(metadata.key, frame_keys::BORDER);
            }
        }
        for kind in ShapeKind::ALL {
            let names: BTreeSet<_> = shape_lines(kind).iter().map(|(n, _)| *n).collect();
            assert_eq!(names.len(), shape_lines(kind).len());
        }
        assert_eq!(image_lines().len(), IMAGE_PROPERTIES.len());
    }
}
