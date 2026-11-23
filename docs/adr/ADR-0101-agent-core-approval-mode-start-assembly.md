# ADR-0101: 내장 에이전트 코어에 승인 요청 저장·모드 상한·시작 확인·채팅 조립을 둔다

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `agent-core-app` 작업이 정했습니다.

- ADR-0005 §3(변경 불가 경로)·§4.2·§4.4·§4.5·§8과 아키텍처 31.3\~31.5·31.9를 구현 수준에서 구체화합니다. ADR-0005의 결정은 바꾸지 않습니다.
- 대상 행: XXIX.3.2, XXIX.3.3, XXIX.3.5, XXIX.3.6, XXIX.6.1, XXIX.6.4\~XXIX.6.6의 엔진 부분입니다. 셸 FFI(XXIX.3.1)와 Swift UI(XXIX.4)는 범위가 아닙니다.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다.

## Context

- ADR-0092가 `stellar-agent-host`에 구독 CLI 실행 경로와 AgentSession 상태 기계를 두었습니다. 세션 효과(요청 거부, 승인 화면 닫기 등)를 실행하는 앱 쪽 저장소와 시작 확인은 XXIX.3.x에 남아 있었습니다.
- ADR-0093이 권한 저장(`stellar_mcp::permission`)과 lease 판정을 두었습니다. 권한 저장은 권한별 확인 상태만 다루고, 사용자 응답을 기다리는 요청의 수명(응답 기한, 창 닫힘, 세션 종료)과 순서는 다루지 않습니다.
- crate graph: agent-host는 core-types·agent-protocol·local-ipc·diagnostics만 쓸 수 있고 project·ops·document에 닿지 않습니다. mcp-core는 agent-host·runtime에 닿을 수 없습니다. runtime은 ADR-0005 §8 표에서 agent-host를 허용 의존으로 갖지만, 정책 표(`tools/xtask/src/crate_graph.rs`)에는 아직 그 edge가 없었습니다.
- 프로젝트 신뢰 판정은 `stellar_project::capability::capability_level`(XVI.7.1 표) 하나입니다. 출처 색인(XIX.6.7)과 금지 필드 scanner(XVI.8.2)는 아직 없습니다.

## Decision

### 1. 승인 요청 저장(XXIX.3.2, `stellar_agent_host::approval`)

- `ApprovalStore<E>`가 사용자 응답을 기다리는 요청을 프로젝트 identity(`ProjectKey`: project id + canonical target) 단위로 메모리에 둡니다. 요청 수명 판정은 이 모듈 하나만 합니다. 권한 저장소와 lease 요청 관리자는 확정 결과(`Resolution`)만 받습니다.
- 대상(`ApprovalSubject`)은 범위 승인(`BeginAgentEdit`), batch 승인, 권한 개별 확인 셋입니다.
- **첫 응답만 유효**: 확정된 요청에 다시 응답하면 `Answered::Ignored { prior }`이고 결과가 바뀌지 않습니다.
- **기본 거부**: 응답 기한이 지나면(기한 뒤 도착한 승인 포함), 요청을 연 창(`WindowSessionId`)이 닫히면, 세션이 끝나면 거부로 확정합니다. 닫힌 창과 끝난 세션으로는 새 요청을 열지 않습니다. 세션 종료는 상태 기계 효과 `RejectPendingRequests`를 `apply_session_effects`가 받아 실행합니다.
- **순서**: 요청 id는 저장소 전체에서 단조 증가하고, 대기 목록과 일괄 거부 결과는 연 순서입니다.
- **승인 근거**: batch 승인 요청은 semantic diff, validation, 렌더 프리뷰를 모두 갖춰야 열립니다(XIX.5.3). 근거 타입은 ops·renderer 결과를 담는 앱 runtime이 정하고, 저장소는 `ApprovalEvidence::parts`로 갖춤 여부만 봅니다. 그래서 agent-host가 ops에 닿지 않습니다.
- 응답 기한 기본값은 5분입니다. 사용자가 semantic diff·렌더 프리뷰를 읽고 판단할 시간이고, 범위 승인 전에는 lease를 잡지 않으므로(ADR-0093 결정 7) 기다리는 동안 다른 편집을 막지 않습니다. XIX.9.5가 MCP 대기 상한을 실측으로 정하면 그 값을 따릅니다.
- 확정 결과 기록은 프로젝트마다 최근 65,536개까지 둡니다. 잊은 요청에 온 응답은 `UnknownRequest`이며 효과가 없습니다.
- 응답(`answer`)은 앱 사용자 UI만 부릅니다. 파일·직렬화 경로가 없고 `Debug`는 개수만 씁니다.

### 2. 모드 상한과 내장 세션 카탈로그(XXIX.3.3, `stellar_agent_host::mode`)

| 권한 | 읽기 전용 | 검토 후 적용(기본) | 승인 범위 내 바로 적용 |
|---|---|---|---|
| Read | 세션 | 세션 | 세션 |
| Propose | 상한 밖 | 세션 | 세션 |
| Apply | 상한 밖 | 상한 밖 | 세션 |
| GitCommit·GitPush | 상한 밖 | 실행 한 번 개별 확인 | 실행 한 번 개별 확인 |
| Terminal·FileDelete | 제외 | 제외 | 제외 |

- 표는 `grant_route(mode, permission)` 하나입니다. 세션 상태 기계의 권한 받기, `tools/list`(`builtin_tools`), `tools/call` 판정(`builtin_tool`)이 모두 이 함수를 씁니다.
- 읽기 전용은 상태를 바꾸는 tool을 하나도 노출하지 않습니다. 그래서 Git 쓰기도 상한 밖입니다. 검토 후 적용은 apply tool을 노출하지 않습니다. batch는 사용자의 batch 승인으로 앱이 적용합니다.
- 내장 세션 목록은 attached backend와 문서·Git namespace에서 시작합니다. 터미널 namespace는 켜지 않습니다. 카탈로그에 없는 이름은 `Unknown`이고, 카탈로그에는 모드·권한·신뢰를 쓰는 tool이 없습니다.
- 모드 변경은 `AgentSession::change_mode_by_user` 하나입니다. 낮추면 새 상한 밖 권한만 `RevokePermissions`로 폐기하고 lease·요청·적용 변경은 그대로 둡니다. 올려도 권한은 생기지 않습니다. 끝난 세션은 바꾸지 않습니다.

### 3. 시작 확인, 약관 고지, 신뢰 철회(XXIX.3.5, `stellar_agent_host::start`, `stellar_runtime::agent`)

- 신뢰 판정은 runtime의 `mcp_capability(trust)`가 `capability_level(ProjectCapability::Mcp, trust)` 결과를 `McpCapability`로 넘깁니다. `Allowed`만 허용이고 제한·금지는 거부입니다. agent-host는 판정하지 않고 결과만 씁니다.
- runtime의 `prepare_agent_start`·`confirm_agent_start`는 신뢰 상태를 값으로 받지 않고 같은 `ProjectIdentity`로 신뢰 저장소 조회 함수에서 읽습니다. 확인 단계는 확인 데이터의 대상 프로젝트가 그 identity와 다르면 `ProjectMismatch`로 시작하지 않습니다. 다른 프로젝트의 신뢰 상태로 확인이 통과하는 경로를 막습니다.
- `prepare_start`는 신뢰와 profile 활성을 확인하고 시작 확인 데이터(`StartConfirmation`: 대상 프로젝트, 창, profile, CLI 이름, 모드, 고지 필요 여부)를 만듭니다. `confirm_start`는 사용자가 확인한 순간 신뢰·profile·고지를 다시 확인합니다. 통과하면 세션을 만들고 그 확인을 `FirstConnectionApproval`로 기록합니다. 이 기록은 결합된 bridge의 첫 연결 승인이며 Read 밖 권한을 주지 않습니다.
- **약관 고지 기록**: 앱이 정한 OS 사용자 데이터 디렉터리 아래 `agent/terms-notices`에 둡니다.
  - 첫 줄은 `stellar-agent-terms-notice 1`이고, CLI마다 `<cli> <약관 검토 일자> <검토 결과>` 한 줄입니다.
  - 기록과 profile의 검토 일자·결과가 다르면 다시 고지합니다. 처음 쓰는 CLI도 고지합니다.
  - 디렉터리 0700, 파일 0600이며, symlink를 거부하고 임시 파일을 쓴 뒤 rename합니다. 형식이 틀린 파일은 덮어쓰지 않고 오류로 닫습니다(시작 0).
- `on_trust_change`는 capability가 허용이 아니게 되면 즉시 `TrustRevoked`로 XIX.6.4 종료 절차를 시작합니다.
- **crate edge**: runtime → agent-host edge를 지금 정책 표에 더합니다. ADR-0005 §8이 이미 허용한 edge입니다. XXIX.3.1의 나머지(agent-protocol·에이전트 편집 구현 crate edge, FFI 함수)는 그 작업이 합니다. FFI에는 아무것도 더하지 않습니다.

### 4. 채팅 보기 조립(XXIX.3.6, `stellar_agent_host::assembly`)

- `ChatAssembly`가 세션 하나의 목록(`TranscriptEntry`)을 도착 순서대로 만듭니다. CLI 보고(`Chat(AgentChatItem)`)와 문서 효과(`Effect`)는 서로의 근거가 아닙니다.
- 문서 효과는 승인 기록(`Resolution`)과 출처 기록 요약(`BatchRecord`: XIX.6.7 출처 색인의 batch 요약)에서만 만듭니다. 받는 규칙은 다음과 같습니다.
  - 다른 세션의 기록은 받지 않습니다.
  - 요청 확정은 먼저 열린 같은 대상의 요청에만 붙고, 한 번만 받습니다.
  - batch 적용은 순번이 늘어날 때만 받습니다. 제안이 있으면 그 batch 승인이 `approved`여야 하고, 승인 하나는 적용 표시 하나에만 씁니다(같은 제안으로 두 번째 batch를 표시하면 `ProposalAlreadyApplied`). 제안 없는 적용은 승인 범위 내 바로 적용 모드에서만 받습니다.
- `verify(opened, resolutions, batches)`는 표시된 효과가 기록과 정확히 같은지 다시 확인합니다.
- CLI 보고 item은 10,000개 상한(오래된 것부터 버림)입니다. 문서 효과는 검토 대기 출처 표시의 근거라 버리지 않습니다.
- 세션 메모리에만 있고 직렬화·파일 경로가 없습니다.

### 5. E2E 시나리오 공통 fixture(XXIX.6.1, `stellar_agent_host::scenario`)

- 형식은 `stellar-agent-e2e-scenario/1` JSON이고 위치는 `crates/agent-host/tests/fixtures/e2e/`입니다. 테스트 대역 `[session]` 명령, 모드, 시각이 붙은 닫힌 단계 목록, `check` 확인을 담습니다.
- 같은 정의를 두 실행기가 씁니다.
  - headless 실행기(agent-host test): 실제 테스트 대역 프로세스·AgentSession·승인 저장소·채팅 조립으로 세션 수준 단계를 실행합니다. bridge 결합은 단계로 흉내 냅니다.
  - 앱 실행기(XXIX.6.2): 문서·history가 필요한 `app` 단계(Undo, 되돌리기, 유지)와 `check.app` 확인을 실행합니다. 이 이름은 닫힌 목록입니다.
- 네 시나리오는 계획서 순서와 같습니다: 완료·Undo, 중지 뒤 되돌리기, 재연결 유예, 디코드 실패.

### 6. 금지 필드 corpus와 inventory(XXIX.6.4\~XXIX.6.6)

- `stellar_agent_host::privacy::AGENT_FORBIDDEN_FIELDS`가 계획서의 아홉 필드와 고정 표지 값을 둡니다. XVI.8.2 scanner와 XVI.8.6 schema inventory가 생기면 이 목록을 corpus로 함께 읽습니다.
- 지금 test가 보는 것은 다음과 같습니다.
  - 표지를 세션 입력 전부에 넣었을 때 진단 기록, `Debug`, 오류 문장, OS 사용자 데이터 파일에서 scan이 0인지
  - 진단 schema key에 에이전트 필드가 없는지
  - 파일을 쓰는 코드가 고지 기록과 실행 scratch뿐인지(재시작 후 세션·출처 0)
- ADR-0005 §3 변경 불가 경로 inventory는 에이전트 경로(mcp-core, stellar-mcp, agent-protocol, local-ipc, stellar-cli)의 제품 코드에 신뢰·권한 응답·모드·고지·출처 정리·되돌리기 쓰기 표지가 없음을 봅니다. 프로젝트 파일 변조 fixture는 runtime test가 맡습니다.
- 출력 표시 격리 inventory는 네 검사를 합니다. 각 검사에 위반 fixture가 있습니다.
  1. agent-host 밖의 공급자 parser 표지·디코더 타입 사용 0, 터미널 crate의 공급자 특별 처리 0, 터미널 crate 의존 closure가 core-types뿐이고 그 crate에 의존하는 crate 0
  2. 디코더 타입은 agent-host 안에만 있음
  3. 디코드 결과 타입에서 명령·승인 타입으로 가는 함수 선언 0
  4. agent-host 의존 closure에 문서·명령 crate 0

## Alternatives

- **승인 요청 수명을 권한 저장소(mcp-core)에 더 둠**: 권한 저장소는 권한 상태이고 batch·범위 요청을 다루지 않습니다. 앱 runtime은 mcp-core에 닿을 수 없습니다(ADR-0005 §8). 두 곳에서 기한·창 닫힘을 판정하면 이중 경로가 됩니다. 탈락.
- **승인 저장소를 runtime에 두고 근거 타입을 구체 타입으로 고정**: 세션 효과와 채팅 조립이 agent-host에 있어 저장소도 같은 crate가 자연스럽습니다. runtime에 두면 agent-host test·E2E fixture가 저장소를 쓸 수 없습니다. 근거는 trait로 갖춤 여부만 보면 충분합니다. 탈락.
- **신뢰 여부를 bool로 받음**: 호출자마다 capability 표를 다르게 읽을 수 있습니다. 표 판정 결과 타입만 받습니다.
- **모드를 세션 시작 뒤 바꾸지 못하게 함**: ADR-0005 §4.5는 사용자 UI에서 바꾸는 설정이라고 정합니다. 낮추기는 즉시 권한을 줄이는 안전한 동작이라 허용합니다.
- **CLI tool 성공 보고를 문서 효과로 표시**: 채팅 보기는 승인 결과의 근거가 아닙니다(§4.4). 탈락.
- **고지 기록을 앱 설정 YAML에 합침**: 앱 설정 저장 형식이 아직 없고, 고지 기록은 CLI마다 한 줄이면 충분합니다. 신뢰 목록처럼 별도 파일로 둡니다.

## Consequences

- 승인·모드·시작·조립을 Rust test와 테스트 대역으로 검증할 수 있습니다. 실제 앱 연결은 다음 작업입니다.
  - MCP proposal·`BeginAgentEdit` 처리에서 요청을 열고 확정 결과를 권한 저장소·lease 관리자에 넘기기, 근거 생성: XXIX.3.1·XXIX.3.4
  - 내장 세션의 attached `tools/list`·`tools/call`에 `builtin_tools`·`builtin_tool` 적용: XXIX.3.1(app host trait 구현)
  - `FirstConnectionApproval`을 권한 저장소의 첫 연결 승인으로 넣기: 권한 저장소가 앱 crate로 옮겨질 때(ADR-0093 결정 12)
  - 출처 색인에서 `BatchRecord` 만들기: XIX.6.7
  - 금지 필드 corpus를 scanner에 넣기: XVI.8.2·XVI.8.6
- runtime이 agent-host(와 그 의존 agent-protocol·local-ipc)를 빌드합니다. FFI header는 바뀌지 않습니다.

## Compatibility

- 파일 포맷, 공개 FFI, schema 변경이 없습니다. 새 앱 내부 파일은 OS 사용자 데이터의 약관 고지 기록(형식 version 1)뿐입니다.
- agent-host 공개 API가 늘었습니다: `approval`, `mode`, `start`, `assembly`, `scenario`, `privacy`, `SessionEffect::RevokePermissions`, `AgentSession::change_mode_by_user`, `SessionEffect::kind`, `SessionState::code`, `TermsResult::name`. 기존 match 사용처(agent-host test 하나)를 맞췄습니다.

## Performance

- 성능 예산 파일은 바꾸지 않습니다. 승인 저장소 연산은 대기 요청 수에 선형이며 UI·문서 스레드 밖(Agent Host 스레드, XXIX.3.4)에서 부르는 것을 전제로 합니다. 채팅 조립의 상한 처리는 상한에 닿았을 때 앞쪽 탐색 한 번입니다.

## Security

- 응답·모드 변경·고지 확인·신뢰 쓰기를 에이전트 경로가 부를 수 없음을 inventory test로 고정합니다(ADR-0005 §3).
- 신뢰되지 않은 프로젝트에서는 시작 확인 데이터도 만들지 않고, 확인 순간에 다시 확인합니다(fail closed).
- 고지 기록에는 CLI 종류·검토 일자·결과만 있고 계정·경로·내용은 없습니다. symlink와 손상 파일은 실패로 닫습니다.
- 진단·`Debug`에 에이전트 금지 필드가 나오지 않음을 표지 test로 확인합니다.

## Migration

- 기존 데이터가 없습니다. agent-host 기존 동작은 바뀌지 않습니다. 권한 받기 판정은 같은 결과를 `grant_route`로 계산합니다.
