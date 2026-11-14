//! 내부 루프 작업량 계수기 (성능 회귀 검사용).
//!
//! 입력 크기에 비례해야 하는 내부 루프는 방문한 원소 수를 [`tick`]으로 알립니다. 테스트 빌드에서만
//! thread-local 계수기에 더하고, 일반 빌드에서는 빈 함수라 비용이 없습니다. 테스트는 [`measure`]로 호출
//! 하나의 작업량을 재서 입력 크기의 상수 배 이하인지(선형 상한), 입력을 4배로 늘렸을 때 작업량도 4배
//! 남짓인지(크기 비례, 제곱이면 16배) 확인합니다. wall-clock이 아닌 결정론적 횟수라 실행 환경 부하와
//! 무관합니다.

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static UNITS: Cell<u64> = const { Cell::new(0) };
}

/// 내부 루프가 원소 `units`개를 방문했음을 알립니다(테스트 빌드에서만 셉니다).
#[inline(always)]
pub(crate) fn tick(units: usize) {
    #[cfg(test)]
    UNITS.with(|count| count.set(count.get().saturating_add(units as u64)));
    #[cfg(not(test))]
    let _ = units;
}

/// `f`를 실행하는 동안 이 thread에서 센 작업량과 `f`의 결과.
#[cfg(test)]
pub(crate) fn measure<T>(f: impl FnOnce() -> T) -> (T, u64) {
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
