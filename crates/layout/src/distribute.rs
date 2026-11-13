//! 공간 분배와 잔여 subpixel 처리 (X.3.2, crate 내부).
//!
//! 분배는 정수 subpixel에서 내림 몫을 먼저 주고, 남은 subpixel(칸 수 미만)을 **목록 순서**대로
//! 앞 칸부터 1씩 줍니다. 호출자는 목록을 child visual order(또는 track 순서)로 넘기므로 잔여값
//! 처리 순서는 StableId가 아니라 visual order입니다.

use crate::rounding::{LayoutUnit, LayoutUnitError};

/// 비음수 `total`을 `count`칸에 균등 분배합니다. `count == 0`이면 빈 목록입니다.
pub(crate) fn distribute_equal(total: LayoutUnit, count: usize) -> Vec<LayoutUnit> {
    distribute_weighted(total, &vec![1; count]).unwrap_or_default()
}

/// 비음수 `total`을 정수 가중치 비례로 분배합니다.
///
/// 칸 `i`의 몫은 `floor(total · w_i / Σw)`이고, 남은 subpixel은 가중치가 0이 아닌 칸에 목록 순서대로
/// 1씩 줍니다. 가중치 합이 0이면 모두 0입니다. 합은 항상 `total`(가중치 합 > 0일 때)입니다.
pub(crate) fn distribute_weighted(
    total: LayoutUnit,
    weights: &[u64],
) -> Result<Vec<LayoutUnit>, LayoutUnitError> {
    let total_raw = i128::from(total.raw().max(0));
    let weight_sum: i128 = weights.iter().map(|w| i128::from(*w)).sum();
    if weight_sum == 0 {
        return Ok(vec![LayoutUnit::ZERO; weights.len()]);
    }
    let mut shares: Vec<i128> = weights
        .iter()
        .map(|w| total_raw * i128::from(*w) / weight_sum)
        .collect();
    let mut residual = total_raw - shares.iter().sum::<i128>();
    for (share, weight) in shares.iter_mut().zip(weights) {
        if residual == 0 {
            break;
        }
        if *weight > 0 {
            *share += 1;
            residual -= 1;
        }
    }
    shares
        .into_iter()
        .map(|share| {
            i32::try_from(share)
                .map(LayoutUnit::from_raw)
                .map_err(|_| LayoutUnitError::OutOfRange)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(values: &[LayoutUnit]) -> Vec<i32> {
        values.iter().map(LayoutUnit::raw).collect()
    }

    #[test]
    fn equal_distribution_gives_residual_in_order() {
        // 10 subpixel / 3 → 3,3,3 + 잔여 1은 첫 칸.
        assert_eq!(
            raw(&distribute_equal(LayoutUnit::from_raw(10), 3)),
            vec![4, 3, 3]
        );
        assert_eq!(
            raw(&distribute_equal(LayoutUnit::from_raw(11), 3)),
            vec![4, 4, 3]
        );
        assert_eq!(
            raw(&distribute_equal(LayoutUnit::from_raw(9), 3)),
            vec![3, 3, 3]
        );
        assert!(distribute_equal(LayoutUnit::from_raw(9), 0).is_empty());
        // 음수 total은 0으로 봅니다.
        assert_eq!(
            raw(&distribute_equal(LayoutUnit::from_raw(-5), 2)),
            vec![0, 0]
        );
    }

    #[test]
    fn weighted_distribution_sums_to_total() {
        let shares = distribute_weighted(LayoutUnit::from_raw(100), &[1, 2, 0, 3]).unwrap();
        // floor: 16, 33, 0, 50 = 99 → 잔여 1은 가중치 > 0인 첫 칸.
        assert_eq!(raw(&shares), vec![17, 33, 0, 50]);
        let zero = distribute_weighted(LayoutUnit::from_raw(7), &[0, 0]).unwrap();
        assert_eq!(raw(&zero), vec![0, 0]);
        // 잔여는 가중치 0 칸을 건너뜁니다.
        let skip = distribute_weighted(LayoutUnit::from_raw(5), &[0, 1, 1]).unwrap();
        assert_eq!(raw(&skip), vec![0, 3, 2]);
    }
}
