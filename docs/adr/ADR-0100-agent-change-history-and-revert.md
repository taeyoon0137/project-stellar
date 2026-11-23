# ADR-0100: 에이전트 변경 기록·종료 절차·되돌리기를 batch 기록과 창 history 연결 지점으로 구현한다

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `agent-history` lane이 정했습니다.

- ADR-0005 §4.2(세션 수명, 종료와 사용자 되돌리기)와 ADR-0093(권한·lease)을 구현 수준에서 구체화합니다. 두 ADR의 결정은 바꾸지 않습니다.
- 대상 행: XIX.6.1\~XIX.6.4, XIX.6.6\~XIX.6.10, XIX.7.1\~XIX.7.3, XIX.8.1의 엔진 부분.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다. 셸 FFI와 Swift UI는 범위가 아닙니다.

## Context

- 에이전트 batch는 `stellar_mcp::attached::edit::apply_approved`(= `execute_leased`)로 적용되고, 결과 `AppliedTransaction`이 change set과 역 transaction을 줍니다(VII.1.4, VII.1.6).
- 창 history(`stellar_history::History`)는 transaction을 **실행하면서** 항목을 쌓습니다(`record_document`). 이미 적용된 변경을 항목으로 넣는 공개 경로가 없습니다.
- crate graph(ADR-0005 §8): mcp-core는 ops·agent-protocol·local-ipc·diagnostics만 직접 의존하고 history에 닿지 않습니다. runtime은 mcp-core에 닿을 수 없습니다. 세 쪽(mcp-core, history, runtime)이 모두 닿는 crate는 commands입니다.
- 앱 시작 세션의 재연결 유예와 상태 전이는 `stellar-agent-host` AgentSession(XXIX.2.8)이 이미 정하고, 종료 효과(`FinalizeInterruptedHistoryEntry`, `ReleaseLeases` 등)를 runtime이 실행하도록 냅니다.
- 문서 값을 StableId+PropertyPath 단위로 읽는 일반 함수가 없습니다. 등록 속성은 `stellar_commands::property::read_property`, 나머지는 레코드 codec(`encode_node_record`, modifier stack·layout record·문서 정의 `encode`)이 있습니다.
- XIII.3.4(세 snapshot semantic reconciliation)는 아직 없습니다.

## Decision

1. **batch 기록**(XIX.6.1, `stellar_commands::agent_batch`). `AgentBatchRecord`는 정방향 transaction, 적용 결과 역 transaction, change set, 생성·삭제 node, 지문 key별 적용 직후 값 지문, modifier 속성 key의 소유 node만 담습니다. 문서 사본(`DocumentModel`·`DocumentRecords`)을 담지 않으므로 크기는 batch 크기에 비례합니다.
   - 지문 key는 change set에서 정합니다. 바뀐 속성 경로는 그대로 key입니다. 생성·삭제 node와 속성 경로 없이 구조만 기록된 node(본문·path 편집, 자식 목록 변경)는 node 레코드 전체 key(`PropertyPath::new(node)`)입니다. 어떤 command도 node root만 있는 경로를 기록하지 않아 속성 key와 섞이지 않습니다.
   - 지문 값: 등록 속성은 `read_property` 값 codec, 부모·순서는 그 필드, 등록되지 않은 key(layout longhand 등)와 레코드 key는 node 레코드·modifier stack·layout record bytes, modifier key는 그 modifier 항목 하나(단일 항목 stack `encode`), 문서·페이지·View는 문서 정의와 기본값 세트 이름입니다. bytes는 프로세스 안 128-bit hash로 줄입니다. 지문은 같은 프로세스 안에서만 비교하고 저장하지 않습니다.
   - 레코드 key 지문은 자식 목록을 포함하지 않습니다. 그래서 자식 삽입·삭제로 구조만 바뀐 부모는 나중에 그 부모 속성이 바뀌지 않는 한 충돌이 아닙니다.
2. **창 history 연결 지점**(XIX.6.2, `stellar_commands::agent_sink::AgentHistorySink`). mcp-core와 history가 서로 닿지 않으므로 commands에 trait를 두고 history(`stellar_history::agent::WindowAgentHistory`)가 창 history로 구현합니다. 세션 관리는 trait만 부릅니다.
   - `confirm_group`: 이미 적용된 batch를 **다시 실행하지 않고** 정방향(이어 붙임)·역방향(역순 이어 붙임)을 history 항목 하나로 넣습니다. 이를 위해 `History`에 crate 내부 함수 `push_applied_document` 하나를 더했습니다(preview·재생 중 거부, redo 갈래와 열린 grouping을 닫음). 출처는 그룹 lease의 `FutureAgent`입니다.
   - `apply_revert`: User 출처 transaction을 `execute_leased`로 실행하고(다른 활성 lease와 겹치면 거부, XIX.4.3) 같은 함수로 항목 하나를 넣습니다. 계획 뒤 revision이 바뀌었으면 `Stale`로 거부합니다. 항목을 넣지 못하면 적용을 역으로 거둡니다.
   - 항목 이름("에이전트 편집", "에이전트 편집(중단됨)", "에이전트 변경 되돌리기")은 `AgentEntryLabels`가 sequence별로 기억합니다. `HistoryEntry::action_name`은 바꾸지 않습니다.
   - 그룹 항목은 확정 시점에 stack 맨 위에 들어갑니다. 세션 동안 사용자가 lease 밖을 편집했다면 그 항목들보다 위입니다. lease 때문에 사용자가 그룹의 node를 바꿀 수 없었으므로 그룹 Undo는 적용 가능합니다(사용자가 에이전트가 만든 node를 밖에서 참조한 경우만 역 실행이 거부되고 항목은 남습니다).
3. **출처 색인**(XIX.6.7, `stellar_mcp::provenance::AgentChangeProvenance`). batch마다 AgentSessionId, CLI 세션 표시 이름, batch 순번(세션별 1부터), lease, 그룹 history sequence(확정 뒤), `AgentBatchRecord`, 결과(진행 중·완료·중단), 표시 상태를 둡니다. 직렬화 경로가 없고 `close_window`로 버립니다. ADR-0093 결정 12와 같이 지금은 mcp-core에 두고, 앱이 세션을 소유하는 에이전트 편집 구현 crate(XXIX.3.1)가 생기면 `agent_history`·`attached`와 함께 옮깁니다. 판정·기록 함수는 commands에 있어 옮겨도 바뀌지 않습니다.
4. **세션 절차**(XIX.6.2\~XIX.6.4, `stellar_mcp::agent_history::AgentEditSessions`).
   - 세션 하나는 한 번에 lease(작업 그룹) 하나만 진행합니다(`begin_edit`가 두 번째를 `EditInProgress`로 거부). 그래서 "중단됨" 항목은 세션마다 많아야 하나입니다.
   - 정상 완료: 진행 중 batch를 `Completed` 그룹으로 확정하고 lease를 풉니다. 세션과 권한은 남습니다. history 확정이 실패하면 lease를 풀지 않고 오류입니다.
   - 종료 절차(`end`, 이유 `EndReason`): (1) 진행 중 batch를 `Interrupted` 그룹 하나로 확정, batch와 표시 유지 (2) lease(대기 요청 포함) 해제 (3) 응답 대기 확인 요청을 거부로 보고하고 세션 권한 폐기(`PermissionStore::end_session`) (4) 적용 전 제안 폐기 (5) 진단 `InterruptedAgentTask`. history 확정이 실패해도 (2)\~(5)는 반드시 실행하고 실패를 보고서에 적습니다. 확정하지 못한 batch는 진행 중으로 남고, 다음 확정 기회(예: preview가 끝난 뒤)에 `confirm_interrupted`가 같은 경로로 "중단됨" 항목 하나로 확정합니다(끝난 세션만 받음).
   - 문서 변경은 DocumentThread에서 transaction 단위로 원자 실행되므로 종료 절차는 transaction 경계에서만 실행됩니다. 적용 중인 transaction은 이미 전부 적용 또는 전부 미적용입니다.
   - 외부 클라이언트 세션은 bridge 연결 종료가 곧 종료(`McpDisconnected`)입니다. 앱 시작 세션은 `on_bridge_disconnected`가 유지하고, 유예 판정은 AgentSession이 하며 넘기면 `ReconnectGraceExpired`로 같은 `end`를 부릅니다. 유예 시간 판정을 두 곳에 두지 않습니다.
   - 사용자 중지(`cancel`)는 `UserStopped` 종료입니다. "중지하고 되돌리기"(`cancel_and_preview_revert`)는 종료 절차를 마친 뒤 세션 단위 되돌리기 미리보기를 만들 뿐이고 문서를 바꾸지 않습니다. 별도 rollback 경로는 없습니다.
   - 끝난 세션 id는 다시 시작할 수 없습니다(`Ended`).
5. **Interrupted Agent Task 진단**(XIX.6.6). `InterruptedAgentTask { session, duration_ms, error_code }` 세 필드뿐이고, 표시 문자열도 이 셋만 씁니다. 오류 코드는 `EndReason::code`이며 AgentSession과 같은 이유는 같은 문자열입니다. 로컬 diagnostic store(XVI.8.1)에 넣는 일은 그 store가 생긴 뒤입니다.
6. **되돌리기 계획**(XIX.6.8, XIX.6.9, `stellar_commands::agent_revert::plan_revert`). 문서 레코드 사본(XIX.5.3 미리보기와 같은 방식)에서 batch를 적용 역순으로 처리합니다.
   - 충돌: 지문이 다르면 `ChangedLater`(소유 node가 없어졌으면 `NodeDeleted`). 에이전트가 만든 node 아래에 그 batch가 만들거나 옮기지 않은 자식이 있으면 `ChildAddedLater`, batch 밖 node가 참조하면 `InboundReference`.
   - 사용자가 `RevertItem`으로 명시적으로 고른 충돌만 복원합니다. 구조 충돌도 기본값은 현재 유지입니다.
   - 역 transaction의 command를 하나씩 사본에 적용해, 그 command가 닿은 key(change set의 속성 경로, 또는 지문이 실제로 바뀐 레코드 key)에 제외 key가 있으면 사본에서 되돌리고 뺍니다. 함께 빠진 key는 `SharedCommand`, 적용할 수 없게 된 command는 `Dependent`입니다. 어떤 command도 닿지 않은 key의 충돌은 보고하지 않되, 구조 충돌과 `NodeDeleted`는 늘 보고합니다.
   - 결과는 계획 revision 기준 User 출처 transaction 하나입니다. command는 결정적이고 빠진 command는 사본에서 역으로 되돌렸으므로 같은 상태에 적용하면 사본과 같은 결과입니다. 세션 단위도 같은 함수(남은 batch 전부)라 전부 성공 또는 전부 미적용입니다.
   - 미리보기(`stellar_mcp::attached::revert::preview_revert`)는 그 transaction을 `attached::edit::preview`(semantic diff·validation·lease 판정, XIX.5.3)로 보여 줍니다. 적용(`apply_revert`)은 연결 지점 `apply_revert`이고, 복원 항목이 있는 batch를 "되돌림"으로 표시합니다. 결과 change set은 일반 autosave 경로 입력입니다.
   - 되돌리기는 lease 판정을 거치므로, 그 batch를 만든 세션의 lease가 아직 활성이면 자기 lease에 막힙니다. 되돌리기는 완료·중지 뒤 동작입니다(ADR-0005 §4.5).
7. **표시 상태 전이**(XIX.6.10). 상태는 검토 대기(`PendingReview`)·확인됨(`Kept`)·이후 변경됨(`ChangedLater`)·실행 취소됨(`Undone`)·되돌림(`Reverted`)입니다.
   - "유지"는 문서·history를 바꾸지 않고 표시만 지웁니다.
   - `observe(change set, 출처, 문서)`: 같은 세션이 아닌 변경이 표시된 key에 닿으면 지문을 다시 비교해 "이후 변경됨"/"검토 대기"를 정합니다. 같은 속성 값이 적용 직후로 돌아오면(그 변경의 Undo) 검토 대기로 돌아갑니다.
   - history 항목이 만든 전이는 (항목 sequence, 이전 상태) stack으로 기억합니다. 그룹 항목 Undo → 실행 취소됨, Redo → 이전 상태. 되돌리기 항목 Undo → 이전 상태, Redo → 되돌림.
   - "유지"와 되돌리기 확정은 `UserUiAction` 표식을 받습니다. MCP catalog에 이 색인을 바꾸는 tool이 없음을 test가 확인합니다(ADR-0005 §3). 표식은 문서화된 약속이고 타입으로 막지는 못합니다. 전체 쓰기 경로 inventory는 XIX.9.7입니다.
   - 세션 단위 되돌리기의 "남은 batch"는 표시 중(검토 대기·이후 변경됨)인 batch입니다. 유지·실행 취소·되돌림 batch는 넣지 않습니다.
8. **외부 변경**(XIX.7.1\~XIX.7.3, `stellar_commands::agent_external`, `stellar_mcp::agent_history::external`).
   - 외부 변경 batch는 창이 마지막으로 맞춘 원본(base)과 외부 원본(incoming)의 semantic diff(ops `diff_records`)입니다. 바뀐 객체를 대상(node, 위치가 바뀐 node, View root)으로 바꿔 `overlapping_leases`가 판정합니다: 창 문서나 외부 원본에서 lease 소속(`LeaseTable::holder`)이면 겹침, 위치가 바뀐 node·View root가 lease subtree를 품으면(`SubtreeLock::overlaps`) 겹침입니다. lease root의 진조상 속성 변경은 겹침이 아닙니다(사용자 편집 판정과 같음). 페이지·View 객체 변경은 View 추가·삭제, 다른 페이지로 이동, root Frame 교체일 때만 View root 대상이고, 페이지·View 이름이나 View 순서만 바뀐 경우는 대상이 아닙니다.
   - 반영은 레코드 단위로 나눕니다(`partition_external`). base→incoming 변경 중 창에서 바뀌지 않은 레코드만 System 출처 `ApplyExternalSnapshot`으로 적용합니다. 창과 외부가 모두 바꾼 레코드는 적용하지 않고 reconciliation 입력(`divergent`)으로 넘깁니다. 그래서 에이전트·사용자의 저장 전 변경을 덮어쓰지 않습니다.
   - 겹치면 이 창의 그 lease 세션을 `LeaseConflict` 종료 절차로 끝내고(자동 rollback 없음), 적용된 에이전트 변경은 표시를 유지한 로컬 변경으로 남깁니다. 창(base→current)과 외부가 같은 객체·속성을 다르게 바꾼 항목은 semantic conflict 후보입니다. 후보와 `divergent`를 실제 병합·Semantic Conflict 화면으로 처리하는 일은 XIII.3.4입니다. 다른 프로세스가 쥔 lease는 그 프로세스가 같은 외부 변경을 보고 끝냅니다.
   - 겹치지 않으면 세션과 lease를 유지하고 같은 분할로 외부 변경을 적용합니다.
9. **ShadowWorktree 부재**(XIX.8.1). MCP·에이전트 crate(mcp-core, agent-protocol, agent-host, stellar-mcp)가 작업 사본 개념 타입을 선언하지 않고, 세션 key가 일반 프로젝트 identity(project id + canonical target)임을 test로 확인합니다. workspace 전체 금지는 XVIII.5.3 test가 맡습니다.

## Alternatives

- **batch마다 문서 사본을 남김**: XIX.6.1이 금지합니다. 메모리가 batch 수 × 문서 크기입니다.
- **그룹 확정 때 batch를 되돌린 뒤 `record_document`로 다시 실행**: history 내부 함수를 더하지 않아도 되지만 확정마다 그룹을 두 번 더 실행하고, 사용자가 만든 inbound ref가 있으면 확정 자체가 실패합니다. 채택하지 않습니다.
- **batch를 적용할 때마다 history 항목 하나로 기록**: §4.2 "Undo entry 하나"와 맞지 않고, 기존 grouping key에는 에이전트 그룹이 없습니다.
- **mcp-core가 history에 직접 의존**: crate graph 정책(ADR-0005 §8)을 바꿔야 하고 runtime이 여전히 mcp-core에 닿지 못합니다. commands의 trait가 두 쪽을 잇습니다.
- **되돌리기 command를 command 종류별로 정적 계산해 고름**: ADR-0093이 거부한 이중 판정입니다. 사본에서 실제 change set으로 고릅니다.
- **충돌 판정에 문서 전체 revision 비교**: ADR-0005 §4.2가 StableId+PropertyPath 단위 전제조건을 정했습니다.
- **외부 변경을 `ApplyExternalSnapshot::between(current, incoming)` 하나로 적용**: 외부 원본에는 창의 저장 전 에이전트 변경이 없으므로 그 변경을 지웁니다. base 기준 분할을 씁니다.
- **앱 시작 세션 재연결 유예를 mcp-core에도 둠**: AgentSession과 시간 판정이 둘이 됩니다.

## Consequences

- 에이전트 batch 적용, 완료, 종료, 되돌리기, 표시 상태가 엔진 함수로 갖춰지고 Rust test로 확인됩니다.
- 앱 연결은 다음 작업입니다: runtime이 `WindowAgentHistory`를 만들고, `AgentEditSessions`·`AgentChangeProvenance`를 소유하는 에이전트 편집 구현 crate로 mcp-core의 세 module을 옮기는 일(XXIX.3.1), Undo·Redo 뒤 `on_history`·`observe`를 부르는 일, 되돌리기 change set을 autosave에 넣는 일, Undo 메뉴 이름에 `AgentEntryLabels`를 쓰는 일.
- XIII.3.4가 생기면 `divergent`와 semantic conflict 후보를 그 경로에 연결합니다.
- `InterruptedAgentTask`는 XVI.8.1 diagnostic store가 생기면 그 schema에 넣습니다. agent-host `SessionDiagnostic`과 모양이 같아 agent-protocol로 합치는 것이 후속입니다.
- 지문 key가 등록되지 않은 속성(layout longhand 등)과 레코드 key는 node 전체 지문이라, 같은 node의 다른 속성 변경도 충돌로 봅니다(보수적).
- 일부 key만 복원된 batch도 "되돌림"으로 표시합니다. 제외된 충돌 항목은 되돌리기 결과 목록에 남습니다.

## Compatibility

- 파일 포맷·FFI·schema 변경은 없습니다. 새 타입은 모두 세션 메모리 전용이고 직렬화 경로가 없습니다.
- 공개 API 추가: `stellar_commands::{agent_batch, agent_revert, agent_sink, agent_external}`, `stellar_history::agent`, `stellar_mcp::{provenance, agent_history, attached::revert}`. 기존 API는 바뀌지 않습니다. `History`에는 crate 내부 함수 하나가 더해졌습니다.

## Performance

- batch 기록은 change set 크기만큼 지문을 계산합니다(key마다 레코드 하나 인코딩). 문서 크기와 무관합니다.
- 되돌리기 미리보기는 문서 사본 두 개(계획 사본과 XIX.5.3 미리보기 사본)를 만듭니다. 사용자가 여는 동작이라 인터랙션 frame 경로가 아닙니다. 큰 문서의 실측은 하지 않았습니다.
- 외부 변경 처리는 semantic snapshot 투영 세 번(base·incoming·current)과 문서 모델 두 번(base·incoming)을 만듭니다. 외부 generation 감지 때만 실행합니다.
- `observe`는 표시 중 batch마다 change set과 겹치는 key만 다시 지문을 계산합니다.

## Security

- 출처 색인·이름표·세션 상태는 창 메모리에만 있고 프로젝트 파일, Git, generation metadata, 복구 snapshot, 진단, 텔레메트리에 들어가지 않습니다. 진단은 session id·duration·error code뿐입니다.
- MCP tool에는 출처 표시를 지우거나 되돌리는 수단이 없습니다. 되돌리기는 다른 활성 lease를 침범하지 않습니다(`execute_leased`).
- 종료 절차는 history 확정이 실패해도 lease·권한을 풉니다(권한이 남지 않음).

## Migration

- 기존 데이터 마이그레이션은 없습니다.
