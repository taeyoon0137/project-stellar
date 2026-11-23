# 작업 보고서: XII.7.5 UseCurrentWindow: 최신 디스크 generation 위 창 snapshot 저장

## WBS

- WBS: XII.7.5
- 작업명: UseCurrentWindow: 최신 디스크 generation 위 창 snapshot 저장
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.7.2, V.5.4
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/window_session.rs`: `resolve_stale`(UseCurrentWindow, 내부 `save_over_latest_with_lock_dir`)

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `window_session::stale_tests::use_current_window_writes_the_window_snapshot_over_the_latest_disk` | 통과 — 선택 후 디스크 문서 = 창 snapshot, 다른 root 문서 유지, history 유지 |
| `window_session::stale_tests::use_current_window_rechecks_the_base_just_before_commit` | 통과 — 최신을 읽은 뒤 commit 전 다른 저장이 끝나면 다시 stale, 쓰기 0 |
| `window_session::stale_tests::use_current_window_does_not_overwrite_an_external_save_after_the_judgement` | 통과 — writer 출처 판정 뒤 CLI가 저장하면 그 generation을 덮지 않고 다시 stale, 재판정은 reconciliation 대기 |
| `app::autosave::pipeline_tests::stale_autosave_requests_one_alert_and_writes_nothing_before_a_choice` | 통과 — 앱 runtime 경로 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- 외부 writer(CLI 등)와 겹친 stale에서는 실행하지 않습니다(`ReconciliationRequired`). XIII.3.4가 연결될 때까지 그 창은 대기합니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.7.5)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `75f2b026` Feat: 창 session의 stale 두 선택 실행과 외부 변경 command 반영 추가 (XII.7.1·XII.7.5·XII.7.6·UX.4.1)
- `aaa3b048` Fix: UseCurrentWindow가 출처를 판정한 generation 위에서만 쓰게 함
- `ba9dc68e` Feat: 앱 runtime에 stale 선택·외부 변경 감시 반영·닫기와 Git UI flush·trace 연결 (XII.7.7·UX.4.1·UX.4.4·XIII.1.6·XIV.2.8·XIII.1.7)
