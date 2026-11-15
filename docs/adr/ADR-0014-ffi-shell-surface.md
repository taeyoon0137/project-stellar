# ADR-0014: 셸↔코어 FFI 표면 확장 기록

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §8.2는 셸과 코어 경계를 다음으로 고정합니다: 안정적인 C ABI, Rust 내부 타입 비노출,
handle·POD 데이터·버전형 메시지와 명시적 수명 관리, "고빈도 입력은 작은 고정 구조체 또는 ring buffer로
전달". II.4.1~II.4.6은 정수 registry handle, 모든 함수의 `StellarStatus` 반환, handle별
`stellar_last_error`, UTF-8 ptr+len 입력·호출자 buffer 출력, `catch_unwind` guard, pull 방식 이벤트
큐, 고정 템플릿 헤더 생성(`cargo xtask generate-ffi-header`)을 정했습니다.

구현 계획 1.2의 3번 규칙은 작업 지시 없이 FFI 시그니처를 추가하지 않도록 하고, I.3.5 정책은
`crates/ffi/` 변경에 같은 diff의 ADR을 요구합니다. 이 ADR은 Phase XII 셸 통합 작업이 추가하는
C ABI를 작업별로 기록합니다. 기존 함수·상태 코드의 의미는 바꾸지 않습니다.

## Decision

### XII.3.2 — macOS 입력 이벤트 전달

XII.3.1이 정의한 공통 `InputEvent`(Rust)를 macOS 셸이 코어에 넘기는 경로입니다.

1. **고정 구조체 `StellarMacEvent`(104 byte, `#[repr(C)]`)**: AppKit `NSEvent` 원시 필드(event type,
   keyCode, subtype, isARepeat, hasPreciseScrollingDeltas, modifierFlags, timestamp, clickCount,
   buttonNumber, scrollingDelta, phase, momentumPhase)와 셸 수신 시각(`receive_uptime`), 캔버스 view
   좌표(`x`, `y`)를 담습니다. 첫 필드 `version`(`STELLAR_MAC_EVENT_VERSION` = 1)이 다르거나 `reserved`가
   0이 아니면 거부합니다. pointer 필드는 없습니다. 크기·offset은 Rust 테스트와 헤더의
   `_Static_assert`/`static_assert`로 고정합니다.
2. **변환은 코어, 복사는 셸**: 셸은 AppKit 값을 가공하지 않고 복사하며, 위치만 AppKit view 변환으로
   캔버스 view 좌표(flipped view, 좌상단 원점, y 아래로 증가)로 옮깁니다. 이벤트 종류·button·modifier·
   scroll phase 해석과 virtual key code → HID usage 변환은 `crates/ffi/src/input_macos.rs`가 합니다.
   변환 규칙은 그 모듈 문서가 기준입니다(공통 집합 밖 이벤트는 대기열에 넣지 않음).
3. **`stellar_runtime_submit_mac_event(handle, const StellarMacEvent *event, uint32_t *out_queued)`**:
   변환 결과를 runtime 입력 대기열에 넣고 넣은 수(0 또는 1)를 씁니다. null `event`는
   `STELLAR_NULL_POINTER`, 규칙 위반은 `STELLAR_INVALID_ARGUMENT`(last error에 세부)입니다.
4. **`stellar_runtime_poll_input(handle, buf, buf_len, out_written)`**: 대기열의 다음 이벤트를
   `InputEvent::encode` 바이트로 읽습니다. 빈 대기열 `STELLAR_NO_ERROR`, `buf == NULL`이면 길이만 보고,
   작은 buffer는 `STELLAR_BUFFER_TOO_SMALL`(꺼내지 않음) — `stellar_runtime_poll_event`(II.4.5)와 같은
   pull 규칙입니다. 셸 검증·진단과 이후 코어 소비자를 위한 읽기 경로입니다.
5. **상태 코드 추가(끝에 덧붙임)**: `STELLAR_INVALID_ARGUMENT = 8`, `STELLAR_QUEUE_FULL = 9`.
   기존 값 0~7은 그대로입니다.
6. **대기열 용량**: `STELLAR_INPUT_QUEUE_CAPACITY` = 4096 이벤트. 가득 차면 새 입력을 버리고
   `STELLAR_QUEUE_FULL`을 반환합니다. 소비자(DocumentThread, XII.1.2)가 생기기 전에도 메모리가 무한히
   늘지 않게 하는 상한이며, UI→DocumentThread queue의 overflow 정책은 XII.1.2가 정합니다.
7. **시간 domain**: runtime마다 `InputTimeDomain`을 두고 첫 제출 이벤트의 `receive_uptime`(셸이 받은
   시각, `NSEvent.timestamp`와 같은 축)과 코어 단조 시계 현재값으로 기준점을 잡습니다. 코어 단조 시계는
   `Instant` 기반이며 기준점을 첫 `stellar_runtime_create`에서 고정합니다(runtime 생성 뒤 발생한 입력이
   0으로 눌리지 않게). wall clock은 쓰지 않습니다(II.3.3).
   - 2026-11-14 보완(보안 리뷰 FR-3·FR-4): 기준점은 검증(버전·예약·종류 변환)을 통과한 첫 이벤트에서만
     잡습니다. 거부된 이벤트는 기준점을 정하지 않습니다. `receive_uptime`은 모든 이벤트에서 유한한 음이
     아닌 값이어야 하고(위반은 `STELLAR_INVALID_ARGUMENT`), `receive_uptime`보다 10초 넘게 늦은
     `timestamp`는 `receive_uptime`으로 자릅니다. 실제 이벤트는 받은 뒤에 일어날 수 없으므로, 미래 시각
     하나가 단조 보정으로 이후 입력 시각을 고정하는 범위를 10초 안으로 묶습니다(허용 오차는 합성 이벤트를
     최대 9초 앞선 timestamp로 만드는 셸 입력 검사 `InputCheck.swift` 때문입니다). 구조체·시그니처는
     그대로입니다.
8. **창 지정 제출(2026-11-15 검토)**: `stellar_window_session_submit_mac_event(handle, session, event, out_request)`는
   같은 변환·시간 domain 규칙으로 창 session 하나를 대상으로 받습니다. 수식키 없는 스크롤은 코어가 그 창의 팬으로
   해석하고, 그 밖의 이벤트는 위 입력 대기열에 넣습니다. 팬 해석과 합치기 규칙은 ADR-0028 "스크롤의 창별 팬
   연결"에 있습니다.

### 요청 없는 event와 event 준비 신호 (2026-11-15 검토)

1. **요청 없는 event**: 셸 요청 없이 코어가 상태를 바꾸면(현재는 컴포넌트 원본 감시의 창 표시 변경, ADR-0028)
   request 0인 완료 event를 같은 outbox에 넣습니다. request id는 1부터이므로 0과 겹치지 않습니다. 요청마다
   정확히 하나라는 기존 완료 event 규칙은 그대로이고, 요청 없는 event는 요청 상한(꺼내지 않은 완료 event 수)에
   들어가지 않습니다. 같은 대상의 요청 없는 event가 아직 남아 있으면 더하지 않아 outbox가 늘지 않습니다.
2. **`int32_t stellar_runtime_event_ready_fd(uint64_t handle)`**: 요청 없는 event가 쌓이면 읽을 수 있게 되는 비차단
   1 byte datagram socket fd를 돌려줍니다(호출자가 소유하고 닫음, 실패 시 -1). 신호는 "꺼낼 event가 있을 수 있음"이며
   개수가 아닙니다. 셸은 깨어나면 남은 신호를 모두 읽고 `stellar_runtime_poll_event`로 꺼냅니다. 요청한 event는
   이 신호를 쓰지 않으므로, 기다리는 요청이 있을 때의 셸 polling(ADR-0009 pull 규칙)은 그대로입니다. callback은 여전히
   없습니다.

### XII.3.4 — 입력 대기열 coalescing

1. **끝 항목과만 병합**: runtime 입력 대기열(`crates/ffi/src/input_queue.rs`)은 새 이벤트를 아직 꺼내지 않은
   마지막 항목과만 병합합니다. pointer down/up·cancel, key, text composition, focus, resize는 병합하지 않으므로
   이들의 상대 순서와 앞뒤 move/wheel과의 순서는 바뀌지 않습니다.
2. **pointer move**: 둘 다 `Move`이고 button·device·modifier가 같으면 최신 이벤트(위치·시각)만 남깁니다.
3. **wheel**: 같은 phase(`Changed`·`Momentum`·`Discrete`)·단위·modifier면 delta를 더하고 위치·시각은 최신 값을
   씁니다. `Began`·`Ended`는 gesture 경계라 병합하지 않고, `f32` 합이 비유한이면 병합하지 않습니다.
4. **`out_queued` 의미**: 시그니처는 그대로이고, 끝 항목에 병합된 이벤트도 "대기열에 반영"으로 보아 1을 씁니다
   (무시된 이벤트만 0). 헤더 주석을 이에 맞췄습니다.
5. **용량**: `STELLAR_INPUT_QUEUE_CAPACITY`는 항목 수입니다. 가득 찬 상태에서도 끝 항목과 병합되는 move/wheel은
   받고, 병합할 수 없는 이벤트만 `STELLAR_QUEUE_FULL`로 버립니다.

### XII.6 코어 측 — 프로젝트 열기·문서 session·완료 event·상태 snapshot

셸의 Open Project 명령(XII.6.2)과 문서 열기·닫기가 부를 코어 경계입니다. 셸 UI는 이 결정에 포함하지
않습니다. `stellar-ffi`는 ADR-0013의 허용 edge(`ffi → runtime`)로 `stellar-runtime`에 처음 의존합니다.

1. **요청은 비동기, 결과는 pull**: `stellar_project_open(handle, path ptr+len, *out_request)`,
   `stellar_project_close(handle, project, *out_request)`,
   `stellar_document_open(handle, project, const uint8_t document_id[16], *out_request)`,
   `stellar_document_close(handle, document, *out_request)`는 기다리지 않고 request id(1부터)를 씁니다.
   작업은 `stellar-runtime`의 `AppRuntime`이 document thread(XII.1.1)의 command 우선순위 job으로
   실행합니다. 요청마다 정확히 하나의 완료 event가 기존 `stellar_runtime_poll_event`로 옵니다(II.4.5
   pull 규칙, callback 없음). 코어 내부 event(`enqueue_event`)가 있으면 그것을 먼저 읽습니다.
2. **event payload**: kind `STELLAR_EVENT_PROJECT_OPENED`(0x0101)·`PROJECT_CLOSED`·`DOCUMENT_OPENED`·
   `DOCUMENT_CLOSED`·`REQUEST_FAILED`(0x0105). payload는 little-endian, 맨 앞 `u32` schema version
   (`STELLAR_EVENT_SCHEMA_VERSION` = 1), 문자열은 `u32` 길이 + UTF-8(최대 `STELLAR_EVENT_TEXT_MAX` =
   4096 byte, 문자 경계에서 자름), id는 StableId 16 byte입니다. 필드 순서는 헤더 주석과
   `crates/runtime/src/app.rs` 모듈 문서가 기준입니다.
3. **실패 코드(`STELLAR_FAILURE_*`)**: 1 unknown project, 2 unknown document, 3 document in other
   project, 10 not a repository, 11 not a Stellar project, 12 manifest unreadable, 13 manifest corrupt,
   14 manifest invalid, 15 invalid project id, 16 untrusted symlink, 17 path unresolved. 값은 바꾸지
   않고 끝에 덧붙입니다. 경계에서 바로 판정할 수 있는 입력 오류(빈·상대 경로, NUL, 잘못된 UTF-8,
   null pointer)는 event 없이 상태 코드(`INVALID_ARGUMENT`·`INVALID_UTF8`·`NULL_POINTER`)와 last
   error로 돌려줍니다(`out_request = 0`).
4. **프로젝트 열기 순서**: 열기 분류(IV.2.4) → 신뢰되지 않은 프로젝트용 안전 probe(IV.5.3: 매니페스트
   메타데이터만 읽고 Git·필터를 실행하지 않으며 symlink 매니페스트를 따르지 않음) → `project.id`
   StableId 변환 → `ProjectIdentity`(IV.2.3). 모든 프로젝트를 신뢰되지 않음으로 엽니다. OS 사용자
   신뢰 저장소의 확인·승인·철회는 별도 요청 `stellar_project_trust`(ADR-0028 XVI.7.2 절)가 합니다.
   문서 청크는 읽지 않습니다.
5. **문서 session**: 문서는 프로젝트 session 아래 StableId로 엽니다. 같은 session의 재열기는 기존
   문서 session(`already_open = 1`), 다른 session이 연 문서는 실패 3입니다(document thread의 열린 문서
   집합이 하나뿐이기 때문, 창별 독립 memory model은 XII.2.4). 프로젝트를 닫으면 그 아래 문서도
   닫습니다. 저장된 문서 내용 로드는 아직 하지 않으며 새로 연 문서는 빈 문서(revision 0)입니다.
6. **상태 snapshot**: `stellar_runtime_state(handle, StellarRuntimeState *)`(고정 40 byte, version
   `STELLAR_RUNTIME_STATE_VERSION` = 1)와 `stellar_document_info(handle, document,
   StellarDocumentInfo *)`(고정 56 byte, version `STELLAR_DOCUMENT_INFO_VERSION` = 1)는 document
   thread가 성공한 요청마다 게시하는 불변 snapshot을 읽습니다. 완료 event보다 snapshot을 먼저
   게시하므로 event를 받은 뒤 읽은 값에는 그 결과가 들어 있습니다. 열려 있지 않은 문서는
   `INVALID_ARGUMENT`입니다. 크기·offset은 Rust 테스트와 헤더 `static_assert`로 고정합니다.
7. **outbox 상한**: 완료 event를 꺼내지 않은 요청(실행 중 포함)이 `STELLAR_EVENT_CAPACITY`(1024)에
   닿으면 새 요청은 `STELLAR_QUEUE_FULL`입니다. 접수한 요청의 완료 event는 버리지 않습니다.
8. **수명**: `AppRuntime`은 handle의 첫 프로젝트·문서 요청에서 시작합니다(그 전의 create/destroy
   비용·동작은 그대로). `stellar_runtime_destroy`는 registry lock을 놓은 뒤 runtime을 파괴하고,
   파괴는 이미 접수한 요청을 실행한 뒤 document thread를 join합니다. join은 최대 2초
   (`AppRuntime::SHUTDOWN_TIMEOUT`)까지만 기다리고, 넘으면 아직 시작하지 않은 요청을 버리고 thread를
   분리한 뒤 반환합니다(2026-11-14 보완, 보안 리뷰 FR-1: 멈춘 파일 시스템 호출이 앱 종료를 막지 않게).
   시그니처와 반환 상태는 그대로입니다.
9. **New/Clone Project는 이 절에 없습니다**: 처음 기록할 때는 crate 정책이 `stellar-runtime →
   stellar-git-porcelain`을 허용하지 않아 `create_project`·`clone_and_open`(IV.4.7·IV.6.4)에 runner를 넘길 수
   없었습니다. ADR-0013이 XIV.5.5를 위해 이 직접 의존을 허용한 뒤, 새 프로젝트 생성은 아래 'XII.6.1 — 새 프로젝트
   생성'과 'Clone Project'에서 같은 요청·event 규칙으로 추가합니다(2026-11-15 정정).
10. **지연 자동 저장 flush(UX.2.11, ADR-0072)**: `stellar_runtime_flush_autosave(handle)`는 앱 비활성화 때
    저장되지 않은 완료 편집을 debounce 없이 캡처하도록 접수하고 기다리지 않습니다. 요청이 아니므로 request id와 완료
    event가 없고 outbox 상한을 쓰지 않습니다. 앱 runtime이 아직 없으면 `STELLAR_OK`, 대기열이 차면 `STELLAR_QUEUE_FULL`.
    자동 저장 결과는 창 session 상태 snapshot(`StellarWindowSessionInfo` v3의 `autosave`, ADR-0028)과 상태 세대로만
    셸에 돌아옵니다. 검사 실행에서 자동 저장을 끄는 방법은 C ABI가 아니라 Debug 코어만 읽는 환경 변수입니다(ADR-0072
    10절). 공개 헤더에는 구성별 조건 선언이 없습니다(ADR-0028).

### 캔버스 Text 편집

- 캔버스 Text 편집 ABI(`stellar_text_*`)와 저장 Text 그리기 ABI(`stellar_window_text_*`)는 ADR-0067이 정합니다.

### XII.6.1 — 새 프로젝트 생성

시작 창의 New Project…가 부르는 코어 경계입니다(2026-11-15 검토에서 소급 기록).

1. **요청**: `stellar_project_create(handle, path ptr+len, *out_request)`. path 규칙은 `stellar_project_open`과 같습니다
   (절대 경로, UTF-8, NUL 없음, 위반은 event 없이 `INVALID_ARGUMENT`·`INVALID_UTF8`·`NULL_POINTER`와 `out_request = 0`).
   마지막 경로 요소가 새 폴더 이름이자 프로젝트 이름입니다. 이미 있는 폴더는 만들지 않습니다.
2. **실행 위치**: document thread가 아니라 전용 worker thread(`stellar-project-create`)에서 실행합니다. 시스템 Git을
   찾아 버전을 확인하고(환경 변수는 HOME·XDG_CONFIG_HOME·GIT_CONFIG_*·PATH·Git 작성자·EMAIL만 넘김, 명령당 30초),
   `stellar-project`의 `create_editable_project`로 폴더 → scaffold → 첫 문서 generation 1 → `git init` → add → 첫 commit을
   진행합니다. 첫 문서의 텍스트 원본 전환은 V.10.7 범위입니다.
3. **완료 event**: `STELLAR_EVENT_PROJECT_CREATED`(`0x010d`: request u64, git_state u32). git_state는 0(첫 commit 완료),
   1(Git 사용자 정보 없음), 2(add 실패), 3(commit 실패)입니다. 1–3은 폴더와 첫 generation을 남기고 Git 초기화만 미완료인
   상태라, 셸은 같은 path로 `stellar_project_open`을 부르고 사용자에게 commit을 안내합니다. 생성 자체가 실패하면
   (폴더 생성·scaffold·저장·`git init` 실패, 시스템 Git 없음) 만든 폴더를 롤백하고 `REQUEST_FAILED` +
   `STELLAR_FAILURE_PROJECT_CREATE_FAILED`(27)와 사유 문구를 보냅니다.
4. **동시 생성**: 한 runtime에서 생성은 한 번에 하나만 실행합니다. 앞 생성이 끝나기 전의 요청도 접수해 request id를
   돌려준 뒤 `REQUEST_FAILED` + `PROJECT_CREATE_FAILED`와 '다른 새 프로젝트를 만드는 중' 문구로 끝냅니다.
   `STELLAR_QUEUE_FULL`은 완료 event 상한(결정 7)에만 씁니다.

### Clone Project — 복제 후 제한 모드 열기

시작 창의 Clone…이 부르는 코어 경계입니다(2026-11-15 검토에서 소급 기록).

1. **요청**: `stellar_project_clone(handle, url ptr+len, target ptr+len, *out_request)`. url과 target은 UTF-8이고 각각
   `STELLAR_EVENT_TEXT_MAX`(4096) byte 이하입니다. target은 절대 경로이고, 마지막 요소는 `.`으로 시작하지 않는 유효한
   폴더 이름이어야 합니다. url은 비어 있지 않고 공백·제어 문자가 없어야 합니다. 위반은 event 없이
   `INVALID_ARGUMENT`·`INVALID_UTF8`·`NULL_POINTER`와 `out_request = 0`입니다.
2. **실행 위치**: 전용 worker thread(`stellar-project-clone`)에서 시스템 Git으로 부모 폴더 아래 소유자 전용 임시
   폴더에 clone한 뒤 최종 폴더로 옮깁니다(IV.6.2). 자격 증명은 시스템 Git의 credential helper·SSH agent에만 위임하고
   Stellar는 저장하지 않습니다(XIV.9.6). 실패·취소 때는 임시 폴더를 지우므로 최종 폴더가 남지 않습니다.
3. **열기**: clone이 끝나면 document thread에서 일반 `stellar_project_open`과 같은 제한 모드 검사로 엽니다(결정 4).
   성공 완료 event는 `PROJECT_OPENED`입니다. 열기 검사가 실패하면 `REQUEST_FAILED`에 열기 실패 코드(10–17)를 씁니다.
4. **실패 코드(끝에 덧붙임)**: `STELLAR_FAILURE_PROJECT_CLONE_FAILED`(28, 시스템 Git 없음·clone 실패·열기 접수 실패),
   `STELLAR_FAILURE_CLONE_TARGET_EXISTS`(29, 최종 폴더가 이미 있음), `STELLAR_FAILURE_CLONE_CANCELLED`(30, 취소).
   event 문구에는 원격 URL을 넣지 않습니다(민감값).
5. **취소**: `stellar_project_clone_cancel(handle, request)`는 그 request가 진행 중인 clone일 때만 취소 신호를 보냅니다.
   다른 request id나 이미 끝난 clone에는 아무것도 하지 않고 `OK`입니다. app runtime이 아직 시작되지 않았으면
   `INVALID_ARGUMENT`, 없는 handle은 `INVALID_HANDLE`입니다. 취소된 clone도 완료 event 하나(`CLONE_CANCELLED`)로
   끝납니다. runtime을 파괴하면 진행 중인 clone을 취소합니다.
6. **동시 복제**: 한 runtime에서 clone은 한 번에 하나만 실행합니다. 겹친 요청은 접수한 뒤 `REQUEST_FAILED` +
   `PROJECT_CLONE_FAILED`와 '다른 프로젝트를 복제하는 중' 문구로 끝냅니다(생성과 같은 규칙).

### 로컬 개발 — 선택 subtree Copy·Paste

Copy·Paste는 한 게시 snapshot에서 읽은 `StellarSelectionTarget`(ADR-0028 "선택 편집과 레이어 순서")을 씁니다.

- `stellar_window_copy_selection(handle, target, bytes, capacity, out_len)`은 셸 worker 전용 읽기 API입니다.
  registry lock을 놓고 문서 ticket을 기다린 뒤 caller-owned buffer에 최대 16 MiB를 씁니다. document thread에서
  대상의 history sequence·active View·revision, 단일 선택, 조상 잠금을 확인하며 history를 바꾸지 않습니다. 복사
  불가 상태는 `NO_ERROR`와 길이 0, 출력 부족은 `BUFFER_TOO_SMALL`과 필요한 길이입니다. null 입력은
  `NULL_POINTER`, 존재하지 않는 handle은 `INVALID_HANDLE`입니다.
- `stellar_window_paste_selection(handle, target, bytes, len, out_request)`는 플랫폼 pasteboard 데이터를 소유 복사해
  창의 일반 history로 전달합니다. 길이는 1 byte~16 MiB이며 복사 전에 검사합니다(범위 밖은 `INVALID_ARGUMENT`).
  동기 거부는 `out_request`를 0으로 초기화하고, 접수 뒤 입력 buffer는 해제할 수 있습니다. 완료는
  `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다. document thread에서 대상 snapshot, 같은 문서 출처,
  목적 컨테이너(Frame·Fragment)와 조상 잠금을 확인하고 새 ID·선택·Undo/Redo를 기존 Paste 경로로 적용합니다.
- 목적지는 실행 시점의 선택에서 정합니다. 선택한 컨테이너 안, 그 외 단일 선택의 부모 아래, 선택이 없으면
  active View root 아래입니다. 다중 선택은 자동 목적지를 정하지 않고 거부합니다. 명시 부모 인자는 없습니다.
- macOS는 `xyz.taeyoon.stellar.node-subtree.v1` pasteboard type을 쓰며 같은 문서 출처만 지원합니다.
  Copy 결과를 pasteboard에 쓸 때는 pasteboard가 그사이 바뀌지 않았는지, 같은 창·session이 열려 있는지만
  확인합니다. 선택 확인은 코어가 복사 시점에 하므로, 결과가 도착하기 전에 선택이 바뀌어도 복사한 내용을 버리지
  않습니다. 텍스트 field editor에서는 표준 AppKit Copy/Paste 응답자 체인을 유지합니다.


## Alternatives

- **Swift에서 `InputEvent` 바이트를 직접 인코딩**: 인코더가 두 벌이 되고 변환 규칙을 Rust 테스트·CI에서
  검증할 수 없습니다. 채택하지 않습니다.
- **필드별 scalar 인자 함수**: 인자가 많고 버전을 붙일 곳이 없어 §8.2의 버전형 메시지 원칙에 맞지
  않습니다. 채택하지 않습니다.
- **공유 메모리 ring buffer**: §8.2가 허용하지만 소비자 thread(XII.1.x)가 없는 지금은 이점이 없습니다.
  DocumentThread 도입 시 재검토합니다.
- **셸이 코어 callback을 받아 push**: II.4.5가 callback 재진입을 금지합니다. 셸→코어 방향은 동기
  호출로 충분합니다.
- **대기열 전체를 훑어 같은 종류를 병합(XII.3.4)**: 사이에 낀 down/up·key를 건너뛰어 병합하면 순서가
  바뀝니다. 끝 항목 병합만 채택합니다.
- **wheel도 최신 값만 유지(XII.3.4)**: scroll 거리가 사라집니다. delta 합을 채택합니다.
- **프로젝트 열기를 동기 FFI 호출로 처리(XII.6)**: 파일 I/O 동안 UI thread가 멈추고, 문서 상태를
  document thread 밖에서 바꾸게 됩니다. 비동기 요청 + pull event를 채택합니다.
- **완료를 callback으로 알림(XII.6)**: II.4.5가 재진입을 금지합니다. 기존 `poll_event`를 재사용합니다.
- **상태를 매번 document thread에 질의(XII.6)**: 질의가 document thread job을 기다리게 되어 UI 대기
  금지(XII.1.2)에 어긋납니다. document thread가 게시한 불변 snapshot을 읽습니다.
- **완료 event를 상한 초과 시 버림(XII.6)**: 셸이 영원히 결과를 기다릴 수 있습니다. 접수 단계에서
  거부(`QUEUE_FULL`)합니다.

## Consequences

- 긍정: macOS 이벤트 해석이 순수 Rust 함수라 golden fixture로 결정론적으로 검증되고, Windows 셸이
  같은 대기열·`poll_input`을 재사용할 수 있습니다.
- 긍정: 구조체 크기·버전 검사로 셸과 코어의 레이아웃 불일치를 실행 전에 잡습니다.
- 부정: 소비자가 없는 동안 실행 중인 앱의 대기열은 병합할 수 없는 이벤트(click·key 등)가 4096개 쌓이면
  `STELLAR_QUEUE_FULL`로 입력을 버립니다. 연속 move/wheel은 병합되어 상한에 닿지 않습니다(XII.3.4).
- 부정: 병합된 move의 중간 위치와 개별 시각은 남지 않습니다. 입력 지연(T0) 측정은 XII.3.5에서 병합 전
  수신 시각 기준으로 정합니다.
- 부정: macOS 전용 구조체가 공통 헤더에 들어갑니다. Windows 재개 시 별도 구조체를 추가합니다.
- 긍정(XII.6): 셸은 프로젝트·문서 작업을 handle·POD·버전형 바이트만으로 요청하고 결과를 받습니다.
  document thread 단일 작성자 규칙이 셸 경계에서도 유지됩니다.
- 부정(XII.6): `stellar-ffi` 정적 라이브러리가 `stellar-project`(FSEvents 감시기 포함)를 담습니다.
  `stellar_project_*`를 참조하는 실행 파일은 `CoreServices` framework를 링크해야 합니다(실험:
  `CoreFoundation`·`Security`만 링크하면 `_FSEventStream*` 미해결, `CoreServices` 추가 또는
  `-dead_strip`이면 통과). Xcode 프로젝트(`apps/macos`)의 `OTHER_LDFLAGS`에는 아직 `CoreServices`가
  없습니다. 현재 셸 코드는 새 함수를 참조하지 않아 Debug smoke 링크는 통과하지만, 셸이 이 함수를
  부르기 전에 framework를 추가해야 합니다.
- 부정(XII.6): 프로젝트 열기 I/O(매니페스트 읽기)가 document thread에서 실행됩니다. worker pool
  (XII.1.4)이 생기면 옮깁니다.

## Compatibility

- 추가만 있습니다. 기존 함수 시그니처·상태 코드 값·헤더 선언 순서는 바꾸지 않았습니다.
- `StellarMacEvent` 레이아웃이 바뀌면 `STELLAR_MAC_EVENT_VERSION`을 올리고 이전 버전은 거부합니다.
- 입력 바이트 레이아웃은 XII.3.1의 `INPUT_EVENT_SCHEMA_VERSION`(1)을 그대로 따릅니다.
- XII.6: 함수·상수·구조체를 추가만 했습니다. `stellar_runtime_poll_event`의 시그니처·규칙은 그대로이며,
  읽을 수 있는 event 종류만 늘었습니다. `StellarRuntimeState`·`StellarDocumentInfo` 레이아웃이 바뀌면
  각 version을 올리고, event payload가 바뀌면 `STELLAR_EVENT_SCHEMA_VERSION`을 올립니다.
- 파일 포맷·저장 구조에는 영향이 없습니다.

## Performance

- 제출 1회는 registry mutex 1회, 104 byte 복사, O(1) 변환과 끝 항목 1개 비교(병합)입니다. 입력 전달 단계 예산(§6.2, 목표
  0.15ms)에 대한 측정은 T0/T1 trace 작업(XII.3.5)에서 합니다.

- XII.6: 요청 접수는 registry mutex 1회, outbox 상한 atomic 예약 1회, command lane 접수(O(1))입니다.
  `stellar_runtime_state`·`stellar_document_info`는 snapshot `Arc` 복제 1회로 document thread를
  기다리지 않습니다. 게시 비용은 성공한 요청마다 열린 프로젝트·문서 수에 비례합니다.

## Security

- 구조체의 모든 필드를 검증합니다(버전, 예약 0, 0/1 bool, 유한 좌표·delta·timestamp, click count,
  scroll phase bit). 실패는 상태 코드와 handle별 last error로만 보고하고 panic은 `ffi_guard`가 막습니다.
- 구조체에 pointer가 없어 셸이 코어에 임의 메모리 참조를 넘기지 않습니다.
- XII.6: 프로젝트 열기는 신뢰되지 않은 프로젝트 규칙(IV.5.3)으로만 실행해 Git·필터·외부 프로세스를
  실행하지 않고 symlink 매니페스트를 따르지 않습니다. 경로는 절대 경로·NUL 없음·UTF-8만 받습니다.
  event 문자열 길이는 상한(4096 byte)으로 자르고, Rust 쪽 decoder는 길이·version·UTF-8·코드를 모두
  검사합니다.

## Migration

- 새 ABI이므로 기존 데이터·코드 마이그레이션은 없습니다. 셸은 재생성한 `stellar_ffi.h`를 사용합니다.
