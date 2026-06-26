//! 안정 식별자 `StableId` (III.1.1).
//!
//! `StableId`는 128비트 식별자로, UUIDv7(RFC 9562) 레이아웃을 사용합니다.
//! 상위 48비트가 millisecond timestamp라서 바이트 사전식 정렬이 곧 시간 순 정렬입니다.
//! 직렬화는 16바이트(big-endian)와 표준 소문자 hyphen UUID 문자열을 제공합니다.
//!
//! 생성기는 timestamp와 random 필드를 명시 입력으로 받습니다(시계·엔트로피를 주입할 수
//! 있도록). 결정적 생성기는 [`crate::ids`] 후속 작업(III.1.2)에서 이 생성자를 사용합니다.

use std::fmt;
use std::str::FromStr;

use crate::clock::MonotonicClock;

/// 128비트 안정 식별자(UUIDv7 레이아웃).
///
/// 정렬은 바이트 순서를 따르며, timestamp가 최상위라 시간 순으로 정렬됩니다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId([u8; 16]);

impl StableId {
    /// 모든 바이트가 0인 nil 식별자.
    pub const NIL: StableId = StableId([0u8; 16]);

    /// 16바이트(big-endian)로부터 구성합니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> StableId {
        StableId(bytes)
    }

    /// 16바이트(big-endian) 표현을 반환합니다.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0
    }

    /// UUIDv7을 생성합니다.
    ///
    /// `unix_ts_ms`는 하위 48비트만 사용하고, `rand_a`는 하위 12비트, `rand_b`는 하위 62비트만
    /// 사용합니다. version(7)과 variant(0b10) 비트는 항상 RFC 9562에 맞게 설정됩니다.
    pub fn new_v7(unix_ts_ms: u64, rand_a: u16, rand_b: u64) -> StableId {
        let ts = unix_ts_ms & 0x0000_FFFF_FFFF_FFFF;
        let ra = rand_a & 0x0FFF;
        let rb = rand_b & 0x3FFF_FFFF_FFFF_FFFF;

        let mut bytes = [0u8; 16];
        bytes[0] = (ts >> 40) as u8;
        bytes[1] = (ts >> 32) as u8;
        bytes[2] = (ts >> 24) as u8;
        bytes[3] = (ts >> 16) as u8;
        bytes[4] = (ts >> 8) as u8;
        bytes[5] = ts as u8;
        // version 7 (상위 nibble) + rand_a 상위 4비트.
        bytes[6] = 0x70 | ((ra >> 8) as u8 & 0x0F);
        bytes[7] = (ra & 0xFF) as u8;
        // variant 0b10 (상위 2비트) + rand_b 상위 6비트.
        bytes[8] = 0x80 | ((rb >> 56) as u8 & 0x3F);
        bytes[9] = (rb >> 48) as u8;
        bytes[10] = (rb >> 40) as u8;
        bytes[11] = (rb >> 32) as u8;
        bytes[12] = (rb >> 24) as u8;
        bytes[13] = (rb >> 16) as u8;
        bytes[14] = (rb >> 8) as u8;
        bytes[15] = rb as u8;
        StableId(bytes)
    }

    /// UUID version nibble(v7이면 7).
    pub const fn version(&self) -> u8 {
        self.0[6] >> 4
    }

    /// UUID variant(RFC 9562이면 0b10 = 2).
    pub const fn variant(&self) -> u8 {
        self.0[8] >> 6
    }

    /// 상위 48비트 timestamp(milliseconds).
    pub const fn timestamp_ms(&self) -> u64 {
        ((self.0[0] as u64) << 40)
            | ((self.0[1] as u64) << 32)
            | ((self.0[2] as u64) << 24)
            | ((self.0[3] as u64) << 16)
            | ((self.0[4] as u64) << 8)
            | (self.0[5] as u64)
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
            b[14], b[15]
        )
    }
}

impl fmt::Debug for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StableId({self})")
    }
}

/// `StableId` 문자열 파싱 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStableIdError {
    /// 길이가 36자가 아님.
    Length,
    /// hyphen 위치가 표준과 다르거나 hex 문자가 아님.
    Format,
}

impl fmt::Display for ParseStableIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseStableIdError::Length => f.write_str("StableId 문자열 길이는 36이어야 합니다"),
            ParseStableIdError::Format => f.write_str("StableId 문자열 형식이 표준 UUID가 아닙니다"),
        }
    }
}

impl std::error::Error for ParseStableIdError {}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl FromStr for StableId {
    type Err = ParseStableIdError;

    /// 표준 hyphen UUID(대소문자 무관)를 파싱합니다. 출력은 항상 소문자입니다.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 36 {
            return Err(ParseStableIdError::Length);
        }
        if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
            return Err(ParseStableIdError::Format);
        }

        let mut out = [0u8; 16];
        let mut out_idx = 0;
        let mut high: Option<u8> = None;
        for (i, &c) in bytes.iter().enumerate() {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                continue;
            }
            let nibble = hex_value(c).ok_or(ParseStableIdError::Format)?;
            match high {
                None => high = Some(nibble),
                Some(h) => {
                    out[out_idx] = (h << 4) | nibble;
                    out_idx += 1;
                    high = None;
                }
            }
        }
        Ok(StableId(out))
    }
}

/// 결정적 PRNG(SplitMix64). 같은 seed면 항상 같은 수열을 생성합니다.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// 테스트용 결정적 `StableId` 생성기(III.1.2).
///
/// timestamp는 주입된 [`MonotonicClock`]에서, random 필드는 seed 기반 PRNG에서 가져옵니다.
/// 같은 seed와 같은 수동 시각 수열이면 항상 동일한 ID 시퀀스를 생성합니다.
pub struct StableIdGenerator {
    rng: SplitMix64,
}

impl StableIdGenerator {
    /// seed로 생성기를 만듭니다.
    pub fn new(seed: u64) -> StableIdGenerator {
        StableIdGenerator {
            rng: SplitMix64::new(seed),
        }
    }

    /// 주어진 시계의 현재 시각(ms = nanos / 1_000_000)으로 다음 `StableId`를 생성합니다.
    pub fn next_id<C: MonotonicClock>(&mut self, clock: &C) -> StableId {
        let unix_ts_ms = clock.now_nanos() / 1_000_000;
        let rand_a = (self.rng.next_u64() & 0x0FFF) as u16;
        let rand_b = self.rng.next_u64();
        StableId::new_v7(unix_ts_ms, rand_a, rand_b)
    }
}

/// 런타임 전용 엔티티 핸들(III.1.3).
///
/// `slot`/`generation`은 crate 내부에서만 접근 가능합니다(`pub(crate)`). 저장(persist)과
/// FFI 경계에는 노출되지 않습니다 — 외부에서는 불투명 핸들로만 다룹니다. 직렬화하지 않습니다.
///
/// slot이 재사용되면 generation이 증가하므로, 삭제된 핸들(옛 generation)로는 lookup이 실패합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeEntityId {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

/// `RuntimeEntityId`의 slot/generation 할당기(generational index).
///
/// `deallocate` 시 해당 slot의 generation을 올려 옛 핸들을 무효화하고, slot을 재사용 풀에 넣습니다.
#[derive(Debug, Default)]
pub struct RuntimeEntityAllocator {
    generations: Vec<u32>,
    free: Vec<u32>,
}

impl RuntimeEntityAllocator {
    /// 빈 할당기를 만듭니다.
    pub fn new() -> RuntimeEntityAllocator {
        RuntimeEntityAllocator {
            generations: Vec::new(),
            free: Vec::new(),
        }
    }

    /// 새 핸들을 할당합니다. 재사용 가능한 slot이 있으면 그 slot을(증가된 generation으로) 씁니다.
    pub fn allocate(&mut self) -> RuntimeEntityId {
        if let Some(slot) = self.free.pop() {
            let generation = self.generations[slot as usize];
            RuntimeEntityId { slot, generation }
        } else {
            let slot = self.generations.len() as u32;
            self.generations.push(0);
            RuntimeEntityId { slot, generation: 0 }
        }
    }

    /// 핸들이 현재 살아 있는지(generation 일치) 확인합니다.
    pub fn is_live(&self, id: RuntimeEntityId) -> bool {
        self.generations.get(id.slot as usize) == Some(&id.generation)
            && !self.free.contains(&id.slot)
    }

    /// 핸들을 해제합니다. generation을 올려 옛 핸들을 무효화하고 slot을 재사용 풀에 넣습니다.
    ///
    /// 이미 죽은 핸들이면 아무 것도 하지 않고 `false`를 반환합니다.
    pub fn deallocate(&mut self, id: RuntimeEntityId) -> bool {
        if !self.is_live(id) {
            return false;
        }
        let slot = id.slot as usize;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        self.free.push(id.slot);
        true
    }
}

/// `RuntimeEntityId`의 slot/generation은 crate 외부에서 접근할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::ids::RuntimeEntityAllocator;
/// let mut alloc = RuntimeEntityAllocator::new();
/// let id = alloc.allocate();
/// let _ = id.slot; // pub(crate) 필드 — 외부 접근 불가
/// ```
#[cfg(doctest)]
struct RuntimeEntityIdVisibilityDocs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;

    #[test]
    fn binary_round_trip() {
        let bytes = [
            0x01, 0x8f, 0x2a, 0x3b, 0x4c, 0x5d, 0x7e, 0x6f, 0x80, 0x11, 0x22, 0x33, 0x44, 0x55,
            0x66, 0x77,
        ];
        let id = StableId::from_bytes(bytes);
        assert_eq!(id.to_bytes(), bytes);
        assert_eq!(StableId::from_bytes(id.to_bytes()), id);
    }

    #[test]
    fn string_round_trip() {
        let id = StableId::new_v7(0x0192_3c4d_5e6f, 0x0abc, 0x0123_4567_89ab_cdef);
        let text = id.to_string();
        let parsed: StableId = text.parse().expect("parse");
        assert_eq!(parsed, id);
        // 출력은 소문자이고 다시 같은 문자열을 만든다.
        assert_eq!(parsed.to_string(), text);
    }

    #[test]
    fn rfc_format_version_and_variant() {
        let id = StableId::new_v7(0x0192_3c4d_5e6f, 0xFFFF, u64::MAX);
        assert_eq!(id.version(), 7);
        assert_eq!(id.variant(), 0b10);

        let text = id.to_string();
        assert_eq!(text.len(), 36);
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(chars[8], '-');
        assert_eq!(chars[13], '-');
        assert_eq!(chars[18], '-');
        assert_eq!(chars[23], '-');
        // version nibble은 14번째 문자(그룹 3 시작), variant nibble은 19번째 문자(그룹 4 시작).
        assert_eq!(chars[14], '7');
        assert!(matches!(chars[19], '8' | '9' | 'a' | 'b'));
    }

    #[test]
    fn timestamp_is_preserved_and_masked() {
        let id = StableId::new_v7(0x0192_3c4d_5e6f, 0, 0);
        assert_eq!(id.timestamp_ms(), 0x0192_3c4d_5e6f);
        // 48비트를 초과하는 timestamp는 마스킹됩니다.
        let over = StableId::new_v7(0xFFFF_FFFF_FFFF_FFFF, 0, 0);
        assert_eq!(over.timestamp_ms(), 0x0000_FFFF_FFFF_FFFF);
    }

    #[test]
    fn sorts_in_time_order() {
        let earlier = StableId::new_v7(1000, 0xFFF, u64::MAX);
        let later = StableId::new_v7(2000, 0, 0);
        // timestamp가 최상위라, rand가 작아도 더 늦은 timestamp가 더 큼.
        assert!(earlier < later);

        let mut ids = [later, earlier];
        ids.sort();
        assert_eq!(ids, [earlier, later]);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!("".parse::<StableId>(), Err(ParseStableIdError::Length));
        assert_eq!(
            "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz".parse::<StableId>(),
            Err(ParseStableIdError::Format)
        );
        // 길이는 36이지만 hyphen이 없음(위치 불일치).
        assert_eq!(
            "0123456789abcdef0123456789abcdef0123".parse::<StableId>(),
            Err(ParseStableIdError::Format)
        );
    }

    fn run_sequence(seed: u64) -> Vec<String> {
        // 1_700_000_000_000 ms를 nanoseconds로 시작.
        let mut clock = ManualClock::new(1_700_000_000_000u64 * 1_000_000);
        let mut generator = StableIdGenerator::new(seed);
        let mut out = Vec::new();
        for _ in 0..5 {
            out.push(generator.next_id(&clock).to_string());
            clock.advance(1_000_000); // +1ms
        }
        out
    }

    #[test]
    fn golden_sequence_is_stable() {
        let expected = [
            "018bcfe5-6800-7e95-a8ef-e333b266f103",
            "018bcfe5-6801-7f52-981c-e1ff0e4ae394",
            "018bcfe5-6802-73f2-9e44-31fa3c80db06",
            "018bcfe5-6803-7d5d-8cf6-35ee9e9e2fa4",
            "018bcfe5-6804-7dd5-9e54-d738297f77ae",
        ];
        assert_eq!(run_sequence(42), expected);
    }

    #[test]
    fn same_seed_and_time_is_deterministic() {
        assert_eq!(run_sequence(42), run_sequence(42));
    }

    #[test]
    fn different_seed_differs() {
        assert_ne!(run_sequence(42), run_sequence(43));
    }

    #[test]
    fn generated_ids_are_v7_and_time_ordered() {
        let mut clock = ManualClock::new(1_700_000_000_000u64 * 1_000_000);
        let mut generator = StableIdGenerator::new(7);
        let first = generator.next_id(&clock);
        clock.advance(5 * 1_000_000);
        let second = generator.next_id(&clock);
        assert_eq!(first.version(), 7);
        assert_eq!(first.variant(), 0b10);
        assert!(first < second);
    }

    #[test]
    fn reused_slot_invalidates_old_generation() {
        let mut alloc = RuntimeEntityAllocator::new();
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert!(alloc.is_live(a));
        assert!(alloc.is_live(b));
        assert_ne!(a.slot, b.slot);

        // a 해제 → 옛 핸들 무효.
        assert!(alloc.deallocate(a));
        assert!(!alloc.is_live(a));
        // 중복 해제는 실패.
        assert!(!alloc.deallocate(a));

        // 같은 slot 재사용 → generation 증가, 옛 핸들 lookup 실패, 새 핸들만 live.
        let c = alloc.allocate();
        assert_eq!(c.slot, a.slot);
        assert_ne!(c.generation, a.generation);
        assert!(alloc.is_live(c));
        assert!(!alloc.is_live(a));
    }

    #[test]
    fn fresh_slots_are_sequential() {
        let mut alloc = RuntimeEntityAllocator::new();
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert_eq!(a.slot, 0);
        assert_eq!(b.slot, 1);
        assert_eq!(a.generation, 0);
        assert_eq!(b.generation, 0);
    }

    #[test]
    fn parse_accepts_uppercase() {
        let id = StableId::new_v7(0x0192_3c4d_5e6f, 0x0abc, 0x0123_4567_89ab_cdef);
        let upper = id.to_string().to_uppercase();
        let parsed: StableId = upper.parse().expect("parse uppercase");
        assert_eq!(parsed, id);
    }
}
