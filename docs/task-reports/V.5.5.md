# 작업 보고서: V.5.5 저장 취소 token

- WBS: V.5.5
- 작업명: 저장 취소 token — pointer 전환 시작 이후에는 취소 무시
- 상태: 완료 (`[x]`)
- 선행: V.5.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/cancel.rs`(신규) — `CancelToken`, `CancelPoint`,
  `run_save`, `run_save_observed`, `SaveRunError`.
- `crates/storage/src/save/mod.rs` — `cancel` 모듈 등록.
- `docs/task-reports/V.5.5.md`(신규), 계획서(V.5.5 `[ ]`→`[x]`).

## 구현 요약

- **`CancelToken`**: `Arc<AtomicBool>` 기반, 복제해 다른 스레드에서 취소 가능.
- **취소 지점**: prepare(V.5.3) 각 단계 직전 + pointer 전환 직전. 취소되면
  `SaveRunError::Cancelled { point }` — current pointer·메모리 상태 모두 불변.
- **전환 시작 이후 무시**: `commit_save`(V.5.4) 진입 후에는 token을 확인하지
  않습니다. atomic replace 하나뿐인 전환을 중간에 끊어 "절반 전환"을 만들지 않기
  위한 정책입니다.
- **`run_save`**: prepare → 최종 취소 확인 → commit을 잇는 전체 파이프라인.
  V.5.6 fault matrix가 이 경로를 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 142 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| prepare 4개 단계 각각 직전 취소: 지점 보고 정확 + pointer·메모리 불변(`cancel_before_each_prepare_stage_keeps_everything_unchanged`) | 통과 |
| pointer 전환 직전 취소: current 유지(`cancel_before_pointer_switch_keeps_current`) | 통과 |
| 전환 시작 후 취소 요청 무시, 저장 완료(`cancel_after_pointer_switch_started_is_ignored`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 취소로 남는 orphan chunk/manifest 파일은 GC(V.7)가 정리합니다.
