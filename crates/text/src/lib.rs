//! stellar-text — Project Stellar 결정론적 텍스트 엔진.
//!
//! 내부 crate 중에서는 core-types만 직접 의존합니다. 외부 의존성은 ADR-0016과
//! `config/dependencies.yaml`의 승인 목록(순수 Rust shaping·Unicode 알고리즘 crate)만 씁니다.
//! raw pointer·FFI가 없도록 `unsafe` 코드를 금지합니다.

#![forbid(unsafe_code)]

/// 폰트 출처, 파일 hash, face 참조 (X.8.1).
pub mod font;

/// 폰트 face registry와 hash+index dedup (X.8.2).
pub mod registry;

/// 명시적 font fallback chain (X.8.3).
pub mod fallback;

/// missing font 진단 (X.8.4).
pub mod missing_font;

/// 공통 shaping 엔진 wrapper (X.9.1).
pub mod shaping;

/// Unicode BiDi level 해석 (X.9.2), 줄 재배치·logical cluster mapping (X.9.3).
pub mod bidi;

/// script·language·direction itemization (X.9.2).
pub mod itemize;

/// 공통 줄바꿈 기회 계산 (X.9.4).
pub mod line_break;

/// grapheme 단위 font fallback과 missing glyph cluster (X.9.5).
pub mod font_runs;

/// 텍스트 레이아웃 고정소수점 단위 (X.10.1).
pub mod units;

/// paragraph shaping pipeline과 원자 segment (X.10.1).
pub mod paragraph;

/// shaping 결과의 너비 기반 줄 나누기 (X.10.1).
pub mod line_layout;

/// 줄 높이·paragraph 간격·가로 정렬·text box 세로 정렬 (X.10.2).
pub mod text_layout;

/// caret 위치·선택 사각형·hit test cluster mapping (X.10.3).
pub mod caret;

#[cfg(test)]
pub(crate) mod test_fonts;
