//! type 오류 보고 비용 회귀 테스트 (EXPR-1).
//!
//! `Add(ref_0, Add(ref_1, …))` 사슬에서 `ref_i`가 모두 타입을 모르는 참조이면 깊이 `i`마다 오류가 하나씩
//! 납니다. 이전 type checker는 오류마다 root→node 경로 `Vec`을 새로 만들어, 사슬 길이 `n`에 대해 할당이
//! O(n²)이었습니다(n = 8,000에서 경로 ID 약 3,200만 개, 약 512 MB). 문서 load 경로
//! (`ExpressionAst::from_canonical_bytes` → `Program::compile`)가 그대로 닿으므로, 비용은 입력 크기에 선형이어야
//! 합니다.
//!
//! 시간 대신 **할당 bytes 총합**(테스트 바이너리 전용 계측 할당자, System 위임)을 작업량으로 셉니다. 결정적이고
//! 실행 환경 부하와 무관합니다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::bytecode::{CompileError, Program};
use stellar_expressions::ops::BinaryOp;
use stellar_expressions::typecheck::type_check;
use stellar_expressions::types::TypeSpec;

// --- 할당 계측 ---

/// System 할당자에 위임하면서 현재 thread가 계측 중에 할당한 bytes 총합을 기록하는 할당자.
struct CountingAlloc;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

/// 계측 중이면 현재 thread의 할당 총합에 더합니다(할당 없음).
fn record(size: usize) {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = ALLOCATED.try_with(|total| total.set(total.get().saturating_add(size)));
        }
    });
}

// SAFETY: 모든 연산을 System 할당자에 그대로 위임하고, 부가 작업(크기 기록)은 할당하지 않습니다.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc_zeroed 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr은 같은 layout으로 이 할당자(=System)에서 할당된 포인터입니다.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::realloc 계약을 지키며, 그대로 System에 전달합니다.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // 늘어난 만큼만 새 작업으로 셉니다.
            record(new_size.saturating_sub(layout.size()));
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

/// `f` 실행 동안 현재 thread가 할당한 bytes 총합을 잽니다.
fn allocated_by<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATED.with(|total| total.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let out = f();
    TRACKING.with(|tracking| tracking.set(false));
    (out, ALLOCATED.with(Cell::get))
}

// --- fixture ---

fn add_id(n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(1, 0, n))
}

fn ref_id(n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(2, 0, n))
}

fn unknown_token(n: u64) -> ReferenceTarget {
    ReferenceTarget::Token(TokenRef::new(StableId::new_v7(3, 0, n)))
}

/// `Add(ref_0, Add(ref_1, … Add(ref_{n−1}, ref_n)))`. 모든 `ref_i`는 등록되지 않은 token이라 오류는 `n + 1`개이고,
/// `ref_i`의 root→node 경로 길이는 `i + 2`(마지막은 `n + 1`)입니다. canonical bytes로 왕복한 AST와 그 bytes 길이를
/// 돌려줍니다(문서 load 경로와 같은 입력).
fn unknown_reference_chain(n: u64) -> (ExpressionAst, usize) {
    let mut nodes = Vec::with_capacity(2 * n as usize + 1);
    for i in 0..n {
        let rhs = if i + 1 == n { ref_id(n) } else { add_id(i + 1) };
        nodes.push(ExprNode::binary(add_id(i), BinaryOp::Add, ref_id(i), rhs));
    }
    for i in 0..=n {
        nodes.push(ExprNode::reference(ref_id(i), unknown_token(i)));
    }
    let ast = ExpressionAst::new(add_id(0), nodes).expect("구조 유효");
    let bytes = ast.to_canonical_bytes().expect("직렬화");
    let restored = ExpressionAst::from_canonical_bytes(&bytes).expect("복원");
    (restored, bytes.len())
}

fn no_types() -> BTreeMap<ReferenceTarget, TypeSpec> {
    BTreeMap::new()
}

/// 문서 load 경로(`Program::compile`)의 할당 총합과 보고된 오류 수.
fn compile_cost(n: u64) -> (usize, usize, usize) {
    let (ast, input_bytes) = unknown_reference_chain(n);
    let types = no_types();
    let (result, allocated) = allocated_by(|| Program::compile(&ast, &types));
    let errors = match result {
        Err(CompileError::TypeErrors(errors)) => errors.len(),
        other => panic!("type 오류 기대: {other:?}"),
    };
    (allocated, errors, input_bytes)
}

#[test]
fn every_error_in_a_deep_chain_is_still_reported() {
    let n = 2_000;
    let (ast, _) = unknown_reference_chain(n);
    let report = type_check(&ast, &no_types());
    assert_eq!(report.errors().len(), n as usize + 1, "오류 누락 없음");
    let nodes: Vec<ExprNodeId> = report.errors().iter().map(|error| error.node).collect();
    let expected: Vec<ExprNodeId> = (0..=n).map(ref_id).collect();
    assert_eq!(nodes, expected, "후위 순서(왼쪽 참조 먼저)");
}

#[test]
fn type_error_reporting_cost_is_linear_in_input_size() {
    let (small, small_errors, small_input) = compile_cost(1_000);
    let (large, large_errors, large_input) = compile_cost(8_000);
    assert_eq!(small_errors, 1_001);
    assert_eq!(large_errors, 8_001);
    // 입력이 8배이면 선형 비용은 약 8배, 이전 O(n²) 경로 복제는 약 64배였습니다.
    let ratio = large as f64 / small as f64;
    assert!(
        ratio < 12.0,
        "입력 8배에 할당 {ratio:.1}배({small} → {large} bytes): 선형이 아님"
    );
    // 입력 bytes당 할당 상한(타입·부모 map·오류 목록의 고정 비용). 이전 구현은 n = 8,000에서 약 600배였습니다.
    for (allocated, input) in [(small, small_input), (large, large_input)] {
        assert!(
            allocated <= 32 * input,
            "입력 {input} bytes에 할당 {allocated} bytes(32배 초과)"
        );
    }
}
