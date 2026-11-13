//! 터미널 세션 식별자 (XVIII.1.1).
//!
//! [`TerminalSessionId`]는 **세션 메모리 전용**입니다. 프로젝트 파일·복구 snapshot·설정에
//! 저장하지 않으며(기술 아키텍처 22.3 "복구하지 않는 대상: 터미널 프로세스와 출력"), 저장해도 다시
//! 쓸 수 없게 다음을 보장합니다.
//!
//! - 식별자는 발급한 [`SessionIdAllocator`](registry마다 하나)의 epoch를 품습니다. epoch는
//!   프로세스마다 무작위인 기준값에 프로세스 안 단조 증가 counter를 더해 만듭니다. 그래서 다른
//!   registry나 앱을 다시 실행한 뒤의 registry는 같은 handle을 해석하지 않습니다(우연히 일치할
//!   확률은 epoch 32bit 충돌 수준).
//! - 직렬화 trait을 구현하지 않고, 문자열에서 식별자를 만드는 parser도 두지 않습니다. FFI 경계로
//!   넘길 때는 [`TerminalSessionId::to_handle`]의 `u64`만 쓰고, 되돌릴 때는 발급한 allocator가
//!   epoch와 발급 범위를 확인합니다([`SessionIdAllocator::decode`]).

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

/// 터미널 세션 식별자. 발급한 registry 안에서만 의미가 있습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalSessionId {
    epoch: u32,
    seq: u32,
}

impl TerminalSessionId {
    /// FFI·UI 경계로 넘길 불투명 handle(`epoch << 32 | seq`). 저장하지 않습니다.
    pub fn to_handle(self) -> u64 {
        (u64::from(self.epoch) << 32) | u64::from(self.seq)
    }

    /// 같은 registry 안에서의 발급 순번(1부터, 재사용 없음).
    pub fn sequence(self) -> u32 {
        self.seq
    }
}

impl fmt::Display for TerminalSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "terminal-session-{:08x}-{}", self.epoch, self.seq)
    }
}

/// 한 allocator가 발급할 수 있는 식별자(`u32` 순번)를 모두 썼습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIdExhausted;

impl fmt::Display for SessionIdExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("터미널 세션 식별자를 더 발급할 수 없습니다")
    }
}

impl std::error::Error for SessionIdExhausted {}

/// registry 하나의 식별자 발급기. 순번은 재사용하지 않습니다.
#[derive(Debug)]
pub struct SessionIdAllocator {
    epoch: u32,
    /// 다음 순번. `0`은 순번 공간을 다 썼다는 표시입니다(`u32::MAX` 다음 wrap).
    next: u32,
}

impl SessionIdAllocator {
    /// 새 epoch를 가진 발급기.
    pub fn new() -> SessionIdAllocator {
        SessionIdAllocator {
            epoch: next_epoch(),
            next: 1,
        }
    }

    /// 새 식별자를 발급합니다.
    pub fn allocate(&mut self) -> Result<TerminalSessionId, SessionIdExhausted> {
        if self.next == 0 {
            return Err(SessionIdExhausted);
        }
        let id = TerminalSessionId {
            epoch: self.epoch,
            seq: self.next,
        };
        self.next = self.next.wrapping_add(1);
        Ok(id)
    }

    /// 이 발급기가 발급한 적이 있는 식별자인지.
    pub fn issued(&self, id: TerminalSessionId) -> bool {
        id.epoch == self.epoch && id.seq != 0 && (self.next == 0 || id.seq < self.next)
    }

    /// handle을 이 발급기의 식별자로 되돌립니다. 다른 발급기(다른 registry, 이전 실행)의 handle과
    /// 아직 발급하지 않은 순번은 `None`입니다.
    pub fn decode(&self, handle: u64) -> Option<TerminalSessionId> {
        let id = TerminalSessionId {
            epoch: (handle >> 32) as u32,
            seq: handle as u32,
        };
        self.issued(id).then_some(id)
    }

    /// 테스트용: 다음 순번을 지정합니다.
    #[cfg(test)]
    pub(crate) fn with_next(mut self, next: u32) -> SessionIdAllocator {
        self.next = next;
        self
    }
}

impl Default for SessionIdAllocator {
    fn default() -> Self {
        SessionIdAllocator::new()
    }
}

/// 프로세스마다 무작위인 기준값 + 단조 증가 counter. `0`은 쓰지 않습니다.
fn next_epoch() -> u32 {
    static BASE: OnceLock<u32> = OnceLock::new();
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let base = *BASE.get_or_init(|| {
        // RandomState는 프로세스마다 OS 난수로 seed됩니다.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.finish() as u32
    });
    loop {
        let epoch = base.wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed));
        if epoch != 0 {
            return epoch;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_never_reused() {
        let mut allocator = SessionIdAllocator::new();
        let first = allocator.allocate().unwrap();
        let second = allocator.allocate().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.sequence(), 1);
        assert_eq!(second.sequence(), 2);
        assert!(first < second);
    }

    #[test]
    fn handle_round_trip_is_limited_to_the_issuing_allocator() {
        let mut allocator = SessionIdAllocator::new();
        let id = allocator.allocate().unwrap();
        // 같은 발급기(같은 앱 실행의 같은 registry) 안에서는 round-trip됩니다.
        assert_eq!(allocator.decode(id.to_handle()), Some(id));

        // 다른 registry나 재실행 뒤의 registry(새 epoch)는 해석하지 않습니다.
        let mut other = SessionIdAllocator::new();
        let _ = other.allocate().unwrap();
        assert_eq!(other.decode(id.to_handle()), None);
        assert!(!other.issued(id));

        // 아직 발급하지 않은 순번과 순번 0도 거부합니다.
        let unissued = (id.to_handle() & !0xffff_ffff) | 99;
        assert_eq!(allocator.decode(unissued), None);
        assert_eq!(allocator.decode(id.to_handle() & !0xffff_ffff), None);
    }

    #[test]
    fn exhausted_allocator_reports_error() {
        let mut allocator = SessionIdAllocator::new().with_next(u32::MAX);
        let last = allocator.allocate().unwrap();
        assert_eq!(last.sequence(), u32::MAX);
        assert_eq!(allocator.allocate(), Err(SessionIdExhausted));
        // 다 쓴 뒤에도 이미 발급한 식별자는 해석합니다.
        assert_eq!(allocator.decode(last.to_handle()), Some(last));
    }

    #[test]
    fn epochs_differ_between_allocators() {
        let a = SessionIdAllocator::new();
        let b = SessionIdAllocator::new();
        assert_ne!(a.epoch, b.epoch);
        assert_ne!(a.epoch, 0);
    }
}
