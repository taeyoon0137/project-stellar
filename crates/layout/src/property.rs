//! 레이아웃 입력 속성 식별자와 dirty class registry (X.1.1, X.1.3, ADR-0076).
//!
//! [`LayoutProperty`]는 [`crate::input::LayoutNode`]가 가진 레이아웃 입력 속성입니다. 이름은 CSS longhand
//! 이름입니다(`padding-top` 등). 오류·진단이 어떤 속성 값이 잘못되었는지 표시할 때도 씁니다.
//!
//! # dirty class (X.1.3)
//!
//! [`LAYOUT_PROPERTIES`] registry는 속성마다 값이 바뀌었을 때의 레이아웃 무효화 범위 [`DirtyClass`]를
//! 고정합니다.
//!
//! | class | 의미 | 속성 |
//! |---|---|---|
//! | `SelfSize` | node 자신의 크기 해석이 바뀜 | `width`·`height`, `min-*`·`max-*`, `aspect-ratio`, content |
//! | `ParentFlow` | 부모 배치 안에서 node 위치·참여가 바뀜 | `position`, inset, `flex-grow`·`-shrink`·`-basis`, `align-self`·`justify-self`, `grid-*-start`·`-end` |
//! | `DescendantConstraint` | node가 자식에게 주는 제약이 바뀜 | children, `display`, `padding-*`, container longhand(`flex-direction`·`gap`·`grid-template-*` 등) |
//! | `PaintOnly` | 레이아웃 값이 바뀌지 않음(렌더만) | transform |
//!
//! `display`는 `none` 전환으로 부모 flow 참여도 바꾸므로 [`dirty_classes`]가 `ParentFlow`를 함께 돌려줍니다.
//!
//! ## 콘텐츠 크기 node의 자기 크기 변경
//!
//! `DescendantConstraint` 속성은 node 크기가 콘텐츠로 정해질 수 있으면(한 축이라도 `width`·`height`가 `<length>`가
//! 아니면) node **자신의 크기**도 바꿉니다. [`dirty_classes`]는 이때 `SelfSize`를 함께 반환합니다. 판단은 부모
//! 문맥 없이 보수적으로 합니다(over-invalidation만 허용).
//!
//! **누락 검사**: [`node_properties`]는 `LayoutNode`와 [`LayoutStyle`]을 `..` 없이 구조 분해하므로 필드가 추가되면
//! 컴파일이 실패합니다. 테스트는 그 목록과 registry가 1:1인지 검사합니다.

use std::fmt;

use crate::input::LayoutNode;
use crate::style::{LayoutStyle, Size};

macro_rules! layout_properties {
    ($( $(#[$doc:meta])* $variant:ident = $tag:literal, $name:literal, $class:ident; )*) => {
        /// 레이아웃 입력 속성 (X.1.1). 이름은 CSS longhand입니다.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum LayoutProperty {
            $( $(#[$doc])* $variant, )*
        }

        impl LayoutProperty {
            /// CSS longhand 이름(식별에 사용 금지 — enum 값이 식별자).
            pub const fn name(self) -> &'static str {
                match self {
                    $( LayoutProperty::$variant => $name, )*
                }
            }

            /// 고정 canonical tag.
            pub const fn tag(self) -> u8 {
                match self {
                    $( LayoutProperty::$variant => $tag, )*
                }
            }
        }

        /// 속성의 dirty class (X.1.3).
        pub const fn dirty_class(property: LayoutProperty) -> DirtyClass {
            match property {
                $( LayoutProperty::$variant => DirtyClass::$class, )*
            }
        }

        /// 레이아웃 속성 registry (X.1.3). 순서는 tag 오름차순입니다.
        pub const LAYOUT_PROPERTIES: &[LayoutPropertyMetadata] = &[
            $( LayoutPropertyMetadata {
                property: LayoutProperty::$variant,
                tag: $tag,
                class: DirtyClass::$class,
            }, )*
        ];
    };
}

layout_properties! {
    /// 자식 목록과 순서(OrderKey).
    Children = 0, "children", DescendantConstraint;
    /// `display`.
    Display = 1, "display", DescendantConstraint;
    /// `position`.
    Position = 2, "position", ParentFlow;
    /// `top`.
    Top = 3, "top", ParentFlow;
    /// `right`.
    Right = 4, "right", ParentFlow;
    /// `bottom`.
    Bottom = 5, "bottom", ParentFlow;
    /// `left`.
    Left = 6, "left", ParentFlow;
    /// `width`.
    Width = 7, "width", SelfSize;
    /// `height`.
    Height = 8, "height", SelfSize;
    /// `min-width`.
    MinWidth = 9, "min-width", SelfSize;
    /// `min-height`.
    MinHeight = 10, "min-height", SelfSize;
    /// `max-width`.
    MaxWidth = 11, "max-width", SelfSize;
    /// `max-height`.
    MaxHeight = 12, "max-height", SelfSize;
    /// `aspect-ratio`.
    AspectRatio = 13, "aspect-ratio", SelfSize;
    /// `padding-top`.
    PaddingTop = 14, "padding-top", DescendantConstraint;
    /// `padding-right`.
    PaddingRight = 15, "padding-right", DescendantConstraint;
    /// `padding-bottom`.
    PaddingBottom = 16, "padding-bottom", DescendantConstraint;
    /// `padding-left`.
    PaddingLeft = 17, "padding-left", DescendantConstraint;
    /// `flex-direction`.
    FlexDirection = 18, "flex-direction", DescendantConstraint;
    /// `flex-wrap`.
    FlexWrap = 19, "flex-wrap", DescendantConstraint;
    /// `justify-content`.
    JustifyContent = 20, "justify-content", DescendantConstraint;
    /// `align-content`.
    AlignContent = 21, "align-content", DescendantConstraint;
    /// `align-items`.
    AlignItems = 22, "align-items", DescendantConstraint;
    /// `justify-items`.
    JustifyItems = 23, "justify-items", DescendantConstraint;
    /// `row-gap`.
    RowGap = 24, "row-gap", DescendantConstraint;
    /// `column-gap`.
    ColumnGap = 25, "column-gap", DescendantConstraint;
    /// `flex-grow`.
    FlexGrow = 26, "flex-grow", ParentFlow;
    /// `flex-shrink`.
    FlexShrink = 27, "flex-shrink", ParentFlow;
    /// `flex-basis`.
    FlexBasis = 28, "flex-basis", ParentFlow;
    /// `align-self`.
    AlignSelf = 29, "align-self", ParentFlow;
    /// `justify-self`.
    JustifySelf = 30, "justify-self", ParentFlow;
    /// `grid-template-columns`.
    GridTemplateColumns = 31, "grid-template-columns", DescendantConstraint;
    /// `grid-template-rows`.
    GridTemplateRows = 32, "grid-template-rows", DescendantConstraint;
    /// `grid-auto-columns`.
    GridAutoColumns = 33, "grid-auto-columns", DescendantConstraint;
    /// `grid-auto-rows`.
    GridAutoRows = 34, "grid-auto-rows", DescendantConstraint;
    /// `grid-auto-flow`.
    GridAutoFlow = 35, "grid-auto-flow", DescendantConstraint;
    /// `grid-column-start`.
    GridColumnStart = 36, "grid-column-start", ParentFlow;
    /// `grid-column-end`.
    GridColumnEnd = 37, "grid-column-end", ParentFlow;
    /// `grid-row-start`.
    GridRowStart = 38, "grid-row-start", ParentFlow;
    /// `grid-row-end`.
    GridRowEnd = 39, "grid-row-end", ParentFlow;
    /// leaf 콘텐츠의 intrinsic 크기·baseline(텍스트·이미지·shape 측정 값).
    Content = 40, "content", SelfSize;
    /// local transform (X.2.4).
    Transform = 41, "transform", PaintOnly;
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

/// node 크기가 콘텐츠로 정해질 수 있는지 여부(`width`·`height` 중 하나라도 `<length>`가 아님).
///
/// `auto`·백분율·콘텐츠 keyword는 부모 문맥에 따라 콘텐츠 크기가 될 수 있으므로 보수적으로 `true`입니다.
pub fn size_may_depend_on_content(node: &LayoutNode) -> bool {
    !matches!(
        (node.style.width, node.style.height),
        (Size::Length(_), Size::Length(_))
    )
}

/// node 상태를 고려한 속성 변경의 dirty class 집합.
///
/// 기본은 registry class 하나([`dirty_class`])입니다. `DescendantConstraint` 속성은
/// [`size_may_depend_on_content`]인 node에서 `SelfSize`를 함께 포함하고, `display`는 `ParentFlow`도 포함합니다.
/// `node`는 변경 전후 **양쪽** 상태로 각각 물어 합집합을 써야 합니다.
pub fn dirty_classes(property: LayoutProperty, node: &LayoutNode) -> DirtyClassSet {
    let class = dirty_class(property);
    let mut set = DirtyClassSet::of(class);
    if property == LayoutProperty::Display {
        set = set.with(DirtyClass::ParentFlow);
    }
    if class == DirtyClass::DescendantConstraint && size_may_depend_on_content(node) {
        set = set.with(DirtyClass::SelfSize);
    }
    set
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

/// 속성의 registry 항목.
pub fn layout_property(property: LayoutProperty) -> Option<&'static LayoutPropertyMetadata> {
    LAYOUT_PROPERTIES.iter().find(|m| m.property == property)
}

/// `LayoutNode`의 모든 입력 필드(식별자 `id` 제외)에 대응하는 속성 목록(tag 순).
///
/// `..` 없는 구조 분해라 `LayoutNode`·`LayoutStyle`에 필드가 추가·제거되면 컴파일이 실패합니다.
pub fn node_properties(node: &LayoutNode) -> Vec<LayoutProperty> {
    let LayoutNode {
        id: _,
        children: _,
        style,
        content: _,
        transform: _,
    } = node;
    let LayoutStyle {
        display: _,
        position: _,
        top: _,
        right: _,
        bottom: _,
        left: _,
        width: _,
        height: _,
        min_width: _,
        min_height: _,
        max_width: _,
        max_height: _,
        aspect_ratio: _,
        padding: _,
        flex_direction: _,
        flex_wrap: _,
        justify_content: _,
        align_content: _,
        align_items: _,
        justify_items: _,
        row_gap: _,
        column_gap: _,
        flex_grow: _,
        flex_shrink: _,
        flex_basis: _,
        align_self: _,
        justify_self: _,
        grid_template_columns: _,
        grid_template_rows: _,
        grid_auto_columns: _,
        grid_auto_rows: _,
        grid_auto_flow: _,
        grid_column_start: _,
        grid_column_end: _,
        grid_row_start: _,
        grid_row_end: _,
    } = style;
    LAYOUT_PROPERTIES.iter().map(|m| m.property).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::id;

    #[test]
    fn every_layout_property_has_a_class_and_css_name() {
        let fields = node_properties(&LayoutNode::new(id(1)));
        assert_eq!(fields.len(), LAYOUT_PROPERTIES.len());
        for (tag, metadata) in LAYOUT_PROPERTIES.iter().enumerate() {
            assert_eq!(usize::from(metadata.tag), tag);
            assert_eq!(metadata.property.tag(), metadata.tag);
            assert_eq!(layout_property(metadata.property), Some(metadata));
            assert_eq!(metadata.class, dirty_class(metadata.property));
        }
        let mut names: Vec<&str> = LAYOUT_PROPERTIES
            .iter()
            .map(|m| m.property.name())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LAYOUT_PROPERTIES.len());
        // 엔진 고유 크기 방식·배치 방식 이름이 남지 않았습니다(ADR-0074).
        for name in names {
            for banned in [
                "sizing",
                "hug",
                "fill",
                "fixed",
                "stack",
                "overlay",
                "constraint",
            ] {
                assert!(!name.contains(banned), "{name}");
            }
        }
    }

    #[test]
    fn dirty_classes_depend_on_content_sizing_and_display() {
        use stellar_core_types::value::Length;
        let fixed = LayoutNode::new(id(1)).with_style(LayoutStyle {
            width: Size::Length(Length::new(10.0).unwrap()),
            height: Size::Length(Length::new(10.0).unwrap()),
            ..LayoutStyle::default()
        });
        let auto = LayoutNode::new(id(1));
        let descendant = DirtyClassSet::of(DirtyClass::DescendantConstraint);
        let with_self = descendant.with(DirtyClass::SelfSize);
        assert_eq!(
            dirty_classes(LayoutProperty::PaddingTop, &fixed),
            descendant
        );
        assert_eq!(dirty_classes(LayoutProperty::PaddingTop, &auto), with_self);
        assert_eq!(
            dirty_classes(LayoutProperty::Display, &fixed),
            descendant.with(DirtyClass::ParentFlow)
        );
        assert_eq!(
            dirty_classes(LayoutProperty::Transform, &auto),
            DirtyClassSet::of(DirtyClass::PaintOnly)
        );
        assert!(!DirtyClassSet::of(DirtyClass::PaintOnly).affects_layout());
        assert!(size_may_depend_on_content(&auto));
        assert!(!size_may_depend_on_content(&fixed));
        assert_eq!(
            with_self.iter().collect::<Vec<_>>(),
            vec![DirtyClass::SelfSize, DirtyClass::DescendantConstraint]
        );
    }
}
