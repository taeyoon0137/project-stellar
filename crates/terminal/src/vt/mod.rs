//! VT escape sequence 해석 (XVIII.4.1).
//!
//! - [`parser`]: DEC ANSI 호환 parser 상태 기계(byte → print·execute·CSI·ESC·OSC·DCS 동작).
//! - [`width`]: 문자 셀 폭(0·1·2). 표는 `tools/gen_width.py`가 생성한 `width_table`입니다.
//!
//! **외부 dependency를 쓰지 않는 이유**: ADR-0019 결정 2는 `stellar-terminal`의 의존을 `core-types`만
//! 허용하고, crate 테스트(`terminal_state_has_no_persistence_dependency`)가 이를 고정합니다. 공개된
//! 상태 기계 명세와 Unicode 데이터만으로 구현할 수 있고, 입력 크기에 비례하는 상한(매개변수·OSC
//! 길이)을 이 crate가 직접 보장해야 하므로 승인 목록(`config/dependencies.yaml`)에 새 crate를 더하지
//! 않았습니다.

pub mod parser;
pub mod width;
mod width_table;
