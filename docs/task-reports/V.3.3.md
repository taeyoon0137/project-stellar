# 작업 보고서: V.3.3 chunk write planner

- WBS: V.3.3
- 작업명: 변경된 logical chunk만 새 payload로 encode하고 나머지 pointer를 재사용하는 planner
- 상태: 완료 (`[x]`)
- 선행: V.3.1, V.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/planner.rs`(신규) — `plan_chunk_writes`,
  `LogicalChunkState`, `ChunkWritePlan`, `PlannedWrite`, `PlanError`.
- `crates/storage/src/chunk/mod.rs` — `planner` 모듈 등록.
- `docs/task-reports/V.3.3.md`(신규), 계획서(V.3.3 `[ ]`→`[x]`).

## 구현 요약

- **입력 모델**: 문서 계층이 dirty 여부를 알므로 `LogicalChunkState::Clean`(base
  pointer 재사용, payload 재인코딩 없음) / `Dirty`(새 payload 제공)로 받습니다.
- **no-op 변경 필터**: Dirty payload의 hash가 base와 같으면 기록 없이 재사용합니다.
- **결과**: 새 manifest pointer 목록(chunk_id 오름차순, V.3.2 validator 통과 보장) +
  실제 기록 대상 `writes`. current에 없는 base pointer는 제거로 간주합니다.
- **오류**: clean인데 base pointer 없음, 입력 ChunkId 중복, pointer 집합 검증 실패.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 86 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 단일 속성 변경 fixture: 5개 중 정확히 1개 chunk만 hash 변경·기록(`single_property_change_touches_exactly_one_chunk`) | 통과 |
| no-op dirty 재사용, 신규 chunk 기록, 제거 chunk 탈락, 결정론(같은 입력 → 같은 계획) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- planner는 계획만 만들고 실제 기록(`ChunkStore::write_chunk` 호출)과 manifest 기록은
  저장 트랜잭션 실행기(V.5.3)가 수행합니다.
