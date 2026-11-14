//! 범위 목록의 정렬 불변식 확인 (debug 빌드).
//!
//! glyph run·segment·BiDi paragraph 목록은 paragraph 텍스트를 논리 순서로 빈틈·겹침 없이 나눕니다. 줄 배치는
//! 이 불변식에 기대 줄과 겹치는 구간을 이분 탐색으로 찾습니다(`text_layout::lay_out_paragraph`,
//! `bidi::BidiParagraph::line_levels`). 목록을 만드는 곳과 쓰는 곳에서 [`debug_assert_tiles`]로 확인해,
//! 불변식이 깨지면 잘못된 배치를 조용히 내지 않고 debug 빌드(테스트)에서 panic하게 합니다. release 빌드에서는
//! 확인하지 않습니다.

use std::ops::Range;

/// `ranges`가 `0..len`을 앞에서부터 순서대로 빈틈·겹침 없이 나누는지. 빈 목록은 `len == 0`일 때만 참입니다.
pub(crate) fn tiles<'a>(ranges: impl IntoIterator<Item = &'a Range<usize>>, len: usize) -> bool {
    let mut covered = 0;
    for range in ranges {
        if range.start != covered || range.end < range.start {
            return false;
        }
        covered = range.end;
    }
    covered == len
}

/// debug 빌드에서 `ranges`가 텍스트 `0..len`을 나누지 않으면 panic합니다. `what`은 목록 이름입니다.
#[inline]
#[track_caller]
pub(crate) fn debug_assert_tiles<'a>(
    what: &str,
    ranges: impl IntoIterator<Item = &'a Range<usize>>,
    len: usize,
) {
    debug_assert!(
        tiles(ranges, len),
        "{what} 범위가 텍스트를 순서대로 빈틈없이 나누지 않습니다(텍스트 {len} byte)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_accepts_only_ordered_gapless_cover() {
        assert!(tiles(&[0..2, 2..5, 5..6], 6));
        assert!(tiles(&[], 0));
        // 빈 범위는 위치만 맞으면 허용합니다.
        assert!(tiles(&[0..2, 2..2, 2..3], 3));
        // 순서 뒤바뀜, 빈틈, 겹침, 뒤집힌 범위, 끝 미달·초과, 빈 목록에 비어 있지 않은 텍스트.
        assert!(!tiles(&[2..5, 0..2], 5));
        assert!(!tiles(&[0..2, 3..5], 5));
        assert!(!tiles(&[0..3, 2..5], 5));
        // 끝이 시작보다 앞인 범위(뒤로 돌아가 끝이 len과 맞아도 거부).
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = [0..2, 2..1];
        assert!(!tiles(&reversed, 1));
        let one = |range: Range<usize>, len: usize| tiles(std::iter::once(&range), len);
        assert!(one(0..3, 3));
        assert!(!one(0..2, 3));
        assert!(!one(0..4, 3));
        assert!(!one(1..3, 3));
        assert!(!tiles(&[], 1));
    }
}
