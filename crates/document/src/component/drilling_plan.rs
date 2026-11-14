//! Property drilling 계획 (IX.4.3).
//!
//! 사용자가 nested instance 안쪽 property를 상위 컴포넌트로 노출하려 할 때(IX.4.2 `RequiresDrilling`), 필요한 중간
//! property와 경계별 link(IX.4.1)를 계산합니다. **계산만 하고 적용하지 않습니다** — library·drilling 모음은 읽기만
//! 합니다. 적용은 승인 후 IX.4.5가 합니다.
//!
//! 입력 [`DrillingRequest`]: 노출을 받는 최상위 컴포넌트 `top`, `top` 정의 tree에서 대상 인스턴스까지의 nested
//! instance `path`(바깥 → 안쪽), 가장 안쪽 컴포넌트의 **공개** property.
//!
//! 계산(안쪽 → 바깥, 경계 `k = n-1 … 0`): 경계 `k`의 owner는 `components[k]`, 인스턴스는 `path[k]`, 값이 들어갈
//! 자리는 `components[k+1]`의 property입니다.
//!
//! - 그 자리에 owner가 소유한 **live link**가 이미 있으면 재사용합니다([`DrillingPlan::reused`]). link source가
//!   다음 바깥 경계의 자리가 됩니다.
//! - 없으면 owner에 새 공개 property를 계획하고([`NewProperty`]) 새 property → 자리 link를 계획합니다
//!   ([`PlannedLink`]). 새 property의 이름·타입·default는 현재 노출 중인 property(가장 안쪽 property 또는 재사용한
//!   link의 source)에서 가져옵니다 — 노출 전과 같은 값이 보이도록 default 값을 그대로 씁니다.
//! - 그 자리를 nested 인스턴스가 정적 override로 채우고 있으면 계획하지 않고 거부합니다(값 원천 두 개, IX.4.1).
//!
//! 결과의 `exposed`가 노출 후 `top`에서 쓰는 property입니다. 모든 경계가 이미 연결돼 있으면 새 항목 없는 계획이고
//! `exposed`는 기존 property입니다. 같은 입력(library·모음·요청)은 항상 같은 계획입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::TypeSpec;

use super::definition::{ComponentLibrary, ComponentPropertyId};
use super::drilling::{resolve_path, DrillingBindingId, DrillingError, DrillingSet};
use super::instance::{ComponentInstancePayload, OverrideKey};

/// 노출 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrillingRequest {
    /// 노출을 받는 최상위 컴포넌트.
    pub top: ComponentRef,
    /// `top` 정의 tree에서 대상 인스턴스까지의 nested instance chain(바깥 → 안쪽).
    pub path: Vec<StableId>,
    /// 가장 안쪽 컴포넌트의 공개 property.
    pub property: ComponentPropertyId,
}

/// 계획 안 property 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlannedProperty {
    /// 이미 있는 property.
    Existing(ComponentPropertyId),
    /// 경계 `boundary` 컴포넌트(`components[boundary]`)에 새로 만들 property.
    New {
        /// 컴포넌트 index.
        boundary: usize,
    },
}

/// 새로 만들 property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProperty {
    /// 컴포넌트 index(`components[boundary]`, 0 = `top`).
    pub boundary: usize,
    /// 컴포넌트.
    pub component: ComponentRef,
    /// 기준 이름(노출하는 property 이름).
    pub base_name: String,
    /// 선언 타입.
    pub value_type: TypeSpec,
    /// default 값(`None`이면 nullable 값 없음).
    pub default: Option<TypedValue>,
}

/// 새로 만들 경계 link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedLink {
    /// 경계 index.
    pub boundary: usize,
    /// link owner(`components[boundary]`).
    pub owner: ComponentRef,
    /// owner 정의 tree 안 nested 인스턴스(`path[boundary]`).
    pub instance: StableId,
    /// owner의 값 원천 property.
    pub source: PlannedProperty,
    /// 인스턴스 컴포넌트(`components[boundary + 1]`)의 대상 property.
    pub target: PlannedProperty,
}

/// 재사용하는 기존 link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReusedLink {
    /// 경계 index.
    pub boundary: usize,
    /// link.
    pub binding: DrillingBindingId,
}

/// drilling 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrillingPlan {
    request: DrillingRequest,
    components: Vec<ComponentRef>,
    new_properties: Vec<NewProperty>,
    links: Vec<PlannedLink>,
    reused: Vec<ReusedLink>,
    exposed: PlannedProperty,
}

/// 계획 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillingPlanError {
    /// path·property 해석 오류(IX.4.1 규칙).
    Drilling(DrillingError),
    /// 경계 자리를 nested 인스턴스가 정적 override로 채우고 있습니다(먼저 reset 필요).
    StaticOverride {
        /// 경계 index.
        boundary: usize,
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
}

impl fmt::Display for DrillingPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrillingPlanError::Drilling(err) => write!(f, "{err}"),
            DrillingPlanError::StaticOverride {
                boundary,
                instance,
                property,
            } => write!(
                f,
                "경계 {boundary}의 인스턴스 {instance}가 property {}를 정적 override합니다",
                property.stable_id()
            ),
        }
    }
}

impl std::error::Error for DrillingPlanError {}

impl From<DrillingError> for DrillingPlanError {
    fn from(err: DrillingError) -> DrillingPlanError {
        DrillingPlanError::Drilling(err)
    }
}

/// 현재 노출 중인 property의 이름·타입·default.
struct Exposed {
    property: PlannedProperty,
    name: String,
    value_type: TypeSpec,
    default: Option<TypedValue>,
}

impl DrillingPlan {
    /// 요청의 계획을 계산합니다. library·모음은 바뀌지 않습니다.
    pub fn compute(
        library: &ComponentLibrary,
        drilling: &DrillingSet,
        request: DrillingRequest,
    ) -> Result<DrillingPlan, DrillingPlanError> {
        if request.path.is_empty() {
            return Err(DrillingError::NoBoundary.into());
        }
        let mut components = vec![request.top];
        components.extend(resolve_path(library, request.top, &request.path)?);
        let depth = request.path.len();
        let innermost = components[depth];
        let leaf = library
            .get(innermost)
            .ok_or(DrillingError::UnknownComponent {
                component: innermost,
            })?;
        let record = leaf
            .property(request.property)
            .filter(|_| leaf.definition().properties().contains(&request.property))
            .ok_or(DrillingError::UnknownProperty {
                component: innermost,
                property: request.property,
            })?;
        if !record.is_public() {
            return Err(DrillingError::PrivateProperty {
                component: innermost,
                property: request.property,
            }
            .into());
        }
        let mut exposed = Exposed {
            property: PlannedProperty::Existing(request.property),
            name: record.name().to_string(),
            value_type: record.value_type(),
            default: record.default_value().cloned(),
        };
        let mut new_properties = Vec::new();
        let mut links = Vec::new();
        let mut reused = Vec::new();
        for boundary in (0..depth).rev() {
            let owner = components[boundary];
            let instance = request.path[boundary];
            if let PlannedProperty::Existing(target) = exposed.property {
                if let Some(link) = drilling
                    .link_into(owner, instance, target)
                    .filter(|link| !drilling.is_orphan(link.id()))
                {
                    let owner_entry = library
                        .get(owner)
                        .ok_or(DrillingError::UnknownComponent { component: owner })?;
                    let source = owner_entry.property(link.source()).ok_or(
                        DrillingError::UnknownProperty {
                            component: owner,
                            property: link.source(),
                        },
                    )?;
                    reused.push(ReusedLink {
                        boundary,
                        binding: link.id(),
                    });
                    exposed = Exposed {
                        property: PlannedProperty::Existing(link.source()),
                        name: source.name().to_string(),
                        value_type: source.value_type(),
                        default: source.default_value().cloned(),
                    };
                    continue;
                }
                let scope = library
                    .get(owner)
                    .ok_or(DrillingError::UnknownComponent { component: owner })?;
                let statically_set = scope
                    .tree()
                    .node(instance)
                    .and_then(|node| ComponentInstancePayload::decode(&node.payload).ok())
                    .is_some_and(|payload| {
                        payload
                            .property_override(&OverrideKey::direct(target))
                            .is_some()
                    });
                if statically_set {
                    return Err(DrillingPlanError::StaticOverride {
                        boundary,
                        instance,
                        property: target,
                    });
                }
            }
            let source = PlannedProperty::New { boundary };
            new_properties.push(NewProperty {
                boundary,
                component: owner,
                base_name: exposed.name.clone(),
                value_type: exposed.value_type,
                default: exposed.default.clone(),
            });
            links.push(PlannedLink {
                boundary,
                owner,
                instance,
                source,
                target: exposed.property,
            });
            exposed.property = source;
        }
        new_properties.reverse();
        links.reverse();
        reused.reverse();
        Ok(DrillingPlan {
            request,
            components,
            new_properties,
            links,
            reused,
            exposed: exposed.property,
        })
    }

    /// 요청.
    pub fn request(&self) -> &DrillingRequest {
        &self.request
    }

    /// 경계별 컴포넌트(`[0]` = top, `[n]` = 가장 안쪽).
    pub fn components(&self) -> &[ComponentRef] {
        &self.components
    }

    /// 새로 만들 property(경계 index 오름차순 = 바깥 → 안쪽).
    pub fn new_properties(&self) -> &[NewProperty] {
        &self.new_properties
    }

    /// 경계 `boundary` 컴포넌트에 새로 만들 property.
    pub fn new_property(&self, boundary: usize) -> Option<&NewProperty> {
        self.new_properties.iter().find(|p| p.boundary == boundary)
    }

    /// 새로 만들 link(경계 index 오름차순).
    pub fn links(&self) -> &[PlannedLink] {
        &self.links
    }

    /// 재사용하는 기존 link(경계 index 오름차순).
    pub fn reused(&self) -> &[ReusedLink] {
        &self.reused
    }

    /// 노출 후 `top`에서 쓰는 property.
    pub fn exposed(&self) -> PlannedProperty {
        self.exposed
    }

    /// 새로 만들 항목이 없는지(이미 노출됨).
    pub fn is_noop(&self) -> bool {
        self.new_properties.is_empty() && self.links.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::drilling::fixtures::*;
    use crate::component::drilling::DrillingBinding;
    use crate::component::property::PropertyVisibility;
    use stellar_expressions::types::ExpressionType;

    fn request(depth: usize) -> DrillingRequest {
        DrillingRequest {
            top: component(0),
            path: (0..depth).map(instance).collect(),
            property: pid(LABEL),
        }
    }

    fn string() -> TypeSpec {
        TypeSpec::required(ExpressionType::String)
    }

    /// 새 property만으로 이루어진 `depth`단계 계획의 기대값.
    fn assert_fresh_plan(plan: &DrillingPlan, depth: usize) {
        assert_eq!(plan.components().len(), depth + 1);
        for (k, component_ref) in plan.components().iter().enumerate() {
            assert_eq!(*component_ref, component(k));
        }
        assert_eq!(plan.new_properties().len(), depth);
        assert_eq!(plan.links().len(), depth);
        assert!(plan.reused().is_empty());
        for k in 0..depth {
            let property = &plan.new_properties()[k];
            assert_eq!(property.boundary, k);
            assert_eq!(property.component, component(k));
            assert_eq!(property.base_name, "label");
            assert_eq!(property.value_type, string());
            assert_eq!(property.default, Some(TypedValue::String("확인".into())));
            let link = plan.links()[k];
            assert_eq!(link.boundary, k);
            assert_eq!(link.owner, component(k));
            assert_eq!(link.instance, instance(k));
            assert_eq!(link.source, PlannedProperty::New { boundary: k });
            let target = if k + 1 == depth {
                PlannedProperty::Existing(pid(LABEL))
            } else {
                PlannedProperty::New { boundary: k + 1 }
            };
            assert_eq!(link.target, target, "경계 {k}");
        }
        assert_eq!(plan.exposed(), PlannedProperty::New { boundary: 0 });
    }

    #[test]
    fn plans_for_one_two_and_ten_level_nesting_are_exact() {
        // 검증 기준(IX.4.3): 1·2·10단계 nesting fixture의 계획 정확.
        for depth in [1, 2, 10] {
            let library = nested_library(depth);
            let set = DrillingSet::new();
            let before = (library.encode(), set.encode());
            let plan = DrillingPlan::compute(&library, &set, request(depth)).unwrap();
            assert_fresh_plan(&plan, depth);
            assert!(!plan.is_noop());
            // 계산만 하고 적용하지 않습니다.
            assert_eq!((library.encode(), set.encode()), before, "depth {depth}");
            // 같은 입력은 같은 계획.
            assert_eq!(
                DrillingPlan::compute(&library, &set, request(depth)).unwrap(),
                plan
            );
        }
    }

    #[test]
    fn existing_links_are_reused_and_exposure_follows_their_source() {
        // 2단계: C1에 이미 title(Optional String) → I1.label link가 있음.
        let mut library = nested_library(2);
        let title = crate::component::property::ComponentProperty::new(
            pid(41_001),
            "title".into(),
            TypeSpec::optional(ExpressionType::String),
            crate::component::typed_expr::test_util::literal(TypedValue::String("제목".into())),
            PropertyVisibility::Public,
        )
        .unwrap();
        declare(&mut library, 1, title);
        let mut set = DrillingSet::new();
        set.insert(DrillingBinding::link(
            bid(1),
            component(1),
            pid(41_001),
            instance(1),
            pid(LABEL),
        ))
        .unwrap();
        let plan = DrillingPlan::compute(&library, &set, request(2)).unwrap();
        assert_eq!(
            plan.reused(),
            &[ReusedLink {
                boundary: 1,
                binding: bid(1)
            }]
        );
        assert_eq!(plan.new_properties().len(), 1);
        let top = &plan.new_properties()[0];
        assert_eq!((top.boundary, top.base_name.as_str()), (0, "title"));
        assert_eq!(top.value_type, TypeSpec::optional(ExpressionType::String));
        assert_eq!(top.default, Some(TypedValue::String("제목".into())));
        assert_eq!(
            plan.links(),
            &[PlannedLink {
                boundary: 0,
                owner: component(0),
                instance: instance(0),
                source: PlannedProperty::New { boundary: 0 },
                target: PlannedProperty::Existing(pid(41_001)),
            }]
        );

        // C0까지 이미 연결되어 있으면 새 항목 없는 계획.
        declare(&mut library, 0, string_property(41_000, "title"));
        set.insert(DrillingBinding::link(
            bid(2),
            component(0),
            pid(41_000),
            instance(0),
            pid(41_001),
        ))
        .unwrap();
        let plan = DrillingPlan::compute(&library, &set, request(2)).unwrap();
        assert!(plan.is_noop());
        assert_eq!(plan.exposed(), PlannedProperty::Existing(pid(41_000)));
        assert_eq!(plan.reused().len(), 2);
        assert_eq!(plan.reused()[0].boundary, 0);
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let mut library = nested_library(2);
        let set = DrillingSet::new();
        // private property.
        assert_eq!(
            DrillingPlan::compute(
                &library,
                &set,
                DrillingRequest {
                    property: pid(SECRET),
                    ..request(2)
                }
            )
            .unwrap_err(),
            DrillingPlanError::Drilling(DrillingError::PrivateProperty {
                component: component(2),
                property: pid(SECRET)
            })
        );
        // 경계 건너뛰기, 빈 path, 없는 property.
        assert!(matches!(
            DrillingPlan::compute(
                &library,
                &set,
                DrillingRequest {
                    path: vec![instance(1)],
                    ..request(2)
                }
            ),
            Err(DrillingPlanError::Drilling(
                DrillingError::InstanceOutOfScope { .. }
            ))
        ));
        assert_eq!(
            DrillingPlan::compute(
                &library,
                &set,
                DrillingRequest {
                    path: Vec::new(),
                    ..request(2)
                }
            )
            .unwrap_err(),
            DrillingPlanError::Drilling(DrillingError::NoBoundary)
        );
        assert!(matches!(
            DrillingPlan::compute(
                &library,
                &set,
                DrillingRequest {
                    property: pid(49_999),
                    ..request(2)
                }
            ),
            Err(DrillingPlanError::Drilling(
                DrillingError::UnknownProperty { .. }
            ))
        ));
        // 경계 자리를 정적 override가 채우면 거부.
        let mut payload = ComponentInstancePayload::new(component(2));
        payload.set_property_override(
            OverrideKey::direct(pid(LABEL)),
            TypedValue::String("정적".into()),
        );
        library
            .entry_mut(component(1))
            .unwrap()
            .tree_mut()
            .set_payload(instance(1), payload.encode());
        assert_eq!(
            DrillingPlan::compute(&library, &set, request(2)).unwrap_err(),
            DrillingPlanError::StaticOverride {
                boundary: 1,
                instance: instance(1),
                property: pid(LABEL)
            }
        );
    }
}
