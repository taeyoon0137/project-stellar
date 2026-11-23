//! 사용 프로젝트 local override 대상 검증 (XV.6.2, ADR-0119 결정 1).
//!
//! `stellar.yaml` 의존성 entry의 `local_override`(XV.6.1)를 실제 경로 매핑([`local_overrides`])으로 바꾸고, 각 대상을
//! 의존성 crate의 [`validate_local_override`]에 제품 검증기 [`ProjectLibraryValidator::checkout`](실제
//! [`probe_project`](crate::probe::probe_project), IV.5.1)로 검사합니다. 일반 Git 저장소(`stellar.yaml` 없음),
//! `.git`이 없는 폴더, 손상 매니페스트, 선언과 다른 library id, symlink를 따라 사용 프로젝트 안을 가리키는 대상을
//! 거부합니다.
//!
//! 검증 결과는 override마다 따로 돌려줍니다. 한 override가 거부되어도 다른 의존성의 override나 lock 기준 cache
//! 로드(XV.5.5)는 영향을 받지 않습니다.

use std::path::Path;

use stellar_dependencies::declaration::{DependencyDeclaration, LogicalName};
use stellar_dependencies::local_override::{
    local_overrides, validate_local_override, LocalOverrideRejection, ValidatedLocalOverride,
};
use stellar_dependencies::node::SchemaError;

use crate::library_entry::ProjectLibraryValidator;

/// override 하나의 검증 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOverrideCheck {
    /// 의존성 선언 이름.
    pub dependency: LogicalName,
    /// 검증 결과.
    pub result: Result<ValidatedLocalOverride, LocalOverrideRejection>,
}

/// 사용 프로젝트 선언의 local override를 모두 검증합니다(선언 이름 순).
///
/// 매핑 단계 오류(프로젝트 안을 가리키는 lexical 경로, 같은 이름의 override 중복)는 선언 자체의 schema 오류이므로
/// `Err`입니다.
pub fn validate_project_local_overrides(
    project_root: &Path,
    declarations: &[DependencyDeclaration],
) -> Result<Vec<LocalOverrideCheck>, SchemaError> {
    let map = local_overrides(declarations, project_root)?;
    let mut validator = ProjectLibraryValidator::checkout();
    let mut checks = Vec::new();
    for (name, target) in map.iter() {
        let Some(declaration) = declarations.iter().find(|d| &d.name == name) else {
            continue;
        };
        checks.push(LocalOverrideCheck {
            dependency: name.clone(),
            result: validate_local_override(
                target,
                project_root,
                declaration.library_id,
                &mut validator,
            ),
        });
    }
    Ok(checks)
}
