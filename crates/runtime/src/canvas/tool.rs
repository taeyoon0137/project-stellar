//! 캔버스 도구 10종과 도구 상태 기계 (XII.10.1, ADR-0104 결정 1).
//!
//! [`CanvasTool`]은 닫힌 enum입니다. 창마다 [`CanvasToolState`] 하나가 지금 고른 도구(base)와 일시 전환(Space를 누르는
//! 동안 Hand, ⌘를 누르는 동안 깊은 선택)을 갖습니다. 도구 상태는 문서·저장·history 어디에도 들어가지 않는 창의 view
//! 상태입니다.
//!
//! 입력은 [`ToolInput`](키 누름·뗌, 수식키 변화, 초점 상실, 제스처 시작·끝, 생성 완료, 파일 고르기 취소)이고 결과는
//! [`ToolEvent`]입니다. 텍스트 편집 중에는 키가 텍스트로 가야 하므로 호출자가 키 입력을 이 기계에 넘기지 않습니다.

use stellar_scene::interaction::InteractionTool;

/// 캔버스 도구(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CanvasTool {
    /// 선택·이동·크기 조절·회전.
    Move,
    /// Frame 만들기.
    Frame,
    /// Rectangle 만들기.
    Rectangle,
    /// Ellipse 만들기.
    Ellipse,
    /// Line 만들기.
    Line,
    /// Polygon 만들기.
    Polygon,
    /// Text 만들기.
    Text,
    /// Image 넣기(파일 고르기).
    Image,
    /// 화면 이동.
    Hand,
    /// 확대·축소.
    Zoom,
}

impl CanvasTool {
    /// 도구 10종(도구 막대 순서).
    pub const ALL: [CanvasTool; 10] = [
        CanvasTool::Move,
        CanvasTool::Frame,
        CanvasTool::Rectangle,
        CanvasTool::Ellipse,
        CanvasTool::Line,
        CanvasTool::Polygon,
        CanvasTool::Text,
        CanvasTool::Image,
        CanvasTool::Hand,
        CanvasTool::Zoom,
    ];

    /// 도구를 고르는 키(ADR-0104 결정 1.5). Polygon은 단일 키가 없습니다.
    pub const fn shortcut(self) -> Option<ToolShortcut> {
        let (key, modifiers) = match self {
            CanvasTool::Move => ('v', Modifiers::NONE),
            CanvasTool::Frame => ('f', Modifiers::NONE),
            CanvasTool::Rectangle => ('r', Modifiers::NONE),
            CanvasTool::Ellipse => ('o', Modifiers::NONE),
            CanvasTool::Line => ('l', Modifiers::NONE),
            CanvasTool::Text => ('t', Modifiers::NONE),
            CanvasTool::Image => ('k', Modifiers::SHIFT_COMMAND),
            CanvasTool::Hand => ('h', Modifiers::NONE),
            CanvasTool::Zoom => ('z', Modifiers::NONE),
            CanvasTool::Polygon => return None,
        };
        Some(ToolShortcut { key, modifiers })
    }

    /// 누른 키에 해당하는 도구. 글자는 대소문자를 가리지 않습니다.
    pub fn from_shortcut(key: char, modifiers: Modifiers) -> Option<CanvasTool> {
        let key = key.to_ascii_lowercase();
        CanvasTool::ALL.into_iter().find(|tool| {
            tool.shortcut()
                .is_some_and(|s| s.key == key && s.modifiers == modifiers.without_control())
        })
    }

    /// node를 만드는 도구인지(만든 뒤 Move로 돌아감).
    pub const fn creates(self) -> bool {
        matches!(
            self,
            CanvasTool::Frame
                | CanvasTool::Rectangle
                | CanvasTool::Ellipse
                | CanvasTool::Line
                | CanvasTool::Polygon
                | CanvasTool::Text
                | CanvasTool::Image
        )
    }

    /// pointer gesture를 해석하는 상호작용 도구(XII.3.6). 만들기 도구는 그 기계 밖이라 `None`입니다.
    pub const fn interaction(self) -> Option<InteractionTool> {
        match self {
            CanvasTool::Move => Some(InteractionTool::Move),
            CanvasTool::Hand => Some(InteractionTool::Hand),
            CanvasTool::Zoom => Some(InteractionTool::Zoom),
            _ => None,
        }
    }
}

/// 도구 키.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToolShortcut {
    /// 소문자 글자.
    pub key: char,
    /// 함께 누르는 수식키.
    pub modifiers: Modifiers,
}

/// 누른 수식키.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Option(Alt).
    pub option: bool,
    /// ⌘(Command).
    pub command: bool,
    /// Control.
    pub control: bool,
}

impl Modifiers {
    /// 아무것도 누르지 않음.
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        option: false,
        command: false,
        control: false,
    };
    /// Shift.
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    /// Option.
    pub const OPTION: Modifiers = Modifiers {
        option: true,
        ..Modifiers::NONE
    };
    /// ⌘.
    pub const COMMAND: Modifiers = Modifiers {
        command: true,
        ..Modifiers::NONE
    };
    /// ⇧⌘.
    pub const SHIFT_COMMAND: Modifiers = Modifiers {
        shift: true,
        command: true,
        ..Modifiers::NONE
    };

    /// Control을 뺀 값. Control-클릭은 문맥 메뉴(XII.10.39)라 도구 키 비교에서 뺍니다.
    pub const fn without_control(self) -> Modifiers {
        Modifiers {
            control: false,
            ..self
        }
    }
}

/// 도구 기계가 보는 키.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanvasKey {
    /// Space.
    Space,
    /// Escape.
    Escape,
    /// 글자 키(도구 키 판정).
    Character(char),
    /// 그 밖의 키(도구 기계는 쓰지 않음).
    Other,
}

/// 도구 기계 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolInput {
    /// 키 누름. `repeat`는 운영체제 키 반복입니다.
    KeyDown {
        /// 키.
        key: CanvasKey,
        /// 수식키.
        modifiers: Modifiers,
        /// 키 반복.
        repeat: bool,
    },
    /// 키 뗌.
    KeyUp {
        /// 키.
        key: CanvasKey,
        /// 수식키.
        modifiers: Modifiers,
    },
    /// 수식키만 바뀜.
    ModifiersChanged(Modifiers),
    /// 캔버스가 초점을 잃음. 수식키·Space 상태를 지웁니다.
    FocusLost,
    /// pointer gesture 시작.
    GestureStarted,
    /// pointer gesture 끝(완료·취소 모두).
    GestureEnded,
    /// 만들기 도구가 node 하나를 만들었음. Move로 돌아갑니다.
    CreationFinished,
    /// Image 도구의 파일 고르기를 취소함. 이전 도구로 돌아갑니다.
    ImagePickCancelled,
}

/// 도구 기계 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEvent {
    /// 실효 도구(일시 전환 포함)가 바뀜.
    Changed {
        /// 이전 실효 도구.
        from: CanvasTool,
        /// 새 실효 도구.
        to: CanvasTool,
    },
    /// 도구 기계가 처리했지만 실효 도구는 그대로입니다(수식키 갱신 등).
    Unchanged,
    /// 제스처 중 Space: Hand로 바꾸지 않고 진행 중 제스처에 넘깁니다(생성 드래그의 시작점 이동).
    SpaceInGesture {
        /// 누름이면 참, 뗌이면 거짓.
        pressed: bool,
    },
    /// 도구 기계가 쓰지 않는 입력. 호출자가 다음 처리(선택 이동 등)에 넘깁니다.
    NotConsumed,
}

/// 창별 도구 상태. 저장·history 대상이 아닙니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasToolState {
    base: CanvasTool,
    /// Image 도구를 고르기 직전 도구(파일 고르기 취소 시 복귀).
    before_image: CanvasTool,
    modifiers: Modifiers,
    /// Space를 누르는 동안 Hand.
    space_hand: bool,
    /// Hand 제스처 중 Space를 뗌: 제스처가 끝나면 일시 Hand를 끝냅니다.
    space_release_pending: bool,
    /// 제스처 중 눌린 Space(제스처 쪽에 넘김).
    space_in_gesture: bool,
    gesture_active: bool,
}

impl Default for CanvasToolState {
    fn default() -> CanvasToolState {
        CanvasToolState::new()
    }
}

impl CanvasToolState {
    /// Move 도구, 수식키 없음.
    pub const fn new() -> CanvasToolState {
        CanvasToolState {
            base: CanvasTool::Move,
            before_image: CanvasTool::Move,
            modifiers: Modifiers::NONE,
            space_hand: false,
            space_release_pending: false,
            space_in_gesture: false,
            gesture_active: false,
        }
    }

    /// 고른 도구(일시 전환 제외).
    pub const fn base(&self) -> CanvasTool {
        self.base
    }

    /// 실효 도구: Space를 누르는 동안 Hand, 아니면 고른 도구.
    pub const fn current(&self) -> CanvasTool {
        if self.space_hand {
            CanvasTool::Hand
        } else {
            self.base
        }
    }

    /// 지금 누른 수식키.
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// ⌘를 누르는 동안 Move 도구의 깊은 선택.
    pub const fn deep_select(&self) -> bool {
        self.modifiers.command && matches!(self.current(), CanvasTool::Move)
    }

    /// pointer gesture가 진행 중인지.
    pub const fn gesture_active(&self) -> bool {
        self.gesture_active
    }

    /// 도구를 고릅니다(도구 막대·메뉴). 일시 Hand 중이면 Space를 뗀 뒤 이 도구가 됩니다.
    pub fn select(&mut self, tool: CanvasTool) -> ToolEvent {
        let from = self.current();
        if tool == CanvasTool::Image && self.base != CanvasTool::Image {
            self.before_image = self.base;
        }
        self.base = tool;
        self.changed(from)
    }

    /// Escape 단계 2: Move가 아닌 도구를 Move로 돌립니다. 바꿨으면 참입니다(ADR-0104 결정 3.5).
    pub fn reset_to_move(&mut self) -> bool {
        if self.base == CanvasTool::Move {
            return false;
        }
        self.base = CanvasTool::Move;
        true
    }

    /// 입력 하나를 처리합니다.
    pub fn handle(&mut self, input: ToolInput) -> ToolEvent {
        let from = self.current();
        match input {
            ToolInput::KeyDown {
                key,
                modifiers,
                repeat,
            } => {
                self.modifiers = modifiers;
                match key {
                    CanvasKey::Space => {
                        if repeat {
                            return ToolEvent::Unchanged;
                        }
                        if self.gesture_active && !self.space_hand {
                            self.space_in_gesture = true;
                            return ToolEvent::SpaceInGesture { pressed: true };
                        }
                        self.space_hand = true;
                        self.space_release_pending = false;
                        self.changed(from)
                    }
                    CanvasKey::Escape => ToolEvent::NotConsumed,
                    CanvasKey::Character(c) if !repeat && !self.gesture_active => {
                        match CanvasTool::from_shortcut(c, modifiers) {
                            Some(tool) => self.select(tool),
                            None => ToolEvent::NotConsumed,
                        }
                    }
                    CanvasKey::Character(_) | CanvasKey::Other => ToolEvent::NotConsumed,
                }
            }
            ToolInput::KeyUp { key, modifiers } => {
                self.modifiers = modifiers;
                if key != CanvasKey::Space {
                    return ToolEvent::NotConsumed;
                }
                if self.space_in_gesture {
                    self.space_in_gesture = false;
                    return ToolEvent::SpaceInGesture { pressed: false };
                }
                if self.space_hand && self.gesture_active {
                    self.space_release_pending = true;
                    return ToolEvent::Unchanged;
                }
                self.space_hand = false;
                self.changed(from)
            }
            ToolInput::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                ToolEvent::Unchanged
            }
            ToolInput::FocusLost => {
                self.modifiers = Modifiers::NONE;
                self.space_in_gesture = false;
                if self.gesture_active && self.space_hand {
                    self.space_release_pending = true;
                } else {
                    self.space_hand = false;
                }
                self.changed(from)
            }
            ToolInput::GestureStarted => {
                self.gesture_active = true;
                ToolEvent::Unchanged
            }
            ToolInput::GestureEnded => {
                self.gesture_active = false;
                self.space_in_gesture = false;
                if self.space_release_pending {
                    self.space_release_pending = false;
                    self.space_hand = false;
                }
                self.changed(from)
            }
            ToolInput::CreationFinished => {
                if self.base.creates() {
                    self.base = CanvasTool::Move;
                }
                self.changed(from)
            }
            ToolInput::ImagePickCancelled => {
                if self.base == CanvasTool::Image {
                    self.base = self.before_image;
                }
                self.changed(from)
            }
        }
    }

    fn changed(&self, from: CanvasTool) -> ToolEvent {
        let to = self.current();
        if from == to {
            ToolEvent::Unchanged
        } else {
            ToolEvent::Changed { from, to }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::save_bytes;
    use crate::window_session::test_fixtures::{sample_document, window};
    use stellar_commands::state::DocumentState;
    use stellar_history::window::WindowHistory;

    fn down(key: CanvasKey, modifiers: Modifiers) -> ToolInput {
        ToolInput::KeyDown {
            key,
            modifiers,
            repeat: false,
        }
    }

    fn up(key: CanvasKey) -> ToolInput {
        ToolInput::KeyUp {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    fn ch(c: char) -> ToolInput {
        down(CanvasKey::Character(c), Modifiers::NONE)
    }

    #[test]
    fn ten_tools_have_distinct_shortcuts_and_roles() {
        assert_eq!(CanvasTool::ALL.len(), 10);
        let mut keys = std::collections::BTreeSet::new();
        for tool in CanvasTool::ALL {
            if let Some(s) = tool.shortcut() {
                assert!(keys.insert((s.key, s.modifiers.shift, s.modifiers.command)));
                assert_eq!(CanvasTool::from_shortcut(s.key, s.modifiers), Some(tool));
                assert_eq!(
                    CanvasTool::from_shortcut(s.key.to_ascii_uppercase(), s.modifiers),
                    Some(tool)
                );
            }
        }
        assert_eq!(CanvasTool::Polygon.shortcut(), None);
        assert_eq!(CanvasTool::from_shortcut('p', Modifiers::NONE), None);
        assert_eq!(CanvasTool::from_shortcut('k', Modifiers::NONE), None);
        let creating: Vec<_> = CanvasTool::ALL
            .into_iter()
            .filter(|t| t.creates())
            .collect();
        assert_eq!(creating.len(), 7);
        assert_eq!(CanvasTool::Move.interaction(), Some(InteractionTool::Move));
        assert_eq!(CanvasTool::Rectangle.interaction(), None);
    }

    #[test]
    fn switching_temporary_switches_and_escape_fixture() {
        // 검증 기준(XII.10.1): 도구 전환·일시 전환·Escape 복귀 fixture.
        let mut tools = CanvasToolState::new();
        assert_eq!(tools.current(), CanvasTool::Move);
        assert_eq!(
            tools.handle(ch('r')),
            ToolEvent::Changed {
                from: CanvasTool::Move,
                to: CanvasTool::Rectangle
            }
        );
        // 키 반복은 무시합니다.
        assert_eq!(
            tools.handle(ToolInput::KeyDown {
                key: CanvasKey::Character('o'),
                modifiers: Modifiers::NONE,
                repeat: true
            }),
            ToolEvent::NotConsumed
        );
        // Space 일시 Hand와 복귀.
        assert_eq!(
            tools.handle(down(CanvasKey::Space, Modifiers::NONE)),
            ToolEvent::Changed {
                from: CanvasTool::Rectangle,
                to: CanvasTool::Hand
            }
        );
        assert_eq!(
            tools.handle(ToolInput::KeyDown {
                key: CanvasKey::Space,
                modifiers: Modifiers::NONE,
                repeat: true
            }),
            ToolEvent::Unchanged
        );
        assert_eq!(tools.base(), CanvasTool::Rectangle);
        assert_eq!(
            tools.handle(up(CanvasKey::Space)),
            ToolEvent::Changed {
                from: CanvasTool::Hand,
                to: CanvasTool::Rectangle
            }
        );
        // Escape: Move가 아닌 도구 → Move. Move에서는 다음 단계로 넘깁니다.
        assert_eq!(
            tools.handle(down(CanvasKey::Escape, Modifiers::NONE)),
            ToolEvent::NotConsumed
        );
        assert!(tools.reset_to_move());
        assert_eq!(tools.current(), CanvasTool::Move);
        assert!(!tools.reset_to_move());
        // ⌘를 누르는 동안 깊은 선택(Move만).
        tools.handle(ToolInput::ModifiersChanged(Modifiers::COMMAND));
        assert!(tools.deep_select());
        tools.select(CanvasTool::Hand);
        assert!(!tools.deep_select());
        tools.select(CanvasTool::Move);
        tools.handle(ToolInput::ModifiersChanged(Modifiers::NONE));
        assert!(!tools.deep_select());
        // 생성 완료 → Move, Image 취소 → 이전 도구.
        tools.select(CanvasTool::Ellipse);
        tools.handle(ToolInput::CreationFinished);
        assert_eq!(tools.base(), CanvasTool::Move);
        tools.select(CanvasTool::Line);
        assert_eq!(
            tools.handle(down(CanvasKey::Character('K'), Modifiers::SHIFT_COMMAND)),
            ToolEvent::Changed {
                from: CanvasTool::Line,
                to: CanvasTool::Image
            }
        );
        tools.handle(ToolInput::ImagePickCancelled);
        assert_eq!(tools.base(), CanvasTool::Line);
    }

    #[test]
    fn space_during_gesture_goes_to_the_gesture_and_hand_waits_for_the_gesture_end() {
        let mut tools = CanvasToolState::new();
        tools.select(CanvasTool::Rectangle);
        tools.handle(ToolInput::GestureStarted);
        // 생성 드래그 중 Space는 Hand가 아니라 시작점 이동.
        assert_eq!(
            tools.handle(down(CanvasKey::Space, Modifiers::NONE)),
            ToolEvent::SpaceInGesture { pressed: true }
        );
        assert_eq!(tools.current(), CanvasTool::Rectangle);
        assert_eq!(
            tools.handle(up(CanvasKey::Space)),
            ToolEvent::SpaceInGesture { pressed: false }
        );
        // 제스처 중 도구 키는 쓰지 않습니다.
        assert_eq!(tools.handle(ch('v')), ToolEvent::NotConsumed);
        tools.handle(ToolInput::GestureEnded);
        // 일시 Hand로 팬하는 중 Space를 떼면 제스처가 끝날 때 돌아갑니다.
        tools.handle(down(CanvasKey::Space, Modifiers::NONE));
        tools.handle(ToolInput::GestureStarted);
        assert_eq!(tools.handle(up(CanvasKey::Space)), ToolEvent::Unchanged);
        assert_eq!(tools.current(), CanvasTool::Hand);
        assert_eq!(
            tools.handle(ToolInput::GestureEnded),
            ToolEvent::Changed {
                from: CanvasTool::Hand,
                to: CanvasTool::Rectangle
            }
        );
        // 초점 상실은 수식키와 Space를 지웁니다.
        tools.handle(down(CanvasKey::Space, Modifiers::COMMAND));
        assert_eq!(tools.current(), CanvasTool::Hand);
        tools.handle(ToolInput::FocusLost);
        assert_eq!(tools.current(), CanvasTool::Rectangle);
        assert_eq!(tools.modifiers(), Modifiers::NONE);
    }

    #[test]
    fn tool_changes_leave_save_bytes_and_history_alone() {
        // 검증 기준(XII.10.1): 전환 전후 save bytes와 history 길이 동일.
        let state = DocumentState::new(sample_document());
        let history = WindowHistory::new(window(1));
        let before = (
            save_bytes(&state),
            history.history().len(),
            state.revision(),
        );
        let mut tools = CanvasToolState::new();
        for input in [
            ch('f'),
            down(CanvasKey::Space, Modifiers::NONE),
            up(CanvasKey::Space),
            ch('t'),
            down(CanvasKey::Escape, Modifiers::NONE),
            ToolInput::ModifiersChanged(Modifiers::COMMAND),
            ToolInput::FocusLost,
        ] {
            tools.handle(input);
        }
        tools.reset_to_move();
        assert_eq!(
            (
                save_bytes(&state),
                history.history().len(),
                state.revision()
            ),
            before
        );
    }
}
