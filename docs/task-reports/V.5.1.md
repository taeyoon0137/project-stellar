# 작업 보고서: V.5.1 SavePlan

- WBS: V.5.1
- 작업명: `SavePlan`에 base generation, target generation, dirty logical chunks, new manifest 정의
- 상태: 완료 (`[x]`)
- 선행: V.3.3, V.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/mod.rs`(신규) — save 모듈 선언.
- `crates/storage/src/save/plan.rs`(신규) — `SavePlan`, `build_save_plan`,
  `SavePlanError`.
- `crates/storage/src/lib.rs` — `pub mod save;` 등록.
- `docs/task-reports/V.5.1.md`(신규), 계획서(V.5.1 `[ ]`→`[x]`).

## 구현 요약

- **`SavePlan`**: base generation number·manifest hash(optimistic check용, V.5.2),
  target generation(base+1, 첫 저장은 1), dirty chunk 목록(V.3.3 `PlannedWrite`),
  검증 완료된 새 `GenerationManifest`를 담습니다.
- **`build_save_plan`**: base(`LoadedGeneration`, 첫 저장은 `None`) + 현재 logical
  chunk 상태 + root document 목록으로부터 계획을 만듭니다. chunk 계획은 V.3.3
  planner를 사용하고, manifest는 V.4.1 검증을 통과해야 반환됩니다.
- 계획은 순수 함수 결과이며 디스크를 건드리지 않습니다(실행은 V.5.3).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 127 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 동일 문서 상태에서 동일 SavePlan(+ manifest bytes·hash 동일)(`identical_document_state_yields_identical_plan`) | 통과 |
| base·target generation 기록, 첫 저장(base 없음), 잘못된 상태 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- optimistic check(V.5.2)와 실행기(V.5.3)가 이 계획을 소비합니다.
