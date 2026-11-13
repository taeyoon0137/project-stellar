//! chunk envelope·canonical decoder fuzz harness (XVI.6.2).
//!
//! XVI.6.1과 같은 방식으로, 외부 fuzz 도구(cargo-fuzz/libFuzzer)는 새 의존성과 nightly
//! toolchain이 필요하므로 저장소 안의 **결정론적** harness를 `#[test]`로 실행합니다.
//!
//! - **입력**: `tests/fuzz_corpus/chunk_canonical/`의 seed corpus(hex 텍스트), 구조를 아는
//!   생성기가 만든 유효 bytes(canonical op stream, chunk 파일, pointer, generation manifest,
//!   ViewTree payload, hash 문자열), 고정 stress 입력(최대 count·길이 프리픽스, 1 MiB 대량 항목,
//!   다중 byte UTF-8 hash 문자열 등).
//! - **변형**: 고정 seed SplitMix64로 bit 뒤집기·경계 byte·삽입·구간 삭제·`u32`/`u64` 경계값
//!   덮어쓰기·token 삽입·구간 복제·splice·구간 반복·절단·인접 구간 교환·같은 길이 다중 byte
//!   UTF-8 덮어쓰기를 입력당 1~8회 적용합니다(64 KiB 상한). 같은 seed는 같은 입력열을 만듭니다.
//! - **대상(입력마다 전부)**:
//!   1. canonical op stream: 입력 자체가 연산 순서를 정합니다(op byte → `take_*`). map value와
//!      field payload는 중첩 stream으로 다시 디코딩합니다(깊이 [`STREAM_MAX_DEPTH`]).
//!   2. `ChunkEnvelope::decode` → kind registry 해석, object 경로 생성.
//!   3. `ChunkPointer::decode`, 4. `GenerationManifest::decode`, 5. `ViewTreeChunk::decode`.
//!   6. `ContentHash`·`ChunkId` 문자열 parse(current pointer 형식 포함).
//!   7. 디스크 chunk 읽기: 입력을 chunk object 경로에 기록한 뒤 `ChunkStore::read_chunk`와
//!      `write_chunk`(기존 파일 검증) 실행.
//! - **판정(입력마다)**: (1) panic 없음(`catch_unwind`), (2) hang 없음(입력당 시간 상한 + 전체
//!   watchdog), (3) 과도한 할당 없음(제품 decoder 호출 하나의 peak heap ≤ [`alloc_limit`]; harness
//!   쪽 재인코딩·중첩 재귀는 제외), (4) 수용한 입력의
//!   canonical 성질: 다시 인코딩하면 소비한 bytes와 byte-identical, hash 문자열은 표기 그대로
//!   재기록, chunk 읽기 성공은 요청 주소의 canonical 파일일 때만, 읽기·쓰기는 파일을 수정하지
//!   않음.
//!
//! 발견한 입력은 `CARGO_TARGET_TMPDIR/chunk-canonical-fuzz/`에 저장하고 테스트를 실패시킵니다.
//! 반복 수 기본값은 고정이며 `STELLAR_STORAGE_FUZZ_ITERATIONS`로 늘릴 수 있습니다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::{Duration, Instant};

use stellar_core_types::ids::StableId;
use stellar_storage::canonical::decode::CanonicalDecoder;
use stellar_storage::canonical::encode::{CanonicalEncoder, Field};
use stellar_storage::chunk::envelope::{
    ChunkEnvelope, ChunkKindCode, CHUNK_FORMAT_VERSION, CHUNK_MAGIC,
};
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kind_registry::ChunkKindRegistry;
use stellar_storage::chunk::kinds::{initial_kind_registry, INITIAL_KINDS, KIND_VIEW_TREE};
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::chunk::store::{ChunkStore, ChunkStoreError, WriteOutcome};
use stellar_storage::chunk::view_tree::{SubtreeRef, ViewTreeChunk};
use stellar_storage::generation::current::{
    current_pointer_path, read_current_pointer, CurrentPointerError,
};
use stellar_storage::generation::manifest::{
    GenerationManifest, MANIFEST_FORMAT_VERSION, MANIFEST_MAGIC,
};
use stellar_storage::generation::scanner::scan_latest_valid_generation;
use stellar_storage::hash::{ContentHash, ParseContentHashError};

// --- 할당 계측 ---

/// System 할당자에 위임하면서 현재 thread의 할당량(현재·peak)을 기록하는 할당자.
struct CountingAlloc;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

/// 계측 중이면 현재 thread의 할당량을 갱신합니다(할당 없음).
fn record(delta: isize) {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = CURRENT.try_with(|current| {
                let now = current.get() + delta;
                current.set(now);
                let _ = PEAK.try_with(|peak| {
                    if now > peak.get() {
                        peak.set(now);
                    }
                });
            });
        }
    });
}

/// 크기를 isize로 변환합니다(Layout 크기는 isize::MAX 이하가 보장됨).
fn signed(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

// SAFETY: 모든 연산을 System 할당자에 그대로 위임하고, 부가 작업(크기 기록)은 할당하지 않습니다.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(signed(layout.size()));
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc_zeroed 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(signed(layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr은 같은 layout으로 이 할당자(=System)에서 할당된 포인터입니다.
        unsafe { System.dealloc(ptr, layout) };
        record(-signed(layout.size()));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::realloc 계약을 지키며, 그대로 System에 전달합니다.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // 이동 순간 두 블록이 공존할 수 있으므로 새 크기를 먼저 더합니다(보수적 peak).
            record(signed(new_size));
            record(-signed(layout.size()));
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

/// `f` 실행 동안 현재 thread의 peak heap 증가량(bytes)을 잽니다.
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
    CURRENT.with(|c| c.set(0));
    PEAK.with(|p| p.set(0));
    TRACKING.with(|t| t.set(true));
    let out = f();
    TRACKING.with(|t| t.set(false));
    let peak = usize::try_from(PEAK.with(Cell::get)).unwrap_or(0);
    (out, peak)
}

// --- panic 기록 ---

thread_local! {
    /// fuzz 대상 실행 중이면 기본 panic 출력을 생략합니다(payload는 catch_unwind로 보고).
    static QUIET: Cell<bool> = const { Cell::new(false) };
    /// 마지막 panic 위치(파일:줄).
    static LAST_PANIC_AT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// fuzz worker thread에서만 panic 출력을 생략하고 위치를 기록하는 hook을 한 번 설치합니다.
fn install_quiet_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if QUIET.with(Cell::get) {
                let at = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                LAST_PANIC_AT.with(|slot| *slot.borrow_mut() = at);
            } else {
                default(info);
            }
        }));
    });
}

/// `f`를 panic 출력 없이 실행하고 panic을 `Err`(payload 메시지)로 돌려줍니다.
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let previous = QUIET.with(|q| q.replace(true));
    let result = catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|q| q.set(previous));
    // 계측 중 panic이면 계측 flag가 켜진 채 남으므로 끕니다.
    TRACKING.with(|t| t.set(false));
    result.map_err(|payload| panic_message(payload.as_ref()))
}

/// panic payload를 문자열로.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    };
    let at = LAST_PANIC_AT.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    if at.is_empty() {
        message
    } else {
        format!("{message} (위치 {at})")
    }
}

// --- 설정 ---

/// 고정 seed.
const SEED: u64 = 0x5354_4f52_4147_4546;
/// 기본 변형 반복 수.
const DEFAULT_ITERATIONS: usize = 25_000;
/// 변형 입력 길이 상한(실행 시간 고정).
const MAX_MUTATED_LEN: usize = 64 * 1024;
/// 대량 stress 입력 크기.
const STRESS_LEN: usize = 1 << 20;
/// 입력당 시간 상한(debug 빌드 기준, 넉넉한 값).
const PER_INPUT_LIMIT: Duration = Duration::from_secs(3);
/// 진행 소식이 이 시간 동안 없으면 hang으로 판정합니다.
const WATCHDOG: Duration = Duration::from_secs(60);
/// 제품 decoder 호출 할당 계수(입력 1 byte당 허용 bytes). 이론 최대(map entry 48 bytes × Vec
/// 증설 시 이전·새 블록 공존 3배 ÷ entry 최소 9 bytes ≈ 16)의 4배 여유.
const ALLOC_FACTOR: usize = 64;
/// 할당 상한 고정 여유(bytes): manifest·ViewTree decoder의 최대 1,024개 선할당 몫 포함.
const ALLOC_SLACK: usize = 256 * 1024;
/// op stream 중첩 디코딩 깊이 상한(harness 쪽 제한, 제품 decoder는 재귀하지 않음).
const STREAM_MAX_DEPTH: usize = 3;
/// op stream의 op 종류 수(op byte % 16).
const OP_COUNT: u8 = 16;
/// field 디코딩에서 아는 tag 목록.
const KNOWN_TAGS: &[u32] = &[0, 1, 2, 3, 5, 8, 13, u32::MAX];

/// 입력 길이별 제품 decoder 호출 하나의 peak heap 상한.
fn alloc_limit(len: usize) -> usize {
    ALLOC_FACTOR * len + ALLOC_SLACK
}

// --- 결정론적 난수 ---

/// SplitMix64 난수기.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` 범위(n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// `num/den` 확률로 참.
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next_u64() % den < num
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        (0..self.below(max + 1))
            .map(|_| self.next_u64() as u8)
            .collect()
    }

    fn id16(&mut self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.next_u64().to_be_bytes());
        out[8..].copy_from_slice(&self.next_u64().to_be_bytes());
        out
    }
}

// --- 변형 ---

/// `u32` 길이·count 경계값.
const U32_EDGES: &[u32] = &[
    0,
    1,
    2,
    3,
    4,
    7,
    8,
    15,
    16,
    32,
    64,
    70,
    71,
    72,
    255,
    256,
    1024,
    0xffff,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0x8000_0003,
    0xffff_fffe,
    0xffff_ffff,
];

/// `u64` 경계값.
const U64_EDGES: &[u64] = &[
    0,
    1,
    4,
    0xff,
    0xffff_ffff,
    0x1_0000_0000,
    0x7fff_ffff_ffff_ffff,
    0x8000_0000_0000_0000,
    u64::MAX,
];

/// 변형에 삽입하는 조각(magic, hash 문자열 조각, 다중 byte·비UTF-8 sequence, 비canonical float).
const TOKENS: &[&[u8]] = &[
    b"STARCHNK",
    b"STARMFST",
    b"sha256:",
    b"SHA256:",
    b"blake3:",
    b"sha256-",
    b":",
    b"-",
    b"0123456789abcdef",
    b"ABCDEF",
    b"g",
    b"\n",
    b"\r\n",
    b"\0",
    b".starchunk",
    b"../",
    "é".as_bytes(),
    "€".as_bytes(),
    "한".as_bytes(),
    "😀".as_bytes(),
    "\u{FEFF}".as_bytes(),
    &[0xc0, 0x80],
    &[0xed, 0xa0, 0x80],
    &[0x80],
    &[0xff, 0xfe],
    &[0xf4, 0x90, 0x80, 0x80],
    &[0x00, 0x00, 0xc0, 0x7f],
    &[0x00, 0x00, 0x80, 0x7f],
    &[0x00, 0x00, 0x80, 0xff],
    &[0x00, 0x00, 0x00, 0x80],
    &[0, 0, 0, 0, 0, 0, 0xf8, 0x7f],
    &[0, 0, 0, 0, 0, 0, 0xf0, 0x7f],
    &[0, 0, 0, 0, 0, 0, 0, 0x80],
    &[0x01, 0, 0, 0],
    &[0xff, 0xff, 0xff, 0xff],
    &[0x47, 0, 0, 0],
    &[0x02],
    &[0x01, 0, 0, 0, 0x01],
    &[0x03, 0, 0, 0x80],
];

/// 같은 길이 덮어쓰기에 쓰는 다중 byte 문자(2·3·4 byte).
const MULTIBYTE: &[char] = &[
    'é',
    'ß',
    '\u{80}',
    '\u{7ff}',
    '€',
    '한',
    '\u{800}',
    '\u{ffff}',
    '😀',
    '\u{10000}',
    '\u{10ffff}',
];

/// 입력 하나에 1~8회 변형을 적용합니다.
fn mutate(rng: &mut Rng, mut data: Vec<u8>, seeds: &[Seed]) -> Vec<u8> {
    for _ in 0..1 + rng.below(8) {
        let len = data.len();
        match rng.below(14) {
            // bit 뒤집기.
            0 if len > 0 => {
                let at = rng.below(len);
                data[at] ^= 1 << rng.below(8);
            }
            // 경계 byte 값.
            1 if len > 0 => {
                let at = rng.below(len);
                data[at] = *rng.pick(&[0x00, 0x01, 0x02, 0x7f, 0x80, 0xfe, 0xff]);
            }
            // 임의 byte 삽입.
            2 => {
                let at = rng.below(len + 1);
                let inserted = rng.bytes(4);
                data.splice(at..at, inserted);
            }
            // 구간 삭제.
            3 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(16)).min(len);
                data.drain(start..end);
            }
            // u32 경계값 덮어쓰기(길이·count 프리픽스).
            4 if len >= 4 => {
                let at = rng.below(len - 3);
                let value = if rng.chance(1, 4) {
                    u32::try_from(len - at).unwrap_or(u32::MAX)
                } else {
                    *rng.pick(U32_EDGES)
                };
                data[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            // u64 경계값 덮어쓰기.
            5 if len >= 8 => {
                let at = rng.below(len - 7);
                data[at..at + 8].copy_from_slice(&rng.pick(U64_EDGES).to_le_bytes());
            }
            // token 삽입.
            6 => {
                let at = rng.below(len + 1);
                let token = *rng.pick(TOKENS);
                data.splice(at..at, token.iter().copied());
            }
            // 구간 복제.
            7 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(64)).min(len);
                let chunk = data[start..end].to_vec();
                let at = rng.below(data.len() + 1);
                data.splice(at..at, chunk);
            }
            // 다른 seed와 splice.
            8 => {
                let other = &rng.pick(seeds).bytes;
                let cut = rng.below(len + 1);
                let from = rng.below(other.len() + 1);
                data.truncate(cut);
                data.extend_from_slice(&other[from..]);
            }
            // 구간 반복(count 증폭).
            9 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(40)).min(len);
                let chunk = data[start..end].to_vec();
                let times = 1 + rng.below(200);
                let repeated: Vec<u8> = chunk
                    .iter()
                    .copied()
                    .cycle()
                    .take(chunk.len() * times)
                    .collect();
                data.splice(end..end, repeated);
            }
            // 절단.
            10 => {
                let cut = rng.below(len + 1);
                data.truncate(cut);
            }
            // 인접 구간 교환(정렬 위반).
            11 if len >= 2 => {
                let width = 1 + rng.below((len / 2).min(32));
                let start = rng.below(len - 2 * width + 1);
                let (left, right) = data[start..start + 2 * width].split_at_mut(width);
                left.swap_with_slice(right);
            }
            // 같은 길이 다중 byte UTF-8 덮어쓰기(길이 프리픽스는 그대로).
            12 => {
                let c = *rng.pick(MULTIBYTE);
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf).as_bytes();
                if len >= encoded.len() {
                    let at = rng.below(len - encoded.len() + 1);
                    data[at..at + encoded.len()].copy_from_slice(encoded);
                }
            }
            // ASCII hex 자리를 다른 ASCII로 바꿈(hash 문자열 손상).
            _ if len > 0 => {
                let at = rng.below(len);
                if data[at].is_ascii_hexdigit() {
                    data[at] = *rng.pick(b"0fFgG:-\n ");
                }
            }
            _ => {}
        }
        if data.len() > MAX_MUTATED_LEN {
            data.truncate(MAX_MUTATED_LEN);
        }
    }
    data
}

// --- 생성기 ---

/// chunk 파일 대상의 주소 문맥: 요청 hash·기대 kind와 원래 payload.
#[derive(Debug, Clone)]
struct ChunkContext {
    hash: ContentHash,
    kind: ChunkKindCode,
    payload: Vec<u8>,
    /// payload hash가 주소와 같아 canonical 파일을 만들 수 있으면 그 bytes.
    canonical_file: Option<Vec<u8>>,
}

impl ChunkContext {
    /// payload의 canonical 주소 문맥.
    fn for_payload(kind: ChunkKindCode, payload: &[u8]) -> ChunkContext {
        let file = chunk_file_bytes(kind, payload);
        ChunkContext {
            hash: ContentHash::compute(payload),
            kind,
            payload: payload.to_vec(),
            canonical_file: Some(file),
        }
    }

    /// 입력의 envelope가 디코딩되면 그 주소·kind·나머지 bytes를, 아니면 기본 문맥을 씁니다.
    ///
    /// decoder가 panic하면 기본 문맥을 씁니다(같은 입력의 panic은 대상 실행이 발견으로 보고).
    fn derive(bytes: &[u8]) -> ChunkContext {
        let decoded = quietly(|| {
            let mut dec = CanonicalDecoder::new(bytes);
            ChunkEnvelope::decode(&mut dec)
                .map(|envelope| (envelope, bytes.len() - dec.remaining()))
        });
        match decoded {
            Ok(Ok((envelope, consumed))) => {
                let rest = &bytes[consumed..];
                let canonical_file = (ContentHash::compute(rest) == envelope.payload_hash)
                    .then(|| chunk_file_bytes(envelope.kind, rest));
                ChunkContext {
                    hash: envelope.payload_hash,
                    kind: envelope.kind,
                    payload: rest.to_vec(),
                    canonical_file,
                }
            }
            Ok(Err(_)) | Err(_) => ChunkContext::default_context(),
        }
    }

    /// 기본 문맥: 빈 ViewTree kind payload.
    fn default_context() -> ChunkContext {
        ChunkContext::for_payload(KIND_VIEW_TREE, &[])
    }
}

/// seed 입력과 chunk 주소 문맥.
#[derive(Debug, Clone)]
struct Seed {
    bytes: Vec<u8>,
    context: Arc<ChunkContext>,
}

/// envelope header + payload(완전한 chunk 파일 bytes).
fn chunk_file_bytes(kind: ChunkKindCode, payload: &[u8]) -> Vec<u8> {
    let mut bytes = ChunkEnvelope::for_payload(kind, payload)
        .encode()
        .expect("생성기 envelope 인코딩");
    bytes.extend_from_slice(payload);
    bytes
}

/// 무작위 유한 f32(경계값 포함).
fn gen_f32(rng: &mut Rng) -> f32 {
    let edges = [
        0.0,
        1.0,
        -1.5,
        f32::MIN_POSITIVE,
        1e-45,
        f32::MAX,
        f32::MIN,
        0.1,
    ];
    if rng.chance(1, 2) {
        *rng.pick(&edges)
    } else {
        let value = f32::from_bits(rng.next_u64() as u32);
        if value.is_finite() {
            value
        } else {
            2.5
        }
    }
}

/// 무작위 유한 f64(경계값 포함).
fn gen_f64(rng: &mut Rng) -> f64 {
    let edges = [
        0.0,
        -2.5,
        f64::MIN_POSITIVE,
        5e-324,
        f64::MAX,
        f64::MIN,
        0.1,
    ];
    if rng.chance(1, 2) {
        *rng.pick(&edges)
    } else {
        let value = f64::from_bits(rng.next_u64());
        if value.is_finite() {
            value
        } else {
            -7.25
        }
    }
}

/// 무작위 UTF-8 문자열.
fn gen_string(rng: &mut Rng) -> String {
    const POOL: &[char] = &[
        'a', 'z', '0', ':', '-', '/', '.', ' ', '\n', '\0', 'é', '€', '한', '😀', '\u{feff}',
    ];
    (0..rng.below(12)).map(|_| *rng.pick(POOL)).collect()
}

/// 무작위 canonical op stream(유효 bytes).
fn gen_stream(rng: &mut Rng, depth: usize) -> Vec<u8> {
    let mut enc = CanonicalEncoder::new();
    for _ in 0..1 + rng.below(10) {
        let op = rng.below(usize::from(OP_COUNT)) as u8;
        enc.put_u8(op);
        let v = rng.next_u64();
        match op {
            0 => enc.put_u8(v as u8),
            1 => enc.put_u16(v as u16),
            2 => enc.put_u32(v as u32),
            3 => enc.put_u64(v),
            4 => enc.put_i8(v as i8),
            5 => enc.put_i16(v as i16),
            6 => enc.put_i32(v as i32),
            7 => enc.put_i64(v as i64),
            8 => enc.put_bool(v & 1 == 1),
            9 => enc.put_f32(gen_f32(rng)).expect("유한 f32"),
            10 => enc.put_f64(gen_f64(rng)).expect("유한 f64"),
            11 => enc.put_bytes(&rng.bytes(24)).expect("짧은 bytes"),
            12 => enc.put_string(&gen_string(rng)).expect("짧은 문자열"),
            13 => {
                let mut keys = BTreeSet::new();
                for _ in 0..rng.below(6) {
                    keys.insert(rng.bytes(4));
                }
                let entries: Vec<(Vec<u8>, Vec<u8>)> = keys
                    .into_iter()
                    .map(|k| {
                        let value = if depth < 2 && rng.chance(1, 2) {
                            gen_stream(rng, depth + 1)
                        } else {
                            rng.bytes(8)
                        };
                        (k, value)
                    })
                    .collect();
                enc.put_map(&entries).expect("고유 key map");
            }
            14 => {
                let mut tags = BTreeSet::new();
                for _ in 0..rng.below(6) {
                    tags.insert(if rng.chance(3, 4) {
                        *rng.pick(KNOWN_TAGS)
                    } else {
                        rng.next_u64() as u32
                    });
                }
                let fields: Vec<Field> = tags
                    .into_iter()
                    .map(|tag| Field {
                        tag,
                        // unknown tag는 대부분 optional(skip 경로), 가끔 mandatory(거부 경로).
                        mandatory: KNOWN_TAGS.contains(&tag) || rng.chance(1, 8),
                        payload: if depth < 2 && rng.chance(1, 2) {
                            gen_stream(rng, depth + 1)
                        } else {
                            rng.bytes(8)
                        },
                    })
                    .collect();
                enc.put_fields(&fields).expect("고유 tag record");
            }
            _ => {
                let raw = rng.bytes(20);
                enc.put_u8(raw.len() as u8);
                enc.put_fixed_raw(&raw);
            }
        }
    }
    enc.into_bytes()
}

/// 무작위 kind code(초기 kind, 미등록 optional·required).
fn gen_kind(rng: &mut Rng) -> ChunkKindCode {
    match rng.below(4) {
        0 | 1 => rng.pick(&INITIAL_KINDS).0,
        2 => ChunkKindCode::optional(rng.next_u64() as u32),
        _ => ChunkKindCode::required(rng.next_u64() as u32),
    }
}

/// 무작위 ViewTree payload(유효).
fn gen_view_tree(rng: &mut Rng) -> ViewTreeChunk {
    let mut roots = BTreeSet::new();
    for _ in 0..rng.below(6) {
        roots.insert(rng.id16());
    }
    ViewTreeChunk {
        view_root: StableId::from_bytes(rng.id16()),
        subtrees: roots
            .into_iter()
            .map(|root| SubtreeRef {
                root_node: StableId::from_bytes(root),
                chunk: ChunkId::from_bytes(rng.id16()),
            })
            .collect(),
    }
}

/// 무작위 chunk pointer.
fn gen_pointer(rng: &mut Rng) -> ChunkPointer {
    ChunkPointer {
        chunk_id: ChunkId::from_bytes(rng.id16()),
        content_hash: ContentHash::compute(&rng.bytes(16)),
        kind: gen_kind(rng),
        schema_version: rng.next_u64() as u32 % 8,
    }
}

/// 무작위 generation manifest(유효).
fn gen_manifest(rng: &mut Rng) -> GenerationManifest {
    let generation = 1 + rng.next_u64() % 1_000;
    let mut pointers: BTreeMap<[u8; 16], ChunkPointer> = BTreeMap::new();
    for _ in 0..rng.below(6) {
        let pointer = gen_pointer(rng);
        pointers.insert(pointer.chunk_id.to_bytes(), pointer);
    }
    let mut roots = BTreeSet::new();
    for _ in 0..rng.below(4) {
        roots.insert(rng.id16());
    }
    GenerationManifest {
        generation,
        parent_generation: (generation > 1 && rng.chance(1, 2))
            .then(|| rng.next_u64() % (generation - 1) + 1),
        project_id: StableId::from_bytes(rng.id16()),
        schema_version: rng.next_u64() as u32 % 8,
        chunk_pointers: pointers.into_values().collect(),
        root_document_ids: roots.into_iter().map(StableId::from_bytes).collect(),
    }
}

/// 구조를 아는 생성기로 새 seed를 만듭니다.
fn gen_seed(rng: &mut Rng) -> Seed {
    let default = || Arc::new(ChunkContext::default_context());
    match rng.below(8) {
        0 => Seed {
            bytes: gen_stream(rng, 0),
            context: default(),
        },
        1..=3 => {
            let kind = gen_kind(rng);
            let payload = match rng.below(4) {
                0 => gen_stream(rng, 0),
                1 => gen_view_tree(rng).encode().expect("유효 ViewTree"),
                2 => rng.bytes(64),
                _ => Vec::new(),
            };
            let context = ChunkContext::for_payload(kind, &payload);
            Seed {
                bytes: context.canonical_file.clone().expect("canonical 문맥"),
                context: Arc::new(context),
            }
        }
        4 => Seed {
            bytes: gen_pointer(rng).encode().expect("유효 pointer"),
            context: default(),
        },
        5 => Seed {
            bytes: gen_manifest(rng).encode().expect("유효 manifest"),
            context: default(),
        },
        6 => Seed {
            bytes: gen_view_tree(rng).encode().expect("유효 ViewTree"),
            context: default(),
        },
        _ => {
            let mut text = ContentHash::compute(&rng.bytes(8)).to_string();
            if rng.chance(1, 2) {
                text.push('\n');
            }
            Seed {
                bytes: text.into_bytes(),
                context: default(),
            }
        }
    }
}

// --- seed corpus ---

/// corpus 파일 하나: 대상·기대 결과·bytes.
#[derive(Debug, Clone)]
struct CorpusEntry {
    name: String,
    target: String,
    expect_ok: bool,
    bytes: Vec<u8>,
}

/// `# key: value` 지시문과 hex 본문을 해석합니다. 형식 위반은 harness 오류(panic)입니다.
fn parse_corpus(name: &str, text: &str) -> CorpusEntry {
    let mut target = None;
    let mut expect_ok = None;
    let mut hex = String::new();
    for line in text.lines() {
        if let Some(comment) = line.trim().strip_prefix('#') {
            let comment = comment.trim();
            if let Some(value) = comment.strip_prefix("target:") {
                target = Some(value.trim().to_string());
            } else if let Some(value) = comment.strip_prefix("expect:") {
                expect_ok = Some(match value.trim() {
                    "ok" => true,
                    "err" => false,
                    other => panic!("{name}: expect는 ok/err여야 합니다: {other}"),
                });
            }
            continue;
        }
        hex.extend(line.chars().filter(|c| !c.is_whitespace()));
    }
    assert!(
        hex.len().is_multiple_of(2),
        "{name}: hex 자리 수가 홀수입니다"
    );
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .unwrap_or_else(|_| panic!("{name}: hex가 아닌 문자"))
        })
        .collect();
    CorpusEntry {
        name: name.to_string(),
        target: target.unwrap_or_else(|| panic!("{name}: target 지시문 없음")),
        expect_ok: expect_ok.unwrap_or_else(|| panic!("{name}: expect 지시문 없음")),
        bytes,
    }
}

/// `tests/fuzz_corpus/chunk_canonical/*.hex`(이름 순).
fn corpus_files() -> Vec<CorpusEntry> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_corpus/chunk_canonical");
    let mut entries: Vec<CorpusEntry> = std::fs::read_dir(&dir)
        .expect("seed corpus 디렉터리")
        .map(|entry| entry.expect("corpus 항목").path())
        .filter(|path| path.extension().is_some_and(|e| e == "hex"))
        .map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(&path).expect("corpus 파일 읽기");
            parse_corpus(&name, &text)
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

// --- stress 입력 ---

/// `u32` LE bytes.
fn le32(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// 길이 프리픽스 bytes.
fn prefixed(payload: &[u8]) -> Vec<u8> {
    let mut out = le32(u32::try_from(payload.len()).expect("짧은 payload")).to_vec();
    out.extend_from_slice(payload);
    out
}

/// envelope header(임의 hash 문자열·길이 허용).
fn raw_envelope(kind: ChunkKindCode, len: u64, hash_text: &[u8]) -> Vec<u8> {
    let mut out = CHUNK_MAGIC.to_vec();
    out.extend_from_slice(&le32(CHUNK_FORMAT_VERSION));
    out.extend_from_slice(&le32(kind.0));
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&prefixed(hash_text));
    out
}

/// manifest header(magic~schema version, parent 없음).
fn manifest_header(generation: u64) -> Vec<u8> {
    let mut out = MANIFEST_MAGIC.to_vec();
    out.extend_from_slice(&le32(MANIFEST_FORMAT_VERSION));
    out.extend_from_slice(&generation.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&[0x11; 16]);
    out.extend_from_slice(&le32(1));
    out
}

/// 번호에서 고유 16 byte id(big-endian 번호 → 정렬 순서 = 번호 순서).
fn numbered_id(n: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[8..].copy_from_slice(&n.to_be_bytes());
    out
}

/// 다중 byte 문자를 hex 자리에 둔 71 byte hash 문자열.
fn multibyte_hash_text(prefix_len: usize, c: char) -> Vec<u8> {
    let mut text = b"sha256:".to_vec();
    text.extend(std::iter::repeat_n(b'a', prefix_len));
    let mut buf = [0u8; 4];
    text.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    while text.len() < 71 {
        text.push(b'a');
    }
    text
}

/// 고정 stress 입력(이름, bytes).
fn stress_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let empty_hash = ContentHash::compute(&[]).to_string().into_bytes();

    let mut many_entries = vec![13u8];
    let entry_count = (STRESS_LEN - 5) / 12;
    many_entries.extend_from_slice(&le32(entry_count as u32));
    for i in 0..entry_count as u32 {
        many_entries.extend_from_slice(&prefixed(&i.to_be_bytes()));
        many_entries.extend_from_slice(&le32(0));
    }

    let mut many_fields = vec![14u8];
    let field_count = (STRESS_LEN - 5) / 9;
    many_fields.extend_from_slice(&le32(field_count as u32));
    for tag in 0..field_count as u32 {
        many_fields.extend_from_slice(&le32(tag));
        many_fields.push(u8::from(KNOWN_TAGS.contains(&tag)));
        many_fields.extend_from_slice(&le32(0));
    }

    let mut nested = vec![0u8, 7];
    for _ in 0..12 {
        let mut outer = vec![13u8];
        outer.extend_from_slice(&le32(1));
        outer.extend_from_slice(&prefixed(b"k"));
        outer.extend_from_slice(&prefixed(&nested));
        nested = outer;
    }

    let big_payload: Vec<u8> = (0..STRESS_LEN).map(|i| (i % 251) as u8).collect();
    let big_chunk = chunk_file_bytes(KIND_VIEW_TREE, &big_payload);

    let pointer_count = 10_000u64;
    let mut many_pointers = manifest_header(1);
    many_pointers.extend_from_slice(&le32(pointer_count as u32));
    for i in 0..pointer_count {
        let pointer = ChunkPointer {
            chunk_id: ChunkId::from_bytes(numbered_id(i)),
            content_hash: ContentHash::compute(&i.to_le_bytes()),
            kind: KIND_VIEW_TREE,
            schema_version: 1,
        };
        many_pointers.extend_from_slice(&pointer.encode().expect("pointer"));
    }
    many_pointers.extend_from_slice(&le32(0));

    let root_count = 60_000u64;
    let mut many_roots = manifest_header(1);
    many_roots.extend_from_slice(&le32(0));
    many_roots.extend_from_slice(&le32(root_count as u32));
    for i in 0..root_count {
        many_roots.extend_from_slice(&numbered_id(i));
    }

    let subtree_count = 30_000u64;
    let mut many_subtrees = vec![0x22; 16];
    many_subtrees.extend_from_slice(&le32(subtree_count as u32));
    for i in 0..subtree_count {
        many_subtrees.extend_from_slice(&numbered_id(i));
        many_subtrees.extend_from_slice(&numbered_id(i + subtree_count));
    }

    let mut pointer_count_max = manifest_header(1);
    pointer_count_max.extend_from_slice(&le32(u32::MAX));
    let mut root_count_max = manifest_header(1);
    root_count_max.extend_from_slice(&le32(0));
    root_count_max.extend_from_slice(&le32(u32::MAX));
    let mut subtree_count_max = vec![0x33; 16];
    subtree_count_max.extend_from_slice(&le32(u32::MAX));

    // manifest 의미 검증 경로: generation 0, parent ≥ generation, 중복 root, pointer 충돌·중복.
    let mut zero_generation = manifest_header(0);
    zero_generation.extend_from_slice(&le32(0));
    zero_generation.extend_from_slice(&le32(0));
    let mut invalid_parent = MANIFEST_MAGIC.to_vec();
    invalid_parent.extend_from_slice(&le32(MANIFEST_FORMAT_VERSION));
    invalid_parent.extend_from_slice(&3u64.to_le_bytes());
    invalid_parent.push(1);
    invalid_parent.extend_from_slice(&3u64.to_le_bytes());
    invalid_parent.extend_from_slice(&[0x11; 16]);
    invalid_parent.extend_from_slice(&le32(1));
    invalid_parent.extend_from_slice(&le32(0));
    invalid_parent.extend_from_slice(&le32(0));
    let mut duplicate_roots = manifest_header(1);
    duplicate_roots.extend_from_slice(&le32(0));
    duplicate_roots.extend_from_slice(&le32(2));
    duplicate_roots.extend_from_slice(&numbered_id(5));
    duplicate_roots.extend_from_slice(&numbered_id(5));
    let same_id_pointers = |second_hash: ContentHash, second_kind: ChunkKindCode| {
        let first = ChunkPointer {
            chunk_id: ChunkId::from_bytes(numbered_id(9)),
            content_hash: ContentHash::compute(b"first"),
            kind: KIND_VIEW_TREE,
            schema_version: 1,
        };
        let second = ChunkPointer {
            content_hash: second_hash,
            kind: second_kind,
            ..first
        };
        let mut pair = [first, second];
        pair.sort();
        let mut out = manifest_header(1);
        out.extend_from_slice(&le32(2));
        for pointer in pair {
            out.extend_from_slice(&pointer.encode().expect("pointer"));
        }
        out.extend_from_slice(&le32(0));
        out
    };
    let conflicting_pointers = same_id_pointers(ContentHash::compute(b"second"), KIND_VIEW_TREE);
    let duplicate_pointers =
        same_id_pointers(ContentHash::compute(b"first"), ChunkKindCode::optional(77));

    let mut inputs = vec![
        ("stream-map-count-max-no-entries", {
            let mut v = vec![13u8];
            v.extend_from_slice(&le32(u32::MAX));
            v
        }),
        ("stream-fields-count-max-no-entries", {
            let mut v = vec![14u8];
            v.extend_from_slice(&le32(u32::MAX));
            v
        }),
        ("stream-bytes-len-max", {
            let mut v = vec![11u8];
            v.extend_from_slice(&le32(u32::MAX));
            v.extend_from_slice(b"abc");
            v
        }),
        ("stream-string-len-over-by-one", {
            let mut v = vec![12u8];
            v.extend_from_slice(&le32(4));
            v.extend_from_slice(b"abc");
            v
        }),
        ("stream-many-map-entries", many_entries),
        ("stream-many-fields", many_fields),
        ("stream-many-u8-ops", [0u8, 0x5a].repeat(STRESS_LEN / 2)),
        ("stream-nested-maps", nested),
        (
            "envelope-len-u64-max",
            raw_envelope(KIND_VIEW_TREE, u64::MAX, &empty_hash),
        ),
        ("envelope-hash-len-u32-max", {
            let mut v = raw_envelope(KIND_VIEW_TREE, 0, b"");
            let at = v.len() - 4;
            v[at..].copy_from_slice(&le32(u32::MAX));
            v.extend_from_slice(&empty_hash);
            v
        }),
        (
            "envelope-hash-uppercase",
            raw_envelope(
                KIND_VIEW_TREE,
                0,
                ContentHash::compute(&[])
                    .to_string()
                    .to_uppercase()
                    .as_bytes(),
            ),
        ),
        ("chunk-large-payload", big_chunk),
        ("manifest-pointer-count-max", pointer_count_max),
        ("manifest-root-count-max", root_count_max),
        ("manifest-zero-generation", zero_generation),
        ("manifest-invalid-parent", invalid_parent),
        ("manifest-duplicate-roots", duplicate_roots),
        ("manifest-conflicting-pointers", conflicting_pointers),
        ("manifest-duplicate-pointers", duplicate_pointers),
        ("manifest-many-pointers", many_pointers),
        ("manifest-many-roots", many_roots),
        ("view-tree-count-max", subtree_count_max),
        ("view-tree-many-subtrees", many_subtrees),
        ("zeros", vec![0u8; STRESS_LEN]),
        ("all-ff", vec![0xffu8; STRESS_LEN]),
        ("magic-repeated", CHUNK_MAGIC.repeat(STRESS_LEN / 8)),
        ("current-pointer-valid", {
            let mut v = empty_hash.clone();
            v.push(b'\n');
            v
        }),
    ];
    // 다중 byte 문자를 hex의 각 위치에 둔 hash 문자열: envelope·pointer·text 경로.
    for (name, prefix, c) in [
        ("multibyte-2-even", 0, 'é'),
        ("multibyte-2-odd", 1, 'é'),
        ("multibyte-3-even", 0, '€'),
        ("multibyte-3-odd", 1, '€'),
        ("multibyte-4-even", 0, '😀'),
        ("multibyte-4-odd", 3, '😀'),
        ("multibyte-3-last", 61, '€'),
    ] {
        let text = multibyte_hash_text(prefix, c);
        inputs.push((
            leak_name(format!("envelope-{name}")),
            raw_envelope(KIND_VIEW_TREE, 0, &text),
        ));
        let mut pointer = vec![0x44; 16];
        pointer.extend_from_slice(&prefixed(&text));
        pointer.extend_from_slice(&le32(KIND_VIEW_TREE.0));
        pointer.extend_from_slice(&le32(1));
        inputs.push((leak_name(format!("pointer-{name}")), pointer));
        let mut line = text.clone();
        line.push(b'\n');
        inputs.push((leak_name(format!("text-{name}")), line));
    }
    inputs
}

/// stress 이름을 `'static`으로 만듭니다(고정 개수, 테스트 수명 동안만 사용).
fn leak_name(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}

// --- 대상 ---

/// fuzz 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Stream,
    Envelope,
    Pointer,
    Manifest,
    ViewTree,
    Text,
    ChunkFile,
}

impl Target {
    const ALL: [Target; 7] = [
        Target::Stream,
        Target::Envelope,
        Target::Pointer,
        Target::Manifest,
        Target::ViewTree,
        Target::Text,
        Target::ChunkFile,
    ];

    fn name(self) -> &'static str {
        match self {
            Target::Stream => "stream",
            Target::Envelope => "envelope",
            Target::Pointer => "pointer",
            Target::Manifest => "manifest",
            Target::ViewTree => "view_tree",
            Target::Text => "text",
            Target::ChunkFile => "chunk_file",
        }
    }

    fn from_name(name: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// 대상 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// 입력을 수용했습니다.
    Accepted,
    /// 오류로 거부했습니다(오류 variant 이름).
    Rejected(String),
}

/// 오류 Debug 표기에서 variant 경로(`Decode.UnexpectedEof` 등, 최대 3단계)를 뽑습니다.
fn variant<E: std::fmt::Debug>(error: &E) -> String {
    let debug = format!("{error:?}");
    let mut parts = Vec::new();
    let mut rest = debug.as_str();
    while parts.len() < 3 {
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        if end == 0 {
            break;
        }
        parts.push(&rest[..end]);
        match rest[end..].strip_prefix('(') {
            Some(inner) => rest = inner,
            None => break,
        }
    }
    parts.join(".")
}

/// 거부 결과를 만들고 오류 Display도 실행합니다(panic 검사).
fn rejected<E: std::fmt::Debug + std::fmt::Display>(error: &E) -> Outcome {
    let shown = error.to_string();
    std::hint::black_box(&shown);
    Outcome::Rejected(variant(error))
}

/// 대상 실행 중 수집하는 값.
#[derive(Debug, Default)]
struct Probe {
    /// 제품 decoder 호출 하나의 최대 peak heap(bytes). harness 쪽 검사·재인코딩·중첩 재귀는
    /// 제외합니다.
    peak: usize,
    /// 중첩 op stream 결과.
    nested: BTreeMap<String, usize>,
}

impl Probe {
    /// 제품 호출 `f`를 실행하고 peak heap을 기록합니다.
    fn call<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let (out, peak) = measure(f);
        self.peak = self.peak.max(peak);
        out
    }
}

/// 대상 공통 실행 환경.
struct Env {
    store: ChunkStore,
    registry: ChunkKindRegistry,
}

/// 대상 하나를 실행합니다. `Err`는 canonical 성질 위반(발견)입니다.
fn run_target(
    target: Target,
    input: &[u8],
    env: &Env,
    context: &ChunkContext,
    probe: &mut Probe,
) -> Result<Outcome, String> {
    match target {
        Target::Stream => decode_stream(input, 0, probe),
        Target::Envelope => target_envelope(input, &env.registry, probe),
        Target::Pointer => target_pointer(input, probe),
        Target::Manifest => target_manifest(input, probe),
        Target::ViewTree => target_view_tree(input, probe),
        Target::Text => target_text(input, probe),
        Target::ChunkFile => target_chunk_file(input, &env.store, context, probe),
    }
}

/// op 하나의 결과: 수용(재인코딩 bytes, 비교 불가면 `None`) 또는 거부.
enum Op {
    Accepted(Option<Vec<u8>>),
    Rejected(Outcome),
}

/// canonical op stream을 끝까지 디코딩합니다.
///
/// 입력 형식: (op byte, op별 값)의 반복. op = byte % 16:
/// 0 `u8` · 1 `u16` · 2 `u32` · 3 `u64` · 4 `i8` · 5 `i16` · 6 `i32` · 7 `i64` · 8 `bool` ·
/// 9 `f32` · 10 `f64` · 11 bytes · 12 string · 13 map · 14 fields · 15 raw(`u8` 길이 + 고정 폭).
/// 첫 디코딩 오류에서 거부로 끝납니다. 수용한 op마다 다시 인코딩한 bytes가 소비한 bytes와
/// 같아야 합니다(canonical 왕복).
fn decode_stream(input: &[u8], depth: usize, probe: &mut Probe) -> Result<Outcome, String> {
    let mut dec = CanonicalDecoder::new(input);
    while dec.remaining() > 0 {
        let op = probe
            .call(|| dec.take_u8())
            .map_err(|e| format!("남은 입력이 있는데 op byte를 읽지 못함: {e}"))?
            % OP_COUNT;
        let before = input.len() - dec.remaining();
        let result = decode_op(&mut dec, input, before, op, depth, probe)?;
        let after = input.len() - dec.remaining();
        if after < before {
            return Err(format!("decoder 위치가 뒤로 이동함: {before} → {after}"));
        }
        match result {
            Op::Accepted(Some(reencoded)) => {
                if reencoded != input[before..after] {
                    return Err(format!(
                        "op {op} 재인코딩이 소비 bytes와 다름(offset {before}, {} vs {} bytes)",
                        reencoded.len(),
                        after - before
                    ));
                }
            }
            Op::Accepted(None) => {}
            Op::Rejected(outcome) => return Ok(outcome),
        }
    }
    dec.expect_end()
        .map_err(|e| format!("입력을 다 읽었는데 expect_end 실패: {e}"))?;
    Ok(Outcome::Accepted)
}

/// 중첩 bytes를 op stream으로 다시 디코딩하고 결과를 집계합니다.
fn decode_nested(bytes: &[u8], depth: usize, probe: &mut Probe) -> Result<(), String> {
    if depth >= STREAM_MAX_DEPTH {
        return Ok(());
    }
    let outcome = decode_stream(bytes, depth + 1, probe)?;
    let key = match outcome {
        Outcome::Accepted => "ok".to_string(),
        Outcome::Rejected(v) => v,
    };
    *probe.nested.entry(key).or_insert(0) += 1;
    Ok(())
}

/// op 하나를 디코딩하고 수용 시 같은 값을 다시 인코딩합니다.
fn decode_op(
    dec: &mut CanonicalDecoder<'_>,
    input: &[u8],
    before: usize,
    op: u8,
    depth: usize,
    probe: &mut Probe,
) -> Result<Op, String> {
    let mut enc = CanonicalEncoder::new();
    macro_rules! fixed {
        ($take:ident, $put:ident) => {
            match probe.call(|| dec.$take()) {
                Ok(value) => enc.$put(value),
                Err(e) => return Ok(Op::Rejected(rejected(&e))),
            }
        };
    }
    match op {
        0 => fixed!(take_u8, put_u8),
        1 => fixed!(take_u16, put_u16),
        2 => fixed!(take_u32, put_u32),
        3 => fixed!(take_u64, put_u64),
        4 => fixed!(take_i8, put_i8),
        5 => fixed!(take_i16, put_i16),
        6 => fixed!(take_i32, put_i32),
        7 => fixed!(take_i64, put_i64),
        8 => fixed!(take_bool, put_bool),
        9 => match probe.call(|| dec.take_f32()) {
            Ok(value) => {
                if !value.is_finite() || value.to_bits() == (-0.0_f32).to_bits() {
                    return Err(format!("비canonical f32 수용: {:#010x}", value.to_bits()));
                }
                enc.put_f32(value)
                    .map_err(|e| format!("수용한 f32 재인코딩 실패: {e}"))?;
            }
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        10 => match probe.call(|| dec.take_f64()) {
            Ok(value) => {
                if !value.is_finite() || value.to_bits() == (-0.0_f64).to_bits() {
                    return Err(format!("비canonical f64 수용: {:#018x}", value.to_bits()));
                }
                enc.put_f64(value)
                    .map_err(|e| format!("수용한 f64 재인코딩 실패: {e}"))?;
            }
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        11 => match probe.call(|| dec.take_bytes()) {
            Ok(bytes) => enc
                .put_bytes(bytes)
                .map_err(|e| format!("수용한 bytes 재인코딩 실패: {e}"))?,
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        12 => match probe.call(|| dec.take_string()) {
            Ok(text) => enc
                .put_string(text)
                .map_err(|e| format!("수용한 문자열 재인코딩 실패: {e}"))?,
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        13 => match probe.call(|| dec.take_map()) {
            Ok(entries) => {
                if entries.windows(2).any(|w| w[0].0 >= w[1].0) {
                    return Err("정렬·고유하지 않은 map key 수용".to_string());
                }
                for (_, value) in &entries {
                    decode_nested(value, depth, probe)?;
                }
                enc.put_map(&entries)
                    .map_err(|e| format!("수용한 map 재인코딩 실패: {e}"))?;
            }
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        14 => match probe.call(|| dec.take_fields(KNOWN_TAGS)) {
            Ok(fields) => {
                if fields.windows(2).any(|w| w[0].tag >= w[1].tag) {
                    return Err("정렬·고유하지 않은 field tag 수용".to_string());
                }
                if let Some(f) = fields.iter().find(|f| !KNOWN_TAGS.contains(&f.tag)) {
                    return Err(format!("모르는 tag {}를 결과에 포함", f.tag));
                }
                for field in &fields {
                    decode_nested(&field.payload, depth, probe)?;
                }
                enc.put_fields(&fields)
                    .map_err(|e| format!("수용한 record 재인코딩 실패: {e}"))?;
                // unknown optional field를 skip했으면 bytes 비교가 성립하지 않습니다.
                let declared = input
                    .get(before..before + 4)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map(u32::from_le_bytes)
                    .ok_or("수용한 record에 count가 없음")?;
                if declared as usize != fields.len() {
                    return Ok(Op::Accepted(None));
                }
            }
            Err(e) => return Ok(Op::Rejected(rejected(&e))),
        },
        _ => {
            let n = match probe.call(|| dec.take_u8()) {
                Ok(n) => n,
                Err(e) => return Ok(Op::Rejected(rejected(&e))),
            };
            match probe.call(|| dec.take_raw(usize::from(n))) {
                Ok(raw) => {
                    if raw.len() != usize::from(n) {
                        return Err(format!("take_raw({n})가 {} bytes를 반환", raw.len()));
                    }
                    enc.put_u8(n);
                    enc.put_fixed_raw(raw);
                }
                Err(e) => return Ok(Op::Rejected(rejected(&e))),
            }
        }
    }
    Ok(Op::Accepted(Some(enc.into_bytes())))
}

/// object 경로가 hash에서 고정 형식으로 만들어졌는지 확인합니다(path traversal 불가).
fn check_object_path(hash: &ContentHash) -> Result<(), String> {
    let path = ChunkObjectPath::for_hash(hash);
    let hex = hash.digest_hex();
    let safe = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase() || c == '-' || c == '.')
    };
    let expected_name = format!("sha256-{hex}.starchunk");
    let components = path.to_relative_path().components().count();
    if path.prefix() != &hex[..2]
        || path.file_name() != expected_name
        || !safe(path.prefix())
        || !safe(path.file_name())
        || components != 4
    {
        return Err(format!("비정상 object 경로: {path}"));
    }
    Ok(())
}

/// envelope decoder.
fn target_envelope(
    input: &[u8],
    registry: &ChunkKindRegistry,
    probe: &mut Probe,
) -> Result<Outcome, String> {
    let mut dec = CanonicalDecoder::new(input);
    match probe.call(|| ChunkEnvelope::decode(&mut dec)) {
        Ok(envelope) => {
            let consumed = input.len() - dec.remaining();
            let reencoded = envelope
                .encode()
                .map_err(|e| format!("수용한 envelope 재인코딩 실패: {e}"))?;
            if reencoded != input[..consumed] {
                return Err("envelope 재인코딩이 소비 bytes와 다름".to_string());
            }
            if let Err(e) = probe.call(|| registry.resolve(envelope.kind)) {
                std::hint::black_box(e.to_string());
            }
            check_object_path(&envelope.payload_hash)?;
            Ok(Outcome::Accepted)
        }
        Err(e) => Ok(rejected(&e)),
    }
}

/// chunk pointer decoder.
fn target_pointer(input: &[u8], probe: &mut Probe) -> Result<Outcome, String> {
    let mut dec = CanonicalDecoder::new(input);
    match probe.call(|| ChunkPointer::decode(&mut dec)) {
        Ok(pointer) => {
            let consumed = input.len() - dec.remaining();
            let reencoded = pointer
                .encode()
                .map_err(|e| format!("수용한 pointer 재인코딩 실패: {e}"))?;
            if reencoded != input[..consumed] {
                return Err("pointer 재인코딩이 소비 bytes와 다름".to_string());
            }
            check_object_path(&pointer.content_hash)?;
            Ok(Outcome::Accepted)
        }
        Err(e) => Ok(rejected(&e)),
    }
}

/// generation manifest decoder.
fn target_manifest(input: &[u8], probe: &mut Probe) -> Result<Outcome, String> {
    match probe.call(|| GenerationManifest::decode(input)) {
        Ok(manifest) => {
            manifest
                .validate()
                .map_err(|e| format!("수용한 manifest가 검증 실패: {e}"))?;
            let (bytes, hash) = manifest
                .encode_with_hash()
                .map_err(|e| format!("수용한 manifest 재인코딩 실패: {e}"))?;
            if bytes != input {
                return Err("manifest 재인코딩이 입력과 다름".to_string());
            }
            if hash != ContentHash::compute(input) {
                return Err("manifest hash가 입력 bytes hash와 다름".to_string());
            }
            Ok(Outcome::Accepted)
        }
        Err(e) => Ok(rejected(&e)),
    }
}

/// ViewTree payload decoder.
fn target_view_tree(input: &[u8], probe: &mut Probe) -> Result<Outcome, String> {
    match probe.call(|| ViewTreeChunk::decode(input)) {
        Ok(tree) => {
            tree.validate()
                .map_err(|e| format!("수용한 ViewTree가 검증 실패: {e}"))?;
            let bytes = tree
                .encode()
                .map_err(|e| format!("수용한 ViewTree 재인코딩 실패: {e}"))?;
            if bytes != input {
                return Err("ViewTree 재인코딩이 입력과 다름".to_string());
            }
            Ok(Outcome::Accepted)
        }
        Err(e) => Ok(rejected(&e)),
    }
}

/// hash·ChunkId 문자열 parse(전체 입력과 마지막 LF를 뗀 current pointer 형식).
fn target_text(input: &[u8], probe: &mut Probe) -> Result<Outcome, String> {
    let text = String::from_utf8_lossy(input);
    let candidates = [
        text.as_ref(),
        text.strip_suffix('\n').unwrap_or(text.as_ref()),
    ];
    let mut outcome = None;
    for candidate in candidates {
        match probe.call(|| candidate.parse::<ContentHash>()) {
            Ok(hash) => {
                if hash.to_string() != candidate {
                    return Err(format!("hash 문자열이 표기 그대로 재기록되지 않음: {hash}"));
                }
                check_object_path(&hash)?;
                outcome = Some(Outcome::Accepted);
            }
            Err(e) => {
                if outcome.is_none() {
                    outcome = Some(rejected(&e));
                }
            }
        }
        if let Ok(id) = probe.call(|| candidate.parse::<ChunkId>()) {
            if id.to_string().parse::<ChunkId>().ok() != Some(id) {
                return Err(format!("ChunkId 문자열 왕복 실패: {id}"));
            }
        }
    }
    Ok(outcome.unwrap_or_else(|| Outcome::Rejected("none".to_string())))
}

/// 디스크 chunk 읽기·기존 파일 검증 쓰기.
fn target_chunk_file(
    input: &[u8],
    store: &ChunkStore,
    context: &ChunkContext,
    probe: &mut Probe,
) -> Result<Outcome, String> {
    let path = store
        .root()
        .join(ChunkObjectPath::for_hash(&context.hash).to_relative_path());
    let parent = path.parent().ok_or("chunk 경로에 부모가 없음")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("harness I/O 실패: {e}"))?;
    std::fs::write(&path, input).map_err(|e| format!("harness I/O 실패: {e}"))?;

    let outcome = match probe.call(|| store.read_chunk(&context.hash, context.kind)) {
        Ok(payload) => {
            if ContentHash::compute(&payload) != context.hash {
                return Err("요청 주소와 hash가 다른 payload를 반환".to_string());
            }
            if chunk_file_bytes(context.kind, &payload) != input {
                return Err("canonical chunk 파일이 아닌 bytes를 수용".to_string());
            }
            Outcome::Accepted
        }
        Err(e) => rejected(&e),
    };
    let unchanged = |stage: &str| -> Result<(), String> {
        let now = std::fs::read(&path).map_err(|e| format!("harness I/O 실패: {e}"))?;
        if now != input {
            return Err(format!("{stage}가 chunk 파일을 수정함"));
        }
        Ok(())
    };
    unchanged("read_chunk")?;

    // 같은 주소에 원래 payload를 다시 쓰면: 파일이 canonical이면 Reused, 아니면 CorruptExisting.
    if let Some(canonical) = &context.canonical_file {
        match probe.call(|| store.write_chunk(context.kind, &context.payload)) {
            Ok(WriteOutcome::Reused { .. }) if input == canonical.as_slice() => {}
            Err(ChunkStoreError::CorruptExisting { .. }) if input != canonical.as_slice() => {}
            other => return Err(format!("기존 파일 검증 쓰기 결과가 다름: {other:?}")),
        }
        unchanged("write_chunk")?;
    }
    std::fs::remove_file(&path).map_err(|e| format!("harness I/O 실패: {e}"))?;
    Ok(outcome)
}

// --- 실행과 판정 ---

/// 발견 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FindingKind {
    Panic,
    Hang,
    ExcessiveAllocation,
    Canonical,
}

/// 발견 한 건.
#[derive(Debug)]
struct Finding {
    kind: FindingKind,
    target: &'static str,
    origin: String,
    detail: String,
    input: Vec<u8>,
    context_hash: ContentHash,
    context_kind: ChunkKindCode,
}

/// 캠페인 결과.
#[derive(Debug, Default)]
struct Report {
    executed: usize,
    /// (대상, 결과) → 횟수.
    outcomes: BTreeMap<(&'static str, String), usize>,
    /// 중첩 op stream 결과.
    nested: BTreeMap<String, usize>,
    /// 1 KiB 이상 입력의 최대 peak/입력 비율.
    max_alloc_ratio: f64,
    max_alloc_origin: String,
    /// 고정 여유보다 큰 입력의 최대 peak/입력 비율(입력에 비례하는 몫이 지배).
    max_large_alloc_ratio: f64,
    max_large_alloc_origin: String,
    max_peak: usize,
    max_peak_origin: String,
    max_elapsed: Duration,
    max_elapsed_origin: String,
    findings: Vec<Finding>,
}

impl Report {
    /// 대상별 수용 수.
    fn accepted(&self, target: Target) -> usize {
        self.outcomes
            .get(&(target.name(), "ok".to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// 대상별 거부 수.
    fn rejected(&self, target: Target) -> usize {
        self.outcomes
            .iter()
            .filter(|((t, o), _)| *t == target.name() && o != "ok")
            .map(|(_, n)| n)
            .sum()
    }
}

/// 입력 하나를 모든 대상에 실행하고 결과를 report에 반영합니다.
fn check(report: &mut Report, env: &Env, origin: &str, input: &[u8], context: &ChunkContext) {
    let started = Instant::now();
    report.executed += 1;
    let mut input_peak = 0usize;
    let mut peak_target = "";
    for target in Target::ALL {
        let mut probe = Probe::default();
        let result = quietly(|| run_target(target, input, env, context, &mut probe));
        let peak = probe.peak;
        if peak > input_peak {
            input_peak = peak;
            peak_target = target.name();
        }
        for (key, n) in probe.nested {
            *report.nested.entry(key).or_insert(0) += n;
        }
        let mut found = |kind: FindingKind, detail: String| {
            report.findings.push(Finding {
                kind,
                target: target.name(),
                origin: origin.to_string(),
                detail,
                input: input.to_vec(),
                context_hash: context.hash,
                context_kind: context.kind,
            });
        };
        match result {
            Err(message) => found(FindingKind::Panic, message),
            Ok(Err(detail)) => found(FindingKind::Canonical, detail),
            Ok(Ok(outcome)) => {
                let key = match outcome {
                    Outcome::Accepted => "ok".to_string(),
                    Outcome::Rejected(v) => v,
                };
                *report.outcomes.entry((target.name(), key)).or_insert(0) += 1;
            }
        }
        let limit = alloc_limit(input.len());
        if peak > limit {
            found(
                FindingKind::ExcessiveAllocation,
                format!(
                    "peak {peak} bytes > limit {limit} (input {} bytes)",
                    input.len()
                ),
            );
        }
    }
    let elapsed = started.elapsed();
    if elapsed > PER_INPUT_LIMIT {
        report.findings.push(Finding {
            kind: FindingKind::Hang,
            target: "all",
            origin: origin.to_string(),
            detail: format!("{elapsed:?} > {PER_INPUT_LIMIT:?}"),
            input: input.to_vec(),
            context_hash: context.hash,
            context_kind: context.kind,
        });
    }
    let ratio = input_peak as f64 / (input.len().max(1)) as f64;
    if input.len() >= 1024 && ratio > report.max_alloc_ratio {
        report.max_alloc_ratio = ratio;
        report.max_alloc_origin = format!("{origin} [{peak_target}, {} bytes]", input.len());
    }
    if input.len() > ALLOC_SLACK && ratio > report.max_large_alloc_ratio {
        report.max_large_alloc_ratio = ratio;
        report.max_large_alloc_origin = format!("{origin} [{peak_target}, {} bytes]", input.len());
    }
    if input_peak > report.max_peak {
        report.max_peak = input_peak;
        report.max_peak_origin = format!("{origin} [{peak_target}]");
    }
    if elapsed > report.max_elapsed {
        report.max_elapsed = elapsed;
        report.max_elapsed_origin = origin.to_string();
    }
}

/// 진행 메시지.
enum Progress {
    Tick,
    Done(Box<Report>),
}

/// 테스트용 임시 디렉터리.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-storage-fuzz-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 변형용 seed 목록: corpus + 고정 생성 seed(별도 난수열).
fn mutation_seeds() -> Vec<Seed> {
    let mut seeds: Vec<Seed> = corpus_files()
        .into_iter()
        .map(|entry| Seed {
            context: Arc::new(ChunkContext::derive(&entry.bytes)),
            bytes: entry.bytes,
        })
        .collect();
    let mut rng = Rng(SEED ^ 0x5eed);
    for _ in 0..64 {
        seeds.push(gen_seed(&mut rng));
    }
    seeds
}

/// 결정론적 변형 입력열의 다음 항목.
fn next_mutation(rng: &mut Rng, seeds: &[Seed]) -> Seed {
    let base = if rng.chance(1, 4) {
        gen_seed(rng)
    } else {
        rng.pick(seeds).clone()
    };
    Seed {
        bytes: mutate(rng, base.bytes, seeds),
        context: base.context,
    }
}

/// 캠페인을 실행합니다(stress → corpus → 생성 seed → 변형).
fn run_campaign(
    root: &Path,
    iterations: usize,
    current: &Mutex<(String, Vec<u8>)>,
    tx: &mpsc::Sender<Progress>,
) -> Report {
    let env = Env {
        store: ChunkStore::new(root),
        registry: initial_kind_registry().expect("초기 kind registry"),
    };
    let mut report = Report::default();
    let run = |report: &mut Report, origin: String, bytes: &[u8], context: &ChunkContext| {
        if let Ok(mut slot) = current.lock() {
            *slot = (origin.clone(), bytes.to_vec());
        }
        check(report, &env, &origin, bytes, context);
        let _ = tx.send(Progress::Tick);
    };

    for (name, bytes) in stress_inputs() {
        let context = ChunkContext::derive(&bytes);
        run(&mut report, format!("stress:{name}"), &bytes, &context);
    }
    let seeds = mutation_seeds();
    for (i, seed) in seeds.iter().enumerate() {
        run(&mut report, format!("seed:{i}"), &seed.bytes, &seed.context);
    }
    let mut rng = Rng(SEED);
    for i in 0..iterations {
        let input = next_mutation(&mut rng, &seeds);
        run(
            &mut report,
            format!("mutation:{i}"),
            &input.bytes,
            &input.context,
        );
    }
    report
}

/// 발견 입력과 설명을 `CARGO_TARGET_TMPDIR`에 저장하고 경로를 반환합니다.
fn save_artifact(index: usize, finding: &Finding) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("chunk-canonical-fuzz");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{:?}-{}-{index}.bin", finding.kind, finding.target));
    let _ = std::fs::write(&path, &finding.input);
    let _ = std::fs::write(
        path.with_extension("txt"),
        format!(
            "origin: {}\ntarget: {}\ndetail: {}\nchunk context: {} kind {:#010x}\n",
            finding.origin,
            finding.target,
            finding.detail,
            finding.context_hash,
            finding.context_kind.0
        ),
    );
    path
}

#[test]
fn chunk_canonical_fuzz_campaign_has_no_findings() {
    install_quiet_panic_hook();
    let iterations = std::env::var("STELLAR_STORAGE_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS);
    let dir = TempDir::new("campaign");
    let root = dir.path.clone();
    let current = Arc::new(Mutex::new((String::new(), Vec::new())));
    let (tx, rx) = mpsc::channel();
    let worker_current = Arc::clone(&current);
    // 기본 크기(2 MiB) stack의 별도 thread에서 실행: 앱 worker thread와 같은 조건.
    std::thread::spawn(move || {
        let report = run_campaign(&root, iterations, &worker_current, &tx);
        let _ = tx.send(Progress::Done(Box::new(report)));
    });

    let started = Instant::now();
    let report = loop {
        match rx.recv_timeout(WATCHDOG) {
            Ok(Progress::Tick) => {}
            Ok(Progress::Done(report)) => break report,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (origin, input) = current.lock().map(|g| g.clone()).unwrap_or_default();
                let finding = Finding {
                    kind: FindingKind::Hang,
                    target: "all",
                    origin: origin.clone(),
                    detail: format!("watchdog {WATCHDOG:?}"),
                    input,
                    context_hash: ContentHash::compute(&[]),
                    context_kind: KIND_VIEW_TREE,
                };
                let path = save_artifact(0, &finding);
                panic!("hang: {origin} 입력이 {WATCHDOG:?} 동안 끝나지 않음 (저장: {path:?})");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let (origin, _) = current.lock().map(|g| g.clone()).unwrap_or_default();
                panic!("fuzz worker가 비정상 종료됨(마지막 입력: {origin})");
            }
        }
    };
    let total = started.elapsed();

    println!(
        "chunk/canonical fuzz: executed={} total={total:?}",
        report.executed
    );
    for target in Target::ALL {
        let rejected: Vec<String> = report
            .outcomes
            .iter()
            .filter(|((t, o), _)| *t == target.name() && o != "ok")
            .map(|((_, o), n)| format!("{o}={n}"))
            .collect();
        println!(
            "  {}: accepted={} rejected={} [{}]",
            target.name(),
            report.accepted(target),
            report.rejected(target),
            rejected.join(", ")
        );
    }
    println!("  nested stream: {:?}", report.nested);
    println!(
        "  max alloc ratio(>=1KiB)={:.1} ({}), max alloc ratio(>{ALLOC_SLACK})={:.1} ({})",
        report.max_alloc_ratio,
        report.max_alloc_origin,
        report.max_large_alloc_ratio,
        report.max_large_alloc_origin
    );
    println!(
        "  max peak={} bytes ({}), max elapsed={:?} ({})",
        report.max_peak, report.max_peak_origin, report.max_elapsed, report.max_elapsed_origin
    );

    let mut lines = Vec::new();
    let mut by_kind: BTreeMap<(FindingKind, &str), usize> = BTreeMap::new();
    let mut by_origin: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (i, finding) in report.findings.iter().enumerate() {
        *by_kind.entry((finding.kind, finding.target)).or_insert(0) += 1;
        let class = finding.origin.split(':').next().unwrap_or("");
        by_origin
            .entry(class)
            .or_default()
            .insert(finding.origin.as_str());
        if lines.len() < 40 {
            let path = save_artifact(i, finding);
            lines.push(format!(
                "{:?} [{}] {} — {} (저장: {path:?})",
                finding.kind, finding.target, finding.origin, finding.detail
            ));
        }
    }
    assert!(
        report.findings.is_empty(),
        "fuzz 발견 {}건 {:?}, 발견 입력 수 {:?}:\n{}",
        report.findings.len(),
        by_kind,
        by_origin
            .iter()
            .map(|(class, origins)| (*class, origins.len()))
            .collect::<Vec<_>>(),
        lines.join("\n")
    );
    assert!(report.executed > iterations);
    // 모든 대상이 수용·거부 경로를 모두 실행했는지(입력 분포가 대상에 닿는지) 확인합니다.
    for target in Target::ALL {
        assert!(
            report.accepted(target) > 0 && report.rejected(target) > 0,
            "{}: 수용 {} / 거부 {} — 입력 분포가 대상에 닿지 않습니다",
            target.name(),
            report.accepted(target),
            report.rejected(target)
        );
    }
}

#[test]
fn campaign_inputs_are_deterministic() {
    install_quiet_panic_hook();
    let seeds = mutation_seeds();
    let sample = |seed: u64| {
        let mut rng = Rng(seed);
        (0..300)
            .map(|_| next_mutation(&mut rng, &seeds).bytes)
            .collect::<Vec<_>>()
    };
    assert_eq!(sample(SEED), sample(SEED));
    assert_ne!(sample(SEED), sample(SEED ^ 1));
}

#[test]
fn allocation_meter_counts_heap_bytes() {
    let (_, peak) = measure(|| {
        let v: Vec<u8> = Vec::with_capacity(1 << 20);
        std::hint::black_box(&v);
    });
    assert!(peak >= 1 << 20, "peak {peak}");
    let (_, idle) = measure(|| 1 + 1);
    assert_eq!(idle, 0);
}

#[test]
fn corpus_seeds_have_expected_outcomes() {
    install_quiet_panic_hook();
    let dir = TempDir::new("corpus");
    let env = Env {
        store: ChunkStore::new(&dir.path),
        registry: initial_kind_registry().expect("초기 kind registry"),
    };
    let entries = corpus_files();
    assert!(entries.len() >= 10, "corpus {}개", entries.len());
    for entry in entries {
        let target = Target::from_name(&entry.target)
            .unwrap_or_else(|| panic!("{}: 알 수 없는 target {}", entry.name, entry.target));
        let context = ChunkContext::derive(&entry.bytes);
        let mut probe = Probe::default();
        let result = quietly(|| run_target(target, &entry.bytes, &env, &context, &mut probe));
        let outcome = match result {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(detail)) => panic!("{}: canonical 위반 {detail}", entry.name),
            Err(message) => panic!("{}: panic {message}", entry.name),
        };
        assert_eq!(
            outcome == Outcome::Accepted,
            entry.expect_ok,
            "{}: 기대 {} 실제 {outcome:?}",
            entry.name,
            if entry.expect_ok { "ok" } else { "err" }
        );
    }
}

/// XVI.6.2 발견 회귀: hex 자리에 다중 byte 문자가 있는 71 byte hash 문자열은 모든 진입 경로에서
/// panic 없이 오류가 되어야 합니다.
#[test]
fn multibyte_hash_text_is_rejected_on_every_path() {
    let dir = TempDir::new("multibyte");
    for c in ['é', '€', '😀'] {
        for prefix in 0..=(64 - c.len_utf8()) {
            let text = multibyte_hash_text(prefix, c);
            assert_eq!(text.len(), 71);
            let line = String::from_utf8(text.clone()).expect("UTF-8");

            // ContentHash parse.
            assert_eq!(
                line.parse::<ContentHash>(),
                Err(ParseContentHashError::InvalidHexDigit { found: c }),
                "prefix {prefix} {c:?}"
            );

            // envelope decoder.
            let envelope = raw_envelope(KIND_VIEW_TREE, 0, &text);
            let mut dec = CanonicalDecoder::new(&envelope);
            assert!(ChunkEnvelope::decode(&mut dec).is_err());

            // pointer decoder.
            let mut pointer = vec![0x44; 16];
            pointer.extend_from_slice(&prefixed(&text));
            pointer.extend_from_slice(&le32(KIND_VIEW_TREE.0));
            pointer.extend_from_slice(&le32(1));
            let mut dec = CanonicalDecoder::new(&pointer);
            assert!(ChunkPointer::decode(&mut dec).is_err());
        }
    }

    // current pointer 파일 내용.
    let current = current_pointer_path(&dir.path);
    std::fs::create_dir_all(current.parent().expect("부모")).expect("디렉터리");
    let mut content = multibyte_hash_text(1, '€');
    content.push(b'\n');
    std::fs::write(&current, &content).expect("current 기록");
    assert!(matches!(
        read_current_pointer(&dir.path),
        Err(CurrentPointerError::InvalidFormat { .. })
    ));

    // fallback scanner의 manifest 파일 이름(`sha256-<hex>.starmanifest`).
    let generations = dir.path.join("design").join("generations");
    std::fs::create_dir_all(&generations).expect("디렉터리");
    let mut name = String::from("sha256-a€");
    name.push_str(&"a".repeat(60));
    name.push_str(".starmanifest");
    std::fs::write(generations.join(&name), b"junk").expect("manifest 기록");
    let project = StableId::from_bytes([0x11; 16]);
    assert!(matches!(
        scan_latest_valid_generation(&dir.path, project),
        Ok(None)
    ));
}
