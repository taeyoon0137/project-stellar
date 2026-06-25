# 작업 보고서: II.4.2 StellarStatus와 handle별 last_error

- WBS: II.4.2
- 작업명: 모든 FFI 함수가 `StellarStatus` 반환 + handle별 `stellar_last_error`(전역 금지)
- 상태: 완료 (`[x]`)
- 선행: II.4.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/ffi/src/lib.rs` (StellarStatus 반환 + handle별 last_error)
- `crates/ffi/tests/runtime_abi.c` (새 시그니처로 갱신)
- `docs/task-reports/II.4.2.md`, 계획서(II.4.2 `[ ]`→`[x]`)

## 구현 요약

- `StellarStatus`(repr i32): Ok/InvalidHandle/NullPointer/BufferTooSmall/NoError/Internal.
- create(out_handle)/destroy/version(…,out_written)/last_error(…)가 모두 StellarStatus 반환. handle은 정수 registry handle 유지.
- last error는 전역이 아니라 각 Runtime(handle)에 보관. `stellar_last_error(handle, …)`로 읽음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| handle별 last_error 격리(두 handle 동시 오류, "buffer 1"/"buffer 2" 안 섞임) | 통과 |
| null out_handle → NullPointer | 통과 |
| create/version/destroy + double free(InvalidHandle) | 통과 |
| C 테스트 1,000회(새 시그니처) | `ok: 1000 cycles, 0 leaks`, 종료 0 |
| `cargo test -p stellar-ffi` / fmt / clippy --locked | 통과(3 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- `stellar_runtime_live_count`는 상태가 아닌 카운트를 반환하는 진단 helper로 유지했습니다(작업 함수가 아님).
