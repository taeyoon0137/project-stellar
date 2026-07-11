# 작업 보고서: V.9.4 정상 종료 marker와 snapshot cleanup

- WBS: V.9.4
- 작업명: 정상 종료 marker + snapshot cleanup, 비정상 종료 simulation에서 snapshot 유지
- 상태: 완료 (`[x]`)
- 선행: V.9.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/recovery_shutdown.rs`(신규) — `begin_window_session`,
  `end_window_session_clean`, `was_clean_shutdown`.
- `crates/project/src/lib.rs` — 모듈 등록.
- `docs/task-reports/V.9.4.md`(신규), 계획서(V.9.4 `[ ]`→`[x]`).

## 구현 요약

- **수명주기**: 세션 시작 시 이전 정상 종료 marker(`<uuid>.clean-exit`)를 제거
  → 실행 중 crash면 marker 없음(비정상 종료 판정). 정상 종료 시 스냅샷을
  삭제하고 marker를 atomic write로 기록합니다.
- **비정상 종료 simulation**: 정상 종료 함수가 호출되지 않는 것 자체가 crash
  모델입니다 — 스냅샷이 남고 marker가 없어 다음 실행에서 복구 제안(V.9.3)
  대상이 됩니다.
- marker 파일은 스냅샷 확장자와 달라 스냅샷 목록(V.9.1)에 나타나지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | 135 passed, 0 failed |
| `cargo test --workspace` | 실패 0 |
| `cargo fmt -p stellar-project -- --check` | 통과 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 정상 종료: snapshot 삭제 + marker 존재(`clean_shutdown_removes_snapshot_and_leaves_marker`) | 통과 |
| 비정상 종료: snapshot 유지 + marker 없음(`crash_leaves_snapshot_and_no_marker`) | 통과 |
| 재시작 시 marker 초기화, 스냅샷 없는 정상 종료, marker 목록 미노출 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 셸(창 수명주기)과의 연결은 앱 계층에서 수행합니다. crash 후 남은 스냅샷의
  보존 기간 정책은 정하지 않았습니다(복구 UI에서 사용자 선택 후 삭제).
