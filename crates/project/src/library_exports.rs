//! library 텍스트 원본에서 export index 읽기 (XV.5.1 제품 공급자, ADR-0119 결정 2).
//!
//! [`SourceExports`]는 [`LibraryExportSource`]를 library root의 텍스트 원본(ADR-0088 Git 원본)으로 구현합니다.
//! `design/document.stellar.yaml`이 있으면 원본 파일 집합 전체([`session::decode_source`])에서 컴포넌트 정의와
//! token set을, 없으면 컴포넌트 원본(`design/components/`)만 읽습니다. 저장 검증(XV.3.4)과 같은 codec입니다.
//!
//! | export 필드 | 출처 |
//! |---|---|
//! | component id·이름 | `ComponentDefinition` StableId·표시 이름 |
//! | property id·이름·공개 여부 | `ComponentProperty` |
//! | property 타입 표기 | [`type_signature`] (`TypeSpec` 표시 형식) |
//! | property default fingerprint | default 표현식 canonical bytes(`CheckedExpression::canonical_bytes`) |
//! | slot | 정의 slot StableId |
//! | token id·이름·타입·값 | `TokenSource` id·이름·[`value_type_signature`]·`TypedValue::to_bytes` |
//! | 외부 참조 | 없음(문서 원본이 아직 외부 참조를 저장하지 않음, XV.5.2 차이) |
//!
//! 사용 프로젝트 command 사전 검사(XV.5.3)와 후보 검증(XV.7.3)은 override 값 타입 표기를 같은
//! [`value_type_signature`]로 만들어 export 표기와 같은 규칙으로 비교합니다.

use std::path::Path;

use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_dependencies::declaration::LibraryId;
use stellar_dependencies::library_snapshot::{
    ExportFingerprint, ExportReadFailure, ExportVisibility, ExportedComponent, ExportedProperty,
    ExportedToken, ExportedTokenSet, LibraryExportSource, LibraryExports, SnapshotLimits,
};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::property::PropertyVisibility;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};

use crate::document_io::component_source;
use crate::document_io::session;
use crate::document_io::source_v4::token::TokenSetSource;
use crate::document_io::text_source::TextSourceError;
use crate::library_entry::generation_components;
use crate::path_resolver::ExternalAccess;

/// property 타입 표기(`T` 또는 `Optional<T>`).
pub fn type_signature(spec: TypeSpec) -> String {
    spec.to_string()
}

/// 값의 타입 표기(비-nullable `T`). override 값·token 값에 씁니다.
pub fn value_type_signature(value: &TypedValue) -> String {
    TypeSpec::required(ExpressionType::of(value)).to_string()
}

/// library root 텍스트 원본에서 export를 읽는 공급자.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceExports;

/// library root의 컴포넌트 정의와 token set을 읽습니다(검증 포함).
///
/// 정의는 generation component library chunk([`generation_components`])와 텍스트 원본 가운데 정의를 담은 쪽에서
/// 읽습니다. 둘 다 담으면 [`ExportReadFailure::Corrupt`]입니다(storage validator와 같은 정책).
pub fn read_library_resources(
    root: &Path,
    library_id: LibraryId,
) -> Result<(ComponentLibrary, Vec<TokenSetSource>), ExportReadFailure> {
    let (text_components, token_sets) = match session::read_source(root).map_err(source_failure)? {
        Some(files) => {
            let project = session::decode_source(&files).map_err(source_failure)?;
            (project.components, project.token_sets)
        }
        None => {
            let (library, _) =
                component_source::read_project_library(root, ExternalAccess::MetadataOnly)
                    .map_err(source_failure)?;
            (library, Vec::new())
        }
    };
    let generation =
        generation_components(root, library_id).map_err(|_| ExportReadFailure::Corrupt)?;
    let components = match generation {
        Some(library) if !library.is_empty() => {
            if !text_components.is_empty() {
                return Err(ExportReadFailure::Corrupt);
            }
            library
        }
        _ => text_components,
    };
    Ok((components, token_sets))
}

fn source_failure(error: TextSourceError) -> ExportReadFailure {
    match error {
        TextSourceError::Io(e) => ExportReadFailure::Io { kind: e.kind() },
        TextSourceError::Unsupported(_) => ExportReadFailure::UnsupportedSchema,
        TextSourceError::Invalid(_) | TextSourceError::Changed => ExportReadFailure::Corrupt,
    }
}

impl LibraryExportSource for SourceExports {
    fn read_exports(
        &mut self,
        root: &Path,
        library: LibraryId,
        limits: &SnapshotLimits,
    ) -> Result<LibraryExports, ExportReadFailure> {
        let (library, token_sets) = read_library_resources(root, library)?;
        exports_of(&library, &token_sets, limits)
    }
}

/// 컴포넌트 정의·token set을 export index로 바꿉니다. 개수 상한은 만드는 동안 확인합니다.
pub fn exports_of(
    library: &ComponentLibrary,
    token_sets: &[TokenSetSource],
    limits: &SnapshotLimits,
) -> Result<LibraryExports, ExportReadFailure> {
    let mut exports = LibraryExports::default();
    for entry in library.iter() {
        if exports.components.len() == limits.max_components {
            return Err(ExportReadFailure::LimitExceeded);
        }
        let definition = entry.definition();
        let mut properties = Vec::new();
        for id in definition.properties() {
            if properties.len() == limits.max_properties_per_component {
                return Err(ExportReadFailure::LimitExceeded);
            }
            let record = entry.property(*id).ok_or(ExportReadFailure::Corrupt)?;
            properties.push(ExportedProperty {
                id: id.stable_id(),
                name: record.name().to_string(),
                type_signature: type_signature(record.value_type()),
                visibility: match record.visibility() {
                    PropertyVisibility::Public => ExportVisibility::Public,
                    PropertyVisibility::Private => ExportVisibility::Private,
                },
                default: ExportFingerprint::new(
                    record.default_expression().canonical_bytes().to_vec(),
                ),
            });
        }
        if definition.slots().len() > limits.max_slots_per_component {
            return Err(ExportReadFailure::LimitExceeded);
        }
        exports.components.push(ExportedComponent {
            id: definition.reference(),
            name: definition.display_name().to_string(),
            properties,
            slots: definition.slots().iter().map(|s| s.stable_id()).collect(),
            references: Vec::new(),
        });
    }
    let mut tokens = 0usize;
    for set in token_sets {
        if exports.token_sets.len() == limits.max_token_sets {
            return Err(ExportReadFailure::LimitExceeded);
        }
        let mut out = Vec::new();
        for token in &set.tokens {
            if tokens == limits.max_tokens {
                return Err(ExportReadFailure::LimitExceeded);
            }
            tokens += 1;
            out.push(ExportedToken {
                id: TokenRef::new(token.id),
                name: token.name.clone(),
                type_signature: value_type_signature(&token.value),
                value: ExportFingerprint::new(token.value.to_bytes()),
                references: Vec::new(),
            });
        }
        exports.token_sets.push(ExportedTokenSet {
            id: set.id,
            tokens: out,
        });
    }
    Ok(exports)
}
