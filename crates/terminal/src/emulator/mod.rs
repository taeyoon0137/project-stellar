//! 터미널 emulator (XVIII.4.1).
//!
//! PTY 출력 byte를 [`Terminal::feed`]로 받아 화면 격자([`Screen`])를 갱신합니다. UI(XVIII.4.3)는 화면을
//! 읽어 그리고, [`Terminal::take_replies`]의 응답(DSR·DA)을 PTY 입력으로 돌려보냅니다.
//!
//! - **스레드**: `feed`는 출력 크기에 비례하는 일을 하므로 main thread가 아니라 출력 reader 쪽에서
//!   부릅니다(기술 아키텍처 6 main-thread 예산). `Terminal`은 `Send`입니다.
//! - **child가 보는 터미널 종류**: registry는 profile이 따로 정하지 않으면 child 환경에
//!   `TERM=`[`TERM_NAME`], `COLORTERM=`[`COLORTERM`]을 넣습니다. 이 emulator가 해석하는 sequence
//!   집합이 macOS terminfo `xterm-256color`의 기본 기능과 맞기 때문입니다.
//! - **저장하지 않음**: 화면·출력은 세션 메모리에만 있습니다(crate 문서).

pub mod cell;
pub mod screen;

#[cfg(test)]
mod corpus;

pub use cell::{Attrs, Cell, CellWidth, Color, Style};
pub use screen::{CursorShape, CursorState, Modes, MouseTracking, Row, Screen, TerminalEvent};

use crate::size::TerminalSize;
use crate::vt::parser::Parser;
use screen::ScreenPerformer;

/// child 환경 `TERM` 기본값.
pub const TERM_NAME: &str = "xterm-256color";
/// child 환경 `COLORTERM` 기본값(24-bit 색 지원 알림).
pub const COLORTERM: &str = "truecolor";

/// VT parser와 화면을 묶은 터미널 emulator.
#[derive(Debug)]
pub struct Terminal {
    parser: Parser,
    screen: Screen,
}

impl Terminal {
    /// 빈 화면의 emulator.
    pub fn new(size: TerminalSize) -> Terminal {
        Terminal {
            parser: Parser::new(),
            screen: Screen::new(size),
        }
    }

    /// PTY 출력을 처리합니다. 입력을 어떻게 나눠 넣어도 결과가 같습니다.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser
            .advance(&mut ScreenPerformer(&mut self.screen), bytes);
    }

    /// 화면.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// 크기를 바꿉니다(PTY resize와 함께 호출).
    pub fn resize(&mut self, size: TerminalSize) {
        self.screen.resize(size);
    }

    /// PTY로 돌려보낼 응답(DSR·DA)을 꺼냅니다.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.screen.take_replies()
    }

    /// 쌓인 알림(BEL·제목·작업 디렉터리)을 꺼냅니다.
    pub fn take_events(&mut self) -> Vec<TerminalEvent> {
        self.screen.take_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Terminal>();
    }
}
