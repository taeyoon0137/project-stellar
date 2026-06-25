/* II.4.1/II.4.2 C ABI 테스트.
 *
 * create/version/destroy를 1,000회 반복해 누수·double free를 확인하고,
 * 모든 함수가 StellarStatus를 반환함을 사용합니다.
 *
 * 빌드 예:
 *   cargo build -p stellar-ffi
 *   cc crates/ffi/tests/runtime_abi.c target/debug/libstellar_ffi.a -o /tmp/abi \
 *      -framework CoreFoundation -framework Security
 *   /tmp/abi
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

typedef int32_t StellarStatus;
enum {
    STELLAR_OK = 0,
    STELLAR_INVALID_HANDLE = 1
};

extern StellarStatus stellar_runtime_create(uint64_t *out_handle);
extern StellarStatus stellar_runtime_destroy(uint64_t handle);
extern StellarStatus stellar_runtime_version(uint64_t handle, uint8_t *buf, size_t buf_len,
                                             size_t *out_written);
extern StellarStatus stellar_last_error(uint64_t handle, uint8_t *buf, size_t buf_len,
                                        size_t *out_written);
extern uint64_t stellar_runtime_live_count(void);

int main(void) {
    for (int i = 0; i < 1000; i++) {
        uint64_t h = 0;
        if (stellar_runtime_create(&h) != STELLAR_OK || h == 0) {
            fprintf(stderr, "create failed at %d\n", i);
            return 1;
        }
        uint8_t buf[16] = {0};
        size_t written = 0;
        if (stellar_runtime_version(h, buf, sizeof(buf), &written) != STELLAR_OK) {
            fprintf(stderr, "version failed at %d\n", i);
            return 2;
        }
        if (stellar_runtime_destroy(h) != STELLAR_OK) {
            fprintf(stderr, "destroy failed at %d\n", i);
            return 3;
        }
        if (stellar_runtime_destroy(h) != STELLAR_INVALID_HANDLE) {
            fprintf(stderr, "double free not detected at %d\n", i);
            return 4;
        }
    }
    uint64_t live = stellar_runtime_live_count();
    if (live != 0) {
        fprintf(stderr, "leak: %llu live handles\n", (unsigned long long)live);
        return 5;
    }
    printf("ok: 1000 cycles, 0 leaks\n");
    return 0;
}
