# 작업 보고서: II.4.1 FFI runtime C ABI

- WBS: II.4.1
- 작업명: ffi crate에 create/destroy/version C ABI(정수 registry handle)
- 상태: 완료 (`[x]`)
- 선행: II.2.7, II.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (verify 열에 CI 태그 없음; macOS에서 C 테스트 실행).

## 변경 파일 목록

- `crates/ffi/Cargo.toml` (`crate-type = ["lib","staticlib"]`)
- `crates/ffi/src/lib.rs` (registry + C ABI)
- `crates/ffi/tests/runtime_abi.c` (신규, C 테스트)
- `docs/task-reports/II.4.1.md`, 계획서(II.4.1 `[ ]`→`[x]`)

## 구현 요약

- handle은 opaque pointer가 아니라 정수(u64) registry handle(0=무효). 전역 registry는 `OnceLock<Mutex<..>>`.
- `stellar_runtime_create() -> u64`, `stellar_runtime_destroy(handle) -> i32`(double free 시 -1), `stellar_runtime_version(handle, buf, len) -> isize`(호출자 buffer, UTF-8), `stellar_runtime_live_count()`(누수 검증).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo test -p stellar-ffi` | 2 passed |
| staticlib 빌드 | OK |
| C 테스트: create/version/destroy 1,000회 | `ok: 1000 cycles, 0 leaks`, 종료 0 |
| double free 감지(-1) | C 테스트에서 확인 |
| fmt / clippy --locked | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- C 링크는 macOS에서 `-framework CoreFoundation -framework Security`가 필요했습니다(Rust std staticlib). CI에서는 동일 링크 플래그를 사용합니다.
- version은 II.4.3의 caller-buffer 규칙을 미리 적용했습니다(NUL 종료 비의존).
