//! 레이아웃 입력 속성 식별자와 dirty class registry (X.1.1, X.1.3).
//!
//! [`LayoutProperty`]는 [`crate::input::LayoutNode`]가 가진 레이아웃 입력 속성을 가리킵니다.
//! 오류·진단이 어떤 속성 값이 잘못되었는지 표시할 때 사용합니다.
//!
//! # dirty class (X.1.3)
//!
//! [`LAYOUT_PROPERTIES`] registry는 속성마다 값이 바뀌었을 때의 레이아웃 무효화 범위
//! [`DirtyClass`]를 고정합니다.
//!
//! | class | 의미 | 속성 |
//! |---|---|---|
//! | `SelfSize` | node 자신의 크기 해석이 바뀜 | width, height, content, size-limits, aspect-ratio |
//! | `ParentFlow` | 부모 배치 안에서 node 위치·참여가 바뀜 | position, flex, visibility, grid-placement, constraints |
//! | `DescendantConstraint` | node가 자식에게 주는 제약(자식 목록·배치 방식·content box)이 바뀜 | children, container-layout, padding |
//! | `PaintOnly` | 레이아웃 값이 바뀌지 않음(렌더만) | transform |
//!
//! ## 콘텐츠 크기 node의 자기 크기 변경
//!
//! registry class는 속성 하나의 **기본** 범위입니다. `DescendantConstraint` 속성(children,
//! container-layout, padding)은 node 크기가 콘텐츠로 정해질 수 있으면(`HugContent` 축, 또는 containing
//! block이 indefinite일 때 콘텐츠 크기로 측정되는 `Fill`·`Percentage` 축) node **자신의 크기**도 바꾸고,
//! 그 변화는 부모 배치까지 전파됩니다. [`dirty_classes`]는 node 상태를 보고 이때 `SelfSize`를 함께
//! 반환합니다. ancestor 전파(X.7.3)는 class 하나([`dirty_class`])가 아니라 이 집합을 써야 under-
//! invalidation이 없습니다. 두 축이 모두 `Fixed`면 콘텐츠가 크기에 영향을 주지 않으므로 기본 class만
//! 반환합니다(aspect ratio는 두 축이 definite면 적용되지 않음, X.6.3).
//!
//! **누락 검사**: [`node_properties`]는 `LayoutNode`를 `..` 없이 구조 분해하므로 node에 필드가 추가되면
//! 컴파일이 실패합니다. 테스트는 그 목록과 registry가 1:1인지 검사합니다. [`dirty_class`]는 enum 전체를
//! match하므로 class 없는 속성은 컴파일되지 않습니다.
//!
//! 문서 모델의 Frame 속성 registry(VI.3.5)와의 대응: Frame `size`는 `Width`·`Height`(SelfSize),
//! `transform`은 `Transform`(PaintOnly — Frame registry에서도 paint만 무효화), `layout-mode`는
//! `ContainerLayout`(DescendantConstraint)입니다.

use std::fmt;

use crate::input::{LayoutNode, SizingMode};

/// 레이아웃 입력 속성 (X.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutProperty {
    /// 자식 목록과 순서(OrderKey).
    Children,
    /// 자식 배치 방식(container layout).
    ContainerLayout,
    /// 안쪽 여백.
    Padding,
    /// 너비 sizing.
    Width,
    /// 높이 sizing.
    Height,
    /// leaf 콘텐츠의 intrinsic 크기·baseline.
    Content,
    /// Free 부모 안에서의 local position (X.2.1).
    Position,
    /// local transform (X.2.4).
    Transform,
    /// Stack 자식 grow·shrink·basis (X.3.3).
    Flex,
    /// 표시 여부 (X.3.5).
    Visibility,
    /// Grid item 열·행 위치 (X.4.3).
    GridPlacement,
    /// Constraint 부모 안의 등식 제약 (X.5.2).
    Constraints,
    /// 너비 : 높이 비율 (X.6.3).
    AspectRatio,
    /// 최소·최대 크기 (X.6.2).
    SizeLimits,
}

impl LayoutProperty {
    /// 사람이 읽는 이름(식별에 사용 금지 — enum 값이 식별자).
    pub const fn name(self) -> &'static str {
        match self {
            LayoutProperty::Children => "children",
            LayoutProperty::ContainerLayout => "container-layout",
            LayoutProperty::Padding => "padding",
            LayoutProperty::Width => "width",
            LayoutProperty::Height => "height",
            LayoutProperty::Content => "content",
            LayoutProperty::Position => "position",
            LayoutProperty::Transform => "transform",
            LayoutProperty::Flex => "flex",
            LayoutProperty::Visibility => "visibility",
            LayoutProperty::GridPlacement => "grid-placement",
            LayoutProperty::Constraints => "constraints",
            LayoutProperty::AspectRatio => "aspect-ratio",
            LayoutProperty::SizeLimits => "size-limits",
        }
    }
}

impl fmt::Display for LayoutProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 속성 변경의 레이아웃 무효화 범위 (X.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirtyClass {
    /// node 자신의 크기 해석이 바뀜. 크기가 실제로 바뀌면 부모 배치까지 전파됩니다.
    SelfSize,
    /// 부모 배치 안에서 node의 위치·참여가 바뀜. 부모가 자식을 다시 배치합니다.
    ParentFlow,
    /// node가 자식에게 주는 제약이 바뀜. node의 자식 배치를 다시 계산합니다. 콘텐츠 크기 node에서는
    /// 자기 크기도 바뀔 수 있어 [`dirty_classes`]가 `SelfSize`를 함께 반환합니다.
    DescendantConstraint,
    /// 레이아웃 결과가 바뀌지 않음(렌더 단계만). layout queue에 넣지 않습니다.
    PaintOnly,
}

impl DirtyClass {
    /// 레이아웃 재계산이 필요한지 여부(`PaintOnly`만 `false`).
    pub const fn affects_layout(self) -> bool {
        !matches!(self, DirtyClass::PaintOnly)
    }
}

/// 속성의 dirty class (X.1.3). enum 전체를 match하므로 새 속성은 class 없이 컴파일되지 않습니다.
pub const fn dirty_class(property: LayoutProperty) -> DirtyClass {
    match property {
        LayoutProperty::Width
        | LayoutProperty::Height
        | LayoutProperty::SizeLimits
        | LayoutProperty::AspectRatio
        | LayoutProperty::Content => DirtyClass::SelfSize,
        LayoutProperty::Position
        | LayoutProperty::Flex
        | LayoutProperty::GridPlacement
        | LayoutProperty::Constraints
        | LayoutProperty::Visibility => DirtyClass::ParentFlow,
        LayoutProperty::Children | LayoutProperty::ContainerLayout | LayoutProperty::Padding => {
            DirtyClass::DescendantConstraint
        }
        LayoutProperty::Transform => DirtyClass::PaintOnly,
    }
}

/// dirty class 집합(고정 순서 SelfSize, ParentFlow, DescendantConstraint, PaintOnly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DirtyClassSet(u8);

impl DirtyClassSet {
    /// 빈 집합.
    pub const EMPTY: DirtyClassSet = DirtyClassSet(0);

    const fn bit(class: DirtyClass) -> u8 {
        match class {
            DirtyClass::SelfSize => 1,
            DirtyClass::ParentFlow => 2,
            DirtyClass::DescendantConstraint => 4,
            DirtyClass::PaintOnly => 8,
        }
    }

    /// class 하나의 집합.
    pub const fn of(class: DirtyClass) -> DirtyClassSet {
        DirtyClassSet(DirtyClassSet::bit(class))
    }

    /// class를 더한 집합.
    pub const fn with(self, class: DirtyClass) -> DirtyClassSet {
        DirtyClassSet(self.0 | DirtyClassSet::bit(class))
    }

    /// class 포함 여부.
    pub const fn contains(self, class: DirtyClass) -> bool {
        self.0 & DirtyClassSet::bit(class) != 0
    }

    /// 레이아웃 재계산이 필요한 class가 있는지 여부.
    pub const fn affects_layout(self) -> bool {
        self.0 & !DirtyClassSet::bit(DirtyClass::PaintOnly) != 0
    }

    /// 포함된 class(고정 순서).
    pub fn iter(self) -> impl Iterator<Item = DirtyClass> {
        [
            DirtyClass::SelfSize,
            DirtyClass::ParentFlow,
            DirtyClass::DescendantConstraint,
            DirtyClass::PaintOnly,
        ]
        .into_iter()
        .filter(move |class| self.contains(*class))
    }
}

/// node 크기가 콘텐츠로 정해질 수 있는지 여부(한 축이라도 `Fixed`가 아님).
///
/// `HugContent`는 항상, `Fill`·`Percentage`는 containing block이 indefinite일 때 콘텐츠 크기로
/// 측정됩니다(X.6.1). 부모 문맥 없이 판단하므로 보수적입니다(over-invalidation만 허용).
pub fn size_may_depend_on_content(node: &LayoutNode) -> bool {
    !matches!(
        (node.width, node.height),
        (SizingMode::Fixed(_), SizingMode::Fixed(_))
    )
}

/// node 상태를 고려한 속성 변경의 dirty class 집합.
///
/// 기본은 registry class 하나([`dirty_class`])입니다. `DescendantConstraint` 속성은
/// [`size_may_depend_on_content`]인 node에서 `SelfSize`를 함께 포함합니다. `node`는 변경 전후 중 한쪽이
/// 아니라 **양쪽** 상태로 각각 물어 합집합을 써야 합니다(예: sizing을 Fixed → Hug로 바꾸는 편집은 그
/// 자체가 SelfSize).
pub fn dirty_classes(property: LayoutProperty, node: &LayoutNode) -> DirtyClassSet {
    let class = dirty_class(property);
    let set = DirtyClassSet::of(class);
    if class == DirtyClass::DescendantConstraint && size_may_depend_on_content(node) {
        set.with(DirtyClass::SelfSize)
    } else {
        set
    }
}

/// registry 항목 하나 (X.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutPropertyMetadata {
    /// 속성.
    pub property: LayoutProperty,
    /// 고정 canonical tag(dirty set 직렬화·fixture용).
    pub tag: u8,
    /// dirty class.
    pub class: DirtyClass,
}

const fn entry(property: LayoutProperty, tag: u8) -> LayoutPropertyMetadata {
    LayoutPropertyMetadata {
        property,
        tag,
        class: dirty_class(property),
    }
}

/// 레이아웃 속성 registry (X.1.3). 순서는 tag 오름차순입니다.
pub const LAYOUT_PROPERTIES: [LayoutPropertyMetadata; 14] = [
    entry(LayoutProperty::Children, 0),
    entry(LayoutProperty::ContainerLayout, 1),
    entry(LayoutProperty::Padding, 2),
    entry(LayoutProperty::Width, 3),
    entry(LayoutProperty::Height, 4),
    entry(LayoutProperty::Content, 5),
    entry(LayoutProperty::Position, 6),
    entry(LayoutProperty::Transform, 7),
    entry(LayoutProperty::Flex, 8),
    entry(LayoutProperty::Visibility, 9),
    entry(LayoutProperty::GridPlacement, 10),
    entry(LayoutProperty::Constraints, 11),
    entry(LayoutProperty::SizeLimits, 12),
    entry(LayoutProperty::AspectRatio, 13),
];

/// 속성의 registry 항목.
pub fn layout_property(property: LayoutProperty) -> Option<&'static LayoutPropertyMetadata> {
    LAYOUT_PROPERTIES.iter().find(|m| m.property == property)
}

/// `LayoutNode`의 모든 입력 필드(식별자 `id` 제외)에 대응하는 속성 목록.
///
/// `..` 없는 구조 분해라 `LayoutNode`에 필드가 추가·제거되면 컴파일이 실패합니다.
pub fn node_properties(node: &LayoutNode) -> [LayoutProperty; 14] {
    let LayoutNode {
        id: _,
        children: _,
        container: _,
        padding: _,
        width: _,
        height: _,
        content: _,
        position: _,
        transform: _,
        flex: _,
        visible: _,
        grid: _,
        constraints: _,
        limits: _,
        aspect_ratio: _,
    } = node;
    [
        LayoutProperty::Children,
        LayoutProperty::ContainerLayout,
        LayoutProperty::Padding,
        LayoutProperty::Width,
        LayoutProperty::Height,
        LayoutProperty::Content,
        LayoutProperty::Position,
        LayoutProperty::Transform,
        LayoutProperty::Flex,
        LayoutProperty::Visibility,
        LayoutProperty::GridPlacement,
        LayoutProperty::Constraints,
        LayoutProperty::SizeLimits,
        LayoutProperty::AspectRatio,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::id;

    #[test]
    fn every_layout_property_has_a_class() {
        // 검증 기준(X.1.3): 모든 layout property에 class가 존재.
        let fields = node_properties(&LayoutNode::new(id(1)));
        assert_eq!(fields.len(), LAYOUT_PROPERTIES.len());
        for property in fields {
            let metadata = layout_property(property).expect("registry 누락");
            assert_eq!(metadata.class, dirty_class(property));
        }
        for metadata in &LAYOUT_PROPERTIES {
            assert!(
                fields.contains(&metadata.property),
                "node에 없는 registry 항목: {}",
                metadata.property
            );
        }
    }

    #[test]
    fn tags_and_names_are_unique_and_sorted() {
        let tags: Vec<u8> = LAYOUT_PROPERTIES.iter().map(|m| m.tag).collect();
        assert_eq!(tags, (0..14).collect::<Vec<u8>>());
        let mut names: Vec<&str> = LAYOUT_PROPERTIES
            .iter()
            .map(|m| m.property.name())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LAYOUT_PROPERTIES.len());
    }

    #[test]
    fn dirty_classes_are_fixed() {
        let of = |class: DirtyClass| -> Vec<LayoutProperty> {
            LAYOUT_PROPERTIES
                .iter()
                .filter(|m| m.class == class)
                .map(|m| m.property)
                .collect()
        };
        assert_eq!(
            of(DirtyClass::SelfSize),
            vec![
                LayoutProperty::Width,
                LayoutProperty::Height,
                LayoutProperty::Content,
                LayoutProperty::SizeLimits,
                LayoutProperty::AspectRatio
            ]
        );
        assert_eq!(
            of(DirtyClass::ParentFlow),
            vec![
                LayoutProperty::Position,
                LayoutProperty::Flex,
                LayoutProperty::Visibility,
                LayoutProperty::GridPlacement,
                LayoutProperty::Constraints
            ]
        );
        assert_eq!(
            of(DirtyClass::DescendantConstraint),
            vec![
                LayoutProperty::Children,
                LayoutProperty::ContainerLayout,
                LayoutProperty::Padding
            ]
        );
        assert_eq!(of(DirtyClass::PaintOnly), vec![LayoutProperty::Transform]);
        assert!(!DirtyClass::PaintOnly.affects_layout());
        assert!(DirtyClass::ParentFlow.affects_layout());
    }

    #[test]
    fn descendant_constraint_on_content_sized_node_includes_self_size() {
        // 리뷰 후속: HugContent node의 padding·children·container 변경은 자기 크기도 바꿉니다.
        use crate::engine::compute_layout;
        use crate::input::{ContainerLayout, LayoutInput, SizingMode};
        use crate::stack::StackLayout;
        use crate::test_support::{key, len};
        use stellar_core_types::value::{Insets, Percentage};

        let hug = SizingMode::HugContent;
        let fixed = SizingMode::Fixed(len(50.0));
        let build = |width: SizingMode, padding: f32| {
            LayoutInput::new(
                id(1),
                vec![
                    LayoutNode::new(id(1))
                        .with_fixed_size(len(200.0), len(200.0))
                        .with_child(id(2), key(1)),
                    LayoutNode::new(id(2))
                        .with_sizing(width, hug)
                        .with_padding(Insets::new(padding, padding, padding, padding).unwrap())
                        .with_container(ContainerLayout::Stack(StackLayout::new(
                            crate::geometry::Axis::Horizontal,
                        )))
                        .with_child(id(3), key(1)),
                    LayoutNode::new(id(3)).with_fixed_size(len(10.0), len(10.0)),
                ],
            )
        };
        let width_of = |input: &LayoutInput| {
            compute_layout(input)
                .unwrap()
                .get(id(2))
                .unwrap()
                .local_rect()
                .width()
        };
        // Hug node: padding 변경 → 자기 너비 변경(부모 배치에 전파될 수 있음).
        assert_ne!(width_of(&build(hug, 0.0)), width_of(&build(hug, 5.0)));
        // Fixed 너비: 그 축은 padding과 무관(세로는 Hug라 여전히 콘텐츠 의존).
        assert_eq!(width_of(&build(fixed, 0.0)), width_of(&build(fixed, 5.0)));

        let node = |width: SizingMode, height: SizingMode| {
            LayoutNode::new(id(1)).with_sizing(width, height)
        };
        let both_fixed = node(fixed, fixed);
        let descendant = DirtyClassSet::of(DirtyClass::DescendantConstraint);
        let with_self = descendant.with(DirtyClass::SelfSize);
        for property in [
            LayoutProperty::Padding,
            LayoutProperty::Children,
            LayoutProperty::ContainerLayout,
        ] {
            assert_eq!(dirty_classes(property, &both_fixed), descendant);
            for content_sized in [
                node(hug, fixed),
                node(fixed, hug),
                node(SizingMode::Fill, fixed),
                node(
                    fixed,
                    SizingMode::Percentage(Percentage::new(50.0).unwrap()),
                ),
            ] {
                assert_eq!(dirty_classes(property, &content_sized), with_self);
            }
        }
        // 다른 class는 node 상태와 무관하게 registry class 하나.
        let hug_node = node(hug, hug);
        for metadata in &LAYOUT_PROPERTIES {
            if metadata.class != DirtyClass::DescendantConstraint {
                assert_eq!(
                    dirty_classes(metadata.property, &hug_node),
                    DirtyClassSet::of(metadata.class)
                );
            }
        }
        // 집합 연산.
        assert_eq!(
            with_self.iter().collect::<Vec<_>>(),
            vec![DirtyClass::SelfSize, DirtyClass::DescendantConstraint]
        );
        assert!(with_self.affects_layout());
        assert!(!DirtyClassSet::of(DirtyClass::PaintOnly).affects_layout());
        assert!(!DirtyClassSet::EMPTY.affects_layout());
        assert!(!with_self.contains(DirtyClass::ParentFlow));
        assert!(size_may_depend_on_content(&hug_node));
        assert!(!size_may_depend_on_content(&both_fixed));
    }

    #[test]
    fn paint_only_transform_does_not_change_layout() {
        // PaintOnly 분류의 근거(X.2.4): transform만 바꾼 입력은 레이아웃 결과 bytes가 같습니다.
        use crate::engine::compute_layout;
        use crate::input::LayoutInput;
        use crate::test_support::{key, len};
        use stellar_core_types::value::Transform2D;
        let build = |transform: Transform2D| {
            LayoutInput::new(
                id(1),
                vec![
                    LayoutNode::new(id(1))
                        .with_fixed_size(len(50.0), len(50.0))
                        .with_child(id(2), key(1)),
                    LayoutNode::new(id(2))
                        .with_fixed_size(len(10.0), len(10.0))
                        .with_transform(transform),
                ],
            )
        };
        let rotated = Transform2D::new(0.0, 1.0, -1.0, 0.0, 5.0, 5.0).unwrap();
        assert_eq!(
            compute_layout(&build(Transform2D::identity()))
                .unwrap()
                .encode(),
            compute_layout(&build(rotated)).unwrap().encode()
        );
    }
}
