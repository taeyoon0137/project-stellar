# ADR-0124: 에이전트 앱 연결 2 — 앱 시작 세션 연결, host trait, IPC frame 앞단, 사용자·외부 변경 관찰, 큰 batch 분할

## Status

Accepted (2026-11-24). engine lane `agent-runtime-2`의 결정입니다. ADR-0112 Consequences의 "남은 일"을 셸 FFI 없이 구현합니다
(계획 XXIX.3.1, XXIX.3.4, XIX.6.7, XIX.7.2의 엔진 부분).

- ADR-0112의 crate 경계·창 상태·I/O 스레드 결정은 그대로입니다. 이 ADR은 그 위에 연결을 더하고, ADR-0005 §8 runtime 행의
  허용 직접 의존에 `stellar-local-ipc`를 더합니다(결정 2).
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다. 셸 FFI와 Swift UI는 범위가 아닙니다.

## Context

- agent-host `AgentSession`(XXIX.2.8)은 앱 시작 세션의 lease·권한 수명과 종료 절차를 `SessionEffect` 목록으로 냅니다. 그 crate는
  문서·history에 닿지 않아 효과를 실행하지 못하고, 실행할 곳(창의 `AgentEditSessions`)은 runtime에 있습니다. 두 쪽 세션 id는
  `AgentSessionId(u64)`(agent-host)와 `AgentSessionId(StableId)`(권한 key·출처 기록)로 다릅니다.
- ADR-0005 §8은 "attached tool 호출은 `stellar-agent-protocol`이 정의한 host trait로 처리하고 runtime이 구현한다"고 정했지만
  trait 정의가 없었습니다. 내장 세션의 `builtin_tools`·`builtin_tool`(ADR-0101)을 attached `tools/list`·`tools/call`에 적용할 곳도
  없었습니다.
- handshake(XIX.1.3) 뒤 bridge↔app 메시지가 정의되지 않아 연결 감시가 data를 받으면 닫습니다. I/O 스레드의 `AgentRequest`를
  만드는 앞단이 없었습니다.
- 출처 표시 "이후 변경됨"은 Undo·Redo 재생과 다른 세션 batch 적용만 `observe`로 판정했습니다. 사용자 편집과 외부 변경 반영은
  `observe`에 닿지 않았고, `respond_to_external`은 runtime에 연결되지 않았습니다. 그 사이 창 session에 외부 변경 반영과 XIII.3.4
  semantic reconciliation(ADR-0110)이 생겨, `respond_to_external`이 돌려주던 반영 transaction은 창 반영 경로와 겹치는 두 번째 적용
  경로가 됐습니다.
- 20만 node 문서에서 lease·batch의 document thread 단일 점유(상한 2 ms, 아키텍처 §6.3)를 잰 적이 없었습니다.

## Decision

1. **앱 시작 세션 연결**(runtime `agent_session`).
   - 창 상태 `WindowAgentEdit`에 `HostedSessions` 표를 둡니다. agent-host id ↔ 편집 id를 1:1로 잇고 `AgentSession` 상태를 함께
     갖습니다. 편집 쪽 id는 `new_edit_session_id`(UUIDv7: unix ms + 프로세스 id + 순번)가 만듭니다. 대응 표는 이 하나입니다.
   - `AgentWindow::start_hosted(StartedSession, …, UserUiAction)`: 시작 확인의 프로젝트(`project_key`)와 창이 이 창이 아니면 거부하고
     아무것도 바꾸지 않습니다. 맞으면 편집 세션을 `AppStarted`로 시작하고 `FirstConnectionApproval`을 권한 저장소의 첫 연결 응답
     (승인)으로 기록합니다.
   - `AgentWindow::host_event`: event를 `AgentSession::apply`에 넣고 효과를 실행합니다. 종료 효과 묶음(`RevokeSessionPermissions`를
     포함한 묶음)은 세션 관리자의 종료 절차 하나(`AgentWindow::end`, 이유는 `AgentSession` 상태의 이유를 `end_reason`으로 바꿈)로,
     정상 완료 효과(`ConfirmUndoEntry`·`ReleaseLeases`)는 `AgentWindow::complete` 하나로 실행합니다. 효과에 실린 batch 수·lease
     목록은 쓰지 않습니다. 확정할 batch와 해제할 lease는 편집 쪽(출처 색인·lease 관리자)이 정합니다. 모드 낮춤의
     `RevokePermissions`는 새 `PermissionStore::revoke`로 그 권한의 확인만 버립니다.
   - 문서 밖 효과(`TerminateProcess`, `CleanupExecutionTraces`, `CloseApprovalUi`, `EndBridgeBinding`, `RecordDiagnostic`)는 창 대기열에
     넣고 `AgentWindow::take_host_effects`로 프로세스 관리자·UI가 꺼냅니다.
   - 편집 쪽이 먼저 끝낸 세션(lease 겹침, I/O의 `End`)은 같은 이유의 event(`end_event`)를 `AgentSession`에 넣어 상태를 맞춥니다.
     이미 끝난 편집 세션에는 종료 절차를 다시 실행하지 않습니다. 창을 닫을 때 앱 시작 세션은 `UserStop` event로 끝내 프로세스 종료
     효과도 냅니다.
   - 편집 쪽 `EndReason`에 `UsageLimitReported`·`AuthErrorReported`를 더해 agent-host 이유와 손실 없이 대응합니다(진단 코드 같음).
   - 문서 변경은 document thread job 안에서 원자 실행되고 host event도 같은 스레드에서 실행되므로, event 시점에 적용 중인
     transaction은 없습니다. 그래서 runtime은 `TransactionStarted`·`TransactionSettled`를 넣지 않습니다.
2. **host trait와 runtime 구현.**
   - `stellar-agent-protocol::host::AttachedHost`(연결 handle·요청·접수 결과를 연관 타입으로 둠): `listed_tools`(attached
     `tools/list`), `permits`(권한 하나의 상한 판정), `submit`(검증한 요청 접수), 기본 구현 `callable_tool`(목록에 있는 tool인지 +
     `permits`). 거부는 `HostRefusal{kind: ResultKind, code}`로 ADR-0085 결과 종류를 씁니다.
   - runtime `agent_attached::AppAgentHost`가 구현합니다. 앱 시작 세션은 `builtin_tools(mode)`·`grant_route`(ADR-0101)를,
     외부 클라이언트는 attached catalog 목록을 씁니다. 실제 권한 상태(첫 연결 승인 등)는 document thread의 권한 저장소가
     판정합니다(이 trait는 I/O 스레드의 앞 단계 거부).
   - runtime 직접 의존에 `stellar-agent-protocol`(ADR-0005 §8 허용 목록에 이미 있음)과 `stellar-local-ipc`(이 ADR이 더함)를 등록합니다.
     runtime은 이미 agent-host를 거쳐 둘에 닿습니다. local-ipc는 document·commands·ops에 닿지 않는 규칙이 그대로라 방향이 바뀌지
     않습니다.
3. **IPC frame과 앞단.**
   - local-ipc `frame`: handshake 뒤 메시지를 길이 접두 binary frame으로 정합니다(`"SF"` magic, version 1, 종류, 요청 id, payload 길이).
     payload는 StableId 16 byte, 문자열·byte열은 u32 길이 접두입니다. 한 frame 상한은 1 MiB(`MAX_FRAME_BYTES`, agent-protocol
     `MAX_MESSAGE_BYTES`와 같음, runtime test가 대조)이고, 모르는 version·종류, 상한 초과, 남는 byte는 거부합니다(fail closed).
   - frame 종류: bridge→app `BeginEdit`(문서, target root, 전역 resource 종류 이름·id, 설명), `Propose`(base revision 표기, 문서,
     command 종류 이름 + canonical bytes), `Complete`; app→bridge `Reply`(결과 종류 이름, 문장).
   - runtime `agent_frame`: frame을 `AgentRequest`로 바꿉니다. 형식 검사(StableId·tool 인자)는 브리지가 이미 했고, 앱은 값 해석만
     기존 공개 함수로 합니다: base revision은 `ProjectRevision::from_str`, 전역 resource는 `ExternalKind::from_name`, command는
     `CommandInput::Canonical`로 넣어 I/O 스레드의 `decode_proposal`(같은 `decode_commands`)이 닫힌 enum으로 바꿉니다. lease id·
     transaction id·완료 항목 id는 앱이 만듭니다. 연결마다 진행 중 lease를 기억해 `Propose`·`Complete`에 씁니다. 연결 종료는
     `Disconnected` 요청입니다. host trait로 권한 상한을 먼저 봅니다(`BeginEdit`은 Propose, `Propose`는 `document.propose_commands`).
   - attached backend에서 proposal은 사용자 승인 대기로 들어갑니다. 그래서 Propose 권한이면 충분하고(검토 후 적용 모드), 승인 범위
     안 바로 적용(`document.apply_commands`, Apply)은 후속입니다.
   - document thread에서 `BeginEdit`·`Propose`는 그 세션의 첫 연결 승인이 `Approved`일 때만 처리합니다(XIX.3.3). 아니면 거부합니다.
4. **사용자 편집 관찰**(XIX.6.7 "이후 변경됨").
   - 창 session의 문서 기록 함수(`apply`, `apply_grouped`, `insert_*`, `apply_and_select`, 외부 반영)가 기록한 change set과 출처를 창
     관찰 대기열에 넣습니다. 상태 snapshot 게시(`build_state`, 모든 document thread job 뒤) 앞에서 runtime이 대기열을 비워 그 창의
     `AgentChangeProvenance::observe`에 넘깁니다. 기록 함수마다 관찰 함수를 따로 부르지 않아 빠지는 경로가 없습니다.
   - 표시된 batch가 없으면 대기열을 쌓지 않습니다(관찰 비용 0).
   - 속성 scrub처럼 frame마다 revision이 바뀌는 history preview는 preview 중 넣지 않고 끝(`end_preview`)에서 마지막 change set
     하나만 넣습니다. scrub은 매 단계 시작 상태에서 다시 계산하므로 마지막 change set이 제스처 전체입니다. 취소는 값을 되돌리므로
     넣지 않습니다. 캔버스 drag는 preview를 문서에 쓰지 않고 끝에서 `apply` 한 번이라 같은 경로입니다.
   - Undo·Redo 재생(`agent_edit::replay`)과 에이전트 batch 적용은 지금처럼 그 자리에서 넣습니다(대기열에 넣지 않음).
5. **외부 변경과 `respond_to_external`**(XIX.7.2).
   - 외부 변경을 창 문서에 넣는 경로는 창 session 하나입니다(clean이면 `ApplyExternalSnapshot`, 미저장 편집이 있으면 XIII.3.4
     reconciliation, 겹치면 두 선택지). `respond_to_external`은 문서를 바꾸지 않고 lease 겹침 판정과 겹친 세션 종료만 합니다.
     돌려주던 반영 transaction(`apply`)과 `divergent`는 지웁니다. 분할·semantic conflict 후보는 분석(`analysis`)에 판정 근거로 남습니다.
   - 창이 lease를 쥔 세션을 가질 때만(`watch_external`) 창 session이 외부 변경 관찰값(base 원본, 외부 원본)을 대기열에 넣습니다.
     reconciliation이 겹쳐 반영하지 못한 경우(두 선택지)도 넣습니다. 외부 변경이 lease와 겹치면 반영 여부와 관계없이 세션을 끝냅니다.
   - 대기열을 비울 때 `respond_to_external`로 겹친 세션을 `LeaseConflict`로 끝내고(자동 rollback 없음), 앱 시작 세션이면
     `AgentSession`에도 알립니다. 반영한 외부 변경 change set은 System 출처로 `observe`에 넘깁니다(결정 4).
6. **큰 batch 분할과 단일 점유 측정**(XXIX.3.4).
   - `AgentEditSessions::apply_proposal_chunk`: 승인한 제안의 앞 command 최대 N개를 한 batch(transaction 하나, 전부 적용 또는 전부
     미적용)로 적용하고 남은 부분을 같은 제안 id로 대기에 둡니다. batch마다 출처 기록이 생기고, 완료·종료는 진행 중 batch 전체를
     history 항목 하나로 확정하므로 Undo 한 번이 제안 전체를 되돌립니다. batch가 거부되면 남은 부분을 버리고 앞 batch는 표시와 함께
     남습니다. 기존 `apply_proposal`은 N = 무한인 같은 함수입니다.
   - runtime `AppRuntime::apply_agent_proposal`: batch 하나를 document thread job 하나로 실행하고, 남으면 다음 batch job을 `Command`
     lane 뒤에 다시 넣습니다. 그 사이 `Input` lane의 사용자 입력이 먼저 처리됩니다. 결과는 마지막 batch 뒤 한 번 보냅니다.
   - N은 `PROPOSAL_BATCH_COMMANDS = 512`입니다. release 측정(20만 node, lease root가 500 node group)에서 command 하나의 적용·출처
     기록이 약 1.4 µs라 512개가 약 0.7 ms이고 상태 게시 여유를 남깁니다. 측정값은 `docs/task-reports/XXIX.3.4.md`에 있습니다.
   - lease 대상 계산(`LeaseTargets::resolve`)은 대상 root마다 조상 사슬만 보므로 20만 node에서도 0.4 ms 안입니다(대상 400개). 나누지
     않습니다.

## Alternatives

- **effect의 batch 수·lease 목록으로 실행**: agent-host와 편집 쪽이 같은 사실을 따로 셉니다. 편집 쪽 기록을 기준으로 두고 효과는
  "무엇을 할지"만 봅니다.
- **편집 세션 id를 agent-host id에서 계산(hash)**: 대응을 표 없이 만들 수 있지만 외부 클라이언트 세션(연결 id 기반)과 규칙이 둘이
  됩니다. 한 표와 한 생성 함수로 둡니다.
- **runtime이 local-ipc에 닿지 않고 agent-host 재수출로 frame 사용**: crate graph 정책을 재수출로 우회합니다. 허용 edge로 등록합니다.
- **frame payload를 JSON으로**: local-ipc는 core-types만 의존하고 JSON 구현이 없습니다. 앱 쪽 해석은 tool 인자 형식이 아니라 브리지가
  검사한 값의 전달이라 고정 binary로 충분합니다. command는 canonical bytes라 해석 규칙이 commands codec 하나입니다.
- **사용자 편집 호출 지점마다 `observe` 호출**: 호출 지점이 20곳이 넘고 새 편집 경로가 빠지기 쉽습니다. 기록 함수 한 층에서 모읍니다.
- **revision이 바뀔 때마다 표시된 모든 지문을 다시 계산**: change set 없이도 되지만 표시 batch 전체 지문을 매 편집마다 계산합니다.
- **`respond_to_external`의 반영 transaction을 창에 적용**: 창 session의 반영·reconciliation과 다른 두 번째 적용 경로가 됩니다.
- **큰 제안을 큐에 넣을 때 여러 제안으로 나눔**: 사용자 승인이 여러 번 필요해집니다. 승인은 한 번, 적용만 나눕니다.
- **제안 전체를 사본에 먼저 적용해 미리 검증한 뒤 나눠 적용**: 20만 node 문서 사본 하나가 단일 점유 상한을 넘습니다.

## Consequences

- 앱 시작 세션의 종료·완료·모드 낮춤이 창의 세션 관리자·권한 저장소에 실행되고, 첫 연결 승인이 기록됩니다.
- attached `tools/list`·`tools/call` 판정이 앱 쪽 host trait 하나로 갑니다. bridge 쪽 frame 쓰기(mcp-core attached backend, XIX.9.4)와
  연결 감시(`stellar_local_ipc::session`)가 frame을 읽도록 바꾸는 일은 후속입니다. 지금 감시는 data를 받으면 닫습니다.
- 사용자 편집·외부 반영이 표시된 batch의 "이후 변경됨"을 바로 바꿉니다.
- 큰 제안은 batch 여러 개로 적용되어 제안 단위 원자성 대신 batch 단위 원자성입니다. 거부되면 앞 batch가 남습니다(출처 표시·되돌리기
  가능).
- 남은 일:
  - lease가 문서 대부분을 덮으면 `LeaseTable::snapshot`(commands `lock::lease`)이 subtree 전체를 걷어 batch 크기와 관계없이 20만
    node에서 약 10 ms입니다. lease 판정이 걸리는 사용자 편집도 같습니다. 변경 node의 조상만 보는 판정으로 바꾸는 일은 commands lock
    소유 작업입니다.
  - BeginAgentEdit 작업 설명을 승인 화면에 넘기는 일(`LeaseTargets`에 설명이 없음), Propose·Apply 권한 확인 요청 흐름, 승인 범위 안
    바로 적용.
  - 사용자가 두 선택지에서 디스크 쪽을 고른 경우(`reload_latest_disk`)의 세션 처리.

## Compatibility

- 파일 포맷·FFI·schema 변경은 없습니다. frame은 프로세스 사이 메시지이고 version 1입니다. 새 상태는 모두 창 메모리 전용입니다.
- 공개 API 변경: `respond_to_external`이 `apply_id`를 받지 않고 `ExternalDecision`에서 `apply`·`divergent`가 빠졌습니다(workspace 안
  사용처는 test뿐). `agent_history::EndReason`에 두 variant가 늘었습니다.
- 공개 API 추가: `PermissionStore::revoke`, `AgentEditSessions::apply_proposal_chunk`·`ChunkApplied`, agent-protocol `host`, local-ipc
  `frame`, runtime `agent_session`·`agent_attached`·`agent_frame`, `AppRuntime::apply_agent_proposal`.

## Performance

- 성능 예산 파일은 바꾸지 않습니다. batch 분할로 큰 제안의 job 하나가 상한 안에 들어갑니다(결정 6, lease snapshot 예외는
  Consequences).
- 사용자 편집 관찰은 표시된 batch가 있을 때만 change set을 복사합니다. 외부 변경 관찰값은 lease를 쥔 세션이 있을 때만 만듭니다.

## Security

- frame 해석은 fail closed이고 상한이 있습니다. 앱은 브리지가 만든 lease·transaction id를 받지 않고 직접 만듭니다.
- 첫 연결 승인 전에는 lease 요청·제안을 처리하지 않습니다. 권한 응답·출처 표시 정리·되돌리기는 여전히 `UserUiAction`이 필요하고
  에이전트 경로 crate는 agent-edit에 닿지 않습니다(ADR-0112).

## Migration

- 데이터 마이그레이션은 없습니다. `respond_to_external` 호출자는 반영을 창 session 경로에 맡깁니다.
