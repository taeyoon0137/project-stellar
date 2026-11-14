//! scrollback: 화면 위로 밀려난 줄의 기록 (XVIII.4.2).
//!
//! - **두 상한**: 줄 수([`ScrollbackLimits::max_lines`])와 메모리([`ScrollbackLimits::max_bytes`]).
//!   둘 중 하나라도 넘으면 가장 오래된 줄부터 버립니다. 줄 수 상한은 최대 [`MAX_SCROLLBACK_LINES`]
//!   (100만 줄)까지 설정할 수 있고, 메모리 상한은 줄이 넓어도(최대 4096열) 함께 지켜집니다.
//! - **메모리 계산**([`Scrollback::memory_bytes`]): 줄마다 셀·결합 문자 heap 크기에 더해, 줄을 담는
//!   ring buffer의 **할당된 칸 전체**(사용하지 않는 여유 칸 포함)를 셉니다. ring buffer는 메모리
//!   상한이 허락하는 칸 수까지만 늘립니다.
//! - **압축**: 줄 끝의 기본 빈 칸(기본 스타일 공백)은 저장하지 않습니다. 빈 줄은 칸 크기만 씁니다.
//! - **절대 줄 번호**: 처음부터 센 번호입니다. 버린 줄 수([`Scrollback::evicted`])가 번호의 시작이므로
//!   선택 영역 같은 위치 값이 출력이 쌓여도 같은 줄을 가리킵니다.
//! - 세션 메모리에만 있고 저장·복구 대상이 아닙니다(crate 문서).

use std::collections::VecDeque;
use std::fmt;

use super::cell::Cell;
use super::screen::Row;

/// 설정할 수 있는 줄 수 상한.
pub const MAX_SCROLLBACK_LINES: usize = 1_000_000;
/// 설정할 수 있는 메모리 상한(byte).
pub const MAX_SCROLLBACK_BYTES: usize = 1024 * 1024 * 1024;
/// 기본 줄 수 상한.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
/// 기본 메모리 상한(byte).
pub const DEFAULT_SCROLLBACK_BYTES: usize = 64 * 1024 * 1024;

/// scrollback 상한 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbackLimitError {
    /// 줄 수가 [`MAX_SCROLLBACK_LINES`]를 넘습니다.
    TooManyLines {
        /// 요청 값.
        lines: usize,
    },
    /// 메모리 상한이 [`MAX_SCROLLBACK_BYTES`]를 넘습니다.
    TooManyBytes {
        /// 요청 값.
        bytes: usize,
    },
}

impl fmt::Display for ScrollbackLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScrollbackLimitError::TooManyLines { lines } => write!(
                f,
                "scrollback 줄 수 상한이 너무 큽니다({lines}, 최대 {MAX_SCROLLBACK_LINES})"
            ),
            ScrollbackLimitError::TooManyBytes { bytes } => write!(
                f,
                "scrollback 메모리 상한이 너무 큽니다({bytes} byte, 최대 {MAX_SCROLLBACK_BYTES})"
            ),
        }
    }
}

impl std::error::Error for ScrollbackLimitError {}

/// scrollback 상한(사용자 설정 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbackLimits {
    max_lines: usize,
    max_bytes: usize,
}

impl Default for ScrollbackLimits {
    fn default() -> Self {
        ScrollbackLimits {
            max_lines: DEFAULT_SCROLLBACK_LINES,
            max_bytes: DEFAULT_SCROLLBACK_BYTES,
        }
    }
}

impl ScrollbackLimits {
    /// 상한을 검증합니다. `max_lines` 0은 scrollback을 끕니다.
    pub fn new(
        max_lines: usize,
        max_bytes: usize,
    ) -> Result<ScrollbackLimits, ScrollbackLimitError> {
        if max_lines > MAX_SCROLLBACK_LINES {
            return Err(ScrollbackLimitError::TooManyLines { lines: max_lines });
        }
        if max_bytes > MAX_SCROLLBACK_BYTES {
            return Err(ScrollbackLimitError::TooManyBytes { bytes: max_bytes });
        }
        Ok(ScrollbackLimits {
            max_lines,
            max_bytes,
        })
    }

    /// 줄 수 상한.
    pub fn max_lines(&self) -> usize {
        self.max_lines
    }

    /// 메모리 상한(byte).
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

/// scrollback에 저장한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLine {
    /// 줄 끝 기본 빈 칸을 뺀 셀.
    cells: Box<[Cell]>,
    combining: Option<Box<[(u16, char)]>>,
    wrapped: bool,
}

impl HistoryLine {
    fn from_row(row: Row) -> HistoryLine {
        let (mut cells, wrapped, combining) = row.into_parts();
        let keep = cells
            .iter()
            .rposition(|cell| !cell.is_default_blank())
            .map_or(0, |last| last + 1);
        cells.truncate(keep);
        let combining = if combining.is_empty() {
            None
        } else {
            Some(combining.into_boxed_slice())
        };
        HistoryLine {
            cells: cells.into_boxed_slice(),
            combining,
            wrapped,
        }
    }

    /// 저장한 셀(줄 끝 기본 빈 칸 제외). 그 뒤 열은 기본 빈 칸입니다.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// 자동 줄바꿈으로 다음 줄에 이어지는지.
    pub fn is_wrapped(&self) -> bool {
        self.wrapped
    }

    /// 결합 문자(열, 문자).
    pub fn combining_marks(&self) -> &[(u16, char)] {
        self.combining.as_deref().unwrap_or_default()
    }

    /// heap 크기(byte).
    fn heap_bytes(&self) -> usize {
        std::mem::size_of_val::<[Cell]>(&self.cells)
            + self
                .combining
                .as_deref()
                .map_or(0, std::mem::size_of_val::<[(u16, char)]>)
    }
}

/// scrollback 기록.
#[derive(Debug)]
pub struct Scrollback {
    lines: VecDeque<HistoryLine>,
    heap_bytes: usize,
    limits: ScrollbackLimits,
    evicted: u64,
}

const SLOT_BYTES: usize = std::mem::size_of::<HistoryLine>();

impl Scrollback {
    /// 빈 기록.
    pub fn new(limits: ScrollbackLimits) -> Scrollback {
        Scrollback {
            lines: VecDeque::new(),
            heap_bytes: 0,
            limits,
            evicted: 0,
        }
    }

    /// 상한.
    pub fn limits(&self) -> ScrollbackLimits {
        self.limits
    }

    /// 상한을 바꿉니다. 줄어들면 오래된 줄부터 버립니다.
    pub fn set_limits(&mut self, limits: ScrollbackLimits) {
        self.limits = limits;
        // 줄 수로 먼저 줄이고 남는 칸을 돌려준 뒤 메모리 상한을 맞춥니다(칸 때문에 줄을 더
        // 버리지 않게).
        let max_slots = self.max_slots();
        while self.lines.len() > max_slots {
            self.pop_oldest();
        }
        if self.lines.capacity() > max_slots {
            self.lines.shrink_to(max_slots);
        }
        while !self.lines.is_empty() && self.memory_bytes() > self.limits.max_bytes {
            self.pop_oldest();
        }
    }

    /// 저장한 줄 수.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 지금까지 버린 줄 수(첫 저장 줄의 절대 번호).
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// 저장 줄(0이 가장 오래됨).
    pub fn line(&self, index: usize) -> Option<&HistoryLine> {
        self.lines.get(index)
    }

    /// 사용 중인 메모리(byte): 줄 heap + ring buffer의 할당된 칸 전체.
    pub fn memory_bytes(&self) -> usize {
        self.heap_bytes + self.lines.capacity() * SLOT_BYTES
    }

    /// 모두 지웁니다(ED 3). 지운 줄도 버린 줄로 셉니다.
    pub fn clear(&mut self) {
        self.evicted += self.lines.len() as u64;
        self.lines.clear();
        self.heap_bytes = 0;
    }

    /// 줄을 추가합니다. 상한을 넘으면 오래된 줄부터 버립니다. 버린 줄 수를 돌려줍니다.
    pub(crate) fn push(&mut self, row: Row) -> u64 {
        let before = self.evicted;
        if self.limits.max_lines == 0 {
            self.evicted += 1;
            return 1;
        }
        let line = HistoryLine::from_row(row);
        let line_bytes = line.heap_bytes();
        if !self.make_room(line_bytes) {
            // 기록을 비워도 이 줄이 메모리 상한에 들어가지 않습니다(저장하지 않고 버린 줄로 셈).
            self.evicted += 1;
            return self.evicted - before;
        }
        self.lines.push_back(line);
        self.heap_bytes += line_bytes;
        self.evicted - before
    }

    /// 칸 수 상한: 줄 수 상한과, 칸만으로 메모리 상한의 절반을 넘지 않는 수 중 작은 값.
    fn max_slots(&self) -> usize {
        self.limits
            .max_lines
            .min(self.limits.max_bytes / 2 / SLOT_BYTES)
            .max(1)
    }

    /// 새 줄(heap `line_bytes`)을 넣을 칸과 메모리를 확보합니다. 오래된 줄을 모두 버려도 들어가지
    /// 않으면 `false`입니다.
    fn make_room(&mut self, line_bytes: usize) -> bool {
        if line_bytes + SLOT_BYTES > self.limits.max_bytes {
            return false;
        }
        let max_slots = self.max_slots();
        while self.lines.len() >= max_slots {
            self.pop_oldest();
        }
        if self.lines.len() == self.lines.capacity() {
            let capacity = self.lines.capacity();
            let grow = capacity.max(16).min(max_slots - capacity.min(max_slots));
            if grow > 0 {
                self.lines.reserve_exact(grow);
            }
        }
        let fits = |history: &Scrollback| {
            history.heap_bytes + line_bytes + history.lines.capacity() * SLOT_BYTES
                <= history.limits.max_bytes
        };
        while !self.lines.is_empty() && !fits(self) {
            self.pop_oldest();
        }
        fits(self)
    }

    fn pop_oldest(&mut self) {
        if let Some(line) = self.lines.pop_front() {
            self.heap_bytes -= line.heap_bytes();
            self.evicted += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulator::cell::{CellWidth, Color, Style};

    fn text_row(cols: usize, text: &str) -> Row {
        let mut row = Row::blank(cols, Cell::BLANK);
        row.write_text(text);
        row
    }

    #[test]
    fn limits_are_validated() {
        assert!(ScrollbackLimits::new(MAX_SCROLLBACK_LINES, MAX_SCROLLBACK_BYTES).is_ok());
        assert_eq!(
            ScrollbackLimits::new(MAX_SCROLLBACK_LINES + 1, 1),
            Err(ScrollbackLimitError::TooManyLines {
                lines: MAX_SCROLLBACK_LINES + 1
            })
        );
        assert!(matches!(
            ScrollbackLimits::new(1, MAX_SCROLLBACK_BYTES + 1),
            Err(ScrollbackLimitError::TooManyBytes { .. })
        ));
        assert_eq!(
            ScrollbackLimits::default().max_lines(),
            DEFAULT_SCROLLBACK_LINES
        );
    }

    #[test]
    fn trailing_default_blanks_are_not_stored() {
        let mut history = Scrollback::new(ScrollbackLimits::default());
        history.push(text_row(80, "abc"));
        history.push(Row::blank(80, Cell::BLANK));
        // 배경색이 있는 빈 칸은 내용이므로 저장합니다.
        let colored = Cell {
            style: Style {
                bg: Color::Indexed(4),
                ..Style::default()
            },
            ..Cell::BLANK
        };
        history.push(Row::blank(3, colored));
        assert_eq!(history.line(0).unwrap().cells().len(), 3);
        assert_eq!(history.line(1).unwrap().cells().len(), 0);
        assert_eq!(history.line(2).unwrap().cells().len(), 3);
        assert_eq!(history.line(0).unwrap().cells()[0].width, CellWidth::Narrow);
    }

    #[test]
    fn line_limit_evicts_oldest_and_keeps_absolute_numbers() {
        let limits = ScrollbackLimits::new(3, DEFAULT_SCROLLBACK_BYTES).unwrap();
        let mut history = Scrollback::new(limits);
        for n in 0..10 {
            history.push(text_row(10, &n.to_string()));
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.evicted(), 7);
        assert_eq!(history.line(0).unwrap().cells()[0].ch, '7');
        assert_eq!(history.line(2).unwrap().cells()[0].ch, '9');
        history.clear();
        assert!(history.is_empty());
        assert_eq!(history.evicted(), 10);
        // 0줄 상한은 저장하지 않고 버린 줄로만 셉니다.
        let mut off = Scrollback::new(ScrollbackLimits::new(0, 1024).unwrap());
        assert_eq!(off.push(text_row(4, "x")), 1);
        assert!(off.is_empty());
        assert_eq!(off.memory_bytes(), 0);
    }

    /// 100만 줄을 넣어도 줄 수·메모리 상한을 지킵니다(XVIII.4.2 완료 조건).
    #[test]
    fn one_million_lines_keep_memory_cap() {
        let cap = 16 * 1024 * 1024;
        let limits = ScrollbackLimits::new(MAX_SCROLLBACK_LINES, cap).unwrap();
        let mut history = Scrollback::new(limits);
        let full = "x".repeat(40);
        let mut peak = 0;
        for n in 0..1_000_000u32 {
            let mut row = text_row(40, &full);
            row.write_text(&format!("{n:>8}"));
            history.push(row);
            if n % 1024 == 0 {
                peak = peak.max(history.memory_bytes());
            }
        }
        peak = peak.max(history.memory_bytes());
        assert!(peak <= cap, "peak {peak} > cap {cap}");
        assert!(history.len() < 1_000_000);
        assert_eq!(history.evicted() + history.len() as u64, 1_000_000);
        // 가장 최근 줄이 남아 있습니다.
        let last = history.line(history.len() - 1).unwrap();
        let text: String = last.cells()[..8].iter().map(|cell| cell.ch).collect();
        assert_eq!(text, "  999999");

        // 빈 줄 100만 개는 칸 크기만 써서 메모리 상한 안에 모두 들어갑니다.
        let roomy = ScrollbackLimits::new(MAX_SCROLLBACK_LINES, 128 * 1024 * 1024).unwrap();
        let mut blank = Scrollback::new(roomy);
        for _ in 0..1_000_000 {
            blank.push(Row::blank(80, Cell::BLANK));
        }
        assert_eq!(blank.len(), 1_000_000);
        assert!(blank.memory_bytes() <= roomy.max_bytes());
        // 줄 수 상한을 넘으면 줄 수로 버립니다.
        blank.push(Row::blank(80, Cell::BLANK));
        assert_eq!(blank.len(), MAX_SCROLLBACK_LINES);
        assert_eq!(blank.evicted(), 1);
    }

    #[test]
    fn wide_lines_respect_byte_cap_and_shrinking_limits() {
        // 4096열 줄(64 KiB)도 메모리 상한을 넘지 않습니다.
        let cap = 1024 * 1024;
        let mut history = Scrollback::new(ScrollbackLimits::new(1_000, cap).unwrap());
        let wide = "w".repeat(4096);
        for _ in 0..100 {
            history.push(text_row(4096, &wide));
            assert!(history.memory_bytes() <= cap);
        }
        assert!(history.len() >= 10, "{}", history.len());
        // 상한을 줄이면 바로 맞춥니다.
        history.set_limits(ScrollbackLimits::new(2, cap).unwrap());
        assert_eq!(history.len(), 2);
        assert!(history.memory_bytes() <= cap);
        // 한 줄도 담을 수 없는 상한이면 저장하지 않습니다.
        let mut tiny = Scrollback::new(ScrollbackLimits::new(10, 64).unwrap());
        tiny.push(text_row(4096, &wide));
        assert!(tiny.is_empty());
        assert_eq!(tiny.evicted(), 1);
        // 줄 하나는 들어가도 ring buffer 칸까지 더하면 넘는 상한: 저장하지 않고 상한을 지킵니다.
        let line_heap = 4096 * std::mem::size_of::<Cell>();
        let tight = ScrollbackLimits::new(100, line_heap + SLOT_BYTES * 4).unwrap();
        let mut tight_history = Scrollback::new(tight);
        for _ in 0..5 {
            tight_history.push(text_row(4096, &wide));
            assert!(tight_history.memory_bytes() <= tight.max_bytes());
        }
        // 상한을 늘렸다 줄여도 칸 때문에 줄을 더 버리지 않습니다.
        let mut shrink = Scrollback::new(ScrollbackLimits::new(1_000, 4 * 1024 * 1024).unwrap());
        for _ in 0..500 {
            shrink.push(text_row(8, "abcdefgh"));
        }
        shrink.set_limits(ScrollbackLimits::new(100, 64 * 1024).unwrap());
        assert_eq!(shrink.len(), 100);
        assert!(shrink.memory_bytes() <= 64 * 1024);
    }
}
