//! 평가 메모리 상한 회귀 테스트 (VIII.4.5 보완).
//!
//! 한 번만 저장된 큰 문자열 literal `S`를 `concat(S, concat(S, …))`처럼 오른쪽으로 중첩하면, evaluator는
//! 왼쪽 인자 `S`의 복제를 operand stack에 계속 쌓습니다. instruction budget·call depth·stack depth만으로는
//! 이 증폭(수준당 1 MiB)을 막지 못하므로, evaluator는 stack·캐시가 보유한 값 bytes를
//! [`EvalLimits::max_value_bytes`]로 제한해야 합니다.
//!
//! 테스트 바이너리 전용 계측 할당자(System 위임)로 평가 호출 하나의 peak heap을 재서 상한을 확인합니다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::bytecode::Program;
use stellar_expressions::eval::{EvalErrorKind, MAX_STRING_BYTES};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

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

// --- fixture ---

const MIB: usize = 1 << 20;

fn id(n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(n, 0, n))
}

/// 공유 literal `S`(`MAX_STRING_BYTES` bytes)와 `levels`단계 오른쪽 중첩 `concat(S, …)`.
fn nested_shared_concat(levels: u64) -> ExpressionAst {
    let shared = id(1_000_000);
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::call(id(n), FunctionId::Concat, &[shared, id(n + 1)]))
        .collect();
    nodes.push(ExprNode::literal(
        id(levels + 1),
        TypedValue::String("x".to_string()),
    ));
    nodes.push(ExprNode::literal(
        shared,
        TypedValue::String("s".repeat(MAX_STRING_BYTES)),
    ));
    ExpressionAst::new(id(1), nodes).expect("유효")
}

fn no_types() -> BTreeMap<ReferenceTarget, TypeSpec> {
    BTreeMap::new()
}

fn no_values() -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
    BTreeMap::new()
}

/// 기본 한도의 값 bytes 상한 + 진행 중인 함수 결과 1개 + 고정 여유.
fn peak_bound() -> usize {
    EvalLimits::DEFAULT.max_value_bytes + 2 * MAX_STRING_BYTES + 4 * MIB
}

#[test]
fn bytecode_evaluation_of_shared_string_amplification_is_memory_bounded() {
    // 수정 전: 400단계면 operand stack에 S 복제 400개(≈400 MiB)가 쌓였습니다.
    let ast = nested_shared_concat(400);
    let program = Program::compile(&ast, &no_types()).expect("컴파일");
    let values = no_values();
    let (result, peak) = measure(|| program.evaluate(&values));
    let error = result.expect_err("값 bytes 상한 초과");
    assert_eq!(
        error.kind,
        EvalErrorKind::ValueMemoryExceeded {
            limit: EvalLimits::DEFAULT.max_value_bytes
        }
    );
    assert!(error.node.is_some(), "중단 지점 node 기록");
    assert!(
        peak <= peak_bound(),
        "peak heap {peak} bytes > 상한 {} bytes",
        peak_bound()
    );
}

#[test]
fn raised_value_limit_reaches_string_length_limit() {
    // 상한을 올리면 같은 구조도 기존 문자열 길이 한도(StringTooLong)에서 멈춥니다.
    let ast = nested_shared_concat(8);
    let program = Program::compile(&ast, &no_types()).expect("컴파일");
    let limits = EvalLimits {
        max_value_bytes: 64 * MIB,
        ..EvalLimits::DEFAULT
    };
    let error = program
        .evaluate_with_limits(&no_values(), limits)
        .expect_err("문자열 길이 초과");
    assert_eq!(error.kind, EvalErrorKind::StringTooLong);
}
