//! Unicode BiDi 처리 (X.9.2 level 해석).
//!
//! UAX #9 구현은 승인 의존성 `unicode-bidi`(ADR-0016)에 맡기고, 결과를 crate 자체 타입
//! ([`BidiLevel`], [`BidiParagraph`])으로 감쌉니다.
//!
//! - [`BidiParagraph::analyze`]: paragraph 텍스트의 byte마다 resolved embedding level(UAX #9
//!   X1~I2, 줄 단위 L1 이전)을 계산합니다. shaping itemization(X.9.2)은 이 level로 방향을 나눕니다.
//! - 기준 방향은 [`BaseDirection`]으로 명시합니다. `Auto`는 첫 strong 문자(P2·P3)로 정하고 strong
//!   문자가 없으면 LTR입니다. 실행 환경 locale이나 UI 방향을 암묵적으로 쓰지 않습니다.
//! - 문단 구분 문자(UAX #9 class B: U+2029 등)가 텍스트 안에 있으면 그 지점에서 BiDi paragraph가
//!   나뉘고, 각 BiDi paragraph는 독립적으로 해석됩니다.

use std::fmt;
use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};

use crate::shaping::TextDirection;

/// paragraph 기준 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BaseDirection {
    /// 왼쪽→오른쪽(level 0).
    #[default]
    LeftToRight,
    /// 오른쪽→왼쪽(level 1).
    RightToLeft,
    /// 첫 strong 문자로 결정(없으면 LTR).
    Auto,
}

/// BiDi embedding level(0..=125). 짝수는 LTR, 홀수는 RTL입니다.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BidiLevel(u8);

impl BidiLevel {
    /// LTR paragraph 기준 level.
    pub const LTR: BidiLevel = BidiLevel(0);
    /// RTL paragraph 기준 level.
    pub const RTL: BidiLevel = BidiLevel(1);
    /// UAX #9 최대 level(implicit 포함).
    pub const MAX: u8 = 126;

    /// level 값을 검증해 만듭니다.
    pub fn new(level: u8) -> Option<BidiLevel> {
        (level <= BidiLevel::MAX).then_some(BidiLevel(level))
    }

    /// level 값.
    pub const fn number(&self) -> u8 {
        self.0
    }

    /// RTL level인지.
    pub const fn is_rtl(&self) -> bool {
        self.0 % 2 == 1
    }

    /// level의 쓰기 방향.
    pub const fn direction(&self) -> TextDirection {
        if self.is_rtl() {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }
    }
}

impl fmt::Debug for BidiLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BidiLevel({})", self.0)
    }
}

/// paragraph 하나의 BiDi 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiParagraph {
    levels: Vec<BidiLevel>,
    paragraphs: Vec<(Range<usize>, BidiLevel)>,
    base_level: BidiLevel,
}

impl BidiParagraph {
    /// 텍스트의 byte별 resolved level을 계산합니다.
    pub fn analyze(text: &str, base: BaseDirection) -> BidiParagraph {
        let default_level = match base {
            BaseDirection::LeftToRight => Some(Level::ltr()),
            BaseDirection::RightToLeft => Some(Level::rtl()),
            BaseDirection::Auto => None,
        };
        let info = BidiInfo::new(text, default_level);
        let levels = info.levels.iter().map(|l| BidiLevel(l.number())).collect();
        let paragraphs: Vec<(Range<usize>, BidiLevel)> = info
            .paragraphs
            .iter()
            .map(|p| (p.range.clone(), BidiLevel(p.level.number())))
            .collect();
        let base_level = paragraphs
            .first()
            .map(|(_, level)| *level)
            .unwrap_or(match base {
                BaseDirection::RightToLeft => BidiLevel::RTL,
                BaseDirection::LeftToRight | BaseDirection::Auto => BidiLevel::LTR,
            });
        BidiParagraph {
            levels,
            paragraphs,
            base_level,
        }
    }

    /// byte별 resolved level(텍스트 byte 길이와 같음).
    pub fn levels(&self) -> &[BidiLevel] {
        &self.levels
    }

    /// `offset` byte의 level. 범위 밖이면 기준 level입니다.
    pub fn level_at(&self, offset: usize) -> BidiLevel {
        self.levels.get(offset).copied().unwrap_or(self.base_level)
    }

    /// 첫 BiDi paragraph의 기준 level(빈 텍스트는 요청 방향, `Auto`는 LTR).
    pub fn base_level(&self) -> BidiLevel {
        self.base_level
    }

    /// BiDi paragraph(byte 범위, 기준 level) 목록. 문단 구분 문자가 없으면 하나입니다.
    pub fn paragraphs(&self) -> &[(Range<usize>, BidiLevel)] {
        &self.paragraphs
    }

    /// RTL level이 하나라도 있는지.
    pub fn has_rtl(&self) -> bool {
        self.levels.iter().any(BidiLevel::is_rtl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_levels(text: &str, base: BaseDirection) -> Vec<u8> {
        let para = BidiParagraph::analyze(text, base);
        text.char_indices()
            .map(|(i, _)| para.level_at(i).number())
            .collect()
    }

    #[test]
    fn levels_follow_uax9_for_basic_mixed_text() {
        // LTR 문단 안 RTL 단어: 사이 공백은 L·R 사이라 문단 방향(0).
        assert_eq!(
            char_levels("ab אב cd", BaseDirection::LeftToRight),
            vec![0, 0, 0, 1, 1, 0, 0, 0]
        );
        // RTL 문단 안 LTR 단어: Latin은 2, 사이 공백은 문단 방향(1).
        assert_eq!(
            char_levels("אב cd", BaseDirection::RightToLeft),
            vec![1, 1, 1, 2, 2]
        );
        // 아랍 문맥의 유럽 숫자는 W2로 AN이 되어 level 2.
        assert_eq!(
            char_levels("عدد 12", BaseDirection::RightToLeft),
            vec![1, 1, 1, 1, 2, 2]
        );
    }

    #[test]
    fn auto_direction_uses_first_strong_character() {
        assert_eq!(
            BidiParagraph::analyze("123 אב", BaseDirection::Auto).base_level(),
            BidiLevel::RTL
        );
        assert_eq!(
            BidiParagraph::analyze("hi אב", BaseDirection::Auto).base_level(),
            BidiLevel::LTR
        );
        // strong 문자가 없으면 LTR.
        assert_eq!(
            BidiParagraph::analyze("123 !", BaseDirection::Auto).base_level(),
            BidiLevel::LTR
        );
        // 빈 텍스트는 요청 방향을 따릅니다.
        let empty = BidiParagraph::analyze("", BaseDirection::RightToLeft);
        assert_eq!(empty.base_level(), BidiLevel::RTL);
        assert!(empty.levels().is_empty());
        assert_eq!(empty.level_at(0), BidiLevel::RTL);
    }

    #[test]
    fn unicode_data_version_is_pinned() {
        // ADR-0016 Decision 7: Unicode 데이터 갱신은 golden 재검증과 함께 의도적으로만 합니다.
        assert_eq!(unicode_bidi::UNICODE_VERSION, (16, 0, 0));
    }

    #[test]
    fn level_type_invariants() {
        assert!(BidiLevel::RTL.is_rtl());
        assert!(!BidiLevel::LTR.is_rtl());
        assert_eq!(
            BidiLevel::new(2).map(|l| l.direction()),
            Some(TextDirection::LeftToRight)
        );
        assert_eq!(BidiLevel::new(127), None);
        assert_eq!(format!("{:?}", BidiLevel::RTL), "BidiLevel(1)");
        let para = BidiParagraph::analyze("a\u{2029}ב", BaseDirection::Auto);
        // 문단 구분 문자로 BiDi paragraph가 둘로 나뉘고 각각 독립 해석됩니다.
        assert_eq!(para.paragraphs().len(), 2);
        assert_eq!(para.paragraphs()[1].1, BidiLevel::RTL);
        assert!(para.has_rtl());
    }
}
