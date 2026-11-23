# 작업 보고서: XXIX.2.10 채팅 보기 모델 AgentChatItem

- WBS: XXIX.2.10
- 작업명: `stellar-agent-host`의 closed enum `AgentChatItem`과 표시 텍스트·item 수 상한
- 상태: `[x]` 완료
- 선행: XXIX.2.1
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/chat.rs`(신규): `AgentChatItem`, `DisplayText`, `ToolPhase`, `CliErrorKind`, `ChatLog`
- `crates/agent-host/tests/chat_items.rs`, `crates/agent-host/tests/fixtures/chat-items.golden`

## 구현 요약

- **variant**(ADR-0005 §4.4 범위: 채팅 보기 기본, 승인·출처 모델이 요구하는 범위를 넘지 않음):
  - `UserPrompt { text }`: 사용자 문장 그대로
  - `AssistantText { text }`
  - `ToolActivity { call_id, tool, phase: Started | Succeeded | Failed }`: CLI가 보고한 tool 이름과 단계만, 입력·결과 내용 없음
  - `SessionStarted { model }`: CLI가 보고한 모델 이름
  - `TurnCompleted { failed }`
  - `CliError { kind: UsageLimit | Authentication | Other, text }`
  - `DecodeStopped { code }`: XXIX.2.13
- 문서 효과(변경 노드·속성·승인 결과) variant는 두지 않습니다. 문서 효과 표시는 승인·출처 기록(XXIX.3.6)이 만들고, 채팅 보기는 그 근거가 아닙니다.
- **금지**: CommandTransaction, document·commands·history·storage·ops 타입, 승인 응답을 담지 않습니다. 그 타입으로 바꾸는 함수(`From`·`into_*`)도 없습니다.
- **상한**:
  - item 표시 텍스트 16 KiB(UTF-8 문자 경계에서 자르고 `truncated` 표시)
  - label(tool·call id·model) 256 B
  - 세션 item 10,000개(넘으면 오래된 것부터 버리고 수를 셈)
  - `ChatLog::since`로 증분 조회. `Debug`는 개수만 씁니다.
- golden 직렬화(`to_golden`)는 테스트 기준인 결정론적 한 줄 JSON이며 저장 형식이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| variant별 golden 직렬화(13 사례, 7 variant 전부, 상한 사례 포함) | 통과 |
| 금지 타입·변환 함수 inventory 0(`chat_and_decoder_hold_no_document_command_or_approval_types`) + crate graph 전이 도달 금지 | 통과 |
| 텍스트 상한 문자 경계, item 수 상한·증분 조회, `Debug` 내용 0(단위 test) | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- variant는 작업이 정하도록 되어 있었고 ADR-0092 결정 7에 기록했습니다.

## 남은 위험 / 한계

- 서식(markdown 등) 범위는 XXIX.4.3에서 정합니다. 표시 텍스트는 원문 그대로이고 제어 문자 무력화는 화면 단계에서 합니다(원문 보기 함수 `neutralize_controls` 재사용 가능).
