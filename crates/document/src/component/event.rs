//! 상태 머신 이벤트 종류 (IX.5.2).
//!
//! 컴포넌트 상태 머신의 transition이 반응하는 초기 이벤트 범위입니다(기술 아키텍처 15.3). numeric tag는
//! 저장 형식 계약이며 golden test로 고정합니다. 이 목록은 닫힌 enum이고, 모르는 tag는 디코딩에서
//! 거부합니다.
//!
//! | kind | tag | payload |
//! |---|---|---|
//! | Activate | 0x01 | 없음 |
//! | Input | 0x02 | `String`(입력 후 전체 텍스트) |
//! | Focus | 0x03 | 없음 |
//! | Blur | 0x04 | 없음 |
//! | Key | 0x05 | `String`(정규화된 key 식별자, 예: `Enter`) |
//! | SelectionChange | 0x06 | `String`(선택된 item identity key, IX.6.5) |
//! | Open | 0x07 | 없음 |
//! | Close | 0x08 | 없음 |
//!
//! Enabled·Disabled 전환은 이벤트가 아니라 `disabled` 상태 값(IX.6.2)으로 표현합니다.

use std::fmt;

use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::{ExpressionType, TypeSpec};

/// 이벤트 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// 활성화(click·tap·Space/Enter 활성화 포함).
    Activate,
    /// 텍스트 입력(IME 확정 포함).
    Input,
    /// focus 획득.
    Focus,
    /// focus 상실.
    Blur,
    /// 키보드 key.
    Key,
    /// 선택 변경.
    SelectionChange,
    /// 열림(modal open 등).
    Open,
    /// 닫힘(modal dismiss 등).
    Close,
}

/// 알 수 없는 이벤트 tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownEventKind(pub u8);

impl fmt::Display for UnknownEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "알 수 없는 이벤트 종류 tag: 0x{:02X}", self.0)
    }
}

impl std::error::Error for UnknownEventKind {}

impl EventKind {
    /// 모든 종류(tag 순).
    pub const ALL: [EventKind; 8] = [
        EventKind::Activate,
        EventKind::Input,
        EventKind::Focus,
        EventKind::Blur,
        EventKind::Key,
        EventKind::SelectionChange,
        EventKind::Open,
        EventKind::Close,
    ];

    /// 안정 numeric tag(저장 형식 계약).
    pub const fn tag(&self) -> u8 {
        match self {
            EventKind::Activate => 0x01,
            EventKind::Input => 0x02,
            EventKind::Focus => 0x03,
            EventKind::Blur => 0x04,
            EventKind::Key => 0x05,
            EventKind::SelectionChange => 0x06,
            EventKind::Open => 0x07,
            EventKind::Close => 0x08,
        }
    }

    /// tag에서 복원합니다.
    pub const fn from_tag(tag: u8) -> Result<EventKind, UnknownEventKind> {
        match tag {
            0x01 => Ok(EventKind::Activate),
            0x02 => Ok(EventKind::Input),
            0x03 => Ok(EventKind::Focus),
            0x04 => Ok(EventKind::Blur),
            0x05 => Ok(EventKind::Key),
            0x06 => Ok(EventKind::SelectionChange),
            0x07 => Ok(EventKind::Open),
            0x08 => Ok(EventKind::Close),
            other => Err(UnknownEventKind(other)),
        }
    }

    /// 진단·표시용 안정 이름.
    pub const fn name(&self) -> &'static str {
        match self {
            EventKind::Activate => "activate",
            EventKind::Input => "input",
            EventKind::Focus => "focus",
            EventKind::Blur => "blur",
            EventKind::Key => "key",
            EventKind::SelectionChange => "selectionChange",
            EventKind::Open => "open",
            EventKind::Close => "close",
        }
    }

    /// 이벤트가 싣는 payload 타입(없으면 `None`).
    pub const fn payload_type(&self) -> Option<TypeSpec> {
        match self {
            EventKind::Input | EventKind::Key | EventKind::SelectionChange => {
                Some(TypeSpec::required(ExpressionType::String))
            }
            EventKind::Activate
            | EventKind::Focus
            | EventKind::Blur
            | EventKind::Open
            | EventKind::Close => None,
        }
    }

    /// payload 값이 이 이벤트 종류에 맞는지. payload가 없는 종류는 `None`만 허용합니다.
    pub fn accepts_payload(&self, payload: Option<&TypedValue>) -> bool {
        match (self.payload_type(), payload) {
            (Some(spec), payload) => spec.accepts(payload),
            (None, None) => true,
            (None, Some(_)) => false,
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_golden_table() {
        // 검증 기준(IX.5.2): kind golden test. tag·이름·payload 타입은 저장 형식 계약입니다.
        let golden: [(EventKind, u8, &str, Option<ExpressionType>); 8] = [
            (EventKind::Activate, 0x01, "activate", None),
            (
                EventKind::Input,
                0x02,
                "input",
                Some(ExpressionType::String),
            ),
            (EventKind::Focus, 0x03, "focus", None),
            (EventKind::Blur, 0x04, "blur", None),
            (EventKind::Key, 0x05, "key", Some(ExpressionType::String)),
            (
                EventKind::SelectionChange,
                0x06,
                "selectionChange",
                Some(ExpressionType::String),
            ),
            (EventKind::Open, 0x07, "open", None),
            (EventKind::Close, 0x08, "close", None),
        ];
        assert_eq!(EventKind::ALL.len(), golden.len());
        for (kind, (expected, tag, name, payload)) in EventKind::ALL.iter().zip(golden) {
            assert_eq!(*kind, expected);
            assert_eq!(kind.tag(), tag);
            assert_eq!(kind.name(), name);
            assert_eq!(kind.payload_type(), payload.map(TypeSpec::required));
            assert_eq!(EventKind::from_tag(tag), Ok(*kind));
        }
        for tag in [0x00, 0x09, 0x7F, 0xFF] {
            assert_eq!(EventKind::from_tag(tag), Err(UnknownEventKind(tag)));
        }
    }

    #[test]
    fn payload_acceptance() {
        let text = TypedValue::String("Enter".into());
        assert!(EventKind::Key.accepts_payload(Some(&text)));
        assert!(!EventKind::Key.accepts_payload(None));
        assert!(!EventKind::Input.accepts_payload(Some(&TypedValue::Boolean(true))));
        assert!(EventKind::Activate.accepts_payload(None));
        assert!(!EventKind::Activate.accepts_payload(Some(&text)));
    }
}
