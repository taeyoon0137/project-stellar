# 작업 보고서: II.1.4 workspace 공통 lint

- WBS: II.1.4
- 작업명: `unsafe_op_in_unsafe_fn`=deny, 공개 API missing docs=warn
- 상태: 완료 (`[x]`)
- 선행: II.1.2 (`[~]`)

## 필수 실행 환경

- `DEV-MAC` (verify 열에 CI 태그 없음).

## 변경 파일 목록

- `Cargo.toml` (`[workspace.lints.rust]`: unused priority -1, `unsafe_op_in_unsafe_fn="deny"`, `missing_docs="warn"`)
- `tools/xtask/src/lib.rs`·`src/main.rs` (`#![allow(missing_docs)]` — 내부 도구 crate, 공개 API 아님)
- `tools/xtask/tests/workspace_lints.rs` (신규, lint 선언 검증)
- `docs/task-reports/II.1.4.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.4 `[ ]` → `[x]`)

## 구현 요약

- `unsafe_op_in_unsafe_fn = "deny"`: unsafe fn 내부 unsafe 연산은 명시적 unsafe 블록 필요(컴파일 강제).
- `missing_docs = "warn"`: 공개 API 문서 누락은 경고. 내부 도구 crate(xtask)는 공개 API가 아니므로 crate 수준 allow. 제품 library crate(II.2~)가 workspace lint를 상속해 공개 API를 문서화합니다.
- lint 그룹 `unused`는 개별 lint와의 우선순위 충돌을 피하려 `priority = -1`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `unsafe_op_in_unsafe_fn="deny"` 선언 | 테스트 통과 |
| `missing_docs="warn"` 선언 | 테스트 통과 |
| `cargo fmt --all -- --check` | 통과 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 종료 0 |
| `cargo test --workspace --locked` | 95 + 4 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- unsafe 위반은 컴파일 단계에서 강제되며 trybuild 미승인이라 compile-fail fixture 대신 lint 선언 검증 테스트로 확인했습니다.
- `missing_docs`는 `-D warnings`와 공존하도록 내부 도구 crate에서만 allow 하고, 공개 API 대상(제품 crate)에는 경고로 적용되게 했습니다.
