# 작업 보고서: XXIX.3.5 내장 세션 시작 전 신뢰 capability 검사와 시작 확인

- WBS: XXIX.3.5
- 작업명: untrusted 프로젝트 시작 거부, 시작 확인을 첫 연결 승인으로 기록, 약관 고지 기록(OS 사용자 데이터), 신뢰 철회 시 즉시 중지
- 상태: `[x]` 완료(엔진 부분). 시작·고지 화면은 UI 재설계 때 만듭니다.
- 선행: XXIX.3.2, XVI.7.1, XIX.3.3, XXIX.2.7
- 설계 기준: ADR-0005 §4.1 약관 고지·§4.5 세션 시작 조건, 아키텍처 31.3, ADR-0101 결정 3

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/src/start.rs`(신규): `McpCapability`, `prepare_start`, `confirm_start`, `StartConfirmation`, `FirstConnectionApproval`, `StartedSession`, `StartRefusal`, `on_trust_change`, `TermsNoticeStore`, `NoticeError`
- `crates/agent-host/src/profile.rs`: `TermsResult::name`
- `crates/runtime/src/agent.rs`(신규): `project_key`, `mcp_capability`, `prepare_agent_start`, `confirm_agent_start`, `on_project_trust_change`와 단위 test
- `crates/runtime/Cargo.toml`, `Cargo.lock`: runtime → agent-host 의존 한 줄
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`: runtime 허용 edge에 agent-host(ADR-0005 §8 표가 이미 허용한 edge)
- `crates/agent-host/tests/start_gate.rs`(신규), `crates/agent-host/src/lib.rs`·`crates/runtime/src/lib.rs`: module 등록

## 구현 요약

- **신뢰 판정**: runtime `mcp_capability(trust)`가 `capability_level(ProjectCapability::Mcp, trust)`(XVI.7.1 표) 결과를 그대로 넘깁니다. `Allowed`만 허용이고 제한·금지는 거부입니다. agent-host는 판정하지 않습니다.
- **시작 순서**:
  1. `prepare_start`: 신뢰와 profile 활성을 확인하고 `StartConfirmation`을 만듭니다. 담는 값은 대상 프로젝트, 창, profile id, CLI 종류·이름, 모드, 고지 필요 여부입니다.
  2. 고지가 필요하면 사용자가 확인한 뒤 앱 UI가 `TermsNoticeStore::acknowledge`를 부릅니다.
  3. `confirm_start`: 확인 순간에 신뢰·profile·고지를 다시 확인합니다. 통과하면 `AgentSession`(상태 `Starting`, 권한·lease 0)과 `FirstConnectionApproval`(프로젝트, 창, 세션, profile, 모드, 시각)을 만듭니다.
  4. 그 뒤 실행 전 확인(XXIX.2.7)과 실행기가 이어집니다.
- **약관 고지 기록**: `<OS 사용자 데이터 디렉터리>/agent/terms-notices`에 둡니다.
  - 첫 줄은 `stellar-agent-terms-notice 1`이고, CLI마다 `<cli> <약관 검토 일자> <검토 결과>` 한 줄입니다.
  - 처음 쓰는 CLI와, 기록과 profile의 검토 일자·결과가 다른 CLI는 다시 고지합니다.
  - 디렉터리 0700, 파일 0600이고 임시 파일에 쓴 뒤 rename합니다. symlink는 거부합니다.
  - 형식이 틀린 파일은 덮어쓰지 않고 오류로 닫습니다(시작 0). 열기만으로는 파일을 만들지 않습니다.
- **신뢰 철회**: `on_trust_change`·`on_project_trust_change`가 허용이 아니게 된 순간 `TrustRevoked`로 XIX.6.4 종료 절차를 시작합니다. 승인 저장소는 같은 효과로 대기 요청을 세션 종료 거부로 닫습니다(XXIX.3.2).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test start_gate` | 6 passed |
| `cargo test -p stellar-runtime --lib agent::` | 5 passed |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo test -p xtask crate_graph` | 46 passed |
| `cargo clippy -p stellar-agent-host -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| untrusted 시작 0 | `untrusted_project_never_starts`(runtime, 실제 `FileTrustStore`), `untrusted_and_disabled_profiles_never_start` | 통과(확인 데이터 단계와 확인 단계 모두 거부, 공식 비활성 profile도 거부) |
| 확인 기록 존재 | `trusted_start_records_first_connection_and_revocation_stops`, `confirmation_is_recorded_as_first_connection_approval` | 통과 |
| 고지 미확인 시 시작 0 | `notice_is_required_before_first_use`(runtime), `confirmation_is_recorded_as_first_connection_approval` | 통과 |
| 검토 결과 변경 후 재고지 | `notice_is_asked_again_after_review_changes` | 통과(검토 일자·결과 변경, 다른 CLI) |
| 철회 fixture에서 lease 0·대기 승인 0·적용 변경 자동 삭제 0 | `trusted_start_records_first_connection_and_revocation_stops`, `trust_revocation_stops_with_termination_procedure` | 통과 |
| 고지 기록 형식·권한·손상·symlink | `notice_record_is_private_minimal_and_fails_closed` | 통과 |

## 차이(작업 지시와 다른 점)

- runtime 허용 의존 추가는 계획서상 XXIX.3.1 범위입니다. 그중 agent-host edge만 이 작업에서 먼저 더했습니다. 신뢰 capability 표(project crate)와 시작 확인(agent-host)을 잇는 곳이 runtime뿐이기 때문입니다. FFI는 바꾸지 않았습니다(ADR-0101 결정 3).

## 남은 위험 / 한계

- `FirstConnectionApproval`을 권한 저장소의 첫 연결 승인으로 넣는 연결은 권한 저장소가 앱 crate로 옮겨질 때 합니다(ADR-0093 결정 12).
- OS 사용자 데이터의 앱 전용 하위 디렉터리 이름은 셸이 정합니다(신뢰 목록과 같은 규칙). 시작·고지 화면은 UI 재설계 때 만듭니다.
- 세션 중 신뢰 변경 알림을 받는 앱 경로(신뢰 저장소 변경 → `on_project_trust_change`)는 앱 runtime 연결(XXIX.3.4) 때 붙입니다.
