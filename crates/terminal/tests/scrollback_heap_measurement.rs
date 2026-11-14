//! XVIII.4.2 완료 조건(100만 줄 memory cap 유지)의 독립 측정.
//!
//! crate 단위 테스트는 crate가 스스로 세는 [`Scrollback::memory_bytes`]로 상한을 확인합니다. 이
//! 테스트는 그 계산과 따로, 테스트 바이너리 전용 계측 할당자(System 위임)로 emulator에 출력 100만
//! 줄을 넣는 동안의 heap을 잽니다.
//!
//! - **요청 크기**(`Layout::size`): 늘어난 heap의 최대값이 scrollback 메모리 상한에 화면 격자·parser의
//!   일시 할당 여유([`TRANSIENT_SLACK`])를 더한 값 안이고, 끝난 뒤 늘어난 heap이 `memory_bytes()`와
//!   여유 안에서 같습니다(자체 계산이 실제 요청 heap과 맞음).
//! - **실제 블록 크기**(macOS `malloc_size`): 할당자의 크기 등급 올림을 포함합니다. 상한은 요청 크기로
//!   세므로 이 값은 상한을 조금 넘을 수 있고, 넘는 양이 상한의 1/8 안인지 확인합니다.
//!
//! [`Scrollback::memory_bytes`]: stellar_terminal::emulator::Scrollback::memory_bytes

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write as _;
use std::thread::LocalKey;

use stellar_terminal::emulator::scrollback::MAX_SCROLLBACK_LINES;
use stellar_terminal::emulator::{ScrollbackLimits, Terminal, TerminalConfig};
use stellar_terminal::size::TerminalSize;

// --- 할당 계측 ---

/// System 할당자에 위임하면서 현재 thread의 heap(요청 크기·실제 블록 크기)을 기록하는 할당자.
struct CountingAlloc;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static REQUESTED: Cell<isize> = const { Cell::new(0) };
    static REQUESTED_PEAK: Cell<isize> = const { Cell::new(0) };
    static ACTUAL: Cell<isize> = const { Cell::new(0) };
    static ACTUAL_PEAK: Cell<isize> = const { Cell::new(0) };
}

fn tracking() -> bool {
    TRACKING.try_with(Cell::get).unwrap_or(false)
}

/// 현재 값과 peak를 갱신합니다(할당 없음).
fn record(
    current: &'static LocalKey<Cell<isize>>,
    peak: &'static LocalKey<Cell<isize>>,
    delta: isize,
) {
    let _ = current.try_with(|current| {
        let now = current.get() + delta;
        current.set(now);
        let _ = peak.try_with(|peak| {
            if now > peak.get() {
                peak.set(now);
            }
        });
    });
}

fn signed(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

#[cfg(target_os = "macos")]
extern "C" {
    /// `<malloc/malloc.h>`: malloc zone 블록의 실제 크기(크기 등급 올림 포함).
    fn malloc_size(ptr: *const std::ffi::c_void) -> usize;
}

/// `ptr` 블록의 실제 크기. macOS 밖에서는 재지 않습니다(0).
fn block_size(ptr: *const u8) -> isize {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `ptr`은 System 할당자(macOS에서는 malloc zone)가 돌려준, 아직 해제하지 않은 블록입니다.
        signed(unsafe { malloc_size(ptr.cast()) })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ptr;
        0
    }
}

fn record_alloc(ptr: *const u8, size: usize) {
    record(&REQUESTED, &REQUESTED_PEAK, signed(size));
    record(&ACTUAL, &ACTUAL_PEAK, block_size(ptr));
}

// SAFETY: 모든 연산을 System 할당자에 그대로 위임하고, 부가 작업(크기 기록)은 할당하지 않습니다.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && tracking() {
            record_alloc(ptr, layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc_zeroed 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && tracking() {
            record_alloc(ptr, layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if tracking() {
            // 해제 전에 실제 크기를 읽습니다.
            record(&ACTUAL, &ACTUAL_PEAK, -block_size(ptr));
            record(&REQUESTED, &REQUESTED_PEAK, -signed(layout.size()));
        }
        // SAFETY: ptr은 같은 layout으로 이 할당자(=System)에서 할당된 포인터입니다.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let tracked = tracking();
        let old_block = if tracked { block_size(ptr) } else { 0 };
        // SAFETY: 호출자가 GlobalAlloc::realloc 계약을 지키며, 그대로 System에 전달합니다.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && tracked {
            // 이동 순간 두 블록이 공존할 수 있으므로 새 크기를 먼저 더합니다(보수적 peak).
            record(&REQUESTED, &REQUESTED_PEAK, signed(new_size));
            record(&REQUESTED, &REQUESTED_PEAK, -signed(layout.size()));
            let new_block = block_size(new_ptr);
            if new_ptr == ptr {
                record(&ACTUAL, &ACTUAL_PEAK, new_block - old_block);
            } else {
                record(&ACTUAL, &ACTUAL_PEAK, new_block);
                record(&ACTUAL, &ACTUAL_PEAK, -old_block);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

// --- 측정 ---

/// scrollback 밖의 일시 할당 여유: 기록으로 옮기는 줄 대신 화면에 새로 만드는 빈 줄(80열 1,280 B),
/// 줄 문자열 변환, parser 상태 등. 측정값은 이보다 훨씬 작습니다(보고서).
const TRANSIENT_SLACK: isize = 64 * 1024;

#[test]
fn one_million_output_lines_stay_within_cap_by_allocator_measurement() {
    let cap = 32 * 1024 * 1024;
    let config = TerminalConfig {
        scrollback: ScrollbackLimits::new(MAX_SCROLLBACK_LINES, cap).unwrap(),
        ..TerminalConfig::default()
    };
    let mut terminal = Terminal::with_config(TerminalSize::new(24, 80).unwrap(), config);
    // 입력 buffer는 측정 전에 충분히 잡아 둡니다(측정 중 다시 할당하지 않음).
    let mut chunk = String::with_capacity(64 * 1024);
    let mut line = String::with_capacity(64);

    TRACKING.with(|tracking| tracking.set(true));
    for n in 0..1_000_000u32 {
        line.clear();
        write!(line, "line {n:07} ........................\r\n").unwrap();
        chunk.push_str(&line);
        if chunk.len() > 60 * 1024 {
            terminal.feed(chunk.as_bytes());
            chunk.clear();
        }
    }
    terminal.feed(chunk.as_bytes());
    TRACKING.with(|tracking| tracking.set(false));

    let history = terminal.screen().history();
    let requested_peak = REQUESTED_PEAK.with(Cell::get);
    let requested_end = REQUESTED.with(Cell::get);
    let accounted = signed(history.memory_bytes());
    let cap = signed(cap);
    // 상한이 실제로 일했습니다(100만 줄 대부분을 버림).
    assert!(
        history.len() > 10_000 && history.len() < 1_000_000,
        "{}",
        history.len()
    );
    assert!(
        requested_peak <= cap + TRANSIENT_SLACK,
        "요청 heap peak {requested_peak} > 상한 {cap} + 여유 {TRANSIENT_SLACK}"
    );
    assert!(
        (requested_end - accounted).abs() <= TRANSIENT_SLACK,
        "요청 heap {requested_end}와 자체 계산 {accounted}의 차이가 여유를 넘습니다"
    );

    if cfg!(target_os = "macos") {
        let actual_peak = ACTUAL_PEAK.with(Cell::get);
        assert!(
            actual_peak >= requested_peak - TRANSIENT_SLACK,
            "{actual_peak}"
        );
        assert!(
            actual_peak <= cap + cap / 8,
            "실제 블록 peak {actual_peak} > 상한 {cap} + 1/8"
        );
    }
    eprintln!(
        "cap {cap} B, 자체 계산 {accounted} B, 요청 heap peak {requested_peak} B·끝 {requested_end} B, \
         실제 블록 peak {} B, 남은 기록 {}줄",
        ACTUAL_PEAK.with(Cell::get),
        history.len()
    );
}
