//! 제한 YAML 값 트리 ↔ 의존성 형식 중립 트리 변환 (XV.1.1·XV.4.1 연결).
//!
//! `stellar-dependencies`는 core-types에만 의존하므로 `stellar.yaml`의 `dependencies` 절과
//! `stellar.lock.yaml` 문서를 [`DeclNode`]로 주고받습니다. 이 모듈은 project의 [`YamlValue`]와
//! [`DeclNode`]를 1:1로 바꿉니다: 스칼라는 문자열 그대로(암시적 타입 추론 없음), 시퀀스·매핑은 순서
//! 보존, 매핑 키는 스칼라만 허용합니다(제한 YAML과 같은 규칙).
//!
//! 입력 트리는 신뢰할 수 없으므로 중첩 깊이를 [`MAX_NODE_DEPTH`]로 제한합니다. 제한 loader가 만든
//! 트리는 이미 같은 상한을 지키지만, 프로그램이 직접 만든 [`YamlValue`]도 같은 경로로 들어오므로
//! 변환이 호출 stack을 무한히 쓰지 않게 막습니다.

use std::fmt;

use stellar_dependencies::node::DeclNode;

use crate::yaml::loader::MAX_NESTING_DEPTH;
use crate::yaml::YamlValue;

/// 변환이 허용하는 최대 컨테이너 중첩 깊이(제한 loader 상한과 같음).
pub const MAX_NODE_DEPTH: usize = MAX_NESTING_DEPTH;

/// 변환 실패. 입력 값은 담지 않고 경로만 담습니다(URL·경로 민감값, XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeConversionError {
    /// 매핑 키가 스칼라가 아님.
    NonScalarKey {
        /// 키가 나타난 매핑 경로.
        path: String,
    },
    /// 컨테이너 중첩이 [`MAX_NODE_DEPTH`]를 넘음.
    TooDeep {
        /// 상한을 넘은 위치.
        path: String,
    },
}

impl fmt::Display for NodeConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeConversionError::NonScalarKey { path } => {
                write!(f, "매핑 키는 스칼라여야 합니다: {path}")
            }
            NodeConversionError::TooDeep { path } => {
                write!(f, "중첩 깊이 상한({MAX_NODE_DEPTH})을 넘습니다: {path}")
            }
        }
    }
}

impl std::error::Error for NodeConversionError {}

/// [`YamlValue`]를 [`DeclNode`]로 바꿉니다. `path`는 오류 경로 접두사입니다.
pub fn yaml_to_node(value: &YamlValue, path: &str) -> Result<DeclNode, NodeConversionError> {
    convert_in(value, path, 0)
}

fn convert_in(
    value: &YamlValue,
    path: &str,
    depth: usize,
) -> Result<DeclNode, NodeConversionError> {
    match value {
        YamlValue::Scalar(text) => Ok(DeclNode::Scalar(text.clone())),
        YamlValue::Sequence(items) => {
            let depth = enter(path, depth)?;
            let mut out = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                out.push(convert_in(item, &format!("{path}[{index}]"), depth)?);
            }
            Ok(DeclNode::Sequence(out))
        }
        YamlValue::Mapping(pairs) => {
            let depth = enter(path, depth)?;
            let mut out = Vec::with_capacity(pairs.len());
            for (key, child) in pairs {
                let Some(name) = key.as_scalar() else {
                    return Err(NodeConversionError::NonScalarKey {
                        path: path.to_string(),
                    });
                };
                let child_path = if path.is_empty() {
                    name.to_string()
                } else {
                    format!("{path}.{name}")
                };
                out.push((name.to_string(), convert_in(child, &child_path, depth)?));
            }
            Ok(DeclNode::Mapping(out))
        }
    }
}

/// 컨테이너 한 단계를 들어갑니다(상한 초과면 오류).
fn enter(path: &str, depth: usize) -> Result<usize, NodeConversionError> {
    let next = depth + 1;
    if next > MAX_NODE_DEPTH {
        return Err(NodeConversionError::TooDeep {
            path: path.to_string(),
        });
    }
    Ok(next)
}

/// [`DeclNode`]를 [`YamlValue`]로 바꿉니다.
///
/// 입력은 의존성 모델(`DependencyDeclaration::to_node_entry`, `LockDocument::to_node`)이 만든
/// canonical 트리뿐이며 깊이가 고정(4 이하)이라 재귀 깊이도 고정입니다. 외부 입력 트리는 이 함수로
/// 들어오지 않습니다.
pub(crate) fn node_to_yaml(node: &DeclNode) -> YamlValue {
    match node {
        DeclNode::Scalar(text) => YamlValue::Scalar(text.clone()),
        DeclNode::Sequence(items) => YamlValue::Sequence(items.iter().map(node_to_yaml).collect()),
        DeclNode::Mapping(pairs) => YamlValue::Mapping(
            pairs
                .iter()
                .map(|(k, v)| (YamlValue::scalar(k.clone()), node_to_yaml(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_is_one_to_one_and_order_preserving() {
        let yaml = YamlValue::Mapping(vec![
            (YamlValue::scalar("b"), YamlValue::scalar("01")),
            (
                YamlValue::scalar("a"),
                YamlValue::Sequence(vec![
                    YamlValue::scalar("yes"),
                    YamlValue::Mapping(vec![(YamlValue::scalar("k"), YamlValue::scalar(""))]),
                ]),
            ),
        ]);
        let node = yaml_to_node(&yaml, "").unwrap();
        assert_eq!(
            node,
            DeclNode::Mapping(vec![
                ("b".to_string(), DeclNode::scalar("01")),
                (
                    "a".to_string(),
                    DeclNode::Sequence(vec![
                        DeclNode::scalar("yes"),
                        DeclNode::Mapping(vec![("k".to_string(), DeclNode::scalar(""))]),
                    ])
                ),
            ])
        );
        assert_eq!(node_to_yaml(&node), yaml);
    }

    #[test]
    fn non_scalar_keys_and_deep_trees_are_rejected() {
        let bad_key = YamlValue::Mapping(vec![(
            YamlValue::scalar("deps"),
            YamlValue::Mapping(vec![(YamlValue::Sequence(vec![]), YamlValue::scalar("x"))]),
        )]);
        assert_eq!(
            yaml_to_node(&bad_key, "dependencies"),
            Err(NodeConversionError::NonScalarKey {
                path: "dependencies.deps".to_string()
            })
        );

        // 상한까지는 허용, 한 단계 더 깊으면 거부(재귀 없이 끝남을 확인).
        let mut at_limit = YamlValue::scalar("leaf");
        for _ in 0..MAX_NODE_DEPTH {
            at_limit = YamlValue::Sequence(vec![at_limit]);
        }
        assert!(yaml_to_node(&at_limit, "x").is_ok());
        let too_deep = YamlValue::Sequence(vec![at_limit]);
        assert!(matches!(
            yaml_to_node(&too_deep, "x"),
            Err(NodeConversionError::TooDeep { .. })
        ));
        // 오류 메시지에 값이 없음.
        let text = yaml_to_node(&bad_key, "dependencies")
            .unwrap_err()
            .to_string();
        assert!(!text.contains('x'), "{text}");
    }
}
