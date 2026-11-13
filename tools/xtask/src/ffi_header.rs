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

/* 이벤트 큐에서 다음 이벤트를 pull 방식으로 읽습니다(FIFO, 빈 큐는 STELLAR_NO_ERROR). */
StellarStatus stellar_runtime_poll_event(uint64_t handle, uint32_t *out_kind, uint8_t *buf, size_t buf_len, size_t *out_written);

/* 셸이 계산한 drawable(backing) 픽셀 크기와 backing scale을 코어에 전달합니다. */
StellarStatus stellar_runtime_set_drawable_size(uint64_t handle, uint32_t pixel_width, uint32_t pixel_height, double scale);

/* 마지막으로 전달된 drawable 픽셀 크기와 scale을 읽습니다(미설정 시 STELLAR_NO_ERROR). */
StellarStatus stellar_runtime_drawable_size(uint64_t handle, uint32_t *out_pixel_width, uint32_t *out_pixel_height, double *out_scale);

/* macOS NSEvent 원시 필드를 공통 입력 이벤트로 변환해 입력 대기열에 넣습니다(out_queued: 넣은 수 0 또는 1). */
StellarStatus stellar_runtime_submit_mac_event(uint64_t handle, const StellarMacEvent *event, uint32_t *out_queued);

/* 입력 대기열의 다음 공통 입력 이벤트 직렬화 바이트를 읽습니다(FIFO, 빈 대기열은 STELLAR_NO_ERROR). */
StellarStatus stellar_runtime_poll_input(uint64_t handle, uint8_t *buf, size_t buf_len, size_t *out_written);

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
