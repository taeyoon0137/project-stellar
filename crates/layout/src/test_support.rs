//! 레이아웃 테스트 보조 함수 (테스트 전용).

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{FiniteF32, Length};

use crate::rounding::LayoutUnit;
use crate::style::{LayoutStyle, Size};

/// 결정적 테스트 StableId.
pub(crate) fn id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// `n` 순서의 OrderKey(같은 길이라 사전식 순서 = `n` 순서, 후행 0 없음).
pub(crate) fn key(n: u32) -> OrderKey {
    let [_, hi, mid, lo] = n.to_be_bytes();
    OrderKey::from_bytes(vec![hi, mid, lo, 1]).expect("후행 1이라 정규형")
}

/// 테스트 길이.
pub(crate) fn len(px: f32) -> Length {
    Length::new(px).expect("테스트 길이는 유한")
}

/// 정수 px.
pub(crate) fn px(value: i32) -> LayoutUnit {
    LayoutUnit::from_px_int(value).expect("테스트 px 범위")
}

/// 생략값에서 `f`로 바꾼 style.
pub(crate) fn style(f: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
    let mut style = LayoutStyle::default();
    f(&mut style);
    style
}

/// `<length>` 크기 값.
pub(crate) fn size_px(value: f32) -> Size {
    Size::Length(len(value))
}

/// `width`·`height`가 px인 style.
pub(crate) fn fixed(width: f32, height: f32) -> LayoutStyle {
    style(|s| {
        s.width = size_px(width);
        s.height = size_px(height);
    })
}

/// 유한 f32.
pub(crate) fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("테스트 값은 유한")
}
