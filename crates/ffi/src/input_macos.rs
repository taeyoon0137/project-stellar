//! macOS 셸 입력 adapter (XII.3.2).
//!
//! AppKit `NSEvent`의 원시 필드를 고정 C 구조체 [`StellarMacEvent`]로 받아 공통
//! [`InputEvent`](crate::input::InputEvent)로 변환합니다. 셸(Swift)은 AppKit 값을 가공하지 않고
//! 복사하며, 위치만 AppKit view 변환(`convert(_:from:)`)으로 캔버스 view 좌표로 옮깁니다.
//! 이벤트 종류·button·modifier·phase 해석과 virtual key code → HID usage 변환은 여기서 합니다.
//!
//! # 변환 규칙
//!
//! - **좌표**: `x`/`y`는 캔버스 view 기준 논리 point(좌상단 원점, y 아래로 증가)입니다. 셸의
//!   캔버스 view는 `isFlipped == true`이므로 `convert(event.locationInWindow, from: nil)` 결과를
//!   그대로 넣습니다. 값은 `f64`→`f32` IEEE 반올림 변환 후 유한해야 합니다.
//! - **시각**: `timestamp`를 runtime 시간 domain([`InputTimeDomain`])으로 옮깁니다. `receive_uptime`
//!   (셸 수신 시각)도 `timestamp`처럼 유한한 음이 아닌 값이어야 합니다. 수신 시각보다 10초 넘게 늦은
//!   `timestamp`는 수신 시각으로 자릅니다. 이벤트는 받은 뒤에 일어날 수 없으므로, 합성 이벤트·셸 버그의
//!   미래 시각 하나가 단조 보정으로 이후 입력 시각을 고정하는 범위를 10초 안으로 묶습니다. 수신 시각보다
//!   이른 값(바쁜 main thread가 늦게 처리한 이벤트)은 그대로 둡니다.
//! - **modifier**: `NSEvent.ModifierFlags`의 device-independent bit(Shift·Control·Option·Command·
//!   Caps Lock)만 [`Modifiers`]로 옮깁니다. numericPad·help·function bit와 하위 16bit
//!   device-dependent bit는 modifier로 쓰지 않습니다(단, `flagsChanged`의 좌우 key 판정에는 사용).
//! - **pointer**: left/right/other mouse down·up·dragged, mouseMoved를 변환합니다. button은 이벤트
//!   종류로 정하고(other mouse만 `buttonNumber` 2=Middle, 3=Back, 4=Forward), 그 밖의 button은
//!   무시합니다. down/up의 click count는 `NSEvent.clickCount`(2=double click)이며 255에서
//!   포화합니다. Control+click도 물리 입력 그대로 Primary + Control입니다. tablet point subtype은
//!   `Pen`, 그 밖(mouse·trackpad)은 `Mouse`입니다.
//! - **wheel**: `hasPreciseScrollingDeltas`(trackpad·Magic Mouse)면 `Point`, 아니면 `Line` 단위입니다.
//!   AppKit `scrollingDelta`는 content 이동 방향이므로 부호를 뒤집어 공통 규칙(scroll 양, x 오른쪽·
//!   y 아래쪽 양수)으로 맞춥니다. `momentumPhase`가 있으면 `Momentum`, `phase`의 began/changed·
//!   stationary/ended·cancelled는 `Began`/`Changed`/`Ended`, 둘 다 없으면 `Discrete`입니다.
//!   `mayBegin`(손가락만 닿고 scroll 전)은 이벤트를 만들지 않습니다.
//! - **key**: keyDown/keyUp은 virtual key code를 HID usage로 바꿉니다. `flagsChanged`는 modifier
//!   key의 down/up으로 바꾸며, 좌우 key는 device-dependent bit로 구분합니다(bit가 없는 합성
//!   이벤트는 device-independent bit로 판정). Caps Lock은 lock이 켜지면 down, 꺼지면 up입니다.
//!   표에 없는 key(예: fn)는 이벤트를 만들지 않습니다.
//! - 위 목록 밖의 이벤트 종류(enter/exit, gesture, tablet proximity 등)는 이벤트를 만들지 않습니다
//!   (`Ok(None)`). trackpad pinch·rotate gesture는 공통 `InputEvent` 집합(XII.3.1)에 없습니다.

use std::fmt;

use stellar_core_types::value::Vec2;

use crate::input::{
    InputEvent, InputEventError, InputEventKind, InputTimeDomain, KeyCode, KeyEvent, KeyState,
    Modifiers, NativeTimestamp, PointerButton, PointerDevice, PointerEvent, PointerPhase,
    WheelDeltaUnit, WheelEvent, WheelPhase,
};

/// [`StellarMacEvent`] 레이아웃 버전. 필드가 바뀌면 올리고 이전 버전은 거부합니다.
pub const MAC_EVENT_VERSION: u32 = 1;

/// `timestamp`가 `receive_uptime`보다 늦어도 그대로 쓰는 최대 초(모듈 문서 "시각").
///
/// 실제 AppKit 이벤트는 받은 뒤에 일어날 수 없어 이 값을 넘지 않습니다. 셸 입력 검사
/// (`InputCheck.swift`)가 합성 이벤트 timestamp를 수신 시각보다 최대 9초 앞서 쓰므로 그보다 크게 둡니다.
/// 미래 시각 하나가 이후 입력 시각을 고정하는 범위는 이 값 이하입니다.
const MAX_TIMESTAMP_AHEAD_OF_RECEIVE_SECONDS: f64 = 10.0;

/// AppKit `NSEvent` 원시 필드(C ABI 고정 구조체, 104 byte).
///
/// 셸은 이벤트 종류에 해당하지 않는 필드를 0으로 채웁니다(AppKit은 종류에 맞지 않는 accessor를
/// 호출하면 예외를 던집니다).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StellarMacEvent {
    /// 레이아웃 버전([`MAC_EVENT_VERSION`]).
    pub version: u32,
    /// `NSEvent.EventType.rawValue`.
    pub event_type: u32,
    /// `NSEvent.keyCode`(keyDown·keyUp·flagsChanged).
    pub key_code: u16,
    /// `NSEvent.subtype.rawValue`(mouse 이벤트).
    pub subtype: i16,
    /// `NSEvent.isARepeat`(keyDown, 0/1).
    pub is_repeat: u8,
    /// `NSEvent.hasPreciseScrollingDeltas`(scrollWheel, 0/1).
    pub has_precise_deltas: u8,
    /// 예약(0이어야 함).
    pub reserved: u16,
    /// `NSEvent.modifierFlags.rawValue`.
    pub modifier_flags: u64,
    /// `NSEvent.timestamp`(시스템 시작 이후 초).
    pub timestamp: f64,
    /// 셸이 이벤트를 받은 시각(`ProcessInfo.systemUptime`, `timestamp`와 같은 축).
    pub receive_uptime: f64,
    /// 캔버스 view 기준 x(논리 point).
    pub x: f64,
    /// 캔버스 view 기준 y(논리 point, 아래로 증가).
    pub y: f64,
    /// `NSEvent.clickCount`(mouse down/up).
    pub click_count: i64,
    /// `NSEvent.buttonNumber`(mouse).
    pub button_number: i64,
    /// `NSEvent.scrollingDeltaX`(scrollWheel).
    pub scrolling_delta_x: f64,
    /// `NSEvent.scrollingDeltaY`(scrollWheel).
    pub scrolling_delta_y: f64,
    /// `NSEvent.phase.rawValue`(scrollWheel).
    pub phase: u64,
    /// `NSEvent.momentumPhase.rawValue`(scrollWheel).
    pub momentum_phase: u64,
}

/// `NSEvent.EventType` raw 값.
pub mod event_type {
    /// `.leftMouseDown`.
    pub const LEFT_MOUSE_DOWN: u32 = 1;
    /// `.leftMouseUp`.
    pub const LEFT_MOUSE_UP: u32 = 2;
    /// `.rightMouseDown`.
    pub const RIGHT_MOUSE_DOWN: u32 = 3;
    /// `.rightMouseUp`.
    pub const RIGHT_MOUSE_UP: u32 = 4;
    /// `.mouseMoved`.
    pub const MOUSE_MOVED: u32 = 5;
    /// `.leftMouseDragged`.
    pub const LEFT_MOUSE_DRAGGED: u32 = 6;
    /// `.rightMouseDragged`.
    pub const RIGHT_MOUSE_DRAGGED: u32 = 7;
    /// `.mouseEntered`.
    pub const MOUSE_ENTERED: u32 = 8;
    /// `.mouseExited`.
    pub const MOUSE_EXITED: u32 = 9;
    /// `.keyDown`.
    pub const KEY_DOWN: u32 = 10;
    /// `.keyUp`.
    pub const KEY_UP: u32 = 11;
    /// `.flagsChanged`.
    pub const FLAGS_CHANGED: u32 = 12;
    /// `.scrollWheel`.
    pub const SCROLL_WHEEL: u32 = 22;
    /// `.otherMouseDown`.
    pub const OTHER_MOUSE_DOWN: u32 = 25;
    /// `.otherMouseUp`.
    pub const OTHER_MOUSE_UP: u32 = 26;
    /// `.otherMouseDragged`.
    pub const OTHER_MOUSE_DRAGGED: u32 = 27;
    /// `.magnify`(trackpad pinch, 변환하지 않음).
    pub const MAGNIFY: u32 = 30;
}

/// `NSEvent.ModifierFlags` bit.
pub mod modifier_flag {
    /// `.capsLock`.
    pub const CAPS_LOCK: u64 = 1 << 16;
    /// `.shift`.
    pub const SHIFT: u64 = 1 << 17;
    /// `.control`.
    pub const CONTROL: u64 = 1 << 18;
    /// `.option`.
    pub const OPTION: u64 = 1 << 19;
    /// `.command`.
    pub const COMMAND: u64 = 1 << 20;
    /// `.numericPad`(modifier로 쓰지 않음).
    pub const NUMERIC_PAD: u64 = 1 << 21;
    /// `.help`(modifier로 쓰지 않음).
    pub const HELP: u64 = 1 << 22;
    /// `.function`(modifier로 쓰지 않음).
    pub const FUNCTION: u64 = 1 << 23;
    /// device-dependent 왼쪽 Control(`NX_DEVICELCTLKEYMASK`).
    pub const DEVICE_LEFT_CONTROL: u64 = 0x0000_0001;
    /// device-dependent 왼쪽 Shift(`NX_DEVICELSHIFTKEYMASK`).
    pub const DEVICE_LEFT_SHIFT: u64 = 0x0000_0002;
    /// device-dependent 오른쪽 Shift(`NX_DEVICERSHIFTKEYMASK`).
    pub const DEVICE_RIGHT_SHIFT: u64 = 0x0000_0004;
    /// device-dependent 왼쪽 Command(`NX_DEVICELCMDKEYMASK`).
    pub const DEVICE_LEFT_COMMAND: u64 = 0x0000_0008;
    /// device-dependent 오른쪽 Command(`NX_DEVICERCMDKEYMASK`).
    pub const DEVICE_RIGHT_COMMAND: u64 = 0x0000_0010;
    /// device-dependent 왼쪽 Option(`NX_DEVICELALTKEYMASK`).
    pub const DEVICE_LEFT_OPTION: u64 = 0x0000_0020;
    /// device-dependent 오른쪽 Option(`NX_DEVICERALTKEYMASK`).
    pub const DEVICE_RIGHT_OPTION: u64 = 0x0000_0040;
    /// device-dependent 오른쪽 Control(`NX_DEVICERCTLKEYMASK`).
    pub const DEVICE_RIGHT_CONTROL: u64 = 0x0000_2000;
}

/// `NSEvent.Phase` bit(scroll gesture의 `phase`와 `momentumPhase`).
pub mod scroll_phase {
    /// `.began`.
    pub const BEGAN: u64 = 0x01;
    /// `.stationary`.
    pub const STATIONARY: u64 = 0x02;
    /// `.changed`.
    pub const CHANGED: u64 = 0x04;
    /// `.ended`.
    pub const ENDED: u64 = 0x08;
    /// `.cancelled`.
    pub const CANCELLED: u64 = 0x10;
    /// `.mayBegin`.
    pub const MAY_BEGIN: u64 = 0x20;
    /// 정의된 bit 전체.
    pub const DEFINED: u64 = 0x3F;
}

/// `NSEvent.EventSubtype.tabletPoint`.
const SUBTYPE_TABLET_POINT: i16 = 1;

/// macOS 이벤트 변환 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacEventError {
    /// 지원하지 않는 [`StellarMacEvent`] 레이아웃 버전.
    UnsupportedVersion(u32),
    /// 예약 필드가 0이 아님.
    ReservedNotZero,
    /// 위치가 유한하지 않거나 `f32` 범위를 넘음.
    NonFiniteCoordinate,
    /// scroll delta가 유한하지 않거나 `f32` 범위를 넘음.
    NonFiniteDelta,
    /// 0/1이어야 하는 필드 값이 잘못됨.
    InvalidBool {
        /// 필드 이름.
        field: &'static str,
        /// 읽은 값.
        value: u8,
    },
    /// mouse down/up의 click count가 1 미만.
    InvalidClickCount(i64),
    /// 정의되지 않은 scroll phase bit.
    UnknownScrollPhase(u64),
    /// 공통 이벤트 생성 규칙 위반(시각 변환 포함).
    Event(InputEventError),
}

impl fmt::Display for MacEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacEventError::UnsupportedVersion(version) => {
                write!(f, "지원하지 않는 macOS 이벤트 구조체 버전입니다: {version}")
            }
            MacEventError::ReservedNotZero => {
                f.write_str("macOS 이벤트 구조체의 예약 필드가 0이 아닙니다")
            }
            MacEventError::NonFiniteCoordinate => {
                f.write_str("이벤트 위치가 유한한 f32 값이 아닙니다")
            }
            MacEventError::NonFiniteDelta => f.write_str("scroll delta가 유한한 f32 값이 아닙니다"),
            MacEventError::InvalidBool { field, value } => {
                write!(f, "{field} 값이 0/1이 아닙니다: {value}")
            }
            MacEventError::InvalidClickCount(count) => {
                write!(
                    f,
                    "mouse down/up의 click count는 1 이상이어야 합니다: {count}"
                )
            }
            MacEventError::UnknownScrollPhase(bits) => {
                write!(f, "정의되지 않은 scroll phase bit입니다: {bits:#x}")
            }
            MacEventError::Event(error) => write!(f, "입력 이벤트 생성 실패: {error}"),
        }
    }
}

impl std::error::Error for MacEventError {}

impl From<InputEventError> for MacEventError {
    fn from(error: InputEventError) -> MacEventError {
        MacEventError::Event(error)
    }
}

/// `NSEvent.ModifierFlags` raw 값을 공통 [`Modifiers`]로 바꿉니다.
pub fn mac_modifiers(flags: u64) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    for (flag, modifier) in [
        (modifier_flag::SHIFT, Modifiers::SHIFT),
        (modifier_flag::CONTROL, Modifiers::CONTROL),
        (modifier_flag::OPTION, Modifiers::ALT),
        (modifier_flag::COMMAND, Modifiers::META),
        (modifier_flag::CAPS_LOCK, Modifiers::CAPS_LOCK),
    ] {
        if flags & flag != 0 {
            modifiers = modifiers | modifier;
        }
    }
    modifiers
}

/// macOS virtual key code(`kVK_*`, ANSI·ISO·JIS 배열)를 HID Keyboard page usage로 바꿉니다.
///
/// 표에 없는 code(`kVK_Function` 등)는 `None`입니다.
pub fn mac_key_code(virtual_key: u16) -> Option<KeyCode> {
    let usage: u16 = match virtual_key {
        0x00 => 0x04, // A
        0x01 => 0x16, // S
        0x02 => 0x07, // D
        0x03 => 0x09, // F
        0x04 => 0x0B, // H
        0x05 => 0x0A, // G
        0x06 => 0x1D, // Z
        0x07 => 0x1B, // X
        0x08 => 0x06, // C
        0x09 => 0x19, // V
        0x0A => 0x64, // ISO Section → Non-US \ |
        0x0B => 0x05, // B
        0x0C => 0x14, // Q
        0x0D => 0x1A, // W
        0x0E => 0x08, // E
        0x0F => 0x15, // R
        0x10 => 0x1C, // Y
        0x11 => 0x17, // T
        0x12 => 0x1E, // 1
        0x13 => 0x1F, // 2
        0x14 => 0x20, // 3
        0x15 => 0x21, // 4
        0x16 => 0x23, // 6
        0x17 => 0x22, // 5
        0x18 => 0x2E, // = +
        0x19 => 0x26, // 9
        0x1A => 0x24, // 7
        0x1B => 0x2D, // - _
        0x1C => 0x25, // 8
        0x1D => 0x27, // 0
        0x1E => 0x30, // ]
        0x1F => 0x12, // O
        0x20 => 0x18, // U
        0x21 => 0x2F, // [
        0x22 => 0x0C, // I
        0x23 => 0x13, // P
        0x24 => 0x28, // Return
        0x25 => 0x0F, // L
        0x26 => 0x0D, // J
        0x27 => 0x34, // ' "
        0x28 => 0x0E, // K
        0x29 => 0x33, // ; :
        0x2A => 0x31, // \ |
        0x2B => 0x36, // , <
        0x2C => 0x38, // / ?
        0x2D => 0x11, // N
        0x2E => 0x10, // M
        0x2F => 0x37, // . >
        0x30 => 0x2B, // Tab
        0x31 => 0x2C, // Space
        0x32 => 0x35, // ` ~
        0x33 => 0x2A, // Delete(Backspace)
        0x35 => 0x29, // Escape
        0x36 => 0xE7, // Right Command
        0x37 => 0xE3, // Command
        0x38 => 0xE1, // Shift
        0x39 => 0x39, // Caps Lock
        0x3A => 0xE2, // Option
        0x3B => 0xE0, // Control
        0x3C => 0xE5, // Right Shift
        0x3D => 0xE6, // Right Option
        0x3E => 0xE4, // Right Control
        0x40 => 0x6C, // F17
        0x41 => 0x63, // Keypad .
        0x43 => 0x55, // Keypad *
        0x45 => 0x57, // Keypad +
        0x47 => 0x53, // Keypad Clear(Num Lock)
        0x48 => 0x80, // Volume Up
        0x49 => 0x81, // Volume Down
        0x4A => 0x7F, // Mute
        0x4B => 0x54, // Keypad /
        0x4C => 0x58, // Keypad Enter
        0x4E => 0x56, // Keypad -
        0x4F => 0x6D, // F18
        0x50 => 0x6E, // F19
        0x51 => 0x67, // Keypad =
        0x52 => 0x62, // Keypad 0
        0x53 => 0x59, // Keypad 1
        0x54 => 0x5A, // Keypad 2
        0x55 => 0x5B, // Keypad 3
        0x56 => 0x5C, // Keypad 4
        0x57 => 0x5D, // Keypad 5
        0x58 => 0x5E, // Keypad 6
        0x59 => 0x5F, // Keypad 7
        0x5A => 0x6F, // F20
        0x5B => 0x60, // Keypad 8
        0x5C => 0x61, // Keypad 9
        0x5D => 0x89, // JIS Yen → International3
        0x5E => 0x87, // JIS Underscore → International1
        0x5F => 0x85, // JIS Keypad Comma
        0x60 => 0x3E, // F5
        0x61 => 0x3F, // F6
        0x62 => 0x40, // F7
        0x63 => 0x3C, // F3
        0x64 => 0x41, // F8
        0x65 => 0x42, // F9
        0x66 => 0x91, // JIS Eisu → LANG2
        0x67 => 0x44, // F11
        0x68 => 0x90, // JIS Kana → LANG1
        0x69 => 0x68, // F13
        0x6A => 0x6B, // F16
        0x6B => 0x69, // F14
        0x6D => 0x43, // F10
        0x6E => 0x65, // Contextual Menu → Application
        0x6F => 0x45, // F12
        0x71 => 0x6A, // F15
        0x72 => 0x49, // Help → Insert
        0x73 => 0x4A, // Home
        0x74 => 0x4B, // Page Up
        0x75 => 0x4C, // Forward Delete
        0x76 => 0x3D, // F4
        0x77 => 0x4D, // End
        0x78 => 0x3B, // F2
        0x79 => 0x4E, // Page Down
        0x7A => 0x3A, // F1
        0x7B => 0x50, // Left Arrow
        0x7C => 0x4F, // Right Arrow
        0x7D => 0x51, // Down Arrow
        0x7E => 0x52, // Up Arrow
        _ => return None,
    };
    // 표의 usage는 모두 0이 아니므로 항상 성공합니다.
    KeyCode::new(usage).ok()
}

/// `flagsChanged` modifier key의 현재 down 여부를 판정합니다. modifier key가 아니면 `None`.
fn modifier_key_down(virtual_key: u16, flags: u64) -> Option<bool> {
    use modifier_flag as m;
    // (이 key의 device bit, 같은 종류 반대편 device bit, device-independent bit)
    let (own, other, independent) = match virtual_key {
        0x38 => (m::DEVICE_LEFT_SHIFT, m::DEVICE_RIGHT_SHIFT, m::SHIFT),
        0x3C => (m::DEVICE_RIGHT_SHIFT, m::DEVICE_LEFT_SHIFT, m::SHIFT),
        0x3B => (m::DEVICE_LEFT_CONTROL, m::DEVICE_RIGHT_CONTROL, m::CONTROL),
        0x3E => (m::DEVICE_RIGHT_CONTROL, m::DEVICE_LEFT_CONTROL, m::CONTROL),
        0x3A => (m::DEVICE_LEFT_OPTION, m::DEVICE_RIGHT_OPTION, m::OPTION),
        0x3D => (m::DEVICE_RIGHT_OPTION, m::DEVICE_LEFT_OPTION, m::OPTION),
        0x37 => (m::DEVICE_LEFT_COMMAND, m::DEVICE_RIGHT_COMMAND, m::COMMAND),
        0x36 => (m::DEVICE_RIGHT_COMMAND, m::DEVICE_LEFT_COMMAND, m::COMMAND),
        // Caps Lock: lock 상태 bit로 down/up을 정합니다.
        0x39 => return Some(flags & m::CAPS_LOCK != 0),
        _ => return None,
    };
    if flags & (own | other) != 0 {
        // device-dependent bit가 있으면 좌우를 정확히 구분합니다.
        Some(flags & own != 0)
    } else {
        // device bit가 없는 합성 이벤트: device-independent bit로 판정합니다.
        Some(flags & independent != 0)
    }
}

fn bool_field(field: &'static str, value: u8) -> Result<bool, MacEventError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(MacEventError::InvalidBool {
            field,
            value: other,
        }),
    }
}

/// `f64`를 `f32`로 IEEE 반올림 변환하고 유한한지 확인합니다.
fn finite_f32(value: f64) -> Option<f32> {
    let narrowed = value as f32;
    narrowed.is_finite().then_some(narrowed)
}

fn position(event: &StellarMacEvent) -> Result<Vec2, MacEventError> {
    let x = finite_f32(event.x).ok_or(MacEventError::NonFiniteCoordinate)?;
    let y = finite_f32(event.y).ok_or(MacEventError::NonFiniteCoordinate)?;
    Vec2::new(x, y).map_err(|_| MacEventError::NonFiniteCoordinate)
}

/// AppKit scroll delta(content 이동 방향)를 공통 scroll 양(x 오른쪽·y 아래쪽 양수)으로 바꿉니다.
fn scroll_delta(event: &StellarMacEvent) -> Result<Vec2, MacEventError> {
    let dx = finite_f32(event.scrolling_delta_x).ok_or(MacEventError::NonFiniteDelta)?;
    let dy = finite_f32(event.scrolling_delta_y).ok_or(MacEventError::NonFiniteDelta)?;
    // `-0.0`은 Vec2가 `0.0`으로 정규화합니다.
    Vec2::new(-dx, -dy).map_err(|_| MacEventError::NonFiniteDelta)
}

fn wheel_phase(phase: u64, momentum: u64) -> Result<Option<WheelPhase>, MacEventError> {
    use scroll_phase as p;
    if phase & !p::DEFINED != 0 {
        return Err(MacEventError::UnknownScrollPhase(phase));
    }
    if momentum & !p::DEFINED != 0 {
        return Err(MacEventError::UnknownScrollPhase(momentum));
    }
    if momentum != 0 {
        return Ok(Some(WheelPhase::Momentum));
    }
    if phase == 0 {
        return Ok(Some(WheelPhase::Discrete));
    }
    if phase & p::BEGAN != 0 {
        return Ok(Some(WheelPhase::Began));
    }
    if phase & (p::CHANGED | p::STATIONARY) != 0 {
        return Ok(Some(WheelPhase::Changed));
    }
    if phase & (p::ENDED | p::CANCELLED) != 0 {
        return Ok(Some(WheelPhase::Ended));
    }
    // mayBegin만 있는 경우: 아직 scroll이 없습니다.
    Ok(None)
}

fn pointer_kind(event: &StellarMacEvent) -> Result<Option<InputEventKind>, MacEventError> {
    use event_type as t;
    let other_button = || match event.button_number {
        2 => Some(PointerButton::Middle),
        3 => Some(PointerButton::Back),
        4 => Some(PointerButton::Forward),
        _ => None,
    };
    let (phase, button) = match event.event_type {
        t::LEFT_MOUSE_DOWN => (PointerPhase::Down, Some(PointerButton::Primary)),
        t::LEFT_MOUSE_UP => (PointerPhase::Up, Some(PointerButton::Primary)),
        t::RIGHT_MOUSE_DOWN => (PointerPhase::Down, Some(PointerButton::Secondary)),
        t::RIGHT_MOUSE_UP => (PointerPhase::Up, Some(PointerButton::Secondary)),
        t::OTHER_MOUSE_DOWN => (PointerPhase::Down, other_button()),
        t::OTHER_MOUSE_UP => (PointerPhase::Up, other_button()),
        t::MOUSE_MOVED => (PointerPhase::Move, Some(PointerButton::None)),
        t::LEFT_MOUSE_DRAGGED => (PointerPhase::Move, Some(PointerButton::Primary)),
        t::RIGHT_MOUSE_DRAGGED => (PointerPhase::Move, Some(PointerButton::Secondary)),
        t::OTHER_MOUSE_DRAGGED => (PointerPhase::Move, other_button()),
        _ => return Ok(None),
    };
    // 공통 집합에 없는 button(6번째 이후)은 무시합니다.
    let Some(button) = button else {
        return Ok(None);
    };
    let click_count = match phase {
        PointerPhase::Down | PointerPhase::Up => {
            if event.click_count < 1 {
                return Err(MacEventError::InvalidClickCount(event.click_count));
            }
            u8::try_from(event.click_count).unwrap_or(u8::MAX)
        }
        PointerPhase::Move | PointerPhase::Cancel => 0,
    };
    let device = if event.subtype == SUBTYPE_TABLET_POINT {
        PointerDevice::Pen
    } else {
        PointerDevice::Mouse
    };
    Ok(Some(InputEventKind::Pointer(PointerEvent {
        phase,
        button,
        device,
        position: position(event)?,
        modifiers: mac_modifiers(event.modifier_flags),
        click_count,
    })))
}

fn key_kind(event: &StellarMacEvent) -> Result<Option<InputEventKind>, MacEventError> {
    let modifiers = mac_modifiers(event.modifier_flags);
    let (state, repeat) = match event.event_type {
        event_type::KEY_DOWN => (KeyState::Down, bool_field("is_repeat", event.is_repeat)?),
        // keyUp은 반복이 없습니다.
        event_type::KEY_UP => (KeyState::Up, false),
        event_type::FLAGS_CHANGED => {
            let Some(down) = modifier_key_down(event.key_code, event.modifier_flags) else {
                return Ok(None);
            };
            let state = if down { KeyState::Down } else { KeyState::Up };
            (state, false)
        }
        _ => return Ok(None),
    };
    let Some(key) = mac_key_code(event.key_code) else {
        return Ok(None);
    };
    Ok(Some(InputEventKind::Key(KeyEvent {
        state,
        key,
        repeat,
        modifiers,
    })))
}

fn wheel_kind(event: &StellarMacEvent) -> Result<Option<InputEventKind>, MacEventError> {
    let Some(phase) = wheel_phase(event.phase, event.momentum_phase)? else {
        return Ok(None);
    };
    let unit = if bool_field("has_precise_deltas", event.has_precise_deltas)? {
        WheelDeltaUnit::Point
    } else {
        WheelDeltaUnit::Line
    };
    Ok(Some(InputEventKind::Wheel(WheelEvent {
        phase,
        unit,
        position: position(event)?,
        delta: scroll_delta(event)?,
        modifiers: mac_modifiers(event.modifier_flags),
    })))
}

/// AppKit 이벤트 원시 필드를 공통 [`InputEvent`]로 변환합니다.
///
/// 공통 집합에 없는 이벤트·button·key는 `Ok(None)`입니다. 시각은 `domain`으로 코어 단조 domain에
/// 옮깁니다(변환한 이벤트만 domain의 단조 상태를 갱신).
pub fn convert_mac_event(
    event: &StellarMacEvent,
    domain: &mut InputTimeDomain,
) -> Result<Option<InputEvent>, MacEventError> {
    if event.version != MAC_EVENT_VERSION {
        return Err(MacEventError::UnsupportedVersion(event.version));
    }
    if event.reserved != 0 {
        return Err(MacEventError::ReservedNotZero);
    }
    let kind = match event.event_type {
        event_type::KEY_DOWN | event_type::KEY_UP | event_type::FLAGS_CHANGED => key_kind(event)?,
        event_type::SCROLL_WHEEL => wheel_kind(event)?,
        _ => pointer_kind(event)?,
    };
    let Some(kind) = kind else {
        return Ok(None);
    };
    // timestamp와 수신 시각은 모두 유한하고 음이 아니며 nanosecond u64 범위여야 합니다.
    NativeTimestamp::MacEventSeconds(event.timestamp).to_native_nanos()?;
    NativeTimestamp::MacEventSeconds(event.receive_uptime).to_native_nanos()?;
    // 이벤트는 받은 뒤에 일어날 수 없으므로 수신 시각보다 허용 오차를 넘게 늦은 timestamp는 수신
    // 시각으로 자릅니다. 합성 이벤트·셸 버그의 미래 시각 하나가 domain의 단조 상태를 끌어올려 이후
    // 입력 시각을 고정하는 범위를 허용 오차 안으로 묶습니다.
    let seconds = if event.timestamp - event.receive_uptime > MAX_TIMESTAMP_AHEAD_OF_RECEIVE_SECONDS
    {
        event.receive_uptime
    } else {
        event.timestamp
    };
    // domain은 시각 변환이 성공할 때만 단조 상태를 갱신합니다.
    let timestamp = domain.convert(NativeTimestamp::MacEventSeconds(seconds))?;
    Ok(Some(InputEvent::new(timestamp, kind)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::MonotonicNanos;
    use event_type as t;
    use modifier_flag as m;

    /// macOS 입력 변환 golden fixture(이름 → 직렬화 hex).
    const GOLDEN: &str = include_str!("../tests/fixtures/mac_events_v1.golden");

    /// 코어 시계 기준점과 macOS 기준 시각(셸 수신 시각).
    const CORE_ANCHOR: u64 = 1_000_000_000;
    const MAC_ANCHOR_SECONDS: f64 = 100.0;

    fn domain() -> InputTimeDomain {
        InputTimeDomain::from_anchor_nanos(100_000_000_000, CORE_ANCHOR)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn base(event_type: u32, after_seconds: f64) -> StellarMacEvent {
        StellarMacEvent {
            version: MAC_EVENT_VERSION,
            event_type,
            timestamp: MAC_ANCHOR_SECONDS + after_seconds,
            receive_uptime: MAC_ANCHOR_SECONDS + after_seconds,
            ..StellarMacEvent::default()
        }
    }

    fn mouse(
        event_type: u32,
        after: f64,
        xy: (f64, f64),
        flags: u64,
        clicks: i64,
    ) -> StellarMacEvent {
        StellarMacEvent {
            x: xy.0,
            y: xy.1,
            modifier_flags: flags,
            click_count: clicks,
            ..base(event_type, after)
        }
    }

    fn key(event_type: u32, after: f64, code: u16, flags: u64, repeat: u8) -> StellarMacEvent {
        StellarMacEvent {
            key_code: code,
            modifier_flags: flags,
            is_repeat: repeat,
            ..base(event_type, after)
        }
    }

    fn scroll(
        after: f64,
        delta: (f64, f64),
        precise: u8,
        phase: u64,
        momentum: u64,
    ) -> StellarMacEvent {
        StellarMacEvent {
            x: 40.0,
            y: 30.0,
            scrolling_delta_x: delta.0,
            scrolling_delta_y: delta.1,
            has_precise_deltas: precise,
            phase,
            momentum_phase: momentum,
            ..base(t::SCROLL_WHEEL, after)
        }
    }

    fn convert_one(event: &StellarMacEvent) -> InputEvent {
        convert_mac_event(event, &mut domain())
            .expect("변환 성공")
            .expect("이벤트 생성")
    }

    fn pointer_of(event: &InputEvent) -> PointerEvent {
        match event.kind() {
            InputEventKind::Pointer(pointer) => *pointer,
            other => panic!("pointer가 아님: {other:?}"),
        }
    }

    fn key_of(event: &InputEvent) -> KeyEvent {
        match event.kind() {
            InputEventKind::Key(key) => *key,
            other => panic!("key가 아님: {other:?}"),
        }
    }

    fn wheel_of(event: &InputEvent) -> WheelEvent {
        match event.kind() {
            InputEventKind::Wheel(wheel) => *wheel,
            other => panic!("wheel이 아님: {other:?}"),
        }
    }

    /// golden fixture의 원시 macOS 이벤트 열(이름, 원시 이벤트).
    fn fixture_events() -> Vec<(&'static str, StellarMacEvent)> {
        vec![
            (
                "left_down_single",
                mouse(t::LEFT_MOUSE_DOWN, 0.125, (120.5, 64.25), 0, 1),
            ),
            (
                "left_up_single",
                mouse(t::LEFT_MOUSE_UP, 0.25, (120.5, 64.25), 0, 1),
            ),
            (
                "left_down_double_shift_command",
                mouse(
                    t::LEFT_MOUSE_DOWN,
                    0.375,
                    (120.5, 64.25),
                    m::SHIFT | m::COMMAND | m::DEVICE_LEFT_SHIFT | m::DEVICE_LEFT_COMMAND,
                    2,
                ),
            ),
            (
                "left_up_double_shift_command",
                mouse(
                    t::LEFT_MOUSE_UP,
                    0.5,
                    (120.5, 64.25),
                    m::SHIFT | m::COMMAND,
                    2,
                ),
            ),
            (
                "left_dragged",
                mouse(t::LEFT_MOUSE_DRAGGED, 0.625, (130.0, 70.0), 0, 0),
            ),
            (
                "mouse_moved_outside_view",
                mouse(t::MOUSE_MOVED, 0.75, (-5.5, -2.0), 0, 0),
            ),
            (
                "right_down_control_option_caps",
                mouse(
                    t::RIGHT_MOUSE_DOWN,
                    0.875,
                    (10.0, 20.0),
                    m::CONTROL | m::OPTION | m::CAPS_LOCK | m::NUMERIC_PAD | m::FUNCTION,
                    1,
                ),
            ),
            (
                "key_down_a_command",
                key(t::KEY_DOWN, 1.0, 0x00, m::COMMAND, 0),
            ),
            ("key_down_a_repeat", key(t::KEY_DOWN, 1.125, 0x00, 0, 1)),
            ("key_up_a", key(t::KEY_UP, 1.25, 0x00, 0, 0)),
            (
                "flags_left_shift_down",
                key(
                    t::FLAGS_CHANGED,
                    1.375,
                    0x38,
                    m::SHIFT | m::DEVICE_LEFT_SHIFT,
                    0,
                ),
            ),
            (
                "flags_left_shift_up",
                key(t::FLAGS_CHANGED, 1.5, 0x38, 0, 0),
            ),
            (
                "scroll_trackpad_began",
                scroll(1.625, (3.0, -5.0), 1, scroll_phase::BEGAN, 0),
            ),
            (
                "scroll_trackpad_momentum",
                scroll(1.75, (0.0, -2.5), 1, 0, scroll_phase::CHANGED),
            ),
            ("scroll_wheel_line", scroll(1.875, (0.0, 2.0), 0, 0, 0)),
        ]
    }

    fn golden() -> Vec<(String, String)> {
        GOLDEN
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next().expect("name").to_string();
                let bytes = parts.next().expect("hex").to_string();
                assert!(parts.next().is_none(), "fixture 줄 형식: {line}");
                (name, bytes)
            })
            .collect()
    }

    #[test]
    fn single_click_matches_cross_platform_golden() {
        // macOS 원시 이벤트에서 만든 single click down이 XII.3.1 양 플랫폼 golden과 같습니다.
        let cross_platform = include_str!("../tests/fixtures/input_events_v1.golden")
            .lines()
            .find_map(|line| line.strip_prefix("pointer_down_primary "))
            .expect("pointer_down_primary");
        let event = convert_mac_event(
            &mouse(t::LEFT_MOUSE_DOWN, 0.125, (120.5, 64.25), 0, 1),
            &mut domain(),
        )
        .expect("변환")
        .expect("이벤트");
        assert_eq!(hex(&event.encode()), cross_platform.trim());
    }

    #[test]
    fn mac_event_sequence_matches_golden_fixture() {
        // 한 domain으로 순서대로 변환한 결과가 golden 바이트와 일치해야 합니다.
        let golden = golden();
        let events = fixture_events();
        assert_eq!(golden.len(), events.len(), "fixture 항목 수");
        let mut domain = domain();
        for ((name, raw), (golden_name, expected)) in events.iter().zip(&golden) {
            assert_eq!(name, golden_name, "fixture 순서");
            let event = convert_mac_event(raw, &mut domain)
                .expect("변환")
                .expect("이벤트");
            assert_eq!(&hex(&event.encode()), expected, "{name} bytes");
        }
    }

    #[test]
    fn coordinates_are_view_points_with_f32_rounding() {
        // 좌상단 원점 view point가 그대로 들어가고, view 밖(drag 중) 음수 좌표도 유지됩니다.
        let down = convert_one(&mouse(t::LEFT_MOUSE_DOWN, 0.0, (120.5, 64.25), 0, 1));
        assert_eq!(pointer_of(&down).position, Vec2::new(120.5, 64.25).unwrap());
        let outside = convert_one(&mouse(t::MOUSE_MOVED, 0.0, (-5.5, 1e4), 0, 0));
        assert_eq!(pointer_of(&outside).position, Vec2::new(-5.5, 1e4).unwrap());
        // f64 → f32는 IEEE 반올림입니다(0.1f64 → 0.1f32).
        let rounded = convert_one(&mouse(t::MOUSE_MOVED, 0.0, (0.1, 1.0 / 3.0), 0, 0));
        assert_eq!(
            pointer_of(&rounded).position,
            Vec2::new(0.1f32, 1.0f32 / 3.0f32).unwrap()
        );
        // -0.0은 0.0으로 정규화되어 바이트가 결정적입니다.
        let negative_zero = convert_one(&mouse(t::MOUSE_MOVED, 0.0, (-0.0, -0.0), 0, 0));
        assert_eq!(pointer_of(&negative_zero).position, Vec2::ZERO);
        assert_eq!(
            negative_zero.encode()[15..23],
            [0u8; 8],
            "-0.0 위치는 +0.0 바이트"
        );
        // 비유한·f32 범위 밖 좌표는 거부합니다.
        for bad in [f64::NAN, f64::INFINITY, 1e300] {
            assert_eq!(
                convert_mac_event(&mouse(t::MOUSE_MOVED, 0.0, (bad, 0.0), 0, 0), &mut domain()),
                Err(MacEventError::NonFiniteCoordinate)
            );
        }
    }

    #[test]
    fn modifier_flags_map_to_physical_modifiers() {
        assert_eq!(mac_modifiers(0), Modifiers::NONE);
        assert_eq!(mac_modifiers(m::SHIFT), Modifiers::SHIFT);
        assert_eq!(mac_modifiers(m::CONTROL), Modifiers::CONTROL);
        assert_eq!(mac_modifiers(m::OPTION), Modifiers::ALT);
        assert_eq!(mac_modifiers(m::COMMAND), Modifiers::META);
        assert_eq!(mac_modifiers(m::CAPS_LOCK), Modifiers::CAPS_LOCK);
        assert_eq!(
            mac_modifiers(m::SHIFT | m::CONTROL | m::OPTION | m::COMMAND | m::CAPS_LOCK).bits(),
            0b1_1111
        );
        // numericPad·help·function·device-dependent bit는 modifier가 아닙니다.
        assert_eq!(
            mac_modifiers(
                m::NUMERIC_PAD
                    | m::HELP
                    | m::FUNCTION
                    | m::DEVICE_LEFT_SHIFT
                    | m::DEVICE_RIGHT_CONTROL
                    | 0xFF
            ),
            Modifiers::NONE
        );
        // 이벤트 종류와 무관하게 같은 규칙입니다.
        let flags = m::SHIFT | m::COMMAND | m::DEVICE_LEFT_SHIFT;
        let expected = Modifiers::SHIFT | Modifiers::META;
        assert_eq!(
            pointer_of(&convert_one(&mouse(
                t::LEFT_MOUSE_DOWN,
                0.0,
                (0.0, 0.0),
                flags,
                1
            )))
            .modifiers,
            expected
        );
        assert_eq!(
            key_of(&convert_one(&key(t::KEY_DOWN, 0.0, 0x00, flags, 0))).modifiers,
            expected
        );
        assert_eq!(
            wheel_of(&convert_one(&StellarMacEvent {
                modifier_flags: flags,
                ..scroll(0.0, (0.0, 1.0), 0, 0, 0)
            }))
            .modifiers,
            expected
        );
    }

    #[test]
    fn click_counts_follow_appkit_click_count() {
        // single → double → triple: down/up 모두 NSEvent.clickCount를 그대로 씁니다.
        for clicks in [1i64, 2, 3] {
            for event_type in [t::LEFT_MOUSE_DOWN, t::LEFT_MOUSE_UP] {
                let event = convert_one(&mouse(event_type, 0.0, (1.0, 1.0), 0, clicks));
                assert_eq!(i64::from(pointer_of(&event).click_count), clicks);
            }
        }
        // 255 초과는 포화합니다.
        let many = convert_one(&mouse(t::LEFT_MOUSE_DOWN, 0.0, (1.0, 1.0), 0, 1_000));
        assert_eq!(pointer_of(&many).click_count, u8::MAX);
        // drag·move는 click count 0(AppKit 값 무시).
        let drag = convert_one(&mouse(t::LEFT_MOUSE_DRAGGED, 0.0, (1.0, 1.0), 0, 2));
        assert_eq!(pointer_of(&drag).click_count, 0);
        // down/up의 0·음수 click count는 거부합니다.
        for bad in [0i64, -1] {
            assert_eq!(
                convert_mac_event(
                    &mouse(t::RIGHT_MOUSE_UP, 0.0, (1.0, 1.0), 0, bad),
                    &mut domain()
                ),
                Err(MacEventError::InvalidClickCount(bad))
            );
        }
    }

    #[test]
    fn mouse_event_types_map_to_phase_and_button() {
        let cases = [
            (
                t::LEFT_MOUSE_DOWN,
                0,
                PointerPhase::Down,
                PointerButton::Primary,
            ),
            (
                t::LEFT_MOUSE_UP,
                0,
                PointerPhase::Up,
                PointerButton::Primary,
            ),
            (
                t::RIGHT_MOUSE_DOWN,
                1,
                PointerPhase::Down,
                PointerButton::Secondary,
            ),
            (
                t::RIGHT_MOUSE_UP,
                1,
                PointerPhase::Up,
                PointerButton::Secondary,
            ),
            (
                t::OTHER_MOUSE_DOWN,
                2,
                PointerPhase::Down,
                PointerButton::Middle,
            ),
            (t::OTHER_MOUSE_UP, 3, PointerPhase::Up, PointerButton::Back),
            (
                t::OTHER_MOUSE_DOWN,
                4,
                PointerPhase::Down,
                PointerButton::Forward,
            ),
            (t::MOUSE_MOVED, 0, PointerPhase::Move, PointerButton::None),
            (
                t::LEFT_MOUSE_DRAGGED,
                0,
                PointerPhase::Move,
                PointerButton::Primary,
            ),
            (
                t::RIGHT_MOUSE_DRAGGED,
                1,
                PointerPhase::Move,
                PointerButton::Secondary,
            ),
            (
                t::OTHER_MOUSE_DRAGGED,
                2,
                PointerPhase::Move,
                PointerButton::Middle,
            ),
        ];
        for (event_type, button_number, phase, button) in cases {
            let raw = StellarMacEvent {
                button_number,
                ..mouse(event_type, 0.0, (1.0, 2.0), 0, 1)
            };
            let pointer = pointer_of(&convert_one(&raw));
            assert_eq!(
                (pointer.phase, pointer.button),
                (phase, button),
                "{event_type}"
            );
            assert_eq!(pointer.device, PointerDevice::Mouse);
        }
        // tablet point subtype은 Pen입니다.
        let pen = StellarMacEvent {
            subtype: SUBTYPE_TABLET_POINT,
            ..mouse(t::LEFT_MOUSE_DOWN, 0.0, (1.0, 2.0), 0, 1)
        };
        assert_eq!(pointer_of(&convert_one(&pen)).device, PointerDevice::Pen);
        // 공통 집합 밖 button(5번 이후)과 enter/exit·gesture는 이벤트를 만들지 않습니다.
        let extra = StellarMacEvent {
            button_number: 5,
            ..mouse(t::OTHER_MOUSE_DOWN, 0.0, (1.0, 2.0), 0, 1)
        };
        assert_eq!(convert_mac_event(&extra, &mut domain()), Ok(None));
        for ignored in [t::MOUSE_ENTERED, t::MOUSE_EXITED, t::MAGNIFY, 99] {
            assert_eq!(
                convert_mac_event(&mouse(ignored, 0.0, (1.0, 2.0), 0, 1), &mut domain()),
                Ok(None),
                "{ignored}"
            );
        }
    }

    #[test]
    fn virtual_key_codes_map_to_hid_usage() {
        // Carbon kVK_* → HID usage 대표값(문자·숫자·기호·편집·방향·기능·keypad·modifier·JIS).
        let cases: [(u16, u16); 24] = [
            (0x00, 0x04), // A
            (0x06, 0x1D), // Z
            (0x1D, 0x27), // 0
            (0x12, 0x1E), // 1
            (0x24, 0x28), // Return
            (0x30, 0x2B), // Tab
            (0x31, 0x2C), // Space
            (0x33, 0x2A), // Delete
            (0x35, 0x29), // Escape
            (0x75, 0x4C), // Forward Delete
            (0x7B, 0x50), // Left
            (0x7C, 0x4F), // Right
            (0x7D, 0x51), // Down
            (0x7E, 0x52), // Up
            (0x7A, 0x3A), // F1
            (0x6F, 0x45), // F12
            (0x5A, 0x6F), // F20
            (0x52, 0x62), // Keypad 0
            (0x4C, 0x58), // Keypad Enter
            (0x37, 0xE3), // Command
            (0x36, 0xE7), // Right Command
            (0x0A, 0x64), // ISO Section
            (0x68, 0x90), // JIS Kana(LANG1)
            (0x66, 0x91), // JIS Eisu(LANG2)
        ];
        for (virtual_key, usage) in cases {
            assert_eq!(
                mac_key_code(virtual_key).map(|key| key.usage()),
                Some(usage),
                "kVK {virtual_key:#04x}"
            );
        }
        // fn·미정의 code는 None입니다.
        for unmapped in [0x34u16, 0x3F, 0x42, 0x7F, 0xFFFF] {
            assert_eq!(mac_key_code(unmapped), None, "{unmapped:#x}");
        }
        // 표는 단사(서로 다른 code가 같은 usage로 겹치지 않음)입니다.
        let mut usages: Vec<u16> = (0..=0x7Fu16)
            .filter_map(mac_key_code)
            .map(|key| key.usage())
            .collect();
        let total = usages.len();
        usages.sort_unstable();
        usages.dedup();
        assert_eq!(usages.len(), total);
        assert_eq!(total, 119);
    }

    #[test]
    fn key_events_carry_state_repeat_and_modifiers() {
        let down = key_of(&convert_one(&key(t::KEY_DOWN, 0.0, 0x00, m::COMMAND, 0)));
        assert_eq!(
            (down.state, down.repeat, down.modifiers, down.key.usage()),
            (KeyState::Down, false, Modifiers::META, 0x04)
        );
        let repeat = key_of(&convert_one(&key(t::KEY_DOWN, 0.0, 0x00, 0, 1)));
        assert!(repeat.repeat);
        // keyUp의 isARepeat는 무시합니다.
        let up = key_of(&convert_one(&key(t::KEY_UP, 0.0, 0x00, 0, 1)));
        assert_eq!((up.state, up.repeat), (KeyState::Up, false));
        // 0/1 외 repeat 값과 표에 없는 key.
        assert_eq!(
            convert_mac_event(&key(t::KEY_DOWN, 0.0, 0x00, 0, 2), &mut domain()),
            Err(MacEventError::InvalidBool {
                field: "is_repeat",
                value: 2
            })
        );
        assert_eq!(
            convert_mac_event(&key(t::KEY_DOWN, 0.0, 0x3F, 0, 0), &mut domain()),
            Ok(None)
        );
    }

    #[test]
    fn flags_changed_distinguishes_left_and_right_modifier_keys() {
        let state = |code: u16, flags: u64| {
            key_of(&convert_one(&key(t::FLAGS_CHANGED, 0.0, code, flags, 0)))
        };
        // 왼쪽 Shift down → 오른쪽 Shift down → 오른쪽 up(왼쪽은 아직 눌림) → 왼쪽 up.
        let left_down = state(0x38, m::SHIFT | m::DEVICE_LEFT_SHIFT);
        assert_eq!(
            (left_down.state, left_down.key.usage()),
            (KeyState::Down, 0xE1)
        );
        let right_down = state(
            0x3C,
            m::SHIFT | m::DEVICE_LEFT_SHIFT | m::DEVICE_RIGHT_SHIFT,
        );
        assert_eq!(
            (right_down.state, right_down.key.usage()),
            (KeyState::Down, 0xE5)
        );
        let right_up = state(0x3C, m::SHIFT | m::DEVICE_LEFT_SHIFT);
        assert_eq!((right_up.state, right_up.key.usage()), (KeyState::Up, 0xE5));
        assert_eq!(right_up.modifiers, Modifiers::SHIFT);
        let left_up = state(0x38, 0);
        assert_eq!(
            (left_up.state, left_up.modifiers),
            (KeyState::Up, Modifiers::NONE)
        );
        // 나머지 modifier key(좌우)와 device bit 없는 합성 이벤트.
        for (code, device, independent, usage) in [
            (0x3B, m::DEVICE_LEFT_CONTROL, m::CONTROL, 0xE0),
            (0x3E, m::DEVICE_RIGHT_CONTROL, m::CONTROL, 0xE4),
            (0x3A, m::DEVICE_LEFT_OPTION, m::OPTION, 0xE2),
            (0x3D, m::DEVICE_RIGHT_OPTION, m::OPTION, 0xE6),
            (0x37, m::DEVICE_LEFT_COMMAND, m::COMMAND, 0xE3),
            (0x36, m::DEVICE_RIGHT_COMMAND, m::COMMAND, 0xE7),
        ] {
            assert_eq!(state(code, independent | device).state, KeyState::Down);
            assert_eq!(state(code, independent).state, KeyState::Down, "합성");
            assert_eq!(state(code, 0).state, KeyState::Up);
            assert_eq!(state(code, 0).key.usage(), usage);
        }
        // Caps Lock: lock on → down, off → up.
        assert_eq!(state(0x39, m::CAPS_LOCK).state, KeyState::Down);
        assert_eq!(state(0x39, 0).state, KeyState::Up);
        // fn과 modifier가 아닌 key의 flagsChanged는 이벤트가 없습니다.
        for code in [0x3Fu16, 0x00] {
            assert_eq!(
                convert_mac_event(
                    &key(t::FLAGS_CHANGED, 0.0, code, m::FUNCTION, 0),
                    &mut domain()
                ),
                Ok(None)
            );
        }
    }

    #[test]
    fn scroll_units_signs_and_phases() {
        // trackpad(precise): point 단위, content 방향 delta를 scroll 양으로 반전.
        let began = wheel_of(&convert_one(&scroll(
            0.0,
            (3.0, -5.0),
            1,
            scroll_phase::BEGAN,
            0,
        )));
        assert_eq!(
            (began.unit, began.phase, began.delta),
            (
                WheelDeltaUnit::Point,
                WheelPhase::Began,
                Vec2::new(-3.0, 5.0).unwrap()
            )
        );
        assert_eq!(began.position, Vec2::new(40.0, 30.0).unwrap());
        // 일반 wheel: line 단위, discrete. 0 delta는 +0.0.
        let line = wheel_of(&convert_one(&scroll(0.0, (0.0, 2.0), 0, 0, 0)));
        assert_eq!(
            (line.unit, line.phase, line.delta),
            (
                WheelDeltaUnit::Line,
                WheelPhase::Discrete,
                Vec2::new(0.0, -2.0).unwrap()
            )
        );
        assert_eq!(line.delta.x().to_bits(), 0.0f32.to_bits());
        let phase = |phase: u64, momentum: u64| {
            convert_mac_event(&scroll(0.0, (0.0, 1.0), 1, phase, momentum), &mut domain())
                .map(|event| event.map(|event| wheel_of(&event).phase))
        };
        assert_eq!(
            phase(scroll_phase::CHANGED, 0),
            Ok(Some(WheelPhase::Changed))
        );
        assert_eq!(
            phase(scroll_phase::STATIONARY, 0),
            Ok(Some(WheelPhase::Changed))
        );
        assert_eq!(phase(scroll_phase::ENDED, 0), Ok(Some(WheelPhase::Ended)));
        assert_eq!(
            phase(scroll_phase::CANCELLED, 0),
            Ok(Some(WheelPhase::Ended))
        );
        for momentum in [
            scroll_phase::BEGAN,
            scroll_phase::CHANGED,
            scroll_phase::ENDED,
        ] {
            assert_eq!(phase(0, momentum), Ok(Some(WheelPhase::Momentum)));
        }
        assert_eq!(phase(scroll_phase::MAY_BEGIN, 0), Ok(None));
        assert_eq!(phase(0x40, 0), Err(MacEventError::UnknownScrollPhase(0x40)));
        assert_eq!(phase(0, 0x80), Err(MacEventError::UnknownScrollPhase(0x80)));
        // 비유한 delta, 0/1 외 precise 값.
        assert_eq!(
            convert_mac_event(&scroll(0.0, (f64::NAN, 0.0), 1, 0, 0), &mut domain()),
            Err(MacEventError::NonFiniteDelta)
        );
        assert_eq!(
            convert_mac_event(&scroll(0.0, (0.0, 1.0), 2, 0, 0), &mut domain()),
            Err(MacEventError::InvalidBool {
                field: "has_precise_deltas",
                value: 2
            })
        );
    }

    #[test]
    fn timestamps_use_core_domain_and_validate_layout() {
        let mut domain = domain();
        let first = convert_mac_event(&mouse(t::MOUSE_MOVED, 0.5, (0.0, 0.0), 0, 0), &mut domain)
            .expect("ok")
            .expect("event");
        assert_eq!(
            first.timestamp(),
            MonotonicNanos::new(CORE_ANCHOR + 500_000_000)
        );
        // 무시되는 이벤트는 domain 단조 상태를 바꾸지 않습니다.
        assert_eq!(
            convert_mac_event(&mouse(t::MOUSE_ENTERED, 9.0, (0.0, 0.0), 0, 0), &mut domain),
            Ok(None)
        );
        let second = convert_mac_event(&mouse(t::MOUSE_MOVED, 0.75, (0.0, 0.0), 0, 0), &mut domain)
            .expect("ok")
            .expect("event");
        assert_eq!(
            second.timestamp(),
            MonotonicNanos::new(CORE_ANCHOR + 750_000_000)
        );
        // 비유한 timestamp, 버전·예약 필드.
        let mut bad_time = mouse(t::MOUSE_MOVED, 0.0, (0.0, 0.0), 0, 0);
        bad_time.timestamp = f64::NAN;
        assert_eq!(
            convert_mac_event(&bad_time, &mut domain),
            Err(MacEventError::Event(InputEventError::NonFiniteTimestamp))
        );
        let mut bad_version = mouse(t::MOUSE_MOVED, 0.0, (0.0, 0.0), 0, 0);
        bad_version.version = 2;
        assert_eq!(
            convert_mac_event(&bad_version, &mut domain),
            Err(MacEventError::UnsupportedVersion(2))
        );
        let mut reserved = mouse(t::MOUSE_MOVED, 0.0, (0.0, 0.0), 0, 0);
        reserved.reserved = 1;
        assert_eq!(
            convert_mac_event(&reserved, &mut domain),
            Err(MacEventError::ReservedNotZero)
        );
    }

    #[test]
    fn timestamps_far_ahead_of_the_receive_time_are_cut() {
        let mut domain = domain();
        let at = |event: &StellarMacEvent, domain: &mut InputTimeDomain| {
            convert_mac_event(event, domain)
                .expect("ok")
                .expect("event")
                .timestamp()
        };
        // 수신 시각(+0.5초)보다 허용 오차를 넘게 미래인 timestamp는 수신 시각으로 자릅니다.
        let mut future = mouse(t::MOUSE_MOVED, 0.5, (0.0, 0.0), 0, 0);
        future.timestamp = 1.0e9;
        assert_eq!(
            at(&future, &mut domain),
            MonotonicNanos::new(CORE_ANCHOR + 500_000_000)
        );
        // 그 뒤 정상 이벤트는 자기 시각을 그대로 씁니다(미래 값에 고정되지 않음).
        let next = mouse(t::MOUSE_MOVED, 0.75, (0.0, 0.0), 0, 0);
        assert_eq!(
            at(&next, &mut domain),
            MonotonicNanos::new(CORE_ANCHOR + 750_000_000)
        );
        // 경계: 허용 오차(10초) 안의 앞선 timestamp는 그대로, 넘으면 수신 시각입니다.
        let mut within = mouse(t::MOUSE_MOVED, 1.0, (0.0, 0.0), 0, 0);
        within.timestamp = within.receive_uptime + MAX_TIMESTAMP_AHEAD_OF_RECEIVE_SECONDS;
        assert_eq!(
            at(&within, &mut domain),
            MonotonicNanos::new(CORE_ANCHOR + 11_000_000_000)
        );
        let mut beyond = mouse(t::MOUSE_MOVED, 20.0, (0.0, 0.0), 0, 0);
        beyond.timestamp = beyond.receive_uptime + MAX_TIMESTAMP_AHEAD_OF_RECEIVE_SECONDS + 0.5;
        assert_eq!(
            at(&beyond, &mut domain),
            MonotonicNanos::new(CORE_ANCHOR + 20_000_000_000)
        );
        // 수신 시각보다 이른 timestamp(늦게 처리한 이벤트)는 자르지 않고 단조 보정만 받습니다.
        let mut late = mouse(t::MOUSE_MOVED, 30.0, (0.0, 0.0), 0, 0);
        late.timestamp = MAC_ANCHOR_SECONDS + 25.0;
        assert_eq!(
            at(&late, &mut domain),
            MonotonicNanos::new(CORE_ANCHOR + 25_000_000_000)
        );
        // 비유한 timestamp는 수신 시각보다 앞서도 자르지 않고 거부합니다.
        let mut infinite = mouse(t::MOUSE_MOVED, 31.0, (0.0, 0.0), 0, 0);
        infinite.timestamp = f64::INFINITY;
        assert_eq!(
            convert_mac_event(&infinite, &mut domain),
            Err(MacEventError::Event(InputEventError::NonFiniteTimestamp))
        );
        // 수신 시각도 유한한 음이 아닌 값이어야 합니다.
        for (receive, error) in [
            (f64::NAN, InputEventError::NonFiniteTimestamp),
            (f64::NEG_INFINITY, InputEventError::NonFiniteTimestamp),
            (-0.5, InputEventError::NegativeTimestamp),
        ] {
            let mut bad = mouse(t::MOUSE_MOVED, 32.0, (0.0, 0.0), 0, 0);
            bad.receive_uptime = receive;
            assert_eq!(
                convert_mac_event(&bad, &mut domain),
                Err(MacEventError::Event(error))
            );
        }
    }

    #[test]
    fn c_layout_is_fixed() {
        // C 헤더의 StellarMacEvent와 같은 크기·offset이어야 합니다.
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<StellarMacEvent>(), 104);
        assert_eq!(offset_of!(StellarMacEvent, event_type), 4);
        assert_eq!(offset_of!(StellarMacEvent, key_code), 8);
        assert_eq!(offset_of!(StellarMacEvent, subtype), 10);
        assert_eq!(offset_of!(StellarMacEvent, is_repeat), 12);
        assert_eq!(offset_of!(StellarMacEvent, has_precise_deltas), 13);
        assert_eq!(offset_of!(StellarMacEvent, reserved), 14);
        assert_eq!(offset_of!(StellarMacEvent, modifier_flags), 16);
        assert_eq!(offset_of!(StellarMacEvent, timestamp), 24);
        assert_eq!(offset_of!(StellarMacEvent, receive_uptime), 32);
        assert_eq!(offset_of!(StellarMacEvent, x), 40);
        assert_eq!(offset_of!(StellarMacEvent, y), 48);
        assert_eq!(offset_of!(StellarMacEvent, click_count), 56);
        assert_eq!(offset_of!(StellarMacEvent, button_number), 64);
        assert_eq!(offset_of!(StellarMacEvent, scrolling_delta_x), 72);
        assert_eq!(offset_of!(StellarMacEvent, scrolling_delta_y), 80);
        assert_eq!(offset_of!(StellarMacEvent, phase), 88);
        assert_eq!(offset_of!(StellarMacEvent, momentum_phase), 96);
    }

    #[test]
    fn errors_have_korean_display() {
        assert_eq!(
            MacEventError::InvalidClickCount(0).to_string(),
            "mouse down/up의 click count는 1 이상이어야 합니다: 0"
        );
        assert_eq!(
            MacEventError::Event(InputEventError::NonFiniteTimestamp).to_string(),
            "입력 이벤트 생성 실패: native timestamp가 유한하지 않습니다"
        );
    }
}
