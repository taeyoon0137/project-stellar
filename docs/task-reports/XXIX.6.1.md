# 작업 보고서: XXIX.6.1 테스트 대역 기반 E2E 시나리오 공통 fixture

- WBS: XXIX.6.1
- 작업명: 네 E2E 시나리오의 공통 정의 형식, loader, headless 실행기
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXIX.3.5, XXIX.2.8이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: `[x]` 완료
- 선행: XXIX.3.5, XXIX.2.8
- 설계 기준: ADR-0092 결정 2, ADR-0101 결정 5

## 필수 실행 환경

- `DEV-MAC`. 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/scenario.rs`(신규): 형식 `stellar-agent-e2e-scenario/1` loader, `Scenario`, `Step`, `Action`, `Check`, `APP_EXPECTATIONS`
- `crates/agent-host/tests/fixtures/e2e/{complete-undo,stop-then-revert,reconnect-grace,decode-failure}.json`(신규), `INDEX.golden`
- `crates/agent-host/tests/e2e_scenarios.rs`(신규): golden, loader 거부 사례, headless 실행기
- `crates/agent-host/src/lib.rs`: module 등록

## 구현 요약

- **형식**: JSON 하나에 id, 설명, 테스트 대역 profile 종류, 모드, 대역 `[session]` 명령 줄, 시각이 붙은 단계 목록을 둡니다.
  - 단계는 닫힌 목록입니다: `start`, `launch`, `bridge-bound`, `bridge-disconnected`, `tick`, `grant`, `lease`, `open-scope`, `open-batch`, `answer`, `apply`, `complete`, `stop`, `release-lease`, `await-output`, `app`, `check`.
  - `check`는 상태, lease 수, 권한, 대기 요청·제안 수, 적용 batch 수를 확인합니다. 이어서 지금까지의 효과 이름(정확히 일치), 채팅 item 종류(포함), 승인 결과 순서, 앱 실행기 전용 확인을 봅니다.
- **loader 거부**: 모르는 key·단계·이름, 형식 이름 불일치, 시각 역행, 첫 단계가 `start`가 아님, 열린 적 없는 요청 번호.
- **두 실행기**:
  - headless 실행기(`tests/e2e_scenarios.rs`)는 실제 테스트 대역 CLI 프로세스, 시작 확인(XXIX.3.5), `AgentSession`, 승인 저장소(XXIX.3.2), 채팅 조립(XXIX.3.6)으로 실행합니다. 매 `check`에서 문서 효과 표시와 기록의 일치도 확인합니다. bridge 결합은 단계로 흉내 냅니다.
  - 앱 실행기(XXIX.6.2)는 같은 정의에 더해 `app` 단계와 `check.app` 확인을 실행합니다. 문서·history·출처 색인이 필요한 부분입니다.
- **네 시나리오**(계획서 순서):

| id | 내용 | headless가 확인하는 것 | 앱 실행기 확인 |
|---|---|---|---|
| `complete-undo` | 시작, 범위 승인, batch 승인·적용, 완료, Undo 1회 | 완료 뒤 lease 0, 대기 0, 효과 `undo-entry`·`release-leases`, 승인 결과 | `history-entries-1`, `provenance-shown`, `undo-restores-baseline`, `default-view-chat` |
| `stop-then-revert` | batch 둘 적용, 제안 하나 대기 중 중지, 변경·세션 되돌리기 | 중지 뒤 적용 batch 2 유지, lease 0, 권한 0, 대기 제안 거부(`denied:session-ended`), 종료 효과 전체 | `applied-batches-kept`, `provenance-shown`, `provenance-matches-index`, `no-auto-rollback`, `revert-batch-baseline`, `revert-session-baseline` |
| `reconnect-grace` | 바로 적용 모드, 끊김·유예 안 재결합, 다시 끊김·유예 초과 | 재연결 중 lease·권한 유지, 재결합 뒤 효과 0, 유예 초과 뒤 중단됨 entry 효과와 적용 batch 유지 | `applied-batches-kept`, `provenance-shown`, `no-auto-rollback` |
| `decode-failure` | batch 적용 뒤 해석할 수 없는 출력 | 상태 running, lease·권한 그대로, 효과 0, `decode-stopped` item | `applied-batches-kept`, `no-auto-rollback` |

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test e2e_scenarios` | 3 passed |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 시나리오 정의 golden | `scenario_definitions_match_golden` | 통과(네 시나리오, 파일 이름 = id, 반복 parse 동일) |
| fixture 로더 test | `loader_rejects_malformed_scenarios`, `headless_runner_executes_session_level_steps` | 통과(거부 8 사례, 네 시나리오 headless 실행) |

## 차이(작업 지시와 다른 점)

- 없음. 앱 실행기(XXIX.6.2)는 이 작업 범위가 아닙니다.

## 남은 위험 / 한계

- headless 실행기는 bridge 결합을 실제 IPC 대신 단계로 흉내 냅니다. 실제 bridge 재결합은 XXIX.6.2에서 봅니다.
- headless 실행기의 batch 근거는 표지 값입니다. 실제 근거는 앱 실행기가 코어 결과로 채웁니다.
