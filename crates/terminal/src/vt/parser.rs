//! VT escape sequence parser (XVIII.4.1).
//!
//! DEC ANSI 호환 터미널의 공개된 parser 상태 기계(ECMA-48 구문 + DEC VT500 계열 상태 전이)를 이 crate
//! 안에서 구현합니다. 입력 byte를 상태에 따라 다음 동작으로 나눠 [`Perform`]에 넘깁니다.
//!
//! - `print`: 출력 문자(UTF-8 해독 결과). 잘못된 UTF-8은 U+FFFD 하나로 바꿉니다(최대 부분 단위).
//! - `execute`: C0 제어 문자(BEL, BS, HT, LF, CR 등).
//! - `csi_dispatch`: `ESC [` 매개변수·중간 byte·최종 byte. `:` subparameter를 지원합니다.
//! - `esc_dispatch`: 그 밖의 `ESC` sequence.
//! - `osc_dispatch`: `ESC ]` 문자열(BEL 또는 ST로 끝남).
//! - DCS(`ESC P`)는 `hook`·`put`·`unhook`으로 넘기고, SOS·PM·APC 문자열은 버립니다.
//!
//! **상한**(악의적이거나 망가진 출력에도 메모리·시간이 입력 크기에 비례): 매개변수 [`MAX_PARAMS`]개,
//! 값은 `u16` 포화, 중간 byte [`MAX_INTERMEDIATES`]개(넘으면 sequence를 무시 표시), OSC
//! [`MAX_OSC_BYTES`] byte(넘으면 그 OSC를 버림). parser는 입력 byte마다 상수 시간에 동작하고 상태를
//! 넘는 버퍼를 늘리지 않습니다.
//!
//! **UTF-8 전용**: 8-bit C1 제어 byte(0x80~0x9F)는 해석하지 않습니다. UTF-8 연속 byte와 겹치기
//! 때문이며, C1 기능은 7-bit `ESC` 형식으로만 받습니다(macOS UTF-8 터미널 관례).

/// CSI·DCS 매개변수(subparameter 포함) 개수 상한.
pub const MAX_PARAMS: usize = 32;
/// 중간 byte(private marker 포함) 개수 상한.
pub const MAX_INTERMEDIATES: usize = 2;
/// OSC 문자열 길이 상한(byte).
pub const MAX_OSC_BYTES: usize = 8 * 1024;

/// parser가 넘기는 동작을 받는 쪽(터미널 emulator).
pub trait Perform {
    /// 출력 문자.
    fn print(&mut self, c: char);
    /// C0 제어 문자 실행.
    fn execute(&mut self, byte: u8);
    /// CSI sequence. `ignored`면 상한을 넘었거나 형식이 깨진 sequence이므로 실행하지 않아야 합니다.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignored: bool, action: u8);
    /// ESC sequence.
    fn esc_dispatch(&mut self, intermediates: &[u8], ignored: bool, byte: u8);
    /// OSC 문자열(`ESC ]`와 종료 문자 제외). `bell_terminated`면 BEL로 끝났습니다.
    fn osc_dispatch(&mut self, data: &[u8], bell_terminated: bool);
    /// DCS 시작. 기본 구현은 무시합니다.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignored: bool, _action: u8) {}
    /// DCS 본문 byte. 기본 구현은 무시합니다.
    fn put(&mut self, _byte: u8) {}
    /// DCS 끝. 기본 구현은 무시합니다.
    fn unhook(&mut self) {}
}

/// CSI·DCS 매개변수 목록. 값은 `;`로 나뉜 group이고, group 안의 값은 `:` subparameter입니다.
/// 빈 값은 0입니다(기본값 해석은 각 기능이 합니다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    values: [u16; MAX_PARAMS],
    /// 해당 값이 앞 값의 subparameter(`:` 뒤)인지.
    sub: [bool; MAX_PARAMS],
    len: usize,
    overflowed: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            values: [0; MAX_PARAMS],
            sub: [false; MAX_PARAMS],
            len: 0,
            overflowed: false,
        }
    }
}

impl Params {
    /// 값이 하나도 없는지(`CSI m`처럼 매개변수 없이 끝남).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 상한을 넘어 버린 값이 있는지.
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// group 목록. 각 group의 첫 값이 매개변수이고 나머지가 subparameter입니다.
    pub fn groups(&self) -> ParamGroups<'_> {
        ParamGroups {
            params: self,
            index: 0,
        }
    }

    /// `index`번째 group의 첫 값(없으면 `None`).
    pub fn get(&self, index: usize) -> Option<u16> {
        self.groups()
            .nth(index)
            .and_then(|group| group.first().copied())
    }

    /// `index`번째 group의 첫 값. 없거나 0이면 `default`(VT의 "0은 기본값" 규칙).
    pub fn get_or(&self, index: usize, default: u16) -> u16 {
        match self.get(index) {
            None | Some(0) => default,
            Some(value) => value,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }

    fn push(&mut self, value: u16, sub: bool) {
        if self.len >= MAX_PARAMS {
            self.overflowed = true;
            return;
        }
        self.values[self.len] = value;
        self.sub[self.len] = sub;
        self.len += 1;
    }

    fn add_digit(&mut self, digit: u8) {
        if self.overflowed || self.len == 0 {
            return;
        }
        let last = &mut self.values[self.len - 1];
        *last = last.saturating_mul(10).saturating_add(u16::from(digit));
    }
}

/// [`Params::groups`] 반복자.
#[derive(Debug)]
pub struct ParamGroups<'a> {
    params: &'a Params,
    index: usize,
}

impl<'a> Iterator for ParamGroups<'a> {
    type Item = &'a [u16];

    fn next(&mut self) -> Option<&'a [u16]> {
        let start = self.index;
        if start >= self.params.len {
            return None;
        }
        let mut end = start + 1;
        while end < self.params.len && self.params.sub[end] {
            end += 1;
        }
        self.index = end;
        Some(&self.params.values[start..end])
    }
}

/// parser 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    SosPmApcString,
}

/// UTF-8 해독 중간 상태.
#[derive(Debug, Clone, Copy, Default)]
struct Utf8 {
    code: u32,
    /// 남은 연속 byte 수.
    remaining: u8,
    /// 다음 연속 byte의 허용 범위(첫 연속 byte만 좁음).
    lower: u8,
    upper: u8,
}

/// VT parser. 입력을 여러 번에 나눠 넣어도 결과가 같습니다(상태가 byte 사이에 유지됨).
#[derive(Debug)]
pub struct Parser {
    state: State,
    params: Params,
    /// 현재 매개변수 값을 받는 중인지.
    param_open: bool,
    /// 마지막 구분자 뒤에 값(빈 값 포함)이 하나 더 있는지.
    separator_pending: bool,
    /// 다음 값이 subparameter인지.
    next_sub: bool,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_len: usize,
    ignoring: bool,
    osc: Vec<u8>,
    osc_overflowed: bool,
    utf8: Utf8,
}

impl Default for Parser {
    fn default() -> Self {
        Parser::new()
    }
}

impl Parser {
    /// 처음 상태(ground)의 parser.
    pub fn new() -> Parser {
        Parser {
            state: State::Ground,
            params: Params::default(),
            param_open: false,
            separator_pending: false,
            next_sub: false,
            intermediates: [0; MAX_INTERMEDIATES],
            intermediate_len: 0,
            ignoring: false,
            osc: Vec::new(),
            osc_overflowed: false,
            utf8: Utf8::default(),
        }
    }

    /// byte를 처리해 동작을 `performer`에 넘깁니다.
    pub fn advance<P: Perform + ?Sized>(&mut self, performer: &mut P, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(performer, byte);
        }
    }

    fn byte<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        if self.utf8.remaining > 0 {
            if (self.utf8.lower..=self.utf8.upper).contains(&byte) {
                self.utf8_continue(p, byte);
                return;
            }
            // 끊긴 sequence는 U+FFFD 하나로 바꾸고 이 byte를 새로 처리합니다.
            self.utf8 = Utf8::default();
            p.print(char::REPLACEMENT_CHARACTER);
        }
        // 어느 상태에서든 적용되는 전이.
        match byte {
            0x18 | 0x1A => {
                match self.state {
                    State::DcsPassthrough => p.unhook(),
                    State::OscString | State::SosPmApcString => {}
                    _ => p.execute(byte),
                }
                self.state = State::Ground;
                return;
            }
            0x1B => {
                match self.state {
                    State::OscString => self.osc_end(p, false),
                    State::DcsPassthrough => p.unhook(),
                    _ => {}
                }
                self.enter_escape();
                return;
            }
            _ => {}
        }
        match self.state {
            State::Ground => self.ground(p, byte),
            State::Escape => self.escape(p, byte),
            State::EscapeIntermediate => self.escape_intermediate(p, byte),
            State::CsiEntry => self.csi_entry(p, byte),
            State::CsiParam => self.csi_param(p, byte),
            State::CsiIntermediate => self.csi_intermediate(p, byte),
            State::CsiIgnore => self.csi_ignore(p, byte),
            State::DcsEntry => self.dcs_entry(p, byte),
            State::DcsParam => self.dcs_param(p, byte),
            State::DcsIntermediate => self.dcs_intermediate(p, byte),
            State::DcsPassthrough => self.dcs_passthrough(p, byte),
            State::DcsIgnore => {}
            State::OscString => self.osc_string(p, byte),
            State::SosPmApcString => {}
        }
    }

    // --- 공통 도우미 ---

    fn is_execute(byte: u8) -> bool {
        matches!(byte, 0x00..=0x17 | 0x19 | 0x1C..=0x1F)
    }

    fn clear_sequence(&mut self) {
        self.params.clear();
        self.param_open = false;
        self.separator_pending = false;
        self.next_sub = false;
        self.intermediate_len = 0;
        self.ignoring = false;
    }

    fn enter_escape(&mut self) {
        self.clear_sequence();
        self.state = State::Escape;
    }

    fn collect(&mut self, byte: u8) {
        if self.intermediate_len >= MAX_INTERMEDIATES {
            self.ignoring = true;
            return;
        }
        self.intermediates[self.intermediate_len] = byte;
        self.intermediate_len += 1;
    }

    fn param(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                if !self.param_open {
                    self.params.push(0, self.next_sub);
                    self.param_open = true;
                }
                self.params.add_digit(byte - b'0');
            }
            b';' | b':' => {
                if !self.param_open {
                    self.params.push(0, self.next_sub);
                }
                self.param_open = false;
                self.separator_pending = true;
                self.next_sub = byte == b':';
            }
            _ => {}
        }
    }

    fn finish_params(&mut self) {
        if self.separator_pending && !self.param_open {
            self.params.push(0, self.next_sub);
        }
        if self.params.overflowed() {
            self.ignoring = true;
        }
    }

    fn intermediates(&self) -> &[u8] {
        &self.intermediates[..self.intermediate_len]
    }

    // --- ground·UTF-8 ---

    fn ground<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            0x20..=0x7E => p.print(char::from(byte)),
            0x7F => {}
            0x00..=0x1F => p.execute(byte),
            0xC2..=0xDF => self.utf8_start(byte & 0x1F, 1, 0x80, 0xBF),
            0xE0 => self.utf8_start(0, 2, 0xA0, 0xBF),
            0xE1..=0xEC | 0xEE..=0xEF => self.utf8_start(byte & 0x0F, 2, 0x80, 0xBF),
            0xED => self.utf8_start(0x0D, 2, 0x80, 0x9F),
            0xF0 => self.utf8_start(0, 3, 0x90, 0xBF),
            0xF1..=0xF3 => self.utf8_start(byte & 0x07, 3, 0x80, 0xBF),
            0xF4 => self.utf8_start(0x04, 3, 0x80, 0x8F),
            // 단독 연속 byte, overlong 시작(C0·C1), 범위 밖(F5~FF).
            0x80..=0xC1 | 0xF5..=0xFF => p.print(char::REPLACEMENT_CHARACTER),
        }
    }

    fn utf8_start(&mut self, lead_bits: u8, remaining: u8, lower: u8, upper: u8) {
        self.utf8 = Utf8 {
            code: u32::from(lead_bits),
            remaining,
            lower,
            upper,
        };
    }

    fn utf8_continue<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        self.utf8.code = (self.utf8.code << 6) | u32::from(byte & 0x3F);
        self.utf8.remaining -= 1;
        self.utf8.lower = 0x80;
        self.utf8.upper = 0xBF;
        if self.utf8.remaining == 0 {
            let c = char::from_u32(self.utf8.code).unwrap_or(char::REPLACEMENT_CHARACTER);
            self.utf8 = Utf8::default();
            p.print(c);
        }
    }

    // --- ESC ---

    fn escape<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::EscapeIntermediate;
            }
            b'P' => {
                self.clear_sequence();
                self.state = State::DcsEntry;
            }
            b'[' => {
                self.clear_sequence();
                self.state = State::CsiEntry;
            }
            b']' => {
                self.osc.clear();
                self.osc_overflowed = false;
                self.state = State::OscString;
            }
            b'X' | b'^' | b'_' => self.state = State::SosPmApcString,
            0x30..=0x7E => {
                p.esc_dispatch(&[], false, byte);
                self.state = State::Ground;
            }
            0x7F => {}
            // 8-bit byte는 sequence를 끝내고 버립니다.
            _ => self.state = State::Ground,
        }
    }

    fn escape_intermediate<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            0x20..=0x2F => self.collect(byte),
            0x30..=0x7E => {
                let ignored = self.ignoring;
                p.esc_dispatch(self.intermediates(), ignored, byte);
                self.state = State::Ground;
            }
            0x7F => {}
            _ => self.state = State::Ground,
        }
    }

    // --- CSI ---

    fn csi_dispatch<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        self.finish_params();
        let ignored = self.ignoring;
        p.csi_dispatch(
            &self.params,
            &self.intermediates[..self.intermediate_len],
            ignored,
            byte,
        );
        self.state = State::Ground;
    }

    fn csi_entry<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::CsiIntermediate;
            }
            b'0'..=b'9' | b';' | b':' => {
                self.param(byte);
                self.state = State::CsiParam;
            }
            0x3C..=0x3F => {
                self.collect(byte);
                self.state = State::CsiParam;
            }
            0x40..=0x7E => self.csi_dispatch(p, byte),
            0x7F => {}
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_param<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            b'0'..=b'9' | b';' | b':' => self.param(byte),
            0x3C..=0x3F => self.state = State::CsiIgnore,
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => self.csi_dispatch(p, byte),
            0x7F => {}
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_intermediate<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            0x20..=0x2F => self.collect(byte),
            0x30..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.csi_dispatch(p, byte),
            0x7F => {}
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_ignore<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b if Self::is_execute(b) => p.execute(b),
            0x40..=0x7E => self.state = State::Ground,
            _ => {}
        }
    }

    // --- DCS ---

    fn dcs_hook<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        self.finish_params();
        let ignored = self.ignoring;
        p.hook(
            &self.params,
            &self.intermediates[..self.intermediate_len],
            ignored,
            byte,
        );
        self.state = State::DcsPassthrough;
    }

    fn dcs_entry<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::DcsIntermediate;
            }
            b'0'..=b'9' | b';' | b':' => {
                self.param(byte);
                self.state = State::DcsParam;
            }
            0x3C..=0x3F => {
                self.collect(byte);
                self.state = State::DcsParam;
            }
            0x40..=0x7E => self.dcs_hook(p, byte),
            0x80..=0xFF => self.state = State::DcsIgnore,
            _ => {}
        }
    }

    fn dcs_param<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            b'0'..=b'9' | b';' | b':' => self.param(byte),
            0x3C..=0x3F => self.state = State::DcsIgnore,
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::DcsIntermediate;
            }
            0x40..=0x7E => self.dcs_hook(p, byte),
            0x80..=0xFF => self.state = State::DcsIgnore,
            _ => {}
        }
    }

    fn dcs_intermediate<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            0x20..=0x2F => self.collect(byte),
            0x30..=0x3F => self.state = State::DcsIgnore,
            0x40..=0x7E => self.dcs_hook(p, byte),
            0x80..=0xFF => self.state = State::DcsIgnore,
            _ => {}
        }
    }

    fn dcs_passthrough<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        if byte != 0x7F {
            p.put(byte);
        }
    }

    // --- OSC ---

    fn osc_string<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match byte {
            0x07 => {
                self.osc_end(p, true);
                self.state = State::Ground;
            }
            0x00..=0x1F => {}
            _ => {
                if self.osc.len() < MAX_OSC_BYTES {
                    self.osc.push(byte);
                } else {
                    self.osc_overflowed = true;
                }
            }
        }
    }

    fn osc_end<P: Perform + ?Sized>(&mut self, p: &mut P, bell: bool) {
        if !self.osc_overflowed {
            p.osc_dispatch(&self.osc, bell);
        }
        self.osc.clear();
        // 큰 OSC 뒤에 buffer를 계속 쥐고 있지 않습니다.
        if self.osc.capacity() > 1024 {
            self.osc.shrink_to(1024);
        }
        self.osc_overflowed = false;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 받은 동작을 문자열로 기록합니다.
    #[derive(Debug, Default)]
    pub(crate) struct Recorder {
        pub(crate) events: Vec<String>,
        text: String,
    }

    impl Recorder {
        fn flush(&mut self) {
            if !self.text.is_empty() {
                let text = std::mem::take(&mut self.text);
                self.events.push(format!("print({text})"));
            }
        }

        pub(crate) fn finish(mut self) -> Vec<String> {
            self.flush();
            self.events
        }
    }

    fn groups(params: &Params) -> String {
        params
            .groups()
            .map(|group| {
                group
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(":")
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    impl Perform for Recorder {
        fn print(&mut self, c: char) {
            self.text.push(c);
        }

        fn execute(&mut self, byte: u8) {
            self.flush();
            self.events.push(format!("exec({byte:#04x})"));
        }

        fn csi_dispatch(&mut self, params: &Params, inter: &[u8], ignored: bool, action: u8) {
            self.flush();
            self.events.push(format!(
                "csi({}|{}{}{})",
                String::from_utf8_lossy(inter),
                groups(params),
                char::from(action),
                if ignored { "|ignored" } else { "" }
            ));
        }

        fn esc_dispatch(&mut self, inter: &[u8], ignored: bool, byte: u8) {
            self.flush();
            self.events.push(format!(
                "esc({}|{}{})",
                String::from_utf8_lossy(inter),
                char::from(byte),
                if ignored { "|ignored" } else { "" }
            ));
        }

        fn osc_dispatch(&mut self, data: &[u8], bell: bool) {
            self.flush();
            self.events.push(format!(
                "osc({}|{})",
                String::from_utf8_lossy(data),
                if bell { "bel" } else { "st" }
            ));
        }

        fn hook(&mut self, params: &Params, inter: &[u8], _ignored: bool, action: u8) {
            self.flush();
            self.events.push(format!(
                "hook({}|{}|{})",
                String::from_utf8_lossy(inter),
                groups(params),
                char::from(action)
            ));
        }

        fn put(&mut self, byte: u8) {
            self.events.push(format!("put({})", char::from(byte)));
        }

        fn unhook(&mut self) {
            self.events.push("unhook".to_owned());
        }
    }

    pub(crate) fn parse(input: &[u8]) -> Vec<String> {
        let mut parser = Parser::new();
        let mut recorder = Recorder::default();
        parser.advance(&mut recorder, input);
        recorder.finish()
    }

    /// 한 byte씩 나눠 넣어도 같은 결과입니다.
    fn parse_both(input: &[u8]) -> Vec<String> {
        let whole = parse(input);
        let mut parser = Parser::new();
        let mut recorder = Recorder::default();
        for byte in input {
            parser.advance(&mut recorder, std::slice::from_ref(byte));
        }
        assert_eq!(recorder.finish(), whole, "{input:?}");
        whole
    }

    #[test]
    fn csi_parameters_subparameters_and_markers() {
        assert_eq!(parse_both(b"\x1b[m"), ["csi(|m)"]);
        assert_eq!(parse_both(b"\x1b[1;2H"), ["csi(|1;2H)"]);
        assert_eq!(parse_both(b"\x1b[;H"), ["csi(|0;0H)"]);
        assert_eq!(parse_both(b"\x1b[;5H"), ["csi(|0;5H)"]);
        assert_eq!(parse_both(b"\x1b[5;H"), ["csi(|5;0H)"]);
        assert_eq!(
            parse_both(b"\x1b[38:2::10:20:30m"),
            ["csi(|38:2:0:10:20:30m)"]
        );
        assert_eq!(parse_both(b"\x1b[4:3;1m"), ["csi(|4:3;1m)"]);
        assert_eq!(parse_both(b"\x1b[?1049h"), ["csi(?|1049h)"]);
        assert_eq!(parse_both(b"\x1b[>c"), ["csi(>|c)"]);
        assert_eq!(parse_both(b"\x1b[2 q"), ["csi( |2q)"]);
        assert_eq!(parse_both(b"\x1b[!p"), ["csi(!|p)"]);
        // 값은 u16에서 포화합니다.
        assert_eq!(parse_both(b"\x1b[99999999C"), ["csi(|65535C)"]);
    }

    #[test]
    fn too_many_params_or_intermediates_mark_sequence_ignored() {
        let many = format!("\x1b[{}m", vec!["1"; MAX_PARAMS + 1].join(";"));
        let events = parse_both(many.as_bytes());
        assert_eq!(events.len(), 1);
        assert!(events[0].ends_with("m|ignored)"), "{events:?}");
        assert_eq!(parse_both(b"\x1b[1 !\"q"), ["csi( !|1q|ignored)"]);
        assert_eq!(parse_both(b"\x1b !\"B"), ["esc( !|B|ignored)"]);
    }

    #[test]
    fn controls_inside_sequences_are_executed_and_can_aborts() {
        // CSI 중간의 C0는 실행되고 sequence는 이어집니다.
        assert_eq!(parse_both(b"\x1b[1\n;2H"), ["exec(0x0a)", "csi(|1;2H)"]);
        // CAN·SUB는 sequence를 버리고 ground로 돌아갑니다.
        assert_eq!(parse_both(b"\x1b[12\x18A"), ["exec(0x18)", "print(A)"]);
        // 잘못된 위치의 private marker는 sequence 전체를 무시합니다.
        assert_eq!(parse_both(b"\x1b[1?2hX"), ["print(X)"]);
        // ESC는 진행 중인 sequence를 끊고 새 sequence를 시작합니다.
        assert_eq!(parse_both(b"\x1b[12\x1b7"), ["esc(|7)"]);
    }

    #[test]
    fn esc_sequences_and_charsets() {
        assert_eq!(
            parse_both(b"\x1b7\x1b8\x1bM\x1b(0\x1b)B\x1b#8"),
            ["esc(|7)", "esc(|8)", "esc(|M)", "esc((|0)", "esc()|B)", "esc(#|8)"]
        );
    }

    #[test]
    fn osc_strings_end_with_bel_or_st_and_are_bounded() {
        assert_eq!(
            parse_both(b"\x1b]0;title\x07after"),
            ["osc(0;title|bel)", "print(after)"]
        );
        assert_eq!(
            parse_both(b"\x1b]2;\xed\x95\x9c\x1b\\x"),
            ["osc(2;\u{d55c}|st)", "esc(|\\)", "print(x)"]
        );
        // 상한을 넘은 OSC는 버리고 뒤 출력은 정상 처리합니다.
        let mut long = b"\x1b]0;".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_OSC_BYTES + 10));
        long.extend_from_slice(b"\x07ok");
        assert_eq!(parse(&long), ["print(ok)"]);
        // CAN은 OSC를 실행하지 않고 끝냅니다.
        assert_eq!(parse_both(b"\x1b]0;x\x18y"), ["print(y)"]);
    }

    #[test]
    fn dcs_is_passed_through_and_sos_pm_apc_are_dropped() {
        assert_eq!(
            parse_both(b"\x1bP1$qm\x1b\\z"),
            ["hook($|1|q)", "put(m)", "unhook", "esc(|\\)", "print(z)"]
        );
        assert_eq!(parse_both(b"\x1b_hidden\x1b\\v"), ["esc(|\\)", "print(v)"]);
        assert_eq!(
            parse_both(b"\x1b^pm\x1b\\\x1bXsos\x1b\\w"),
            ["esc(|\\)", "esc(|\\)", "print(w)"]
        );
    }

    #[test]
    fn utf8_is_decoded_and_invalid_input_becomes_replacement() {
        assert_eq!(parse_both("한글 ok €𝄞".as_bytes()), ["print(한글 ok €𝄞)"]);
        // 단독 연속 byte, overlong, surrogate, 범위 밖, 끊긴 sequence.
        assert_eq!(parse_both(b"\x80"), ["print(\u{fffd})"]);
        assert_eq!(parse_both(b"\xc0\xaf"), ["print(\u{fffd}\u{fffd})"]);
        assert_eq!(
            parse_both(b"\xed\xa0\x80"),
            ["print(\u{fffd}\u{fffd}\u{fffd})"]
        );
        assert_eq!(parse_both(b"\xf5"), ["print(\u{fffd})"]);
        assert_eq!(parse_both(b"\xe2\x82A"), ["print(\u{fffd}A)"]);
        // UTF-8 중간에 제어 문자가 오면 U+FFFD 뒤 제어 문자를 실행합니다.
        assert_eq!(parse_both(b"\xe2\x82\r"), ["print(\u{fffd})", "exec(0x0d)"]);
        // DEL은 무시합니다.
        assert_eq!(parse_both(b"a\x7fb"), ["print(ab)"]);
    }

    #[test]
    fn params_accessors_apply_vt_defaults() {
        struct Capture(Option<Params>);
        impl Perform for Capture {
            fn print(&mut self, _: char) {}
            fn execute(&mut self, _: u8) {}
            fn csi_dispatch(&mut self, params: &Params, _: &[u8], _: bool, _: u8) {
                self.0 = Some(params.clone());
            }
            fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
            fn osc_dispatch(&mut self, _: &[u8], _: bool) {}
        }
        let mut capture = Capture(None);
        Parser::new().advance(&mut capture, b"\x1b[0;7;;3:4H");
        let params = capture.0.unwrap();
        assert_eq!(params.get(0), Some(0));
        assert_eq!(params.get_or(0, 1), 1);
        assert_eq!(params.get_or(1, 1), 7);
        assert_eq!(params.get_or(2, 9), 9);
        assert_eq!(params.groups().nth(3), Some(&[3, 4][..]));
        assert_eq!(params.get(4), None);
        assert!(!params.is_empty());
        assert!(!params.overflowed());
    }
}
