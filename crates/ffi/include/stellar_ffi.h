/* stellar_ffi.h — Project Stellar C ABI.
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
 *   WINDOW_VIEW_CHANGED  : request u64, session u64 (탭·active view·viewport·선택, 새 값은 상태 snapshot)
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
#define STELLAR_EVENT_GIT_NAMES 0x010c
#define STELLAR_EVENT_PROJECT_CREATED 0x010d
/* request u64, session u64, filename text */
#define STELLAR_EVENT_COMPONENT_CREATED 0x010e

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
#define STELLAR_FAILURE_GIT_NAMES_UNAVAILABLE 26
#define STELLAR_FAILURE_PROJECT_CREATE_FAILED 27

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
    uint8_t id[16]; uint32_t parent, flags; /* parent: root UINT32_MAX, flags: bit0 clip children, bit1 hit target, bit2 Rectangle Shape, bit3 component instance */
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

/* Read fresh source components on a worker. Output is a tagged little-endian buffer:
 * 0, u32 legacy chunk count, u32 row count, rows (UUID, root UUID, u32 nodes, f32 width, f32 height,
 * length-prefixed UTF-8 name, filename, hash); or 1, length-prefixed UTF-8 error.
 * out_len reports required bytes. STELLAR_BUFFER_TOO_SMALL means retry with a larger buffer. */
/* Queues a background component source read and a WINDOW_VIEW_CHANGED event. */
StellarStatus stellar_window_reload_component_canvas(uint64_t handle, uint64_t session, uint64_t *out_request);
StellarStatus stellar_window_component_library(uint64_t handle, uint64_t session, uint8_t *buf, size_t capacity, size_t *out_len);

/* Conditional root-size edit on a worker. id is 16 bytes; hash is the source SHA-256 shown by the library.
 * Pass an error buffer of STELLAR_EVENT_TEXT_MAX or more before calling. Non-OK may report UTF-8 detail. */
StellarStatus stellar_window_component_resize(uint64_t handle, uint64_t session, const uint8_t *id, const uint8_t *hash, size_t hash_len, float width, float height, uint8_t *error, size_t capacity, size_t *out_len);

/* 열린 신뢰 창의 프로젝트에 독립 컴포넌트 원본 한 개 생성. 완료는 COMPONENT_CREATED 또는 REQUEST_FAILED. */
StellarStatus stellar_window_component_create(uint64_t handle, uint64_t session, const uint8_t *name, size_t name_len, uint64_t *out_request);
typedef struct StellarFrameSelection { uint64_t revision; uint8_t view[16], node[16]; } StellarFrameSelection;
StellarStatus stellar_window_selected_frame(uint64_t handle, uint64_t session, StellarFrameSelection *out);
StellarStatus stellar_window_component_from_selection(uint64_t handle, uint64_t session, const uint8_t *name, size_t name_len, const uint8_t view[16], const uint8_t node[16], uint64_t revision, uint64_t *out_request);
StellarStatus stellar_window_place_component(uint64_t handle, uint64_t session, const uint8_t view[16], uint64_t revision, const uint8_t component[16], const uint8_t *hash, size_t hash_len, float x, float y, uint64_t *out_request);

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

/* action 0: refresh, 1: stage displayed row, 2: unstage displayed row. Event: request u64, session u64, rows u32, other_changes u8, selection_applied u8.
 * A row contains document and node IDs (16 bytes each), staged u8, and HEAD/index/working names
 * (each u32 little-endian UTF-8 byte length followed by those bytes). No Git I/O occurs in the row getter. */
StellarStatus stellar_git_names_request(uint64_t handle, uint64_t session, uint32_t action, uint64_t generation, uint32_t row, uint64_t *out_request);
StellarStatus stellar_git_names_row(uint64_t handle, uint64_t session, uint64_t generation, uint32_t row, uint8_t *buf, size_t capacity, size_t *out_len);
/* Component row: component ID (16 bytes), staged u8, unstaged u8, then path and HEAD/index/working
 * SHA-256 strings (each u32 little-endian length + UTF-8, empty means absent). */
StellarStatus stellar_git_component_row(uint64_t handle, uint64_t session, uint64_t generation, uint32_t row, uint8_t *buf, size_t capacity, size_t *out_len);
StellarStatus stellar_git_names_cancel(uint64_t handle, uint64_t session, uint64_t request);

/* 창 session 닫기를 요청합니다(문서 state·history·선택 폐기, 저장하지 않음). */
StellarStatus stellar_window_session_close(uint64_t handle, uint64_t session, uint64_t *out_request);

/* 공유 command 이름(UTF-8 ptr+len)의 코어 명령 코드를 씁니다. 코어 명령이 아니면 STELLAR_NO_ERROR와 0(handle 불필요). */
StellarStatus stellar_command_code(const uint8_t *name, size_t name_len, uint32_t *out_code);

/* 창 session에 명령(STELLAR_COMMAND_*)을 보냅니다. 모르는 코드는 STELLAR_INVALID_ARGUMENT(요청 없음). */
StellarStatus stellar_window_session_perform(uint64_t handle, uint64_t session, uint32_t command, uint64_t *out_request);

/* 새 창의 실제 canvas 크기(logical point)로 active View의 root를 맞춥니다. 완료는 WINDOW_VIEW_CHANGED. */
StellarStatus stellar_window_session_fit_root(uint64_t handle, uint64_t session, double width, double height, uint64_t *out_request);
/* Pan active viewport content in screen points; completes with WINDOW_VIEW_CHANGED or REQUEST_FAILED. */
StellarStatus stellar_window_session_pan(uint64_t handle, uint64_t session, double dx, double dy, uint64_t *out_request);
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
/* 활성 View가 없으면 STELLAR_NO_ERROR, 열려 있지 않은 창이면 STELLAR_INVALID_ARGUMENT입니다. */
StellarStatus stellar_window_selection_target(uint64_t handle, uint64_t session, StellarSelectionTarget *out);
/* NULL/0은 위 target의 선택 전체, ids가 있으면 순서와 무관하게 실제 선택 전체와 일치해야 합니다. */
StellarStatus stellar_window_edit_selection(uint64_t handle, const StellarSelectionTarget *target, uint32_t action, const uint8_t *ids, size_t count, uint64_t *out_request);
/* Clipboard subtree data uses caller-owned byte buffers. */
StellarStatus stellar_window_session_copy_subtree(uint64_t handle, uint64_t session, const uint8_t view[16], uint64_t revision, uint8_t *bytes, size_t capacity, size_t *out_len);
StellarStatus stellar_window_session_paste_subtree(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t parent[16], uint64_t revision, const uint8_t *bytes, size_t len, uint64_t *out_request);
/* 기존 호출자는 요청 접수 시점의 게시 선택을 사용합니다. */
StellarStatus stellar_window_session_delete_selection(uint64_t handle, uint64_t session, const uint8_t view[16], uint64_t revision, uint64_t *out_request);
StellarStatus stellar_window_session_duplicate_selection(uint64_t handle, uint64_t session, const uint8_t view[16], uint64_t revision, uint64_t *out_request);
/* 화면에 표시한 View·revision에서 선택한 Frame을 부모 좌표계의 delta만큼 이동합니다. */
StellarStatus stellar_window_session_move_frame(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, float dx, float dy, uint64_t *out_request);
StellarStatus stellar_window_session_resize_frame(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, float width, float height, uint64_t *out_request);
/* 선택한 Free Frame 배경. space가 0이면 제거합니다. */
StellarStatus stellar_window_session_set_frame_background(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, StellarCanvasColor color, uint64_t *out_request);
/* 표시한 부모 Frame의 local rect에 자식 Free Frame을 삽입하고 한 history entry로 선택합니다. */
StellarStatus stellar_window_session_insert_frame(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t parent[16], uint64_t revision, float x, float y, float width, float height, uint64_t *out_request);
/* 실제 Rectangle Shape와 단색 Fill/Transform modifier를 생성합니다. */
StellarStatus stellar_window_session_insert_rectangle(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t parent[16], uint64_t revision, float x, float y, float width, float height, uint64_t *out_request);
/* 창 session의 선택을 ids(StableId 16 byte × count)로 바꾸고 그 창 history에 기록합니다(count 0 = 비움). */
StellarStatus stellar_window_session_select(uint64_t handle, uint64_t session, const uint8_t *ids, size_t count, uint64_t *out_request);

/* 활성 View의 같은 게시 snapshot에서 카메라와 painter 순서 목록 조회. rows=NULL이면 metadata만 씁니다.
 * 용량 부족 시 최신 count와 STELLAR_BUFFER_TOO_SMALL, active View 없음은 STELLAR_NO_ERROR. */
StellarStatus stellar_window_canvas(uint64_t handle, uint64_t session, float width, float height, StellarCanvasInfo *info, StellarCanvasFrame *rows, size_t capacity);

/* 셸 worker 전용. 활성 View/revision의 직계 자식을 앞에서부터 조회합니다.
 * rows: STELLAR_LAYER_PAGE_SIZE개 caller buffer. 플래그 bit 0 표시, 1 잠금, 2 자식 있음.
 * UTF-8 text는 페이지·View·행 이름 순서. BUFFER_TOO_SMALL은 info.text_len만큼 확보해 재호출합니다. */
/* Text composition. Begin and preview run on the window worker.
 * Begin uses document Unicode scalar offsets. Mark uses UTF-16 offsets within marked text.
 * One transient token per window. Commit reports its request result via the event queue. */
StellarStatus stellar_text_begin(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, uint32_t start, uint32_t end, uint64_t *out_token);
StellarStatus stellar_text_mark(uint64_t handle, uint64_t session, uint64_t token, const uint8_t *text, size_t len, uint32_t start, uint32_t count);
StellarStatus stellar_text_preview(uint64_t handle, uint64_t session, uint64_t token, uint8_t *buf, size_t capacity, size_t *out_len);
StellarStatus stellar_text_commit(uint64_t handle, uint64_t session, uint64_t token, const uint8_t *text, size_t len, uint64_t *out_request);
StellarStatus stellar_text_cancel(uint64_t handle, uint64_t session, uint64_t token);
StellarStatus stellar_window_layer_rename(uint64_t handle, uint64_t session, const uint8_t view[16], const uint8_t node[16], uint64_t revision, const uint8_t *name, size_t name_len, uint64_t *out_request);
StellarStatus stellar_window_layer_page(uint64_t handle, uint64_t session, const StellarLayerQuery *query, StellarLayerInfo *info, StellarLayerRow *rows, uint8_t *text, size_t capacity);

/* 열린 창 session의 view state(탭 수, active view, viewport)를 씁니다(없는 창 session은 STELLAR_INVALID_ARGUMENT). */
StellarStatus stellar_window_view_info(uint64_t handle, uint64_t session, StellarWindowViewInfo *out_info);

/* 열린 창 session의 최신 snapshot 정보를 씁니다(없는 창 session은 STELLAR_INVALID_ARGUMENT). */
StellarStatus stellar_window_session_info(uint64_t handle, uint64_t session, StellarWindowSessionInfo *out_info);

#if defined(DEBUG) && DEBUG
/* Debug native integration check only; no symbol in Release. Real transaction/queue, delay capped at 500 ms. */
StellarStatus stellar_window_session_check_edit(uint64_t handle, uint64_t session, const uint8_t *node, const uint8_t *name, size_t name_len, uint32_t verify, uint32_t delay_ms, uint64_t *out_request);
#endif

/* 살아 있는 runtime handle 수(진단용). */
uint64_t stellar_runtime_live_count(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* STELLAR_FFI_H */
