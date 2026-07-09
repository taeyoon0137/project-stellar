# 작업 보고서: V.3.4 chunk remap manifest

- WBS: V.3.4
- 작업명: split/merge 후 외부 참조가 ChunkId를 통해 유지되도록 remap manifest 타입 추가(실제 split은 후속)
- 상태: 완료 (`[x]`)
- 선행: V.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/remap.rs`(신규) — `ChunkRemapManifest`,
  `RemapInsertError`, `RemapValidateError`.
- `crates/storage/src/chunk/mod.rs` — `remap` 모듈 등록.
- `docs/task-reports/V.3.4.md`(신규), 계획서(V.3.4 `[ ]`→`[x]`).

## 구현 요약

- **`ChunkRemapManifest`**: `from → to` ChunkId 재매핑. 등록 시 자기 재매핑·중복
  source를 거부합니다.
- **`resolve(id)`**: chain(`a → b → c`)을 최종 대상까지 따라갑니다. cycle이면 `None`
  (검증 전 데이터 방어).
- **`validate(live)`**: 모든 chain이 cycle 없이 끝나고, 최종 대상이 살아 있는
  ChunkId 집합에 존재해야 합니다. 실제 split/merge planner는 V.6.4/V.6.5에서
  이 타입을 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 92 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| remap chain cycle 거부(`cycle_is_rejected`) | 통과 |
| missing target 거부(`missing_target_is_rejected`) | 통과 |
| chain resolve, 자기 재매핑·중복 source 거부, 빈 manifest 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- remap manifest의 binary 직렬화(generation manifest 내 embed 여부)는 V.4.1 manifest
  설계에서 확정합니다.
