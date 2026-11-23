//! 속성 registry 통합 view (IX.11.2·IX.11.3).
//!
//! 종류별 registry(Frame `frame_metadata`, Modifier `modifier_metadata`, Shape·Path·Text·Image
//! `payload_metadata`)를 범위([`PropertyScope`])와 함께 한 목록으로 읽습니다. CLI·MCP·codegen·텍스트 원본이
//! 속성 이름과 CSS 매핑을 여기서 읽고, semantic diff 표시 문자열도 같은 이름을 씁니다.
//!
//! [`snapshot_text`]는 registry 전체를 결정론적 텍스트로 냅니다. golden(`registry_snapshot.txt`)과 속성 모델
//! 표(`docs/design/property_model.md` 5장·9장)가 이 텍스트와 맞는지 test가 확인합니다.

use stellar_core_types::property::PropertyKey;

use super::mapping::CssMapping;
use crate::frame_metadata::FRAME_PROPERTIES;
use crate::modifier_metadata::MODIFIER_PROPERTIES;
use crate::payload_metadata::{
    PayloadPropertyMetadata, IMAGE_PROPERTIES, NODE_PROPERTIES, PATH_PROPERTIES, SHAPE_PROPERTIES,
    TEXT_PROPERTIES,
};

/// 속성이 속한 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropertyScope {
    /// 모든 node의 공통 필드(`0x00xx`).
    Node,
    /// Frame payload(`0x01xx`).
    Frame,
    /// Shape payload(`0x03xx`).
    Shape,
    /// Path payload(`0x04xx`).
    Path,
    /// Text payload(`0x05xx`).
    Text,
    /// Image payload(`0x06xx`).
    Image,
    /// modifier(`0x10xx`).
    Modifier,
}

impl PropertyScope {
    /// 정의된 모든 범위(표시 순).
    pub const ALL: [PropertyScope; 7] = [
        PropertyScope::Node,
        PropertyScope::Frame,
        PropertyScope::Shape,
        PropertyScope::Path,
        PropertyScope::Text,
        PropertyScope::Image,
        PropertyScope::Modifier,
    ];

    /// 표기 이름.
    pub fn name(self) -> &'static str {
        match self {
            PropertyScope::Node => "node",
            PropertyScope::Frame => "frame",
            PropertyScope::Shape => "shape",
            PropertyScope::Path => "path",
            PropertyScope::Text => "text",
            PropertyScope::Image => "image",
            PropertyScope::Modifier => "modifier",
        }
    }
}

fn payload_entries(
    scope: PropertyScope,
    table: &'static [PayloadPropertyMetadata],
) -> impl Iterator<Item = RegistryEntry> {
    table.iter().map(move |m| RegistryEntry {
        scope,
        key: m.key,
        name: m.name,
        css: m.css,
        bundle: &[],
    })
}

/// registry 항목 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryEntry {
    /// 범위.
    pub scope: PropertyScope,
    /// 고정 numeric key.
    pub key: PropertyKey,
    /// CSS longhand 이름 또는 Stellar 이름.
    pub name: &'static str,
    /// CSS 매핑.
    pub css: CssMapping,
    /// 묶음 key면 나뉜 longhand key.
    pub bundle: &'static [PropertyKey],
}

impl RegistryEntry {
    /// 표시 문자열: 보통 항목은 `name`, 묶음 key는 대응 longhand 이름 목록입니다.
    pub fn display_label(&self) -> String {
        if self.bundle.is_empty() {
            return self.name.to_string();
        }
        self.bundle
            .iter()
            .filter_map(|key| entry(self.scope, *key))
            .map(|e| e.name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 모든 registry 항목(범위 순, 범위 안 key 순).
pub fn entries() -> Vec<RegistryEntry> {
    let node = payload_entries(PropertyScope::Node, &NODE_PROPERTIES);
    let frame = FRAME_PROPERTIES.iter().map(|m| RegistryEntry {
        scope: PropertyScope::Frame,
        key: m.key,
        name: m.name,
        css: m.css,
        bundle: m.longhands,
    });
    let modifier = MODIFIER_PROPERTIES.iter().map(|m| RegistryEntry {
        scope: PropertyScope::Modifier,
        key: m.key,
        name: m.name,
        css: m.css,
        bundle: m.longhands,
    });
    node.chain(frame)
        .chain(payload_entries(PropertyScope::Shape, &SHAPE_PROPERTIES))
        .chain(payload_entries(PropertyScope::Path, &PATH_PROPERTIES))
        .chain(payload_entries(PropertyScope::Text, &TEXT_PROPERTIES))
        .chain(payload_entries(PropertyScope::Image, &IMAGE_PROPERTIES))
        .chain(modifier)
        .collect()
}

/// `scope`의 `key` 항목.
pub fn entry(scope: PropertyScope, key: PropertyKey) -> Option<RegistryEntry> {
    entries()
        .into_iter()
        .find(|e| e.scope == scope && e.key == key)
}

/// `scope`에서 이름이 `name`인 항목.
pub fn entry_by_name(scope: PropertyScope, name: &str) -> Option<RegistryEntry> {
    entries()
        .into_iter()
        .find(|e| e.scope == scope && e.name == name)
}

/// key의 표시 문자열(범위와 무관하게 key가 유일한 대역 규칙을 씁니다). 모르는 key는 `None`.
pub fn display_label(key: PropertyKey) -> Option<String> {
    entries()
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.display_label())
}

/// registry 전체의 결정론적 텍스트(golden과 속성 모델 표 비교용). 한 줄에 항목 하나:
/// `scope 0xKEY name | status [flags] | longhands | grammar | rules`.
pub fn snapshot_text() -> String {
    let mut out = String::new();
    for e in entries() {
        let flags = e.css.flags();
        let rules: Vec<String> = e
            .css
            .rules
            .iter()
            .map(|r| format!("{}={}", r.variant, r.status.name()))
            .collect();
        out.push_str(&format!(
            "{} 0x{:04x} {} | {}{} | {} | {} | {}\n",
            e.scope.name(),
            e.key.0,
            e.name,
            e.css.status.name(),
            if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            },
            if e.css.longhands.is_empty() {
                "-".to_string()
            } else {
                e.css.longhands.join(", ")
            },
            e.css.grammar.map_or("-", |g| g.name()),
            if rules.is_empty() {
                "-".to_string()
            } else {
                rules.join(", ")
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::mapping::MappingStatus;
    use crate::css::mapping::{is_css_property_name, is_known_longhand, shorthand_longhands};
    use std::collections::{BTreeMap, BTreeSet};

    /// CSS 이름이 아닌 Stellar 이름을 쓰는 항목(이름 이유: CSS 속성 하나로 대응하지 않음). 상태가
    /// `stellar-only`인 항목과 묶음 key는 따로 허용합니다.
    const STELLAR_NAMED: &[&str] = &[
        "visible",
        "path-data",
        "clipping",
        "shadow-color",
        "shadow-blur",
        "shadow-offset-x",
        "shadow-offset-y",
        "blur-radius",
    ];

    #[test]
    fn every_entry_has_a_css_name_or_a_stellar_name_and_a_complete_mapping() {
        // 검증 기준(IX.11.2): 매핑 누락 0.
        let mut scoped = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for e in entries() {
            assert!(is_css_property_name(e.name), "이름 형식: {}", e.name);
            assert!(
                scoped.insert((e.scope, e.name)),
                "범위 안 이름 중복: {}",
                e.name
            );
            assert!(keys.insert(e.key), "key 중복: 0x{:04x}", e.key.0);
            let css_named = is_known_longhand(e.name) || shorthand_longhands(e.name).is_some();
            assert!(
                css_named
                    || e.css.status == MappingStatus::StellarOnly
                    || !e.bundle.is_empty()
                    || STELLAR_NAMED.contains(&e.name),
                "CSS 이름도 Stellar 이름 허용 목록도 아님: {}",
                e.name
            );
            if e.css.status != MappingStatus::StellarOnly {
                assert!(!e.css.longhands.is_empty(), "CSS 대상 없음: {}", e.name);
            }
            for longhand in e.css.longhands {
                assert!(is_known_longhand(longhand), "{} → {longhand}", e.name);
            }
            if is_known_longhand(e.name) && e.css.status != MappingStatus::StellarOnly {
                assert!(e.css.longhands.contains(&e.name), "{}", e.name);
            }
        }
        // 묶음 key의 표시 문자열은 longhand 목록입니다.
        assert_eq!(
            display_label(crate::frame::keys::SIZE).as_deref(),
            Some("width, height")
        );
        assert_eq!(
            display_label(crate::frame::keys::CORNER_RADIUS).as_deref(),
            Some(
                "border-top-left-radius, border-top-right-radius, border-bottom-right-radius, border-bottom-left-radius"
            )
        );
        assert_eq!(
            display_label(crate::modifier_metadata::keys::SHADOW_OFFSET).as_deref(),
            Some("shadow-offset-x, shadow-offset-y")
        );
        assert_eq!(display_label(PropertyKey(0xFFFF)), None);
    }

    #[test]
    fn clipping_is_a_single_stellar_property_with_variant_rules() {
        let clipping = entry(PropertyScope::Frame, crate::frame::keys::CLIPPING).unwrap();
        assert_eq!(clipping.name, "clipping");
        assert_eq!(clipping.css.status, MappingStatus::Converted);
        assert!(clipping.css.initial_differs);
        let rules: Vec<(&str, &str)> = clipping
            .css
            .rules
            .iter()
            .map(|r| (r.variant, r.css))
            .collect();
        assert_eq!(
            rules,
            vec![
                ("disabled", "overflow: visible"),
                (
                    "frame-bounds",
                    "overflow: clip; overflow-clip-margin: border-box"
                ),
                ("path", "clip-path: url(#<path-id>)"),
            ]
        );
        let mask = entry(
            PropertyScope::Modifier,
            crate::modifier_metadata::keys::MASK_SOURCE,
        )
        .unwrap();
        assert_eq!(mask.css.status, MappingStatus::Approximated);
    }

    #[test]
    fn registry_snapshot_matches_golden() {
        // 검증 기준(IX.11.2·IX.11.3): registry snapshot. 갱신은 STELLAR_UPDATE_GOLDEN=1.
        let actual = snapshot_text();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/css/registry_snapshot.txt");
        if std::env::var_os("STELLAR_UPDATE_GOLDEN").is_some() {
            std::fs::write(path, &actual).unwrap();
        }
        let golden = include_str!("registry_snapshot.txt");
        assert_eq!(actual, golden, "registry snapshot이 golden과 다릅니다");
    }

    /// 속성 모델 표의 첫 열에서 `0x....` key(`\~` 범위 포함)를 모읍니다.
    fn table_keys(cell: &str) -> Vec<u32> {
        let bytes = cell.as_bytes();
        let mut out = Vec::new();
        let mut previous: Option<u32> = None;
        let mut range = false;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i..].starts_with(b"0x") {
                let digits = bytes[i + 2..]
                    .iter()
                    .take_while(|b| b.is_ascii_hexdigit())
                    .count();
                let hex = std::str::from_utf8(&bytes[i + 2..i + 2 + digits]).unwrap();
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    match (range, previous) {
                        (true, Some(start)) => out.extend(start + 1..=value),
                        _ => out.push(value),
                    }
                    previous = Some(value);
                    range = false;
                }
                i += 2 + digits;
                continue;
            }
            if bytes[i..].starts_with(b"\\~") {
                range = true;
            }
            i += 1;
        }
        out
    }

    #[test]
    fn property_model_tables_agree_with_registry_status() {
        // 검증 기준(IX.11.2·IX.11.3): 속성 모델 표(3장 공통, 5장 Frame, 6장 Shape·Path·Image, 7장 Text, 9장
        // Modifier)와 registry 일치. 표의 상태 열(셋째 열)에
        // 적힌 상태 가운데 registry 상태(또는 변형 규칙 상태)가 있어야 하고, registry key는 모두 표에 있어야 합니다.
        let model = include_str!("../../../../docs/design/property_model.md");
        let mut statuses: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        let mut section = 0;
        for line in model.lines() {
            if let Some(rest) = line.strip_prefix("## ") {
                section = rest
                    .split('.')
                    .next()
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0);
                continue;
            }
            if ![3, 5, 6, 7, 9].contains(&section) || !line.starts_with('|') {
                continue;
            }
            // code span 안의 `\|`는 열 구분이 아닙니다.
            let line = line.replace("\\|", "/");
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() < 4 {
                continue;
            }
            for key in table_keys(cells[1]) {
                let entry = statuses.entry(key).or_default();
                for status in MappingStatus::ALL {
                    if cells[3].contains(&format!("`{}`", status.name())) {
                        entry.insert(status.name().to_string());
                    }
                }
            }
        }
        for e in entries() {
            let documented = statuses
                .get(&e.key.0)
                .unwrap_or_else(|| panic!("속성 모델 표에 없음: 0x{:04x} {}", e.key.0, e.name));
            let mut allowed: BTreeSet<&str> = BTreeSet::from([e.css.status.name()]);
            allowed.extend(e.css.rules.iter().map(|r| r.status.name()));
            assert!(
                documented.iter().any(|s| allowed.contains(s.as_str())),
                "0x{:04x} {}: 표 {:?}, registry {:?}",
                e.key.0,
                e.name,
                documented,
                allowed
            );
        }
    }
}
