//! adapter 노트가 가리키는 접근성 의미 facet(공용).

use stellar_accessibility::node::{ActionKind, StateKind};

/// 접근성 의미 facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    /// 접근 가능한 이름.
    Label,
    /// 값.
    Value,
    /// 상태.
    State(StateKind),
    /// 동작.
    Action(ActionKind),
    /// 역할.
    Role,
    /// 읽기 순서.
    ReadingOrder,
}
