# 작업 보고서: XXIX.6.6 출력 표시 격리 inventory test

- WBS: XXIX.6.6
- 작업명: 공급자 출력 parser가 agent-host 밖에 없고, 디코드 결과에서 명령·문서 변경·승인 응답으로 가는 경로가 없음을 검사
- 상태: `[x]` 완료
- 선행: XXIX.3.6, XVIII.5.1, XXIX.1.1
- 설계 기준: ADR-0005 §4.4·§8, ADR-0092 결정 8, ADR-0101 결정 6

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/tests/output_isolation.rs`(신규). 소스 도구는 `tests/support/scan.rs`(XXIX.6.5 커밋)를 씁니다.

## 구현 요약

네 검사와 위반 fixture입니다.

1. **터미널과 다른 경로의 parser 0, 터미널 출력은 제품 상태가 아님**
   - agent-host 밖(`crates`, `tools`, `apps`)의 제품 코드에 공급자 출력 형식 표지가 없습니다. 표지는 `"thread.started"`, `"item.completed"`, `"turn.completed"`, `"tool_use"`, `"tool_result"`, `stream-json`, `exec-jsonl`, `"agent_message"`, `SDKMessage`, `"stream_event"`입니다.
   - 터미널 crate(`crates/terminal`)는 공급자 이름(claude·codex·anthropic·openai)을 다루지 않습니다.
   - 터미널 crate의 의존 closure는 core-types뿐이고, 터미널 crate에 의존하는 crate가 0입니다. 그래서 터미널 출력이 명령·문서로 해석될 경로가 없습니다.
2. **디코더는 agent-host 안에만**: `AgentChatItem`, `decode_all`, `DecoderKind`, `DecodeOutput`, `stellar_agent_host::decoder`가 agent-host 밖 제품 코드에 0입니다.
3. **디코드 결과 → 명령·승인 경로 0**: agent-host의 함수·`impl From` 선언 중 디코드 결과 타입(`AgentChatItem`, `DecodeOutput`, `CliSignal`, `OutputSignal`, `DecodeFailure`)을 다루면서 명령·승인 타입(`CommandTransaction`, `Command`, `Resolution`, `UserAnswer`, `Answered`, `Outcome`, `Permission`, `LeaseId`, `ApprovalRequest`, `EffectEntry`, `BatchRecord`)을 쓰는 것이 0입니다. 채팅 조립의 `push_chat`은 item을 받기만 하고 아무것도 돌려주지 않습니다.
4. **agent-host 전이 도달 0**: manifest path 의존을 따라간 closure에 commands·document·history·storage·ops가 없습니다. 자기 dev 의존은 포함하고 그 뒤로는 normal·build만 따라가며, xtask `crate_graph::reach`와 같은 규칙입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test output_isolation` | 4 passed |
| `cargo xtask validate-crate-graph` | 종료 코드 0(같은 도달 금지의 정책 검사) |

| 검증 기준 | test | 결과 |
|---|---|---|
| 검사 1 | `no_provider_parser_outside_agent_host_and_terminal_output_is_not_product_state` | 0건 |
| 검사 2 | 같은 test(디코더 타입 이름) | 0건 |
| 검사 3 | `decode_results_have_no_path_to_commands_documents_or_approvals` | 0건 |
| 검사 4 | `agent_host_reaches_no_document_or_command_crate` | 0건 |
| 위반 fixture 거부 | `checkers_reject_violation_fixtures` | 통과. 공급자 표지·디코더 경로·터미널 공급자 이름 3건, 디코드 결과 → 승인·명령 함수 3건, 문서 crate 도달 1건을 검출하고 agent-host 안의 표지는 세지 않음 |

## 차이(작업 지시와 다른 점)

- "셸·FFI"는 `apps`와 `crates/ffi`를 검사 대상에 넣어 확인했습니다. macOS 셸 UI는 ADR-0098로 지워져 빈 창 껍데기만 있습니다.

## 남은 위험 / 한계

- 표지 검사는 알려진 형식 이름에 기대어 있습니다. 새 공급자 profile을 더하면 그 형식의 표지를 이 목록에 더해야 합니다.
- 검사 3은 선언 문자열 검사라 타입 별칭으로 우회하면 잡지 못합니다. 구조적 보장은 검사 4의 crate 도달 금지가 맡습니다.
