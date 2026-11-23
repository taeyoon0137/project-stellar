//! 사용 프로젝트가 저장한 외부 component 참조 표 (ADR-0137 결정 1).
//!
//! `stellar.yaml` 의존성 선언의 `components` 목록([`DependencyDeclaration::components`])을 component StableId →
//! [`ExternalComponentRef`] 표로 바꿉니다. 문서 instance payload는 component StableId만 담으므로 instance가 어느 의존성의
//! component를 가리키는지는 이 표가 정합니다. 같은 StableId를 두 선언이 저장했으면 둘 다 남기고, 판정하는 쪽이 모호로
//! 다룹니다(선언 이름 순).

use std::collections::BTreeMap;

use stellar_core_types::reference::ComponentRef;

use crate::declaration::DependencyDeclaration;
use crate::external_ref::ExternalComponentRef;

/// component StableId → 저장된 외부 참조.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentImports {
    by_component: BTreeMap<ComponentRef, Vec<ExternalComponentRef>>,
}

impl ComponentImports {
    /// 사용 프로젝트 root 선언에서 만듭니다.
    pub fn from_declarations(declarations: &[DependencyDeclaration]) -> ComponentImports {
        let mut by_component: BTreeMap<ComponentRef, Vec<ExternalComponentRef>> = BTreeMap::new();
        for declaration in declarations {
            for component in &declaration.components {
                by_component
                    .entry(*component)
                    .or_default()
                    .push(ExternalComponentRef::new(
                        declaration.name.clone(),
                        declaration.library_id,
                        *component,
                    ));
            }
        }
        for refs in by_component.values_mut() {
            refs.sort_by(|a, b| a.dependency().cmp(b.dependency()));
            refs.dedup();
        }
        ComponentImports { by_component }
    }

    /// `component`를 저장한 참조(선언 이름 순). 없으면 빈 slice입니다.
    pub fn get(&self, component: ComponentRef) -> &[ExternalComponentRef] {
        self.by_component
            .get(&component)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 저장된 참조가 하나도 없는지.
    pub fn is_empty(&self) -> bool {
        self.by_component.is_empty()
    }

    /// 모든 (component, 참조) 쌍(component 순).
    pub fn iter(&self) -> impl Iterator<Item = (ComponentRef, &[ExternalComponentRef])> {
        self.by_component
            .iter()
            .map(|(c, refs)| (*c, refs.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;

    use super::*;
    use crate::declaration::fixtures::{decl, library};
    use crate::declaration::LogicalName;

    fn component(n: u64) -> ComponentRef {
        ComponentRef::new(StableId::new_v7(0x0195_c530_3acd, 0x0bc1, n))
    }

    #[test]
    fn imports_map_components_to_their_declarations_and_keep_conflicts() {
        let mut ds = decl("ds", 1, "https://e.com/ds.git", "v1");
        ds.components = [component(1), component(2)].into_iter().collect();
        let mut old = decl("ds-old", 1, "https://e.com/ds.git", "v0");
        old.components = [component(2)].into_iter().collect();
        let imports = ComponentImports::from_declarations(&[old, ds]);
        assert_eq!(
            imports.get(component(1)),
            [ExternalComponentRef::new(
                LogicalName::new("ds").unwrap(),
                library(1),
                component(1)
            )]
        );
        let names: Vec<&str> = imports
            .get(component(2))
            .iter()
            .map(|r| r.dependency().as_str())
            .collect();
        assert_eq!(names, ["ds", "ds-old"], "두 선언이 저장한 참조는 모두 남김");
        assert!(imports.get(component(3)).is_empty());
        assert_eq!(imports.iter().count(), 2);
        assert!(ComponentImports::default().is_empty());
    }
}
