# 작업 보고서: II.4.6 FFI C 헤더 생성·drift 검사

- WBS: II.4.6
- 작업명: `cargo xtask generate-ffi-header`로 C 헤더 생성, 생성물 drift를 CI에서 검사
- 상태: 완료 (`[x]`)
- 선행: II.4.1~II.4.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (생성·단위테스트). CI drift 게이트는 `CI-MAC`/`CI-WIN`에서 push마다 실행.

## 변경 파일 목록

- `tools/xtask/src/ffi_header.rs` (고정 템플릿 + `generate_header`, 단위테스트)
- `tools/xtask/src/lib.rs` (`pub mod ffi_header;`)
- `tools/xtask/src/commands.rs` (`generate-ffi-header` 명령 등록·디스패치, COMMANDS 13→14)
- `crates/ffi/include/stellar_ffi.h` (생성물)
- `.github/workflows/bootstrap.yml` (양 job에 `ffi-header drift` 단계)
- `docs/task-reports/II.4.6.md`, 계획서(II.4.6 `[ ]`→`[x]`)

## 구현 요약

- std-only 제약(승인 의존성 외 추가 금지)에 맞춰 cbindgen 등 외부 도구 대신 **고정 템플릿** 방식 채택. `ffi_header::HEADER` 상수가 헤더 source of truth.
- `cargo xtask generate-ffi-header`는 `crates/ffi/include/stellar_ffi.h`를 생성(부모 디렉터리 자동 생성). `--check`는 디스크 내용과 템플릿을 비교만 하고, 다르면 정책 위반(exit 65) 반환.
- 헤더는 `#[no_mangle] extern "C"` 8개 함수와 `StellarStatus` enum(repr i32)만 노출. Rust 전용 API `enqueue_event`는 C ABI가 아니므로 제외. C++ `extern "C"` 가드 포함.
- CI drift 게이트: 양 job에서 재생성 후 `git diff --exit-code -- crates/ffi/include/stellar_ffi.h`로 검사(verify 기준 "재생성 후 Git diff 없음"과 동일).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `generate-ffi-header` 생성 → `--check` | 생성 후 "drift 없음" |
| 재생성 idempotent(2회 실행, diff 없음) | 통과 |
| `committed_header_matches_template` 단위테스트 | 통과 |
| `header_exposes_each_extern_fn_and_guard`(8 함수 + 가드, enqueue_event 미노출) | 통과 |
| `usage_lists_each_command_once`(COMMANDS 14) | 통과 |
| `cargo test -p xtask` / fmt --check / clippy --locked -D warnings | 통과(103 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "자동 생성하거나 고정 템플릿에서 생성" 중 후자를 선택했습니다(std-only 의존성 정책 준수). 헤더와 Rust 표면의 정합은 단위테스트(함수명 노출 검사)와 검토로 보장합니다.
- CI drift 검사는 `--check`와 동등한 `git diff --exit-code`로 구현해 verify 기준 문구와 일치시켰습니다.
