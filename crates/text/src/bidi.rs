//! Unicode BiDi 처리 (X.9.2 level 해석, X.9.3 줄 재배치·cluster mapping).
//!
//! UAX #9 해석(X1~I2)은 승인 의존성 `unicode-bidi`(ADR-0016)에 맡기고, 결과를 crate 자체 타입
//! ([`BidiLevel`], [`BidiParagraph`], [`LineOrder`])으로 감쌉니다.
//!
//! - [`BidiParagraph::analyze`]: paragraph 텍스트의 byte마다 resolved embedding level(UAX #9
//!   X1~I2, 줄 단위 L1 이전)을 계산합니다. shaping itemization(X.9.2)은 이 level로 방향을 나눕니다.
//! - [`BidiParagraph::line_order`] (X.9.3): 줄 하나에 L1(줄 끝·구분자 앞 공백을 문단 level로)과
//!   L2(가장 높은 level부터 연속 구간 뒤집기)를 적용해 **visual 순서**를 만들고, **logical cluster
//!   mapping**(visual 위치 ↔ 논리 cluster index)을 양방향으로 유지합니다. cluster는 UAX #29 extended
//!   grapheme cluster이며 재배치의 최소 단위라, 결합 부호가 base 문자와 떨어지지 않습니다(L3 효과).
//!   L4(거울 glyph)는 shaping 엔진이 RTL run에서 처리합니다.
//! - 기준 방향은 [`BaseDirection`]으로 명시합니다. `Auto`는 첫 strong 문자(P2·P3)로 정하고 strong
//!   문자가 없으면 LTR입니다. 실행 환경 locale이나 UI 방향을 암묵적으로 쓰지 않습니다.
//! - 문단 구분 문자(UAX #9 class B: U+2029 등)가 텍스트 안에 있으면 그 지점에서 BiDi paragraph가
//!   나뉘고, 각 BiDi paragraph는 독립적으로 해석됩니다.

use std::fmt;
use std::ops::Range;

use unicode_bidi::{BidiClass, BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

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
    classes: Vec<BidiClass>,
    paragraphs: Vec<(Range<usize>, BidiLevel)>,
    base_level: BidiLevel,
}

/// 줄 재배치 입력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BidiError {
    /// 해석한 텍스트와 길이가 다른 텍스트가 들어왔습니다.
    TextMismatch {
        /// 해석 시 byte 길이.
        analyzed: usize,
        /// 입력 byte 길이.
        given: usize,
    },
    /// 줄 범위가 텍스트를 벗어나거나 문자 경계가 아닙니다.
    InvalidLine {
        /// 시작.
        start: usize,
        /// 끝.
        end: usize,
    },
}

impl fmt::Display for BidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BidiError::TextMismatch { analyzed, given } => write!(
                f,
                "BiDi 해석 텍스트 길이 {analyzed}와 입력 길이 {given}이(가) 다릅니다"
            ),
            BidiError::InvalidLine { start, end } => {
                write!(
                    f,
                    "줄 범위 {start}..{end}이(가) 텍스트 문자 경계가 아닙니다"
                )
            }
        }
    }
}

impl std::error::Error for BidiError {}

/// 줄 안에서 같은 level을 가진 연속 논리 구간(visual 순서로 나열).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VisualRun {
    /// paragraph 안 byte 범위(논리 순서로 연속).
    pub range: Range<usize>,
    /// L1 적용 level.
    pub level: BidiLevel,
}

/// 줄 하나의 visual 순서와 logical cluster mapping (X.9.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOrder {
    line: Range<usize>,
    clusters: Vec<Range<usize>>,
    levels: Vec<BidiLevel>,
    visual_to_logical: Vec<usize>,
    logical_to_visual: Vec<usize>,
    runs: Vec<VisualRun>,
}

impl LineOrder {
    /// 줄 byte 범위.
    pub fn line(&self) -> Range<usize> {
        self.line.clone()
    }

    /// 논리 순서 cluster(grapheme) byte 범위.
    pub fn clusters(&self) -> &[Range<usize>] {
        &self.clusters
    }

    /// 논리 cluster별 L1 적용 level.
    pub fn cluster_levels(&self) -> &[BidiLevel] {
        &self.levels
    }

    /// visual 위치(왼쪽→오른쪽) → 논리 cluster index.
    pub fn visual_to_logical(&self) -> &[usize] {
        &self.visual_to_logical
    }

    /// 논리 cluster index → visual 위치.
    pub fn logical_to_visual(&self) -> &[usize] {
        &self.logical_to_visual
    }

    /// visual 순서 run.
    pub fn visual_runs(&self) -> &[VisualRun] {
        &self.runs
    }

    /// byte offset을 포함하는 논리 cluster index(줄 밖이면 `None`).
    pub fn cluster_at(&self, offset: usize) -> Option<usize> {
        let index = self.clusters.partition_point(|c| c.end <= offset);
        self.clusters
            .get(index)
            .filter(|c| c.start <= offset)
            .map(|_| index)
    }
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
        let classes = info.original_classes.clone();
        let paragraphs: Vec<(Range<usize>, BidiLevel)> = info
            .paragraphs
            .iter()
            .map(|p| (p.range.clone(), BidiLevel(p.level.number())))
            .collect();
        // `line_levels`의 이분 탐색이 기대는 불변식입니다(debug 빌드, `crate::tiling`).
        crate::tiling::debug_assert_tiles(
            "BiDi paragraph",
            paragraphs.iter().map(|(range, _)| range),
            text.len(),
        );
        let base_level = paragraphs
            .first()
            .map(|(_, level)| *level)
            .unwrap_or(match base {
                BaseDirection::RightToLeft => BidiLevel::RTL,
                BaseDirection::LeftToRight | BaseDirection::Auto => BidiLevel::LTR,
            });
        BidiParagraph {
            levels,
            classes,
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

    /// 테스트용: 정렬 불변식이 깨진 입력을 만들려고 BiDi paragraph 목록을 직접 고칩니다(debug 빌드 panic 검사).
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn paragraphs_mut(&mut self) -> &mut Vec<(Range<usize>, BidiLevel)> {
        &mut self.paragraphs
    }

    /// RTL level이 하나라도 있는지.
    pub fn has_rtl(&self) -> bool {
        self.levels.iter().any(BidiLevel::is_rtl)
    }

    /// 줄 `line`에 L1을 적용한 byte별 level(줄 길이만큼). `text`는 해석한 텍스트여야 합니다.
    pub fn line_levels(&self, text: &str, line: Range<usize>) -> Result<Vec<BidiLevel>, BidiError> {
        self.check_line(text, &line)?;
        let mut out = self.levels[line.clone()].to_vec();
        // BiDi paragraph 경계마다 따로 L1을 적용합니다. BiDi paragraph는 텍스트를 순서대로 빈틈없이 나누므로
        // 줄과 겹치는 첫 paragraph를 이분 탐색하고 겹치는 동안만 훑습니다(줄마다 전체를 훑지 않음). 불변식은
        // debug 빌드에서 `analyze`가 확인합니다.
        let first = self
            .paragraphs
            .partition_point(|(range, _)| range.end <= line.start);
        let overlapping = self.paragraphs[first..]
            .iter()
            .take_while(|(range, _)| range.start < line.end);
        for (para_range, para_level) in overlapping {
            crate::op_count::tick(1);
            let start = para_range.start.max(line.start);
            let end = para_range.end.min(line.end);
            if start >= end {
                continue;
            }
            let base = start - line.start;
            apply_l1(
                &text[start..end],
                &self.classes[start..end],
                &mut out[base..base + (end - start)],
                *para_level,
            );
        }
        Ok(out)
    }

    /// 줄 `line`의 visual 순서와 logical cluster mapping을 만듭니다 (X.9.3).
    pub fn line_order(&self, text: &str, line: Range<usize>) -> Result<LineOrder, BidiError> {
        let byte_levels = self.line_levels(text, line.clone())?;
        let clusters: Vec<Range<usize>> = text[line.clone()]
            .grapheme_indices(true)
            .map(|(i, g)| line.start + i..line.start + i + g.len())
            .collect();
        let levels: Vec<BidiLevel> = clusters
            .iter()
            .map(|c| byte_levels[c.start - line.start])
            .collect();

        // 논리 순서 level run(cluster index 범위).
        let mut logical_runs: Vec<Range<usize>> = Vec::new();
        for (i, level) in levels.iter().enumerate() {
            match logical_runs.last_mut() {
                Some(run) if levels[run.start] == *level => run.end = i + 1,
                _ => logical_runs.push(i..i + 1),
            }
        }
        let run_order = reorder_runs(&logical_runs, &levels);

        let mut visual_to_logical = Vec::with_capacity(clusters.len());
        let mut runs = Vec::with_capacity(run_order.len());
        for &r in &run_order {
            let run = &logical_runs[r];
            let level = levels[run.start];
            if level.is_rtl() {
                visual_to_logical.extend(run.clone().rev());
            } else {
                visual_to_logical.extend(run.clone());
            }
            runs.push(VisualRun {
                range: clusters[run.start].start..clusters[run.end - 1].end,
                level,
            });
        }
        let mut logical_to_visual = vec![0; clusters.len()];
        for (visual, &logical) in visual_to_logical.iter().enumerate() {
            logical_to_visual[logical] = visual;
        }
        Ok(LineOrder {
            line,
            clusters,
            levels,
            visual_to_logical,
            logical_to_visual,
            runs,
        })
    }

    fn check_line(&self, text: &str, line: &Range<usize>) -> Result<(), BidiError> {
        if text.len() != self.levels.len() {
            return Err(BidiError::TextMismatch {
                analyzed: self.levels.len(),
                given: text.len(),
            });
        }
        let valid = line.start <= line.end
            && line.end <= text.len()
            && text.is_char_boundary(line.start)
            && text.is_char_boundary(line.end);
        if valid {
            Ok(())
        } else {
            Err(BidiError::InvalidLine {
                start: line.start,
                end: line.end,
            })
        }
    }
}

/// UAX #9 L1: 구분자(S·B)와 그 앞 공백·isolate 서식 문자열, 줄 끝 공백열을 문단 level로 되돌립니다.
/// 보존된 explicit 서식 문자(LRE 등)·BN은 앞 문자 level을 따릅니다(`unicode-bidi`와 같은 규칙).
fn apply_l1(text: &str, classes: &[BidiClass], levels: &mut [BidiLevel], para: BidiLevel) {
    use BidiClass::{B, BN, FSI, LRE, LRI, LRO, PDF, PDI, RLE, RLI, RLO, S, WS};
    let mut reset_from: Option<usize> = Some(0);
    let mut prev = para;
    for (i, c) in text.char_indices() {
        let len = c.len_utf8();
        let mut reset_to = None;
        match classes[i] {
            B | S => {
                reset_to = Some(i + len);
                if reset_from.is_none() {
                    reset_from = Some(i);
                }
            }
            WS | FSI | LRI | RLI | PDI => {
                if reset_from.is_none() {
                    reset_from = Some(i);
                }
            }
            RLE | LRE | RLO | LRO | PDF | BN => {
                if reset_from.is_none() {
                    reset_from = Some(i);
                }
                for level in &mut levels[i..i + len] {
                    *level = prev;
                }
            }
            _ => reset_from = None,
        }
        if let (Some(from), Some(to)) = (reset_from, reset_to) {
            for level in &mut levels[from..to] {
                *level = para;
            }
            reset_from = None;
        }
        prev = levels[i];
    }
    if let Some(from) = reset_from {
        for level in &mut levels[from..] {
            *level = para;
        }
    }
}

/// UAX #9 L2: 가장 높은 level부터 가장 낮은 홀수 level까지, 그 level 이상인 연속 run 구간을
/// 뒤집습니다. 반환값은 visual 순서의 run index입니다. level은 126 이하라 반복 횟수가 유한합니다.
fn reorder_runs(runs: &[Range<usize>], levels: &[BidiLevel]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..runs.len()).collect();
    let run_level = |r: usize| levels[runs[r].start].number();
    let Some(max) = (0..runs.len()).map(run_level).max() else {
        return order;
    };
    let min = (0..runs.len()).map(run_level).min().unwrap_or(0);
    let lowest_odd = if min % 2 == 1 { min } else { min + 1 };
    let mut level = max;
    while level >= lowest_odd {
        let mut i = 0;
        while i < order.len() {
            if run_level(order[i]) < level {
                i += 1;
                continue;
            }
            let start = i;
            while i < order.len() && run_level(order[i]) >= level {
                i += 1;
            }
            order[start..i].reverse();
        }
        level -= 1;
    }
    order
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

    /// 고정 BiDi corpus 항목: (입력, 기준 방향, cluster별 L1 level, visual 순서의 논리 cluster index).
    ///
    /// 기대값은 UAX #9 규칙을 손으로 적용해 정했습니다(주석에 적용 규칙). 문자는 escape로 적어
    /// 편집기 BiDi 표시와 무관하게 읽을 수 있게 했습니다. א=U+05D0, ב=U+05D1, ג=U+05D2(R),
    /// ا=U+0627, ب=U+0628, ت=U+062A(AL).
    type CorpusCase = (&'static str, BaseDirection, &'static [u8], &'static [usize]);

    const CORPUS: &[CorpusCase] = &[
        // 1. LTR 문자만.
        ("abc", BaseDirection::LeftToRight, &[0, 0, 0], &[0, 1, 2]),
        // 2. LTR 문단의 R 문자: I1로 level 1, L2로 뒤집힘.
        (
            "\u{5D0}\u{5D1}\u{5D2}",
            BaseDirection::LeftToRight,
            &[1, 1, 1],
            &[2, 1, 0],
        ),
        // 3. Auto: 첫 strong R → 문단 level 1.
        (
            "\u{5D0}\u{5D1}\u{5D2}",
            BaseDirection::Auto,
            &[1, 1, 1],
            &[2, 1, 0],
        ),
        // 4. L 공백 R: 공백은 L·R 사이 중립(N2) → 문단 level 0.
        (
            "abc \u{5D0}\u{5D1}\u{5D2}",
            BaseDirection::LeftToRight,
            &[0, 0, 0, 0, 1, 1, 1],
            &[0, 1, 2, 3, 6, 5, 4],
        ),
        // 5. RTL 문단의 L 단어: I2로 level 2, L2 두 번 뒤집기.
        (
            "\u{5D0}\u{5D1}\u{5D2} abc",
            BaseDirection::RightToLeft,
            &[1, 1, 1, 1, 2, 2, 2],
            &[4, 5, 6, 3, 2, 1, 0],
        ),
        // 6. L 뒤 EN: W7로 L.
        (
            "abc 123 \u{5D0}\u{5D1}\u{5D2}",
            BaseDirection::LeftToRight,
            &[0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1],
            &[0, 1, 2, 3, 4, 5, 6, 7, 10, 9, 8],
        ),
        // 7. RTL 문단의 R 뒤 EN: EN 유지, I2로 level 2. 공백은 R·EN(N1에서 R) 사이라 1.
        (
            "\u{5D0}\u{5D1}\u{5D2} 123",
            BaseDirection::RightToLeft,
            &[1, 1, 1, 1, 2, 2, 2],
            &[4, 5, 6, 3, 2, 1, 0],
        ),
        // 8. AL 뒤 EN: W2로 AN, I2로 level 2.
        (
            "\u{627}\u{628}\u{62A} 12",
            BaseDirection::RightToLeft,
            &[1, 1, 1, 1, 2, 2],
            &[4, 5, 3, 2, 1, 0],
        ),
        // 9. 줄 끝 공백은 L1로 문단 level(0).
        (
            "\u{5D0}\u{5D1}\u{5D2}  ",
            BaseDirection::LeftToRight,
            &[1, 1, 1, 0, 0],
            &[2, 1, 0, 3, 4],
        ),
        // 10. RTL 문단 줄 끝 공백도 L1로 문단 level(1).
        (
            "abc  ",
            BaseDirection::RightToLeft,
            &[2, 2, 2, 1, 1],
            &[4, 3, 0, 1, 2],
        ),
        // 11. N0: 괄호 안 R만 있고 앞 문맥 L → 괄호는 문단 방향(L).
        (
            "a(\u{5D1})c",
            BaseDirection::LeftToRight,
            &[0, 0, 1, 0, 0],
            &[0, 1, 2, 3, 4],
        ),
        // 12. N0: RTL 문단, 괄호 안 L만, 앞 문맥 R → 괄호는 문단 방향(R).
        (
            "\u{5D0}\u{5D1}(cd)",
            BaseDirection::RightToLeft,
            &[1, 1, 1, 2, 2, 1],
            &[5, 3, 4, 2, 1, 0],
        ),
        // 13. RLI…PDI isolate: 내부는 level 1, isolate 문자는 바깥 중립(L 사이 → 0).
        (
            "a\u{2067}\u{5D1}\u{2069}c",
            BaseDirection::LeftToRight,
            &[0, 0, 1, 0, 0],
            &[0, 1, 2, 3, 4],
        ),
        // 14. W4: EN CS EN → EN. RTL 문단에서 level 2, 숫자 순서 유지.
        ("1,2", BaseDirection::RightToLeft, &[2, 2, 2], &[0, 1, 2]),
        // 15. L R L 혼합.
        (
            "abc \u{5D0}\u{5D1}\u{5D2} def",
            BaseDirection::LeftToRight,
            &[0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0],
            &[0, 1, 2, 3, 6, 5, 4, 7, 8, 9, 10],
        ),
        // 16. RTL 문단의 L 단어 뒤 EN: W7로 L, 사이 공백도 L·L 사이라 L(level 2).
        (
            "\u{5D0}\u{5D1}\u{5D2} abc 123",
            BaseDirection::RightToLeft,
            &[1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2],
            &[4, 5, 6, 7, 8, 9, 10, 3, 2, 1, 0],
        ),
        // 17. 결합 부호(U+05BC NSM)는 base와 한 cluster: cluster 2개를 뒤집어도 부호가 붙어 있습니다.
        (
            "\u{5D0}\u{5BC}\u{5D1}",
            BaseDirection::RightToLeft,
            &[1, 1],
            &[1, 0],
        ),
        // 18. 빈 줄.
        ("", BaseDirection::RightToLeft, &[], &[]),
        // 19. 구분자(TAB, class S)는 L1로 문단 level: RTL 문단 "abc<TAB>def".
        (
            "abc\tdef",
            BaseDirection::RightToLeft,
            &[2, 2, 2, 1, 2, 2, 2],
            &[4, 5, 6, 3, 0, 1, 2],
        ),
    ];

    #[test]
    fn fixed_bidi_corpus_passes() {
        for (index, (text, base, levels, order)) in CORPUS.iter().enumerate() {
            let para = BidiParagraph::analyze(text, *base);
            let line = para.line_order(text, 0..text.len()).expect("줄");
            let got_levels: Vec<u8> = line.cluster_levels().iter().map(|l| l.number()).collect();
            assert_eq!(&got_levels, levels, "corpus {} levels: {text:?}", index + 1);
            assert_eq!(
                line.visual_to_logical(),
                *order,
                "corpus {} order: {text:?}",
                index + 1
            );
            // 양방향 mapping이 서로 역함수입니다.
            for (visual, &logical) in line.visual_to_logical().iter().enumerate() {
                assert_eq!(line.logical_to_visual()[logical], visual);
            }
            // visual run은 줄 전체를 정확히 한 번씩 덮습니다.
            let covered: usize = line.visual_runs().iter().map(|r| r.range.len()).sum();
            assert_eq!(covered, text.len());
        }
    }

    #[test]
    fn corpus_agrees_with_reference_reordering() {
        // cluster가 문자 하나인 항목은 `unicode-bidi`의 L1(reordered_levels)·L2(reorder_visual)
        // 결과와 문자 단위로 일치해야 합니다.
        for (text, base, _, _) in CORPUS.iter().filter(|c| c.0 != "\u{5D0}\u{5BC}\u{5D1}") {
            let default_level = match base {
                BaseDirection::LeftToRight => Some(Level::ltr()),
                BaseDirection::RightToLeft => Some(Level::rtl()),
                BaseDirection::Auto => None,
            };
            let info = BidiInfo::new(text, default_level);
            let para = BidiParagraph::analyze(text, *base);
            let line = para.line_order(text, 0..text.len()).expect("줄");
            let Some(reference_para) = info.paragraphs.first() else {
                assert!(line.clusters().is_empty());
                continue;
            };
            let reference = info.reordered_levels_per_char(reference_para, 0..text.len());
            let ours: Vec<u8> = line.cluster_levels().iter().map(|l| l.number()).collect();
            let theirs: Vec<u8> = reference.iter().map(|l| l.number()).collect();
            assert_eq!(ours, theirs, "{text:?}");
            assert_eq!(
                line.visual_to_logical(),
                BidiInfo::reorder_visual(&reference).as_slice(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn visual_runs_and_cluster_lookup() {
        let text = "abc \u{5D0}\u{5D1} def";
        let para = BidiParagraph::analyze(text, BaseDirection::LeftToRight);
        let line = para.line_order(text, 0..text.len()).expect("줄");
        let runs: Vec<(Range<usize>, u8)> = line
            .visual_runs()
            .iter()
            .map(|r| (r.range.clone(), r.level.number()))
            .collect();
        assert_eq!(runs, vec![(0..4, 0), (4..8, 1), (8..12, 0)]);
        assert_eq!(line.cluster_at(5), Some(4));
        assert_eq!(line.cluster_at(4), Some(4));
        assert_eq!(line.cluster_at(12), None);
        // 부분 줄: 둘째 줄 "\u{5D0}\u{5D1} def"만 재배치.
        let second = para.line_order(text, 4..12).expect("부분 줄");
        assert_eq!(second.line(), 4..12);
        assert_eq!(second.visual_to_logical(), &[1, 0, 2, 3, 4, 5]);
        assert_eq!(second.clusters()[0], 4..6);
    }

    #[test]
    fn line_order_rejects_invalid_input() {
        let text = "a\u{5D0}";
        let para = BidiParagraph::analyze(text, BaseDirection::LeftToRight);
        assert_eq!(
            para.line_order("ab", 0..2).map(|_| ()),
            Err(BidiError::TextMismatch {
                analyzed: 3,
                given: 2
            })
        );
        assert_eq!(
            para.line_order(text, 0..2).map(|_| ()),
            Err(BidiError::InvalidLine { start: 0, end: 2 })
        );
        assert_eq!(
            para.line_levels(text, 0..9).map(|_| ()),
            Err(BidiError::InvalidLine { start: 0, end: 9 })
        );
        assert!(BidiError::InvalidLine { start: 0, end: 9 }
            .to_string()
            .contains("0..9"));
    }

    #[test]
    fn deep_embedding_is_bounded() {
        // explicit embedding 깊이는 UAX #9 상한(125)으로 제한되고 재배치는 유한 반복입니다.
        let text = format!("{}a\u{5D0}", "\u{202B}\u{202A}".repeat(200));
        let para = BidiParagraph::analyze(&text, BaseDirection::LeftToRight);
        let line = para.line_order(&text, 0..text.len()).expect("줄");
        assert!(line
            .cluster_levels()
            .iter()
            .all(|l| l.number() <= BidiLevel::MAX));
        assert_eq!(line.visual_to_logical().len(), line.clusters().len());
    }

    #[test]
    fn unicode_data_version_is_pinned() {
        // ADR-0016 Decision 7: Unicode 데이터 갱신은 golden 재검증과 함께 의도적으로만 합니다.
        assert_eq!(unicode_bidi::UNICODE_VERSION, (16, 0, 0));
        assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
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
