# ADR-0092: 구독 CLI 호스트 코어를 stellar-agent-host에 둠

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `agent-host` lane이 정했습니다.

- ADR-0005 §4(L3 내장 에이전트)·§6(스레드)·§8(crate 배치)과 ADR-0023 결정 9를 구현 수준에서 구체화합니다. ADR-0005의 결정은 바꾸지 않습니다.
- 계획서 XXIX.2.1\~XXIX.2.13(UI와 FFI 제외)이 이 ADR을 구현합니다. 셸 FFI(XXIX.3.1)와 Swift UI(XXIX.4)는 범위가 아닙니다.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다.

## Context

- ADR-0005는 사용자가 직접 설치·로그인한 공급자 공식 CLI를 Stellar가 비대화형으로 실행하는 L3 경계와 불변식을 정했지만, crate 내부 구성과 실행 세부는 구현 작업에 맡겼습니다(§9).
- 이전에는 `crates/agent-host`가 없었고 crate graph 정책 표에 `stellar-agent-host` 허용 edge 행이 없었습니다. 전이 도달 금지 규칙(`FORBIDDEN_REACH`)만 미리 있었습니다.
- workspace는 외부 crate를 쓰지 않습니다. JSON reader·writer는 `stellar-cli`와 `stellar-mcp-core`에 의도적 사본으로만 있습니다.
- 공급자 약관 검토는 제품 소유자 몫이며 2026-11-23 현재 끝나지 않았습니다(ADR-0005 §4.1 약관 검토와 활성 목록).

## Decision

### 1. crate와 의존

- `stellar-agent-host`(`crates/agent-host`)를 workspace member로 추가합니다. 허용 직접 의존은 core-types, agent-protocol, local-ipc, diagnostics이고 실제 의존은 agent-protocol(권한 어휘 `Permission`)과 local-ipc(`TeamId`·`CodeRule`, 사용자 임시 디렉터리, 결합 token)뿐입니다.
- `tools/xtask/src/crate_graph.rs` `allowed_edges()`에 행을 더합니다. 기존 `FORBIDDEN_REACH`(document, commands, history, storage, ops)는 그대로 적용됩니다.
- 모델 제공자 API client, API key field, 자격 증명 저장, 모델 라우팅, 사용량 계량 타입을 두지 않고 `tests/inventory.rs`로 고정합니다.
- JSON은 `crates/mcp-core/src/json.rs` reader·writer의 의도적 사본(`src/json.rs`)을 씁니다. ops 타입 변환 두 개만 빼며, 본문 일치를 테스트로 검사합니다. 공용화는 XXIX.1.2에서 정합니다.

### 2. 테스트 대역(XXIX.2.2)

- 테스트 전용 실행 파일 `stellar-agent-host-test-cli`(`test-cli/main.rs`)를 둡니다. 테스트는 이 파일을 임시 디렉터리에 CLI 이름으로 복사하고, 옆에 `<exe>.scenario`를 둡니다. 대역은 자기 경로로 scenario를 찾습니다. 인자로 버전·연결 상태·세션 단계를 고르고, 출력 재생·조각 쓰기·대기·hang·종료 코드·하위 프로세스 띄우기·환경과 cwd 기록을 합니다.
- 대역은 환경 변수로 설정을 받지 않습니다. library를 쓰지 않아 framework를 link하지 않으므로 대역 스스로 환경 변수를 더하지 않습니다. 그래서 child 환경 측정이 정확합니다(CoreFoundation은 load 시 `__CF_USER_TEXT_ENCODING`을 스스로 더함). network API를 쓰지 않습니다.
- 대역은 공급자 서명을 가질 수 없습니다. 서명 확인은 `CodeVerifier` trait로 주입하고, 테스트는 결과를 정해 둔 구현을 넣습니다. 앱은 `AppleStaticCodeVerifier`를 씁니다. 대역은 앱 번들에 넣지 않습니다.

### 3. 실행 대상 확인, 연결 상태, 실행 전 확인(XXIX.2.3·XXIX.2.5·XXIX.2.7)

- **사용자 확인 기록**: 정규 경로, CLI 종류, 파일 지문(장치·inode·크기·수정 시각·mode)입니다.
  - 실행 직전에 경로를 다시 해석하고 지문을 다시 비교합니다. 바뀌면 다시 확인받습니다.
  - 파일은 일반 파일이고 실행 bit가 있어야 합니다. group·other가 쓸 수 없고, 소유자는 root나 현재 사용자여야 합니다.
- **코드 서명**: `SecStaticCodeCreateWithPath`와 `SecStaticCodeCheckValidity`로 확인합니다.
  - flag는 `kSecCSStrictValidate | kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSNoNetworkAccess`입니다.
  - 요구 사항은 profile Team ID에 대한 `anchor apple generic and certificate leaf[subject.OU] = "<team>"`(TN3127)입니다.
  - 서명이 없는 배포 형태는 실행하지 않습니다.
- **연결 상태**: profile의 공식 상태 명령만 실행합니다(Claude Code `claude auth status`: 로그인하면 종료 코드 0, 아니면 1).
  - 종료 코드만 쓰고 출력은 보관하지 않습니다(상한 0). 그래서 계정 식별 정보가 Stellar에 들어오지 않습니다.
  - 판정할 수 없으면 사용 불가입니다. 판정은 worker 스레드(`spawn_probe`)나 실행 전 확인 안에서 하고 결과를 캐시하지 않습니다.
- **실행 전 확인 순서**: profile 활성, 신원, 버전 범위, 연결 상태 순서입니다. 하나라도 실패하면 세션을 실행하지 않습니다.
  - 신원 확인이 끝나기 전에는 CLI를 한 번도 실행하지 않습니다.
  - 세션 실행 함수는 통과 증표 `PreflightPass`로만 부를 수 있습니다.

### 4. launch profile(XXIX.2.6)

- JSON fixture(`crates/agent-host/profiles/*.json`, 형식 `stellar-launch-profile/1`)는 다음을 담습니다.
  - 실행 인자: 버전·상태·세션, 자리 표시자는 `{mcp_config}`만
  - 입력 방식, decoder profile id와 버전
  - 환경 변수 **이름** 목록, timeout
  - 약관·공식 문서 검토 일자와 결과
- 모르는 key, 값이 붙은 환경 항목, 약관 검토 `allowed`가 아닌 활성 profile, Team ID 없는 활성 profile은 load에서 거부합니다.
- 공식 profile 두 개(`claude-code-2.1`, `codex-exec-0`)는 약관 검토 `pending`이고 Team ID가 null이라 비활성입니다. 실제 CLI는 실행되지 않습니다. 활성화는 제품 소유자의 약관 검토와 공급자 서명 Team ID 확인 뒤 profile을 갱신하는 일입니다.
- 불변식 1(문서 변경은 Stellar MCP tool로만)은 Claude Code profile의 공식 설정으로 지킵니다.
  - `--tools ""`: 내장 tool 전부 끔
  - `--strict-mcp-config --mcp-config <scratch 파일>`: Stellar MCP만
  - `--allowedTools mcp__stellar`
- 불변식 2(흔적 없음)는 `--no-session-persistence`와 아래 결정 6의 scratch cwd로 지킵니다. Codex profile의 MCP 연결 방법은 확인하지 못해 비활성 사유로 기록합니다.

### 5. 허용 목록 환경(ADR-0023 결정 9)

- child 환경은 빈 환경에서 시작해 profile 목록 이름만 앱 환경에서 그대로 전달합니다. Stellar가 더하는 변수는 0입니다.
- 결합 token(XIX.1.8)은 CLI 환경이 아니라 MCP 설정 파일의 서버 `env`로만 넘깁니다.

### 6. 실행기(XXIX.2.4)

- PTY 없이 파이프로 실행하고, 새 process group(`process_group(0)`)으로 만듭니다.
- cwd는 세션마다 OS 사용자 임시 디렉터리(`_CS_DARWIN_USER_TEMP_DIR`) 아래의 mode 0700 scratch `work/`입니다. MCP 설정은 같은 scratch에 mode 0600으로 둡니다. 세션이 끝나면 scratch 전체를 지웁니다.
- 종료와 timeout은 다음과 같습니다.
  - 중지와 시작 timeout(첫 stdout byte까지 `startup_ms`)이면 group에 SIGTERM을 보내고, 유예(`terminate_grace_ms`) 뒤 SIGKILL을 보냅니다.
  - CLI가 끝나면 group에 SIGKILL을 보내 하위 프로세스를 남기지 않습니다.
  - 기본값은 version 5 s, status 10 s, startup 60 s, grace 3 s입니다. startup 60 s는 Claude Code `--mcp-config`가 `-p`에서 MCP 서버 연결을 최대 `MCP_TIMEOUT` 30 s(문서 기본값)까지 기다리는 시간의 두 배입니다.
- reader 스레드가 stdout·stderr를 64 KiB 단위로 바로 출력 수신처에 넘기므로 UI가 느려도 수신이 멈추지 않습니다.
- stdin은 전용 writer 스레드가 쓰고, 파이프 fd에 `F_SETNOSIGPIPE`를 켭니다. 프로세스 종료를 기다릴 때는 stdin을 먼저 닫습니다.
- 입력 방식은 둘입니다.
  - Claude Code는 `--input-format stream-json` user message 줄로 여러 turn을 보냅니다.
  - Codex는 첫 메시지를 stdin에 쓰고 닫습니다(한 turn).

### 7. AgentChatItem(XXIX.2.10)

- closed enum variant는 다음과 같습니다.
  - `UserPrompt`, `AssistantText`
  - `ToolActivity { call_id, tool, phase }`: 이름과 단계만
  - `SessionStarted { model }`: CLI가 보고한 모델 이름
  - `TurnCompleted { failed }`
  - `CliError { kind: UsageLimit | Authentication | Other, text }`
  - `DecodeStopped { code }`
- 문서 효과 variant는 없습니다. 문서 효과 표시는 승인·출처 기록(XXIX.3.6)이 만듭니다.
- 상한은 item 텍스트 16 KiB, label 256 B, 세션 item 10,000개입니다. 넘으면 오래된 것부터 버리고 개수를 셉니다.

### 8. 표시 전용 출력 디코더(XXIX.2.9·XXIX.2.11)

- `claude-stream-json@1`(Agent SDK `SDKMessage` JSON Lines)과 `codex-exec-jsonl@1`(`codex exec --json` event)을 둡니다. 버전이 정확히 같을 때만 resolve합니다.
- 줄 단위로 자르며 입력 경계를 이어 붙입니다. 결과는 조각 크기와 무관하게 같습니다.
- 실패 판정(fail closed)은 `invalid-utf8`, `line-too-long`(1 MiB), `invalid-json`, `not-object`, `missing-type`, `shape`입니다. 실패하면 판정 하나를 내고 닫습니다.
- 공식 문서에 있지만 보이지 않는 type과 모르는 type은 item 없이 건너뜁니다.
- 사용량·비용 field는 읽지 않습니다.
- Claude `SDKAssistantMessageError` 값은 이렇게 나눕니다.
  - `rate_limit`·`billing_error`: 사용 한도
  - `authentication_failed`·`oauth_org_not_allowed`·`account_on_hold`·`cloud_credential_error`: 인증 오류

### 9. 원문 버퍼(XXIX.2.12)

- 세션당 8 MiB 상한입니다. 넘으면 오래된 byte부터 버리고 잘린 byte 수를 표시합니다.
- 받은 byte를 그대로 저장합니다. 표시 텍스트는 C0(줄바꿈·탭 제외)·DEL·C1을 Control Pictures나 U+FFFD로 바꿉니다.
- 직렬화 수단이 없고 `Debug`는 크기만 씁니다.

### 10. 디코드 실패의 세션 처리(XXIX.2.13)

- 디코드 실패는 채팅 디코드만 닫습니다. `DecodeStopped` item 하나를 넣고, 이후 stdout은 원문 버퍼에만 넣습니다.
- 세션 상태 기계에는 `DecodeFailed` 신호가 가지만 효과는 0입니다. 상태·권한·lease·대기 요청·적용 변경은 그대로입니다.
- 세션을 끝내지 않습니다. 세션을 끝내면 권한이 줄어들어 "디코드 실패는 권한을 넓히거나 좁히는 수단이 아님"(ADR-0005 §4.4)에 어긋나기 때문입니다. 끝내기는 사용자 중지와 다른 종료 조건이 합니다.

### 11. AgentSession 상태 기계(XXIX.2.8)

- 상태는 `Starting → Running ⇄ Reconnecting → Ending → Ended`입니다. 단조 시각 입력으로 결정적입니다.
- 재연결 유예 기본값은 15 s입니다. 유예가 지난 뒤 도착한 재결합은 받지 않고 유예 초과로 끝냅니다. bridge 재시작과 재결합(handshake·peer 인증)은 수 초 안이고, 사용자가 기다리는 lease 점유 시간은 짧게 둡니다.
- lease·권한은 세션이 소유하고 모드 상한(읽기 전용, 검토 후 적용, 승인 범위 내 바로 적용) 안에서만 받습니다.
- 종료 trigger는 사용자 중지, CLI 프로세스 종료, 유예 초과, 신뢰 철회, lease 충돌, 사용자 lease 해제, CLI 보고 사용 한도·인증 오류입니다.
- 종료 trigger가 오면 XIX.6.4 절차를 효과 목록으로 냅니다. 적용 중 transaction이 있으면 끝날 때까지 기다린 뒤 순서대로 냅니다.
  1. 중단됨 history entry
  2. lease 해제, 권한 폐기
  3. 요청 거부, 제안 폐기, 승인 화면 닫기
  4. 결합 종료, 흔적 정리
  5. 진단(id·duration·code)
- 적용 batch 기록은 지우지 않습니다.
- history·document에 도달하지 않으므로 효과 실행은 runtime이 합니다(XXIX.3.1).
- 끝난 세션은 모든 event를 거부합니다. 재개·재시작·재전송 API는 없습니다.

## Alternatives

- **테스트 대역을 shell script로 둠**: `/bin/sh`가 `PWD`·`SHLVL`·`_`를 스스로 더해 child 환경 부분집합 검사를 할 수 없고, script는 서명 확인 경로도 다르게 탑니다. 탈락.
- **대역 실행 파일이 library를 씀**: CoreFoundation link로 `__CF_USER_TEXT_ENCODING`이 더해져 같은 이유로 탈락.
- **실행 파일 내용 hash를 확인 기록에 저장**: 외부 hash crate가 없고, 코드 서명 strict 검증이 이미 code directory hash로 내용을 확인합니다. 지문(inode·크기·시각)으로 바뀜만 감지합니다.
- **디코드 실패 시 세션 중지**: 권한 축소가 되어 ADR-0005 §4.4와 충돌합니다. 탈락.
- **모르는 SDKMessage type을 실패로 판정**: 공식 문서 union에 type이 자주 늘어 지원 버전 안에서도 정상 세션이 닫힙니다. 잘못된 내용을 보이지 않는 건너뜀이 닫힘 원칙과 충돌하지 않으므로 건너뜁니다.
- **연결 상태 출력(JSON)을 파싱**: 계정 식별 정보가 메모리에 들어옵니다. 종료 코드가 공식 문서로 정의되어 있으므로 탈락.

## Consequences

- 엔진 수준에서 내장 세션 실행 경로 전체를 Rust 테스트로 검증할 수 있습니다. 네트워크와 실제 계정은 쓰지 않습니다.
- 공식 profile이 비활성이므로 제품에서 실제 CLI는 아직 실행되지 않습니다. 약관 검토와 Team ID 확인이 릴리스 전 남은 일입니다.
- 효과 목록을 실행하는 runtime 연결, 승인 저장, history entry 확정은 XXIX.3.x가 합니다.
- JSON 사본이 셋이 됩니다. 사본 일치 테스트로 표류를 막습니다.

## Compatibility

- 파일 포맷, 공개 FFI, 보호 경로에 영향이 없습니다. 새 workspace member와 crate graph 정책 행만 더합니다.
- launch profile 형식 `stellar-launch-profile/1`은 앱 내부 fixture이며 프로젝트 파일이 아닙니다.

## Performance

- 성능 예산 파일은 바꾸지 않습니다.
- 측정값(개발 Mac, 다른 작업과 동시 실행)은 다음과 같습니다.
  - 테스트 대역 32 MiB 출력을 UI 미소비 상태로 수신: 0.08\~1.0 s, 원문 상한 유지
  - 8 MiB 상한 버퍼에 32 MiB 추가와 표시 텍스트 준비: XXIX.2.12 보고서
- 디코드·원문 버퍼링은 reader 스레드에서 하고 UI 스레드에는 증분 조회만 남습니다.

## Security

- network API 0, 자격 증명 저장 위치 접근 0, 계정 식별 문자열 보관 0, 환경 허용 목록, scratch 0700·설정 0600을 지킵니다.
- 서명 확인은 network 없이 하고, 실패하면 닫힙니다.
- 결합 token은 MCP 설정 파일에만 있고 세션 종료와 함께 지워집니다.
- 남은 위험: 같은 사용자 권한의 다른 프로세스는 scratch를 읽을 수 있습니다(신뢰 경계는 OS 사용자, 아키텍처 §37). CLI가 자기 설정 디렉터리에 쓰는 상태 파일(실행 횟수 등)은 CLI 동작이며, 릴리스 전 수동 검증(XXIX.6.8)에서 확인합니다.

## Migration

- 기존 데이터가 없습니다. 기존 코드는 바꾸지 않습니다.
