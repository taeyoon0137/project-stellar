//! 화면 buffer·scrollback·선택·clipboard 검사 (XVIII.4.2).

use super::*;

fn size(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize::new(rows, cols).unwrap()
}

fn numbered(t: &mut Terminal, from: usize, to: usize) {
    for n in from..=to {
        t.feed(format!("\r\n{n}").as_bytes());
    }
}

fn history_texts(t: &Terminal) -> Vec<String> {
    let history = t.screen().history();
    (0..history.len())
        .map(|index| BufferLine::History(history.line(index).unwrap()).text())
        .collect()
}

fn display(t: &Terminal) -> Vec<String> {
    (0..t.screen().rows())
        .map(|row| t.screen().display_line(row).unwrap().text())
        .collect()
}

fn select(t: &mut Terminal, mode: SelectionMode, from: (u64, usize), to: (u64, usize)) -> String {
    let mut selection = Selection::new(mode, BufferPoint::new(from.0, from.1));
    selection.update(BufferPoint::new(to.0, to.1));
    t.screen_mut().set_selection(Some(selection));
    t.copy_selection().unwrap().text
}

#[test]
fn scrolled_lines_enter_scrollback_in_order() {
    let mut t = Terminal::new(size(3, 10));
    t.feed(b"1");
    numbered(&mut t, 2, 10);
    assert_eq!(history_texts(&t), ["1", "2", "3", "4", "5", "6", "7"]);
    assert_eq!(t.screen().lines(), ["8", "9", "10"]);
    let screen = t.screen();
    assert_eq!(screen.first_line(), 0);
    assert_eq!(screen.screen_top_line(), 7);
    assert_eq!(screen.last_line(), 9);
    assert_eq!(screen.buffer_line(0).unwrap().text(), "1");
    assert_eq!(screen.buffer_line(8).unwrap().text(), "9");
    assert!(screen.buffer_line(10).is_none());
    // SU도 기록합니다. ED 3은 기록을 지웁니다.
    t.feed(b"\x1b[2S");
    assert_eq!(t.screen().history().len(), 9);
    t.feed(b"\x1b[3J");
    assert!(t.screen().history().is_empty());
    assert_eq!(t.screen().first_line(), 9);
    assert_eq!(t.screen().lines(), ["10", "", ""]);
}

#[test]
fn region_and_alternate_screen_lines_are_not_recorded() {
    let mut t = Terminal::new(size(4, 10));
    t.feed(b"a\r\nb\r\nc\r\nd\x1b[2;3r\x1b[3;1H\n\n\n");
    assert!(t.screen().history().is_empty());
    assert_eq!(t.screen().lines(), ["a", "", "", "d"]);

    // 1049: cursor 저장 + 빈 대체 화면, 나오면 주 화면과 cursor 복원.
    let mut t = Terminal::new(size(3, 10));
    t.feed(b"one\r\ntwo\x1b[?1049h");
    assert!(t.screen().modes().alt_screen);
    assert_eq!(t.screen().lines(), ["", "", ""]);
    t.feed(b"\x1b[1;1Halt");
    numbered(&mut t, 1, 9);
    assert!(t.screen().history().is_empty());
    // 대체 화면에서는 기록이 보이지 않고 보기 위치도 움직이지 않습니다.
    t.screen_mut().scroll_display(5);
    assert_eq!(t.screen().display_offset(), 0);
    t.feed(b"\x1b[?1049l");
    assert!(!t.screen().modes().alt_screen);
    assert_eq!(t.screen().lines(), ["one", "two", ""]);
    let cursor = t.screen().cursor();
    assert_eq!((cursor.row, cursor.col), (1, 3));
    // 47·1047도 화면을 바꿉니다(cursor는 그대로).
    t.feed(b"\x1b[?47hX\x1b[?47l");
    assert_eq!(t.screen().lines(), ["one", "two", ""]);
    t.feed(b"\x1b[?1047h");
    assert!(t.screen().modes().alt_screen);
    t.feed(b"\x1b[?1047l");
    assert_eq!(t.screen().lines(), ["one", "two", ""]);
}

#[test]
fn viewport_scrolls_back_and_stays_anchored() {
    let mut t = Terminal::new(size(3, 10));
    t.feed(b"1");
    numbered(&mut t, 2, 10);
    t.screen_mut().scroll_display(2);
    assert_eq!(display(&t), ["6", "7", "8"]);
    // 거슬러 보는 중 출력이 쌓여도 같은 줄을 보입니다.
    numbered(&mut t, 11, 12);
    assert_eq!(t.screen().display_offset(), 4);
    assert_eq!(display(&t), ["6", "7", "8"]);
    // 범위 밖은 끝에서 멈춥니다.
    t.screen_mut().scroll_display(100);
    assert_eq!(display(&t), ["1", "2", "3"]);
    t.screen_mut().scroll_display(-1);
    assert_eq!(display(&t), ["2", "3", "4"]);
    // 붙여넣기는 현재 화면으로 돌아갑니다.
    t.paste("x").unwrap();
    assert_eq!(t.screen().display_offset(), 0);
    assert_eq!(display(&t), ["10", "11", "12"]);
    t.screen_mut().scroll_display(-5);
    assert_eq!(t.screen().display_offset(), 0);
}

#[test]
fn shrinking_moves_top_rows_into_scrollback() {
    let mut t = Terminal::new(size(4, 10));
    t.feed(b"a\r\nb\r\nc\r\nd");
    t.resize(size(2, 10));
    assert_eq!(history_texts(&t), ["a", "b"]);
    assert_eq!(t.screen().lines(), ["c", "d"]);
    // 대체 화면 중 축소: 보관한 주 화면도 맞추고 cursor 줄을 지킵니다.
    let mut t = Terminal::new(size(4, 10));
    t.feed(b"a\r\nb\r\nc\r\nd\x1b[?1049h");
    t.resize(size(2, 10));
    assert!(t.screen().history().len() == 2);
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.screen().lines(), ["c", "d"]);
    assert_eq!(t.screen().cursor().row, 1);
}

#[test]
fn selection_modes_produce_expected_text() {
    let mut t = Terminal::new(size(4, 20));
    t.feed(b"hello world foo\r\nsecond line\r\nopen (path/to/file) x");
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (0, 6), (0, 10)),
        "world"
    );
    // 끝점이 앞에 있어도 같습니다.
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (0, 10), (0, 6)),
        "world"
    );
    assert_eq!(select(&mut t, SelectionMode::Word, (0, 8), (0, 8)), "world");
    assert_eq!(
        select(&mut t, SelectionMode::Word, (2, 9), (2, 9)),
        "path/to/file"
    );
    // 단어가 아닌 칸은 그 칸만 선택합니다.
    assert_eq!(select(&mut t, SelectionMode::Word, (0, 5), (0, 5)), "");
    assert_eq!(
        select(&mut t, SelectionMode::Line, (0, 3), (1, 0)),
        "hello world foo\nsecond line"
    );
    assert_eq!(
        select(&mut t, SelectionMode::Block, (0, 0), (1, 3)),
        "hell\nseco"
    );
    // 여러 줄 흐르는 선택은 줄 끝 공백을 빼고 개행으로 잇습니다.
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (0, 12), (1, 5)),
        "foo\nsecond"
    );
    // 그릴 때 쓰는 줄별 열 범위.
    assert_eq!(t.screen().selected_columns(0), Some(12..20));
    assert_eq!(t.screen().selected_columns(1), Some(0..6));
    assert_eq!(t.screen().selected_columns(2), None);
    select(&mut t, SelectionMode::Block, (0, 4), (1, 1));
    assert_eq!(t.screen().selected_columns(1), Some(1..5));
}

#[test]
fn wrapped_wide_and_history_spanning_selection() {
    // 자동 줄바꿈으로 이어진 줄은 개행 없이 잇고, 줄바꿈 지점의 공백을 지킵니다.
    let mut t = Terminal::new(size(3, 5));
    t.feed(b"abcd efgh");
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (0, 0), (1, 3)),
        "abcd efgh"
    );
    let mut t = Terminal::new(size(3, 5));
    t.feed(b"abcde fghi");
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (0, 0), (1, 4)),
        "abcde fghi"
    );
    // 넓은 문자 뒤 칸에서 시작해도 문자 전체가 들어갑니다.
    let mut t = Terminal::new(size(2, 10));
    t.feed("a한b".as_bytes());
    assert_eq!(select(&mut t, SelectionMode::Simple, (0, 2), (0, 3)), "한b");
    assert_eq!(select(&mut t, SelectionMode::Word, (0, 2), (0, 2)), "a한b");
    // 기록과 화면에 걸친 선택.
    let mut t = Terminal::new(size(2, 10));
    t.feed(b"1");
    numbered(&mut t, 2, 5);
    assert_eq!(t.screen().screen_top_line(), 3);
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (1, 0), (4, 0)),
        "2\n3\n4\n5"
    );
}

#[test]
fn selection_follows_eviction_and_is_cleared_on_screen_switch() {
    let limits = ScrollbackLimits::new(3, 1024 * 1024).unwrap();
    let config = TerminalConfig {
        scrollback: limits,
        ..TerminalConfig::default()
    };
    let mut t = Terminal::with_config(size(2, 10), config);
    t.feed(b"0");
    numbered(&mut t, 1, 4);
    // 기록: 0,1,2 / 화면: 3,4.
    assert_eq!(t.screen().first_line(), 0);
    assert_eq!(
        select(&mut t, SelectionMode::Simple, (1, 0), (3, 0)),
        "1\n2\n3"
    );
    numbered(&mut t, 5, 5);
    // 0이 버려져도 선택은 같은 줄(1~3)입니다.
    assert_eq!(t.screen().first_line(), 1);
    assert_eq!(t.copy_selection().unwrap().text, "1\n2\n3");
    // 선택 시작 줄이 버려지면 잘라 냅니다.
    numbered(&mut t, 6, 6);
    assert_eq!(t.copy_selection().unwrap().text, "2\n3");
    // 선택 전체가 버려지면 선택이 사라집니다.
    numbered(&mut t, 7, 8);
    assert!(t.screen().selection().is_none());
    assert!(t.copy_selection().is_none());
    // 버려진 줄로 선택을 만들면 잘라 내거나 지웁니다.
    let mut gone = Selection::new(SelectionMode::Simple, BufferPoint::new(0, 0));
    gone.update(BufferPoint::new(1, 0));
    t.screen_mut().set_selection(Some(gone));
    assert!(t.screen().selection().is_none());
    // 대체 화면 전환·크기 변경은 선택을 지웁니다.
    select(&mut t, SelectionMode::Line, (7, 0), (7, 0));
    t.feed(b"\x1b[?1049h");
    assert!(t.screen().selection().is_none());
    t.feed(b"\x1b[?1049l");
    select(&mut t, SelectionMode::Line, (7, 0), (7, 0));
    t.resize(size(3, 10));
    assert!(t.screen().selection().is_none());
}

#[test]
fn copy_is_bounded_and_marks_truncation() {
    let mut t = Terminal::new(size(2, 10));
    t.feed("가나다라마\r\nabc".as_bytes());
    let mut selection = Selection::new(SelectionMode::Line, BufferPoint::new(0, 0));
    selection.update(BufferPoint::new(1, 0));
    t.screen_mut().set_selection(Some(selection));
    let full = t.screen().selected_text_capped(1024).unwrap();
    assert_eq!(full.text, "가나다라마\nabc");
    assert!(!full.truncated);
    // 한글 문자 경계에서 자릅니다.
    let cut = t.screen().selected_text_capped(7).unwrap();
    assert_eq!(cut.text, "가나");
    assert!(cut.truncated);
}

#[test]
fn paste_follows_bracketed_mode() {
    let mut t = Terminal::new(size(2, 10));
    let plain = t.paste("ls\nrm x").unwrap();
    assert_eq!(plain.bytes(), b"ls\rrm x");
    assert_eq!(plain.line_count(), 2);
    t.feed(b"\x1b[?2004h");
    let bracketed = t.paste("ls\nrm x\x1b[201~").unwrap();
    assert_eq!(bracketed.bytes(), b"\x1b[200~ls\rrm x[201~\x1b[201~");
    assert!(bracketed.is_bracketed());
}

#[test]
fn osc52_clipboard_policy() {
    let write = b"\x1b]52;c;aGVsbG8=\x07";
    // 기본은 거부: 알림 없음.
    let mut t = Terminal::new(size(2, 10));
    t.feed(write);
    assert!(t.take_events().is_empty());
    // 쓰기 허용: UI에 알림. 읽기 요청·잘못된 base64·UTF-8 아님은 무시.
    let config = TerminalConfig {
        clipboard: ClipboardAccess::AllowWrite,
        ..TerminalConfig::default()
    };
    let mut t = Terminal::with_config(size(2, 10), config);
    t.feed(write);
    t.feed(b"\x1b]52;c;?\x07\x1b]52;c;@@@@\x07\x1b]52;c;/w==\x07\x1b]52;nodata\x07");
    assert_eq!(
        t.take_events(),
        [TerminalEvent::ClipboardWrite("hello".to_owned())]
    );
    assert!(t.take_replies().is_empty());
}

/// 100만 줄 출력을 emulator로 넣어도 scrollback 줄 수·메모리 상한을 지킵니다(XVIII.4.2 완료 조건).
#[test]
fn one_million_output_lines_keep_scrollback_cap() {
    let cap = 32 * 1024 * 1024;
    let config = TerminalConfig {
        scrollback: ScrollbackLimits::new(scrollback::MAX_SCROLLBACK_LINES, cap).unwrap(),
        ..TerminalConfig::default()
    };
    let mut t = Terminal::with_config(size(24, 80), config);
    let mut chunk = String::with_capacity(64 * 1024);
    let mut peak = 0;
    for n in 0..1_000_000u32 {
        chunk.push_str(&format!("line {n:07} ........................\r\n"));
        if chunk.len() > 60 * 1024 {
            t.feed(chunk.as_bytes());
            chunk.clear();
            peak = peak.max(t.screen().history().memory_bytes());
        }
    }
    t.feed(chunk.as_bytes());
    let history = t.screen().history();
    peak = peak.max(history.memory_bytes());
    assert!(peak <= cap, "peak {peak} > cap {cap}");
    // 화면에 남은 23줄 + 빈 cursor 줄을 뺀 나머지는 기록에 들어갔거나 버려졌습니다.
    assert_eq!(history.evicted() + history.len() as u64, 1_000_000 - 23);
    assert!(history.len() > 10_000, "{}", history.len());
    let newest = BufferLine::History(history.line(history.len() - 1).unwrap()).text();
    assert_eq!(newest, "line 0999976 ........................");
    assert_eq!(
        t.screen().lines()[22],
        "line 0999999 ........................"
    );

    // 기본 설정은 10,000줄에서 멈춥니다.
    let mut t = Terminal::new(size(24, 80));
    for _ in 0..20 {
        t.feed("x\r\n".repeat(1_000).as_bytes());
    }
    assert_eq!(
        t.screen().history().len(),
        scrollback::DEFAULT_SCROLLBACK_LINES
    );
}
