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
9. **New/Clone Project는 이 결정에 없습니다**: `create_project`·`clone_and_open`(IV.4.7·IV.6.4)은
   `stellar-git-porcelain`의 `GitRunner`·`GitVersion`을 인자로 받는데, crate 정책은
   `stellar-runtime → stellar-git-porcelain`을 허용하지 않고 `stellar-project`도 runner를 만드는 공개
   API가 없습니다. 정책 변경(ADR-0013 갱신) 또는 project의 시스템 Git facade가 생기면 같은 요청·event
   규칙으로 추가합니다.

### 로컬 개발 — subtree Paste 전달

`stellar_window_session_paste_subtree(handle, session, view[16], parent[16], revision,
bytes, len, out_request)`는 플랫폼 pasteboard 데이터를 소유 복사하여 기존 runtime Paste에
전달합니다. 길이는 1 byte~16 MiB이며 복사 전에 검사합니다. null 입력은 `NULL_POINTER`,
범위 밖 길이는 `INVALID_ARGUMENT`, 존재하지 않는 handle은 `INVALID_HANDLE`입니다.
동기 거부는 `out_request`를 0으로 초기화하며, 접수 후 입력 buffer는 해제할 수 있습니다.
완료는 기존 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다. document thread에서
같은 문서 출처·active View·revision·목적 컨테이너(Frame·Fragment)와 조상 잠금·프로젝트 신뢰를 검사하고,
새 ID와 일반 창 history·선택·Undo/Redo를 기존 Paste 경로로 적용합니다.
기존 event schema와 ABI 구조체는 변경하지 않습니다. 부모가 NIL이면 현재 선택한 컨테이너(Frame·Fragment) 안,
그 외 단일 선택의 부모 아래, 선택이 없으면 active View root 아래를 목적지로 정합니다.
다중 선택은 자동 목적지를 결정하지 않고 거부합니다.

`stellar_window_session_copy_subtree`는 셸 worker 전용 읽기 API입니다. registry lock을 놓고
문서 ticket을 기다린 뒤 caller-owned buffer에 최대 16 MiB를 씁니다. 단일 선택·신뢰·revision·
View·잠금을 검사하며 history를 바꾸지 않습니다. 복사 불가 상태는 `NO_ERROR`와 길이 0,
출력 부족은 `BUFFER_TOO_SMALL`과 필요한 길이를 반환합니다.
macOS는 `xyz.taeyoon.stellar.node-subtree.v1` pasteboard type을 쓰며 같은 문서 출처만 지원합니다.
Copy 결과가 도착하기 전에 pasteboard가 바뀌거나 창이 닫히면 덮어쓰지 않습니다.
텍스트 field editor에서는 표준 AppKit Copy/Paste 응답자 체인을 유지합니다.


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

### 캔버스 Text 조합과 문서 편집

- 창의 worker에서 `stellar_text_begin`으로 현재 선택한 Text, 활성 View, 문서 revision, 대체할 Unicode scalar 범위를 전달합니다. 같은 창의 새 입력은 새 토큰을 발급하고 이전 조합을 취소합니다.
- `stellar_text_mark`는 marked text 안의 UTF-16 선택을 공통 좌표로 변환합니다. surrogate pair 중간은 기존 조합을 보존하며 거부합니다. `stellar_text_preview`는 호출자 버퍼에 미확정 본문을 씁니다. 문단과 run을 포함한 레이아웃용 preview는 runtime의 동일한 편집 세션에서 얻습니다.
- `stellar_text_commit`은 View·선택·revision을 다시 확인해 단일 ReplaceText를 창 history에 기록합니다. 값이 같으면 history를 추가하지 않습니다. `stellar_text_cancel` 또는 창 닫기는 미확정 본문을 버립니다.
