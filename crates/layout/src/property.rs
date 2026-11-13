//! 레이아웃 입력 속성 식별자 (X.1.1).
//!
//! [`LayoutProperty`]는 [`crate::input::LayoutNode`]가 가진 레이아웃 입력 속성을 가리킵니다.
//! 오류·진단이 어떤 속성 값이 잘못되었는지 표시할 때 사용합니다.

use std::fmt;

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
        }
    }
}

impl fmt::Display for LayoutProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
