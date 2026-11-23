# ADR-0132: 에이전트 lease 판정 비용, bridge frame 연결, 문서 밖 의미 변경 관찰과 fail closed

## Status

Accepted (2026-11-24). engine lane `agent-lease-perf`의 결정입니다. ADR-0124 Consequences의 "남은 일" 가운데 lease 판정 비용,
bridge 쪽 frame 쓰기와 연결 감시 연결을 구현하고, 외부 변경 관찰의 빈 곳(문서 밖 의미 파일, 판정 실패)을 닫습니다(계획
XXIX.3.4, XIX.7.2, XIX.5.3, XIX.9.4 일부).

- ADR-0093(lease 판정 규칙), ADR-0112(crate 경계·I/O 스레드), ADR-0124(frame·관찰 대기열)의 결정은 그대로입니다. 판정 규칙의
  결과는 바꾸지 않고 비용만 바꿉니다.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다. 셸 FFI와 Swift UI는 범위가 아닙니다.

## Context

- `LeaseTable::snapshot`(commands `lock::lease`)은 판정 전에 잠긴 subtree 전체를 걸어 node → lease 표를 만들었습니다. lease가
  문서 대부분을 덮으면 20만 node에서 batch 크기와 관계없이 약 10 ms라 document thread 단일 점유 상한 2 ms(아키텍처 §6.3)를
  넘었습니다. 그 lease가 있는 동안 lease 판정을 거치는 사용자 편집도 같았습니다(XXIX.3.4 보고서).
- 판정에 필요한 것은 바뀐·삭제 node의 적용 전 소속뿐입니다. 소속은 조상을 따라 올라가 lease root를 찾아 정하므로(XIX.4.2)
  변경 node의 조상 사슬만 보면 됩니다. 그런데 판정은 transaction을 적용한 뒤 change set으로 하므로, 그때는 삭제된 node와 옮겨진
  node의 적용 전 부모가 문서에 없습니다. command 종류별로 대상을 미리 계산하지 않는다는 규칙(ADR-0093)도 지켜야 합니다.
- local-ipc 연결 감시(`ConnectionMonitor`)는 handshake 뒤 data를 받으면 닫았고, runtime 앞단 `AgentFrontEnd::serve`는 있었지만
  감시와 이어지지 않았습니다. bridge 쪽(mcp-core)에는 frame을 쓰는 코드가 없었습니다.
- 외부 변경 관찰(ADR-0124 결정 5)은 문서 원본이 바뀐 경우만 봤습니다. 컴포넌트 원본·token 파일만 바뀌면 창 session은
  `adopt_library_change`로 문서 밖 의미만 받고 관찰값을 넣지 않아, 그 resource를 전역 resource로 잠근 세션이 그대로 남았습니다.
- `agent_observe::respond`는 lease 표 읽기나 `respond_to_external`이 실패하면 아무 세션도 끝내지 않았습니다(fail open).
- attached 편집 미리보기(`agent-edit attached::edit::preview`)는 레코드만 비교하는 `diff_records`를 썼고, 미리보기 사본은
  레코드로만 만들어 Component Context(ADR-0099)를 잃었습니다. Context command의 미리보기 diff가 비고, 사본에 적용한 결과도 실제
  적용과 달랐습니다.

## Decision

1. **변경 node의 조상만 보는 lease 판정.**
   - `LeaseTable::snapshot(before, changes)`는 적용 전 문서에서 change set의 바뀐·삭제 node마다 조상 사슬을 걸어 소속을 찾고,
     lease root의 진조상을 기록합니다. 진조상은 부모·순서 변경 판정에만 쓰므로 change set에 그런 경로가 있을 때만 걷습니다.
     비용은 (변경 node 수 + lease root 수) × 깊이이고 잠긴 subtree 크기와 관계없습니다. `judge`·`touched_leases`의 규칙은
     그대로이고, 모든 lease 배치·command·주체 조합에서 옛 subtree 걷기 판정과 결과가 같음을 test가 대조합니다.
   - 적용 전 문서는 `execute_leased`가 만듭니다. transaction을 적용해 change set과 역 transaction을 얻은 뒤, 역 transaction의
     command를 `TransactionGuard` 안에서 적용해(undo와 같은 primitive, 검증 없음) 적용 전 문서를 잠깐 되살리고, 소속을 기록한
     뒤 guard journal로 적용 후 문서로 되돌립니다. 문서를 복제하지 않고 비용은 transaction이 바꾼 primitive 수에 비례합니다.
     이 문서에 lease root가 없으면(전역 resource만 잠근 lease) 이 단계를 건너뜁니다.
   - 역 transaction 적용이나 guard 되돌리기가 실패하면 판정하지 않고 거부합니다(`ViolationReason::Unjudged`, fail closed).
     문서와 revision은 기존 거부 경로대로 실행 전 상태입니다.
   - 미리보기(`lease::preview`)는 원본 문서를 바꾸지 않으므로 원본 문서로 바로 소속을 만듭니다.
2. **local-ipc 감시가 frame 연결을 앞단에 넘김.**
   - `ConnectionMonitor::serve(connection, origin, events, service)`: 감시 스레드가 handshake를 마친 연결을
     `FrameService::serve`에 넘기고, 서비스가 돌아오면 같은 종료 event를 한 번 기록합니다. frame 연결의 종료 절차는 서비스가
     맡으므로 `SessionTermination`을 받지 않습니다(종료 절차 경로 하나). 기존 `watch`(frame 없는 연결)는 그대로입니다.
   - 종료 이유는 `CloseCause::of_frame_error` 하나로 정합니다: frame 경계의 끝은 `PeerClosed`, 입출력 오류는 `ReadError`, 형식
     위반은 `UnexpectedData`입니다.
   - runtime `agent_frame::FrameConnection`이 `FrameService`를 구현합니다. 연결마다 앞단 하나(`AgentIo` 복제로 만든 host 하나)를
     두고, 연결을 `ConnectionBinding`(외부 클라이언트: 창·이름, 앱 시작 세션: 창·세션·모드)대로 묶은 뒤 `AgentFrontEnd::serve`로
     끝까지 처리합니다. 결합이 거부되면 frame을 읽지 않고 `AppShutdown`으로 닫습니다. 연결 종료 절차는 `serve`가 넣는
     `Disconnected` 요청 하나입니다.
3. **bridge 쪽 frame 쓰기**(mcp-core `attached`).
   - `AttachedLink`: handshake 뒤 stream에 요청 frame을 쓰고 같은 요청 id의 `Reply` 하나를 기다립니다(처리 중 요청 하나,
     ADR-0085 `MAX_IN_FLIGHT` 1). 요청 id는 link가 1부터 셉니다.
   - 값 변환은 기존 함수 하나씩입니다. `document.begin_agent_edit`은 `parse_begin_agent_edit`, `document.propose_commands`는
     `parse_transaction`(`stellar apply`와 같은 `command_input`) 뒤 `decode_commands`로 닫힌 enum을 만들고 commands codec
     `encode_command`의 canonical bytes와 종류 이름으로 싣습니다. 앱은 같은 codec으로 읽습니다(ADR-0124 결정 3).
   - `AttachedLink::call_tool`은 앱 응답을 MCP tool 결과로 바꿉니다. 성공은 `{"result", "message"}` 성공 결과, 거부는 그 결과
     종류의 실패, 연결 실패는 `unavailable`입니다. 입력 오류와 frame 상한(1 MiB) 초과는 보내기 전에 거부하고 연결을 유지합니다.
   - 응답 id가 다르거나, 응답이 아닌 frame이거나, 모르는 결과 종류 이름이면 연결을 더 쓰지 않습니다(fail closed).
4. **문서 밖 의미 변경과 판정 실패의 세션 종료**(runtime `agent_observe`).
   - 창 세션이 lease를 쥐면 창 원본의 문서 밖 의미(`SourceLibrary`: 컴포넌트 정의, token set, image record)를 기억하고, 상태
     게시 앞 관찰 대기열을 비울 때 지금 값과 비교합니다. 다르면 바뀐 resource를 StableId 단위로 셉니다: 컴포넌트는 정의 entry,
     token은 token 하나와 그 token set(이름·구성이 바뀌면 set id), asset은 image record입니다. 그 resource를 전역 resource로
     잠근 이 창 세션을 `LeaseConflict`로 끝냅니다(자동 rollback 없음). 창 원본을 바꾸는 경로마다 관찰 함수를 부르지 않고
     기억한 값과 지금 값만 비교하므로 `adopt_library_change`를 포함한 모든 경로가 같은 판정을 거칩니다.
   - 기억한 의미가 없던 창(cache로 열어 원본을 아직 읽지 않음)은 원본 파일 hash로 문서 밖 의미 파일이 바뀌었는지만 알 수 있어,
     바뀌었으면 전역 resource를 쥔 세션을 모두 끝냅니다(fail closed).
   - lease 표를 읽지 못하거나 `respond_to_external`이 실패하면 겹침을 판정할 수 없으므로 lease를 쥔 이 창 세션을 모두
     `LeaseConflict`로 끝냅니다(fail closed). 실패 전에 이미 끝낸 세션은 그대로입니다.
   - 창 session은 문서 밖 의미를 읽는 crate 내부 접근자(`SourceMode::library`) 하나만 더하고 기록 함수는 바꾸지 않습니다.
5. **미리보기 diff는 Context도 비교.**
   - `lease::preview`의 사본은 레코드와 함께 Component Context와 활성 기본값 세트 이름을 옮기고, `LeasePreview`가 적용 전·후
     영속 Context(`ContextSet::live`)를 싣습니다. 사본에 적용한 결과가 실제 적용과 같습니다.
   - `attached::edit::preview`의 semantic diff는 `diff_documents`(문서 범위, 레코드와 Context)로 만듭니다. 외부 변경 분석
     (`analyze_external`)·창 reconciliation과 같은 투영입니다.

## Alternatives

- **change set에 적용 전 부모를 기록**: commands `change`·`edit`(이 lane 소유 밖)의 기록 규칙을 command마다 바꿔야 하고, 기록이
  빠진 command가 있으면 판정이 조용히 틀립니다. 역 transaction은 undo가 이미 정확해야 하는 값이라 새 불변식이 없습니다.
- **invalidation flag로 "부모가 바뀌지 않음"을 추정하고 빠른 경로를 둠**: 구조 무효화는 크기·표시 변경에도 켜져 빠른 경로가
  거의 없고, 삭제 뒤 재삽입처럼 부모 경로 없이 부모가 바뀌는 경우를 구분하지 못합니다.
- **snapshot을 lease 표와 함께 cache하고 change set으로 갱신**: 문서를 바꾸는 모든 경로(외부 반영 포함)가 cache를 갱신해야 하고,
  첫 snapshot이 여전히 subtree 전체를 걷습니다.
- **판정 전에 문서를 실제로 되돌렸다가 다시 적용**: 역 transaction과 transaction을 모두 검증·실행해 비용이 세 배입니다. guard
  journal 되돌리기는 primitive 재생만 합니다.
- **감시 스레드와 앞단 사이에 공유 앞단 하나(Mutex)**: 한 연결의 응답 대기가 다른 연결을 막습니다. 연결 상태는 연결마다라
  나눠도 잃는 것이 없습니다.
- **창 session의 `adopt_library_change`에서 관찰값을 넣음**: 창 session 기록 경로마다 에이전트 관찰을 부르게 되고 다른 반영
  경로(다시 열기)가 빠지기 쉽습니다. ADR-0124 결정 4와 같이 한 곳(대기열 비우기)에서 비교합니다.
- **판정 실패 시 세션 유지**: 겹침을 모르는 채 에이전트가 계속 쓰게 됩니다. ADR-0005 §4.2의 외부 변경 겹침 종료와 같은
  방향(끝냄, rollback 없음)이 안전합니다.

## Consequences

- 문서 전체를 덮는 lease에서도 batch 하나의 lease 판정 비용이 batch 크기에 비례합니다. 20만 node release 측정에서 문서 root
  lease의 이름 변경 1024 command batch(적용 + 출처 기록)가 11.9 ms에서 2.2–2.4 ms로, 분할 단위 512 command batch가 약
  1.1 ms로 상한 안에 듭니다. lease 안 사용자 편집 하나의 거부(적용·판정·되돌리기)는 약 70 µs입니다
  (`docs/task-reports/XXIX.3.4.md`). 역 transaction 적용 단계 때문에 작은 lease(500 node group)의 499 command batch는
  0.69 ms에서 0.98–1.10 ms로 늘었습니다.
- 실제 IPC 연결에서 bridge frame이 감시 스레드를 거쳐 앞단에 닿고 응답이 돌아옵니다. bridge 쪽은 frame을 쓰고 응답을 tool 결과로
  바꿀 수 있습니다.
- 컴포넌트 원본·token·asset만 바뀌는 외부 변경도 잠근 세션을 끝냅니다. 판정하지 못하면 세션을 끝냅니다.
- 남은 일:
  - MCP 서버 루프(`server`)·실행 진입점(`entry`)이 attached 모드에서 `AttachedLink::call_tool`로 `tools/call`을 보내고, 앱 연결
    감시(`attach::watch_app_connection`)를 link의 응답 읽기로 바꾸는 일(XIX.9.4, mcp-core server·entry 소유). 지금 bridge는
    frame을 보내지 않으므로 기존 감시가 그대로 맞습니다.
  - 앱 listener의 연결 event를 `ConnectionMonitor::serve`와 `FrameConnection`에 잇는 앱 수명 코드(앱 시작 시 listener, 외부
    클라이언트 결합 창 선택 UI)는 UI 재설계 뒤입니다.
  - `CompleteAgentEdit` tool은 attached catalog에 없어 link 함수(`complete`)만 있습니다.
  - cache로 연 창이 원본을 읽기 전에 바뀐 asset(image 파일은 문서 원본 파일)은 문서 원본 경로의 외부 변경으로만 보입니다.

## Compatibility

- 파일 포맷·FFI·schema·frame 형식 변경은 없습니다.
- 공개 API 변경: `LeaseTable::snapshot`이 `(before, changes)`를 받습니다(workspace 안 사용처는 test와 측정 test뿐).
  `ViolationReason::Unjudged`, `LeasePreview::{before_context, after_context}`가 늘었습니다.
- 공개 API 추가: local-ipc `FrameService`, `ConnectionMonitor::serve`, `CloseCause::of_frame_error`; runtime
  `FrameConnection`, `ConnectionBinding`; mcp-core `attached`(`AttachedLink`, `LinkReply`, `LinkError`, `begin_frame`,
  `propose_frame`).

## Performance

- 성능 예산 파일은 바꾸지 않습니다. lease 판정의 subtree 전체 걷기를 없애 큰 lease의 batch가 분할 단위 안에서 상한 안에 듭니다.
- 문서 밖 의미 비교는 lease를 쥔 세션이 있을 때만 하고, 의미 객체가 같으면(같은 `Arc`) 바로 끝납니다.

## Security

- 판정할 수 없으면 거부하거나(편집) 세션을 끝냅니다(관찰). frame 연결은 결합을 확인한 뒤에만 frame을 읽습니다.
- bridge는 lease·transaction id를 보내지 않고 앱이 만듭니다(ADR-0124). 응답에는 고정 문장만 실립니다.

## Migration

- 데이터 마이그레이션은 없습니다. `LeaseTable::snapshot` 호출자는 적용 전 문서와 change set을 넘깁니다.
