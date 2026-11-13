//! IPC session nonce (XIX.1.3).
//!
//! nonce는 OS CSPRNG(`/dev/urandom`)에서 얻은 128 bit 값이며 소문자 16진수 32자로 주고받습니다.
//! [`NonceLedger`]는 한 프로세스가 본 nonce를 기억해 재사용을 거부합니다.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::io::{self, Read};

/// nonce byte 수.
pub const NONCE_BYTES: usize = 16;
/// 16진수 표기 길이.
pub const NONCE_HEX_LEN: usize = NONCE_BYTES * 2;
/// [`NonceLedger::new`] 기본 기억 개수.
pub const DEFAULT_LEDGER_CAPACITY: usize = 4096;

/// 128 bit session nonce.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Nonce([u8; NONCE_BYTES]);

impl fmt::Debug for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Nonce({})", self.to_hex())
    }
}

impl Nonce {
    /// OS CSPRNG에서 새 nonce를 만듭니다.
    pub fn generate() -> io::Result<Nonce> {
        let mut bytes = [0u8; NONCE_BYTES];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(Nonce(bytes))
    }

    /// byte 값에서 만듭니다(테스트·재현용).
    pub fn from_bytes(bytes: [u8; NONCE_BYTES]) -> Nonce {
        Nonce(bytes)
    }

    /// 소문자 16진수 32자 표기를 읽습니다. 대문자·길이 불일치·기타 문자는 거부합니다.
    pub fn parse_hex(text: &str) -> Option<Nonce> {
        let b = text.as_bytes();
        if b.len() != NONCE_HEX_LEN {
            return None;
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; NONCE_BYTES];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (digit(b[2 * i])? << 4) | digit(b[2 * i + 1])?;
        }
        Some(Nonce(out))
    }

    /// 소문자 16진수 32자 표기.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// 이미 사용한 nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonceReused;

impl fmt::Display for NonceReused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("이미 사용한 session nonce입니다")
    }
}

impl std::error::Error for NonceReused {}

/// 본 nonce를 기억해 재사용을 거부하는 장부.
///
/// 메모리 상한을 위해 최근 `capacity`개만 기억합니다. 128 bit 무작위 nonce는 우연히 겹칠 확률이
/// 무시할 수준이므로, 상한은 재전송 공격 창을 최근 `capacity`개 session으로 제한합니다.
#[derive(Debug, Clone)]
pub struct NonceLedger {
    seen: HashSet<Nonce>,
    order: VecDeque<Nonce>,
    capacity: usize,
}

impl Default for NonceLedger {
    fn default() -> Self {
        NonceLedger::new()
    }
}

impl NonceLedger {
    /// 기본 상한([`DEFAULT_LEDGER_CAPACITY`])의 빈 장부.
    pub fn new() -> NonceLedger {
        NonceLedger::with_capacity(DEFAULT_LEDGER_CAPACITY)
    }

    /// 상한을 지정한 빈 장부(최소 1).
    pub fn with_capacity(capacity: usize) -> NonceLedger {
        NonceLedger {
            seen: HashSet::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// 처음 보는 nonce면 기록하고, 이미 봤으면 [`NonceReused`]입니다.
    pub fn record(&mut self, nonce: Nonce) -> Result<(), NonceReused> {
        if !self.seen.insert(nonce) {
            return Err(NonceReused);
        }
        self.order.push_back(nonce);
        if self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        Ok(())
    }

    /// 기록 여부.
    pub fn contains(&self, nonce: &Nonce) -> bool {
        self.seen.contains(nonce)
    }

    /// 기억 중인 nonce 수.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// 비었는지 여부.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_is_strict() {
        let nonce = Nonce::from_bytes([0xab; NONCE_BYTES]);
        assert_eq!(nonce.to_hex(), "ab".repeat(NONCE_BYTES));
        assert_eq!(Nonce::parse_hex(&nonce.to_hex()), Some(nonce));
        for bad in [
            "AB".repeat(NONCE_BYTES),
            "ab".repeat(NONCE_BYTES - 1),
            format!("{}0", "ab".repeat(NONCE_BYTES)),
            format!("{}zz", "ab".repeat(NONCE_BYTES - 1)),
            String::new(),
        ] {
            assert_eq!(Nonce::parse_hex(&bad), None, "{bad}");
        }
    }

    #[test]
    fn generated_nonces_are_unique() {
        let mut ledger = NonceLedger::new();
        for _ in 0..1000 {
            ledger.record(Nonce::generate().unwrap()).unwrap();
        }
        assert_eq!(ledger.len(), 1000);
    }

    #[test]
    fn ledger_rejects_reuse_and_bounds_memory() {
        let a = Nonce::from_bytes([1; NONCE_BYTES]);
        let b = Nonce::from_bytes([2; NONCE_BYTES]);
        let c = Nonce::from_bytes([3; NONCE_BYTES]);
        let mut ledger = NonceLedger::with_capacity(2);
        assert_eq!(ledger.record(a), Ok(()));
        assert_eq!(ledger.record(a), Err(NonceReused));
        assert_eq!(ledger.record(b), Ok(()));
        assert_eq!(ledger.record(b), Err(NonceReused));
        assert_eq!(ledger.record(c), Ok(()));
        // 상한 2: 가장 오래된 a를 잊고 b·c를 기억합니다.
        assert_eq!(ledger.len(), 2);
        assert!(!ledger.contains(&a));
        assert!(ledger.contains(&b) && ledger.contains(&c));
    }
}
