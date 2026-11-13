//! 레이아웃 테스트 보조 함수 (테스트 전용).

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Length;

use crate::rounding::LayoutUnit;

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
