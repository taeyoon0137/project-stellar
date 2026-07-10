# 작업 보고서: V.8.4 temp generation migration과 pointer 전환

- WBS: V.8.4
- 작업명: migration 전체를 temp generation에서 수행, 모든 검증 후 current pointer 전환
- 상태: 완료 (`[x]`)
- 선행: V.8.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/migration/runner.rs` — 단계 중간 실패 fault injection 테스트
  추가(`failure_at_each_save_stage_keeps_original_current`).
- `docs/task-reports/V.8.4.md`(신규), 계획서(V.8.4 `[ ]`→`[x]`).

## 구현 요약

- **temp generation 수행**: migration 결과 기록은 저장 파이프라인(V.5)을 그대로
  사용하므로 chunk·manifest는 전부 content-addressed **새 파일 추가**입니다 —
  current pointer가 전환되기 전까지는 어떤 reader에게도 보이지 않는 temp
  generation입니다.
- **모든 검증 후 전환**: 기록 chunk 재읽기 전체 검증(V.5.3)과 manifest 기록이
  끝난 뒤에야 pointer 전환(V.5.4)이 일어납니다. 실패 시 원본 current가 그대로
  열립니다.
- 이 작업은 V.8.3 runner의 실패 경로를 fault injection으로 고정하는 검증
  중심 변경입니다(runner 자체는 V.8.3에서 이 구조로 구현됨).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 192 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 저장 4단계 + pointer 전환 3지점 실패 주입 전부에서 원본 current 유지·schema v1 로드(`failure_at_each_save_stage_keeps_original_current`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Git working tree 통합 fixture(V.8.5)와 golden fixture runner(V.8.6)는 후속입니다.
