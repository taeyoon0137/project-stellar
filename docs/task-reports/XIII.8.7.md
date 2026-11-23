# 작업 보고서: XIII.8.7 정상 종료 후 복구 snapshot 삭제와 stale 무시

## WBS

- WBS: XIII.8.7
- 작업명: 정상 종료 후 복구 snapshot 삭제와 stale 무시
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XIII.8.3(`[ ]`)이 `[x]`가 아닙니다. XIII.8.3이 끝나면 근거 커밋 기록만으로 `[x]`입니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.9.4, XIII.8.3
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/app/recovery/lifecycle_tests.rs`(신규)

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
| `app::recovery::lifecycle_tests::normal_save_and_close_remove_the_checkpoint_and_offer_no_prompt` | 통과 — 저장·닫기 뒤 복구본 0, 후보 없음, 낡은 기준 복구본은 다음 시작에서 제안 안 함 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- 없음.

## 후속 (2026-11-23, lane `recovery-git-terminal`)

- 결함: 정상 저장·닫기의 삭제가 실패하면 기준 세대가 current와 같은 복구본(닫으며 버린 편집)이 다음 시작에 다시 제안됐습니다. 위 시험은 기준 세대가 낡은 경우만 확인했습니다.
- 수정: 삭제는 `stellar_project::recovery_shutdown::discard_snapshot`이 맡고, 지울 bytes의 hash를 폐기 표식에 먼저 씁니다. probe 판정(`judge_stored_snapshot`)은 표식과 같은 bytes를 `StaleReason::Discarded`로 제안하지 않습니다([ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 2).
- 선행 XIII.8.3을 같은 lane에서 완료했으므로 이 행은 `[x]` 후보입니다(원장 판정은 통합 때).

| 테스트 | 결과 |
|---|---|
| `app::recovery::lifecycle_tests::failed_removal_after_normal_close_is_stale_on_next_start` | 통과 — 복구본에 macOS 변경 불가 flag를 건 채 창을 정상 닫기 → 삭제 실패, 표식 기록 → 다음 시작 probe `None`(normal close 후 prompt 없음) |
| `recovery_shutdown::tests::failed_removal_leaves_a_marker_that_matches_only_those_bytes` | 통과 — 삭제 실패를 보고, 표식은 그 bytes에만 맞고 새 bytes에는 맞지 않음, 다음 폐기 성공 뒤 표식 정리 |
| `recovery_shutdown::tests::discard_removes_snapshot_and_marker` | 통과 |
| `app::recovery::lifecycle_tests::discarded_checkpoint_left_by_failed_removal_cannot_be_restored` | 통과 — 검토 보완. 사용자 Discard가 삭제에 실패하면 `RequestFailed`, 표식이 남고, 같은 후보로 Restore해도 `RequestFailed`이며 창 문서는 그대로, 같은 시작과 다음 시작의 probe 모두 `None`. 보완 전 코드에서는 Restore가 폐기한 복구본을 창에 복원해 이 시험이 실패했습니다 |

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib app::recovery` | 0 — 8 passed |
| `cargo test -p stellar-project --lib recovery` | 0 — 23 passed |

- 검토 보완: Restore 적용(`prepare_recovery_in`)이 제안과 같은 `judge_stored_snapshot`을 다시 써서 폐기 표식이 가리키는 복구본을 복원하지 않습니다. `discard_snapshot`은 복구본을 지웠으면 표식 기록·정리 실패를 오류로 돌려주지 않습니다(ADR-0120 결정 2).

### 근거 커밋

- `63c95449` Task(XIII.8.3): 앱 시작 복구 probe가 최신 복구본 요약을 돌려주고 복구·폐기를 같은 저장소에 적용(폐기 표식 포함)
