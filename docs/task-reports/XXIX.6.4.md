# 작업 보고서: XXIX.6.4 에이전트 관련 진단 금지 필드 검사

- WBS: XXIX.6.4
- 작업명: 에이전트 금지 필드 corpus 정의와 진단·`Debug`·사용자 데이터 누출 검사, 재시작 후 복원 0 검사
- 상태: `[~]` 부분 완료(corpus와 agent-host 검사 완료, XVI.8.2 scanner·XVI.8.6 inventory·macOS 앱 재시작 검사가 남음)
- 선행: XXIX.6.2(미착수), XVI.8.2(미착수), XVI.8.6(미착수), XIII.8.2
- 설계 기준: ADR-0005 §4.2 저장 범위·§4.4 기록, XVI.8, ADR-0101 결정 6

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/src/privacy.rs`(신규): `AGENT_FORBIDDEN_FIELDS`(아홉 필드와 표지 값), `scan`
- `crates/agent-host/tests/privacy_fields.rs`(신규)
- `crates/agent-host/Cargo.toml`, `Cargo.lock`: dev 의존 `stellar-diagnostics` 한 줄(허용 직접 의존 안)
- `crates/agent-host/src/lib.rs`: module 등록

## 구현 요약

- **corpus**: 사용자 입력 문장, 채팅 item, 원문 버퍼 내용, CLI 표준 오류, CLI 실행 파일 경로, CLI가 보고한 계정 식별 정보, 세션 결합 수단, IPC 상대 정보, 출처 색인입니다. 필드마다 이름(`agent_*`)과 고정 표지 값을 둡니다. `scan`은 표지 값이나 필드 이름이 보이면 그 필드를 냅니다.
- **누출 검사**: 표지 값을 세션 입력 전부에 넣습니다.
  - 넣는 곳: 사용자 문장, 대역 CLI의 stdout 텍스트·원문·stderr, 실행 파일 경로, 결합 token, 프로젝트 경로
  - 실제로 세션 메모리에 들어간 것을 먼저 확인합니다. 그 뒤 진단으로 남는 것을 모두 scan합니다.
  - scan 대상: 세션 종료 효과의 진단 기록, 상태별 `Debug`, 오류 문장, OS 사용자 데이터에 쓴 파일(약관 고지 기록)
- **schema inventory(XVI.8.6 입력)**: 진단 결과 key(`RESULT_KEYS`)와 interaction trace key(`TRACE_LINE_KEYS`)에 에이전트 필드 이름과 낱말(prompt·chat·stderr·account·binding·peer·provenance)이 없습니다. 세션 진단 기록 형식은 session id·duration·error code뿐입니다.
- **재시작 후 복원 0**: 세션·승인·채팅·출처를 읽어 오는 경로가 없습니다.
  - 파일을 쓰는 코드는 `start.rs`(약관 고지 기록)와 `process.rs`(세션 종료 때 지우는 실행 scratch)뿐입니다.
  - 세션·채팅·원문·승인·조립·출력 module에는 직렬화·저장·복원 함수가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test privacy_fields` | 3 passed |
| `cargo test -p stellar-agent-host --lib privacy` | 통과(corpus가 아홉 필드를 모두 잡음) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| privacy scan 0건 | `diagnostics_debug_and_user_data_hold_no_agent_fields` | 통과(엔진 출력 대상) |
| 재시작 후 세션 0·출처 표시 0 | `restart_restores_no_session_or_provenance` | 통과(코드 inventory) |
| schema inventory | `diagnostic_schemas_have_no_agent_fields` | 통과 |

## 차이(작업 지시와 다른 점)

- XVI.8.2 scanner와 XVI.8.6 schema inventory가 아직 없습니다. 그래서 "corpus에 추가"는 agent-host에 공개 corpus를 두고 그 scanner가 읽게 하는 형태로 했습니다.
- recovery snapshot·텔레메트리 경로 자체의 검사는 그 경로가 에이전트 상태를 받지 않는다는 코드 inventory로 대신했습니다(agent-host 상태는 project·storage crate에 닿지 않음).

## 남은 위험 / 한계

- XVI.8.2 scanner가 생기면 `AGENT_FORBIDDEN_FIELDS`를 corpus로 넣고, XVI.8.6 inventory가 이 이름을 schema에서 찾게 해야 합니다.
- macOS 앱 재시작 후 세션 0·출처 표시 0 확인은 XXIX.6.2 앱 E2E와 출처 색인(XIX.6.7) 뒤에 합니다.
- IPC 상대 정보는 local-ipc가 다루므로 그 crate의 진단 출력 검사는 local-ipc 작업에서 해야 합니다.
