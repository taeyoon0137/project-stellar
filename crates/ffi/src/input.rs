//! 공통 입력 이벤트 (XII.3.1).
//!
//! macOS·Windows 셸은 각자의 native 이벤트를 같은 [`InputEvent`]로 변환해 코어에 전달합니다.
//! 이벤트 종류는 pointer, wheel, key, text composition, focus, resize 여섯 가지로 닫혀 있고,
//! 직렬화는 플랫폼과 무관한 하나의 versioned little-endian 레이아웃([`InputEvent::encode`])을
//! 사용합니다. 두 셸이 같은 논리 이벤트를 만들면 바이트까지 같아야 합니다(fixture로 검증).
//!
//! # 시각
//!
//! native timestamp(`NSEvent.timestamp`, `mach_absolute_time`, Windows QPC)는
//! [`NativeTimestamp`]로 받아 [`InputTimeDomain`]이 코어 단조 시계([`MonotonicClock`]) domain의
//! [`MonotonicNanos`]로 옮깁니다. 시스템 wall clock은 사용하지 않습니다(II.3.3).
//!
//! # 좌표 규칙
//!
//! - 위치·크기·wheel delta(point 단위)는 캔버스 view 기준 **논리 point**입니다(backing scale 이전).
//! - 원점은 캔버스 view의 좌상단이고 y는 아래로 증가합니다.
//! - text range는 이벤트 `text`의 **UTF-8 byte offset**입니다. native UTF-16 range는
//!   [`TextRange::from_utf16`]으로 변환합니다.

use std::fmt;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::value::{FiniteF32, Size2, Vec2};
use stellar_runtime::command_queue::InputEventClass;

/// 직렬화 레이아웃 버전. 레이아웃이 바뀌면 올리고 이전 버전 decoder는 거부합니다.
pub const INPUT_EVENT_SCHEMA_VERSION: u8 = 1;

/// 직렬화 header 길이: version(u8) + kind(u8) + timestamp(u64).
const HEADER_LEN: usize = 10;

/// 입력 이벤트 생성·변환·직렬화 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventError {
    /// native timestamp가 NaN 또는 ±무한대.
    NonFiniteTimestamp,
    /// native timestamp가 음수.
    NegativeTimestamp,
    /// nanosecond 변환 결과가 `u64` 범위를 넘음.
    TimestampOverflow,
    /// timebase(QPC frequency 또는 mach numer/denom)가 0.
    ZeroTimebase,
    /// 정의되지 않은 modifier bit.
    UnknownModifierBits(u8),
    /// pointer down/up에 button이 없음.
    MissingPointerButton,
    /// pointer down/up의 click count가 0.
    MissingClickCount,
    /// pointer move/cancel에 click count가 있음.
    UnexpectedClickCount,
    /// key up 이벤트에 repeat가 설정됨.
    RepeatOnKeyUp,
    /// HID usage 0(예약값)은 key code가 될 수 없음.
    InvalidKeyCode,
    /// text range가 text 범위를 넘거나 문자 경계가 아님.
    InvalidTextRange,
    /// composition cancel 이벤트에 text 또는 range가 있음.
    CancelWithText,
    /// text가 직렬화 한도(`u32::MAX` byte)를 넘음.
    TextTooLong,
    /// resize scale이 0 이하.
    NonPositiveScale,
    /// 지원하지 않는 레이아웃 버전.
    UnsupportedVersion(u8),
    /// 알 수 없는 이벤트 kind tag.
    UnknownKind(u8),
    /// 알 수 없는 필드 tag.
    UnknownTag {
        /// 필드 이름.
        field: &'static str,
        /// 읽은 tag 값.
        value: u8,
    },
    /// 입력 바이트가 레이아웃보다 짧음.
    Truncated,
    /// 레이아웃 뒤에 남는 바이트가 있음.
    TrailingBytes,
    /// float 필드가 유한하지 않음.
    NonFiniteFloat,
    /// 크기 필드가 음수.
    NegativeSize,
    /// bool 필드가 0/1이 아님.
    InvalidBool(u8),
    /// text 바이트가 유효한 UTF-8이 아님.
    InvalidUtf8,
}

impl fmt::Display for InputEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputEventError::NonFiniteTimestamp => {
                f.write_str("native timestamp가 유한하지 않습니다")
            }
            InputEventError::NegativeTimestamp => f.write_str("native timestamp가 음수입니다"),
            InputEventError::TimestampOverflow => {
                f.write_str("timestamp nanosecond 변환이 u64 범위를 넘습니다")
            }
            InputEventError::ZeroTimebase => f.write_str("timestamp timebase가 0입니다"),
            InputEventError::UnknownModifierBits(bits) => {
                write!(f, "정의되지 않은 modifier bit입니다: {bits:#04x}")
            }
            InputEventError::MissingPointerButton => {
                f.write_str("pointer down/up에는 button이 필요합니다")
            }
            InputEventError::MissingClickCount => {
                f.write_str("pointer down/up의 click count는 1 이상이어야 합니다")
            }
            InputEventError::UnexpectedClickCount => {
                f.write_str("pointer move/cancel에는 click count가 없어야 합니다")
            }
            InputEventError::RepeatOnKeyUp => f.write_str("key up 이벤트는 repeat일 수 없습니다"),
            InputEventError::InvalidKeyCode => f.write_str("HID usage 0은 key code가 아닙니다"),
            InputEventError::InvalidTextRange => {
                f.write_str("text range가 범위를 넘거나 문자 경계가 아닙니다")
            }
            InputEventError::CancelWithText => {
                f.write_str("composition cancel에는 text와 range가 없어야 합니다")
            }
            InputEventError::TextTooLong => f.write_str("text가 직렬화 한도를 넘습니다"),
            InputEventError::NonPositiveScale => f.write_str("resize scale은 0보다 커야 합니다"),
            InputEventError::UnsupportedVersion(version) => {
                write!(
                    f,
                    "지원하지 않는 입력 이벤트 레이아웃 버전입니다: {version}"
                )
            }
            InputEventError::UnknownKind(tag) => {
                write!(f, "알 수 없는 입력 이벤트 kind입니다: {tag}")
            }
            InputEventError::UnknownTag { field, value } => {
                write!(f, "알 수 없는 {field} tag입니다: {value}")
            }
            InputEventError::Truncated => f.write_str("입력 이벤트 바이트가 잘렸습니다"),
            InputEventError::TrailingBytes => {
                f.write_str("입력 이벤트 뒤에 남는 바이트가 있습니다")
            }
            InputEventError::NonFiniteFloat => f.write_str("float 필드가 유한하지 않습니다"),
            InputEventError::NegativeSize => f.write_str("크기 필드가 음수입니다"),
            InputEventError::InvalidBool(value) => {
                write!(f, "bool 필드 값이 0/1이 아닙니다: {value}")
            }
            InputEventError::InvalidUtf8 => f.write_str("text가 유효한 UTF-8이 아닙니다"),
        }
    }
}

impl std::error::Error for InputEventError {}

// ---------------------------------------------------------------------------
// 시각
// ---------------------------------------------------------------------------

/// 코어 단조 시계 domain의 시각(nanoseconds). [`MonotonicClock::now_nanos`]와 같은 축입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicNanos(u64);

impl MonotonicNanos {
    /// nanoseconds 값으로 만듭니다.
    pub const fn new(nanos: u64) -> MonotonicNanos {
        MonotonicNanos(nanos)
    }

    /// nanoseconds 값.
    pub const fn get(&self) -> u64 {
        self.0
    }
}

/// 플랫폼 native 이벤트 timestamp.
///
/// 각 변형은 해당 플랫폼의 단조 기준점(부팅 이후) 시각이며, wall clock이 아닙니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeTimestamp {
    /// macOS `NSEvent.timestamp`: 시스템 시작 이후 초(`TimeInterval`).
    MacEventSeconds(f64),
    /// macOS `mach_absolute_time()` tick과 `mach_timebase_info`(nanos = ticks × numer / denom).
    MacMachAbsolute {
        /// `mach_absolute_time()` 값.
        ticks: u64,
        /// timebase 분자.
        numer: u32,
        /// timebase 분모.
        denom: u32,
    },
    /// Windows `QueryPerformanceCounter` tick과 `QueryPerformanceFrequency`(Hz).
    WindowsQpc {
        /// QPC 값(pointer 입력은 `POINTER_INFO.PerformanceCount`).
        ticks: u64,
        /// 초당 tick 수.
        frequency: u64,
    },
}

impl NativeTimestamp {
    /// native 기준점 이후 nanoseconds로 변환합니다.
    ///
    /// 초 단위 float는 가장 가까운 nanosecond로 반올림하고, tick 단위는 내림합니다.
    pub fn to_native_nanos(self) -> Result<u64, InputEventError> {
        match self {
            NativeTimestamp::MacEventSeconds(seconds) => {
                if !seconds.is_finite() {
                    return Err(InputEventError::NonFiniteTimestamp);
                }
                if seconds < 0.0 {
                    return Err(InputEventError::NegativeTimestamp);
                }
                let nanos = (seconds * 1_000_000_000.0).round();
                // 2^64 이상은 u64로 표현할 수 없습니다.
                if nanos >= 18_446_744_073_709_551_616.0 {
                    return Err(InputEventError::TimestampOverflow);
                }
                Ok(nanos as u64)
            }
            NativeTimestamp::MacMachAbsolute {
                ticks,
                numer,
                denom,
            } => {
                if numer == 0 || denom == 0 {
                    return Err(InputEventError::ZeroTimebase);
                }
                let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
                u64::try_from(nanos).map_err(|_| InputEventError::TimestampOverflow)
            }
            NativeTimestamp::WindowsQpc { ticks, frequency } => {
                if frequency == 0 {
                    return Err(InputEventError::ZeroTimebase);
                }
                let nanos = u128::from(ticks) * 1_000_000_000 / u128::from(frequency);
                u64::try_from(nanos).map_err(|_| InputEventError::TimestampOverflow)
            }
        }
    }
}

/// native timestamp를 코어 단조 domain([`MonotonicNanos`])으로 옮기는 변환기.
///
/// 같은 순간에 잡은 기준쌍(native nanos, core nanos)으로부터
/// `core = core_anchor + (native - native_anchor)`를 계산합니다. 결과는 `[0, u64::MAX]`로
/// 고정하고, 변환 순서대로 단조 비감소가 되도록 이전 결과보다 이르면 이전 결과로 고정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTimeDomain {
    native_anchor: u64,
    core_anchor: u64,
    last: Option<u64>,
}

impl InputTimeDomain {
    /// 지금 읽은 native 시각과 코어 시계로 기준쌍을 잡습니다.
    pub fn new(
        native_now: NativeTimestamp,
        clock: &impl MonotonicClock,
    ) -> Result<InputTimeDomain, InputEventError> {
        Ok(InputTimeDomain::from_anchor_nanos(
            native_now.to_native_nanos()?,
            clock.now_nanos(),
        ))
    }

    /// 이미 nanoseconds로 계산된 기준쌍으로 만듭니다.
    pub const fn from_anchor_nanos(native_anchor: u64, core_anchor: u64) -> InputTimeDomain {
        InputTimeDomain {
            native_anchor,
            core_anchor,
            last: None,
        }
    }

    /// native timestamp를 코어 단조 domain 시각으로 변환합니다.
    pub fn convert(&mut self, native: NativeTimestamp) -> Result<MonotonicNanos, InputEventError> {
        let native_nanos = native.to_native_nanos()?;
        let offset = i128::from(native_nanos) - i128::from(self.native_anchor);
        let mapped = (i128::from(self.core_anchor) + offset).clamp(0, i128::from(u64::MAX));
        // clamp 이후이므로 항상 u64 범위입니다.
        let mapped = u64::try_from(mapped).map_err(|_| InputEventError::TimestampOverflow)?;
        let monotonic = match self.last {
            Some(last) if mapped < last => last,
            _ => mapped,
        };
        self.last = Some(monotonic);
        Ok(MonotonicNanos(monotonic))
    }
}

// ---------------------------------------------------------------------------
// 공통 필드
// ---------------------------------------------------------------------------

/// 물리 modifier key 집합(bit set).
///
/// 플랫폼 단축키 관례(macOS Command ↔ Windows Ctrl)는 여기서 바꾸지 않고 물리 key 그대로 둡니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// modifier 없음.
    pub const NONE: Modifiers = Modifiers(0);
    /// Shift.
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    /// Control.
    pub const CONTROL: Modifiers = Modifiers(1 << 1);
    /// macOS Option, Windows Alt.
    pub const ALT: Modifiers = Modifiers(1 << 2);
    /// macOS Command, Windows key.
    pub const META: Modifiers = Modifiers(1 << 3);
    /// Caps Lock 상태.
    pub const CAPS_LOCK: Modifiers = Modifiers(1 << 4);

    const DEFINED_BITS: u8 = 0b1_1111;

    /// bit 값으로 만듭니다. 정의되지 않은 bit는 거부합니다.
    pub const fn from_bits(bits: u8) -> Result<Modifiers, InputEventError> {
        if bits & !Modifiers::DEFINED_BITS != 0 {
            return Err(InputEventError::UnknownModifierBits(bits));
        }
        Ok(Modifiers(bits))
    }

    /// bit 값.
    pub const fn bits(&self) -> u8 {
        self.0
    }

    /// 두 집합의 합집합.
    pub const fn union(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }

    /// `other`의 모든 bit를 포함하는지.
    pub const fn contains(&self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;

    fn bitor(self, rhs: Modifiers) -> Modifiers {
        self.union(rhs)
    }
}

/// 닫힌 u8 tag enum과 encode/decode helper를 정의합니다.
macro_rules! tag_enum {
    ($(#[$meta:meta])* $name:ident, $field:literal { $($(#[$vmeta:meta])* $variant:ident = $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// 직렬화 tag 값.
            pub const fn tag(&self) -> u8 {
                match self {
                    $($name::$variant => $value),+
                }
            }

            /// tag 값에서 복원합니다. 알 수 없는 tag는 거부합니다.
            pub const fn from_tag(tag: u8) -> Result<$name, InputEventError> {
                match tag {
                    $($value => Ok($name::$variant),)+
                    _ => Err(InputEventError::UnknownTag { field: $field, value: tag }),
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// pointer
// ---------------------------------------------------------------------------

tag_enum!(
    /// pointer 이벤트 단계.
    PointerPhase, "pointer phase" {
        /// 버튼 누름.
        Down = 1,
        /// 버튼 뗌.
        Up = 2,
        /// 이동(버튼 유무 무관).
        Move = 3,
        /// 운영체제가 진행 중 pointer를 취소(capture 손실 등).
        Cancel = 4,
    }
);

tag_enum!(
    /// pointer button.
    PointerButton, "pointer button" {
        /// button 없음(hover move 등).
        None = 0,
        /// 주 버튼(왼쪽, pen tip, touch contact).
        Primary = 1,
        /// 보조 버튼(오른쪽, pen barrel).
        Secondary = 2,
        /// 가운데 버튼.
        Middle = 3,
        /// 뒤로 버튼(X1).
        Back = 4,
        /// 앞으로 버튼(X2).
        Forward = 5,
    }
);

tag_enum!(
    /// pointer 입력 장치. macOS trackpad 클릭은 `Mouse`입니다.
    PointerDevice, "pointer device" {
        /// 마우스·trackpad.
        Mouse = 1,
        /// pen·stylus.
        Pen = 2,
        /// touch.
        Touch = 3,
    }
);

/// pointer down/up/move/cancel 이벤트.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerEvent {
    /// 단계.
    pub phase: PointerPhase,
    /// 변화한(down/up) 또는 누르고 있는(move) button.
    pub button: PointerButton,
    /// 입력 장치.
    pub device: PointerDevice,
    /// 캔버스 view 기준 논리 point 위치.
    pub position: Vec2,
    /// modifier.
    pub modifiers: Modifiers,
    /// down/up의 연속 click 수(1=single, 2=double). move/cancel은 0.
    pub click_count: u8,
}

impl PointerEvent {
    fn validate(&self) -> Result<(), InputEventError> {
        match self.phase {
            PointerPhase::Down | PointerPhase::Up => {
                if self.button == PointerButton::None {
                    return Err(InputEventError::MissingPointerButton);
                }
                if self.click_count == 0 {
                    return Err(InputEventError::MissingClickCount);
                }
            }
            PointerPhase::Move | PointerPhase::Cancel => {
                if self.click_count != 0 {
                    return Err(InputEventError::UnexpectedClickCount);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// wheel
// ---------------------------------------------------------------------------

tag_enum!(
    /// wheel delta 단위.
    WheelDeltaUnit, "wheel unit" {
        /// 논리 point(macOS precise scrolling 등).
        Point = 1,
        /// 줄 단위(일반 wheel notch).
        Line = 2,
    }
);

tag_enum!(
    /// wheel·scroll gesture 단계.
    WheelPhase, "wheel phase" {
        /// 단계 없는 개별 wheel 이벤트.
        Discrete = 1,
        /// gesture 시작.
        Began = 2,
        /// gesture 진행.
        Changed = 3,
        /// gesture 종료.
        Ended = 4,
        /// 손을 뗀 뒤 관성 scroll.
        Momentum = 5,
    }
);

/// wheel·scroll 이벤트.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelEvent {
    /// 단계.
    pub phase: WheelPhase,
    /// delta 단위.
    pub unit: WheelDeltaUnit,
    /// 캔버스 view 기준 논리 point 위치.
    pub position: Vec2,
    /// scroll 양(x 오른쪽, y 아래쪽 양수).
    pub delta: Vec2,
    /// modifier.
    pub modifiers: Modifiers,
}

// ---------------------------------------------------------------------------
// key
// ---------------------------------------------------------------------------

tag_enum!(
    /// key 상태 변화.
    KeyState, "key state" {
        /// 누름.
        Down = 1,
        /// 뗌.
        Up = 2,
    }
);

/// 플랫폼 중립 물리 key: USB HID Keyboard/Keypad page(0x07) usage ID.
///
/// macOS virtual key code와 Windows scan code는 각 셸이 이 값으로 변환합니다(XII.3.2, XII.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyCode(u16);

impl KeyCode {
    /// HID usage ID로 만듭니다. 0(예약값)은 거부합니다.
    pub const fn new(usage: u16) -> Result<KeyCode, InputEventError> {
        if usage == 0 {
            return Err(InputEventError::InvalidKeyCode);
        }
        Ok(KeyCode(usage))
    }

    /// HID usage ID.
    pub const fn usage(&self) -> u16 {
        self.0
    }
}

/// 물리 key 이벤트. 입력 문자는 key가 아니라 [`CompositionEvent`]로 전달합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// 상태 변화.
    pub state: KeyState,
    /// 물리 key.
    pub key: KeyCode,
    /// 자동 반복 down인지(up은 항상 false).
    pub repeat: bool,
    /// modifier.
    pub modifiers: Modifiers,
}

impl KeyEvent {
    fn validate(&self) -> Result<(), InputEventError> {
        if self.state == KeyState::Up && self.repeat {
            return Err(InputEventError::RepeatOnKeyUp);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// text composition
// ---------------------------------------------------------------------------

tag_enum!(
    /// IME composition 단계.
    CompositionPhase, "composition phase" {
        /// 조합 시작.
        Start = 1,
        /// 조합 중간 text(marked text) 갱신.
        Update = 2,
        /// 확정. IME 없는 직접 입력도 단독 `Commit`으로 전달합니다.
        Commit = 3,
        /// 조합 취소(text 없음).
        Cancel = 4,
    }
);

/// text 안의 UTF-8 byte range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextRange {
    start: u32,
    end: u32,
}

impl TextRange {
    /// 빈 range(0..0).
    pub const EMPTY: TextRange = TextRange { start: 0, end: 0 };

    /// `text` 안의 UTF-8 byte range를 만듭니다. 범위·문자 경계를 검사합니다.
    pub fn new(text: &str, start: u32, end: u32) -> Result<TextRange, InputEventError> {
        let range = TextRange { start, end };
        range.check(text)?;
        Ok(range)
    }

    /// native UTF-16 code unit range(macOS `NSRange`, Windows TSF ACP)를 UTF-8 byte range로
    /// 변환합니다. surrogate pair 중간이나 text 밖을 가리키면 거부합니다.
    pub fn from_utf16(
        text: &str,
        start_utf16: u32,
        len_utf16: u32,
    ) -> Result<TextRange, InputEventError> {
        let end_utf16 = start_utf16
            .checked_add(len_utf16)
            .ok_or(InputEventError::InvalidTextRange)?;
        let start = utf16_to_utf8_offset(text, start_utf16)?;
        let end = utf16_to_utf8_offset(text, end_utf16)?;
        TextRange::new(text, start, end)
    }

    /// 시작 byte offset.
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// 끝 byte offset(배타).
    pub const fn end(&self) -> u32 {
        self.end
    }

    fn check(&self, text: &str) -> Result<(), InputEventError> {
        let (start, end) = (self.start as usize, self.end as usize);
        if start > end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            return Err(InputEventError::InvalidTextRange);
        }
        Ok(())
    }
}

/// UTF-16 code unit offset을 UTF-8 byte offset으로 변환합니다.
fn utf16_to_utf8_offset(text: &str, target_utf16: u32) -> Result<u32, InputEventError> {
    let target = target_utf16 as usize;
    let mut utf16 = 0usize;
    for (byte_index, ch) in text.char_indices() {
        if utf16 == target {
            return u32::try_from(byte_index).map_err(|_| InputEventError::TextTooLong);
        }
        utf16 += ch.len_utf16();
        if utf16 > target {
            // surrogate pair 중간.
            return Err(InputEventError::InvalidTextRange);
        }
    }
    if utf16 == target {
        return u32::try_from(text.len()).map_err(|_| InputEventError::TextTooLong);
    }
    Err(InputEventError::InvalidTextRange)
}

/// IME composition·text 입력 이벤트.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionEvent {
    /// 단계.
    pub phase: CompositionPhase,
    /// 조합 중(marked) 또는 확정 text.
    pub text: String,
    /// `text` 안의 선택·caret range(UTF-8 byte).
    pub selection: TextRange,
}

impl CompositionEvent {
    fn validate(&self) -> Result<(), InputEventError> {
        if u32::try_from(self.text.len()).is_err() {
            return Err(InputEventError::TextTooLong);
        }
        if self.phase == CompositionPhase::Cancel
            && (!self.text.is_empty() || self.selection != TextRange::EMPTY)
        {
            return Err(InputEventError::CancelWithText);
        }
        self.selection.check(&self.text)
    }
}

// ---------------------------------------------------------------------------
// focus, resize
// ---------------------------------------------------------------------------

tag_enum!(
    /// 캔버스 view keyboard focus 변화.
    FocusEvent, "focus" {
        /// focus 획득.
        Gained = 1,
        /// focus 상실.
        Lost = 2,
    }
);

/// 캔버스 view 크기·backing scale 변화.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeEvent {
    /// 논리 point 크기.
    pub logical_size: Size2,
    /// backing scale(> 0). drawable pixel = 논리 크기 × scale.
    pub scale: FiniteF32,
}

impl ResizeEvent {
    fn validate(&self) -> Result<(), InputEventError> {
        if self.scale.get() <= 0.0 {
            return Err(InputEventError::NonPositiveScale);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InputEvent
// ---------------------------------------------------------------------------

/// 입력 이벤트 종류(닫힌 집합).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEventKind {
    /// pointer down/up/move/cancel.
    Pointer(PointerEvent),
    /// wheel·scroll.
    Wheel(WheelEvent),
    /// 물리 key.
    Key(KeyEvent),
    /// IME composition·text 입력.
    TextComposition(CompositionEvent),
    /// keyboard focus.
    Focus(FocusEvent),
    /// 크기·scale 변화.
    Resize(ResizeEvent),
}

impl InputEventKind {
    /// 직렬화 kind tag.
    pub const fn tag(&self) -> u8 {
        match self {
            InputEventKind::Pointer(_) => 1,
            InputEventKind::Wheel(_) => 2,
            InputEventKind::Key(_) => 3,
            InputEventKind::TextComposition(_) => 4,
            InputEventKind::Focus(_) => 5,
            InputEventKind::Resize(_) => 6,
        }
    }

    fn validate(&self) -> Result<(), InputEventError> {
        match self {
            InputEventKind::Pointer(event) => event.validate(),
            InputEventKind::Wheel(_) | InputEventKind::Focus(_) => Ok(()),
            InputEventKind::Key(event) => event.validate(),
            InputEventKind::TextComposition(event) => event.validate(),
            InputEventKind::Resize(event) => event.validate(),
        }
    }
}

/// 양 플랫폼 공통 입력 이벤트: 코어 단조 domain 시각 + 종류별 payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    timestamp: MonotonicNanos,
    kind: InputEventKind,
}

impl InputEvent {
    /// 검증된 이벤트를 만듭니다.
    pub fn new(
        timestamp: MonotonicNanos,
        kind: InputEventKind,
    ) -> Result<InputEvent, InputEventError> {
        kind.validate()?;
        Ok(InputEvent { timestamp, kind })
    }

    /// 코어 단조 domain 시각.
    pub const fn timestamp(&self) -> MonotonicNanos {
        self.timestamp
    }

    /// 이벤트 종류.
    pub const fn kind(&self) -> &InputEventKind {
        &self.kind
    }

    /// document thread 수신함 접수 분류(XII.1.2). 모든 입력은 input 우선순위이며, pointer move는
    /// 장치별로, resize는 최신 값으로 병합하고 그 밖은 병합하지 않습니다.
    pub fn queue_class(&self) -> InputEventClass {
        match &self.kind {
            InputEventKind::Pointer(pointer) if pointer.phase == PointerPhase::Move => {
                InputEventClass::PointerMove {
                    device: pointer.device.tag(),
                }
            }
            InputEventKind::Resize(_) => InputEventClass::Resize,
            InputEventKind::Wheel(_) => InputEventClass::Wheel,
            InputEventKind::Pointer(_)
            | InputEventKind::Key(_)
            | InputEventKind::TextComposition(_)
            | InputEventKind::Focus(_) => InputEventClass::Discrete,
        }
    }

    /// 플랫폼 공통 little-endian 레이아웃으로 직렬화합니다.
    ///
    /// ```text
    /// header : version u8 | kind u8 | timestamp_nanos u64
    /// 1 pointer : phase u8 | button u8 | device u8 | click_count u8 | modifiers u8 | x f32 | y f32
    /// 2 wheel   : phase u8 | unit u8 | modifiers u8 | x f32 | y f32 | dx f32 | dy f32
    /// 3 key     : state u8 | repeat u8 | modifiers u8 | hid_usage u16
    /// 4 text    : phase u8 | sel_start u32 | sel_end u32 | text_len u32 | text UTF-8
    /// 5 focus   : focus u8
    /// 6 resize  : width f32 | height f32 | scale f32
    /// ```
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + 19);
        out.push(INPUT_EVENT_SCHEMA_VERSION);
        out.push(self.kind.tag());
        out.extend_from_slice(&self.timestamp.0.to_le_bytes());
        match &self.kind {
            InputEventKind::Pointer(event) => {
                out.push(event.phase.tag());
                out.push(event.button.tag());
                out.push(event.device.tag());
                out.push(event.click_count);
                out.push(event.modifiers.bits());
                put_vec2(&mut out, event.position);
            }
            InputEventKind::Wheel(event) => {
                out.push(event.phase.tag());
                out.push(event.unit.tag());
                out.push(event.modifiers.bits());
                put_vec2(&mut out, event.position);
                put_vec2(&mut out, event.delta);
            }
            InputEventKind::Key(event) => {
                out.push(event.state.tag());
                out.push(u8::from(event.repeat));
                out.push(event.modifiers.bits());
                out.extend_from_slice(&event.key.0.to_le_bytes());
            }
            InputEventKind::TextComposition(event) => {
                out.push(event.phase.tag());
                out.extend_from_slice(&event.selection.start.to_le_bytes());
                out.extend_from_slice(&event.selection.end.to_le_bytes());
                // InputEvent::new에서 u32 범위를 검증했습니다.
                let len = event.text.len() as u32;
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(event.text.as_bytes());
            }
            InputEventKind::Focus(event) => out.push(event.tag()),
            InputEventKind::Resize(event) => {
                out.extend_from_slice(&event.logical_size.width().to_le_bytes());
                out.extend_from_slice(&event.logical_size.height().to_le_bytes());
                out.extend_from_slice(&event.scale.to_le_bytes());
            }
        }
        out
    }

    /// [`InputEvent::encode`] 레이아웃을 복원합니다. 버전·tag·float·UTF-8·길이를 모두 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<InputEvent, InputEventError> {
        let mut reader = Reader { bytes, pos: 0 };
        let version = reader.u8()?;
        if version != INPUT_EVENT_SCHEMA_VERSION {
            return Err(InputEventError::UnsupportedVersion(version));
        }
        let kind_tag = reader.u8()?;
        let timestamp = MonotonicNanos(reader.u64()?);
        let kind = match kind_tag {
            1 => InputEventKind::Pointer(PointerEvent {
                phase: PointerPhase::from_tag(reader.u8()?)?,
                button: PointerButton::from_tag(reader.u8()?)?,
                device: PointerDevice::from_tag(reader.u8()?)?,
                click_count: reader.u8()?,
                modifiers: Modifiers::from_bits(reader.u8()?)?,
                position: reader.vec2()?,
            }),
            2 => InputEventKind::Wheel(WheelEvent {
                phase: WheelPhase::from_tag(reader.u8()?)?,
                unit: WheelDeltaUnit::from_tag(reader.u8()?)?,
                modifiers: Modifiers::from_bits(reader.u8()?)?,
                position: reader.vec2()?,
                delta: reader.vec2()?,
            }),
            3 => InputEventKind::Key(KeyEvent {
                state: KeyState::from_tag(reader.u8()?)?,
                repeat: reader.bool()?,
                modifiers: Modifiers::from_bits(reader.u8()?)?,
                key: KeyCode::new(reader.u16()?)?,
            }),
            4 => {
                let phase = CompositionPhase::from_tag(reader.u8()?)?;
                let start = reader.u32()?;
                let end = reader.u32()?;
                let len = reader.u32()? as usize;
                let text = std::str::from_utf8(reader.take(len)?)
                    .map_err(|_| InputEventError::InvalidUtf8)?
                    .to_string();
                InputEventKind::TextComposition(CompositionEvent {
                    phase,
                    text,
                    selection: TextRange { start, end },
                })
            }
            5 => InputEventKind::Focus(FocusEvent::from_tag(reader.u8()?)?),
            6 => {
                let width = reader.f32()?;
                let height = reader.f32()?;
                let scale = reader.f32()?;
                let logical_size = Size2::new(width.get(), height.get())
                    .map_err(|_| InputEventError::NegativeSize)?;
                InputEventKind::Resize(ResizeEvent {
                    logical_size,
                    scale,
                })
            }
            other => return Err(InputEventError::UnknownKind(other)),
        };
        if reader.pos != bytes.len() {
            return Err(InputEventError::TrailingBytes);
        }
        InputEvent::new(timestamp, kind)
    }
}

fn put_vec2(out: &mut Vec<u8>, value: Vec2) {
    out.extend_from_slice(&value.x().to_le_bytes());
    out.extend_from_slice(&value.y().to_le_bytes());
}

/// 경계 검사 byte reader.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], InputEventError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(InputEventError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(InputEventError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], InputEventError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, InputEventError> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool, InputEventError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(InputEventError::InvalidBool(other)),
        }
    }

    fn u16(&mut self) -> Result<u16, InputEventError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, InputEventError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, InputEventError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<FiniteF32, InputEventError> {
        FiniteF32::from_le_bytes(self.array()?).map_err(|_| InputEventError::NonFiniteFloat)
    }

    fn vec2(&mut self) -> Result<Vec2, InputEventError> {
        let x = self.f32()?;
        let y = self.f32()?;
        Vec2::new(x.get(), y.get()).map_err(|_| InputEventError::NonFiniteFloat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::clock::ManualClock;

    /// 양 플랫폼 golden fixture(이름 → 직렬화 hex).
    const GOLDEN: &str = include_str!("../tests/fixtures/input_events_v1.golden");

    /// 코어 시계 기준점(두 플랫폼 공통).
    const CORE_ANCHOR: u64 = 1_000_000_000;
    /// macOS 기준점: NSEvent.timestamp 100초.
    const MAC_ANCHOR_SECONDS: f64 = 100.0;
    /// Windows 기준점: QPC 10MHz에서 200초.
    const WIN_QPC_FREQUENCY: u64 = 10_000_000;
    const WIN_ANCHOR_TICKS: u64 = 2_000_000_000;
    /// Apple Silicon mach timebase(24MHz: 125/3).
    const MACH_NUMER: u32 = 125;
    const MACH_DENOM: u32 = 3;
    const MACH_ANCHOR_TICKS: u64 = 2_400_000_000;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).expect("finite")
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
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

    fn composition(phase: CompositionPhase, text: &str, utf16: (u32, u32)) -> InputEventKind {
        // macOS NSRange와 Windows TSF ACP는 모두 UTF-16 code unit range입니다.
        let selection = if phase == CompositionPhase::Cancel {
            TextRange::EMPTY
        } else {
            TextRange::from_utf16(text, utf16.0, utf16.1).expect("utf16 range")
        };
        InputEventKind::TextComposition(CompositionEvent {
            phase,
            text: text.to_string(),
            selection,
        })
    }

    /// 두 플랫폼 공통 논리 이벤트: (fixture 이름, 기준점 이후 초, 종류).
    ///
    /// 초는 f64·QPC(10MHz)·mach(24MHz)에서 모두 정확히 표현되는 값만 사용합니다.
    fn logical_events() -> Vec<(&'static str, f64, InputEventKind)> {
        let pointer = |phase, button, position, modifiers, click_count| {
            InputEventKind::Pointer(PointerEvent {
                phase,
                button,
                device: PointerDevice::Mouse,
                position,
                modifiers,
                click_count,
            })
        };
        let key = |state, modifiers| {
            InputEventKind::Key(KeyEvent {
                state,
                key: KeyCode::new(0x04).expect("hid"),
                repeat: false,
                modifiers,
            })
        };
        vec![
            (
                "pointer_down_primary",
                0.125,
                pointer(
                    PointerPhase::Down,
                    PointerButton::Primary,
                    v(120.5, 64.25),
                    Modifiers::NONE,
                    1,
                ),
            ),
            (
                "pointer_up_primary_double_shift",
                0.25,
                pointer(
                    PointerPhase::Up,
                    PointerButton::Primary,
                    v(120.5, 64.25),
                    Modifiers::SHIFT,
                    2,
                ),
            ),
            (
                "pointer_move_hover",
                0.375,
                pointer(
                    PointerPhase::Move,
                    PointerButton::None,
                    v(130.0, 70.0),
                    Modifiers::NONE,
                    0,
                ),
            ),
            (
                "wheel_line_control",
                0.5,
                InputEventKind::Wheel(WheelEvent {
                    phase: WheelPhase::Discrete,
                    unit: WheelDeltaUnit::Line,
                    position: v(130.0, 70.0),
                    delta: v(0.0, -3.0),
                    modifiers: Modifiers::CONTROL,
                }),
            ),
            (
                "key_down_a_meta",
                0.625,
                key(KeyState::Down, Modifiers::META),
            ),
            ("key_up_a", 0.75, key(KeyState::Up, Modifiers::NONE)),
            (
                "composition_start",
                0.875,
                composition(CompositionPhase::Start, "", (0, 0)),
            ),
            (
                "composition_update_han",
                1.0,
                composition(CompositionPhase::Update, "한", (1, 0)),
            ),
            (
                "composition_commit_hangul",
                1.125,
                composition(CompositionPhase::Commit, "한글", (2, 0)),
            ),
            (
                "composition_cancel",
                1.25,
                composition(CompositionPhase::Cancel, "", (0, 0)),
            ),
            (
                "focus_gained",
                1.375,
                InputEventKind::Focus(FocusEvent::Gained),
            ),
            ("focus_lost", 1.5, InputEventKind::Focus(FocusEvent::Lost)),
            (
                "resize_800x600_at_2x",
                1.625,
                InputEventKind::Resize(ResizeEvent {
                    logical_size: Size2::new(800.0, 600.0).expect("size"),
                    scale: FiniteF32::new(2.0).expect("finite"),
                }),
            ),
        ]
    }

    /// 플랫폼 하나의 native 시각 원천.
    #[derive(Clone, Copy)]
    enum Platform {
        MacEventSeconds,
        MacMachAbsolute,
        WindowsQpc,
    }

    fn native_at(platform: Platform, after_seconds: f64) -> NativeTimestamp {
        match platform {
            Platform::MacEventSeconds => {
                NativeTimestamp::MacEventSeconds(MAC_ANCHOR_SECONDS + after_seconds)
            }
            Platform::MacMachAbsolute => {
                // 24MHz tick: 초 × 24,000,000.
                let delta = (after_seconds * 24_000_000.0) as u64;
                NativeTimestamp::MacMachAbsolute {
                    ticks: MACH_ANCHOR_TICKS + delta,
                    numer: MACH_NUMER,
                    denom: MACH_DENOM,
                }
            }
            Platform::WindowsQpc => {
                let delta = (after_seconds * WIN_QPC_FREQUENCY as f64) as u64;
                NativeTimestamp::WindowsQpc {
                    ticks: WIN_ANCHOR_TICKS + delta,
                    frequency: WIN_QPC_FREQUENCY,
                }
            }
        }
    }

    fn platform_events(platform: Platform) -> Vec<(&'static str, InputEvent)> {
        let clock = ManualClock::new(CORE_ANCHOR);
        let mut domain =
            InputTimeDomain::new(native_at(platform, 0.0), &clock).expect("time domain");
        logical_events()
            .into_iter()
            .map(|(name, after_seconds, kind)| {
                let timestamp = domain
                    .convert(native_at(platform, after_seconds))
                    .expect("timestamp");
                (name, InputEvent::new(timestamp, kind).expect("valid event"))
            })
            .collect()
    }

    #[test]
    fn both_platforms_match_golden_fixture() {
        // 양 플랫폼 event fixture schema 동일: macOS(NSEvent 초, mach tick)와 Windows(QPC)
        // 원천에서 만든 같은 논리 이벤트가 golden 바이트와 바이트 단위로 같아야 합니다.
        let golden = golden();
        let mac = platform_events(Platform::MacEventSeconds);
        let mach = platform_events(Platform::MacMachAbsolute);
        let win = platform_events(Platform::WindowsQpc);
        assert_eq!(golden.len(), mac.len(), "fixture 항목 수");
        for (index, (name, expected_hex)) in golden.iter().enumerate() {
            for (label, events) in [("mac", &mac), ("mach", &mach), ("win", &win)] {
                let (event_name, event) = &events[index];
                assert_eq!(event_name, name, "{label} fixture 순서");
                assert_eq!(&hex(&event.encode()), expected_hex, "{label} {name} bytes");
            }
            // golden 바이트는 같은 이벤트로 복원됩니다.
            let decoded = InputEvent::decode(&unhex(expected_hex)).expect("decode golden");
            assert_eq!(decoded, mac[index].1, "{name} decode");
        }
    }

    #[test]
    fn golden_covers_every_event_kind() {
        let mut tags: Vec<u8> = platform_events(Platform::WindowsQpc)
            .iter()
            .map(|(_, event)| event.kind().tag())
            .collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn queue_class_merges_only_moves_and_resizes() {
        // XII.3.5: hit-test request 접수 분류. 모든 golden 이벤트가 정해진 분류로 갑니다.
        for (name, event) in platform_events(Platform::WindowsQpc) {
            let expected = match event.kind() {
                InputEventKind::Pointer(pointer) if pointer.phase == PointerPhase::Move => {
                    InputEventClass::PointerMove {
                        device: PointerDevice::Mouse.tag(),
                    }
                }
                InputEventKind::Resize(_) => InputEventClass::Resize,
                InputEventKind::Wheel(_) => InputEventClass::Wheel,
                _ => InputEventClass::Discrete,
            };
            assert_eq!(event.queue_class(), expected, "{name}");
        }
        let pen_move = InputEvent::new(
            MonotonicNanos::new(1),
            InputEventKind::Pointer(PointerEvent {
                phase: PointerPhase::Move,
                button: PointerButton::Primary,
                device: PointerDevice::Pen,
                position: v(0.0, 0.0),
                modifiers: Modifiers::NONE,
                click_count: 0,
            }),
        )
        .expect("pen move");
        assert_eq!(
            pen_move.queue_class(),
            InputEventClass::PointerMove { device: 2 }
        );
    }

    #[test]
    fn golden_header_bytes_are_hand_checked() {
        // focus_gained: version 1, kind 5, timestamp 1e9 + 1.375e9 = 2_375_000_000 LE, Gained 1.
        let entry = golden()
            .into_iter()
            .find(|(name, _)| name == "focus_gained")
            .expect("focus_gained");
        assert_eq!(entry.1, "0105c09f8f8d0000000001");
        assert_eq!(
            2_375_000_000u64.to_le_bytes(),
            [0xc0, 0x9f, 0x8f, 0x8d, 0, 0, 0, 0]
        );
    }

    #[test]
    fn native_timestamps_convert_to_nanos() {
        assert_eq!(
            NativeTimestamp::MacEventSeconds(1.5).to_native_nanos(),
            Ok(1_500_000_000)
        );
        assert_eq!(
            NativeTimestamp::MacMachAbsolute {
                ticks: 24_000_000,
                numer: MACH_NUMER,
                denom: MACH_DENOM,
            }
            .to_native_nanos(),
            Ok(1_000_000_000)
        );
        assert_eq!(
            NativeTimestamp::WindowsQpc {
                ticks: 15_000_000,
                frequency: WIN_QPC_FREQUENCY,
            }
            .to_native_nanos(),
            Ok(1_500_000_000)
        );
    }

    #[test]
    fn invalid_native_timestamps_are_rejected() {
        assert_eq!(
            NativeTimestamp::MacEventSeconds(f64::NAN).to_native_nanos(),
            Err(InputEventError::NonFiniteTimestamp)
        );
        assert_eq!(
            NativeTimestamp::MacEventSeconds(f64::INFINITY).to_native_nanos(),
            Err(InputEventError::NonFiniteTimestamp)
        );
        assert_eq!(
            NativeTimestamp::MacEventSeconds(-0.5).to_native_nanos(),
            Err(InputEventError::NegativeTimestamp)
        );
        assert_eq!(
            NativeTimestamp::MacEventSeconds(1.0e12).to_native_nanos(),
            Err(InputEventError::TimestampOverflow)
        );
        assert_eq!(
            NativeTimestamp::WindowsQpc {
                ticks: 1,
                frequency: 0
            }
            .to_native_nanos(),
            Err(InputEventError::ZeroTimebase)
        );
        assert_eq!(
            NativeTimestamp::MacMachAbsolute {
                ticks: 1,
                numer: 1,
                denom: 0
            }
            .to_native_nanos(),
            Err(InputEventError::ZeroTimebase)
        );
        assert_eq!(
            NativeTimestamp::WindowsQpc {
                ticks: u64::MAX,
                frequency: 1
            }
            .to_native_nanos(),
            Err(InputEventError::TimestampOverflow)
        );
    }

    #[test]
    fn time_domain_maps_to_core_clock_and_stays_monotonic() {
        let clock = ManualClock::new(CORE_ANCHOR);
        let mut domain =
            InputTimeDomain::new(NativeTimestamp::MacEventSeconds(MAC_ANCHOR_SECONDS), &clock)
                .expect("domain");
        let t1 = domain
            .convert(NativeTimestamp::MacEventSeconds(100.5))
            .expect("t1");
        assert_eq!(t1, MonotonicNanos::new(CORE_ANCHOR + 500_000_000));
        // native 시각이 뒤로 가도(이벤트 원천 간 순서 뒤섞임) 결과는 감소하지 않습니다.
        let t2 = domain
            .convert(NativeTimestamp::MacEventSeconds(100.25))
            .expect("t2");
        assert_eq!(t2, t1);
        let t3 = domain
            .convert(NativeTimestamp::MacEventSeconds(101.0))
            .expect("t3");
        assert_eq!(t3, MonotonicNanos::new(CORE_ANCHOR + 1_000_000_000));
    }

    #[test]
    fn time_domain_clamps_before_core_origin() {
        // 기준점보다 오래된 native 이벤트는 코어 domain 0으로 고정됩니다(음수 없음).
        let mut domain = InputTimeDomain::from_anchor_nanos(5_000, 1_000);
        let early = domain
            .convert(NativeTimestamp::WindowsQpc {
                ticks: 0,
                frequency: 1_000_000_000,
            })
            .expect("early");
        assert_eq!(early, MonotonicNanos::new(0));
    }

    #[test]
    fn same_instant_from_each_platform_maps_to_same_core_time() {
        let clock = ManualClock::new(CORE_ANCHOR);
        let mut mac =
            InputTimeDomain::new(native_at(Platform::MacEventSeconds, 0.0), &clock).expect("mac");
        let mut win =
            InputTimeDomain::new(native_at(Platform::WindowsQpc, 0.0), &clock).expect("win");
        let a = mac
            .convert(native_at(Platform::MacEventSeconds, 0.75))
            .expect("mac t");
        let b = win
            .convert(native_at(Platform::WindowsQpc, 0.75))
            .expect("win t");
        assert_eq!(a, b);
        assert_eq!(a.get(), CORE_ANCHOR + 750_000_000);
    }

    #[test]
    fn utf16_ranges_convert_to_utf8_bytes() {
        // "한"(UTF-16 1, UTF-8 3), "😀"(UTF-16 2, UTF-8 4).
        let text = "a한😀b";
        assert_eq!(
            TextRange::from_utf16(text, 1, 1),
            TextRange::new(text, 1, 4)
        );
        assert_eq!(
            TextRange::from_utf16(text, 2, 2),
            TextRange::new(text, 4, 8)
        );
        assert_eq!(
            TextRange::from_utf16(text, 5, 0),
            TextRange::new(text, 9, 9)
        );
        // surrogate pair 중간, text 밖, overflow.
        assert_eq!(
            TextRange::from_utf16(text, 3, 0),
            Err(InputEventError::InvalidTextRange)
        );
        assert_eq!(
            TextRange::from_utf16(text, 6, 0),
            Err(InputEventError::InvalidTextRange)
        );
        assert_eq!(
            TextRange::from_utf16(text, u32::MAX, 1),
            Err(InputEventError::InvalidTextRange)
        );
        // UTF-8 문자 경계가 아닌 byte range.
        assert_eq!(
            TextRange::new(text, 2, 4),
            Err(InputEventError::InvalidTextRange)
        );
    }

    #[test]
    fn modifiers_reject_unknown_bits() {
        let combined = Modifiers::SHIFT | Modifiers::META;
        assert!(combined.contains(Modifiers::SHIFT));
        assert!(!combined.contains(Modifiers::ALT));
        assert_eq!(Modifiers::from_bits(combined.bits()), Ok(combined));
        assert_eq!(
            Modifiers::from_bits(0b10_0000),
            Err(InputEventError::UnknownModifierBits(0b10_0000))
        );
    }

    #[test]
    fn invalid_event_payloads_are_rejected() {
        let t = MonotonicNanos::new(1);
        let pointer = |phase, button, click_count| {
            InputEventKind::Pointer(PointerEvent {
                phase,
                button,
                device: PointerDevice::Mouse,
                position: Vec2::ZERO,
                modifiers: Modifiers::NONE,
                click_count,
            })
        };
        assert_eq!(
            InputEvent::new(t, pointer(PointerPhase::Down, PointerButton::None, 1)),
            Err(InputEventError::MissingPointerButton)
        );
        assert_eq!(
            InputEvent::new(t, pointer(PointerPhase::Up, PointerButton::Primary, 0)),
            Err(InputEventError::MissingClickCount)
        );
        assert_eq!(
            InputEvent::new(t, pointer(PointerPhase::Move, PointerButton::Primary, 1)),
            Err(InputEventError::UnexpectedClickCount)
        );
        assert_eq!(
            InputEvent::new(
                t,
                InputEventKind::Key(KeyEvent {
                    state: KeyState::Up,
                    key: KeyCode::new(0x04).expect("hid"),
                    repeat: true,
                    modifiers: Modifiers::NONE,
                })
            ),
            Err(InputEventError::RepeatOnKeyUp)
        );
        assert_eq!(KeyCode::new(0), Err(InputEventError::InvalidKeyCode));
        assert_eq!(
            InputEvent::new(
                t,
                InputEventKind::TextComposition(CompositionEvent {
                    phase: CompositionPhase::Cancel,
                    text: "가".to_string(),
                    selection: TextRange::EMPTY,
                })
            ),
            Err(InputEventError::CancelWithText)
        );
        assert_eq!(
            InputEvent::new(
                t,
                InputEventKind::TextComposition(CompositionEvent {
                    phase: CompositionPhase::Update,
                    text: "가".to_string(),
                    selection: TextRange { start: 0, end: 4 },
                })
            ),
            Err(InputEventError::InvalidTextRange)
        );
        assert_eq!(
            InputEvent::new(
                t,
                InputEventKind::Resize(ResizeEvent {
                    logical_size: Size2::ZERO,
                    scale: FiniteF32::ZERO,
                })
            ),
            Err(InputEventError::NonPositiveScale)
        );
    }

    #[test]
    fn every_fixture_event_round_trips() {
        for (name, event) in platform_events(Platform::MacEventSeconds) {
            let bytes = event.encode();
            assert_eq!(InputEvent::decode(&bytes), Ok(event), "{name}");
        }
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let focus = InputEvent::new(
            MonotonicNanos::new(7),
            InputEventKind::Focus(FocusEvent::Gained),
        )
        .expect("focus")
        .encode();

        let mut bad_version = focus.clone();
        bad_version[0] = 2;
        assert_eq!(
            InputEvent::decode(&bad_version),
            Err(InputEventError::UnsupportedVersion(2))
        );

        let mut bad_kind = focus.clone();
        bad_kind[1] = 9;
        assert_eq!(
            InputEvent::decode(&bad_kind),
            Err(InputEventError::UnknownKind(9))
        );

        let mut bad_tag = focus.clone();
        bad_tag[HEADER_LEN] = 3;
        assert_eq!(
            InputEvent::decode(&bad_tag),
            Err(InputEventError::UnknownTag {
                field: "focus",
                value: 3
            })
        );

        assert_eq!(
            InputEvent::decode(&focus[..focus.len() - 1]),
            Err(InputEventError::Truncated)
        );
        assert_eq!(InputEvent::decode(&[]), Err(InputEventError::Truncated));

        let mut trailing = focus.clone();
        trailing.push(0);
        assert_eq!(
            InputEvent::decode(&trailing),
            Err(InputEventError::TrailingBytes)
        );

        // key repeat bool 값 2.
        let key = InputEvent::new(
            MonotonicNanos::new(7),
            InputEventKind::Key(KeyEvent {
                state: KeyState::Down,
                key: KeyCode::new(0x04).expect("hid"),
                repeat: false,
                modifiers: Modifiers::NONE,
            }),
        )
        .expect("key")
        .encode();
        let mut bad_bool = key.clone();
        bad_bool[HEADER_LEN + 1] = 2;
        assert_eq!(
            InputEvent::decode(&bad_bool),
            Err(InputEventError::InvalidBool(2))
        );

        // resize width NaN, 음수 height.
        let resize = InputEvent::new(
            MonotonicNanos::new(7),
            InputEventKind::Resize(ResizeEvent {
                logical_size: Size2::new(1.0, 1.0).expect("size"),
                scale: FiniteF32::new(1.0).expect("finite"),
            }),
        )
        .expect("resize")
        .encode();
        let mut nan = resize.clone();
        nan[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            InputEvent::decode(&nan),
            Err(InputEventError::NonFiniteFloat)
        );
        let mut negative = resize.clone();
        negative[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(
            InputEvent::decode(&negative),
            Err(InputEventError::NegativeSize)
        );

        // text의 invalid UTF-8.
        let text = InputEvent::new(
            MonotonicNanos::new(7),
            InputEventKind::TextComposition(CompositionEvent {
                phase: CompositionPhase::Commit,
                text: "ab".to_string(),
                selection: TextRange::EMPTY,
            }),
        )
        .expect("text")
        .encode();
        let mut bad_utf8 = text.clone();
        let last = bad_utf8.len() - 1;
        bad_utf8[last] = 0xFF;
        assert_eq!(
            InputEvent::decode(&bad_utf8),
            Err(InputEventError::InvalidUtf8)
        );
    }

    #[test]
    fn errors_have_korean_display() {
        assert_eq!(
            InputEventError::UnknownKind(9).to_string(),
            "알 수 없는 입력 이벤트 kind입니다: 9"
        );
        assert_eq!(
            InputEventError::UnknownModifierBits(0x20).to_string(),
            "정의되지 않은 modifier bit입니다: 0x20"
        );
    }
}
