# 작업 보고서: XXIX.6.5 ADR-0005 §3 변경 불가 경로 inventory test

- WBS: XXIX.6.5
- 작업명: 프로젝트 파일 편집과 에이전트 경로로 신뢰·권한·모드·출처 표시·되돌리기를 바꾸는 경로가 없음을 검사
- 상태: `[x]` 완료
- 선행: XIX.9.7, XXIX.3.3
- 설계 기준: ADR-0005 §3 변경 불가 경로·Security, ADR-0101 결정 6

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/tests/immutable_paths.rs`(신규)
- `crates/agent-host/tests/support/scan.rs`(신규, 소스 inventory 도구), `crates/agent-host/tests/support/mod.rs`(module 한 줄)
- 프로젝트 파일 변조 fixture: `crates/runtime/src/agent.rs`의 `project_file_tampering_changes_no_trust_mode_or_notice_state`(XXIX.3.5 커밋)

## 구현 요약

- **에이전트 경로**: `crates/mcp-core`, `tools/stellar-mcp`, `crates/agent-protocol`, `crates/local-ipc`, `tools/stellar-cli`의 제품 코드(주석·단위 테스트·tests 디렉터리 제외)입니다.
- **쓰기 표지**(이 경로에서 0이어야 함):

| 상태 | 표지 |
|---|---|
| 신뢰 | `.grant(`, `.revoke(`, `TrustStore::set`, `.forget(` |
| 권한·승인 응답 | `.respond_connection(`, `.respond(`(JSON-RPC `self.respond(&id` 제외), `ApprovalStore`, `UserAnswer` |
| 모드 | `change_mode_by_user`, `.set_mode(`, `SessionMode` |
| 약관 고지 | `TermsNoticeStore`, `.acknowledge(` |
| 출처 표시·되돌리기 | `clear_provenance`, `keep_provenance`, `dismiss_provenance`, `revert_batch`, `revert_session`, `revert_agent`, `AgentChangeProvenance` |

- **세션 event**: bridge·출력·앱 효과에서 오는 `SessionEvent`에는 모드·신뢰·권한 부여·출처·되돌리기 event가 없습니다. 출력 신호(`output.rs`)에는 권한·모드·신뢰 타입이 없습니다. 모드는 `change_mode_by_user` 하나로만 바뀝니다(XXIX.3.3).
- **프로젝트 파일 변조**: `stellar.yaml`에 `trust`·`agent.mode`·`permissions`·`terms_acknowledged`를, `.stellar/agent-state.yaml`에 모드·출처·신뢰를 써도 다음이 그대로입니다.
  - 신뢰 저장소 상태 `NotTrusted`와 시작 거부
  - 약관 고지 기록 bytes
  - 사용자가 신뢰한 뒤의 세션 모드(사용자 선택값)와 권한 0
- crate graph의 `FORBIDDEN_REACH`(mcp-core·stellar-mcp·ops·cli → agent-host)와 함께, 이 test는 호출 표지 자체가 없음을 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test immutable_paths` | 3 passed |
| `cargo test -p stellar-runtime --lib agent::tests::project_file_tampering_changes_no_trust_mode_or_notice_state` | 통과 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 프로젝트 파일 변조 fixture에서 상태 변경 0 | `project_file_tampering_changes_no_trust_mode_or_notice_state`(runtime) | 통과 |
| agent 경로의 출처 표시 정리·되돌리기 호출 0 | `agent_paths_cannot_write_immutable_state` | 통과(검사 파일 30개 초과) |
| 세션 event·출력 신호에 쓰기 수단 0 | `bridge_events_and_tool_calls_have_no_mode_or_permission_write` | 통과 |
| 검사기 위반 fixture 거부 | `checker_rejects_violation_fixture` | 통과(6줄 위반 검출, 주석·단위 테스트는 제외) |

## 차이(작업 지시와 다른 점)

- 출처 표시 정리·되돌리기 함수(XIX.6.8\~XIX.6.10)가 아직 없어 이름 표지로 검사합니다. 함수가 생기면 실제 이름을 표지 목록에 맞춰야 합니다.

## 남은 위험 / 한계

- 이름 표지 검사라 다른 이름으로 같은 동작을 하는 코드는 잡지 못합니다. 구조적 보장은 crate graph 도달 금지(에이전트 경로 → agent-host·runtime 0)가 맡습니다.
- 권한 저장소(`stellar_mcp::permission`)는 지금 에이전트 경로 crate(mcp-core) 안에 정의돼 있습니다. 응답 함수 정의는 있고 호출은 없습니다. 앱 crate로 옮기면(ADR-0093 결정 12) 정의도 에이전트 경로에서 빠집니다.
