//! 수신 메시지 하나의 메모리 상한 회귀 테스트 (IPC-05).
//!
//! JSON reader의 입력 상한은 1 MiB이지만, `[0,0,…]`처럼 작은 값이 많은 메시지는 값마다 node와 숫자
//! 원문 할당이 생겨 메모리가 입력의 수십 배로 늘고, `params` 복제가 그 비용을 한 번 더 냅니다.
//! reader는 값 개수를 [`JsonLimits::max_nodes`]로 제한하고 분류는 `params`를 복제하지 않아야 합니다.
//!
//! 테스트 바이너리 전용 계측 할당자(System 위임)로 frame 처리 호출 하나의 peak heap을 잽니다. 계측은
//! 현재 thread만 셉니다.
//!
//! [`JsonLimits::max_nodes`]: stellar_mcp::json::JsonLimits

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use stellar_mcp::json::{parse, parse_bytes, write, JsonLimits, JsonValue};
use stellar_mcp::log::Logger;
use stellar_mcp::server::{Server, MAX_MESSAGE_BYTES, MAX_MESSAGE_DEPTH, MAX_MESSAGE_NODES};

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

const MIB: usize = 1 << 20;

/// `params`가 `count`개의 `0`인 ping request.
fn zeros_request(count: usize) -> Vec<u8> {
    let mut text = String::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":["#);
    for i in 0..count {
        if i > 0 {
            text.push(',');
        }
        text.push('0');
    }
    text.push_str("]}");
    text.into_bytes()
}

/// 상한 크기 안에 들어가는 가장 큰 zeros request.
fn largest_zeros_request() -> Vec<u8> {
    let overhead = zeros_request(1).len() - 1;
    let count = (MAX_MESSAGE_BYTES - overhead) / 2;
    let bytes = zeros_request(count);
    assert!(bytes.len() <= MAX_MESSAGE_BYTES);
    bytes
}

fn handle(server: &mut Server, bytes: &[u8]) -> (JsonValue, usize) {
    let mut log = Logger::new(Vec::new());
    let (reply, peak) = measure(|| server.handle_frame(bytes, &mut log));
    let reply = write(&reply.expect("request에는 응답"));
    (parse(&reply, JsonLimits::default()).unwrap(), peak)
}

#[test]
fn small_value_flood_within_size_limit_is_memory_bounded() {
    // 수정 전: 52만 개 `0` 배열 한 메시지(약 1 MiB)에 peak heap 약 50 MiB(값 node·숫자 원문 할당에
    // params 복제까지)를 썼습니다.
    let bytes = largest_zeros_request();
    let mut server = Server::new();
    let (reply, peak) = handle(&mut server, &bytes);
    assert!(
        peak <= 8 * MIB,
        "1 MiB 메시지 하나의 peak heap {peak} bytes(상한 8 MiB)"
    );
    // 값 개수 상한을 넘으므로 JSON 파싱 오류로 거부합니다.
    let error = reply.get("error").expect("오류 응답");
    assert_eq!(error.get("code").and_then(JsonValue::as_i64), Some(-32700));
    assert!(
        error
            .get("message")
            .and_then(JsonValue::as_str)
            .is_some_and(|m| m.contains("too-many-nodes")),
        "{reply:?}"
    );
}

#[test]
fn accepted_message_is_not_duplicated_during_classification() {
    // 값 개수 상한 안의 가장 큰 배열: 분류가 params를 복제하면 parse 결과만큼 peak가 늘어납니다.
    // 최상위 object·jsonrpc·id·method·params 배열이 값 5개입니다.
    let count = MAX_MESSAGE_NODES - 5;
    let bytes = zeros_request(count);
    let mut server = Server::new();
    // 비교 기준: 같은 입력을 파싱만 할 때의 peak.
    let limits = JsonLimits {
        max_bytes: MAX_MESSAGE_BYTES,
        max_depth: MAX_MESSAGE_DEPTH,
        max_nodes: MAX_MESSAGE_NODES,
    };
    let (parsed, parse_peak) = measure(|| parse_bytes(&bytes, limits));
    drop(parsed.expect("상한 안"));
    let (reply, peak) = handle(&mut server, &bytes);
    assert!(reply.get("result").is_some(), "ping 성공: {reply:?}");
    assert!(
        peak < parse_peak + parse_peak / 4,
        "frame 처리 peak {peak} bytes는 파싱 peak {parse_peak} bytes의 1.25배 미만"
    );
}
