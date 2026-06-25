# 작업 보고서: II.4.4 FFI catch_unwind

- WBS: II.4.4
- 작업명: FFI 경계에 catch_unwind 적용, panic → InternalPanic 변환
- 상태: 완료 (`[x]`)
- 선행: II.4.2, II.1.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/ffi/src/lib.rs` (`ffi_guard` + 모든 경계 함수 래핑, `StellarStatus::InternalPanic`)
- `docs/task-reports/II.4.4.md`, 계획서(II.4.4 `[ ]`→`[x]`)

## 구현 요약

- `ffi_guard`(catch_unwind + AssertUnwindSafe)로 모든 extern "C" 경계 함수를 감싸 panic을 `InternalPanic`(7)로 변환 — 프로세스 종료 방지.
- 정상 경로는 panic 없이 상태를 반환(release `panic=abort`와 정합).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 테스트 전용 panic → `InternalPanic`(프로세스 생존) | 통과 |
| edge 입력(잘못된 handle·null)에서 panic 없이 상태 반환 | 통과 |
| C 테스트 1,000회(ABI 불변) | `ok: 1000 cycles, 0 leaks` |
| `cargo test -p stellar-ffi` / fmt / clippy --locked | 통과(8 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- panic 발생 함수는 별도 production API 대신 테스트 내 `ffi_guard(|| panic!())`로 검증(테스트 시 panic hook을 잠시 억제).
