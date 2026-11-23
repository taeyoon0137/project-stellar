# 작업 보고서: XVI.8.3 텔레메트리 동의 상태

- WBS: XVI.8.3
- 작업명: telemetry consent 상태 NotAsked, Declined, Granted 정의(기본 NotAsked), 자동 전송은 Granted에서만
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XVI.8.1이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 검증 기준(state transition fixture)을 끝냈습니다.
- 결정: ADR-0109 결정 5
- 선행: XVI.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/consent.rs`(신규).
- `crates/diagnostics/src/consent_transitions.txt`(신규: 전이 fixture).
- `crates/diagnostics/src/lib.rs`(module 등록 한 줄).
- `docs/task-reports/XVI.8.3.md`(신규).

## 구현 요약

- `ConsentState`는 `NotAsked`(기본), `Declined`, `Granted`이고 `allows_automatic_upload`는 `Granted`만 참입니다.
- 사건은 `Grant`, `Decline`, `Revoke`이고 전이 규칙은 `transition` 하나입니다. `Grant`는 어디서나 `Granted`, `Decline`은 어디서나 `Declined`, `Revoke`는 `Granted`에서만 `Declined`이고 그 밖에서는 `nothing_to_revoke` 오류입니다. 실패한 전이는 상태와 시각을 바꾸지 않습니다.
- `Transition::leaves_granted`는 XVI.8.5가 대기열을 버릴 때 씁니다.
- 동의 파일(`stellar.telemetry-consent v1`, `consent state= changed_ms=` 한 줄)은 store와 같은 줄 codec과 소유자 전용 원자 쓰기를 씁니다. 파일이 없으면 `Missing`, 손상·읽기 실패면 `Unreadable`이고 둘 다 `NotAsked`로 닫힙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib consent` | 4 passed, 0 failed |
| `cargo clippy -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `transitions_match_the_fixture`: 검증 기준(state transition fixture). fixture의 9개 (상태, 사건) 쌍 전부에서 결과 상태, 변경 여부, 전송 허용, 시각 갱신, `leaves_granted`, 실패 시 불변을 봅니다. 쌍이 빠지거나 겹치면 실패합니다.
- `default_is_not_asked_and_blocks_upload`.
- `upload_is_allowed_only_after_grant_in_any_sequence`: 사건 열 81개(길이 4) 전부에서 전송 허용은 마지막 성공 사건이 `Grant`일 때뿐입니다.
- `consent_file_round_trips_and_fails_closed`: 파일 bytes golden, round-trip, 손상 7가지(모르는 상태, 끝 줄바꿈 없음, version 불일치, 줄 중복, 머리 줄 없음, 빈 파일, 비UTF-8)가 모두 `Unreadable`이고 전송을 허용하지 않음.

## 성능

- 해당 없음(사용자 사건마다 한 번).

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 동의 화면은 UI 재설계 때 만듭니다. 지금은 Rust API와 CLI(`stellar diagnostics consent`, XVI.8.5 커밋)로 바꿉니다.
