//! stellar-history — Project Stellar 창별 undo/redo history (Phase VII).
//!
//! 의존 방향은 `history -> commands -> document -> core-types`입니다. history는 문서를 직접
//! 바꾸지 않고 commands의 transaction 실행 경로만 사용합니다. history는 현재 실행 세션의 편집
//! 이벤트이며 Git 이력·저장 파일과 분리됩니다(기술 아키텍처 §3.4·§23).

/// history 항목 (VII.4.1).
pub mod entry;

/// 창 하나의 history stack (VII.4).
pub mod history;

/// history grouping 정책 (VII.4.3~VII.4.5).
pub mod grouping;

/// 창별 history (VII.5).
pub mod window;

/// random command sequence undo/redo property test (VII.6.2).
#[cfg(test)]
mod sequence_property;

/// 영속 history journal schema (XXV.1.1, 로드맵).
pub mod journal;

/// history journal 기록 위치와 기록 (XXV.1.2, 로드맵).
pub mod journal_store;

/// 앱 재실행 후 history 복원과 journal 봉인 (XXV.1.3, 로드맵).
pub mod journal_restore;

/// checkout·merge·rebase 등 기준점 변경 시 journal 봉인 (XXV.1.4, 로드맵).
pub mod journal_base_change;

/// journal 저장 테스트 공용 fixture(임시 디렉터리·hermetic Git).
#[cfg(test)]
mod test_fs;
