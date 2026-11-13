//! 터미널 크기(행·열) (XVIII.2.1).
//!
//! PTY를 만들 때의 초기 크기와 resize 요청에 씁니다. 화면 buffer(XVIII.4.2)의 메모리를 제한할 수
//! 있게 행·열 모두 `1..=`[`MAX_DIMENSION`]만 허용합니다.

use std::fmt;

/// 행·열 상한.
pub const MAX_DIMENSION: u16 = 4096;

/// 검증된 터미널 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSize {
    rows: u16,
    cols: u16,
}

impl TerminalSize {
    /// 기본 크기(24행 × 80열).
    pub const DEFAULT: TerminalSize = TerminalSize { rows: 24, cols: 80 };

    /// 크기를 검증합니다.
    pub fn new(rows: u16, cols: u16) -> Result<TerminalSize, SizeError> {
        let valid = |value: u16| (1..=MAX_DIMENSION).contains(&value);
        if !valid(rows) || !valid(cols) {
            return Err(SizeError { rows, cols });
        }
        Ok(TerminalSize { rows, cols })
    }

    /// 행 수.
    pub fn rows(self) -> u16 {
        self.rows
    }

    /// 열 수.
    pub fn cols(self) -> u16 {
        self.cols
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        TerminalSize::DEFAULT
    }
}

/// 허용 범위 밖의 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeError {
    /// 요청 행 수.
    pub rows: u16,
    /// 요청 열 수.
    pub cols: u16,
}

impl fmt::Display for SizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "터미널 크기는 1~{MAX_DIMENSION} 범위여야 합니다({}행 × {}열)",
            self.rows, self.cols
        )
    }
}

impl std::error::Error for SizeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_bounded() {
        let size = TerminalSize::new(50, 132).unwrap();
        assert_eq!((size.rows(), size.cols()), (50, 132));
        assert_eq!(TerminalSize::default(), TerminalSize::DEFAULT);
        assert!(TerminalSize::new(MAX_DIMENSION, MAX_DIMENSION).is_ok());
        for (rows, cols) in [(0, 80), (24, 0), (MAX_DIMENSION + 1, 80), (24, u16::MAX)] {
            assert_eq!(
                TerminalSize::new(rows, cols),
                Err(SizeError { rows, cols }),
                "{rows}x{cols}"
            );
        }
    }
}
