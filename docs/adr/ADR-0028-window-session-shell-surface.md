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
   창 선택을 바꾸고 history에 기록합니다(VII.4.6). 완료는 `WINDOW_VIEW_CHANGED`입니다. 이동 도구는 창이 표시한 scene에서
   클릭한 Frame의 StableId를 보내고 빈 영역을 누르면 선택을 비웁니다. 제한 모드에서도 선택할 수 있습니다.
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
   Zoom 도구의 `stellar_window_session_zoom_at`은 화면 중앙에서 측정한 logical point를 받습니다.
   클릭은 그 점의 문서 좌표를 고정해 메뉴와 같은 한 단계로 확대·축소하고, 영역 드래그는 선택 사각형의
   중앙을 화면 가운데에 놓고 양 축이 들어오는 배율을 적용합니다. 기존 viewport 배율 상하한을 따릅니다.
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
  사용자가 고르기 전에는 자동 overwrite·자동 discard·재오픈이 없습니다. 고르는 방법은 아래
  '창 수명·복구 정리'의 닫기 실패 선택 시트를 따릅니다.
  timeout/취소/실제 닫힘/session 교체 뒤 늦은 callback은 자동 닫기를 실행하지 않습니다.
- 셸 응답 watchdog은 10초입니다. 이는 무기한 자동 닫기 대기를 막는 임시 correctness 상한이며
  `AutosavePolicy`의 성능 설정값이나 XIII.1.1의 벤치마크 확정값이 아닙니다. config 누락을 기본값으로
  대체하지 않으며 config와 성능 예산은 변경하지 않습니다. timeout은 Rust 작업 취소를 뜻하지 않습니다.
  이미 접수된 저장이 나중에 끝날 수 있으나, 그 응답으로 창을 닫지는 않습니다.
- GUI thread는 run loop timer와 비동기 event만 사용합니다. 직렬화·쓰기의 기존 document thread 실행은
  그대로이며 worker 이동·snapshot 캡처·autosave 병합(XIII.1.3~.5)과 앱 비활성화 flush는 미구현입니다. 앱 종료(⌘Q)
  저장은 아래 '닫기 실패 선택 시트와 앱 종료'를 따릅니다. 따라서 XIII.1.6은 완료 처리하지 않습니다.
- 저장을 기다리는 동안 같은 창에 다른 시트가 붙으면 닫기 요청을 시트가 끝날 때까지 보관하고, 원래 창과 session의
  편집 상태를 다시 확인한 뒤 닫습니다. 시트 응답 시간은 응답 watchdog에 넣지 않습니다.
- 저수준 `NSWindow.close()`와 `stellar_window_session_close`는 저장하지 않는 명시적 폐기 API로 유지합니다. 사용자의
  정상 닫기(닫기 버튼, Close Project, 메뉴)는 `performClose`를 거쳐 위 저장 확인을 받습니다.
- 세대를 기록하는 Save Now는 같은 OS 사용자의 `stellar apply`와 `stellar-ops::lock::SaveLock`을 공유합니다.
  열린 프로젝트 ID와 symlink를 해석한 target으로 잠금을 잡고, 같은 경로의 current manifest를 확인한 뒤
  pointer 교체와 session 갱신까지 보유합니다. 경합이나 잠금 준비 실패는 SaveFailed로 반환하며 편집을 보존합니다.
  작업 문서가 없거나 저장 bytes가 같으면 잠금을 만들지 않습니다. 기존 세대가 있는 문서는 무변경 저장에서도
  current manifest를 확인합니다. Git checkout 등 외부 변경이 base 검사 전에 끝나면 stale을 반환합니다. 외부 변경의 reconciliation은 XXIV.4.6 범위입니다.

## 창 수명·복구 정리 (2026-11-15 검토)

2026-11-15 검토에서 창 닫기·앱 종료·복구 경로의 C ABI를 다음과 같이 정리합니다.

### Debug 전용 창 편집 검사 ABI 삭제

- `stellar_window_session_check_edit`와 runtime `AppRuntime::check_window_edit`, Swift `checkWindowEdit`를
  삭제합니다. 이 함수는 검사만을 위해 공개 헤더와 runtime에 들어간 편집·이름 검증·지연 주입 경로였고,
  Release와 Debug의 헤더가 달라지게 했습니다.
- 헤더의 `#if defined(DEBUG) && DEBUG` 블록과, 그 블록 때문에 Debug 구성에 넣은 `GCC_PREPROCESSOR_DEFINITIONS`의
  `DEBUG=1`을 함께 삭제합니다. 이제 공개 헤더에는 구성별 조건 선언이 없습니다. `bundle_check.sh`는 헤더를
  C 전처리한 결과에서 함수 목록을 뽑는 방식을 유지합니다(`#ifdef __APPLE__` 선언 판정).
- 창 닫기 검사(`--window-close-check`)는 제품 편집 경로인 `stellar_window_layer_rename`으로 dirty 상태를 만들고,
  정상 닫기와 앱 종료(⌘Q) 저장만 확인합니다. 저장된 이름은 다시 연 창의 `stellar_window_layer_page_with_history`로
  읽습니다.

### 닫기 실패 선택 시트와 앱 종료

- 정상 닫기나 앱 종료(⌘Q)에서 저장이 실패하면(쓰기 실패, stale, 접수 실패, 상태 조회 실패, 응답 timeout)
  창에 선택 시트를 띄웁니다(아키텍처 §22.2의 선택 팝업, 2026-11-15 사용자 결정 D6). 선택지는 셋입니다.
  - 다시 시도: 같은 닫기 경로로 Save Now를 다시 요청합니다. 에이전트 확인은 다시 묻지 않습니다.
  - 저장하지 않고 닫기: 저수준 `NSWindow.close()`로 창을 닫습니다. 창 session을 닫으면서 메모리의 편집과
    그 창의 복구 체크포인트를 버립니다. 디스크의 프로젝트 파일은 바꾸지 않습니다.
  - 취소: 창과 편집을 그대로 둡니다.
- 앱 종료는 `applicationShouldTerminate`에서 `terminateLater`를 돌려주고, 저장이 필요한 창을 하나씩 위 닫기 저장
  경로로 저장합니다. 모든 창이 저장되었거나 사용자가 저장하지 않고 닫기를 고른 뒤에만 종료를 승인합니다.
  한 창에서 취소를 고르면 종료를 취소하고 나머지 창도 그대로 둡니다. 실제 창은 종료 승인 전에 닫지 않습니다
  (저장하지 않고 닫기를 고른 창만 닫습니다).
- 이 결정은 셸 동작이며 C ABI를 더하지 않습니다. 기존 Save Now·창 session 닫기 ABI를 그대로 씁니다.

### 비정상 종료 복구 체크포인트

U125가 ADR 없이 넣은 복구 경로를 아키텍처 §22.3 범위(Restore·Discard·Later)로 줄이고 여기에 기록합니다.

1. **저장 위치·형식**: V.9.1 `RecoverySnapshotStore`(OS 앱 데이터의
   `recovery/<project identity hash>/<window uuid>.starrecovery`, identity 디렉터리 권한 0700)와 V.9.2
   `RecoveryPayload`(활성 문서의 전체 chunk, 기준 manifest hash, revision, 탭·viewport·선택)를 그대로 씁니다.
   프로젝트의 Git 작업 트리와 index에는 쓰지 않습니다. Undo/Redo history는 담지 않습니다.
2. **캡처 시점**: 상태를 게시할 때 document thread는 문서 revision·dirty 표식만 남깁니다(O(1)). 창별 worker가 마지막
   표식 뒤 1초(편집이 이어져도 첫 표식 뒤 5초 안)를 기다린 뒤 캡처 job을 document thread의 background lane에 한 번
   제출합니다. 입력·일반 command job이 먼저 처리됩니다. 직렬화와 원자적 파일 교체는 worker에서 합니다. 선택·viewport만
   바뀐 경우는 기록하지 않고, 저장되거나 폐기되면 복구본을 지웁니다. 1초·5초는 XIII.8.1 벤치마크 전의 임시
   correctness 값이며 config와 성능 예산은 바꾸지 않습니다.
3. **측정**(2026-11-15, DEV-MAC, release, 한 부모 아래 평평한 Frame 문서, 5회 중 최솟값, REF-MAC 판정 아님): 한 번
   캡처의 `to_records` 비용은 1,000 node 0.14ms, 10,000 node 1.7ms, 50,000 node 12.6ms입니다. 이전에는 이 비용이
   편집 요청마다 들었고 지금은 debounce 구간마다 한 번입니다. 약 1만 node를 넘는 문서에서는 한 번의 캡처가 document
   thread 2ms 상한을 넘습니다. autosave와 같은 immutable snapshot을 공유하는 XIII.8.2 전까지 남는 한계입니다.
4. **잠금·정리**: worker는 처음 쓸 때 자기 `<window uuid>.lock`을 OS 파일 잠금으로 쥡니다. 다른 프로세스·창은 잠긴
   복구본을 제안하지 않습니다. worker가 끝날 때 남긴 복구본이 없으면 잠금 파일도 지웁니다. runtime 파괴는 document
   thread를 멈춘 뒤 worker의 남은 정리를 최대 1초(`AppRuntime::RECOVERY_FLUSH_TIMEOUT`) 기다립니다.
5. **제안 규칙**(V.9.3): 기준 manifest hash가 디스크의 current와 같고 revision이 0보다 큰 복구본만 제안합니다. 다른
   살아 있는 창이 잠근 것, 해석할 수 없는 것(사유만 기록), 저장한 적 없는 기준, 뒤에 정상 저장되어 current가 바뀐 것은
   제안하지 않습니다. 최근 창의 복구본부터 하나만 고릅니다.
6. **C ABI**
   - `StellarRecoveryCandidate`(고정 56 byte, `STELLAR_RECOVERY_CANDIDATE_VERSION` = 1): version, has_candidate,
     window_id\[16\], content_hash\[32\](복구본 파일 bytes의 SHA-256). 크기·offset은 Rust 테스트와 헤더
     `static_assert`로 고정합니다.
   - `stellar_window_recovery_probe(handle, session, *out_candidate)`: 파일을 읽으므로 UI thread 밖에서 부릅니다. 없으면
     `has_candidate = 0`과 `OK`, 복구 저장소를 읽지 못하면 `INTERNAL`과 last error, 없는 창 session은
     `INVALID_ARGUMENT`, app runtime이 시작되지 않은 handle은 `INVALID_HANDLE`입니다.
   - `stellar_window_recovery_apply(handle, session, *candidate, action, *out_request)`: action은
     `STELLAR_RECOVERY_RESTORE`(1) 또는 `STELLAR_RECOVERY_DISCARD`(2)입니다. worker에서 같은 bytes hash인지 다시 확인한
     뒤에만 적용합니다. Restore는 창이 같은 프로젝트·같은 문서이고 revision 0·미저장 변경 없음이며 loaded generation과
     디스크 current가 복구본의 기준과 같을 때만 문서·탭·viewport·선택을 메모리에 복원합니다. 디스크는 쓰지 않고, 다음
     Save Now나 정상 닫기가 일반 저장으로 기록합니다. 원래 복구본은 이 창이 새 복구본을 쓰거나 지울 때까지 보존합니다.
     Discard는 복구본과 그 잠금 파일을 지웁니다. 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`
     (`WINDOW_SESSION_LOAD_FAILED` 18, 창·디스크 기준이 바뀌었으면 `SAVE_STALE` 21)입니다. 모르는 action,
     `has_candidate = 0`, 다른 version은 `INVALID_ARGUMENT`이며 요청을 만들지 않습니다.
   - Later는 요청이 없습니다. 복구본을 그대로 두어 다음 열기에 다시 제안합니다.
7. **셸**: 창 session을 연 뒤 worker에서 probe하고, 복구본이 있으면 Restore Changes·Discard Changes·Later 시트를
   띄웁니다. 그 창에 이미 미저장 편집이 있으면 Restore를 보이지 않습니다.
8. **제거**
   - 문자열 ticket 프로토콜(첫 byte 태그와 `<uuid> <hash>` 텍스트, 호출자 buffer 출력): 고정 구조체로 바꿨습니다.
   - Save Recovered Document…와 runtime `export_recovery`: 기준이 바뀐 복구본을 원본 밖 새 폴더에 별도 Git 저장소로
     내보내는 경로입니다. §22.3에 없는 기능이고 새 프로젝트 생성 경로를 중복 구현했습니다. 기준이 바뀐 복구본은
     위 제안 규칙대로 제안하지 않습니다.
   - 앱 검사 `ShellDocumentRecoveryCheck`(`--document-recovery-check`)와 `scripts/macos/document-recovery-check.py`:
     앱 안에 넣은 원격 조작 검사입니다. 코어 규칙은 runtime 단위 테스트가 다룹니다.

## 스크롤의 창별 팬 연결 — XII.10.37 부분 구현

2026-11-15 검토에서 셸이 스크롤을 해석하던 구조(ADR-0014 XII.3.2 결정 2 위반, XII.3.4 delta 병합 우회,
접수 거부 시 delta 유실)를 아래처럼 코어로 옮겼습니다.

- 셸 `CanvasView.scrollWheel`은 `StellarMacEvent`를 복사해
  `stellar_window_session_submit_mac_event(handle, session, event, out_request)`로 그 창 session에 넘깁니다.
  창 session이 없으면 기존 `stellar_runtime_submit_mac_event`로 보냅니다.
- 코어는 이벤트를 runtime 시간 domain의 공통 입력으로 바꾼 뒤(`stellar_runtime_submit_mac_event`와 같은 규칙),
  Control·Option·Command가 없는 wheel·trackpad 스크롤을 그 창의 팬으로 처리합니다. Shift만 누른 휠은 시스템이
  가로 delta로 바꿔 주므로 같은 규칙의 가로 팬입니다(ADR-0071 UX.3.1에서 추가). delta 단위가
  point(precise scrolling)면 그대로, 줄(일반 휠)이면 한 줄을 `WHEEL_LINE_POINTS`(10pt)로 환산하며, 공통 wheel
  delta(scroll 양)의 반대 방향이 내용 이동입니다. 관성 단계도 같은 규칙입니다.
- 팬은 창별 대기 합으로 모읍니다(`AppRuntime::wheel_pan`). 합을 가져갈 job이 이미 대기 중이면 새 요청을 만들지
  않고 `out_request`는 0입니다. job은 실행 시점의 합 전체로 한 번 이동하고 `WINDOW_VIEW_CHANGED`를 보냅니다.
  접수가 거부되면(`STELLAR_QUEUE_FULL`) 합을 버리지 않고 다음 스크롤 때 함께 제출합니다. 이동량이 0이면 요청이
  없습니다.
- 그 밖의 이벤트(Control·Option·Command 스크롤 포함)는 runtime 입력 대기열에 넣고 `out_request`는 0입니다.
  ⌘ 스크롤 줌과 핀치는 셸이 이 함수 대신 `stellar_window_session_zoom_by`로 보냅니다(ADR-0071 UX.3.1).
- 셸은 접수된 팬 요청의 완료 event를 받은 뒤 표시 목록을 다시 읽고, 창 닫기는 그 완료를 기다립니다. 진행 중인
  Frame 드래그 미리보기는 화면 좌표라 스크롤이 오면 먼저 취소합니다.
- `stellar_window_session_pan(handle, session, dx, dy, out_request)`은 Hand 도구·Space 임시 Hand 드래그 전용으로
  남깁니다(화면
  point 내용 이동, 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`).
- 검증은 Rust 테스트로 합니다: runtime `wheel_pan_merges_pending_scroll_and_keeps_a_refused_delta`, C ABI
  `core_interprets_scroll_as_a_pan_of_only_the_target_window`.

### 저장된 Frame의 이동

기본 창의 이동 도구는 표시된 node를 드래그할 때 해당 창 scene에서 그 하위 트리를 임시 이동합니다.
마우스를 놓으면 선택 전체 이동(`stellar_window_canvas_move`, ADR-0071 UX.3.7)으로 보냅니다. 처음 기록한 단일
Frame 이동 `stellar_window_session_move_frame`은 같은 이동의 중복 경로라 2026-11-15 검토에서 삭제했습니다.
Escape·창 포커스 상실로 미리보기를 취소하면 문서 변경이 없습니다. 창의 Undo·Redo 및 Save Now가
이 transaction을 처리합니다.

### 캔버스 직접 조작 함수(ADR-0071)

모두 창 session handle을 받고, 요청형 함수의 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다. 의미는 ADR-0071에
있습니다.

| 함수 | 역할 |
|---|---|
| `stellar_window_session_zoom_by` | 화면 점을 고정한 연속 줌(핀치, ⌘ 스크롤). 문서·history 불변 |
| `stellar_window_canvas_pick` | 포인터 아래 행에서 선택. flag `STELLAR_PICK_TOGGLE`·`_DEEP`·`_ENTER`·`_PRESS`·`_RELEASE` |
| `stellar_window_canvas_pick_area` | world 영역 선택. flag `STELLAR_AREA_CONTAIN`·`_ADD` |
| `stellar_window_canvas_move` | 선택 전체 이동·Option 복제. flag `STELLAR_MOVE_DUPLICATE`·`_PARENT_SPACE` |
| `stellar_window_canvas_resize` | 하나뿐인 선택의 크기 핸들(속성 탭과 같은 longhand `SetProperty`) |
| `stellar_window_canvas_line_end` | Line 끝점 핸들(`x1`·`y1` 또는 `x2`·`y2`) |
| `stellar_window_canvas_radius` | 반경 핸들(`border-*-radius`, Polygon 반경) |
| `stellar_window_canvas_selection` | 최신 snapshot의 선택 상자 `StellarCanvasSelectionItem`과 flag `STELLAR_CANVAS_ITEM_*` |

`stellar_window_session_insert_shape`는 끝에서 두 번째 인자로 `StellarShapeStyle`을 받습니다(ADR-0071 UX.1.5).
헤더의 상수 값과 함수 선언은 `crates/ffi` 헤더 테스트가 Rust 상수와 대조합니다.

호스트 검사 `scripts/macos/canvas-move-check.py`는 CLI로 부모 2배 변환과 자식 Frame의 위치를 저장하고,
실제 기본 창에 합성 AppKit 입력을 보냅니다. 제한 모드 이동과 Undo(D1), 50% 배율에서의 이동량, 미리보기·Escape 원복
픽셀, 다른 창 상태, Undo·Redo와 Save Now 뒤 재열기 좌표, 여러 node를 고른 채 끌기·Option 복제·클릭으로 하나 고르기를
확인합니다.

### 자식 Free Frame 생성

`stellar_window_session_insert_frame`은 누른 부모 Frame의 ID, 표시한 View·revision과 부모 local rect를
받습니다. document thread가 창의 실제 View·revision·부모 잠금 및 지원하는 자유 배치 여부를 확인하고,
마지막 형제 뒤에 `InsertNode`를 기록합니다. 이전 선택과 생성된 Frame의 선택을 같은 history entry에 보관하므로
Undo 한 번은 node와 선택을 함께 되돌리고 Redo는 같은 ID를 복원합니다. 화면의 드래그 미리보기는 저장되지 않습니다.

도형 도구는 `stellar_window_session_insert_shape`에 저장 포맷의 ShapeKind tag와 부모 local 시작·끝 두 점을 전달합니다.
Ellipse와 Polygon은 좌상단을 Transform에, 로컬 원점 기준 크기를 Shape에 저장합니다. Line은 시작점을 Transform에, 부호 있는 끝점 차이를 Shape에 저장해 드래그 방향을 보존합니다.
Frame ABI와 같이 document thread가 표시 revision·편집 가능한 부모를 확인하며 생성과 선택을 한 번에 기록합니다.

### 기본 창의 레이어 읽기

`stellar_window_layer_page`는 셸 worker에서 창 handle·활성 View·표시 revision·부모·직전 자식 ID로 직계 자식 64개까지 읽습니다. 코어는 document thread에서 해당 창이 이미 연 문서를 조회합니다. cursor의 부모와 활성 View 소속을 확인하고 순서 인덱스에서 다음 형제부터 반환합니다. 반환값은 Page·View 이름, 원본 노드 종류·표시 이름·잠금·표시 상태, 자식 유무와 창 선택입니다. registry 잠금은 읽기 접수까지만 유지합니다. UI thread는 저장 폴더를 열거나 조회 완료를 기다리지 않습니다.

레이어 이름 편집은 stellar_window_layer_rename에 창·View·표시 revision·노드 ID와 UTF-8 이름을 전달합니다. 코어가 표시 revision, View 소속 및 조상 잠금을 확인하고 기존 RenameNode를 창의 문서 transaction으로 실행합니다.

### 레이어 표시·잠금 변경 — UX.3.4

- `stellar_window_layer_set_flag(handle, session, view, revision, nodes, count, flag, value, out_request)`는
  레이어 행에서 고른 노드들의 `node.visible`(`STELLAR_LAYER_FLAG_VISIBLE`) 또는 `node.locked`
  (`STELLAR_LAYER_FLAG_LOCKED`)을 `value`(0·1)로 바꿉니다. document thread가 창, 표시 revision, 활성 View 소속,
  중복을 확인하고 기존 `SetProperty::visibility`·`SetProperty::locked`를 창의 문서 transaction 하나로 기록합니다.
  값이 이미 같은 노드는 건너뛰고, 바뀌는 노드가 없으면 기록하지 않습니다.
- 표시·잠금은 문서 편집이므로 프로젝트 신뢰를 요구하지 않습니다(D1). `stellar_window_layer_set_flag`와
  `STELLAR_SELECTION_TOGGLE_*`는 신뢰하지 않은 프로젝트에서도 실행됩니다. 다른 선택 편집도 정리 통합(편집 경로
  신뢰 게이트 제거)으로 신뢰와 무관합니다.
- 레이어 행 플래그(`StellarLayerRow.flags`)에 bit 3(표시 변경 가능)·bit 4(잠금 변경 가능)를 더합니다. 코어가 잠금
  규칙으로 계산하며 셸은 행 토글 버튼의 활성에만 씁니다. 구조체 크기와 기존 bit 0–2는 그대로입니다.
- `stellar_window_edit_selection`의 `STELLAR_SELECTION_TOGGLE_VISIBILITY`(7)·`STELLAR_SELECTION_TOGGLE_LOCK`(8)은
  메뉴 막대와 캔버스 문맥 메뉴의 Hide·Lock 명령입니다. 표시한 선택 전체가 켜져 있으면 끄고, 하나라도 꺼져
  있으면 모두 켭니다. 값은 실행 시점 문서에서 코어가 정하므로 셸이 선택 행을 모두 읽어 둘 필요가 없습니다.
- 잠금 규칙(D8): 잠긴 조상 아래의 노드는 표시·잠금을 바꾸지 않고, 표시 변경은 노드 자신의 잠금도 거부합니다.
  잠금 해제는 그 노드의 잠금 변경으로만 합니다. 캔버스는 기존대로 숨긴 노드를 그리지 않고 잠긴 노드와 그 자손을
  hit 대상에서 뺍니다.
- 기존 구조체·입력 encoding·저장 포맷은 바꾸지 않습니다. 거부는 `REQUEST_FAILED`(`UNKNOWN_VIEW`)로 옵니다.

### 선택 능력 mask와 문맥 메뉴 선택 — UX.3.2·3.4

- `stellar_window_selection_flags(handle, target)`는 `stellar_window_layer_order_flags`처럼 같은 게시 시점(창·
  활성 View·revision·history sequence·선택 수가 모두 같음)의 선택에 대한 mask를 상태 snapshot에서 읽고, 시점이
  다르면 0을 돌려줍니다. bit는 `STELLAR_SELECTION_CAN_DELETE`(1)·`CAN_DUPLICATE`(2)·`CAN_COPY`(4)·`CAN_PASTE`(8)·
  `CAN_HIDE`(16)·`CAN_LOCK`(32)과 토글 방향 `STELLAR_SELECTION_SHOWS`(64, 표시 토글이 보이게 함)·
  `STELLAR_SELECTION_UNLOCKS`(128, 잠금 토글이 풂)입니다. 실행 경로와 같은 규칙 함수(View root 제외, 자신·조상·
  자손 잠금, 붙여넣을 부모)로 계산하므로 셸은 메뉴 막대·캔버스 문맥 메뉴·레이어 문맥 메뉴의 활성과 제목을 이
  값 하나로 정합니다. 클립보드에 노드가 있는지는 셸이 확인합니다. 신뢰는 mask에 넣지 않습니다(D1).
- 삭제 가능 여부는 선택 subtree를 훑으므로 창마다 마지막 (revision, 활성 View, 선택) 결과를 캐시하고, 셋 중
  하나가 바뀔 때만 다시 계산합니다. 카메라 이동 같은 게시에는 다시 훑지 않습니다.
- `stellar_window_context_select(handle, target, node, out_request)`는 캔버스 우클릭의 선택을 코어가 정합니다.
  누른 노드나 그 조상(View root 제외)이 실행 시점의 전체 선택에 있으면 선택을 그대로 두고, 아니면 그 노드만
  선택합니다. `node`가 null(빈 곳)이면 선택을 비웁니다. target 시점이 다르거나 노드가 활성 View에 없으면
  `REQUEST_FAILED`(`UNKNOWN_VIEW`)입니다. 선택 변경은 기존 선택 history 규칙(같은 선택이면 기록 없음)을 따르고,
  셸은 완료 event 뒤에 메뉴를 엽니다. 누를 노드는 셸 hit 결과이며, 클릭 선택 규칙(같은 깊이 선택)이 코어에
  들어오면 그 규칙으로 고른 노드를 같은 진입점에 넘깁니다.

### 사이드바 머리 줄 Page › View — UX.3.5

- `stellar_window_view_outline(handle, session, rows, row_capacity, text, text_capacity, out_rows, out_text_len)`는
  셸 worker에서 창 작업 문서의 Page·View를 정의 순서로 읽습니다. 행(`StellarViewOutlineRow`, 48 byte)은 View마다
  하나이고 View 없는 Page는 `view`가 0입니다. UTF-8 text는 행 순서로 Page 이름(그 Page의 첫 행만)과 View 이름이며,
  공간이 부족하면 `BUFFER_TOO_SMALL`과 필요한 행 수·길이를 씁니다. 창이 없거나 작업 문서가 없으면 `NO_ERROR`입니다.
  registry 잠금은 읽기 접수까지만 유지하고 UI thread는 기다리지 않습니다.
- `stellar_window_session_open_view(handle, session, view, out_request)`는 기존 runtime `open_view_tab`을 호출해
  작업 문서의 View 탭을 열고 active로 만듭니다(이미 열려 있으면 active만). 문서 history에 기록하지 않으며 완료는
  `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다.
- Page·View 이름 변경과 추가는 코어 Command가 없어 이 ABI에 넣지 않습니다(UX.12.1에서 Command와 함께 정합니다).

### Frame Inspector의 개별 모서리 반경

이 절의 `stellar_window_selected_frame_corners`·`stellar_window_session_set_frame_corner`는 2026-11-15 검토에서
지웠습니다. 모든 node 종류의 행 읽기와 편집(4모서리 `border-*-radius` longhand 포함)은 ADR-0070의
`stellar_window_inspector`·`stellar_window_edit_property`가 대체합니다. 모서리 네 값을 원본 그대로 보존하고 캔버스·
hit test에는 bounds 기준 정규화 반경을 쓰는 규칙은 그대로입니다.

## 창 session 자동 저장 상태 — UX.2.11 (ADR-0072)

- `StellarWindowSessionInfo`를 version 3으로 올립니다. 끝(offset 88)에 `autosave`(u32)와 `reserved`(u32, 0)를 더해
  88 → 96 byte가 됩니다. 앞 필드의 offset은 그대로이고 셸은 version을 확인합니다.
- `autosave` 값: `STELLAR_AUTOSAVE_IDLE`(0, 저장할 완료 편집 없음, 자동 저장을 켜지 않은 runtime 포함),
  `_PENDING`(1, 저장 기한을 기다림), `_SAVING`(2, writer가 쓰는 중), `_FAILED`(3, 마지막 자동 저장 실패, 다시 시도를
  기다리거나 쓰는 중이며 성공하거나 clean이면 풀림), `_STALE`(4, 다른 창·프로세스가 먼저 저장해 멈춤, clean이 될 때까지
  쓰지 않음). 겹치면 stale → 실패 → 쓰는 중 → 대기 순입니다.
- 자동 저장이 디스크에 쓰면 같은 info의 `generation`과 `has_unsaved_changes`가 바뀝니다. 셸은 `autosave`가
  `PENDING`·`SAVING`·`FAILED`인 동안 `stellar_runtime_state`의 상태 세대를 낮은 주기로 읽어 이 변화를 관찰합니다(callback
  없음, ADR-0072 7절).
- Save Now(`STELLAR_COMMAND_SAVE_NOW`)는 그 창을 포함한 진행 중인 자동 저장 쓰기를 먼저 기다려 반영한 뒤 남은 편집만
  저장합니다. 기다림이 `window_close_flush_timeout_ms`를 넘으면 `STELLAR_FAILURE_SAVE_FAILED`입니다. 기존 저장 규칙과
  실패 코드는 그대로입니다.
- 앱 비활성화 flush `stellar_runtime_flush_autosave`의 기록은 ADR-0014 XII.6 10번과 ADR-0072입니다.
- 통합 때 UX-E의 Debug 전용 `stellar_runtime_check_disable_autosave`를 살리려고 헤더의 `#if defined(DEBUG) && DEBUG`
  블록과 Debug 구성의 `DEBUG=1`이 되살아났습니다. 위 "Debug 전용 창 편집 검사 ABI 삭제" 결정(구성마다 같은 공개
  헤더)을 지키려고 셋을 다시 지웠습니다. 검사 실행에서 자동 저장을 끄는 방법은 Debug 코어가 앱 runtime을 시작할 때
  읽는 검사 전용 환경 변수 `STELLAR_CHECK_DISABLE_AUTOSAVE=1`입니다(ADR-0072 10절). Release 코어는 이 변수를 읽지
  않습니다.

## 컴포넌트 라이브러리 창 계열 ABI 제거 (2026-11-15 검토)

사용자가 정의하지 않은 Component Library 창(U099)과 그 위의 크기·채우기 편집(U104·U119), 숫자 좌표
인스턴스 배치(U114), 빈 컴포넌트 생성(U097)은 이 ADR에 기록되지 않은 채 `crates/ffi`를 넓혔습니다.
2026-11-15 검토에서 반려해 앱 진입점과 함께 아래 ABI를 삭제합니다. 앱에서 컴포넌트를 찾고 배치하고
편집하는 UX는 사용자 결정(D4) 뒤 새 ABI로 다시 정합니다. 그동안 원본 조회와 편집은 CLI(`stellar query
<경로> components`, `component-source resize·set-fill·add-frame`)와 MCP `stellar://components`가 맡습니다.

- `stellar_window_component_library`, `stellar_window_component_library_with_fill`: 라이브러리 창 전용
  행 목록 조회입니다. 창과 함께 삭제했습니다.
- `stellar_window_component_resize`, `stellar_window_component_set_fill`: 라이브러리 창의 root 크기·채우기
  편집입니다. Document Thread와 history를 우회해 Undo가 없고 모든 실패를 `InvalidArgument` 하나로 돌려줬습니다.
  runtime `resize_component_root`·`set_component_root_background`, ops·project의 `resize_project_root`·
  `edit_project_root`와 함께 삭제했습니다. 이 경로만 쓰던 `crates/storage`의 `source_file::write_checked_resolved`도
  삭제하고 `write_new_resolved`를 새 파일 전용 본문으로 되돌렸습니다. CLI `component-source resize·set-fill`이
  쓰는 단일 파일 `edit_root`는 유지합니다.
- `stellar_window_place_component`: 라이브러리 창의 숫자 X·Y 시트로 인스턴스를 배치하는 요청입니다. 컴포넌트
  생성과 같은 진행 플래그를 써서 서로를 거부했습니다. runtime `place_component_instance`(`instance_place.rs`)와
  창 전용 조회 `ProjectSummary::component_library`도 삭제했습니다. 정의 hash를 확인하는 한 transaction 삽입은
  `stellar apply`의 `insert-component-instance` command로 옮겼습니다(`stellar_ops::apply`).
- `stellar_window_component_create`: 정의되지 않은 **Insert → New Component…** 메뉴가 크기 0×0인 빈 컴포넌트를
  만드는 요청입니다. runtime `create_component` 래퍼와 함께 삭제했습니다. 선택에서 만드는
  `stellar_window_component_from_selection`은 사용자 결정 D3(당분간 제거)에 따라 B4에서 삭제했습니다(아래
  "선택에서 컴포넌트 만들기 제거"). 빈 컴포넌트는 CLI `component-source create`로 만듭니다.
- `stellar_window_selected_definition`: Component Library 창을 열 때 선택한 인스턴스의 정의 행으로 이동하려고
  창 상태 요약에 넣은 정의 ID 조회입니다. 창이 없어져 소비자가 없으므로 runtime 요약 필드
  `selected_definition`과 함께 삭제했습니다.

이 계열에서 남는 창 ABI는 캔버스 재읽기(`stellar_window_reload_component_canvas`)입니다. 원본 감시 fd
(`stellar_window_component_watch_fd`)는 삭제했습니다. 재읽기와 감시의 기록은 아래 "캔버스 표시·입력·감시 ABI 기록"에
있습니다.

## 프로젝트 신뢰 확인·승인·철회 — XVI.7.2 (2026-11-15 소급 기록)

U051이 이 ADR 없이 추가한 ABI를 기록합니다. 의미는 아래와 같고 시그니처는 바꾸지 않았습니다.

- `stellar_project_trust(handle, project, action, out_request)`: action 0 조회, 1 사용자 승인, 2 사용자 철회입니다.
  다른 값은 요청 없이 `INVALID_ARGUMENT`입니다. 완료는 `STELLAR_EVENT_PROJECT_TRUST`(`0x010B`: request u64,
  project u64, trusted u8) 또는 `REQUEST_FAILED`입니다. 신뢰 저장소를 열거나 기록하지 못했거나 폴더·ID가 바뀌었으면
  실패 코드 25(`TRUST_UNAVAILABLE`), 닫힌 프로젝트는 1입니다.
- 기록 위치는 OS 사용자 데이터의 신뢰 목록(`FileTrustStore`)뿐입니다. 프로젝트 파일과 Git config는 바꾸지 않습니다.
  조회와 승인은 열린 폴더의 실제 target과 매니페스트 project ID가 열 때와 같은지 다시 확인합니다.
- 프로젝트는 항상 신뢰되지 않음으로 엽니다(ADR-0014 결정 4). 승인 결과는 요청한 프로젝트 session에만 반영합니다.
- 결과가 신뢰가 아니거나(철회 포함) 확인에 실패하면, symlink를 해석한 실제 target과 project ID가 같은 모든 프로젝트
  session을 신뢰되지 않음으로 낮추고, 그 창의 컴포넌트 원본 감시와 캔버스 라이브러리를 내립니다. 실패 완료에도
  상태 snapshot을 게시하므로 이후 요청은 낮춘 신뢰를 봅니다.
- 한계: 다른 창의 셸 표시는 그 창이 신뢰를 다시 조회할 때 갱신됩니다. 신뢰 변경 알림 event는 없습니다.

## Git 패널 이름·컴포넌트 Stage ABI 제거 (2026-11-15 검토)

창의 Git 패널에서 저장 노드 이름 한 속성(U055·U057)과 컴포넌트 원본 파일 하나(U115)를 Stage·Unstage하던 ABI는
이 ADR에 기록되지 않은 채 `crates/ffi`를 넓혔습니다. 행을 표시 세대와 행 번호로 고르고, 이름 변경 64개 상한이
있었습니다. 컴포넌트 경로는 격리 작업 트리와 `core.hooksPath=/dev/null`로 사용자 Git의 hook·filter를 우회했습니다.
계획의 범용 Git 패널 API(XIV.13.1·XIV.13.2)와도 겹칩니다. 2026-11-15 검토에서 반려해 아래 ABI를 앱 연결과
함께 삭제합니다.

- `stellar_git_names_request`, `stellar_git_names_row`, `stellar_git_component_row`, `stellar_git_names_cancel`.
- event `0x010C`(이름 목록과 HEAD 표시 payload)와 실패 코드 26. 두 값은 비워 두고 다시 쓰지 않습니다.
- runtime의 창별 Git 행 캐시와 worker, 이 경로만 쓰던 helper(`WrittenCandidate::single_file`,
  `list_working_tree_pathspec`, 공개 `git_component_library`)도 삭제했습니다.

staging 코어(`prepare_stage_candidate`·`prepare_unstage_candidate`·`apply_stage_candidate`)와 프로젝트 신뢰
요청 `stellar_project_trust`는 유지합니다. 앱 Git 패널은 신뢰 확인·승인·철회만 표시합니다. 변경 목록과
Stage·Unstage는 XIV.13.1·XIV.13.2에서 표시 행 번호가 아닌 변경 ID 기반 ABI로 다시 정합니다.

## 캔버스 편집 ABI 정리 (2026-11-15 검토, 묶음 B4)

2026-11-15 검토에서 Codex가 이 ADR에 기록하지 않고 넓힌 캔버스 편집·선택·레이어·Inspector ABI를 정리합니다.
남기는 ABI는 이 절에 기록하고, 지운 ABI는 사유와 함께 적습니다.

### 제한 모드의 문서 편집 (사용자 결정 D1)

- 제한 모드(신뢰 전)의 목적은 신뢰하지 않은 저장소에서 코드 실행을 막는 것입니다. 대상은 Git hook·filter,
  내장 터미널, MCP·에이전트 노출, 외부 명령, 프로젝트 밖 symlink 내용 노출입니다(아키텍처 §24.3).
- 캔버스·Inspector·Layers의 문서 편집과 저장은 프로젝트 신뢰와 무관하게 허용합니다. CLI `stellar apply`도
  신뢰를 요구하지 않으므로 앱과 CLI의 규칙이 같아집니다.
- runtime의 편집 대상 조회는 일반 창 조회 `window_mut`이며 신뢰를 검사하지 않습니다(이전 `trusted_canvas_window`,
  그 뒤 이름만 바꾼 감싸기 `canvas_window`는 지웠습니다).
  이동·크기·배경·모서리, Frame·Shape 삽입, 선택 삭제·복제·순서, Copy·Paste, 레이어 이름 변경은
  `STELLAR_FAILURE_TRUST_UNAVAILABLE`로 거부되지 않습니다.
- 셸은 편집 경로에서 신뢰 시트를 요청하지 않고, Inspector에 신뢰 이유의 읽기 전용 상태를 표시하지 않습니다.
  신뢰 시트는 Git 패널과 실행 계열 기능에서만 엽니다.
- 이미지 배치(`stellar_window_place_png`)도 같은 규칙입니다. 이미지 경로의 신뢰 확인은 캔버스 렌더·입력·감시
  묶음(B8)에서 지웠습니다(아래 "PNG 배치와 표시"). 이미지 레코드 위치(D2)는 그 절의 남은 일입니다.


### 새 영속 ID와 셸 제스처 해석의 임시 구조

- 캔버스에서 만드는 새 node(Frame·Shape)와 그 modifier, Move가 처음 넣는 Transform modifier는
  `stellar_project::new_ids`(UUIDv7, OS 난수)로 ID를 받습니다. 이전 구현은 창 ID와 요청 순번의 해시나
  node ID의 마지막 byte를 바꾼 값에서 ID를 파생했습니다. 복제·붙여넣기는 계획 API가 ID 생성기를 받으므로
  생성기 seed를 같은 OS 난수에서 채웁니다.
- 같은 규칙을 캔버스 이동·크기 핸들(`stellar_window_canvas_move`·`_resize`, Option 드래그 복제 포함)과 속성 패널
  편집(`stellar_window_edit_property`)에도 적용합니다. 통합 때 이 경로들은 command ID와 node ID의 hash,
  command ID bytes를 seed로 한 생성기, node ID XOR command ID로 새 modifier ID를 만들어 위 규칙이 되돌아갔습니다.
  지금은 runtime이 OS 난수 seed의 생성기(`fresh_ids`·`fresh_generator`)로 ID를 만들고,
  `stellar_commands::inspector::edit_commands`는 seed 대신 호출자가 주는 ID 공급자를 받습니다. 이동·크기·속성
  패널이 처음 넣는 modifier ID가 version 7이고 같은 창·요청 순번으로 다시 해도 겹치지 않음을 테스트로 고정했습니다.
- 이동·생성 도구의 제스처(드래그 임계값, 수식키, 클릭 생성 크기)는 지금 셸이 해석해 결과 좌표만 위
  ABI로 보냅니다. canvas_editing 원칙 2("셸은 해석하지 않고 전달")와 다른 임시 구조입니다. hit test와 렌더를
  코어로 옮기는 후속(XI.8.2 → XII.4.1)에서 코어 도구 상태 기계로 옮깁니다.

### 선택 편집과 레이어 순서

- **대상**: `StellarSelectionTarget`(48 byte, align 8)은 한 게시 snapshot에서 읽은 `session`(u64, offset 0),
  `revision`(u64, 8), `history_sequence`(u64, 16), `selection_count`(u64, 24), 활성 `view`(StableId 16 byte, 32)입니다.
  `stellar_window_selection_target(handle, session, out)`이 채웁니다. 활성 View가 없으면 `NO_ERROR`와 0 값,
  열려 있지 않은 창이면 `INVALID_ARGUMENT`입니다. 선택 삭제·복제·순서, Copy·Paste, PNG 배치가 이 대상을 씁니다.
- **편집**: `stellar_window_edit_selection(handle, target, action, ids, count, out_request)`의 action은
  1 Delete, 2 Duplicate, 3 Forward(바로 위 형제와 교체), 4 Backward, 5 Front, 6 Back입니다(`STELLAR_SELECTION_*`).
  `ids`가 NULL이면 대상 선택 전체이고, 값이 있으면 `count == selection_count`여야 하며 순서와 무관하게 실행
  시점의 선택 전체와 같아야 합니다(Layers 요청). 일부만 보인 페이지와 중복 ID는 거부합니다.
- **검증 시점**: 접수는 기다리지 않습니다. document thread가 실행할 때 창의 `history_sequence`·활성 View·
  revision이 대상과 같은지 확인합니다. 접수 뒤 선택이 바뀌었거나 같은 revision에서 다른 선택으로 갔다가
  돌아온 경우도 sequence가 달라 거부합니다(`REQUEST_FAILED`, UnknownView). Undo·Redo가 원래 sequence를
  복원하면 다시 받습니다.
- **삭제·복제**: 삭제는 선택한 subtree root를 한 transaction으로 지우고, 잠긴 조상·자손이 있으면 전체를
  거부합니다. 복제는 단일 선택 subtree를 같은 부모 아래 원본 바로 위(원본과 다음 형제의 순서 키 사이)에
  넣고 복제본을 선택합니다. 흐름 컨테이너에서도 원본 다음 자리가 됩니다.
- **레이어 순서**: `stellar_window_layer_order_flags(handle, target)`는 대상이 현재 게시 snapshot과 같고 단일
  선택일 때 `STELLAR_ORDER_CAN_FORWARD`(1), `_CAN_BACKWARD`(2), `_FLOW_PARENT`(4)를 돌려주고, 아니면 0입니다.
  셸은 이 값으로 Bring Forward·to Front·Send Backward·to Back 메뉴를 켜고, 흐름 부모이면 순서 변경 뒤
  배치 순서도 바뀐다고 VoiceOver에 알립니다.
- **삭제한 레거시 ABI**: 접수 시점의 게시 선택을 쓰던 `stellar_window_session_delete_selection`,
  `stellar_window_session_duplicate_selection`과 runtime `delete_selected_nodes`·`duplicate_selected_node`는
  셸 호출자가 없고 대상 ABI와 기능이 겹쳐 삭제했습니다. 경쟁 테스트는 `edit_selection`으로 옮겼습니다.
- 단정 없이 시간을 출력만 하던 `measure_layer_reorder`(commands)를 삭제했습니다.

### 선택 Copy·Paste

- Copy·Paste ABI는 `stellar_window_copy_selection`·`stellar_window_paste_selection` 둘이며 위 선택 대상을 씁니다.
  규칙은 ADR-0014 "선택 subtree Copy·Paste"에 있습니다. Paste의 목적지는 실행 시점의 선택에서 정하며 명시 부모
  인자는 없습니다.
- 삭제한 레거시 ABI: View·revision만 받던 `stellar_window_session_copy_subtree`와 명시 부모를 받던
  `stellar_window_session_paste_subtree`, runtime `copy_selected_subtree`·`paste_node_subtree`. 셸 호출자가 없고
  선택 대상 ABI와 겹쳤으며 ADR이 레거시만 설명하고 있었습니다.
- 셸은 Copy 결과 전달 때 pasteboard·창·session만 다시 확인합니다. 이전에는 선택·key 창·텍스트 포커스까지
  다시 확인해, 복사한 뒤 선택이 바뀌면 결과를 조용히 버렸습니다. 검사만을 위해 둔 key 창 주입 seam과 그
  검사 블록도 지웠습니다.
- FFI 내부 helper도 `copy_selection_bytes`·`paste_selection_bytes`로 이름을 맞췄습니다(ABI 변화 없음).

### 레이어 페이지와 이름 변경

- `stellar_window_layer_page(handle, session, query, info, rows, text, capacity, out_history)` 하나로 합칩니다.
  `out_history`는 필수이며, 행의 선택 순서와 같은 document thread 시점의 창 history sequence를 씁니다. 셸은
  이 값이 표시 중인 선택 대상의 sequence와 다르면 행을 반영하지 않고 다시 읽습니다. null이면
  `NULL_POINTER`입니다. 같은 기능을 선택 인자로 받던 `stellar_window_layer_page_with_history`는 삭제했습니다.
  행 레이아웃(`StellarLayerQuery`·`StellarLayerInfo`·`StellarLayerRow`)과 64행 페이지 크기는 그대로입니다.
- `stellar_window_layer_rename`은 표시 revision·View 소속·조상 잠금을 확인하고 RenameNode 하나를 창 history에
  기록합니다. 잠긴 레이어(또는 잠긴 부모 아래)는 stale 문구가 아니라 잠금 문구로 거부합니다.
- 헤더에서 레이어 페이지 주석이 Text 미리보기 선언 위에 붙어 있던 것을 해당 선언 위로 옮기고 이름 변경 주석을
  더했습니다. 검사 전용으로 셸 sidebar source에 둔 `isReadingLayers`와 그 교차 읽기 검사 시나리오는 지웠습니다.

### 선택에서 컴포넌트 만들기 제거 (사용자 결정 D3)

- 정의되지 않은 **Object ▸ Create Component from Selection…** 메뉴와 이름 시트를 지웠습니다. Text·도형·modifier가
  든 Frame은 이름을 입력한 뒤 실패했고, 복사 방식이라 원본과 내용이 두 벌로 갈라졌습니다. 원본 형식이 Text와
  Shape를 담을 수 있게 된 뒤 다시 설계합니다.
- 삭제한 ABI: `stellar_window_component_from_selection`, 완료 event `STELLAR_EVENT_COMPONENT_CREATED`(0x010E,
  다른 event가 쓰지 않는 번호로 남깁니다), runtime `create_component_from_selection`과 진행 플래그.
  호출자가 없어진 `stellar_project`의 `ComponentSource::from_selection`과 그 전용 `new_ids_vec`도 묶음 통합 때
  삭제했습니다.

### Inspector 선택 snapshot과 Frame 속성 편집

이 절에 처음 기록한 `stellar_window_selected_node`(`StellarSelectedNode`, runtime `SelectedNode`·`SelectedFrame`)와
`stellar_window_session_set_frame_property`(`StellarFrameValue`, runtime `FrameValue`·`set_canvas_frame_property`)는
같은 날 UX 1단계 통합에서 지웠습니다. 속성 패널의 행 읽기는 `stellar_window_inspector`, 편집은
`stellar_window_edit_property` 하나가 맡고, 모든 node 종류와 다중 선택을 다룹니다(ADR-0070 결정 6). 남는 규칙은
그대로입니다.

- Inspector 값과 편집 가능 여부는 core가 문서에서 읽은 선택 snapshot 하나에서 나옵니다. 셸은 캔버스 투영이나 Git
  데이터 소스에서 값을 읽지 않습니다.
- 모든 편집은 표시한 View·revision·history sequence를 다시 확인하고 Undo 가능한 transaction 하나로 기록합니다.
- 그보다 먼저 지운 ABI: `stellar_window_selected_frame`·`stellar_window_selected_frame_corners`
  (`StellarFrameSelection`), `stellar_window_session_resize_frame`, `_set_frame_background`, `_set_frame_corner`와
  runtime의 같은 이름 래퍼. 셸이 배경 색을 캔버스의 CGColor에서 역산하던 경로도 지웠습니다.


### 캔버스 행 flags와 Rectangle 생성

- `StellarCanvasFrame.flags`(u32)는 painter 순서 행의 표시·입력 성질입니다. 남는 bit는 다음과 같습니다.

  | bit | 의미 |
  | --- | --- |
  | 0 | 자식을 자기 모양으로 자름(clip children) |
  | 1 | hit test 대상(표시·잠금 해제, 셸 선택 가능) |
  | 2 | Rectangle Shape |
  | 3 | 컴포넌트 인스턴스가 펼친 행 |
  | 4 | Text(저장 텍스트 그리기 ABI로 글리프를 읽음) |
  | 5 | Image(PNG 내용 ABI로 픽셀을 읽음) |
  | 6 | Ellipse·Line·Polygon 경로 Shape(`stellar_canvas_shape_path`로 경로를 읽음) |

  bit 7 이상은 0입니다. bit 4의 내용 ABI는 ADR-0067 §2, bit 5·6은 아래 "PNG 배치와 표시"·"도형 경로" 절에
  있습니다.
- Rectangle 도구는 다른 도형과 같은 `stellar_window_session_insert_shape`(ShapeKind tag 1, 부모 local 두 점)를
  씁니다. 수식키(Shift 정사각형, Option 중심 기준)와 클릭 생성 크기가 다른 도형과 같아집니다. 같은 일을 하던
  `stellar_window_session_insert_rectangle`과 runtime `insert_canvas_rectangle`은 삭제했습니다.

### 새 도형의 기본 스타일 (사용자 결정 D7)

- `stellar_window_session_insert_shape`는 끝에서 둘째 인자로 고정 44 byte `StellarShapeStyle`(`fill`
  `StellarCanvasColor` offset 0, `stroke` 20, `stroke_width` f32 40)의 포인터(`const StellarShapeStyle *`, null 거부)를
  받습니다. 색공간 0은 그 modifier를 만들지 않는다는 뜻이고, `stroke`가 있으면 `stroke_width`는 유한한 양수여야
  합니다. Line은 `stroke`가, 닫힌 도형은 `fill`이나 `stroke` 하나가 있어야 만듭니다. 함수 위치와 검증 규칙은
  ADR-0071 UX.1.5가 기준입니다. core는 기본 색을 정하지 않습니다. 이전에는 core가 `default_for`의 검정을 근거
  없이 썼습니다.
- macOS 셸은 값을 사용자 앱 설정(Settings ▸ Canvas의 Default Shape Fill·Default Line Color, `UserDefaults`)에서
  생성 시점에 읽고, Rectangle·Ellipse·Polygon에는 채우기만, Line에는 선 색 1px만 넘깁니다. 초기값은 채우기
  #D9D9D9, 선 #000000 1px이고 프로젝트 파일에는 기본값을 쓰지 않습니다.
  Frame 생성의 흰 배경과 Text 삽입은 이 결정의 범위가 아니어서 바꾸지 않았습니다.

### 루트 Frame 맞춤

- `stellar_window_session_fit_root(handle, session, width, height, out_request)`는 셸이 잰 실제 캔버스 크기(logical
  point, 유한한 양수)로 활성 View의 root Frame이 화면에 들어오도록 viewport를 정합니다. 가장자리마다
  `min(24pt, 짧은 변의 10%)`를 남기고 100%보다 키우지 않습니다. 중심은 root Frame의 world bounds 중심입니다.
- 새 프로젝트 창의 첫 표시와 View ▸ Zoom to Fit이 같은 ABI를 씁니다. 완료는 `WINDOW_VIEW_CHANGED`, 맞출 Frame이나
  크기가 없으면 `REQUEST_FAILED`(InvalidViewport)입니다. viewport 변경은 history에 기록하지 않습니다.

## 캔버스 표시·입력·감시 ABI 기록 (2026-11-15 검토)

2026-11-15 검토에서 이 ADR 없이 `crates/ffi`에 들어온 캔버스 계열 ABI를 아래처럼 기록하거나 바꿉니다. 남는 ABI만 적습니다.

### 저장 Frame 표시 목록 — `stellar_window_canvas`

- `stellar_window_canvas(handle, session, width, height, info, rows, capacity)`는 창 session의 마지막 게시 상태
  snapshot 하나에서 active View 표시 목록과 카메라를 복사합니다. document thread를 기다리지 않습니다. width·height는
  캔버스 logical point이며 유한한 양수여야 합니다(아니면 `STELLAR_INVALID_ARGUMENT`). 열려 있지 않은 창도
  `STELLAR_INVALID_ARGUMENT`, active View·표시 목록이 없으면 `STELLAR_NO_ERROR`입니다. `rows`가 null이면
  `info`만 씁니다. `capacity`가 모자라면 `info.count`에 최신 개수를 쓰고 `STELLAR_BUFFER_TOO_SMALL`입니다.
- `StellarCanvasInfo`(v1): revision u64, view\[16\], primary\[16\](표시 primary 선택, 없으면 0), camera f32×6, count u32,
  skipped u32(코어가 해석하지 못해 건너뛴 subtree 수). 좌표는 좌상단 원점 logical point이고 camera·world는
  CGAffineTransform 순서 (a, b, c, d, tx, ty)입니다.
- `StellarCanvasFrame`(v1, painter 순서): id\[16\], parent u32(root는 `UINT32_MAX`), flags u32, world f32×6,
  size f32×2, radii f32×4(TL·TR·BR·BL, bounds 기준 정규화 뒤), fill·border `StellarCanvasColor`, border_width f32.
  flags는 bit0 자식 clip, bit1 hit 대상, bit2 Rectangle Shape, bit3 컴포넌트 인스턴스 투영 행, bit4 Text, bit5 Image,
  bit6 경로 Shape입니다.
- `StellarCanvasColor`: space u32(0 색 없음, 1 sRGB, 2 Display P3)와 비연관 RGBA f32×4.
- 표시 목록은 문서 revision이나 active View가 바뀔 때만 다시 평가합니다. 창 문서의 revision 번호는 창마다
  따로 세므로, 저장하지 않은 변경이 없는 창끼리만 같은 프로젝트·generation·문서·View·컴포넌트 원본 hash로
  목록 하나를 공유합니다(`canvas::SharedScenes`). 편집 중인 창과 Text 조합 미리보기가 있는 창(ADR-0067 §6)은 자기
  목록을 평가합니다. 새 글꼴 face를 읽으면(ADR-0067 §5) 창별 목록과 함께 공유 목록도 버립니다.
- 현재 macOS 셸은 이 목록을 Swift Core Graphics 경로(`CanvasFrameScene`)로 그리고 hit test·선택 외곽선도 셸에서
  계산합니다. 아키텍처 §3.5·§10.1과 다른 임시 구조입니다. 코어 SceneSnapshot을 Metal로 그리고 hit test를 코어
  질의로 바꾸는 일(XI.8.2 다음 XII.4.1)에서 이 경로를 지우며, 그때까지 CG 경로에는 기능을 더하지 않습니다.

### 컴포넌트 원본 재읽기와 감시 — `stellar_window_reload_component_canvas`, flags bit3

- `stellar_window_reload_component_canvas(handle, session, out_request)`는 그 창 프로젝트의 `design/components/`
  원본을 문서 thread 밖 worker에서 읽고, document thread에서 창 표시 목록의 정의 집합만 바꿉니다. 문서·history·
  선택은 바꾸지 않습니다. 완료는 `WINDOW_VIEW_CHANGED`, 창이 닫혔으면 `REQUEST_FAILED`(`UNKNOWN_WINDOW_SESSION`)입니다.
  읽기는 시작 순서를 가지며, 늦게 끝난 이전 읽기는 새 결과를 덮지 않습니다.
- 읽기와 감시는 프로젝트 신뢰와 무관합니다(사용자 결정 D1, ADR-0061 결정 8). 창 세션의 신뢰 상태는
  `ExternalAccess::from_trust`로 symlink 정책만 정합니다. 신뢰하지 않은 프로젝트는 프로젝트 안 원본을 읽어 인스턴스를
  그리고, 프로젝트 밖 target의 내용은 읽지 않습니다. 외부 target을 읽는 정책으로 시작한 읽기는 반영 시점에 신뢰가
  철회됐으면 버리고, 신뢰를 철회하면 진행 중인 읽기와 무관하게 정의를 비웁니다. 셸은 신뢰 상태가 바뀔 때마다 다시
  읽기를 요청하므로, 철회 뒤에는 외부 target을 읽지 않는 정책으로 정의가 다시 채워집니다. 이전 구현(2026-11-15 통합
  시점)은 신뢰하지 않은 프로젝트의 읽기·감시를 통째로 막아, 제한 모드에서 배치한 인스턴스가 그려지지 않았습니다.
- 표시 목록에서 컴포넌트 인스턴스는 override가 없는 경우에만 정의 Frame 트리를 펼친 행(flags bit3)으로 그립니다.
  override가 있거나 정의를 찾지 못한 인스턴스는 그리지 않고 `skipped`로 셉니다. override 표시가 필요해지면 캔버스
  투영을 문서의 `InstanceExpansion` 결과로 옮깁니다(후속).
- **원본 감시**: 코어가 프로젝트 session마다(신뢰와 무관) XIII.2 `FsEventsWatcher` 하나를 둡니다. 그 프로젝트 창의
  첫 재읽기 요청에서 시작하고, 프로젝트가 닫히거나 창이 모두 닫히면 끝납니다(감시 thread가 250ms마다 상태
  snapshot을 확인). `design/components` 폴더 자체나 그 바로 아래 `*.stellar.yaml`의 변경, 또는 넘침을 보면
  (`FileWatchEvent::affects_component_sources`) 그 프로젝트의 모든 창에 재읽기를 예약합니다. 앱·CLI·내장 터미널·Git
  checkout 중 무엇이 고쳤는지 구분하지 않으므로 창이 계속 key 창이어도 반영됩니다.
- **요청 없는 event**: 감시가 예약한 재읽기로 정의가 실제로 바뀐 창에는 request 0인 `WINDOW_VIEW_CHANGED`를 보냅니다.
  같은 창의 요청 없는 event가 아직 남아 있으면 더하지 않고, 요청 상한 계산에 들어가지 않습니다. 셸은 이 event를 받으면
  그 창의 표시 목록을 revision과 무관하게 다시 읽습니다. 셸이 깨어나는 방법은 ADR-0014의 event 준비 신호
  (`stellar_runtime_event_ready_fd`)입니다.
- 셸은 신뢰 확인·변경과 문서 복구 뒤에 재읽기를 요청합니다. 창 복귀(key 창 전환) 때마다 다시 읽던 동작은 감시와
  기능이 겹쳐 삭제했습니다.
- 삭제: `stellar_window_component_watch_fd`는 창마다 전용 FSEvents stream을 만들고 그 알림 fd를 셸로 넘겼습니다.
  XIII.2 `FsEventsWatcher`를 우회하고, 같은 프로젝트를 연 창 수만큼 stream이 생겼습니다. 이 ABI와 project
  `ComponentFileWatch`, runtime 창 entry의 감시 필드, Swift 감시 객체를 함께 삭제했습니다.

### PNG 배치와 표시 — `stellar_window_place_png`, `stellar_window_canvas_png`, flags bit5

- `stellar_window_place_png(handle, target, parent[16], x, y, path, path_len, digest[32], out_request)`: 셸이 고르고
  디코드한 PNG(절대 경로, UTF-8 4096 byte 이하, SHA-256 `digest`)를 `target`(창·View·revision·선택 이력, 48 byte
  `StellarSelectionTarget`)에서 누른 부모 Frame의 local 좌표 `(x, y)`에 이미지 node 하나로 배치합니다. worker가
  파일을 다시 읽어 digest·크기(32 MiB, 16 megapixel 이하)·PNG 형식을 확인하고 에셋 저장소에 가져온 뒤, document
  thread에서 대상을 다시 확인하고 InsertNode와 선택을 한 history 항목으로 기록합니다. 동시에 하나만 받으며 둘째 요청은
  `STELLAR_QUEUE_FULL`입니다. 완료는 `WINDOW_VIEW_CHANGED`, 실패 코드는 표시 대상이 바뀌었거나 편집할 수 없으면
  `UNKNOWN_VIEW`, 프로젝트 generation이 바뀌었으면 `SAVE_STALE`, 열린 폴더·ID가 바뀌었으면 `PATH_UNRESOLVED`·
  `INVALID_PROJECT_ID`, 파일 읽기·형식·가져오기 실패는 `SAVE_FAILED`, 창이 닫혔으면 `UNKNOWN_WINDOW_SESSION`입니다.
- `stellar_window_canvas_png(handle, session, revision, row, node[16], digest[32], buf, capacity, out_len)`: 셸 worker
  전용입니다. 같은 revision·행·node의 표시 목록 이미지 행이 참조하는 PNG의 내용 hash와 길이를 쓰고, `buf`가 있으면
  bytes도 복사합니다. 행이 맞지 않거나 읽을 수 없으면 `STELLAR_NO_ERROR`(길이 0), 용량이 모자라면
  `STELLAR_BUFFER_TOO_SMALL`입니다. 셸은 node별 마지막 내용 hash를 revision이 바뀌어도 유지하고 다시 확인하는 동안
  이전 픽셀을 그립니다. 내용 hash가 바뀐 node만 새로 디코드합니다.
- flags bit5는 이미지 행입니다(픽셀은 위 함수로 worker에서 읽음).
- 배치와 표시는 사용자 결정 D1에 따라 프로젝트 신뢰와 무관합니다. 신뢰 저장소 대신 열린 폴더의 실경로와 매니페스트
  프로젝트 ID만 확인합니다. 에셋 저장소는 symlink를 따르지 않으므로 제한 모드에서도 프로젝트 밖 내용을 쓰거나
  읽지 않습니다.
- 도구 흐름(tools_menus): Image 도구를 고른 뒤 캔버스를 누르면 선택 패널을 열고, 고른 이미지를 누른 위치에
  배치합니다. 취소하거나 열 수 없는 파일이면 이전 도구로 돌아갑니다.
- 남은 일(사용자 결정 D2): 에셋 레코드는 아직 창 session(저장 전)과 generation의 `asset-metadata` chunk(저장 뒤)에
  있습니다. D2는 글꼴·이미지 레코드를 텍스트 asset catalog 한 곳에 두고 문서는 참조만 하도록 정했으며 migration
  경로(IX.11.21)를 먼저 정합니다. 그 catalog가 생기면 레코드를 그 한 곳으로 옮기고, 저장 전 레코드가 복구와 다른
  창에서 빠지는 한계도 함께 없앱니다. 이번 검토에서는 `crates/storage` `ChunkStore::read_chunk_limited`를 다시
  비공개로 되돌리고(U127이 이 ADR 없이 공개함) 공개 `read_chunk` 뒤 길이를 확인합니다.

### 도형 경로 — `stellar_canvas_shape_path`, flags bit6

- Ellipse·Line·Polygon처럼 사각형이 아닌 도형 행은 flags bit6(경로 Shape)이고, 경로를 표시 목록 행에 넣지 않고 따로
  조회합니다. `stellar_canvas_shape_path(handle, session, query, rows, capacity, count, cap_flags)`는
  `StellarCanvasTextQuery`(view\[16\], node\[16\], revision u64, row u32)가 마지막 게시 snapshot의 같은 View·revision·
  행·node를 가리킬 때만 그 행의 local 경로를 복사합니다. 맞지 않거나 도형 경로가 없으면 `STELLAR_NO_ERROR`(개수 0),
  열려 있지 않은 창은 `STELLAR_INVALID_ARGUMENT`입니다. document thread를 기다리지 않습니다.
- 요소는 `StellarCanvasTextElement`(op u32, points f32×6)입니다. op 1·2·3·4는 Move·Line·Cubic·Close이고, op 5는 길이
  0인 Line의 cap을 그릴 local 사각형(x, y, w, h)입니다. `cap_flags`에는 stroke cap(0 butt, 1 round, 2 square)을 씁니다.
  `rows`가 null이면 `count`만 쓰고, `capacity`가 모자라면 `STELLAR_BUFFER_TOO_SMALL`입니다.
- 경로는 코어 SceneBuilder가 확정한 값이며 셸은 그대로 그립니다. 이 함수와 bit6은 위 임시 CG 경로의 일부라 Metal 이관 때
  다시 정합니다.
