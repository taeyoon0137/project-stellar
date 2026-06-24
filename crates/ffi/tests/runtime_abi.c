/* II.4.1 C ABI 테스트: create/version/destroy를 1,000회 반복해 누수·double free를 확인합니다.
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

extern uint64_t stellar_runtime_create(void);
extern int32_t stellar_runtime_destroy(uint64_t handle);
extern intptr_t stellar_runtime_version(uint64_t handle, uint8_t *buf, size_t buf_len);
extern uint64_t stellar_runtime_live_count(void);

int main(void) {
    for (int i = 0; i < 1000; i++) {
        uint64_t h = stellar_runtime_create();
        if (h == 0) {
            fprintf(stderr, "create failed at %d\n", i);
            return 1;
        }
        uint8_t buf[16] = {0};
        intptr_t n = stellar_runtime_version(h, buf, sizeof(buf));
        if (n < 0) {
            fprintf(stderr, "version failed at %d\n", i);
            return 2;
        }
        if (stellar_runtime_destroy(h) != 0) {
            fprintf(stderr, "destroy failed at %d\n", i);
            return 3;
        }
        if (stellar_runtime_destroy(h) != -1) {
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
