# ADR-0028: 창 session과 셸 명령 C ABI

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §22.2는 같은 프로젝트를 여러 창에서 열 수 있고 각 창이 독립된 인메모리 문서 상태와
Undo/Redo 스택을 가지도록 정합니다. §23.2는 창별로 독립된 history를, §8.2는 handle·POD·버전형 메시지만
쓰는 셸 경계를 요구합니다. 구현 계획 XII.2.1은 코어에 `ProjectWindowSession`(ProjectIdentity,
WindowSessionId, 독립 document state, view state, history, loaded generation)을 두고, XII.2.2는 macOS
`NSWindowController`가 그 handle 하나를 소유하도록 연결합니다.

기존 C ABI(ADR-0009, ADR-0014)는 프로젝트 session(`stellar_project_*`)과 빈 문서 session
(`stellar_document_*`)만 노출합니다. 창 하나가 소유할 편집 session의 handle과 그 상태를 읽는 경로가 없습니다.
구현 계획 1.2의 3번 규칙과 I.3.5 정책에 따라 `crates/ffi/` 변경을 이 ADR에 행별로 기록합니다. 기존 함수·
상태 코드·event의 의미는 바꾸지 않습니다.

## Decision

### XII.2.2 — 창 session handle

1. **코어 표**: `stellar-runtime`의 document thread session 표에 창 session(`ProjectWindowSession`)을
   둡니다. 창 session은 열린 프로젝트 session 아래에서만 열리고, 프로젝트 session을 닫으면 그 아래 창
   session도 폐기합니다. handle은 runtime 정수(`WindowSessionHandle`, 1부터 증가, 재사용 없음)이고, 영속
   식별자 `WindowSessionId`(UUIDv7, 복구 snapshot 키 V.9.1)와 별개입니다.
2. **함수**(모두 기다리지 않고 request id를 돌려줌, 완료는 pull event):
   - `stellar_window_session_open(handle, project, out_request)` → `STELLAR_EVENT_WINDOW_SESSION_OPENED`
     (`0x0106`: request, project, session, window_id\[16\], document_id\[16\], generation) 또는
     `REQUEST_FAILED`.
   - `stellar_window_session_close(handle, session, out_request)` → `STELLAR_EVENT_WINDOW_SESSION_CLOSED`
     (`0x0107`: request, session). 닫으면 저장하지 않은 편집·history·선택을 폐기합니다.
   - `stellar_window_session_info(handle, session, out_info)`: 최신 상태 snapshot에서 고정 88 byte
     `StellarWindowSessionInfo`(version 1: has_document, session, project, revision, generation,
     undo_depth, redo_depth, selection_count, window_id, document_id)를 씁니다. document thread를 기다리지
     않습니다. 없는 session은 `STELLAR_INVALID_ARGUMENT`.
3. **실패 코드**(끝에 덧붙임): `STELLAR_FAILURE_UNKNOWN_WINDOW_SESSION`(4, 문서 session 실패 코드 옆
   빈 번호), `STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED`(18, current generation·작업 문서 로드 실패).
4. **`StellarRuntimeState` version 2**: 끝에 `window_session_count`(u64)를 더해 48 byte가 됩니다. 셸은
   `version == STELLAR_RUNTIME_STATE_VERSION`을 확인합니다.
5. **작업 문서**: 창 session은 current generation의 첫 root 문서 하나를 작업 문서로 로드합니다. 저장한 적
   없는 프로젝트(current pointer 없음)는 작업 문서 없이(document_id 0, generation 0) 엽니다. 문서 전환은
   이 결정의 범위 밖입니다.
6. **셸 소유**: macOS `ShellWindowController.windowSession`이 창 session handle 하나를 소유합니다. 셸은
   프로젝트 session을 연 뒤 창 session까지 열린 다음에 프로젝트 창을 ready로 두고, 창 session이 실패하면
   프로젝트 session을 닫고 열기 실패로 안내합니다. 창을 닫으면 창 session → 프로젝트 session 순서로 한 번씩
   닫습니다.

### XII.2.5 — 창별 탭·active view·viewport

1. **위치**: 열린 탭(표시 순), active 탭, 탭별 viewport(화면 중심 문서 좌표 + 배율, `CanvasViewport`)를
   `stellar-commands`의 `WindowViewState`(선택과 같은 창 view state)에 둡니다. history event가 아니고 저장하지
   않습니다. 배율은 2%\~25600%(`MIN_ZOOM`·`MAX_ZOOM`)로 맞추고 유한 값만 받습니다.
2. **기본 탭**: 창 session이 작업 문서를 로드하면 첫 페이지의 첫 View 탭을 기본 viewport(원점, 100%)로 열어
   active로 둡니다.
3. **runtime 요청**: `open_view_tab`·`close_view_tab`·`set_viewport`는 성공하면
   `STELLAR_EVENT_WINDOW_VIEW_CHANGED`(`0x0108`: request, session)를 보내고 새 값은 상태 snapshot에 둡니다.
   실패 코드 `STELLAR_FAILURE_UNKNOWN_VIEW`(19, 문서에 없는 View·열리지 않은 탭·작업 문서 없음),
   `STELLAR_FAILURE_NO_ACTIVE_VIEW`(20, active 탭 없음)를 덧붙입니다. 이 행에서는 C 함수를 더하지 않고 event
   kind와 실패 코드만 헤더에 노출합니다(셸 viewport 연결은 XII.4.3).

### XII.6.4 — 활성 창 session의 Undo·Redo·Save Now

1. **명령 코드**: `stellar-runtime::app_command::AppCommand`(`STELLAR_COMMAND_UNDO` 1, `_REDO` 2, `_SAVE_NOW` 3,
   끝에 덧붙임). `stellar_window_session_perform(handle, session, command, out_request)`가 명령을 그 창 session
   하나에만 보냅니다. 모르는 코드는 `STELLAR_INVALID_ARGUMENT`(요청 없음).
2. **완료 event**: Undo·Redo는 `STELLAR_EVENT_HISTORY_REPLAYED`(`0x0109`: request, session, redo u8, applied u8,
   revision u64, 재생할 항목이 없으면 applied 0), Save Now는 `STELLAR_EVENT_WINDOW_SESSION_SAVED`(`0x010A`: request,
   session, generation u64, written u8, 바뀐 chunk가 없으면 written 0이고 새 generation을 쓰지 않음).
3. **저장 규칙**: 작업 문서를 직렬화해 loaded generation 위에 SavePlan을 만들고, 작업 문서가 차지하지 않은 base
   chunk는 Clean으로 이어 씁니다. 쓰기 전에 current pointer가 loaded generation과 같은지 확인하고 다르면
   `STELLAR_FAILURE_SAVE_STALE`(21)로 아무것도 쓰지 않습니다(두 선택지 정책은 XII.7). 그 밖의 저장 실패는
   `STELLAR_FAILURE_SAVE_FAILED`(22), history 재생 불가(preview 진행 중 등)는 `STELLAR_FAILURE_HISTORY_UNAVAILABLE`(23).
4. **선택**: `stellar_window_session_select(handle, session, ids, count, out_request)`(StableId 16 byte × count)가 그
   창 선택을 바꾸고 history에 기록합니다(VII.4.6). 완료는 `WINDOW_VIEW_CHANGED`입니다. 캔버스 선택 도구가 생기기 전에는
   셸 검사가 Undo 대상 history를 만들 때 씁니다.
5. **셸**: 메뉴·단축키는 응답자 체인으로 key 창의 router가 받으므로 명령은 언제나 그 창의 창 session으로 갑니다.
   Undo·Redo는 그 창 undo·redo stack이 비어 있지 않을 때, Save Now는 작업 문서가 있을 때만 활성입니다. 저장 실패는 alert
   한 번으로 알립니다.

### XII.6.5 — 코어가 공유하는 command ID와 운영체제별 단축키

1. **공유 이름**: 코어 명령마다 셸 메뉴 명령 식별자와 같은 공유 이름을 둡니다(`edit.undo`, `edit.redo`,
   `file.save_now`, `AppCommand::name`). 코드(`STELLAR_COMMAND_*`)와 이름은 코어가 정합니다.
2. **조회**: `stellar_command_code(name, name_len, out_code)`는 runtime handle 없이 공유 이름의 코드를 씁니다. 코어
   명령이 아닌 이름(셸 전용·미구현)은 `STELLAR_NO_ERROR`와 0입니다. 셸은 앱을 구성할 때 코어 소유 메뉴 명령마다 이
   함수로 물어 command ID 표(`ShellCommandCoreIDs`)를 만들고, 그 표에 있는 ID로만 `stellar_window_session_perform`을
   부릅니다. 셸은 숫자 ID를 직접 적지 않습니다.
3. **단축키**: 단축키는 셸이 운영체제 관례로 정합니다. macOS는 HIG 표준 Undo ⌘Z, Redo ⇧⌘Z, Save Now ⌘S이고(기존
   `ShellCommand.shortcut`, `ShortcutPolicy` 감사), Windows 셸(ADR-0006로 보류)은 같은 공유 이름·코드에 Ctrl+Z,
   Ctrl+Y(보조 Ctrl+Shift+Z), Ctrl+S를 둡니다. 단축키를 바꿔도 command ID는 바뀌지 않습니다.

### XII.4.3 — viewport와 scene camera

1. **camera 입력**: 창 session의 active 탭 viewport(화면 중심 문서 좌표 + 배율)와 캔버스 화면 크기(point)로
   `stellar_scene::visibility::Viewport`(화면 크기 + 문서 → 화면 변환 `view = (world − center) × zoom + size / 2`)를
   만듭니다(`ProjectWindowSession::camera`). 가시 후보 계산(XI.3.2)·hit test·pass 계획이 이 값을 씁니다. 문서 geometry는
   바꾸지 않습니다.
2. **crate 경계**: 이를 위해 `stellar-runtime → stellar-scene` 직접 의존을 허용합니다(`tools/xtask/src/crate_graph.rs`,
   `docs/architecture/crate-dependency-graph.md`). scene은 core-types에만 의존하므로 순환이 없습니다. ADR-0013의 runtime
   허용 의존 목록에 scene을 덧붙이는 결정입니다.
3. **줌 명령**: `STELLAR_COMMAND_ZOOM_IN`(4, ×2), `_ZOOM_OUT`(5, ÷2), `_ZOOM_ACTUAL_SIZE`(6, 100%)는 화면 중심을 고정하고
   배율을 2%\~25600%로 맞춥니다. 공유 이름은 `view.zoom_in`·`view.zoom_out`·`view.zoom_actual_size`이고 완료는
   `WINDOW_VIEW_CHANGED`입니다. 이동은 runtime `pan_viewport`(화면 point 단위)로 합니다(입력 파이프라인 연결은 XII.3 범위).
   유한하지 않은 결과는 `STELLAR_FAILURE_INVALID_VIEWPORT`(24)입니다. 이동·확대는 history에 기록하지 않습니다.
4. **view 정보**: `stellar_window_view_info(handle, session, out_info)`가 고정 56 byte `StellarWindowViewInfo`(version 1:
   has_view, tab_count, center_x, center_y, zoom, active_view)를 씁니다. 셸은 이 배율로 툴바 줌 제목을 보이고, active view가
   있을 때만 줌 명령을 활성화합니다.

## Alternatives

- **프로젝트 session handle에 창 상태를 합침**: 같은 프로젝트를 연 두 창(XII.2.4)이 독립 문서·history를
  가져야 하므로 프로젝트 session 하나에 묶을 수 없습니다. 채택하지 않습니다.
- **창 session 열기를 프로젝트 열기 요청에 포함**: 복원·Locate Folder·다시 시도처럼 같은 창에서 프로젝트를
  다시 여는 경로와 같은 프로젝트의 둘째 창(XII.2.4)이 따로 창 session을 열어야 합니다. 요청을 나눕니다.
- **`StellarRuntimeState`를 그대로 두고 개수 조회 함수를 추가**: 상태 snapshot 요약을 한 번에 읽는 기존
  규칙(ADR-0014)과 맞지 않습니다. version을 올려 필드를 덧붙입니다.
- **영속 `WindowSessionId`를 FFI handle로 사용**: 16 byte id를 매 호출에 넘기게 되고, 닫은 뒤 같은 id를
  다시 쓸 수 있는지 규칙이 필요해집니다. runtime 정수 handle을 쓰고 id는 정보로만 노출합니다.
- **viewport를 runtime 창 session에만 두고 `WindowViewState`와 분리**: 선택과 탭·viewport가 같은 "창 view
  state"(§22.3 복구 대상)인데 위치가 갈리면 복구 snapshot(V.9)과 창 닫기 폐기 규칙을 두 곳에서 맞춰야 합니다.
  같은 type에 둡니다.
- **viewport 변경을 history에 기록**: §23.2는 문서 편집과 선택만 같은 stack에 기록합니다. 이동·확대는
  Undo 대상이 아닙니다.
- **셸이 `STELLAR_COMMAND_*` 상수를 직접 메뉴 명령에 대응**: 두 셸이 같은 대응표를 따로 적게 되고 코어가 명령을
  늘릴 때 셸 표가 어긋납니다. 공유 이름 조회로 코어 표 하나만 기준으로 둡니다.
- **단축키 표를 코어에 둠**: 단축키는 운영체제 관례·지역화 키보드·시스템 단축키 충돌 규칙에 따르는 셸의 몫입니다
  (§8.1 운영체제 네이티브 메뉴). 코어는 명령 의미와 ID만 소유합니다.

## Consequences

- 긍정: 창 controller 하나가 코어 창 session 하나를 소유하는 경로가 생기고, 창마다 문서·history·선택이
  분리됩니다(XII.2.1 검증).
- 긍정: 이후 창별 명령(Undo/Redo/Save Now, XII.6.4)과 view state(XII.2.5)를 같은 handle에 붙일 수 있습니다.
- 부정: 창 session은 열 때 작업 문서 전체를 document thread에서 로드합니다. 큰 문서의 lazy 로드와 비활성
  문서 unload(XVI.5.5)는 창 session에 아직 연결하지 않았습니다.
- 한계: 같은 프로젝트를 두 창에서 여는 셸 정책(XII.2.4)과 저장 충돌 정책(XII.7)은 바뀌지 않았습니다.

## Compatibility

파일 포맷·저장 구조에 영향이 없습니다. C ABI는 함수·event kind·실패 코드를 덧붙이고
`StellarRuntimeState`를 version 2(40 → 48 byte)로 올립니다. 기존 함수 시그니처와 event payload는 그대로이며,
헤더(`cargo xtask generate-ffi-header`)와 Rust 테스트가 크기·offset·상수를 고정합니다.

## Performance

성능 예산(`config/performance-budgets.yaml`)은 바꾸지 않습니다. 정보 조회는 기존 상태 snapshot 읽기와 같은
O(1) 임계 구역입니다. 창 session 열기의 generation·문서 로드는 document thread의 Command 우선순위 job이며
UI thread는 기다리지 않습니다.

## Security

새 외부 의존성과 `unsafe` 패턴이 없습니다(기존 포인터 규칙 그대로). 신뢰되지 않은 프로젝트도 창 session은
저장된 design chunk만 읽고 Git·필터를 실행하지 않습니다.

## Migration

기존 데이터 migration은 없습니다. 셸은 새 헤더로 다시 빌드합니다.

## 정상 창 닫기 저장 보존 — XIII.1.6의 부분 근거

- `StellarWindowSessionInfo`를 version 2로 올립니다. v1의 `reserved`(offset 52)를
  `has_unsaved_changes`(u32, 0/1)로 정의합니다. 크기 88 byte와 나머지 offset은 유지합니다.
  셸은 version을 확인합니다. runtime은 `revision != saved_revision`을 snapshot에 게시합니다.
  선택·viewport는 dirty가 아니며, 실패한 저장은 dirty를 지우지 않습니다. 기존 변경 없는 Save Now의
  O(1) revision 비교와 current manifest stale 검사는 그대로입니다.
- 정상 `NSWindow.performClose`/닫기 버튼/Close Project는 delegate에서 clean이면 즉시 허용하고,
  dirty 또는 해당 창의 미완료 명령이 있으면 창을 유지합니다. 미완료 명령 완료 뒤 dirty를 다시 확인하고
  필요한 경우에만 기존 Save Now 요청을 보냅니다. 저장 성공 event의 session과 최신 clean 상태를 확인한
  뒤 delegate를 다시 거쳐 닫습니다. 새 창별 명령 접수는 이 대기 중 거부합니다.
- 저장 실패(stale 포함), 요청 거부, 정보 조회 실패, 응답 timeout은 실제 창과 메모리 session을 유지합니다.
  sheet로 알리고 사용자가 다시 닫아 재시도할 수 있습니다. 자동 overwrite·자동 discard·재오픈은 없습니다.
  timeout/취소/실제 닫힘/session 교체 뒤 늦은 callback은 자동 닫기를 실행하지 않습니다.
- 셸 응답 watchdog은 10초입니다. 이는 무기한 자동 닫기 대기를 막는 임시 correctness 상한이며
  `AutosavePolicy`의 성능 설정값이나 XIII.1.1의 벤치마크 확정값이 아닙니다. config 누락을 기본값으로
  대체하지 않으며 config와 성능 예산은 변경하지 않습니다. timeout은 Rust 작업 취소를 뜻하지 않습니다.
  이미 접수된 저장이 나중에 끝날 수 있으나, 그 응답으로 창을 닫지는 않습니다.
- GUI thread는 run loop timer와 비동기 event만 사용합니다. 직렬화·쓰기의 기존 document thread 실행은
  그대로이며 worker 이동·snapshot 캡처·autosave 병합(XIII.1.3~.5), 앱 비활성화 flush와 앱 종료는 미구현입니다.
  따라서 XIII.1.6은 완료 처리하지 않습니다.
- Debug에만 `stellar_window_session_check_edit` 검사 심벌을 추가합니다. 기존 저장 프로젝트 fixture의
  node에 실제 `CommandTransaction`을 적용하거나 이름을 검증하며, 제한된 지연(최대 500ms)을 document queue에
  넣습니다. Release에는 심벌이 없고 Swift 호출도 `#if DEBUG` 안입니다. 파일 쓰기·stale·저장 event는
  제품 경로를 그대로 검사합니다. 헤더 생성기가 검사 선언도 소유합니다.
- `stellar-ops::lock::SaveLock`은 headless writer용 비대기 advisory lock입니다. runtime의 기존 Save Now는
  이 lock을 획득하지 않습니다. 같은 runtime의 독립 창은 단일 document thread와 저장 전 stale 검사로
  순서가 정해지지만, 별도 앱·CLI의 검사/쓰기 간 경쟁은 이 변경에서 해결하지 않습니다. 공통 lock 경로와
  모든 writer 적용은 별도 통합이 필요합니다.

검증·남은 범위는 [정상 창 닫기 보고서](../task-reports/XIII.1.6.md)를 따릅니다.
