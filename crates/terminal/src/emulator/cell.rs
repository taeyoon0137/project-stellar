//! 화면 셀·색·글자 속성 (XVIII.4.1).

/// 셀 색. 실제 RGB 값(팔레트·테마)은 UI가 정합니다(시스템 semantic color 사용, macOS 셸 설계).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    /// 기본 전경·배경.
    #[default]
    Default,
    /// 256색 팔레트 번호(0~7 기본, 8~15 밝은 색, 16~255 확장).
    Indexed(u8),
    /// 24-bit 색.
    Rgb(u8, u8, u8),
}

/// 글자 속성 bit 집합.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attrs(u16);

impl Attrs {
    /// 굵게(SGR 1).
    pub const BOLD: Attrs = Attrs(1 << 0);
    /// 흐리게(SGR 2).
    pub const DIM: Attrs = Attrs(1 << 1);
    /// 기울임(SGR 3).
    pub const ITALIC: Attrs = Attrs(1 << 2);
    /// 밑줄(SGR 4, 4:1~4:5).
    pub const UNDERLINE: Attrs = Attrs(1 << 3);
    /// 이중 밑줄(SGR 21, 4:2).
    pub const DOUBLE_UNDERLINE: Attrs = Attrs(1 << 4);
    /// 깜박임(SGR 5·6).
    pub const BLINK: Attrs = Attrs(1 << 5);
    /// 반전(SGR 7).
    pub const INVERSE: Attrs = Attrs(1 << 6);
    /// 숨김(SGR 8).
    pub const HIDDEN: Attrs = Attrs(1 << 7);
    /// 취소선(SGR 9).
    pub const STRIKE: Attrs = Attrs(1 << 8);

    /// 속성 없음.
    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    /// `other`의 bit가 모두 켜져 있는지.
    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    /// 속성이 없는지.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 두 집합의 합.
    pub const fn union(self, other: Attrs) -> Attrs {
        Attrs(self.0 | other.0)
    }

    pub(crate) fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    pub(crate) fn remove(&mut self, other: Attrs) {
        self.0 &= !other.0;
    }
}

/// 셀 표시 스타일(SGR 결과).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    /// 전경색.
    pub fg: Color,
    /// 배경색.
    pub bg: Color,
    /// 글자 속성.
    pub attrs: Attrs,
}

/// 넓은 문자(2칸) 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CellWidth {
    /// 1칸 문자(또는 빈 칸).
    #[default]
    Narrow,
    /// 2칸 문자의 앞 칸(문자가 있음).
    Wide,
    /// 2칸 문자의 뒤 칸(문자 없음, 표시하지 않음).
    WideSpacer,
}

/// 화면 셀 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    /// 문자(빈 칸은 공백). `WideSpacer`는 공백입니다.
    pub ch: char,
    /// 스타일.
    pub style: Style,
    /// 폭 표시.
    pub width: CellWidth,
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

impl Cell {
    /// 기본 스타일의 빈 칸.
    pub const BLANK: Cell = Cell {
        ch: ' ',
        style: Style {
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::empty(),
        },
        width: CellWidth::Narrow,
    };

    /// 지우기로 만든 빈 칸. 배경색만 현재 pen을 따릅니다(xterm `bce`).
    pub(crate) fn erased(pen: &Style) -> Cell {
        Cell {
            style: Style {
                bg: pen.bg,
                ..Style::default()
            },
            ..Cell::BLANK
        }
    }

    /// 기본 빈 칸과 같은지(scrollback 저장 시 줄 끝 정리에 씀).
    pub fn is_default_blank(&self) -> bool {
        *self == Cell::BLANK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_bits_and_cell_size() {
        let mut attrs = Attrs::empty();
        assert!(attrs.is_empty());
        attrs.insert(Attrs::BOLD);
        attrs.insert(Attrs::UNDERLINE);
        assert!(attrs.contains(Attrs::BOLD) && attrs.contains(Attrs::UNDERLINE));
        attrs.remove(Attrs::BOLD);
        assert!(!attrs.contains(Attrs::BOLD));
        // scrollback 메모리 계산의 기준이 되는 셀 크기(char 4 + 색 2×4 + 속성 2 + 폭 1, 정렬 4).
        assert_eq!(std::mem::size_of::<Cell>(), 16);
        let pen = Style {
            fg: Color::Indexed(1),
            bg: Color::Rgb(1, 2, 3),
            attrs: Attrs::BOLD,
        };
        let erased = Cell::erased(&pen);
        assert_eq!(erased.style.bg, Color::Rgb(1, 2, 3));
        assert_eq!(erased.style.fg, Color::Default);
        assert!(erased.style.attrs.is_empty());
        assert!(Cell::default().is_default_blank());
    }
}
