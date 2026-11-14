//! 내부 루프 작업량 계수기 (성능 회귀 검사용).
//!
//! 입력 크기에 비례해야 하는 내부 루프는 방문한 원소 수를 [`tick`]으로 알립니다. 이 crate의 테스트 빌드와,
//! 기본값이 꺼진 `test-support` feature를 `[dev-dependencies]`에서 켠 다른 crate의 테스트 빌드에서만
//! thread-local 계수기에 더합니다. 일반 빌드에서는 빈 함수라 비용이 없습니다.
//!
//! 테스트는 `measure`로 호출 하나의 작업량을 재서, 입력 크기의 상수 배 이하인지(선형 상한)와 입력을 4배로
//! 늘렸을 때 작업량도 4배 남짓인지(제곱이면 16배) 확인합니다. wall-clock이 아닌 결정론적 횟수라 실행 환경
//! 부하와 무관합니다.
//!
//! 현재 계수 지점: [`NodeColumns::iter_live`](crate::node_columns::NodeColumns::iter_live)의 live 핸들 하나 —
//! 문서 전체를 훑는 모든 순회(자식 조회, index 구성, 레코드 생성)가 이 지점을 지납니다.

#[cfg(any(test, feature = "test-support"))]
use std::cell::Cell;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static UNITS: Cell<u64> = const { Cell::new(0) };
}

/// 내부 루프가 원소 `units`개를 방문했음을 알립니다(테스트 빌드에서만 셉니다).
#[inline(always)]
pub(crate) fn tick(units: usize) {
    #[cfg(any(test, feature = "test-support"))]
    UNITS.with(|count| count.set(count.get().saturating_add(units as u64)));
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = units;
}

/// `f`를 실행하는 동안 이 thread에서 센 작업량과 `f`의 결과.
#[cfg(any(test, feature = "test-support"))]
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, u64) {
    let before = UNITS.with(Cell::get);
    let value = f();
    let after = UNITS.with(Cell::get);
    (value, after.saturating_sub(before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_counts_only_ticks_inside_the_closure() {
        tick(5);
        let (value, units) = measure(|| {
            tick(2);
            tick(3);
            7
        });
        assert_eq!((value, units), (7, 5));
        assert_eq!(measure(|| ()).1, 0);
    }
}
