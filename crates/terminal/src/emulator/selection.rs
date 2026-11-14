//! 선택 영역 (XVIII.4.2).
//!
//! 위치는 **절대 줄 번호**([`BufferPoint::line`], scrollback 첫 줄부터 센 번호)와 열입니다. 출력이
//! 쌓여 화면이 올라가도 같은 내용을 가리키고, scrollback에서 버려진 줄은 선택에서 빠집니다
//! ([`Screen::set_selection`](super::Screen::set_selection) 문서).
//!
//! - [`SelectionMode::Simple`]: 시작점부터 끝점까지 흐르는 선택(드래그).
//! - [`SelectionMode::Word`]: 양 끝을 단어 경계까지 넓힘(두 번 누름). 단어 문자는 공백과
//!   [`WORD_SEPARATORS`]가 아닌 문자입니다.
//! - [`SelectionMode::Line`]: 줄 전체(세 번 누름).
//! - [`SelectionMode::Block`]: 직사각형(열 범위 고정, Option 드래그).
//!
//! 복사 문자열은 자동 줄바꿈으로 이어진 줄을 개행 없이 잇고, 줄 끝 공백을 뺍니다.

/// 단어 선택에서 단어를 끊는 문자(공백 외).
pub const WORD_SEPARATORS: &str = "()[]{}<>'\"`|;,";

/// 화면 buffer 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferPoint {
    /// 절대 줄 번호.
    pub line: u64,
    /// 열(0부터).
    pub col: usize,
}

impl BufferPoint {
    /// 위치.
    pub fn new(line: u64, col: usize) -> BufferPoint {
        BufferPoint { line, col }
    }
}

/// 선택 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionMode {
    /// 흐르는 선택.
    Simple,
    /// 단어 단위.
    Word,
    /// 줄 단위.
    Line,
    /// 직사각형.
    Block,
}

/// 선택 영역(시작점 `anchor`와 움직이는 끝점 `head`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    mode: SelectionMode,
    anchor: BufferPoint,
    head: BufferPoint,
}

impl Selection {
    /// `at`에서 시작하는 선택.
    pub fn new(mode: SelectionMode, at: BufferPoint) -> Selection {
        Selection {
            mode,
            anchor: at,
            head: at,
        }
    }

    /// 끝점을 옮깁니다(드래그).
    pub fn update(&mut self, head: BufferPoint) {
        self.head = head;
    }

    /// 방식.
    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// 시작점.
    pub fn anchor(&self) -> BufferPoint {
        self.anchor
    }

    /// 끝점.
    pub fn head(&self) -> BufferPoint {
        self.head
    }

    /// (앞, 뒤) 순서의 두 점.
    pub fn ordered(&self) -> (BufferPoint, BufferPoint) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// 첫 줄이 `first_line`보다 앞이면 잘라 냅니다. 선택 전체가 앞이면 `None`.
    pub(crate) fn clamp_to(self, first_line: u64) -> Option<Selection> {
        let (start, end) = self.ordered();
        if end.line < first_line {
            return None;
        }
        if start.line >= first_line {
            return Some(self);
        }
        let clamped = BufferPoint::new(first_line, 0);
        let mut selection = self;
        if selection.anchor <= selection.head {
            selection.anchor = clamped;
        } else {
            selection.head = clamped;
        }
        Some(selection)
    }
}

/// 단어 문자인지.
pub(crate) fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !WORD_SEPARATORS.contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_and_clamping() {
        let mut selection = Selection::new(SelectionMode::Simple, BufferPoint::new(5, 3));
        selection.update(BufferPoint::new(2, 7));
        assert_eq!(
            selection.ordered(),
            (BufferPoint::new(2, 7), BufferPoint::new(5, 3))
        );
        assert_eq!(selection.clamp_to(6), None);
        let clamped = selection.clamp_to(4).unwrap();
        assert_eq!(
            clamped.ordered(),
            (BufferPoint::new(4, 0), BufferPoint::new(5, 3))
        );
        assert_eq!(clamped.anchor(), BufferPoint::new(5, 3));
        assert_eq!(selection.clamp_to(2), Some(selection));
        assert!(is_word_char('a') && is_word_char('한') && is_word_char('/'));
        assert!(!is_word_char(' ') && !is_word_char('(') && !is_word_char('"'));
    }
}
