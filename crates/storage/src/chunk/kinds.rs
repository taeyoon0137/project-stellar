//! 초기 chunk kind 정의 (V.6.1).
//!
//! kind code 숫자 값은 저장 포맷의 일부이므로 golden test로 고정합니다.
//! required bit(V.2.7)는 "이 kind를 모르는 reader가 문서를 잘못 해석하게 되는가"
//! 기준으로 부여합니다:
//!
//! - required: ProjectMetadata, DocumentManifest, ViewTree, ComponentLibrary,
//!   TokenSet — 문서 구조·의미 해석에 필수.
//! - optional: AssetMetadata — 없이도 문서 구조를 안전하게 열 수 있는 부가 정보.

use super::envelope::ChunkKindCode;
use super::kind_registry::{ChunkKindRegistry, KindRegistryError};

/// 프로젝트 메타데이터 chunk.
pub const KIND_PROJECT_METADATA: ChunkKindCode = ChunkKindCode::required(1);

/// 문서 목록/구조 manifest chunk.
pub const KIND_DOCUMENT_MANIFEST: ChunkKindCode = ChunkKindCode::required(2);

/// View subtree 트리 chunk.
pub const KIND_VIEW_TREE: ChunkKindCode = ChunkKindCode::required(3);

/// 컴포넌트 라이브러리 chunk.
pub const KIND_COMPONENT_LIBRARY: ChunkKindCode = ChunkKindCode::required(4);

/// 디자인 토큰 집합 chunk.
pub const KIND_TOKEN_SET: ChunkKindCode = ChunkKindCode::required(5);

/// asset 메타데이터 chunk(원본 파일은 `assets/`).
pub const KIND_ASSET_METADATA: ChunkKindCode = ChunkKindCode::optional(6);

/// (code, 이름) 초기 kind 목록.
pub const INITIAL_KINDS: [(ChunkKindCode, &str); 6] = [
    (KIND_PROJECT_METADATA, "project-metadata"),
    (KIND_DOCUMENT_MANIFEST, "document-manifest"),
    (KIND_VIEW_TREE, "view-tree"),
    (KIND_COMPONENT_LIBRARY, "component-library"),
    (KIND_TOKEN_SET, "token-set"),
    (KIND_ASSET_METADATA, "asset-metadata"),
];

/// 초기 kind가 전부 등록된 registry를 만듭니다.
pub fn initial_kind_registry() -> Result<ChunkKindRegistry, KindRegistryError> {
    let mut registry = ChunkKindRegistry::new();
    for (code, name) in INITIAL_KINDS {
        registry.register(code, name)?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::kind_registry::KindResolution;

    #[test]
    fn golden_kind_codes_are_stable() {
        // 검증 기준(V.6.1): kind registry golden test.
        // 숫자 값은 저장 포맷의 일부 — 변경은 format version 상승과 함께만 허용.
        let golden: [(u32, &str); 6] = [
            (0x8000_0001, "project-metadata"),
            (0x8000_0002, "document-manifest"),
            (0x8000_0003, "view-tree"),
            (0x8000_0004, "component-library"),
            (0x8000_0005, "token-set"),
            (0x0000_0006, "asset-metadata"),
        ];
        assert_eq!(INITIAL_KINDS.len(), golden.len());
        for ((code, name), (expected_code, expected_name)) in INITIAL_KINDS.iter().zip(golden) {
            assert_eq!(code.0, expected_code, "{name}");
            assert_eq!(*name, expected_name);
        }
    }

    #[test]
    fn registry_resolves_all_initial_kinds() {
        let registry = initial_kind_registry().unwrap();
        assert_eq!(registry.len(), 6);
        for (code, name) in INITIAL_KINDS {
            assert_eq!(
                registry.resolve(code).unwrap(),
                KindResolution::Known { name }
            );
        }
    }

    #[test]
    fn requiredness_matches_policy() {
        assert!(KIND_PROJECT_METADATA.is_required());
        assert!(KIND_DOCUMENT_MANIFEST.is_required());
        assert!(KIND_VIEW_TREE.is_required());
        assert!(KIND_COMPONENT_LIBRARY.is_required());
        assert!(KIND_TOKEN_SET.is_required());
        assert!(!KIND_ASSET_METADATA.is_required());
    }

    #[test]
    fn unknown_kinds_follow_version_policy() {
        let registry = initial_kind_registry().unwrap();
        // 미래 optional kind → skip.
        assert_eq!(
            registry.resolve(ChunkKindCode::optional(999)).unwrap(),
            KindResolution::SkipUnknownOptional
        );
        // 미래 required kind → 오류.
        assert!(registry.resolve(ChunkKindCode::required(999)).is_err());
    }
}
