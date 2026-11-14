//! 화면 격자와 VT 동작 (XVIII.4.1), 화면 buffer·scrollback·선택·clipboard (XVIII.4.2).
//!
//! parser([`crate::vt::parser`])가 넘긴 동작을 행×열 격자에 적용합니다. 지원 범위는 `TERM`
//! [`TERM_NAME`](super::TERM_NAME)(macOS terminfo `xterm-256color`)이 쓰는 기본 기능입니다.
//!
//! - **cursor**: CUP·HVP·CUU·CUD·CUF·CUB·CNL·CPL·CHA·HPA·HPR·VPA·VPR, CR·LF·BS·HT·CHT·CBT·HTS·TBC,
//!   IND·NEL·RI, DECSC/DECRC·SCOSC/SCORC, origin mode(DECOM), 자동 줄바꿈(DECAWM, 마지막 열의 지연
//!   줄바꿈 `xenl`), DECTCEM, DECSCUSR.
//! - **색·속성**: SGR 0~9·21~29, 16색·밝은 16색, 256색과 24-bit 색(`;`·`:` 두 형식), 39·49.
//! - **지우기·편집**: ED·EL·ECH·ICH·DCH·IL·DL·SU·SD·REP, DECSTBM scroll region, IRM·LNM. 지운 칸은 현재
//!   배경색을 씁니다(`bce`).
//! - **문자**: UTF-8, 넓은 문자(2칸)와 결합 문자([`char_width`]), DEC Special Graphics 선 문자
//!   (`ESC ( 0`, SO/SI).
//! - **응답·알림**: DSR(5·6)·DA1·DA2 응답은 [`Screen::take_replies`]로, BEL·제목(OSC 0·2)·작업
//!   디렉터리(OSC 7)는 [`TerminalEvent`]로 냅니다. 둘 다 상한이 있습니다.
//! - RIS·DECSTR·DECALN.
//!
//! **화면 buffer**(XVIII.4.2):
//!
//! - **scrollback**: 주 화면에서 scroll region이 맨 위 줄부터일 때 올라간 줄과, 크기 축소로 위에서
//!   빠진 줄을 [`Scrollback`]에 넣습니다(줄 수·메모리 상한, [`TerminalConfig::scrollback`]). scroll
//!   region이 화면 중간이면 xterm처럼 기록하지 않습니다. ED 3은 기록을 지웁니다. RIS는 화면만
//!   초기화하고 기록은 남깁니다.
//! - **대체 화면**(DECSET 47·1047·1049): 전체 화면 프로그램용 화면입니다. 들어갈 때 빈 화면을 쓰고 나올
//!   때 주 화면을 되돌립니다. 1049는 cursor 저장·복원도 합니다. 대체 화면의 줄은 기록하지 않고,
//!   들어가 있는 동안 기록은 보이지 않습니다.
//! - **보기 위치**: [`Screen::scroll_display`]로 기록을 거슬러 봅니다. 거슬러 보는 중에 출력이 쌓이면
//!   같은 내용을 계속 보이게 위치를 옮기고, 버려진 줄만큼 줄입니다.
//! - **줄 번호**: 기록 첫 줄부터 센 절대 번호([`BufferPoint::line`])로 줄을 찾습니다
//!   ([`Screen::buffer_line`]). 선택 영역은 이 번호를 써서 출력이 쌓여도 같은 내용을 가리킵니다.
//!   기록에서 버려진 줄은 선택에서 빠지고, 대체 화면 전환·크기 변경·RIS는 선택을 지웁니다.
//! - **clipboard**: 선택 문자열 복사([`Screen::selected_text`])와 OSC 52 정책([`ClipboardAccess`],
//!   기본 거부)은 [`clipboard`](super::clipboard) 모듈 문서를 따릅니다.
//!
//! 이 모듈은 출력 내용을 제품 상태로 해석하지 않습니다. 특정 CLI 출력용 parser를 두지 않습니다
//! (기술 아키텍처 31.9 "터미널 경로에는 공급자별 출력 parser를 두지 않음").

use std::collections::VecDeque;
use std::path::PathBuf;

use super::cell::{Attrs, Cell, CellWidth, Color, Style};
use super::clipboard::{decode_base64, ClipboardAccess, CopiedText, MAX_COPY_BYTES};
use super::scrollback::{HistoryLine, Scrollback, ScrollbackLimits};
use super::selection::{is_word_char, BufferPoint, Selection, SelectionMode};
use crate::cwd::check_path_syntax;
use crate::size::TerminalSize;
use crate::vt::parser::{Params, Perform};
use crate::vt::width::char_width;

/// 보관하는 알림 수 상한(넘으면 오래된 것부터 버림).
pub const MAX_EVENTS: usize = 256;
/// 보관하는 응답 byte 상한(넘는 응답은 버림).
pub const MAX_REPLY_BYTES: usize = 4096;
/// 창 제목 길이 상한(문자).
pub const MAX_TITLE_CHARS: usize = 256;
/// 셀 하나에 붙는 결합 문자 수 상한.
pub const MAX_COMBINING_PER_CELL: usize = 4;

/// 1차 장치 속성(DA1) 응답: VT220 + ANSI color.
const DA1_REPLY: &[u8] = b"\x1b[?62;22c";
/// 2차 장치 속성(DA2) 응답.
const DA2_REPLY: &[u8] = b"\x1b[>1;10;0c";

/// 화면 알림.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// BEL.
    Bell,
    /// 창 제목이 바뀜(OSC 0·2, 제어 문자 제거·길이 제한).
    TitleChanged(String),
    /// 셸이 보고한 작업 디렉터리(OSC 7 `file://host/path`). header의 현재 cwd 관찰값으로 씁니다
    /// ([`TerminalHeader::build`](crate::header::TerminalHeader::build)). host가 이 기기인지는
    /// 호출자가 판단합니다.
    WorkingDirectory {
        /// URL의 host(비어 있을 수 있음).
        host: String,
        /// 절대 경로(percent-decoding 결과).
        path: PathBuf,
    },
    /// 프로그램이 clipboard 쓰기를 요청함(OSC 52, [`ClipboardAccess::AllowWrite`]일 때만). 실제로
    /// 쓸지는 UI가 정합니다.
    ClipboardWrite(String),
}

/// emulator 설정(사용자 설정 값, 프로젝트에 저장하지 않음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalConfig {
    /// scrollback 상한.
    pub scrollback: ScrollbackLimits,
    /// 프로그램의 clipboard 요청 정책(OSC 52).
    pub clipboard: ClipboardAccess,
}

/// cursor 모양(DECSCUSR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// 블록.
    #[default]
    Block,
    /// 밑줄.
    Underline,
    /// 세로 막대.
    Bar,
}

/// mouse 보고 방식(DECSET 1000·1002·1003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseTracking {
    /// 보고하지 않음.
    #[default]
    Off,
    /// 누름·뗌(1000).
    Click,
    /// 누른 채 이동 포함(1002).
    Drag,
    /// 모든 이동(1003).
    Motion,
}

/// 터미널 mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// IRM(SM 4): 삽입.
    pub insert: bool,
    /// LNM(SM 20): LF가 CR도 함께 함.
    pub linefeed_newline: bool,
    /// DECOM(?6): cursor 좌표가 scroll region 기준.
    pub origin: bool,
    /// DECAWM(?7): 자동 줄바꿈.
    pub autowrap: bool,
    /// DECTCEM(?25): cursor 표시.
    pub cursor_visible: bool,
    /// cursor 깜박임(?12, DECSCUSR 홀수).
    pub cursor_blinking: bool,
    /// DECCKM(?1): 방향 키를 application mode(`ESC O`)로 보냄.
    pub application_cursor_keys: bool,
    /// DECKPAM(`ESC =`): keypad application mode.
    pub application_keypad: bool,
    /// DECSCNM(?5): 화면 반전.
    pub reverse_video: bool,
    /// bracketed paste(?2004).
    pub bracketed_paste: bool,
    /// focus in/out 보고(?1004).
    pub focus_events: bool,
    /// mouse 보고 방식.
    pub mouse_tracking: MouseTracking,
    /// SGR mouse 좌표 형식(?1006).
    pub mouse_sgr: bool,
    /// synchronized output(?2026) 진행 중.
    pub synchronized_output: bool,
    /// 대체 화면(?47·?1047·?1049) 사용 중.
    pub alt_screen: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            insert: false,
            linefeed_newline: false,
            origin: false,
            autowrap: true,
            cursor_visible: true,
            cursor_blinking: false,
            application_cursor_keys: false,
            application_keypad: false,
            reverse_video: false,
            bracketed_paste: false,
            focus_events: false,
            mouse_tracking: MouseTracking::Off,
            mouse_sgr: false,
            synchronized_output: false,
            alt_screen: false,
        }
    }
}

/// 문자 집합(G0·G1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Charset {
    #[default]
    Ascii,
    DecSpecialGraphics,
}

/// DEC Special Graphics(0x5F~0x7E) 대응 문자.
const DEC_SPECIAL: [char; 32] = [
    ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─',
    '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

/// cursor 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Cursor {
    row: usize,
    col: usize,
    /// 마지막 열에 쓴 뒤 다음 문자에서 줄바꿈할 차례(`xenl`).
    pending_wrap: bool,
}

/// DECSC로 저장한 상태.
#[derive(Debug, Clone, Copy)]
struct SavedCursor {
    cursor: Cursor,
    pen: Style,
    origin: bool,
    charsets: [Charset; 2],
    gl: usize,
}

/// UI가 읽는 cursor 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    /// 행(0부터).
    pub row: usize,
    /// 열(0부터).
    pub col: usize,
    /// 표시 여부(DECTCEM).
    pub visible: bool,
    /// 모양.
    pub shape: CursorShape,
    /// 깜박임.
    pub blinking: bool,
}

/// 화면 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    cells: Vec<Cell>,
    /// 마지막 열에서 자동 줄바꿈되어 다음 줄로 이어지는지.
    wrapped: bool,
    /// 결합 문자(열, 문자). 열 순서로 붙은 순서를 유지합니다.
    combining: Vec<(u16, char)>,
}

impl Row {
    pub(crate) fn blank(cols: usize, fill: Cell) -> Row {
        Row {
            cells: vec![fill; cols],
            wrapped: false,
            combining: Vec::new(),
        }
    }

    /// 셀.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// 자동 줄바꿈으로 다음 줄에 이어지는지(복사할 때 개행을 넣지 않음).
    pub fn is_wrapped(&self) -> bool {
        self.wrapped
    }

    /// `col`에 붙은 결합 문자.
    pub fn combining(&self, col: usize) -> impl Iterator<Item = char> + '_ {
        self.combining
            .iter()
            .filter(move |(at, _)| usize::from(*at) == col)
            .map(|(_, c)| *c)
    }

    /// `[start, end)` 열의 문자열(넓은 문자 뒤 칸 제외, 결합 문자 포함, 줄 끝 공백 제외).
    pub fn text_range(&self, start: usize, end: usize) -> String {
        BufferLine::Screen(self).text_range(start, end)
    }

    /// 줄 전체 문자열(줄 끝 공백 제외).
    pub fn text(&self) -> String {
        self.text_range(0, self.cells.len())
    }

    pub(crate) fn reset(&mut self, cols: usize, fill: Cell) {
        self.cells.clear();
        self.cells.resize(cols, fill);
        self.wrapped = false;
        self.combining.clear();
    }

    pub(crate) fn set_wrapped(&mut self, wrapped: bool) {
        self.wrapped = wrapped;
    }

    pub(crate) fn into_parts(self) -> (Vec<Cell>, bool, Vec<(u16, char)>) {
        (self.cells, self.wrapped, self.combining)
    }

    /// 테스트용: 0열부터 1칸 문자를 씁니다.
    #[cfg(test)]
    pub(crate) fn write_text(&mut self, text: &str) {
        for (cell, ch) in self.cells.iter_mut().zip(text.chars()) {
            cell.ch = ch;
        }
    }

    fn remove_combining(&mut self, start: usize, end: usize) {
        if !self.combining.is_empty() {
            self.combining
                .retain(|(col, _)| !(start..end).contains(&usize::from(*col)));
        }
    }

    fn add_combining(&mut self, col: usize, c: char) {
        let Ok(at) = u16::try_from(col) else {
            return;
        };
        if self.combining(col).count() >= MAX_COMBINING_PER_CELL {
            return;
        }
        self.combining.push((at, c));
    }

    /// `col`이 넓은 문자의 반쪽이면 그 넓은 문자의 두 칸을 모두 빈 칸(스타일 유지)으로 만듭니다.
    /// 덮어쓰기·지우기 전, 또는 짝이 갈라지는 이동(ICH·DCH) 전에 호출합니다.
    fn break_wide_at(&mut self, col: usize) {
        let cols = self.cells.len();
        let (leader, spacer) = match self.cells[col].width {
            CellWidth::WideSpacer => (col.checked_sub(1), Some(col)),
            CellWidth::Wide => (Some(col), (col + 1 < cols).then_some(col + 1)),
            CellWidth::Narrow => return,
        };
        for index in [leader, spacer].into_iter().flatten() {
            let cell = &mut self.cells[index];
            *cell = Cell {
                ch: ' ',
                width: CellWidth::Narrow,
                ..*cell
            };
        }
        if let Some(leader) = leader {
            self.remove_combining(leader, leader + 1);
        }
    }

    /// 열 수를 바꿉니다(줄 다시 흘리기 없음). 잘린 넓은 문자는 빈 칸이 됩니다.
    fn resize_cols(&mut self, cols: usize) {
        if cols < self.cells.len() {
            self.cells.truncate(cols);
            if let Some(last) = self.cells.last_mut() {
                if last.width == CellWidth::Wide {
                    *last = Cell {
                        ch: ' ',
                        width: CellWidth::Narrow,
                        ..*last
                    };
                }
            }
            self.combining.retain(|(col, _)| usize::from(*col) < cols);
            self.wrapped = false;
        } else {
            self.cells.resize(cols, Cell::BLANK);
        }
    }
}

/// 화면 buffer의 줄 하나(화면 줄 또는 scrollback 줄).
#[derive(Debug, Clone, Copy)]
pub enum BufferLine<'a> {
    /// 현재 화면 줄.
    Screen(&'a Row),
    /// scrollback 줄(저장하지 않은 줄 끝은 기본 빈 칸).
    History(&'a HistoryLine),
}

impl<'a> BufferLine<'a> {
    /// 셀(저장 범위 밖은 기본 빈 칸).
    pub fn cell(&self, col: usize) -> Cell {
        self.stored_cells().get(col).copied().unwrap_or(Cell::BLANK)
    }

    /// 저장된 셀(scrollback 줄은 줄 끝 기본 빈 칸 제외).
    pub fn stored_cells(&self) -> &'a [Cell] {
        match self {
            BufferLine::Screen(row) => &row.cells,
            BufferLine::History(line) => line.cells(),
        }
    }

    /// 자동 줄바꿈으로 다음 줄에 이어지는지.
    pub fn is_wrapped(&self) -> bool {
        match self {
            BufferLine::Screen(row) => row.wrapped,
            BufferLine::History(line) => line.is_wrapped(),
        }
    }

    /// 결합 문자(열, 문자).
    pub fn combining_marks(&self) -> &'a [(u16, char)] {
        match self {
            BufferLine::Screen(row) => &row.combining,
            BufferLine::History(line) => line.combining_marks(),
        }
    }

    /// `col`의 문자(넓은 문자 뒤 칸이면 앞 칸 문자).
    pub fn char_at(&self, col: usize) -> char {
        let cell = self.cell(col);
        if cell.width == CellWidth::WideSpacer && col > 0 {
            self.cell(col - 1).ch
        } else {
            cell.ch
        }
    }

    /// `[start, end)` 열의 문자열(넓은 문자 뒤 칸 제외, 결합 문자 포함, 줄 끝 공백 제외).
    pub fn text_range(&self, start: usize, end: usize) -> String {
        self.text_with(start, end, true)
    }

    /// 줄 전체 문자열(줄 끝 공백 제외).
    pub fn text(&self) -> String {
        self.text_range(0, self.stored_cells().len())
    }

    fn text_with(&self, start: usize, end: usize, trim: bool) -> String {
        let cells = self.stored_cells();
        let marks = self.combining_marks();
        let stored_end = end.min(cells.len());
        let mut text = String::new();
        for (col, cell) in cells.iter().enumerate().take(stored_end).skip(start) {
            if cell.width == CellWidth::WideSpacer {
                continue;
            }
            text.push(cell.ch);
            text.extend(
                marks
                    .iter()
                    .filter(|(at, _)| usize::from(*at) == col)
                    .map(|(_, c)| *c),
            );
        }
        if trim {
            let trimmed = text.trim_end_matches(' ').len();
            text.truncate(trimmed);
        } else {
            // 저장하지 않은 줄 끝 빈 칸(자동 줄바꿈 줄의 이어지는 공백 보존).
            text.extend(std::iter::repeat_n(
                ' ',
                end.saturating_sub(cells.len().max(start)),
            ));
        }
        text
    }
}

/// 화면 상태: 격자, cursor, pen, mode, scroll region, tab stop, 알림·응답.
#[derive(Debug)]
pub struct Screen {
    rows: usize,
    cols: usize,
    grid: Vec<Row>,
    cursor: Cursor,
    pen: Style,
    saved: Option<SavedCursor>,
    top: usize,
    bottom: usize,
    modes: Modes,
    tabs: Vec<bool>,
    charsets: [Charset; 2],
    gl: usize,
    last_printed: Option<char>,
    cursor_shape: CursorShape,
    title: String,
    events: VecDeque<TerminalEvent>,
    replies: Vec<u8>,
    history: Scrollback,
    /// 기록을 거슬러 보는 줄 수(0이면 현재 화면).
    display_offset: usize,
    /// 대체 화면 사용 중 보관한 주 화면.
    primary: Option<Vec<Row>>,
    selection: Option<Selection>,
    clipboard: ClipboardAccess,
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|col| col > 0 && col % 8 == 0).collect()
}

impl Screen {
    /// 기본 설정의 빈 화면.
    pub fn new(size: TerminalSize) -> Screen {
        Screen::with_config(size, TerminalConfig::default())
    }

    /// 설정을 지정한 빈 화면.
    pub fn with_config(size: TerminalSize, config: TerminalConfig) -> Screen {
        let rows = usize::from(size.rows());
        let cols = usize::from(size.cols());
        Screen {
            rows,
            cols,
            grid: (0..rows).map(|_| Row::blank(cols, Cell::BLANK)).collect(),
            cursor: Cursor::default(),
            pen: Style::default(),
            saved: None,
            top: 0,
            bottom: rows - 1,
            modes: Modes::default(),
            tabs: default_tabs(cols),
            charsets: [Charset::Ascii; 2],
            gl: 0,
            last_printed: None,
            cursor_shape: CursorShape::Block,
            title: String::new(),
            events: VecDeque::new(),
            replies: Vec::new(),
            history: Scrollback::new(config.scrollback),
            display_offset: 0,
            primary: None,
            selection: None,
            clipboard: config.clipboard,
        }
    }

    // --- 읽기 API ---

    /// 행 수.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// 열 수.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// 화면 줄(0이 맨 위).
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.grid.get(row)
    }

    /// 셀.
    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.grid.get(row)?.cells.get(col)
    }

    /// 화면 줄별 문자열(줄 끝 공백 제외).
    pub fn lines(&self) -> Vec<String> {
        self.grid.iter().map(Row::text).collect()
    }

    /// cursor 상태.
    pub fn cursor(&self) -> CursorState {
        CursorState {
            row: self.cursor.row,
            col: self.cursor.col,
            visible: self.modes.cursor_visible,
            shape: self.cursor_shape,
            blinking: self.modes.cursor_blinking,
        }
    }

    /// mode.
    pub fn modes(&self) -> &Modes {
        &self.modes
    }

    /// 현재 pen(다음 출력 문자의 스타일).
    pub fn pen(&self) -> Style {
        self.pen
    }

    /// scroll region(포함 구간, 0부터).
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.top, self.bottom)
    }

    /// 창 제목.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// 쌓인 응답(DSR·DA)을 꺼냅니다. 호출자가 PTY 입력으로 씁니다.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    /// 쌓인 알림을 꺼냅니다.
    pub fn take_events(&mut self) -> Vec<TerminalEvent> {
        self.events.drain(..).collect()
    }

    // --- 내부 도우미 ---

    fn erased(&self) -> Cell {
        Cell::erased(&self.pen)
    }

    fn push_event(&mut self, event: TerminalEvent) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn reply(&mut self, bytes: &[u8]) {
        if self.replies.len() + bytes.len() <= MAX_REPLY_BYTES {
            self.replies.extend_from_slice(bytes);
        }
    }

    /// `[start, end)` 열을 지웁니다(넓은 문자 경계 보정, 현재 배경색).
    fn clear_cells(&mut self, row: usize, start: usize, end: usize) {
        let end = end.min(self.cols);
        if start >= end {
            return;
        }
        let fill = self.erased();
        let line = &mut self.grid[row];
        line.break_wide_at(start);
        line.break_wide_at(end - 1);
        line.cells[start..end].fill(fill);
        line.remove_combining(start, end);
    }

    fn clear_rows(&mut self, start: usize, end: usize) {
        let fill = self.erased();
        let cols = self.cols;
        for row in &mut self.grid[start..end] {
            row.reset(cols, fill);
        }
    }

    /// scroll region을 `n`줄 올립니다. 주 화면이고 region이 맨 위 줄부터면 올라간 줄을 기록에
    /// 넣고, 아니면 버립니다.
    pub(crate) fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        self.grid[top..=bottom].rotate_left(n);
        if top == 0 && self.primary.is_none() {
            let fill = self.erased();
            let cols = self.cols;
            let mut evicted = 0;
            for index in bottom + 1 - n..=bottom {
                let row = std::mem::replace(&mut self.grid[index], Row::blank(cols, fill));
                evicted += self.history.push(row);
            }
            self.history_grew(n, evicted);
        } else {
            self.clear_rows(bottom + 1 - n, bottom + 1);
        }
    }

    /// 기록에 `added`줄이 들어오고 `evicted`줄이 버려진 뒤 보기 위치와 선택을 맞춥니다.
    fn history_grew(&mut self, added: usize, evicted: u64) {
        if self.display_offset > 0 {
            self.display_offset = (self.display_offset + added).min(self.history.len());
        }
        if evicted > 0 {
            let first = self.first_line();
            self.selection = self.selection.and_then(|s| s.clamp_to(first));
        }
    }

    /// scroll region을 `n`줄 내립니다.
    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        self.grid[top..=bottom].rotate_right(n);
        self.clear_rows(top, top + n);
    }

    fn index(&mut self) {
        if self.cursor.row == self.bottom {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor.row == self.top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        self.index();
        if self.modes.linefeed_newline {
            self.cursor.col = 0;
        }
    }

    fn origin_top(&self) -> usize {
        if self.modes.origin {
            self.top
        } else {
            0
        }
    }

    /// 1부터 센 좌표로 이동합니다(origin mode 반영).
    fn goto(&mut self, row: usize, col: usize) {
        let (min_row, max_row) = if self.modes.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        };
        self.cursor.row = (min_row + row.saturating_sub(1)).min(max_row);
        self.cursor.col = col.saturating_sub(1).min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_row(&mut self, row: usize) {
        let col = self.cursor.col + 1;
        self.goto(row, col);
    }

    fn goto_col(&mut self, col: usize) {
        self.cursor.col = col.saturating_sub(1).min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn move_up(&mut self, n: usize) {
        let limit = if self.cursor.row >= self.top {
            self.top
        } else {
            0
        };
        self.cursor.row = self.cursor.row.saturating_sub(n).max(limit);
        self.cursor.pending_wrap = false;
    }

    fn move_down(&mut self, n: usize) {
        let limit = if self.cursor.row <= self.bottom {
            self.bottom
        } else {
            self.rows - 1
        };
        self.cursor.row = self.cursor.row.saturating_add(n).min(limit);
        self.cursor.pending_wrap = false;
    }

    fn move_right(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_add(n).min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn move_left(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
        self.cursor.pending_wrap = false;
    }

    fn tab_forward(&mut self, n: usize) {
        for _ in 0..n {
            let next = (self.cursor.col + 1..self.cols).find(|&col| self.tabs[col]);
            self.cursor.col = next.unwrap_or(self.cols - 1);
        }
        self.cursor.pending_wrap = false;
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            let previous = (0..self.cursor.col).rev().find(|&col| self.tabs[col]);
            self.cursor.col = previous.unwrap_or(0);
        }
        self.cursor.pending_wrap = false;
    }

    // --- 출력 ---

    fn print(&mut self, c: char) {
        let c = match self.charsets[self.gl] {
            Charset::DecSpecialGraphics if ('\u{5f}'..='\u{7e}').contains(&c) => {
                DEC_SPECIAL[(u32::from(c) - 0x5F) as usize]
            }
            _ => c,
        };
        let width = usize::from(char_width(c));
        if width == 0 {
            self.attach_combining(c);
            return;
        }
        if width > self.cols {
            return;
        }
        if self.cursor.pending_wrap {
            if self.modes.autowrap {
                self.grid[self.cursor.row].set_wrapped(true);
                self.cursor.col = 0;
                self.index();
            }
            self.cursor.pending_wrap = false;
        }
        if self.cursor.col + width > self.cols {
            // 넓은 문자가 마지막 열에 들어가지 않습니다.
            if self.modes.autowrap {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.clear_cells(row, col, self.cols);
                self.grid[row].set_wrapped(true);
                self.cursor.col = 0;
                self.index();
            } else {
                self.cursor.col = self.cols - width;
            }
        }
        if self.modes.insert {
            self.insert_blanks(width);
        }
        let (row, col) = (self.cursor.row, self.cursor.col);
        let style = self.pen;
        let line = &mut self.grid[row];
        line.break_wide_at(col);
        if width == 2 {
            line.break_wide_at(col + 1);
        }
        line.cells[col] = Cell {
            ch: c,
            style,
            width: if width == 2 {
                CellWidth::Wide
            } else {
                CellWidth::Narrow
            },
        };
        if width == 2 {
            line.cells[col + 1] = Cell {
                ch: ' ',
                style,
                width: CellWidth::WideSpacer,
            };
        }
        line.remove_combining(col, col + width);
        self.last_printed = Some(c);
        if col + width >= self.cols {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = col + width;
        }
    }

    /// 결합 문자를 앞 문자(지연 줄바꿈 중이면 cursor 칸)에 붙입니다.
    fn attach_combining(&mut self, c: char) {
        let (row, cursor_col) = (self.cursor.row, self.cursor.col);
        let col = if self.cursor.pending_wrap {
            cursor_col
        } else if cursor_col > 0 {
            cursor_col - 1
        } else {
            return;
        };
        let line = &mut self.grid[row];
        let col = if line.cells[col].width == CellWidth::WideSpacer && col > 0 {
            col - 1
        } else {
            col
        };
        line.add_combining(col, c);
    }

    fn repeat_last(&mut self, n: usize) {
        if let Some(c) = self.last_printed {
            // 한 번에 화면 전체 이상은 반복하지 않습니다(시간 상한).
            for _ in 0..n.min(self.rows * self.cols) {
                self.print(c);
            }
        }
    }

    // --- 편집 ---

    fn insert_blanks(&mut self, n: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let cols = self.cols;
        let n = n.min(cols - col);
        let fill = self.erased();
        let line = &mut self.grid[row];
        // 넓은 문자 뒤 칸에서 삽입하면 짝이 갈라지므로 그 문자를 지웁니다(앞 칸이면 짝째 밀림).
        if line.cells[col].width == CellWidth::WideSpacer {
            line.break_wide_at(col);
        }
        line.cells.copy_within(col..cols - n, col + n);
        line.cells[col..col + n].fill(fill);
        if let Some(last) = line.cells.last_mut() {
            if last.width == CellWidth::Wide {
                *last = Cell {
                    ch: ' ',
                    width: CellWidth::Narrow,
                    ..*last
                };
            }
        }
        for (at, _) in &mut line.combining {
            if usize::from(*at) >= col {
                *at = at.saturating_add(u16::try_from(n).unwrap_or(u16::MAX));
            }
        }
        line.combining.retain(|(at, _)| usize::from(*at) < cols);
    }

    fn delete_chars(&mut self, n: usize) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let cols = self.cols;
        let n = n.min(cols - col);
        let fill = self.erased();
        let line = &mut self.grid[row];
        // 삭제 경계가 넓은 문자를 가르면 그 문자를 지웁니다.
        if line.cells[col].width == CellWidth::WideSpacer {
            line.break_wide_at(col);
        }
        if col + n < cols && line.cells[col + n].width == CellWidth::WideSpacer {
            line.break_wide_at(col + n);
        }
        line.cells.copy_within(col + n..cols, col);
        line.cells[cols - n..].fill(fill);
        line.remove_combining(col, col + n);
        let shift = u16::try_from(n).unwrap_or(u16::MAX);
        for (at, _) in &mut line.combining {
            if usize::from(*at) >= col + n {
                *at -= shift;
            }
        }
        self.cursor.pending_wrap = false;
    }

    fn insert_lines(&mut self, n: usize) {
        let row = self.cursor.row;
        if row < self.top || row > self.bottom {
            return;
        }
        let n = n.min(self.bottom - row + 1);
        self.grid[row..=self.bottom].rotate_right(n);
        self.clear_rows(row, row + n);
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn delete_lines(&mut self, n: usize) {
        let row = self.cursor.row;
        if row < self.top || row > self.bottom {
            return;
        }
        let n = n.min(self.bottom - row + 1);
        self.grid[row..=self.bottom].rotate_left(n);
        self.clear_rows(self.bottom + 1 - n, self.bottom + 1);
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn erase_in_line(&mut self, mode: u16) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        match mode {
            0 => {
                self.clear_cells(row, col, self.cols);
                self.grid[row].set_wrapped(false);
            }
            1 => self.clear_cells(row, 0, col + 1),
            2 => {
                self.clear_cells(row, 0, self.cols);
                self.grid[row].set_wrapped(false);
            }
            _ => {}
        }
    }

    fn erase_in_display(&mut self, mode: u16) {
        let row = self.cursor.row;
        match mode {
            0 => {
                self.erase_in_line(0);
                self.clear_rows(row + 1, self.rows);
            }
            1 => {
                self.clear_rows(0, row);
                self.erase_in_line(1);
            }
            2 => self.clear_rows(0, self.rows),
            3 => {
                self.history.clear();
                self.display_offset = 0;
                let first = self.first_line();
                self.selection = self.selection.and_then(|s| s.clamp_to(first));
            }
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, params: &Params) {
        let top = usize::from(params.get_or(0, 1)) - 1;
        let bottom = (usize::from(params.get_or(1, u16::MAX)) - 1).min(self.rows - 1);
        if top < bottom {
            self.top = top;
            self.bottom = bottom;
            self.goto(1, 1);
        }
    }

    fn save_cursor(&mut self) {
        self.saved = Some(SavedCursor {
            cursor: self.cursor,
            pen: self.pen,
            origin: self.modes.origin,
            charsets: self.charsets,
            gl: self.gl,
        });
    }

    fn restore_cursor(&mut self) {
        match self.saved {
            Some(saved) => {
                self.cursor = saved.cursor;
                self.cursor.row = self.cursor.row.min(self.rows - 1);
                self.cursor.col = self.cursor.col.min(self.cols - 1);
                self.pen = saved.pen;
                self.modes.origin = saved.origin;
                self.charsets = saved.charsets;
                self.gl = saved.gl;
            }
            None => {
                self.cursor = Cursor::default();
                self.pen = Style::default();
                self.modes.origin = false;
                self.charsets = [Charset::Ascii; 2];
                self.gl = 0;
            }
        }
    }

    /// 화면 전체 초기화(RIS). 크기는 유지합니다.
    pub(crate) fn full_reset(&mut self) {
        let size = TerminalSize::new(
            u16::try_from(self.rows).unwrap_or(1),
            u16::try_from(self.cols).unwrap_or(1),
        )
        .unwrap_or(TerminalSize::DEFAULT);
        let config = TerminalConfig {
            scrollback: self.history.limits(),
            clipboard: self.clipboard,
        };
        let events = std::mem::take(&mut self.events);
        let replies = std::mem::take(&mut self.replies);
        let history = std::mem::replace(&mut self.history, Scrollback::new(config.scrollback));
        *self = Screen::with_config(size, config);
        self.events = events;
        self.replies = replies;
        self.history = history;
    }

    fn soft_reset(&mut self) {
        self.modes.insert = false;
        self.modes.origin = false;
        self.modes.autowrap = true;
        self.modes.cursor_visible = true;
        self.modes.application_cursor_keys = false;
        self.modes.application_keypad = false;
        self.pen = Style::default();
        self.top = 0;
        self.bottom = self.rows - 1;
        self.charsets = [Charset::Ascii; 2];
        self.gl = 0;
        self.saved = None;
        self.cursor.pending_wrap = false;
    }

    fn alignment_test(&mut self) {
        let fill = Cell {
            ch: 'E',
            ..Cell::BLANK
        };
        let cols = self.cols;
        for row in &mut self.grid {
            row.reset(cols, fill);
        }
        self.top = 0;
        self.bottom = self.rows - 1;
        self.cursor = Cursor::default();
    }

    /// 크기를 바꿉니다. 줄을 다시 흘리지 않고 잘라내거나 늘립니다. 줄이 줄면 cursor 줄이 보이게
    /// 위쪽 줄부터 버립니다. scroll region은 전체로 돌아갑니다.
    pub fn resize(&mut self, size: TerminalSize) {
        let rows = usize::from(size.rows());
        let cols = usize::from(size.cols());
        self.selection = None;
        if cols != self.cols {
            for row in self
                .grid
                .iter_mut()
                .chain(self.primary.iter_mut().flatten())
            {
                row.resize_cols(cols);
            }
            let mut tabs = default_tabs(cols);
            let keep = cols.min(self.cols);
            tabs[..keep].copy_from_slice(&self.tabs[..keep]);
            self.tabs = tabs;
            self.cols = cols;
        }
        if rows < self.rows {
            let excess = self.rows - rows;
            let from_top = (self.cursor.row + 1).saturating_sub(rows).min(excess);
            self.remove_top_rows(from_top);
            self.grid.truncate(rows);
            self.cursor.row -= from_top;
        } else {
            self.grid
                .extend((self.rows..rows).map(|_| Row::blank(cols, Cell::BLANK)));
        }
        if let Some(primary) = self.primary.as_mut() {
            // 보관한 주 화면은 1049가 저장한 cursor 줄이 보이게 맞춥니다.
            let keep_row = self.saved.map_or(0, |saved| saved.cursor.row);
            let removed = (keep_row + 1)
                .saturating_sub(rows)
                .min(primary.len().saturating_sub(rows));
            let mut evicted = 0;
            for row in primary.drain(..removed).collect::<Vec<Row>>() {
                evicted += self.history.push(row);
            }
            primary.truncate(rows);
            primary.extend((primary.len()..rows).map(|_| Row::blank(cols, Cell::BLANK)));
            if let Some(saved) = self.saved.as_mut() {
                saved.cursor.row = saved.cursor.row.saturating_sub(removed).min(rows - 1);
            }
            self.history_grew(removed, evicted);
        }
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.pending_wrap = false;
    }

    /// 크기 축소로 화면 위에서 빠지는 줄을 주 화면이면 기록에 넣고, 대체 화면이면 버립니다.
    fn remove_top_rows(&mut self, count: usize) {
        let removed: Vec<Row> = self.grid.drain(..count).collect();
        if self.primary.is_none() {
            let mut evicted = 0;
            for row in removed {
                evicted += self.history.push(row);
            }
            self.history_grew(count, evicted);
        }
    }

    // --- 대체 화면 ---

    fn enter_alt_screen(&mut self, save_cursor: bool) {
        if self.primary.is_some() {
            return;
        }
        if save_cursor {
            self.save_cursor();
        }
        let fill = self.erased();
        let cols = self.cols;
        let alt = (0..self.rows).map(|_| Row::blank(cols, fill)).collect();
        self.primary = Some(std::mem::replace(&mut self.grid, alt));
        self.modes.alt_screen = true;
        self.display_offset = 0;
        self.selection = None;
    }

    fn leave_alt_screen(&mut self, restore_cursor: bool) {
        let Some(primary) = self.primary.take() else {
            return;
        };
        self.grid = primary;
        self.modes.alt_screen = false;
        self.selection = None;
        if restore_cursor {
            self.restore_cursor();
        }
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.pending_wrap = false;
    }

    // --- SGR ---

    fn sgr(&mut self, params: &Params) {
        if params.is_empty() {
            self.pen = Style::default();
            return;
        }
        let mut groups = params.groups();
        while let Some(group) = groups.next() {
            let code = group[0];
            match code {
                0 => self.pen = Style::default(),
                1 => self.pen.attrs.insert(Attrs::BOLD),
                2 => self.pen.attrs.insert(Attrs::DIM),
                3 => self.pen.attrs.insert(Attrs::ITALIC),
                4 => {
                    self.pen
                        .attrs
                        .remove(Attrs::UNDERLINE.union(Attrs::DOUBLE_UNDERLINE));
                    match group.get(1).copied() {
                        Some(0) => {}
                        Some(2) => self.pen.attrs.insert(Attrs::DOUBLE_UNDERLINE),
                        _ => self.pen.attrs.insert(Attrs::UNDERLINE),
                    }
                }
                5 | 6 => self.pen.attrs.insert(Attrs::BLINK),
                7 => self.pen.attrs.insert(Attrs::INVERSE),
                8 => self.pen.attrs.insert(Attrs::HIDDEN),
                9 => self.pen.attrs.insert(Attrs::STRIKE),
                21 => self.pen.attrs.insert(Attrs::DOUBLE_UNDERLINE),
                22 => self.pen.attrs.remove(Attrs::BOLD.union(Attrs::DIM)),
                23 => self.pen.attrs.remove(Attrs::ITALIC),
                24 => self
                    .pen
                    .attrs
                    .remove(Attrs::UNDERLINE.union(Attrs::DOUBLE_UNDERLINE)),
                25 => self.pen.attrs.remove(Attrs::BLINK),
                27 => self.pen.attrs.remove(Attrs::INVERSE),
                28 => self.pen.attrs.remove(Attrs::HIDDEN),
                29 => self.pen.attrs.remove(Attrs::STRIKE),
                30..=37 => self.pen.fg = Color::Indexed((code - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((code - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((code - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let color = if group.len() > 1 {
                        colon_color(&group[1..])
                    } else {
                        semicolon_color(&mut groups)
                    };
                    match (code, color) {
                        (38, Some(color)) => self.pen.fg = color,
                        (48, Some(color)) => self.pen.bg = color,
                        // 58(밑줄 색)은 표시하지 않으므로 값만 소비합니다.
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    // --- mode ---

    fn set_ansi_mode(&mut self, params: &Params, on: bool) {
        for group in params.groups() {
            match group[0] {
                4 => self.modes.insert = on,
                20 => self.modes.linefeed_newline = on,
                _ => {}
            }
        }
    }

    fn set_dec_mode(&mut self, params: &Params, on: bool) {
        for group in params.groups() {
            match group[0] {
                1 => self.modes.application_cursor_keys = on,
                5 => self.modes.reverse_video = on,
                6 => {
                    self.modes.origin = on;
                    self.goto(1, 1);
                }
                7 => {
                    self.modes.autowrap = on;
                    if !on {
                        self.cursor.pending_wrap = false;
                    }
                }
                12 => self.modes.cursor_blinking = on,
                25 => self.modes.cursor_visible = on,
                47 | 1047 => {
                    if on {
                        self.enter_alt_screen(false);
                    } else {
                        self.leave_alt_screen(false);
                    }
                }
                1049 => {
                    if on {
                        self.enter_alt_screen(true);
                    } else {
                        self.leave_alt_screen(true);
                    }
                }
                1000 => self.set_mouse(MouseTracking::Click, on),
                1002 => self.set_mouse(MouseTracking::Drag, on),
                1003 => self.set_mouse(MouseTracking::Motion, on),
                1004 => self.modes.focus_events = on,
                1006 => self.modes.mouse_sgr = on,
                2004 => self.modes.bracketed_paste = on,
                2026 => self.modes.synchronized_output = on,
                _ => {}
            }
        }
    }

    fn set_mouse(&mut self, tracking: MouseTracking, on: bool) {
        if on {
            self.modes.mouse_tracking = tracking;
        } else if self.modes.mouse_tracking == tracking {
            self.modes.mouse_tracking = MouseTracking::Off;
        }
    }

    fn report_cursor(&mut self, private: bool) {
        let row = self.cursor.row - self.origin_top() + 1;
        let col = self.cursor.col + 1;
        let reply = if private {
            format!("\x1b[?{row};{col}R")
        } else {
            format!("\x1b[{row};{col}R")
        };
        self.reply(reply.as_bytes());
    }

    fn set_cursor_style(&mut self, style: u16) {
        let (shape, blinking) = match style {
            0 | 1 => (CursorShape::Block, true),
            2 => (CursorShape::Block, false),
            3 => (CursorShape::Underline, true),
            4 => (CursorShape::Underline, false),
            5 => (CursorShape::Bar, true),
            6 => (CursorShape::Bar, false),
            _ => return,
        };
        self.cursor_shape = shape;
        self.modes.cursor_blinking = blinking;
    }

    // --- 동작 분배 ---

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.push_event(TerminalEvent::Bell),
            0x08 => self.move_left(1),
            0x09 => self.tab_forward(1),
            0x0A..=0x0C => self.linefeed(),
            0x0D => {
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            0x0E => self.gl = 1,
            0x0F => self.gl = 0,
            _ => {}
        }
    }

    fn csi(&mut self, params: &Params, intermediates: &[u8], action: u8) {
        let n = |index: usize| usize::from(params.get_or(index, 1));
        match (intermediates, action) {
            ([], b'@') => self.insert_blanks(n(0)),
            ([], b'A') => self.move_up(n(0)),
            ([], b'B') => self.move_down(n(0)),
            ([], b'C') | ([], b'a') => self.move_right(n(0)),
            ([], b'D') => self.move_left(n(0)),
            ([], b'E') => {
                self.move_down(n(0));
                self.cursor.col = 0;
            }
            ([], b'F') => {
                self.move_up(n(0));
                self.cursor.col = 0;
            }
            ([], b'G') | ([], b'`') => self.goto_col(n(0)),
            ([], b'H') | ([], b'f') => self.goto(n(0), n(1)),
            ([], b'I') => self.tab_forward(n(0)),
            ([], b'J') => self.erase_in_display(params.get(0).unwrap_or(0)),
            ([], b'K') => self.erase_in_line(params.get(0).unwrap_or(0)),
            ([], b'L') => self.insert_lines(n(0)),
            ([], b'M') => self.delete_lines(n(0)),
            ([], b'P') => self.delete_chars(n(0)),
            ([], b'S') => self.scroll_up(n(0)),
            ([], b'T') => self.scroll_down(n(0)),
            ([], b'X') => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.clear_cells(row, col, col.saturating_add(n(0)));
            }
            ([], b'Z') => self.tab_backward(n(0)),
            ([], b'b') => self.repeat_last(n(0)),
            ([], b'c') if params.get(0).unwrap_or(0) == 0 => self.reply(DA1_REPLY),
            ([b'>'], b'c') if params.get(0).unwrap_or(0) == 0 => self.reply(DA2_REPLY),
            ([], b'd') => self.goto_row(n(0)),
            ([], b'e') => self.move_down(n(0)),
            ([], b'g') => match params.get(0).unwrap_or(0) {
                0 => self.tabs[self.cursor.col] = false,
                3 => self.tabs.fill(false),
                _ => {}
            },
            ([], b'h') => self.set_ansi_mode(params, true),
            ([], b'l') => self.set_ansi_mode(params, false),
            ([b'?'], b'h') => self.set_dec_mode(params, true),
            ([b'?'], b'l') => self.set_dec_mode(params, false),
            ([], b'm') => self.sgr(params),
            ([], b'n') => match params.get(0).unwrap_or(0) {
                5 => self.reply(b"\x1b[0n"),
                6 => self.report_cursor(false),
                _ => {}
            },
            ([b'?'], b'n') if params.get(0) == Some(6) => self.report_cursor(true),
            ([], b'r') => self.set_scroll_region(params),
            ([], b's') if params.is_empty() => self.save_cursor(),
            ([], b'u') if params.is_empty() => self.restore_cursor(),
            ([b' '], b'q') => self.set_cursor_style(params.get(0).unwrap_or(0)),
            ([b'!'], b'p') => self.soft_reset(),
            _ => {}
        }
    }

    fn esc(&mut self, intermediates: &[u8], byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => {
                self.cursor.pending_wrap = false;
                self.index();
            }
            ([], b'E') => {
                self.cursor.col = 0;
                self.linefeed();
            }
            ([], b'H') => self.tabs[self.cursor.col] = true,
            ([], b'M') => {
                self.cursor.pending_wrap = false;
                self.reverse_index();
            }
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.application_keypad = true,
            ([], b'>') => self.modes.application_keypad = false,
            ([b'('], final_byte) => self.charsets[0] = charset(final_byte),
            ([b')'], final_byte) => self.charsets[1] = charset(final_byte),
            ([b'#'], b'8') => self.alignment_test(),
            _ => {}
        }
    }

    fn osc(&mut self, data: &[u8]) {
        let mut parts = data.splitn(2, |&b| b == b';');
        let command = parts.next().unwrap_or_default();
        let payload = parts.next().unwrap_or_default();
        match command {
            b"0" | b"2" => {
                let title: String = String::from_utf8_lossy(payload)
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_TITLE_CHARS)
                    .collect();
                self.title.clone_from(&title);
                self.push_event(TerminalEvent::TitleChanged(title));
            }
            b"7" => {
                if let Some((host, path)) = parse_file_url(payload) {
                    self.push_event(TerminalEvent::WorkingDirectory { host, path });
                }
            }
            b"52" => self.clipboard_request(payload),
            _ => {}
        }
    }
}

/// 화면 buffer API(XVIII.4.2).
impl Screen {
    /// scrollback 기록.
    pub fn history(&self) -> &Scrollback {
        &self.history
    }

    /// 주소를 쓸 수 있는 첫 줄의 절대 번호(대체 화면이면 화면 첫 줄).
    pub fn first_line(&self) -> u64 {
        if self.primary.is_some() {
            self.screen_top_line()
        } else {
            self.history.evicted()
        }
    }

    /// 화면 첫 줄의 절대 번호.
    pub fn screen_top_line(&self) -> u64 {
        self.history.evicted() + self.history.len() as u64
    }

    /// 마지막 줄의 절대 번호.
    pub fn last_line(&self) -> u64 {
        self.screen_top_line() + self.rows as u64 - 1
    }

    /// 절대 번호의 줄. 버려졌거나(또는 대체 화면에서 기록 줄) 범위 밖이면 `None`.
    pub fn buffer_line(&self, line: u64) -> Option<BufferLine<'_>> {
        if line < self.first_line() || line > self.last_line() {
            return None;
        }
        let top = self.screen_top_line();
        if line >= top {
            let row = usize::try_from(line - top).ok()?;
            return self.grid.get(row).map(BufferLine::Screen);
        }
        let index = usize::try_from(line - self.history.evicted()).ok()?;
        self.history.line(index).map(BufferLine::History)
    }

    /// 기록을 거슬러 보는 줄 수(0이면 현재 화면).
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    /// 보기 위치를 옮깁니다. 양수는 기록 쪽(위), 음수는 현재 화면 쪽(아래)입니다. 대체 화면에서는
    /// 움직이지 않습니다.
    pub fn scroll_display(&mut self, delta: isize) {
        if self.primary.is_some() {
            return;
        }
        let max = self.history.len();
        self.display_offset = self.display_offset.saturating_add_signed(delta).min(max);
    }

    /// 현재 화면으로 돌아갑니다(입력·붙여넣기 때).
    pub fn scroll_display_to_bottom(&mut self) {
        self.display_offset = 0;
    }

    /// 보기 위치 기준 `row`번째 표시 줄.
    pub fn display_line(&self, row: usize) -> Option<BufferLine<'_>> {
        if row >= self.rows {
            return None;
        }
        let line = self.screen_top_line() - self.display_offset as u64 + row as u64;
        self.buffer_line(line)
    }

    /// 선택 영역.
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// 선택 영역을 정합니다. 버려진 줄은 잘라 내고, 전부 버려진 줄이면 선택을 지웁니다.
    pub fn set_selection(&mut self, selection: Option<Selection>) {
        let first = self.first_line();
        self.selection = selection.and_then(|s| s.clamp_to(first));
    }

    /// 선택 방식을 적용해 넓힌 (앞, 뒤) 점. 블록 선택은 열 범위를 그대로 둡니다.
    pub fn selection_bounds(&self) -> Option<(BufferPoint, BufferPoint)> {
        let selection = self.selection?;
        let (mut start, mut end) = selection.ordered();
        start.line = start.line.max(self.first_line());
        end.line = end.line.min(self.last_line());
        if start.line > end.line {
            return None;
        }
        let max_col = self.cols - 1;
        start.col = start.col.min(max_col);
        end.col = end.col.min(max_col);
        match selection.mode() {
            SelectionMode::Simple | SelectionMode::Block => {
                if let Some(line) = self.buffer_line(start.line) {
                    if line.cell(start.col).width == CellWidth::WideSpacer && start.col > 0 {
                        start.col -= 1;
                    }
                }
            }
            SelectionMode::Word => {
                if let Some(line) = self.buffer_line(start.line) {
                    if is_word_char(line.char_at(start.col)) {
                        while start.col > 0 && is_word_char(line.char_at(start.col - 1)) {
                            start.col -= 1;
                        }
                    }
                }
                if let Some(line) = self.buffer_line(end.line) {
                    if is_word_char(line.char_at(end.col)) {
                        while end.col < max_col && is_word_char(line.char_at(end.col + 1)) {
                            end.col += 1;
                        }
                    }
                }
            }
            SelectionMode::Line => {
                start.col = 0;
                end.col = max_col;
            }
        }
        Some((start, end))
    }

    /// 선택 영역에서 `line`의 선택된 열 범위(그릴 때 씀).
    pub fn selected_columns(&self, line: u64) -> Option<std::ops::Range<usize>> {
        let selection = self.selection?;
        let (start, end) = self.selection_bounds()?;
        if line < start.line || line > end.line {
            return None;
        }
        if selection.mode() == SelectionMode::Block {
            let (a, b) = (start.col.min(end.col), start.col.max(end.col));
            return Some(a..b + 1);
        }
        let from = if line == start.line { start.col } else { 0 };
        let to = if line == end.line {
            end.col + 1
        } else {
            self.cols
        };
        (from < to).then_some(from..to)
    }

    /// 선택 영역 문자열(복사). 자동 줄바꿈 줄은 개행 없이 잇고 줄 끝 공백을 뺍니다.
    pub fn selected_text(&self) -> Option<CopiedText> {
        self.selected_text_capped(MAX_COPY_BYTES)
    }

    pub(crate) fn selected_text_capped(&self, cap: usize) -> Option<CopiedText> {
        let selection = self.selection?;
        let (start, end) = self.selection_bounds()?;
        let mut text = String::new();
        let block = selection.mode() == SelectionMode::Block;
        let (block_from, block_to) = (start.col.min(end.col), start.col.max(end.col) + 1);
        for number in start.line..=end.line {
            let Some(line) = self.buffer_line(number) else {
                continue;
            };
            let last = number == end.line;
            let piece = if block {
                line.text_with(block_from, block_to, true)
            } else {
                let from = if number == start.line { start.col } else { 0 };
                let to = if last { end.col + 1 } else { self.cols };
                // 다음 줄로 이어지는 줄은 공백도 내용입니다.
                line.text_with(from, to, last || !line.is_wrapped())
            };
            text.push_str(&piece);
            if !last && (block || !line.is_wrapped()) {
                text.push('\n');
            }
            if text.len() > cap {
                let mut cut = cap;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                return Some(CopiedText {
                    text,
                    truncated: true,
                });
            }
        }
        Some(CopiedText {
            text,
            truncated: false,
        })
    }

    /// OSC 52 요청(`대상;base64` 또는 `대상;?`).
    fn clipboard_request(&mut self, payload: &[u8]) {
        if self.clipboard != ClipboardAccess::AllowWrite {
            return;
        }
        let Some(data) = payload.splitn(2, |&b| b == b';').nth(1) else {
            return;
        };
        // 읽기 요청은 정책과 관계없이 거부합니다.
        if data == b"?" {
            return;
        }
        if let Some(text) = decode_base64(data).and_then(|bytes| String::from_utf8(bytes).ok()) {
            self.push_event(TerminalEvent::ClipboardWrite(text));
        }
    }
}

fn charset(final_byte: u8) -> Charset {
    match final_byte {
        b'0' => Charset::DecSpecialGraphics,
        _ => Charset::Ascii,
    }
}

fn color_component(value: u16) -> Option<u8> {
    u8::try_from(value).ok()
}

/// `38:5:n`, `38:2:r:g:b`, `38:2:cs:r:g:b`(group의 38 뒤 값).
fn colon_color(rest: &[u16]) -> Option<Color> {
    match rest {
        [5, index, ..] => color_component(*index).map(Color::Indexed),
        [2, _space, r, g, b, ..] => Some(Color::Rgb(
            color_component(*r)?,
            color_component(*g)?,
            color_component(*b)?,
        )),
        [2, r, g, b] => Some(Color::Rgb(
            color_component(*r)?,
            color_component(*g)?,
            color_component(*b)?,
        )),
        _ => None,
    }
}

/// `38;5;n`, `38;2;r;g;b`(다음 group들을 소비).
fn semicolon_color<'a>(groups: &mut impl Iterator<Item = &'a [u16]>) -> Option<Color> {
    match groups.next()?.first().copied()? {
        5 => color_component(*groups.next()?.first()?).map(Color::Indexed),
        2 => {
            let r = *groups.next()?.first()?;
            let g = *groups.next()?.first()?;
            let b = *groups.next()?.first()?;
            Some(Color::Rgb(
                color_component(r)?,
                color_component(g)?,
                color_component(b)?,
            ))
        }
        _ => None,
    }
}

/// OSC 7 `file://host/path`를 (host, 절대 경로)로 해석합니다. 경로는 percent-decoding 뒤 UTF-8이고
/// cwd 문법(절대 경로·NUL 없음·길이 상한)을 지켜야 합니다.
fn parse_file_url(payload: &[u8]) -> Option<(String, PathBuf)> {
    let rest = payload.strip_prefix(b"file://")?;
    let slash = rest.iter().position(|&b| b == b'/')?;
    let (host, path) = rest.split_at(slash);
    let mut decoded = Vec::with_capacity(path.len());
    let mut index = 0;
    while index < path.len() {
        let byte = path[index];
        if byte == b'%' {
            let hex = path.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(byte);
            index += 1;
        }
    }
    let path = PathBuf::from(String::from_utf8(decoded).ok()?);
    check_path_syntax(&path).ok()?;
    let host: String = String::from_utf8_lossy(host)
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect();
    Some((host, path))
}

/// parser 동작을 화면에 적용하는 연결 type(crate 밖에 [`Perform`] 구현을 드러내지 않음).
pub(crate) struct ScreenPerformer<'a>(pub(crate) &'a mut Screen);

impl Perform for ScreenPerformer<'_> {
    fn print(&mut self, c: char) {
        self.0.print(c);
    }

    fn execute(&mut self, byte: u8) {
        self.0.execute(byte);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignored: bool, action: u8) {
        if !ignored {
            self.0.csi(params, intermediates, action);
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignored: bool, byte: u8) {
        if !ignored {
            self.0.esc(intermediates, byte);
        }
    }

    fn osc_dispatch(&mut self, data: &[u8], _bell_terminated: bool) {
        self.0.osc(data);
    }
}
