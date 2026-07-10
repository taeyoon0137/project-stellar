# 작업 보고서: V.8.2 migration step registry

- WBS: V.8.2
- 작업명: `MigrationStep { from, to, migrate }` registry — 연속 경로 없으면 거부
- 상태: 완료 (`[x]`)
- 선행: V.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/migration/registry.rs`(신규) — `MigrationRegistry`,
  `MigrationStep`, `MigrationDocument`, `MigrationRegistryError`.
- `crates/storage/src/migration/mod.rs` — `registry` 모듈 등록.
- `docs/task-reports/V.8.2.md`(신규), 계획서(V.8.2 `[ ]`→`[x]`).

## 구현 요약

- **`MigrationStep { from, to, migrate }`**: `migrate`는 in-memory
  `MigrationDocument`(스키마 버전 + logical chunk payload들 + root doc 목록)를
  변환하는 boxed 함수입니다. 버전 갱신은 registry가 수행합니다.
- **등록 규칙**: 역방향·제자리 step 거부(cycle의 근원 차단), 출발 버전당 step
  하나(중복 거부).
- **`plan_path(from, to)`**: 출발 버전에서 step을 따라가는 연속 경로. 끊기면
  `NoPath`(gap), 목표를 지나치면 `NoPath`, 방문 재진입은 `Cycle`(등록 규칙 위에
  이중 방어).
- **`migrate_to`**: 경로 계획 + 순차 실행. step 실패 시 그 지점에서 중단하고
  버전을 갱신하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 189 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| gap fixture 실패(`gap_is_rejected`) | 통과 |
| cycle fixture 실패(역방향 등록 거부 + plan 방어)(`cycle_is_rejected`) | 통과 |
| duplicate step fixture 실패(`duplicate_step_is_rejected`) | 통과 |
| 연속 경로 계획·실행, overshoot 거부, step 실패 중단 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 실제 세대 파일에 대한 migration 실행(원본 보존 + 새 generation 기록)은
  V.8.3/V.8.4에서 구현합니다.
