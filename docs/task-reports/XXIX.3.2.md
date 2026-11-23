# 작업 보고서: XXIX.3.2 승인 요청을 ProjectIdentity 단위로 코어에 보관

- WBS: XXIX.3.2
- 작업명: 사용자 응답을 기다리는 승인 요청의 저장과 수명 판정(첫 응답만 유효, 창 닫힘·기한 초과·세션 종료 거부)
- 상태: `[~]` 부분 완료(엔진 부분 완료, 앱 연결은 XXIX.3.1·XXIX.3.4)
- 선행: XXIX.3.1(미착수, 셸 FFI), XIX.4.1, XIX.3.4
- 설계 기준: ADR-0005 §3·§4.5 승인 요청 저장, ADR-0093, ADR-0101 결정 1

## 필수 실행 환경

- `DEV-MAC`. 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/approval.rs`(신규): `ProjectKey`, `ApprovalSubject`, `ApprovalEvidence`·`EvidenceParts`, `ApprovalRequest`, `ApprovalStore`, `Resolution`, `Answered`, `Denial`, `Outcome`
- `crates/agent-host/src/lib.rs`: module 등록
- `crates/agent-host/tests/approval_store.rs`(신규)

## 구현 요약

- **저장 단위**: `ProjectKey`(project id + canonical target, IV.2.3)마다 대기 목록을 둡니다. 앱 runtime이 `ProjectIdentity`에서 key를 만듭니다(`stellar_runtime::agent::project_key`, XXIX.3.5 커밋). `Debug`는 경로를 쓰지 않습니다.
- **대상**: 범위 승인(`Scope { targets, externals }`, XIX.4.1), batch 승인(`Batch { proposal }`, XIX.5.3), 권한 개별 확인(`Permission`, XIX.3.4).
- **수명 판정은 이 모듈 하나**입니다. 권한 저장소(`stellar_mcp::permission`)와 lease 요청 관리자는 `Resolution`만 받습니다.
  - 첫 응답만 유효합니다. 다시 응답하면 `Answered::Ignored { prior }`입니다.
  - 응답 기한 초과(`expire`), 기한 뒤 도착한 승인(`answer`가 기한 초과 거부로 확정), 창 닫힘(`close_window`), 세션 종료(`end_session`, `apply_session_effects`가 상태 기계 효과 `RejectPendingRequests`를 받아 실행)는 모두 거부입니다.
  - 닫힌 창과 끝난 세션으로는 새 요청을 열지 않습니다. 이미 확정된 요청은 이후 일괄 거부의 영향을 받지 않습니다.
- **순서**: 요청 id는 저장소 전체에서 단조 증가합니다. 대기 목록과 일괄 거부 결과는 연 순서입니다.
- **승인 근거**: batch 요청은 `ApprovalEvidence::parts`가 semantic diff·validation·렌더 프리뷰를 모두 갖췄다고 할 때만 열립니다. 근거 타입은 ops·renderer 결과를 담는 앱 runtime이 정합니다(agent-host는 ops에 닿지 않음).
- **응답 기한 기본값** `DEFAULT_RESPONSE_DEADLINE_MS` = 5분입니다.
  - 근거: 사용자가 semantic diff와 렌더 프리뷰를 읽고 판단할 시간입니다. 범위 승인 전에는 lease를 잡지 않으므로(ADR-0093 결정 7) 기다리는 동안 다른 편집을 막지 않습니다.
  - XIX.9.5가 MCP 대기 상한을 실측으로 정하면 그 값으로 바꿉니다(잠정값).
- **기록 상한**: 확정 결과는 프로젝트마다 최근 65,536개입니다. 잊은 요청에 온 응답은 `UnknownRequest`이며 효과가 없습니다.
- 메모리 전용이며 파일·직렬화 경로가 없습니다. 응답 함수는 앱 사용자 UI만 부릅니다(XXIX.6.5 inventory).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test approval_store` | 6 passed |
| `cargo test -p stellar-agent-host --lib approval` | 통과(`Debug` 경로 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 중복 응답 무시 | `first_answer_wins_and_duplicates_are_ignored` | 통과(승인 뒤 거부·승인·만료·창 닫힘·세션 종료 모두 결과 불변) |
| 창 닫힘·기한 초과·세션 종료 거부 fixture | `deadline_window_close_and_session_end_deny_by_default` | 통과(기한 뒤 승인도 거부, 닫힌 창·끝난 세션 새 요청 거부) |
| 동시 요청 순서 유지 | `concurrent_requests_keep_open_order`, `bulk_denials_are_reported_in_open_order` | 통과(8 스레드 × 50 요청, id 순서와 스레드별 순서 유지) |
| 근거 갖춤 | `deadline_must_be_in_future_and_batch_needs_full_evidence` | 통과 |
| 프로젝트 분리 | `projects_are_separate_and_answers_need_the_right_project` | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XXIX.3.1(FFI)은 사용자 방향(2026-11-23, 셸 FFI 추가 금지)에 따라 하지 않았습니다. 저장소와 판정은 FFI 없이 Rust test로 검증했습니다.

## 남은 위험 / 한계

- MCP proposal·`BeginAgentEdit` 처리에서 요청을 열고 확정 결과를 권한 저장소와 lease 관리자에 넘기는 앱 연결이 남았습니다(XXIX.3.1·XXIX.3.4). 지금 권한 저장소는 mcp-core에 있어 runtime이 닿지 않습니다(ADR-0093 결정 12의 이동 뒤 연결).
- 실제 근거(semantic diff·validation·렌더 프리뷰)를 만드는 runtime 구현이 없습니다. 렌더 프리뷰 생산 경로가 생길 때 `ApprovalEvidence`를 구현합니다.
- 응답 기한 5분은 잠정값입니다(XIX.9.5).
