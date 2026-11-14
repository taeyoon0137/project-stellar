//! 문자 셀 폭 (XVIII.4.1).
//!
//! 터미널 격자에서 문자 하나가 차지하는 열 수(0·1·2)입니다. 표(`vt/width_table.rs`)는
//! `crates/terminal/tools/gen_width.py`가 Unicode 데이터베이스에서 생성합니다(규칙은 생성기 문서).
//!
//! - 0: 결합 부호·서식 문자·Hangul 중성/종성 자모. 앞 셀에 붙습니다(NFD 한글 파일 이름 등).
//! - 2: East Asian Width W·F(한글 음절, CJK, 전각 문자, emoji 표현 문자).
//! - 1: 그 밖(East Asian Width A는 1로 봅니다).
//!
//! grapheme cluster 단위 폭(emoji ZWJ 연쇄 등)은 계산하지 않습니다. ZWJ 자체는 폭 0이라 앞 셀에
//! 붙고, 연쇄의 다음 emoji는 다시 2칸을 차지합니다(일반 터미널과 같은 한계).

use super::width_table::{DOUBLE_WIDTH, UNICODE_VERSION, ZERO_WIDTH};

/// 폭 표를 만든 Unicode 버전.
pub fn unicode_version() -> &'static str {
    UNICODE_VERSION
}

fn in_table(table: &[(u32, u32)], cp: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// 문자 하나의 셀 폭(0·1·2). 제어 문자는 1로 돌려주지만 parser가 print로 넘기지 않습니다.
pub fn char_width(c: char) -> u8 {
    let cp = u32::from(c);
    if cp < 0x0300 {
        // U+0300 미만의 출력 문자는 모두 폭 1입니다(생성기가 확인).
        return 1;
    }
    if in_table(ZERO_WIDTH, cp) {
        0
    } else if in_table(DOUBLE_WIDTH, cp) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_of_common_scripts() {
        for c in ['a', 'Z', ' ', '~', 'é', 'ß', '─', 'Ω', 'ж', '\u{00AD}'] {
            assert_eq!(char_width(c), 1, "{c:?}");
        }
        for c in [
            '한', '글', '漢', 'か', 'カ', 'Ａ', '　', '😀', '🚀', '\u{1100}',
        ] {
            assert_eq!(char_width(c), 2, "{c:?}");
        }
        // 결합 부호, NFD 한글 중성·종성, ZWJ, variation selector, skin tone modifier.
        for c in [
            '\u{0301}',
            '\u{1161}',
            '\u{11A8}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{1F3FB}',
            '\u{0E31}',
        ] {
            assert_eq!(char_width(c), 0, "{c:?}");
        }
        // 반각 가타카나는 1칸입니다.
        assert_eq!(char_width('ｶ'), 1);
    }

    #[test]
    fn tables_are_sorted_and_disjoint() {
        for table in [ZERO_WIDTH, DOUBLE_WIDTH] {
            assert!(table.iter().all(|&(lo, hi)| lo <= hi));
            assert!(table.windows(2).all(|pair| pair[0].1 < pair[1].0));
        }
        for &(lo, hi) in ZERO_WIDTH {
            assert!(!in_table(DOUBLE_WIDTH, lo) && !in_table(DOUBLE_WIDTH, hi));
        }
        assert!(!unicode_version().is_empty());
    }
}
