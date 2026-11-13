//! local path override 매핑 (XV.6.1).
//!
//! 사용 프로젝트 `stellar.yaml`의 의존성 entry에 있는 `local_override`는 그 logical name을 로컬
//! Stellar 프로젝트 root로 매핑합니다(아키텍처 30.1 "로컬 개발 시 로컬 경로나 worktree override").
//! 상대 경로는 사용 프로젝트 root 기준이며, 결과 root는 사용 프로젝트 자신이나 그 내부일 수
//! 없습니다(자기 참조·프로젝트 안 의존성 checkout 금지, XV.3.1). library 매니페스트의 전이 선언에
//! 있는 override는 사용하지 않습니다(resolver가 무시).
//!
//! 위치 검사는 lexical 정규화 기준입니다. symlink 해석·대상 probe(일반 저장소·library id 검사)는
//! XV.6.2가 담당합니다.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::declaration::{DependencyDeclaration, LocalOverridePath, LogicalName};
use crate::node::{invalid, join, SchemaError, ValueError};

/// logical name 하나의 override 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOverrideRoot {
    /// 선언 경로(원문).
    pub declared: LocalOverridePath,
    /// 사용 프로젝트 root 기준으로 lexical 정규화한 절대 경로.
    pub root: PathBuf,
}

/// logical name → override 대상.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalOverrideMap {
    overrides: BTreeMap<LogicalName, LocalOverrideRoot>,
}

impl LocalOverrideMap {
    /// override 대상 조회.
    pub fn get(&self, name: &LogicalName) -> Option<&LocalOverrideRoot> {
        self.overrides.get(name)
    }

    /// 모든 override(이름 순).
    pub fn iter(&self) -> impl Iterator<Item = (&LogicalName, &LocalOverrideRoot)> {
        self.overrides.iter()
    }

    /// override 수.
    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

/// `..`·`.`을 lexical로 접습니다(파일 시스템을 읽지 않음).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 사용 프로젝트 선언에서 override 매핑을 만들고 검증합니다.
///
/// `project_root`는 사용 프로젝트 root의 절대 경로입니다. 같은 logical name의 override가 두 번
/// 있으면 오류입니다(XV.1.3 중복 이름 판정과 일치).
pub fn local_overrides(
    declarations: &[DependencyDeclaration],
    project_root: &Path,
) -> Result<LocalOverrideMap, SchemaError> {
    let project = lexical_normalize(project_root);
    let mut map = LocalOverrideMap::default();
    for d in declarations {
        let Some(declared) = &d.local_override else {
            continue;
        };
        let path = join(&join("dependencies", d.name.as_str()), "local_override");
        let mut target = if declared.is_absolute() {
            PathBuf::from("/")
        } else {
            project.clone()
        };
        for part in declared.components() {
            target.push(part);
        }
        let target = lexical_normalize(&target);
        if target.starts_with(&project) {
            return Err(invalid(&path, ValueError::OverrideInsideProject));
        }
        if map.overrides.contains_key(&d.name) {
            return Err(invalid(&path, ValueError::Inconsistent));
        }
        map.overrides.insert(
            d.name.clone(),
            LocalOverrideRoot {
                declared: declared.clone(),
                root: target,
            },
        );
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::dependencies_from_node;
    use crate::declaration::fixtures::decl;
    use crate::node::DeclNode;

    const PROJECT: &str = "/work/app";

    fn with_override(name: &str, path: &str) -> DependencyDeclaration {
        let mut d = decl(name, 1, "https://e.com/ds.git", "v1");
        d.local_override = Some(LocalOverridePath::new(path).unwrap());
        d
    }

    fn resolve_one(path: &str) -> Result<PathBuf, SchemaError> {
        local_overrides(&[with_override("ds", path)], Path::new(PROJECT)).map(|m| {
            m.get(&LogicalName::new("ds").unwrap())
                .unwrap()
                .root
                .clone()
        })
    }

    #[test]
    fn override_maps_logical_name_to_local_project_root() {
        assert_eq!(
            resolve_one("../design-system").unwrap(),
            Path::new("/work/design-system")
        );
        assert_eq!(resolve_one("../../src/ds").unwrap(), Path::new("/src/ds"));
        assert_eq!(
            resolve_one("/Users/dev/ds").unwrap(),
            Path::new("/Users/dev/ds")
        );
        // ..로 나갔다가 형제 프로젝트로 들어가는 경로.
        assert_eq!(
            resolve_one("../app-lib/../ds").unwrap(),
            Path::new("/work/ds")
        );

        let declarations = [
            with_override("ds", "../ds"),
            decl("icons", 2, "https://e.com/icons.git", "v1"),
            with_override("tokens", "../tokens"),
        ];
        let map = local_overrides(&declarations, Path::new(PROJECT)).unwrap();
        assert_eq!(map.len(), 2);
        let names: Vec<&str> = map.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["ds", "tokens"]);
        assert!(map.get(&LogicalName::new("icons").unwrap()).is_none());
    }

    #[test]
    fn override_inside_or_equal_to_project_is_rejected() {
        let inside = |p: &str| {
            assert_eq!(
                resolve_one(p),
                Err(SchemaError::InvalidValue {
                    path: "dependencies.ds.local_override".to_string(),
                    error: ValueError::OverrideInsideProject
                }),
                "{p}"
            );
        };
        inside("design");
        inside("libs/ds");
        inside("../app");
        inside("../app/vendor/ds");
        inside("/work/app");
        inside("/work/app/sub");
        inside("sub/../../app");
        // 형제 이름이 접두사만 같은 경우는 밖(`/work/app-lib`).
        assert_eq!(
            resolve_one("../app-lib").unwrap(),
            Path::new("/work/app-lib")
        );
    }

    #[test]
    fn override_path_syntax_is_validated() {
        for bad in ["a//b", "x/", "./ds", "../ds/.", "a\\b", "/", "\u{7}x"] {
            assert!(LocalOverridePath::new(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            LocalOverridePath::new("a//b"),
            Err(ValueError::InvalidOverridePath)
        );
        assert_eq!(
            LocalOverridePath::new("a\\b"),
            Err(ValueError::InvalidCharacter)
        );
        // 스키마 경로에서도 거부.
        let (name, DeclNode::Mapping(mut pairs)) =
            decl("ds", 1, "https://e.com/ds.git", "v1").to_node_entry()
        else {
            unreachable!()
        };
        pairs.push(("local_override".to_string(), DeclNode::scalar("../ds/")));
        assert_eq!(
            dependencies_from_node(&DeclNode::Mapping(vec![(name, DeclNode::Mapping(pairs))])),
            Err(SchemaError::InvalidValue {
                path: "dependencies.ds.local_override".to_string(),
                error: ValueError::InvalidOverridePath
            })
        );
        // 같은 이름 override 두 번.
        assert_eq!(
            local_overrides(
                &[with_override("ds", "../a"), with_override("ds", "../b")],
                Path::new(PROJECT)
            ),
            Err(SchemaError::InvalidValue {
                path: "dependencies.ds.local_override".to_string(),
                error: ValueError::Inconsistent
            })
        );
    }
}
