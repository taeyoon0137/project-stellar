# 작업 보고서: I.3.5 ADR 보호 경로 정책

- WBS: I.3.5
- 작업명: `tools/xtask/src/adr_policy.rs` — 보호 경로 변경 시 ADR 강제
- 상태: 완료 (`[x]`)
- 선행: I.3.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/adr_policy.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod adr_policy` 추가)
- `docs/task-reports/I.3.5.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.3.5 `[ ]` → `[x]`)

## 구현 요약

- `check_adr_policy(changed_paths) -> Vec<AdrError>`.
- 보호 경로: `crates/core-types/`(public API·numeric registry), `crates/storage/`(format), `crates/ffi/`(FFI), `schemas/`, `config/performance-budgets.yaml`(performance budget).
- 보호 경로 변경 + 동반 ADR(`docs/adr/ADR-*.md`, TEMPLATE 제외) 없음 → `missing-adr`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 보호 경로만 변경 | `missing-adr` |
| 보호 경로 + ADR 동반 | 통과 |
| docs-only / TEMPLATE-only | 오탐 0 |
| TEMPLATE는 ADR 요건 미충족 | `missing-adr` 유지 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 80 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 보호 경로 패턴은 아키텍처의 계획 레이아웃 기준으로 정의했습니다(crate가 생성되면 동일 prefix가 적용됨).

## 후속 기록 (사후 리뷰 EB-4: 보호 경로 추가)

I.3.5를 마친 뒤 생긴 `crates/bindings/`와 `crates/expressions/`는 보호 경로 prefix에 없었습니다.
그래서 `ValueBinding` 저장 형식, canonical AST 형식, 환경 입력 registry를 바꿔도 `validate-adr`가
실패하지 않았습니다. 이 결정들을 ADR-0026에 소급 기록한 뒤, 그 결정이 있는 파일 세 개를 정확히
일치하는 보호 경로(`PROTECTED_EXACT`)에 추가했습니다.

- `crates/bindings/src/binding.rs`
- `crates/expressions/src/codec.rs`
- `crates/expressions/src/environment.rs`

두 crate의 나머지 파일(평가기, command, scheduler 등)은 저장 형식도 numeric registry도 아니므로 crate
전체를 보호하지 않았습니다. canonical AST bytes에 들어가는 연산 tag(`ops.rs` 등)와 함수
ID(`functions.rs`)는 ADR에 기록되지 않았고 아직 보호 경로가 아닙니다(ADR-0026 Consequences).

| 검사 | 결과 |
|---|---|
| `binding_and_expression_format_files_are_protected`: 세 파일 단독 변경은 `missing-adr`, ADR 동반 시 통과, `fallback.rs`·`vm.rs` 변경은 통과 | 보호 경로 추가 전 실패, 추가 후 통과 |
