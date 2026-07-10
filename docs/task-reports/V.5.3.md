# 작업 보고서: V.5.3 저장 단계 실행기

- WBS: V.5.3
- 작업명: dirty chunk 기록 → 재읽기 검증 → manifest 기록 단계 실행기
- 상태: 완료 (`[x]`)
- 선행: V.5.2, V.2.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/executor.rs`(신규) — `prepare_save`, `SaveStage`,
  `SaveObserver`, `SaveExecError`, `PreparedSave`.
- `crates/storage/src/save/mod.rs` — `executor` 모듈 등록.
- `docs/task-reports/V.5.3.md`(신규), 계획서(V.5.3 `[ ]`→`[x]`).

## 구현 요약

- **고정 단계 순서**: optimistic base check(V.5.2, 실패 시 무기록) → dirty chunk
  전부 기록(V.2.4 immutable + V.2.5 atomic write) → 기록 chunk 재읽기 전체 검증
  (V.2.6: envelope·length·hash) → manifest 파일 기록(content-addressed).
- **핵심 불변식**: 여기까지의 산출물은 전부 새 파일 추가뿐이고 current pointer는
  건드리지 않으므로, 어느 단계에서 실패해도 current generation이 그대로 열립니다.
  pointer 전환은 V.5.4.
- **fault injection**: `SaveObserver`가 각 단계 직전 호출됩니다(주입 중단 모사).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 135 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 4개 단계 각각 중단 후 current generation 완전 로드 유지(`failure_at_each_stage_keeps_current_generation_loadable`) | 통과 |
| 기록·검증 사이 손상 주입을 재읽기 검증이 탐지(`corrupted_written_chunk_is_caught_by_verify`) | 통과 |
| stale base에서 chunk 미기록, 정상 prepare 후 current 불변 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 실패 시 남는 orphan chunk·manifest 파일 정리는 GC(V.7) 소관입니다.
