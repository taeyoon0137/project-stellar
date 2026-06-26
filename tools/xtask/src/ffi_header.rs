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
    STELLAR_INTERNAL_PANIC = 7
} StellarStatus;

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
            "stellar_runtime_live_count",
        ] {
            assert!(HEADER.contains(needle), "헤더에 {needle} 누락");
        }
        // 내부 전용 Rust API는 C 헤더에 노출하지 않습니다.
        assert!(!HEADER.contains("enqueue_event"));
    }
}
