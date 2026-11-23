//! 프로토타입 그래프 node 종류 registry (XXX.1.3, ADR-0094 결정 3).
//!
//! node 종류는 **닫힌 집합**입니다. 속성 출력(PropertyPath), expression block, 상태 변수, 이벤트 원천, Transition,
//! semantic emit, Context 읽기, MotionSpec, 변환 node만 있습니다. 사용자 정의 코드·JavaScript·파일·네트워크
//! node는 enum에 variant가 없으므로 표현할 수 없고, 모르는 tag는 [`PrototypeNodeKind::from_tag`]가 거부합니다.
//!
//! | kind | tag | 원본 |
//! |---|---|---|
//! | PropertyOutput | 0x01 | `ValueBinding` target(PropertyPath) |
//! | ExpressionBlock | 0x02 | Expression AST node |
//! | StateVariable | 0x03 | 상태 머신 변수 |
//! | EventSource | 0x04 | EventKind trigger의 source node |
//! | Transition | 0x05 | 상태 머신 transition |
//! | SemanticEmit | 0x06 | 상태 머신 semantic action |
//! | ContextRead | 0x07 | Context 읽기(XXX.7.4 전까지 projection 원본 없음) |
//! | MotionSpec | 0x08 | `MotionSpec` record |
//! | Conversion | 0x09 + 변환 tag | 등록된 변환(상태 변화→펄스, 펄스→상태 토글·카운터) |
//!
//! 각 종류의 socket 형태는 [`PrototypeNodeKind::sockets`]가 고정합니다.

use std::fmt;

use super::socket::SocketKind;

/// 등록된 변환 node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConversionKind {
    /// 상태 값 변화 → 펄스.
    StateChangeToPulse,
    /// 펄스 → Boolean 상태 토글.
    PulseToggle,
    /// 펄스 → Number 상태 카운터(+1).
    PulseCounter,
}

impl ConversionKind {
    /// 모든 변환(tag 순).
    pub const ALL: [ConversionKind; 3] = [
        ConversionKind::StateChangeToPulse,
        ConversionKind::PulseToggle,
        ConversionKind::PulseCounter,
    ];

    /// 안정 tag.
    pub const fn tag(self) -> u8 {
        match self {
            ConversionKind::StateChangeToPulse => 0x01,
            ConversionKind::PulseToggle => 0x02,
            ConversionKind::PulseCounter => 0x03,
        }
    }

    /// tag에서 복원합니다.
    pub const fn from_tag(tag: u8) -> Option<ConversionKind> {
        match tag {
            0x01 => Some(ConversionKind::StateChangeToPulse),
            0x02 => Some(ConversionKind::PulseToggle),
            0x03 => Some(ConversionKind::PulseCounter),
            _ => None,
        }
    }

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ConversionKind::StateChangeToPulse => "state-change-to-pulse",
            ConversionKind::PulseToggle => "pulse-toggle",
            ConversionKind::PulseCounter => "pulse-counter",
        }
    }

    /// (입력 종류, 출력 종류).
    pub const fn signature(self) -> (SocketKind, SocketKind) {
        match self {
            ConversionKind::StateChangeToPulse => (SocketKind::State, SocketKind::Pulse),
            ConversionKind::PulseToggle | ConversionKind::PulseCounter => {
                (SocketKind::Pulse, SocketKind::State)
            }
        }
    }
}

/// 프로토타입 node 종류(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrototypeNodeKind {
    /// 속성 출력.
    PropertyOutput,
    /// expression block.
    ExpressionBlock,
    /// 상태 변수.
    StateVariable,
    /// 이벤트 원천.
    EventSource,
    /// Transition.
    Transition,
    /// semantic emit.
    SemanticEmit,
    /// Context 읽기.
    ContextRead,
    /// MotionSpec.
    MotionSpec,
    /// 변환 node.
    Conversion(ConversionKind),
}

/// 알 수 없는 node 종류 tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownNodeKind {
    /// 종류 tag.
    pub tag: u8,
    /// 변환 tag(종류가 변환일 때).
    pub sub: u8,
}

impl fmt::Display for UnknownNodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "알 수 없는 프로토타입 node 종류: 0x{:02X}/0x{:02X}",
            self.tag, self.sub
        )
    }
}

impl std::error::Error for UnknownNodeKind {}

/// socket 한 칸의 형태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketShape {
    /// 안정 이름(node 안에서 유일).
    pub name: &'static str,
    /// 종류.
    pub kind: SocketKind,
    /// 입력이면 `true`.
    pub input: bool,
    /// 같은 이름을 원본 수만큼 반복하는지(Transition의 action 등).
    pub repeated: bool,
}

const fn input(name: &'static str, kind: SocketKind) -> SocketShape {
    SocketShape {
        name,
        kind,
        input: true,
        repeated: false,
    }
}

const fn output(name: &'static str, kind: SocketKind) -> SocketShape {
    SocketShape {
        name,
        kind,
        input: false,
        repeated: false,
    }
}

const fn repeated(shape: SocketShape) -> SocketShape {
    SocketShape {
        repeated: true,
        ..shape
    }
}

use SocketKind::{Pulse, State};

const PROPERTY_OUTPUT: &[SocketShape] = &[input("value", State), input("motion", State)];
/// expression block: child slot마다 `operand`, 참조 block은 참조 대상(상태 변수·이벤트 payload)이 들어오는 `reference`.
const EXPRESSION_BLOCK: &[SocketShape] = &[
    repeated(input("operand", State)),
    input("reference", State),
    output("result", State),
];
const STATE_VARIABLE: &[SocketShape] = &[repeated(input("write", Pulse)), output("value", State)];
const EVENT_SOURCE: &[SocketShape] = &[
    repeated(output("event", Pulse)),
    repeated(output("payload", State)),
];
const TRANSITION: &[SocketShape] = &[
    input("trigger", Pulse),
    input("condition", State),
    repeated(input("set-value", State)),
    repeated(output("set", Pulse)),
    repeated(output("emit", Pulse)),
];
const SEMANTIC_EMIT: &[SocketShape] = &[repeated(input("emit", Pulse)), output("event", Pulse)];
const CONTEXT_READ: &[SocketShape] = &[output("value", State)];
const MOTION_SPEC: &[SocketShape] = &[input("start", State), repeated(output("drive", State))];
const STATE_CHANGE_TO_PULSE: &[SocketShape] = &[input("value", State), output("changed", Pulse)];
const PULSE_TO_STATE: &[SocketShape] = &[input("pulse", Pulse), output("value", State)];

impl PrototypeNodeKind {
    /// 모든 종류(tag 순, 변환은 변환 tag 순).
    pub const ALL: [PrototypeNodeKind; 11] = [
        PrototypeNodeKind::PropertyOutput,
        PrototypeNodeKind::ExpressionBlock,
        PrototypeNodeKind::StateVariable,
        PrototypeNodeKind::EventSource,
        PrototypeNodeKind::Transition,
        PrototypeNodeKind::SemanticEmit,
        PrototypeNodeKind::ContextRead,
        PrototypeNodeKind::MotionSpec,
        PrototypeNodeKind::Conversion(ConversionKind::StateChangeToPulse),
        PrototypeNodeKind::Conversion(ConversionKind::PulseToggle),
        PrototypeNodeKind::Conversion(ConversionKind::PulseCounter),
    ];

    /// (종류 tag, 변환 tag). 변환이 아니면 변환 tag는 0입니다.
    pub const fn tag(self) -> (u8, u8) {
        match self {
            PrototypeNodeKind::PropertyOutput => (0x01, 0),
            PrototypeNodeKind::ExpressionBlock => (0x02, 0),
            PrototypeNodeKind::StateVariable => (0x03, 0),
            PrototypeNodeKind::EventSource => (0x04, 0),
            PrototypeNodeKind::Transition => (0x05, 0),
            PrototypeNodeKind::SemanticEmit => (0x06, 0),
            PrototypeNodeKind::ContextRead => (0x07, 0),
            PrototypeNodeKind::MotionSpec => (0x08, 0),
            PrototypeNodeKind::Conversion(kind) => (0x09, kind.tag()),
        }
    }

    /// tag에서 복원합니다. 모르는 tag, 변환이 아닌 종류의 0 아닌 변환 tag는 거부합니다.
    pub const fn from_tag(tag: u8, sub: u8) -> Result<PrototypeNodeKind, UnknownNodeKind> {
        let kind = match (tag, sub) {
            (0x01, 0) => PrototypeNodeKind::PropertyOutput,
            (0x02, 0) => PrototypeNodeKind::ExpressionBlock,
            (0x03, 0) => PrototypeNodeKind::StateVariable,
            (0x04, 0) => PrototypeNodeKind::EventSource,
            (0x05, 0) => PrototypeNodeKind::Transition,
            (0x06, 0) => PrototypeNodeKind::SemanticEmit,
            (0x07, 0) => PrototypeNodeKind::ContextRead,
            (0x08, 0) => PrototypeNodeKind::MotionSpec,
            (0x09, sub) => match ConversionKind::from_tag(sub) {
                Some(kind) => PrototypeNodeKind::Conversion(kind),
                None => return Err(UnknownNodeKind { tag, sub }),
            },
            _ => return Err(UnknownNodeKind { tag, sub }),
        };
        Ok(kind)
    }

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            PrototypeNodeKind::PropertyOutput => "property-output",
            PrototypeNodeKind::ExpressionBlock => "expression",
            PrototypeNodeKind::StateVariable => "state",
            PrototypeNodeKind::EventSource => "event-source",
            PrototypeNodeKind::Transition => "transition",
            PrototypeNodeKind::SemanticEmit => "semantic-emit",
            PrototypeNodeKind::ContextRead => "context-read",
            PrototypeNodeKind::MotionSpec => "motion",
            PrototypeNodeKind::Conversion(kind) => kind.name(),
        }
    }

    /// 이름에서 복원합니다.
    pub fn from_name(name: &str) -> Option<PrototypeNodeKind> {
        PrototypeNodeKind::ALL
            .into_iter()
            .find(|k| k.name() == name)
    }

    /// socket 형태.
    pub const fn sockets(self) -> &'static [SocketShape] {
        match self {
            PrototypeNodeKind::PropertyOutput => PROPERTY_OUTPUT,
            PrototypeNodeKind::ExpressionBlock => EXPRESSION_BLOCK,
            PrototypeNodeKind::StateVariable => STATE_VARIABLE,
            PrototypeNodeKind::EventSource => EVENT_SOURCE,
            PrototypeNodeKind::Transition => TRANSITION,
            PrototypeNodeKind::SemanticEmit => SEMANTIC_EMIT,
            PrototypeNodeKind::ContextRead => CONTEXT_READ,
            PrototypeNodeKind::MotionSpec => MOTION_SPEC,
            PrototypeNodeKind::Conversion(ConversionKind::StateChangeToPulse) => {
                STATE_CHANGE_TO_PULSE
            }
            PrototypeNodeKind::Conversion(
                ConversionKind::PulseToggle | ConversionKind::PulseCounter,
            ) => PULSE_TO_STATE,
        }
    }

    /// 이름이 `name`인 socket 형태.
    pub fn socket(self, name: &str) -> Option<SocketShape> {
        self.sockets().iter().copied().find(|s| s.name == name)
    }

    /// 실행 코드·파일·네트워크·시스템 시각 접근을 가진 종류인지. 닫힌 집합의 모든 종류는 `false`입니다
    /// (XXX.6.3 inventory가 이 값을 확인합니다).
    pub const fn has_side_channel(self) -> bool {
        match self {
            PrototypeNodeKind::PropertyOutput
            | PrototypeNodeKind::ExpressionBlock
            | PrototypeNodeKind::StateVariable
            | PrototypeNodeKind::EventSource
            | PrototypeNodeKind::Transition
            | PrototypeNodeKind::SemanticEmit
            | PrototypeNodeKind::ContextRead
            | PrototypeNodeKind::MotionSpec
            | PrototypeNodeKind::Conversion(_) => false,
        }
    }
}

impl fmt::Display for PrototypeNodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn registry_is_complete_and_round_trips() {
        // 검증 기준(XXX.1.3): registry 완전성.
        let mut tags = BTreeSet::new();
        let mut names = BTreeSet::new();
        for kind in PrototypeNodeKind::ALL {
            let (tag, sub) = kind.tag();
            assert_eq!(PrototypeNodeKind::from_tag(tag, sub), Ok(kind));
            assert_eq!(PrototypeNodeKind::from_name(kind.name()), Some(kind));
            assert!(tags.insert((tag, sub)), "중복 tag {kind}");
            assert!(names.insert(kind.name()), "중복 이름 {kind}");
            assert!(!kind.sockets().is_empty(), "{kind} socket 없음");
            let mut socket_names = BTreeSet::new();
            for socket in kind.sockets() {
                assert!(socket_names.insert(socket.name), "{kind} socket 이름 중복");
            }
            assert!(!kind.has_side_channel());
        }
        for conversion in ConversionKind::ALL {
            let (from, to) = conversion.signature();
            assert_ne!(from, to, "변환은 종류를 바꿉니다");
            let shapes = PrototypeNodeKind::Conversion(conversion).sockets();
            assert_eq!(
                shapes.iter().filter(|s| s.input).map(|s| s.kind).next(),
                Some(from)
            );
            assert_eq!(
                shapes.iter().filter(|s| !s.input).map(|s| s.kind).next(),
                Some(to)
            );
        }
    }

    #[test]
    fn unknown_serialized_kind_is_rejected() {
        // 검증 기준(XXX.1.3): serialized unknown node kind 거부.
        for (tag, sub) in [
            (0x00, 0),
            (0x0A, 0),
            (0xFF, 0),
            (0x01, 0x01),
            (0x09, 0x00),
            (0x09, 0x04),
        ] {
            assert_eq!(
                PrototypeNodeKind::from_tag(tag, sub),
                Err(UnknownNodeKind { tag, sub })
            );
        }
        for name in ["script", "javascript", "file", "network", "fetch"] {
            assert_eq!(PrototypeNodeKind::from_name(name), None);
        }
    }

    #[test]
    fn only_conversion_nodes_change_socket_kind() {
        for kind in PrototypeNodeKind::ALL {
            if matches!(kind, PrototypeNodeKind::Conversion(_)) {
                continue;
            }
            // 변환이 아닌 종류도 입력·출력 종류가 섞일 수 있지만(예: Transition은 pulse trigger와 state condition),
            // 같은 socket 하나가 두 종류를 받지는 않습니다. 형태가 종류를 정확히 하나만 가지는지 확인합니다.
            for socket in kind.sockets() {
                assert!(matches!(socket.kind, SocketKind::State | SocketKind::Pulse));
            }
        }
    }
}
