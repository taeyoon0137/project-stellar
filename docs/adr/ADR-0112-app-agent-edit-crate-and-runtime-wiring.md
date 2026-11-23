# ADR-0112: 앱 쪽 에이전트 편집 crate `stellar-agent-edit`와 runtime 연결

## Status

Accepted (2026-11-23). engine lane `agent-edit-crate`의 결정입니다. 계획 XXIX.3.1과 XXIX.3.4의 엔진 부분(셸 FFI 제외)을
구현합니다.

- ADR-0093 결정 12("앱이 권한 저장소·요청 관리를 소유하게 되면 mcp-core의 permission·lease·attached를 에이전트 편집 구현
  crate로 옮긴다")와 ADR-0100 결정 3("출처 색인은 `agent_history`·`attached`와 함께 옮긴다")을 실행합니다. 두 ADR의 판정·기록
  결정은 바꾸지 않고, 그 module이 있는 crate 경로만 이 ADR이 대체합니다. 두 ADR 본문은 고치지 않습니다.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다. 셸 FFI와 Swift UI는 범위가 아닙니다.

## Context

- ADR-0005 §8: runtime은 mcp-core에 닿을 수 없습니다(`FORBIDDEN_REACH`). 그런데 앱이 소유해야 할 상태인 권한 저장소
  (`PermissionStore`), lease 요청 관리(`AgentEditLeases`), 승인 전 미리보기·적용, 출처 색인(`AgentChangeProvenance`), 세션
  종료 절차(`AgentEditSessions`), 되돌리기가 mcp-core에 임시로 있었습니다. 그래서 runtime이 창 history와 이 상태를 이을 수
  없었고, 변경 불가 경로 inventory(XXIX.6.5)는 이 파일들을 에이전트 경로 검사에서 이름으로 빼야 했습니다(`APP_SIDE_MODULES`).
- 위 module 중 MCP 브리지 제품 코드(server·headless·catalog)가 부르는 것은 없었습니다. 브리지가 쓰는 것은 tool 정의와 인자
  해석(`document.begin_agent_edit` 정의, `parse_begin_agent_edit`, proposal 해석기 `proposal::parse_transaction`)뿐입니다.
- 창 history 연결 지점(`stellar_commands::agent_sink::AgentHistorySink`)과 창 history 구현(`stellar_history::agent::
  WindowAgentHistory`, `AgentEntryLabels`)은 이미 있습니다(ADR-0100 결정 2). runtime의 창 Undo·Redo는 재생 결과를 버리고
  bool만 돌려줬습니다.
- runtime document thread 수신함은 우선순위 lane(Input, Command, Background)을 가진 bounded queue입니다(XII.1.2). lane마다
  용량이 따로이고, `Background`는 넘칠 때 오래된 항목을 밀어냅니다.
- 사용자 방향(2026-11-23): 엔진·기능만 만들고 Swift UI·셸 FFI는 더하지 않습니다.

## Decision

1. **crate.** `crates/agent-edit`(package `stellar-agent-edit`)를 만들고 mcp-core에서 다음을 module 이름 그대로 옮깁니다:
   `permission`(+test), `provenance`(+fixture·test), `agent_history/**`, `attached/{edit, revert}`(+test). lease 요청 관리는
   새 `lease` module입니다. 옛 경로 `stellar_mcp::X`는 `stellar_agent_edit::X`가 됩니다. 판정·기록 함수는 commands에 그대로라
   동작은 바뀌지 않습니다.
   - 허용 직접 의존: `stellar-ops`(commands 재수출·diff·validate), `stellar-agent-protocol`(권한 어휘·결과 종류). runtime이 이
     crate에 의존합니다.
   - `FORBIDDEN_REACH`: mcp-core, stellar-mcp, agent-protocol, local-ipc, stellar-cli, agent-host, ops는 이 crate에 직접·전이로
     닿지 않습니다. 이 crate는 mcp-core, stellar-mcp, local-ipc, agent-host, runtime, ffi에 닿지 않습니다. 그래서 에이전트
     경로에서 권한 응답·출처 표시 정리·되돌리기를 부를 수단이 crate graph로 막힙니다(ADR-0005 §3).
   - 변경 불가 경로 inventory의 `APP_SIDE_MODULES` 예외를 지우고, 에이전트 경로 소스 전체에 `stellar_agent_edit`와 옮긴 module
     호출 표지가 없는지 예외 없이 봅니다. XIX.8.1 작업 사본 타입 inventory는 이 crate도 검사합니다.
2. **브리지와 앱의 경계.** mcp-core에는 tool 정의와 인자 형식 해석만 남습니다.
   - `stellar_mcp::lease`: `BEGIN_AGENT_EDIT`, `BeginAgentEdit`, `parse_begin_agent_edit`, `LeaseRequestError`, 상한 상수. 문서를
     보는 `resolve`는 뺍니다.
   - `stellar_agent_edit::lease`: 앱이 받은 대상 선언 `LeaseTargets{document, targets, externals}`, 문서 기준 확인
     `LeaseTargets::resolve`(다른 문서·없는 node·대상 모양, `TargetError`), `AgentEditLeases`·`LeaseState`·`LeaseServiceError`·
     `open_project_coordinator`. 문서 기준 확인은 이 crate 한 곳입니다.
   - `stellar_agent_edit::attached::edit`: JSON을 받지 않습니다. 브리지 해석기가 만든 `stellar_ops::apply::ApplyRequest`를
     `decode_proposal`(같은 `decode_commands`, 빈 요청·상한 포함)로 닫힌 enum `DecodedProposal`로 바꾸고,
     `DecodedProposal::transaction`이 창 문서와 lease로 transaction을 만듭니다. `proposal_transaction`은 둘을 잇습니다.
   - 브리지 쪽 해석 결과(`BeginAgentEdit`)와 앱 쪽 입력(`LeaseTargets`, `ApplyRequest`)은 IPC 경계 양쪽의 값입니다. 앞단(IPC
     frame 해석)이 생기면 그 경계에서 바꿉니다. 규칙은 각 쪽에 하나씩입니다(형식은 브리지, 문서 기준은 앱).
3. **창 상태.** runtime `agent_edit::WindowAgentEdit`가 창마다 세션 관리자, 출처 색인, `AgentEntryLabels`, 권한 저장소, lease
   요청 관리자(조정자는 처음 필요할 때 창 프로젝트 identity로 엶)를 갖습니다. 앱 창 표 항목(`WindowEntry`)의 필드이고 창과
   함께 버립니다. 권한 key는 창 프로젝트 identity(project id + canonical target)와 세션입니다.
   - `AgentWindow`(창 session과 이 상태를 함께 빌린 view)가 `WindowAgentHistory{window, doc, labels}`를 만들어 완료·종료 절차와
     되돌리기의 연결 지점으로 씁니다.
   - 창·프로젝트를 닫으면 남은 세션을 사용자 중지(`UserStopped`)로 끝내(lease 해제·권한 폐기, rollback 없음) 출처 색인을
     버립니다. 확정한 history 항목은 창과 함께 버려집니다.
4. **Undo·Redo·자동 저장·이름.**
   - 창 Undo·Redo(`AppCommand::Undo/Redo`)는 `agent_edit::replay`를 거칩니다. `ProjectWindowSession::replay_history`가 재생한
     항목과 change set을 돌려주고(기존 `undo`·`redo`도 이 함수를 부름), `on_history(sequence, 방향)`로 표시 상태를 전이한 뒤
     재생 change set을 그 항목 출처로 `observe`에 넘깁니다.
   - 승인한 제안을 적용하면 그 change set을 batch 출처로 `observe`에 넘깁니다. 같은 세션 batch는 `observe`가 건너뛰고, 다른
     세션의 표시된 batch는 다시 판정됩니다(ADR-0100 결정 7 "다른 세션 batch").
   - 되돌리기 적용·제안 적용은 작업 문서 revision을 올립니다. 앱 사용자 UI 동작은 `AppRuntime::with_agent_edit`(job 접수기
     `JobPublisher`)로 document thread에서 실행되고, job 뒤 상태 게시가 자동 저장 예약을 관찰하므로 되돌리기 change set은 다음
     자동 저장에 들어갑니다. 자동 저장 경로를 따로 두지 않습니다.
   - Undo·Redo 메뉴 이름은 `AgentWindow::history_names`(`AppRuntime::agent_history_names`)가 `AgentEntryLabels::action_name`으로
     냅니다. 에이전트 항목이 아니면 일반 이름입니다.
   - 사용자 UI 전용 동작(lease 승인, 제안 적용·거부, 유지, 되돌리기 적용, 권한 응답)은 `UserUiAction` 표식을 받습니다.
5. **Agent Host·IPC I/O 스레드 코어**(XXIX.3.4). runtime `agent_io`.
   - `AgentIo`(접수기)와 `AgentIoThread`(스레드 `stellar-agent-io`, bounded 수신함)는 document thread job 접수기만 갖고 문서에
     닿는 수단이 없습니다.
   - 이 스레드에서 검증합니다: proposal은 `decode_proposal`로 닫힌 enum command가 된 뒤에만 넣습니다. 바꿀 수 없으면 넣지 않고
     거부합니다.
   - 검증한 요청은 `Command` lane에 넣습니다(사용자 입력 `Input`보다 늦게 처리). `Background`는 넘칠 때 밀어내므로 쓰지 않습니다.
   - 에이전트 요청은 처리 전 요청 수 상한(기본 64, `Command` lane 기본 용량 1024보다 작음)을 따로 둡니다. 상한·lane·수신함이
     차면 `Busy`로 바로 거부하고 이미 접수한 요청은 버리지 않습니다. 에이전트가 사용자 command 자리를 다 쓰지 못합니다.
   - document thread에서는 세션 등록, lease 요청(승인 대기), 제안 대기, 완료, 연결 종료, 종료 절차만 합니다. 문서 적용은 사용자가
     승인한 뒤의 `apply_proposal`입니다.

## Alternatives

- **mcp-core에 두고 runtime → mcp-core edge를 허용**: ADR-0005 §8 runtime 행을 뒤집습니다. MCP 브리지 library 전체(JSON-RPC,
  catalog, headless backend)가 앱 프로세스에 들어오고 에이전트 경로와 사용자 UI 상태가 다시 한 crate에 섞입니다.
- **agent-edit가 mcp-core에 의존**: runtime이 전이로 mcp-core에 닿습니다.
- **mcp-core가 agent-edit에 의존해 해석기를 공유**: 에이전트 경로가 사용자 UI 전용 함수에 닿을 수 있게 됩니다.
- **`BeginAgentEdit` 해석기를 형식 중립 trait로 agent-edit에 두고 브리지 해석기를 지움**: 브리지가 tool 인자를 먼저 거부하지
  못합니다. 형식 검사는 브리지, 문서 기준 확인은 앱으로 나눕니다.
- **에이전트 전용 queue lane 추가**: XII.1.2 queue 구조 변경입니다. 처리 전 요청 상한으로 같은 격리를 얻습니다.
- **창 Undo·Redo 재생을 runtime agent module에 다시 구현**: 창 session의 재생 경로가 둘이 됩니다. `replay_history` 하나를 둡니다.
- **되돌리기 change set을 자동 저장에 직접 넣는 함수**: 자동 저장 예약은 revision 관찰 하나로 동작합니다. 같은 관찰 경로를 씁니다.

## Consequences

- 앱 runtime이 에이전트 세션 등록, lease 요청·승인, 제안 대기·승인 적용, 완료·종료, 되돌리기, 유지, Undo·Redo 표시 전이를 갖고
  Rust test로 확인됩니다.
- 남은 일:
  - IPC frame 읽기와 tool 인자 → `AgentRequest` 변환(브리지 attached 경로, local-ipc 연결).
  - `stellar-agent-host` `AgentSession` 종료 효과(`ReleaseLeases`, `FinalizeInterruptedHistoryEntry` 등)와 이 세션 관리자 연결,
    agent-host `AgentSessionId`(u64)와 이 crate `AgentSessionId`(StableId) 대응, `FirstConnectionApproval`을 권한 저장소 첫 연결
    응답으로 넣는 일, runtime의 agent-protocol host trait 구현(XXIX.3.1).
  - 셸 FFI(XXIX.3.1)와 UI.
  - 사용자 편집 경로마다 change set을 `observe`에 넘기는 일. 지금은 Undo·Redo 재생과 다른 세션 batch 적용만 넘깁니다.
  - 20만 node 문서의 lease 대상 계산·batch 적용 단일 점유 trace(XXIX.3.4 검증 열).
- 미리보기 validation에 넘길 컴포넌트 정의 목록은 호출자가 줍니다(창 컴포넌트 원본 cache와의 연결은 후속).

## Compatibility

- 파일 포맷·FFI·schema 변경은 없습니다. 새 상태는 모두 창 메모리 전용이고 직렬화 경로가 없습니다.
- 공개 API 이동: `stellar_mcp::{permission, provenance, agent_history, attached}` → `stellar_agent_edit::{…}`(이름 그대로).
  `stellar_mcp::lease`는 tool 정의·인자 해석만 남고, 요청 관리·문서 기준 확인은 `stellar_agent_edit::lease`(`LeaseTargets`,
  `TargetError`, `LeaseServiceError::{Target, Identity}`)입니다. `attached::edit::proposal_transaction`은 JSON 대신
  `ApplyRequest`를 받습니다.
- 공개 API 추가: runtime `agent_edit::{WindowAgentEdit, AgentWindow, HistoryNames, replay, monotonic_ms}`,
  `AppRuntime::{with_agent_edit, agent_history_names, agent_io, agent_io_with_limit}`, `agent_io::{AgentIo, AgentIoThread,
  AgentRequest, AgentReply, AgentIoError}`.

## Performance

- Undo·Redo마다 표시 중 batch의 겹치는 key만 지문을 다시 계산합니다(ADR-0100 결정 7). 에이전트 batch가 없으면 비용이 없습니다.
- 에이전트 요청은 `Command` lane 순서를 따르고 처리 전 요청 상한으로 묶입니다. 사용자 입력 lane에는 영향이 없습니다.
- lease 요청·승인과 제안 대기는 대상 subtree 순회만 합니다. 큰 batch 적용은 transaction 하나라 한 job 안에서 끝납니다. 단일 점유
  상한과의 관계는 측정하지 않았습니다(Consequences).

## Security

- 앱 사용자 UI 전용 상태와 함수는 에이전트 경로 crate가 닿지 않는 crate에 있고, crate graph 검사와 inventory test가 이를
  확인합니다.
- I/O 스레드는 문서를 쓰지 않고, 닫힌 enum command로 바뀐 요청만 document thread에 넣습니다.
- 창을 닫으면 남은 세션의 lease와 권한을 풉니다. 상태는 프로젝트 파일·Git·복구 snapshot·진단에 들어가지 않습니다.

## Migration

- 기존 데이터 마이그레이션은 없습니다. 옮긴 module을 쓰던 코드는 crate 경로만 바꿉니다(workspace 안 사용처는 이 변경에서
  모두 바꿨습니다).
