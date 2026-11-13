# 작업 보고서: VII.4.5 slider·scrub begin/end grouping

- WBS: VII.4.5
- 작업명: slider/scrub begin/end grouping, 취소 시 preview 전 상태로 돌아가고 history를 만들지 않음
- 상태: 완료 (`[x]`)
- 선행: VII.4.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/history.rs` — `begin_scrub`, `cancel_preview`, 모듈 문서, scrub fixture 테스트 2건.
- `docs/task-reports/VII.4.5.md`(신규), 계획서(VII.4.5 `[ ]`→`[x]`).

## 구현 요약

- **begin/update/end**: scrub은 VII.4.2 preview 경로(`update_preview`가 직전 preview를 원자적으로 교체, `end_preview`가 한 항목 확정)를
  `Scrub { control }` grouping key로 그대로 씁니다.
- **cancel**: 적용 중인 preview의 역(gesture 시작 전 상태로의 transaction)을 현재 revision으로 rebase해 실행하고 preview를 닫습니다. history
  항목은 만들지 않습니다(§23.3). 되돌리기가 실패하면 문서는 그대로(원자 실행)이고 preview도 유지되어 다시 시도할 수 있습니다. pointer
  gesture도 같은 `cancel_preview`로 취소할 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 14 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| corner radius scrub 50 update → end 후 항목 1건(Scrub key), 역 적용 시 원복(`scrub_commit_records_one_entry`) | 통과 |
| 앞선 기록 1건 뒤 scrub 30 update → cancel: snapshot = preview 전, 항목 수 그대로, update 없는 취소·비활성 취소 거부, 취소 후 일반 기록 정상(`scrub_cancel_restores_pre_preview_state_without_history`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
