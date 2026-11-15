//! `generate-ffi-header` (II.4.6).
//!
//! `crates/ffi/include/stellar_ffi.h`를 고정 템플릿에서 생성합니다.
//! 이 모듈의 [`HEADER`] 상수가 헤더의 source of truth이며,
//! `crates/ffi/src/lib.rs`의 `#[no_mangle] extern "C"` 표면과 일치해야 합니다.
//! 생성물 drift는 `--check`(또는 CI의 `git diff`)로 검사합니다.

use std::path::Path;

/// 생성 대상 헤더 경로(저장소 루트 기준).
pub const HEADER_PATH: &str = "crates/ffi/include/stellar_ffi.h";

/// 고정 템플릿: stellar-ffi C ABI 헤더.
///
/// Rust 전용 API(`enqueue_event`)는 `#[no_mangle] extern "C"`가 아니므로 노출하지 않습니다.
pub const HEADER: &str = r#"/* stellar_ffi.h — Project Stellar C ABI.
 *
 * 생성 파일입니다. 직접 수정하지 마세요.
 * 원본: tools/xtask/src/ffi_header.rs (cargo xtask generate-ffi-header).
 */
#ifndef STELLAR_FFI_H
#define STELLAR_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* FFI 호출 상태 코드 (repr i32). */
typedef enum StellarStatus {
    STELLAR_OK = 0,
    STELLAR_INVALID_HANDLE = 1,
    STELLAR_NULL_POINTER = 2,
    STELLAR_BUFFER_TOO_SMALL = 3,
    STELLAR_NO_ERROR = 4,
    STELLAR_INTERNAL = 5,
    STELLAR_INVALID_UTF8 = 6,
    STELLAR_INTERNAL_PANIC = 7,
    STELLAR_INVALID_ARGUMENT = 8,
    STELLAR_QUEUE_FULL = 9
} StellarStatus;

/* 셸→코어 입력 대기열 용량(이벤트 수). 가득 차면 STELLAR_QUEUE_FULL. */
#define STELLAR_INPUT_QUEUE_CAPACITY 4096

/* StellarMacEvent 레이아웃 버전. */
#define STELLAR_MAC_EVENT_VERSION 1

/* macOS NSEvent 원시 필드(고정 104 byte). 이벤트 종류에 해당하지 않는 필드는 0으로 채웁니다. */
typedef struct StellarMacEvent {
    uint32_t version;           /* STELLAR_MAC_EVENT_VERSION */
    uint32_t event_type;        /* NSEvent.EventType.rawValue */
    uint16_t key_code;          /* NSEvent.keyCode (keyDown/keyUp/flagsChanged) */
    int16_t subtype;            /* NSEvent.subtype.rawValue (mouse) */
    uint8_t is_repeat;          /* NSEvent.isARepeat (keyDown, 0/1) */
    uint8_t has_precise_deltas; /* NSEvent.hasPreciseScrollingDeltas (scrollWheel, 0/1) */
    uint16_t reserved;          /* 0 */
    uint64_t modifier_flags;    /* NSEvent.modifierFlags.rawValue */
    double timestamp;           /* NSEvent.timestamp (초) */
    double receive_uptime;      /* 셸 수신 시각 ProcessInfo.systemUptime (초) */
    double x;                   /* 캔버스 view 논리 point, 좌상단 원점 */
    double y;                   /* 캔버스 view 논리 point, 아래로 증가 */
    int64_t click_count;        /* NSEvent.clickCount (mouse down/up) */
    int64_t button_number;      /* NSEvent.buttonNumber (mouse) */
    double scrolling_delta_x;   /* NSEvent.scrollingDeltaX (scrollWheel) */
    double scrolling_delta_y;   /* NSEvent.scrollingDeltaY (scrollWheel) */
    uint64_t phase;             /* NSEvent.phase.rawValue (scrollWheel) */
    uint64_t momentum_phase;    /* NSEvent.momentumPhase.rawValue (scrollWheel) */
} StellarMacEvent;

#ifdef __cplusplus
static_assert(sizeof(StellarMacEvent) == 104, "StellarMacEvent layout");
#else
_Static_assert(sizeof(StellarMacEvent) == 104, "StellarMacEvent layout");
#endif

/* 프로젝트·문서 요청의 완료 event(stellar_runtime_poll_event의 out_kind와 payload).
 * 요청마다 정확히 하나가 옵니다. payload는 little-endian이고 u32 schema version
 * (STELLAR_EVENT_SCHEMA_VERSION)으로 시작합니다. str은 u32 길이 + UTF-8 바이트
 * (최대 STELLAR_EVENT_TEXT_MAX byte, 문자 경계에서 자름), id는 StableId 16 byte입니다.
 *   PROJECT_CREATED: request u64, git_state u32 (0 committed, 1 missing identity, 2 add failed, 3 commit failed)
 *   PROJECT_OPENED : request u64, project u64, project_id[16], name str, root str
 *   PROJECT_CLOSED : request u64, project u64, closed_documents u32
 *   DOCUMENT_OPENED: request u64, project u64, document u64, document_id[16], already_open u8
 *   DOCUMENT_CLOSED: request u64, document u64
 *   REQUEST_FAILED : request u64, failure u32 (STELLAR_FAILURE_*), message str
 *   WINDOW_SESSION_OPENED: request u64, project u64, session u64, window_id[16],
 *                          document_id[16] (작업 문서 없으면 0), generation u64
 *   WINDOW_SESSION_CLOSED: request u64, session u64
 *   WINDOW_VIEW_CHANGED  : request u64, session u64 (탭·active view·viewport·선택, 새 값은 상태 snapshot).
 *                          request 0은 요청 없는 변경(컴포넌트 원본 감시)이며 stellar_runtime_event_ready_fd를 깨웁니다.
 *   HISTORY_REPLAYED     : request u64, session u64, redo u8, applied u8, revision u64
 *   WINDOW_SESSION_SAVED : request u64, session u64, generation u64, written u8 */
#define STELLAR_EVENT_SCHEMA_VERSION 1
#define STELLAR_EVENT_TEXT_MAX 4096
#define STELLAR_EVENT_PROJECT_OPENED 0x0101
#define STELLAR_EVENT_PROJECT_CLOSED 0x0102
#define STELLAR_EVENT_DOCUMENT_OPENED 0x0103
#define STELLAR_EVENT_DOCUMENT_CLOSED 0x0104
#define STELLAR_EVENT_REQUEST_FAILED 0x0105
#define STELLAR_EVENT_WINDOW_SESSION_OPENED 0x0106
#define STELLAR_EVENT_WINDOW_SESSION_CLOSED 0x0107
#define STELLAR_EVENT_WINDOW_VIEW_CHANGED 0x0108
#define STELLAR_EVENT_HISTORY_REPLAYED 0x0109
#define STELLAR_EVENT_WINDOW_SESSION_SAVED 0x010a
#define STELLAR_EVENT_PROJECT_TRUST 0x010b
#define STELLAR_EVENT_PROJECT_CREATED 0x010d

/* 완료 event를 아직 꺼내지 않은 요청(실행 중 포함) 상한. 닿으면 새 요청은 STELLAR_QUEUE_FULL. */
#define STELLAR_EVENT_CAPACITY 1024

/* REQUEST_FAILED 실패 코드. */
#define STELLAR_FAILURE_UNKNOWN_PROJECT 1
#define STELLAR_FAILURE_UNKNOWN_DOCUMENT 2
#define STELLAR_FAILURE_DOCUMENT_IN_OTHER_PROJECT 3
#define STELLAR_FAILURE_NOT_A_REPOSITORY 10
#define STELLAR_FAILURE_NOT_A_STELLAR_PROJECT 11
#define STELLAR_FAILURE_MANIFEST_UNREADABLE 12
#define STELLAR_FAILURE_MANIFEST_CORRUPT 13
#define STELLAR_FAILURE_MANIFEST_INVALID 14
#define STELLAR_FAILURE_INVALID_PROJECT_ID 15
#define STELLAR_FAILURE_UNTRUSTED_SYMLINK 16
#define STELLAR_FAILURE_PATH_UNRESOLVED 17
#define STELLAR_FAILURE_UNKNOWN_WINDOW_SESSION 4
#define STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED 18
#define STELLAR_FAILURE_UNKNOWN_VIEW 19
#define STELLAR_FAILURE_NO_ACTIVE_VIEW 20
#define STELLAR_FAILURE_SAVE_STALE 21
#define STELLAR_FAILURE_SAVE_FAILED 22
#define STELLAR_FAILURE_HISTORY_UNAVAILABLE 23
#define STELLAR_FAILURE_INVALID_VIEWPORT 24
#define STELLAR_FAILURE_TRUST_UNAVAILABLE 25
#define STELLAR_FAILURE_PROJECT_CREATE_FAILED 27
#define STELLAR_FAILURE_PROJECT_CLONE_FAILED 28
#define STELLAR_FAILURE_CLONE_TARGET_EXISTS 29
#define STELLAR_FAILURE_CLONE_CANCELLED 30

/* 창 session 명령 코드(stellar_window_session_perform). 코어가 정하고 셸과 공유합니다. 셸은 메뉴 명령의 공유 이름
 * (edit.undo, edit.redo, file.save_now, view.zoom_in, view.zoom_out, view.zoom_actual_size)으로
 * stellar_command_code에 물어 코드를 얻습니다. 단축키는 셸이 운영체제 관례로 둡니다(macOS ⌘Z·⇧⌘Z·⌘S·⌘+·⌘-·⌘0).
 * 줌 명령은 active 탭 viewport만 바꾸고(×2·÷2·100%, 화면 중심 고정) WINDOW_VIEW_CHANGED를 보냅니다. */
#define STELLAR_COMMAND_UNDO 1
#define STELLAR_COMMAND_REDO 2
#define STELLAR_COMMAND_SAVE_NOW 3
#define STELLAR_COMMAND_ZOOM_IN 4
#define STELLAR_COMMAND_ZOOM_OUT 5
#define STELLAR_COMMAND_ZOOM_ACTUAL_SIZE 6

/* StellarRuntimeState 레이아웃 버전. 2: window_session_count 추가. */
#define STELLAR_RUNTIME_STATE_VERSION 2

/* 앱 runtime 최신 상태 snapshot 요약(고정 48 byte, 코어가 씀). */
typedef struct StellarRuntimeState {
    uint32_t version;         /* STELLAR_RUNTIME_STATE_VERSION */
    uint32_t reserved;        /* 0 */
    uint64_t generation;      /* snapshot 게시 세대(성공한 요청마다 1 증가, 0 = 아직 없음) */
    uint64_t project_count;   /* 열린 프로젝트 session 수 */
    uint64_t document_count;  /* 열린 문서 session 수 */
    uint64_t outstanding;     /* 완료 event를 아직 꺼내지 않은 요청 수(실행 중 포함) */
    uint64_t window_session_count; /* 열린 창 session 수 */
} StellarRuntimeState;

/* StellarDocumentInfo 레이아웃 버전. */
#define STELLAR_DOCUMENT_INFO_VERSION 1

/* 열린 문서 session 정보(고정 56 byte, 코어가 씀). */
typedef struct StellarDocumentInfo {
    uint32_t version;         /* STELLAR_DOCUMENT_INFO_VERSION */
    uint32_t reserved;        /* 0 */
    uint64_t document;        /* 문서 session handle */
    uint64_t project;         /* 소속 프로젝트 session handle */
    uint64_t revision;        /* snapshot 게시 시점의 문서 revision */
    uint64_t node_count;      /* snapshot 게시 시점의 node 수 */
    uint8_t document_id[16];  /* 문서 StableId(UUID 바이트 순서) */
} StellarDocumentInfo;

/* StellarWindowSessionInfo 레이아웃 버전. */
#define STELLAR_WINDOW_SESSION_INFO_VERSION 2

/* 열린 창 session 정보(고정 88 byte, 코어가 씀). 창 controller 하나가 창 session handle 하나를 소유합니다. */
typedef struct StellarWindowSessionInfo {
    uint32_t version;         /* STELLAR_WINDOW_SESSION_INFO_VERSION */
    uint32_t has_document;    /* 작업 문서가 있으면 1(저장한 적 없는 프로젝트는 0) */
    uint64_t session;         /* 창 session handle */
    uint64_t project;         /* 소속 프로젝트 session handle */
    uint64_t revision;        /* snapshot 게시 시점의 작업 문서 revision */
    uint64_t generation;      /* loaded generation 번호(저장한 적 없으면 0) */
    uint32_t undo_depth;      /* undo stack 깊이 */
    uint32_t redo_depth;      /* redo stack 깊이 */
    uint32_t selection_count; /* 선택된 node 수 */
    uint32_t has_unsaved_changes; /* 저장되지 않은 문서 변경 1, 선택·viewport 제외(v1 reserved) */
    uint8_t window_id[16];    /* 창 편집 session id(UUID 바이트 순서) */
    uint8_t document_id[16];  /* 작업 문서 StableId(없으면 0) */
} StellarWindowSessionInfo;

/* v1: 좌상단 원점 logical point. world/camera는 CGAffineTransform 순서 (a,b,c,d,tx,ty). */
typedef struct StellarCanvasColor { uint32_t space; float rgba[4]; } StellarCanvasColor; /* 0 없음, 1 sRGB, 2 Display P3 */
typedef struct StellarCanvasFrame {
    uint8_t id[16]; uint32_t parent, flags; /* parent: root UINT32_MAX, flags: bit0 clip children, bit1 hit target, bit2 Rectangle Shape, bit3 component instance, bit4 Text, bit5 Image, bit6 path Shape(Ellipse·Line·Polygon, stellar_canvas_shape_path) */
    float world[6], size[2], radii[4]; /* TL,TR,BR,BL */
    StellarCanvasColor fill, border; float border_width;
} StellarCanvasFrame;
typedef struct StellarCanvasInfo {
    uint64_t revision; uint8_t view[16], primary[16]; float camera[6]; uint32_t count, skipped;
} StellarCanvasInfo;

/* 읽기 전용 창 sidebar. parent가 0이면 active View root, after는 직전 페이지의 마지막 자식입니다. */
#define STELLAR_LAYER_PAGE_SIZE 64
typedef struct StellarLayerQuery { uint8_t view[16], parent[16], after[16]; uint64_t revision; } StellarLayerQuery;
typedef struct StellarLayerInfo {
    uint8_t page[16], primary[16], reveal_child[16];
    uint32_t count, more; size_t page_name_len, view_name_len, text_len;
} StellarLayerInfo;
typedef struct StellarLayerRow {
    uint8_t id[16]; uint16_t kind; uint8_t shape, flags; size_t name_len, selection_order; /* selection_order: 미선택 0, 선택 순번 1부터 */
} StellarLayerRow;

/* StellarWindowViewInfo 레이아웃 버전. */
#define STELLAR_WINDOW_VIEW_INFO_VERSION 1

/* 창 view state 정보(고정 56 byte, 코어가 씀): 탭 수, active view와 그 viewport(scene camera 입력). */
typedef struct StellarWindowViewInfo {
    uint32_t version;         /* STELLAR_WINDOW_VIEW_INFO_VERSION */
    uint32_t has_view;        /* active 탭이 있으면 1(없으면 아래 값은 0) */
    uint32_t tab_count;       /* 열린 탭 수 */
    uint32_t reserved;        /* 0 */
    double center_x;          /* 화면 중심의 문서 x */
    double center_y;          /* 화면 중심의 문서 y */
    double zoom;              /* 배율(1.0 = 100%, 0.02~256) */
    uint8_t active_view[16];  /* active view StableId */
} StellarWindowViewInfo;

#ifdef __cplusplus
static_assert(sizeof(StellarRuntimeState) == 48, "StellarRuntimeState layout");
static_assert(sizeof(StellarDocumentInfo) == 56, "StellarDocumentInfo layout");
static_assert(sizeof(StellarWindowSessionInfo) == 88, "StellarWindowSessionInfo layout");
static_assert(sizeof(StellarWindowViewInfo) == 56, "StellarWindowViewInfo layout");
#else
_Static_assert(sizeof(StellarRuntimeState) == 48, "StellarRuntimeState layout");
_Static_assert(sizeof(StellarDocumentInfo) == 56, "StellarDocumentInfo layout");
_Static_assert(sizeof(StellarWindowSessionInfo) == 88, "StellarWindowSessionInfo layout");
_Static_assert(sizeof(StellarWindowViewInfo) == 56, "StellarWindowViewInfo layout");
#endif

/* runtime을 생성하고 handle을 out_handle에 씁니다. */
StellarStatus stellar_runtime_create(uint64_t *out_handle);

/* runtime을 파괴합니다(알 수 없는 handle은 STELLAR_INVALID_HANDLE). */
StellarStatus stellar_runtime_destroy(uint64_t handle);

/* 버전 문자열을 호출자 buffer에 UTF-8로 씁니다(buf=NULL이면 길이만 보고). */
StellarStatus stellar_runtime_version(uint64_t handle, uint8_t *buf, size_t buf_len, size_t *out_written);

/* handle별 마지막 오류 메시지(없으면 STELLAR_NO_ERROR). */
StellarStatus stellar_last_error(uint64_t handle, uint8_t *buf, size_t buf_len, size_t *out_written);

/* handle label 설정(입력: UTF-8 ptr+len, NUL 종료 비의존). */
StellarStatus stellar_runtime_set_label(uint64_t handle, const uint8_t *ptr, size_t len);

/* handle label 조회(출력: 호출자 buffer). */
StellarStatus stellar_runtime_label(uint64_t handle, uint8_t *buf, size_t buf_len, size_t *out_written);

/* 이벤트 큐에서 다음 이벤트를 pull 방식으로 읽습니다(FIFO, 빈 큐는 STELLAR_NO_ERROR, 프로젝트·문서 완료 event 포함). */
StellarStatus stellar_runtime_poll_event(uint64_t handle, uint32_t *out_kind, uint8_t *buf, size_t buf_len, size_t *out_written);

/* 요청 없는 event(request 0)가 쌓이면 읽을 수 있게 되는 비차단 1 byte datagram 신호 fd(호출자 소유, 실패 시 -1).
 * 깨어나면 남은 신호를 모두 읽고 stellar_runtime_poll_event로 event를 꺼냅니다. 요청한 event는 신호를 쓰지 않습니다. */
int32_t stellar_runtime_event_ready_fd(uint64_t handle);

/* 셸이 계산한 drawable(backing) 픽셀 크기와 backing scale을 코어에 전달합니다. */
StellarStatus stellar_runtime_set_drawable_size(uint64_t handle, uint32_t pixel_width, uint32_t pixel_height, double scale);

/* 마지막으로 전달된 drawable 픽셀 크기와 scale을 읽습니다(미설정 시 STELLAR_NO_ERROR). */
StellarStatus stellar_runtime_drawable_size(uint64_t handle, uint32_t *out_pixel_width, uint32_t *out_pixel_height, double *out_scale);

/* macOS NSEvent 원시 필드를 공통 입력 이벤트로 변환해 입력 대기열에 넣습니다(out_queued: 반영 수 0 또는 1, 연속 move/wheel 병합 포함). */
StellarStatus stellar_runtime_submit_mac_event(uint64_t handle, const StellarMacEvent *event, uint32_t *out_queued);

/* 입력 대기열의 다음 공통 입력 이벤트 직렬화 바이트를 읽습니다(FIFO, 빈 대기열은 STELLAR_NO_ERROR). */
StellarStatus stellar_runtime_poll_input(uint64_t handle, uint8_t *buf, size_t buf_len, size_t *out_written);

/* 프로젝트 폴더 열기를 요청합니다(입력: 절대 경로 UTF-8 ptr+len). 기다리지 않고 request id를 씁니다(실패 시 0). */
StellarStatus stellar_project_open(uint64_t handle, const uint8_t *path, size_t path_len, uint64_t *out_request);

/* 새 폴더 생성. 완료 event 뒤 같은 path로 stellar_project_open을 호출할 수 있습니다. */
StellarStatus stellar_project_create(uint64_t handle, const uint8_t *path, size_t path_len, uint64_t *out_request);
/* Clone to a new target and open in restricted mode. Completion: PROJECT_OPENED or REQUEST_FAILED.
 * url and target are UTF-8 and bounded by STELLAR_EVENT_TEXT_MAX. No remote URL is included in events. */
StellarStatus stellar_project_clone(uint64_t handle, const uint8_t *url, size_t url_len, const uint8_t *target, size_t target_len, uint64_t *out_request);
StellarStatus stellar_project_clone_cancel(uint64_t handle, uint64_t request);

/* 창의 컴포넌트 원본을 worker에서 다시 읽고 완료 WINDOW_VIEW_CHANGED를 보냅니다. 신뢰된 프로젝트면 코어가
 * 그 프로젝트의 원본 감시를 시작하고, 이후 원본이 바뀌면 요청 없는 WINDOW_VIEW_CHANGED(request 0)를 보냅니다. */
StellarStatus stellar_window_reload_component_canvas(uint64_t handle, uint64_t session, uint64_t *out_request);

/* 속성 패널(UX.2.2, ADR-0070). 확정 선택의 CSS 속성 행을 게시 snapshot에서 읽습니다. 텍스트는 이름(단일 선택이면 레이어
 * 이름, 선택이 없으면 "Page · View") 뒤에 행마다 이름·값·선택지(공백 구분)가 이어집니다. 용량이 모자라면
 * BUFFER_TOO_SMALL과 함께 info에 필요한 크기를 씁니다. */
#define STELLAR_INSPECTOR_MIXED        0x0001u
#define STELLAR_INSPECTOR_READ_ONLY    0x0002u
#define STELLAR_INSPECTOR_COLOR        0x0004u
#define STELLAR_INSPECTOR_GROUP        0x0008u
#define STELLAR_INSPECTOR_LONGHAND     0x0010u
#define STELLAR_INSPECTOR_STELLAR_ONLY 0x0020u
#define STELLAR_INSPECTOR_NUMERIC      0x0040u
#define STELLAR_INSPECTOR_KEYWORD      0x0080u
#define STELLAR_INSPECTOR_BOOL         0x0100u
typedef struct StellarInspectorInfo {
    uint32_t version, count, classes, row_count;
    uint64_t revision, history_sequence;
    uint8_t view[16];
    uint32_t name_len, text_len;
} StellarInspectorInfo;
typedef struct StellarInspectorRow { uint32_t key, flags, section, name_len, value_len, choices_len; } StellarInspectorRow;
StellarStatus stellar_window_inspector(uint64_t handle, uint64_t session, StellarInspectorInfo *info, StellarInspectorRow *rows, size_t row_capacity, uint8_t *text, size_t text_capacity);
/* 확정 선택 전체에 속성 하나를 적용합니다. mode 0은 CSS 값 텍스트, 1은 각 node 현재 값에 delta를 더함.
 * phase 0 확정, 1 연속 중간 값(포인터 scrub preview), 2 연속 끝(Undo 한 항목), 3 연속 취소(열린 preview를 늘 닫음),
 * 4 반복 입력(화살표·색, 바로 기록하고 같은 속성 0.5초 안 반복은 한 항목). 값 문법 오류는 요청 없이
 * INVALID_ARGUMENT와 "code: 이유" 진단을 씁니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED. */
typedef struct StellarPropertyEdit {
    uint8_t view[16];
    uint64_t revision, history_sequence;
    double delta;
    uint32_t mode, phase;
} StellarPropertyEdit;
StellarStatus stellar_window_edit_property(uint64_t handle, uint64_t session, const StellarPropertyEdit *edit, const uint8_t *name, size_t name_len, const uint8_t *value, size_t value_len, uint64_t *out_request, uint8_t *diagnostic, size_t diagnostic_capacity, size_t *out_diagnostic_len);

/* 프로젝트 session 닫기를 요청합니다(그 아래 문서도 닫음). */
StellarStatus stellar_project_close(uint64_t handle, uint64_t project, uint64_t *out_request);

/* 프로젝트 session 아래에서 문서 열기를 요청합니다(document_id: StableId 16 byte). */
StellarStatus stellar_document_open(uint64_t handle, uint64_t project, const uint8_t *document_id, uint64_t *out_request);

/* 문서 session 닫기를 요청합니다. */
StellarStatus stellar_document_close(uint64_t handle, uint64_t document, uint64_t *out_request);

/* 최신 상태 snapshot 요약을 씁니다(document thread를 기다리지 않음). */
StellarStatus stellar_runtime_state(uint64_t handle, StellarRuntimeState *out_state);

/* 열린 문서 session의 최신 snapshot 정보를 씁니다(없는 문서는 STELLAR_INVALID_ARGUMENT). */
StellarStatus stellar_document_info(uint64_t handle, uint64_t document, StellarDocumentInfo *out_info);

/* 프로젝트 session 아래에서 창 session 열기를 요청합니다(current generation·작업 문서 로드). */
StellarStatus stellar_window_session_open(uint64_t handle, uint64_t project, uint64_t *out_request);

/* 열린 project의 신뢰 등록을 확인합니다. action 0 조회, 1 사용자 승인, 2 사용자 철회.
 * PROJECT_TRUST event: request u64, project u64, trusted u8. */
StellarStatus stellar_project_trust(uint64_t handle, uint64_t project, uint32_t action, uint64_t *out_request);

/* 창 session 닫기를 요청합니다(문서 state·history·선택 폐기, 저장하지 않음). */
StellarStatus stellar_window_session_close(uint64_t handle, uint64_t session, uint64_t *out_request);

/* 공유 command 이름(UTF-8 ptr+len)의 코어 명령 코드를 씁니다. 코어 명령이 아니면 STELLAR_NO_ERROR와 0(handle 불필요). */
StellarStatus stellar_command_code(const uint8_t *name, size_t name_len, uint32_t *out_code);

/* 창 session에 명령(STELLAR_COMMAND_*)을 보냅니다. 모르는 코드는 STELLAR_INVALID_ARGUMENT(요청 없음). */
StellarStatus stellar_window_session_perform(uint64_t handle, uint64_t session, uint32_t command, uint64_t *out_request);

/* 실제 canvas 크기(logical point)로 active View의 root Frame이 화면에 들어오도록 카메라를 맞춥니다(100% 이하).
 * 새 창의 첫 표시와 View ▸ Zoom to Fit이 씁니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED. */
StellarStatus stellar_window_session_fit_root(uint64_t handle, uint64_t session, double width, double height, uint64_t *out_request);
/* Hand 도구 드래그: active viewport 내용을 화면 point만큼 옮깁니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED. */
StellarStatus stellar_window_session_pan(uint64_t handle, uint64_t session, double dx, double dy, uint64_t *out_request);
/* 창 하나를 대상으로 NSEvent 원시 필드를 제출합니다. 해석은 코어가 합니다. 수식키 없는 스크롤은 그 창의 팬이며
 * 새 팬 요청이면 out_request에 id(완료 WINDOW_VIEW_CHANGED), 대기 중인 팬에 더했거나 이동량이 0이면 0입니다.
 * 그 밖의 이벤트는 stellar_runtime_submit_mac_event와 같이 입력 대기열에 넣고 out_request는 0입니다. */
StellarStatus stellar_window_session_submit_mac_event(uint64_t handle, uint64_t session, const StellarMacEvent *event, uint64_t *out_request);
/* Click: fit_factor=0 and zoom_out=0/1. Area: fit_factor>0 centers the anchor and scales the viewport. */
StellarStatus stellar_window_session_zoom_at(uint64_t handle, uint64_t session, double x, double y, double fit_factor, uint32_t zoom_out, uint64_t *out_request);
/* ADR-0071: 화면 중심에서 (x, y) point인 점을 고정하고 배율을 factor(> 0)배 합니다(핀치, ⌘ 스크롤). history 없음. */
StellarStatus stellar_window_session_zoom_by(uint64_t handle, uint64_t session, double x, double y, double factor, uint64_t *out_request);
/* 한 게시 시점의 선택. 기존 창·뷰 info의 고정 레이아웃과 독립적입니다. */
typedef struct StellarSelectionTarget {
    uint64_t session;
    uint64_t revision;
    uint64_t history_sequence;
    uint64_t selection_count;
    uint8_t view[16];
} StellarSelectionTarget;
#define STELLAR_SELECTION_DELETE 1
#define STELLAR_SELECTION_DUPLICATE 2
#define STELLAR_SELECTION_FORWARD 3
#define STELLAR_SELECTION_BACKWARD 4
#define STELLAR_SELECTION_FRONT 5
#define STELLAR_SELECTION_BACK 6
#define STELLAR_ORDER_CAN_FORWARD 1
#define STELLAR_ORDER_CAN_BACKWARD 2
#define STELLAR_ORDER_FLOW_PARENT 4
uint32_t stellar_window_layer_order_flags(uint64_t handle, const StellarSelectionTarget *target);
/* 활성 View가 없으면 STELLAR_NO_ERROR, 열려 있지 않은 창이면 STELLAR_INVALID_ARGUMENT입니다. */
StellarStatus stellar_window_selection_target(uint64_t handle, uint64_t session, StellarSelectionTarget *out);
/* NULL/0은 위 target의 선택 전체, ids가 있으면 순서와 무관하게 실제 선택 전체와 일치해야 합니다. */
StellarStatus stellar_window_edit_selection(uint64_t handle, const StellarSelectionTarget *target, uint32_t action, const uint8_t *ids, size_t count, uint64_t *out_request);
/* Place selected, decoded PNG bytes by SHA-256. Read validated image content on a worker. */
StellarStatus stellar_window_place_png(uint64_t handle, const StellarSelectionTarget *target, const uint8_t parent[16], float x, float y, const uint8_t *path, size_t path_len, const uint8_t digest[32], uint64_t *out_request);
StellarStatus stellar_window_canvas_png(uint64_t handle, uint64_t session, uint64_t revision, uint32_t row, const uint8_t node[16], uint8_t digest[32], uint8_t *buf, size_t capacity, size_t *out_len);
/* 셸 worker 전용 Copy: 표시한 단일 선택 subtree를 caller buffer에 씁니다(최대 16 MiB). 복사할 수 없으면 NO_ERROR와
 * 길이 0, 출력 부족은 BUFFER_TOO_SMALL과 필요한 길이입니다. Paste: 같은 문서의 subtree bytes(1 byte~16 MiB)를
 * 실행 시점의 선택에서 정한 목적지에 넣습니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED입니다. */
StellarStatus stellar_window_copy_selection(uint64_t handle, const StellarSelectionTarget *target, uint8_t *bytes, size_t capacity, size_t *out_len);
StellarStatus stellar_window_paste_selection(uint64_t handle, const StellarSelectionTarget *target, const uint8_t *bytes, size_t len, uint64_t *out_request);
/* 화면에 표시한 View·revision에서 선택한 Frame을 부모 좌표계의 delta만큼 이동합니다. */
StellarStatus stellar_window_session_move_frame(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, float dx, float dy, uint64_t *out_request);
/* 한 게시 snapshot의 선택과 단일 선택의 Inspector 값(고정 104 byte). core가 문서에서 읽습니다.
 * kind: NodeKind 저장 tag(단일 선택이 아니면 0), shape: ShapeKind tag(Shape가 아니면 0).
 * flags bit 0: width·height·radii(TL·TR·BR·BL)·background가 유효한 Frame, bit 1: Inspector 편집을 받을 수 있음.
 * 활성 View가 없으면 STELLAR_NO_ERROR와 0 값, 열려 있지 않은 창이면 STELLAR_INVALID_ARGUMENT입니다. */
typedef struct StellarSelectedNode {
    uint64_t revision, history_sequence, selection_count;
    uint8_t view[16], node[16];
    uint16_t kind; uint8_t shape, flags;
    float width, height, radii[4];
    StellarCanvasColor background;
} StellarSelectedNode;
#define STELLAR_SELECTED_FRAME 1
#define STELLAR_SELECTED_EDITABLE 2
StellarStatus stellar_window_selected_node(uint64_t handle, uint64_t session, StellarSelectedNode *out);
/* Inspector가 Frame 속성 하나를 바꾸는 값. 길이 key는 length, BACKGROUND(0x0104)는 color(space 0 = 제거)를 씁니다. */
typedef struct StellarFrameValue { float length; StellarCanvasColor color; } StellarFrameValue;
/* 선택 Frame의 CSS longhand 하나: WIDTH 0x0108, HEIGHT 0x0109, 모서리 0x010a–0x010d, BACKGROUND 0x0104.
 * 표시한 View·revision·history_sequence가 실행 시점과 같고 편집 가능한 Frame일 때만 한 Undo 항목으로 기록합니다. */
StellarStatus stellar_window_session_set_frame_property(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, uint64_t history_sequence, uint32_t property_key, StellarFrameValue value, uint64_t *out_request);
/* 표시한 부모 Frame의 local rect에 자식 Free Frame을 삽입하고 한 history entry로 선택합니다. */
StellarStatus stellar_window_session_insert_frame(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t parent[16], uint64_t revision, float x, float y, float width, float height, uint64_t *out_request);
/* ADR-0071: 새 도형의 칠(D7). 셸이 앱 설정 값을 넘기며 코어는 기본 색을 두지 않습니다.
 * 색공간 0은 없음. stroke가 있으면 stroke_width > 0. Line은 stroke, 닫힌 도형은 fill 또는 stroke가 필요합니다. */
typedef struct StellarShapeStyle { StellarCanvasColor fill; StellarCanvasColor stroke; float stroke_width; } StellarShapeStyle;
/* ShapeKind tags: 1 Rectangle, 2 Ellipse, 3 Line, 4 Polygon. Coordinates: ordered parent-local endpoints. */
StellarStatus stellar_window_session_insert_shape(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t parent[16], uint64_t revision, uint8_t shape, float x0, float y0, float x1, float y1, const StellarShapeStyle *style, uint64_t *out_request);
/* 창 session의 선택을 ids(StableId 16 byte × count)로 바꾸고 그 창 history에 기록합니다(count 0 = 비움). */
StellarStatus stellar_window_session_select(uint64_t handle, uint64_t session, const uint8_t *ids, size_t count, uint64_t *out_request);

/* 활성 View의 같은 게시 snapshot에서 카메라와 painter 순서 목록 조회. rows=NULL이면 metadata만 씁니다.
 * 용량 부족 시 최신 count와 STELLAR_BUFFER_TOO_SMALL, active View 없음은 STELLAR_NO_ERROR. */
typedef struct StellarCanvasTextQuery { uint8_t view[16], node[16]; uint64_t revision; uint32_t row; } StellarCanvasTextQuery;
typedef struct StellarCanvasTextElement { uint32_t op; float points[6]; } StellarCanvasTextElement;
/* Shared canvas row query and Move/Line/Cubic/Close elements (op 1–4). For a row with flags bit 6,
 * cap_flags = cap(0 butt, 1 round, 2 square); op 5 is a zero-length Line cap rect (xywh). Null rows query count. */
StellarStatus stellar_canvas_shape_path(uint64_t handle, uint64_t session, StellarCanvasTextQuery query, StellarCanvasTextElement *rows, size_t capacity, size_t *count, uint32_t *cap_flags);
/* Saved text: local outline commands 1..4, line bounds 5, end of unique outline 6,
 * placement 7 with index into the definitions and points[0..2] = pen position. */
typedef struct StellarCanvasTextPart { uint32_t op, index; float points[6]; } StellarCanvasTextPart;
StellarStatus stellar_window_text_parts(uint64_t handle, uint64_t session, StellarCanvasTextQuery query, StellarCanvasTextPart *rows, size_t capacity, size_t *count, uint64_t *paint_token);
StellarStatus stellar_window_text_hit(uint64_t handle, uint64_t session, StellarCanvasTextQuery query, float x, float y, uint32_t *offset, uint32_t *upstream);
StellarStatus stellar_window_text_rects(uint64_t handle, uint64_t session, StellarCanvasTextQuery query, uint32_t start, uint32_t end, uint32_t upstream, float (*rects)[4], size_t capacity, size_t *count);
StellarStatus stellar_window_canvas(uint64_t handle, uint64_t session, float width, float height, StellarCanvasInfo *info, StellarCanvasFrame *rows, size_t capacity);

/* 캔버스 Text 편집(ADR-0067 §6). 조합 상태는 코어 창 session이 소유하고, 셸은 begin의 out_request(token)만 씁니다.
 * begin·mark·commit·cancel·replace는 비동기 요청이며 완료는 event queue로 옵니다. 범위는 문서 Unicode scalar입니다.
 * begin: new_text 0은 선택한 Text(target), 1은 부모 Frame(target)의 local (x, y)에 확정 때 만들 새 Text입니다.
 * out_node는 조합 대상 node(새 Text는 확정 때 쓸 ID)입니다. commit은 본문이 같으면 history를 늘리지 않습니다.
 * content는 셸 worker 전용입니다. buf가 NULL이면 out_len만, 모자라면 STELLAR_BUFFER_TOO_SMALL을 돌려줍니다. */
StellarStatus stellar_text_begin(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t target[16], uint64_t revision, uint32_t new_text, float x, float y, uint32_t start, uint32_t end, uint8_t out_node[16], uint64_t *out_request);
StellarStatus stellar_text_mark(uint64_t handle, uint64_t session, uint64_t token, const uint8_t *text, size_t len, uint64_t *out_request);
StellarStatus stellar_text_commit(uint64_t handle, uint64_t session, uint64_t token, const uint8_t *text, size_t len, uint64_t *out_request);
StellarStatus stellar_text_cancel(uint64_t handle, uint64_t session, uint64_t token, uint64_t *out_request);
StellarStatus stellar_text_replace(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, uint32_t start, uint32_t end, const uint8_t *text, size_t len, uint64_t *out_request);
StellarStatus stellar_text_content(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, uint8_t *buf, size_t capacity, size_t *out_len);

/* 표시한 View·revision의 레이어 이름을 UTF-8로 바꿉니다. 코어가 View 소속과 조상 잠금을 확인하고 RenameNode 하나를
 * 창 history에 기록합니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED입니다. */
StellarStatus stellar_window_layer_rename(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, const uint8_t *name, size_t name_len, uint64_t *out_request);

/* ADR-0071 캔버스 직접 조작. 셸은 그린 행·world 값만 넘기고 선택·편집 의미는 코어가 정합니다. */
#define STELLAR_PICK_TOGGLE 1
#define STELLAR_PICK_DEEP 2
#define STELLAR_PICK_ENTER 4
#define STELLAR_AREA_CONTAIN 1
#define STELLAR_AREA_ADD 2
/* leaf: 포인터 아래 가장 위 행의 node(NULL = 모든 Frame 밖). 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED. */
StellarStatus stellar_window_canvas_pick(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t *leaf, uint32_t flags, uint64_t *out_request);
/* world 영역의 두 꼭짓점. root의 직계 자식 가운데 겹치는(CONTAIN이면 완전히 포함된) node를 고릅니다. */
StellarStatus stellar_window_canvas_pick_area(uint64_t handle, uint64_t session, const uint8_t view[16], float x0, float y0, float x1, float y1, uint32_t flags, uint64_t *out_request);
#define STELLAR_MOVE_DUPLICATE 1
#define STELLAR_MOVE_PARENT_SPACE 2
/* 선택 전부를 (dx, dy)만큼 옮깁니다(기본 world, PARENT_SPACE는 부모 공간). anchor는 드래그를 시작한 행(키 이동은 NULL).
 * DUPLICATE는 복제본을 원본 바로 위에 만들어 옮기고 선택합니다. 한 호출이 Undo 한 번입니다. */
StellarStatus stellar_window_canvas_move(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t *anchor, float dx, float dy, uint32_t flags, uint64_t *out_request);
#define STELLAR_CANVAS_ITEM_MOVABLE 1
#define STELLAR_CANVAS_ITEM_RESIZABLE 2
#define STELLAR_CANVAS_ITEM_RADIUS 4
#define STELLAR_CANVAS_ITEM_RADIUS_PER_CORNER 8
#define STELLAR_CANVAS_ITEM_FIXED_WIDTH 16
#define STELLAR_CANVAS_ITEM_FIXED_HEIGHT 32
/* kind: 1 Frame, 2 Rectangle, 3 Ellipse, 4 Line, 5 Polygon, 6 Text, 7 Image, 8 Instance.
 * world: local→world (a b c d tx ty). rect: local x y w h. radii: tl tr br bl. line: Line의 local 시작·끝점(x0 y0 x1 y1). */
typedef struct StellarCanvasSelectionItem { uint8_t id[16]; uint32_t kind, flags; float world[6], rect[4], radii[4], line[4]; } StellarCanvasSelectionItem;
/* 하나뿐인 선택 node의 상자를 지금 local 좌표 (x, y, w, h)로 바꿉니다(크기 핸들, Undo 한 번, 크기 > 0). */
StellarStatus stellar_window_canvas_resize(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], float x, float y, float width, float height, uint64_t *out_request);
/* 하나뿐인 선택 node의 모서리 반경(corner -1 네 모서리, 0–3 tl tr br bl 하나). Frame·Rectangle은 border-*-radius, Polygon은 값 하나. */
StellarStatus stellar_window_canvas_radius(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], int32_t corner, float radius, uint64_t *out_request);
/* 하나뿐인 선택 Line의 끝점 하나(end 0 시작, 1 끝)를 local (x, y)로 옮깁니다(Undo 한 번). */
StellarStatus stellar_window_canvas_line_end(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint32_t end, float x, float y, uint64_t *out_request);
/* 최신 게시 snapshot의 선택 상자. items=NULL이면 수만, 용량 부족은 BUFFER_TOO_SMALL과 필요한 수. */
StellarStatus stellar_window_canvas_selection(uint64_t handle, uint64_t session, StellarCanvasSelectionItem *items, size_t capacity, size_t *out_count, uint64_t *out_revision);

/* 셸 worker 전용. 활성 View/revision의 직계 자식을 앞에서부터 조회합니다.
 * rows: STELLAR_LAYER_PAGE_SIZE개 caller buffer. 플래그 bit 0 표시, 1 잠금, 2 자식 있음.
 * UTF-8 text는 페이지·View·행 이름 순서. BUFFER_TOO_SMALL은 info.text_len만큼 확보해 재호출합니다.
 * out_history(필수)에는 행의 선택 순서와 같은 시점의 창 history sequence를 씁니다. 읽지 못하면 0입니다. */
StellarStatus stellar_window_layer_page(uint64_t handle, uint64_t session, const StellarLayerQuery *query, StellarLayerInfo *info, StellarLayerRow *rows, uint8_t *text, size_t capacity, uint64_t *out_history);

/* 열린 창 session의 view state(탭 수, active view, viewport)를 씁니다(없는 창 session은 STELLAR_INVALID_ARGUMENT). */
StellarStatus stellar_window_view_info(uint64_t handle, uint64_t session, StellarWindowViewInfo *out_info);

/* 열린 창 session의 최신 snapshot 정보를 씁니다(없는 창 session은 STELLAR_INVALID_ARGUMENT). */
StellarStatus stellar_window_session_info(uint64_t handle, uint64_t session, StellarWindowSessionInfo *out_info);

/* 비정상 종료 복구(§22.3). probe는 V.9.3 규칙(기준 세대 = current, 미저장 revision 있음)에 맞는 복구본 하나를 고릅니다. */
#define STELLAR_RECOVERY_CANDIDATE_VERSION 1
#define STELLAR_RECOVERY_RESTORE 1 /* 이 창 session 메모리에 복원(디스크는 쓰지 않음) */
#define STELLAR_RECOVERY_DISCARD 2 /* 복구본 삭제 */
typedef struct StellarRecoveryCandidate {
    uint32_t version;          /* STELLAR_RECOVERY_CANDIDATE_VERSION */
    uint32_t has_candidate;    /* 제안할 복구본이 있으면 1(없으면 나머지 0) */
    uint8_t window_id[16];     /* 복구본을 남긴 창 편집 session id */
    uint8_t content_hash[32];  /* 복구본 파일 bytes의 SHA-256 */
} StellarRecoveryCandidate;
#ifdef __cplusplus
static_assert(sizeof(StellarRecoveryCandidate) == 56, "StellarRecoveryCandidate layout");
#else
_Static_assert(sizeof(StellarRecoveryCandidate) == 56, "StellarRecoveryCandidate layout");
#endif
/* 파일을 읽으므로 UI thread 밖에서 부릅니다. 없으면 has_candidate 0, 저장소 읽기 실패는 STELLAR_INTERNAL과 last error. */
StellarStatus stellar_window_recovery_probe(uint64_t handle, uint64_t session, StellarRecoveryCandidate *out_candidate);
/* probe 결과에 RESTORE 또는 DISCARD를 적용합니다(같은 bytes일 때만). 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED. */
StellarStatus stellar_window_recovery_apply(uint64_t handle, uint64_t session, const StellarRecoveryCandidate *candidate, uint32_t action, uint64_t *out_request);

/* 살아 있는 runtime handle 수(진단용). */
uint64_t stellar_runtime_live_count(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* STELLAR_FFI_H */
"#;

/// 헤더를 생성하거나(`check == false`) 디스크 내용을 템플릿과 비교합니다(`check == true`).
///
/// 반환값: 생성 시 항상 `true`. 검사 시 디스크 내용이 템플릿과 일치하면 `true`, 다르면 `false`.
pub fn generate_header(repo_root: &Path, check: bool) -> Result<bool, String> {
    let path = repo_root.join(HEADER_PATH);
    if check {
        let current =
            std::fs::read_to_string(&path).map_err(|e| format!("읽기 실패 {HEADER_PATH}: {e}"))?;
        Ok(current == HEADER)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("디렉터리 생성 실패: {e}"))?;
        }
        std::fs::write(&path, HEADER).map_err(|e| format!("쓰기 실패 {HEADER_PATH}: {e}"))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).to_path_buf()
    }

    #[test]
    fn committed_header_matches_template() {
        // 생성물 drift 검사: 디스크의 헤더가 고정 템플릿과 일치해야 합니다(재생성 후 diff 없음).
        assert!(generate_header(&root(), true).expect("헤더 읽기"));
    }

    #[test]
    fn header_exposes_each_extern_fn_and_guard() {
        for needle in [
            "#ifndef STELLAR_FFI_H",
            "stellar_runtime_create",
            "stellar_runtime_destroy",
            "stellar_runtime_version",
            "stellar_last_error",
            "stellar_runtime_set_label",
            "stellar_runtime_label",
            "stellar_runtime_poll_event",
            "stellar_runtime_submit_mac_event",
            "stellar_runtime_poll_input",
            "typedef struct StellarMacEvent",
            "STELLAR_MAC_EVENT_VERSION",
            "stellar_runtime_live_count",
        ] {
            assert!(HEADER.contains(needle), "헤더에 {needle} 누락");
        }
        // 내부 전용 Rust API는 C 헤더에 노출하지 않습니다.
        assert!(!HEADER.contains("enqueue_event"));
    }
}
