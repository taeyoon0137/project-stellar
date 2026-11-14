//! VT sequence corpus (XVIII.4.1): cursor·색·지우기 기본 fixture와 견고성 검사.
//!
//! 모든 fixture는 입력을 한 번에 넣은 결과와 한 byte씩 나눠 넣은 결과가 같은지도 확인합니다
//! ([`run`]).

use std::path::PathBuf;

use super::screen::{MAX_EVENTS, MAX_REPLY_BYTES};
use super::*;

fn size(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize::new(rows, cols).unwrap()
}

fn assert_same_screen(a: &Screen, b: &Screen, input: &[u8]) {
    assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
    for row in 0..a.rows() {
        assert_eq!(a.row(row), b.row(row), "row {row}, input {input:?}");
    }
    assert_eq!(a.cursor(), b.cursor(), "{input:?}");
    assert_eq!(a.pen(), b.pen(), "{input:?}");
    assert_eq!(a.modes(), b.modes(), "{input:?}");
    assert_eq!(a.title(), b.title(), "{input:?}");
    assert_eq!(a.scroll_region(), b.scroll_region(), "{input:?}");
}

/// 입력을 한 번에, 그리고 한 byte씩 넣어 같은 화면인지 확인하고 한 번에 넣은 쪽을 돌려줍니다.
fn run(rows: u16, cols: u16, input: &[u8]) -> Terminal {
    let mut whole = Terminal::new(size(rows, cols));
    whole.feed(input);
    let mut split = Terminal::new(size(rows, cols));
    for byte in input {
        split.feed(std::slice::from_ref(byte));
    }
    assert_same_screen(whole.screen(), split.screen(), input);
    assert_eq!(whole.take_replies(), split.take_replies());
    assert_eq!(whole.take_events(), split.take_events());
    // 확인용으로 꺼낸 응답·알림을 다시 만들기 위해 한 번 더 실행합니다.
    let mut fresh = Terminal::new(size(rows, cols));
    fresh.feed(input);
    fresh
}

fn lines(t: &Terminal) -> Vec<String> {
    t.screen().lines()
}

fn at(t: &Terminal) -> (usize, usize) {
    let cursor = t.screen().cursor();
    (cursor.row, cursor.col)
}

fn style(t: &Terminal, row: usize, col: usize) -> Style {
    t.screen().cell(row, col).unwrap().style
}

#[test]
fn text_crlf_autowrap_and_scrolling() {
    let t = run(3, 5, b"abc\r\ndef");
    assert_eq!(lines(&t), ["abc", "def", ""]);
    assert_eq!(at(&t), (1, 3));

    // 마지막 열에 쓰면 cursor는 그 열에 머물고(지연 줄바꿈), 다음 문자에서 줄을 바꿉니다.
    let t = run(3, 5, b"abcde");
    assert_eq!(at(&t), (0, 4));
    let t = run(3, 5, b"abcdef");
    assert_eq!(lines(&t), ["abcde", "f", ""]);
    assert!(t.screen().row(0).unwrap().is_wrapped());
    assert!(!t.screen().row(1).unwrap().is_wrapped());
    assert_eq!(at(&t), (1, 1));
    // 지연 줄바꿈 중 CR LF는 빈 줄을 만들지 않습니다(`xenl`).
    let t = run(3, 5, b"abcde\r\nX");
    assert_eq!(lines(&t), ["abcde", "X", ""]);
    assert!(!t.screen().row(0).unwrap().is_wrapped());

    // DECAWM 끔: 마지막 열을 덮어씁니다.
    let t = run(2, 5, b"\x1b[?7labcdefg");
    assert_eq!(lines(&t), ["abcdg", ""]);
    assert_eq!(at(&t), (0, 4));

    // 맨 아래에서 LF는 화면을 올립니다.
    let t = run(3, 5, b"1\r\n2\r\n3\r\n4");
    assert_eq!(lines(&t), ["2", "3", "4"]);
    // LNM이면 LF가 CR도 합니다.
    let t = run(3, 5, b"\x1b[20hab\ncd");
    assert_eq!(lines(&t), ["ab", "cd", ""]);
}

#[test]
fn cursor_positioning_and_relative_moves() {
    let t = run(5, 10, b"\x1b[3;4HX");
    assert_eq!(t.screen().cell(2, 3).unwrap().ch, 'X');
    assert_eq!(at(&t), (2, 4));
    // 화면 밖으로 가는 이동은 가장자리에서 멈춥니다.
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[10A")), (0, 3));
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[99B")), (4, 3));
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[99C")), (2, 9));
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[99D")), (2, 0));
    assert_eq!(at(&run(5, 10, b"\x1b[99;99H")), (4, 9));
    // 매개변수 0·생략은 1입니다.
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[0A\x1b[C")), (1, 4));
    assert_eq!(at(&run(5, 10, b"\x1b[;5H")), (0, 4));
    assert_eq!(at(&run(5, 10, b"\x1b[3;4H\x1b[H")), (0, 0));
    // CHA·HPA·HPR·VPA·VPR·CNL·CPL.
    assert_eq!(at(&run(5, 10, b"\x1b[2;2H\x1b[5G")), (1, 4));
    assert_eq!(at(&run(5, 10, b"\x1b[2;2H\x1b[7`")), (1, 6));
    assert_eq!(at(&run(5, 10, b"\x1b[2;2H\x1b[3a")), (1, 4));
    assert_eq!(at(&run(5, 10, b"\x1b[2;2H\x1b[4d")), (3, 1));
    assert_eq!(at(&run(5, 10, b"\x1b[2;2H\x1b[2e")), (3, 1));
    assert_eq!(at(&run(5, 10, b"\x1b[2;5H\x1b[2E")), (3, 0));
    assert_eq!(at(&run(5, 10, b"\x1b[4;5H\x1b[2F")), (1, 0));
    // BS, HT, CHT, CBT, HTS, TBC.
    assert_eq!(lines(&run(2, 10, b"ab\x08X")), ["aX", ""]);
    let t = run(2, 20, b"a\tb");
    assert_eq!(t.screen().cell(0, 8).unwrap().ch, 'b');
    assert_eq!(at(&run(2, 20, b"\x1b[2I")), (0, 16));
    assert_eq!(at(&run(2, 20, b"\x1b[1;19H\x1b[Z")), (0, 16));
    assert_eq!(at(&run(2, 20, b"\x1b[3g\t")), (0, 19));
    assert_eq!(at(&run(2, 20, b"\x1b[3g\x1b[1;4H\x1bH\x1b[1;1H\t")), (0, 3));
    assert_eq!(at(&run(2, 20, b"\x1b[1;9H\x1b[g\x1b[1;1H\t")), (0, 16));
}

#[test]
fn save_restore_and_origin_mode() {
    // DECSC/DECRC는 위치와 pen을 되돌립니다.
    let t = run(5, 10, b"\x1b[2;3H\x1b[31m\x1b7\x1b[5;5H\x1b[0m\x1b8X");
    assert_eq!(t.screen().cell(1, 2).unwrap().ch, 'X');
    assert_eq!(style(&t, 1, 2).fg, Color::Indexed(1));
    // SCOSC/SCORC(`CSI s`·`CSI u`)도 같습니다.
    let t = run(5, 10, b"\x1b[3;3H\x1b[s\x1b[1;1H\x1b[uY");
    assert_eq!(t.screen().cell(2, 2).unwrap().ch, 'Y');
    // 저장 없이 복원하면 원점·기본 pen.
    assert_eq!(at(&run(5, 10, b"\x1b[3;3H\x1b8")), (0, 0));

    // DECOM: 좌표가 scroll region 기준이고 region 밖으로 나가지 않습니다.
    let t = run(5, 10, b"\x1b[2;4r\x1b[?6h\x1b[1;1HX");
    assert_eq!(t.screen().cell(1, 0).unwrap().ch, 'X');
    assert_eq!(at(&run(5, 10, b"\x1b[2;4r\x1b[?6h\x1b[9;1H")), (3, 0));
    // CPR은 origin 기준 좌표를 보고합니다.
    let mut t = run(5, 10, b"\x1b[2;4r\x1b[?6h\x1b[2;3H\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[2;3R");
    let mut t = run(5, 10, b"\x1b[3;7H\x1b[6n\x1b[?6n");
    assert_eq!(t.take_replies(), b"\x1b[3;7R\x1b[?3;7R");
}

#[test]
fn sgr_colors_and_attributes() {
    let t = run(2, 20, b"\x1b[31mR\x1b[42mG\x1b[0mN\x1b[91;102mB");
    assert_eq!(style(&t, 0, 0).fg, Color::Indexed(1));
    assert_eq!(style(&t, 0, 0).bg, Color::Default);
    assert_eq!(
        (style(&t, 0, 1).fg, style(&t, 0, 1).bg),
        (Color::Indexed(1), Color::Indexed(2))
    );
    assert_eq!(style(&t, 0, 2), Style::default());
    assert_eq!(
        (style(&t, 0, 3).fg, style(&t, 0, 3).bg),
        (Color::Indexed(9), Color::Indexed(10))
    );

    // 256색과 24-bit 색: `;` 형식과 `:` 형식(색 공간 칸 있음·없음).
    let t = run(
        2,
        20,
        b"\x1b[38;5;196mA\x1b[48:5:21mB\x1b[0;38;2;10;20;30mC\x1b[38:2::1:2:3mD\x1b[38:2:4:5:6mE\x1b[48;2;7;8;9mF",
    );
    assert_eq!(style(&t, 0, 0).fg, Color::Indexed(196));
    assert_eq!(style(&t, 0, 1).bg, Color::Indexed(21));
    assert_eq!(style(&t, 0, 2).fg, Color::Rgb(10, 20, 30));
    assert_eq!(style(&t, 0, 2).bg, Color::Default);
    assert_eq!(style(&t, 0, 3).fg, Color::Rgb(1, 2, 3));
    assert_eq!(style(&t, 0, 4).fg, Color::Rgb(4, 5, 6));
    assert_eq!(style(&t, 0, 5).bg, Color::Rgb(7, 8, 9));

    // 속성 켜기·끄기.
    let t = run(2, 20, b"\x1b[1;2;3;4;5;7;8;9mA\x1b[22;23;24;25;27;28;29mB");
    let a = style(&t, 0, 0).attrs;
    for attr in [
        Attrs::BOLD,
        Attrs::DIM,
        Attrs::ITALIC,
        Attrs::UNDERLINE,
        Attrs::BLINK,
        Attrs::INVERSE,
        Attrs::HIDDEN,
        Attrs::STRIKE,
    ] {
        assert!(a.contains(attr), "{attr:?}");
    }
    assert!(style(&t, 0, 1).attrs.is_empty());
    let t = run(2, 20, b"\x1b[4mA\x1b[4:0mB\x1b[4:2mC\x1b[21mD\x1b[24mE");
    assert!(style(&t, 0, 0).attrs.contains(Attrs::UNDERLINE));
    assert!(style(&t, 0, 1).attrs.is_empty());
    assert!(style(&t, 0, 2).attrs.contains(Attrs::DOUBLE_UNDERLINE));
    assert!(style(&t, 0, 3).attrs.contains(Attrs::DOUBLE_UNDERLINE));
    assert!(style(&t, 0, 4).attrs.is_empty());

    // 39·49 기본색, 범위 밖·잘린 색은 무시하고 뒤 값은 계속 처리합니다.
    let t = run(
        2,
        20,
        b"\x1b[31;41m\x1b[39;49mA\x1b[38;5;300mB\x1b[38;5m\x1b[1mC",
    );
    assert_eq!(style(&t, 0, 0), Style::default());
    assert_eq!(style(&t, 0, 1).fg, Color::Default);
    assert!(style(&t, 0, 2).attrs.contains(Attrs::BOLD));
    // `CSI m`은 0과 같습니다.
    assert_eq!(
        run(2, 5, b"\x1b[1;31m\x1b[m").screen().pen(),
        Style::default()
    );
}

/// 3×5 화면에 `abcde`/`fghij`/`klmno`를 채우고 cursor를 (1, 2)에 둔 뒤 `tail`을 실행합니다.
fn erase_fixture(tail: &[u8]) -> Terminal {
    let mut input = b"abcde\r\nfghij\r\nklmno\x1b[2;3H".to_vec();
    input.extend_from_slice(tail);
    run(3, 5, &input)
}

#[test]
fn erase_in_line_display_and_characters() {
    assert_eq!(lines(&erase_fixture(b"\x1b[K")), ["abcde", "fg", "klmno"]);
    assert_eq!(lines(&erase_fixture(b"\x1b[0K")), ["abcde", "fg", "klmno"]);
    assert_eq!(
        lines(&erase_fixture(b"\x1b[1K")),
        ["abcde", "   ij", "klmno"]
    );
    assert_eq!(lines(&erase_fixture(b"\x1b[2K")), ["abcde", "", "klmno"]);
    assert_eq!(lines(&erase_fixture(b"\x1b[J")), ["abcde", "fg", ""]);
    assert_eq!(lines(&erase_fixture(b"\x1b[1J")), ["", "   ij", "klmno"]);
    let t = erase_fixture(b"\x1b[2J");
    assert_eq!(lines(&t), ["", "", ""]);
    // ED 2는 cursor를 옮기지 않습니다.
    assert_eq!(at(&t), (1, 2));
    // ECH는 cursor 위치부터 지우고 cursor를 옮기지 않습니다.
    let t = erase_fixture(b"\x1b[2X");
    assert_eq!(lines(&t), ["abcde", "fg  j", "klmno"]);
    assert_eq!(at(&t), (1, 2));
    assert_eq!(lines(&erase_fixture(b"\x1b[99X")), ["abcde", "fg", "klmno"]);
    // 지운 칸은 현재 배경색을 씁니다(bce). 전경·속성은 기본값입니다.
    let t = erase_fixture(b"\x1b[1;31;44m\x1b[K");
    let erased = t.screen().cell(1, 3).unwrap();
    assert_eq!(erased.ch, ' ');
    assert_eq!(erased.style.bg, Color::Indexed(4));
    assert_eq!(erased.style.fg, Color::Default);
    assert!(erased.style.attrs.is_empty());
    let t = erase_fixture(b"\x1b[45m\x1b[2J");
    assert_eq!(style(&t, 0, 0).bg, Color::Indexed(5));
    assert_eq!(style(&t, 2, 4).bg, Color::Indexed(5));
}

#[test]
fn insert_delete_scroll_and_regions() {
    assert_eq!(lines(&run(1, 5, b"abcde\x1b[1;2H\x1b[2@")), ["a  bc"]);
    assert_eq!(lines(&run(1, 5, b"abcde\x1b[1;2H\x1b[2P")), ["ade"]);
    assert_eq!(lines(&run(1, 5, b"abcde\x1b[1;2H\x1b[99P")), ["a"]);
    let five = b"1\r\n2\r\n3\r\n4\r\n5";
    let with = |tail: &[u8]| {
        let mut input = five.to_vec();
        input.extend_from_slice(tail);
        lines(&run(5, 5, &input))
    };
    assert_eq!(with(b"\x1b[2;1H\x1b[L"), ["1", "", "2", "3", "4"]);
    assert_eq!(with(b"\x1b[2;1H\x1b[2M"), ["1", "4", "5", "", ""]);
    assert_eq!(with(b"\x1b[2S"), ["3", "4", "5", "", ""]);
    assert_eq!(with(b"\x1b[T"), ["", "1", "2", "3", "4"]);
    // scroll region 안에서만 줄이 움직입니다.
    assert_eq!(with(b"\x1b[2;4r\x1b[4;1H\n"), ["1", "3", "4", "", "5"]);
    assert_eq!(with(b"\x1b[2;4r\x1b[2;1H\x1bM"), ["1", "", "2", "3", "5"]);
    assert_eq!(with(b"\x1b[2;4r\x1b[S"), ["1", "3", "4", "", "5"]);
    assert_eq!(with(b"\x1b[2;4r\x1b[3;1H\x1b[L"), ["1", "2", "", "3", "5"]);
    // region 밖의 IL은 무시합니다.
    assert_eq!(with(b"\x1b[2;4r\x1b[5;1H\x1b[L"), ["1", "2", "3", "4", "5"]);
    // 잘못된 region(top >= bottom)은 무시합니다.
    assert_eq!(run(5, 5, b"\x1b[4;2r").screen().scroll_region(), (0, 4));
    assert_eq!(run(5, 5, b"\x1b[2;4r").screen().scroll_region(), (1, 3));
    // IRM, REP, DECALN.
    assert_eq!(lines(&run(1, 6, b"abc\x1b[1;1H\x1b[4hX")), ["Xabc"]);
    assert_eq!(lines(&run(1, 6, b"abc\x1b[1;1H\x1b[4h\x1b[4lX")), ["Xbc"]);
    assert_eq!(lines(&run(1, 6, b"a\x1b[3b")), ["aaaa"]);
    assert_eq!(lines(&run(2, 3, b"\x1b#8")), ["EEE", "EEE"]);
}

#[test]
fn wide_and_combining_characters() {
    let t = run(2, 5, "한글".as_bytes());
    let screen = t.screen();
    assert_eq!(screen.cell(0, 0).unwrap().ch, '한');
    assert_eq!(screen.cell(0, 0).unwrap().width, CellWidth::Wide);
    assert_eq!(screen.cell(0, 1).unwrap().width, CellWidth::WideSpacer);
    assert_eq!(screen.cell(0, 2).unwrap().ch, '글');
    assert_eq!(lines(&t), ["한글", ""]);
    assert_eq!(at(&t), (0, 4));
    // 마지막 열에 들어가지 않는 넓은 문자는 다음 줄로 갑니다.
    let t = run(2, 5, "abcd한".as_bytes());
    assert_eq!(lines(&t), ["abcd", "한"]);
    assert!(t.screen().row(0).unwrap().is_wrapped());
    // 넓은 문자의 반쪽을 덮어쓰면 다른 반쪽은 빈 칸이 됩니다.
    let t = run(1, 5, "한\x1b[1;2HX".as_bytes());
    assert_eq!(t.screen().cell(0, 0).unwrap().ch, ' ');
    assert_eq!(t.screen().cell(0, 0).unwrap().width, CellWidth::Narrow);
    assert_eq!(lines(&t), [" X"]);
    let t = run(1, 5, "한\x1b[1;1HX".as_bytes());
    assert_eq!(lines(&t), ["X"]);
    assert_eq!(t.screen().cell(0, 1).unwrap().width, CellWidth::Narrow);
    // 넓은 문자 중간에서 지우기를 시작해도 반쪽이 남지 않습니다.
    let t = run(1, 5, "a한b\x1b[1;3H\x1b[K".as_bytes());
    assert_eq!(lines(&t), ["a"]);
    // NFD 한글(macOS 파일 이름): 초성 + 중성 + 종성이 한 칸(2열)에 모입니다.
    let nfd = "\u{1112}\u{1161}\u{11AB}";
    let t = run(1, 6, nfd.as_bytes());
    assert_eq!(lines(&t), [nfd]);
    assert_eq!(at(&t), (0, 2));
    assert_eq!(
        t.screen().row(0).unwrap().combining(0).collect::<String>(),
        "\u{1161}\u{11AB}"
    );
    // 결합 악센트와 emoji.
    let t = run(1, 6, "e\u{0301}x😀".as_bytes());
    assert_eq!(lines(&t), ["e\u{0301}x😀"]);
    assert_eq!(at(&t), (0, 4));
    // 덮어쓰면 결합 문자도 지워집니다.
    let t = run(1, 6, "e\u{0301}\x1b[1;1Hf".as_bytes());
    assert_eq!(lines(&t), ["f"]);
}

#[test]
fn dec_special_graphics_line_drawing() {
    assert_eq!(lines(&run(1, 10, b"\x1b(0lqk\x1b(Bq")), ["┌─┐q"]);
    assert_eq!(lines(&run(1, 10, b"\x1b)0\x0eqx\x0fq")), ["─│q"]);
    // DECSC/DECRC는 문자 집합도 되돌립니다.
    assert_eq!(lines(&run(1, 10, b"\x1b(0\x1b7\x1b(B\x1b8q")), ["─"]);
}

#[test]
fn replies_events_and_bounds() {
    let mut t = run(2, 5, b"\x1b[c\x1b[0c\x1b[>c\x1b[5n");
    assert_eq!(
        t.take_replies(),
        b"\x1b[?62;22c\x1b[?62;22c\x1b[>1;10;0c\x1b[0n"
    );
    assert!(t.take_replies().is_empty());

    let mut t = run(2, 5, b"\x07\x1b]0;one\x07\x1b]2;t\x01wo\x1b\\");
    assert_eq!(
        t.take_events(),
        [
            TerminalEvent::Bell,
            TerminalEvent::TitleChanged("one".to_owned()),
            TerminalEvent::TitleChanged("two".to_owned()),
        ]
    );
    assert_eq!(t.screen().title(), "two");

    let mut t = run(
        2,
        5,
        b"\x1b]7;file://mac.local/Users/me/my%20dir\x07\x1b]7;file:///tmp\x07",
    );
    assert_eq!(
        t.take_events(),
        [
            TerminalEvent::WorkingDirectory {
                host: "mac.local".to_owned(),
                path: PathBuf::from("/Users/me/my dir"),
            },
            TerminalEvent::WorkingDirectory {
                host: String::new(),
                path: PathBuf::from("/tmp"),
            },
        ]
    );
    // 형식이 틀린 OSC 7은 버립니다(scheme, 경로 없음, 잘못된 escape, NUL).
    let mut t = run(
        2,
        5,
        b"\x1b]7;http://h/x\x07\x1b]7;file://host\x07\x1b]7;file://h/%zz\x07\x1b]7;file://h/a%00b\x07",
    );
    assert!(t.take_events().is_empty());

    // 응답과 알림은 상한을 넘지 않습니다.
    let mut t = Terminal::new(size(24, 80));
    for _ in 0..2000 {
        t.feed(b"\x1b[6n\x07");
    }
    assert!(t.take_replies().len() <= MAX_REPLY_BYTES);
    assert_eq!(t.take_events().len(), MAX_EVENTS);
    // 제목은 제어 문자를 빼고 길이를 자릅니다.
    let long = format!("\x1b]0;{}\x07", "가".repeat(1000));
    let t = run(2, 5, long.as_bytes());
    assert_eq!(t.screen().title().chars().count(), screen::MAX_TITLE_CHARS);
}

#[test]
fn modes_are_tracked() {
    let t = run(
        2,
        5,
        b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?25l\x1b[5 q\x1b[?5h",
    );
    let modes = t.screen().modes();
    assert!(modes.application_cursor_keys);
    assert!(modes.application_keypad);
    assert!(modes.bracketed_paste);
    assert_eq!(modes.mouse_tracking, MouseTracking::Drag);
    assert!(modes.mouse_sgr && modes.focus_events && modes.reverse_video);
    let cursor = t.screen().cursor();
    assert!(!cursor.visible);
    assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Bar, true));
    let t = run(2, 5, b"\x1b[?1000h\x1b[?1002l\x1b[?1000l\x1b>\x1b[2 q");
    assert_eq!(t.screen().modes().mouse_tracking, MouseTracking::Off);
    assert!(!t.screen().modes().application_keypad);
    assert_eq!(
        (t.screen().cursor().shape, t.screen().cursor().blinking),
        (CursorShape::Block, false)
    );
}

#[test]
fn full_and_soft_reset() {
    let t = run(3, 5, b"abc\x1b[31m\x1b[2;3r\x1b[?25l\x1b]0;x\x07\x1bc");
    assert_eq!(lines(&t), ["", "", ""]);
    assert_eq!(at(&t), (0, 0));
    assert_eq!(t.screen().pen(), Style::default());
    assert_eq!(t.screen().scroll_region(), (0, 2));
    assert!(t.screen().cursor().visible);
    assert_eq!(t.screen().title(), "");
    // DECSTR은 화면 내용을 지우지 않습니다.
    let t = run(3, 5, b"abc\x1b[1;31m\x1b[?7l\x1b[4h\x1b[2;3r\x1b[!p");
    assert_eq!(lines(&t), ["abc", "", ""]);
    assert_eq!(t.screen().pen(), Style::default());
    assert!(t.screen().modes().autowrap);
    assert!(!t.screen().modes().insert);
    assert_eq!(t.screen().scroll_region(), (0, 2));
}

#[test]
fn resize_keeps_cursor_row_and_cuts_wide_cells() {
    let mut t = run(5, 5, b"1\r\n2\r\n3\r\n4\r\n5");
    t.resize(size(3, 5));
    assert_eq!(lines(&t), ["3", "4", "5"]);
    assert_eq!(at(&t), (2, 1));
    t.resize(size(4, 8));
    assert_eq!(lines(&t), ["3", "4", "5", ""]);
    assert_eq!(t.screen().scroll_region(), (0, 3));
    let mut t = run(1, 4, "a한".as_bytes());
    t.resize(size(1, 2));
    assert_eq!(lines(&t), ["a"]);
    assert_eq!(t.screen().cell(0, 1).unwrap().width, CellWidth::Narrow);
    assert_eq!(at(&t), (0, 1));
}

/// 결정론적 xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_input_never_panics_and_stays_bounded() {
    // sequence가 자주 생기도록 ESC·`[`·숫자·`;`·최종 byte 쪽으로 치우친 입력.
    const ALPHABET: &[u8] = b"\x1b\x1b\x1b[[];:?>0123456789mHJKABCDPLMr@Xh l\x07\x08\t\n\r\x0e\x0fq(0#8\xed\x95\x9c\x80\xff";
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let mut t = Terminal::new(size(7, 13));
    let mut input = Vec::with_capacity(512 * 1024);
    for _ in 0..512 * 1024 {
        let pick = rng.next();
        let byte = if pick.is_multiple_of(4) {
            (pick >> 8) as u8
        } else {
            ALPHABET[(pick >> 8) as usize % ALPHABET.len()]
        };
        input.push(byte);
    }
    let mut offset = 0;
    while offset < input.len() {
        let chunk = 1 + (rng.next() % 97) as usize;
        let end = (offset + chunk).min(input.len());
        t.feed(&input[offset..end]);
        offset = end;
        let cursor = t.screen().cursor();
        assert!(cursor.row < t.screen().rows() && cursor.col < t.screen().cols());
        if rng.next().is_multiple_of(256) {
            let rows = 1 + (rng.next() % 30) as u16;
            let cols = 1 + (rng.next() % 30) as u16;
            t.resize(size(rows, cols));
        }
        assert!(t.take_replies().len() <= MAX_REPLY_BYTES);
        assert!(t.take_events().len() <= MAX_EVENTS);
    }
    let screen = t.screen();
    for row in 0..screen.rows() {
        let line = screen.row(row).unwrap();
        assert_eq!(line.cells().len(), screen.cols());
        // 넓은 문자의 앞·뒤 칸은 항상 짝을 이룹니다.
        for (col, cell) in line.cells().iter().enumerate() {
            match cell.width {
                CellWidth::Wide => {
                    assert_eq!(
                        line.cells()[col + 1].width,
                        CellWidth::WideSpacer,
                        "{row},{col}"
                    )
                }
                CellWidth::WideSpacer => {
                    assert_eq!(line.cells()[col - 1].width, CellWidth::Wide, "{row},{col}")
                }
                CellWidth::Narrow => {}
            }
        }
    }
}

/// 실제 PTY 출력(ONLCR 변환 포함)과 terminfo `xterm-256color`의 `tput` 출력을 해석합니다.
#[cfg(target_os = "macos")]
#[test]
fn pty_output_with_xterm_256color_terminfo() {
    use crate::launch::TerminalLauncher;
    use crate::pty::tests::{request_with_env, wait_exit, TIMEOUT};
    use crate::pty::PtyLauncher;
    use crate::test_support::{pty_shared, OutputCollector, TempDir};
    use std::ffi::OsString;

    let _pty = pty_shared();
    let dir = TempDir::new();
    let script = "printf 'line1\\nline2\\n'; \
        tput cup 4 10; printf X; \
        tput setaf 1; printf R; tput setab 4; printf B; tput sgr0; printf N; \
        tput cup 6 0; printf 'abcdefgh'; tput cup 6 3; tput el; \
        tput cup 7 0; tput bold; printf W; tput sgr0; \
        tput cup 8 2; printf '\\033(0lqk\\033(B'";
    let env = vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (OsString::from("TERM"), OsString::from(TERM_NAME)),
    ];
    let request = request_with_env("/bin/sh", &["-c", script], dir.path(), env);
    let mut process = PtyLauncher::with_grace(std::time::Duration::from_millis(500))
        .launch(request)
        .unwrap();
    let output = OutputCollector::start(process.output_reader().unwrap());
    let report = wait_exit(process.as_mut());
    assert_eq!(report.status, crate::state::TerminalExitStatus::Exited(0));
    let bytes = output.wait_eof(TIMEOUT);
    let mut t = Terminal::new(TerminalSize::DEFAULT);
    t.feed(bytes.as_bytes());
    let screen = t.screen();
    let lines = screen.lines();
    assert_eq!(lines[0], "line1");
    assert_eq!(lines[1], "line2");
    assert_eq!(lines[4], format!("{}XRBN", " ".repeat(10)));
    assert_eq!(style(&t, 4, 11).fg, Color::Indexed(1));
    assert_eq!(style(&t, 4, 12).bg, Color::Indexed(4));
    assert_eq!(style(&t, 4, 13), Style::default());
    assert_eq!(lines[6], "abc");
    assert!(style(&t, 7, 0).attrs.contains(Attrs::BOLD));
    assert_eq!(lines[8], "  ┌─┐");
}
