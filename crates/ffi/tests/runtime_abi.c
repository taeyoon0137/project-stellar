/* II.4.1/II.4.2 C ABI 테스트 + 프로젝트·문서 session C ABI 확인(ADR-0014).
 *
 * create/version/destroy를 1,000회 반복해 누수·double free를 확인하고,
 * 모든 함수가 StellarStatus를 반환함을 사용합니다. 이어서 생성 헤더로 프로젝트 열기 요청 →
 * pull event(REQUEST_FAILED, NOT_A_REPOSITORY) → 상태 snapshot을 확인합니다.
 *
 * 빌드 예(정적 라이브러리에 프로젝트 감시기(FSEvents) 코드가 들어 있으므로 CoreServices도
 * 링크합니다):
 *   cargo build -p stellar-ffi
 *   cc -Icrates/ffi/include crates/ffi/tests/runtime_abi.c target/debug/libstellar_ffi.a \
 *      -o /tmp/abi -framework CoreFoundation -framework CoreServices -framework Security
 *   /tmp/abi
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "stellar_ffi.h"

static int session_check(void) {
    uint64_t h = 0;
    if (stellar_runtime_create(&h) != STELLAR_OK || h == 0) {
        fprintf(stderr, "session: create failed\n");
        return 10;
    }
    StellarRuntimeState state;
    memset(&state, 0xAB, sizeof(state));
    if (stellar_runtime_state(h, &state) != STELLAR_OK ||
        state.version != STELLAR_RUNTIME_STATE_VERSION || state.generation != 0) {
        fprintf(stderr, "session: initial state mismatch\n");
        return 11;
    }
    /* 존재하지 않는 절대 경로: 요청은 접수되고 실패 event로 끝납니다. */
    const char *path = "/stellar-abi-test-no-such-project";
    uint64_t request = 0;
    if (stellar_project_open(h, (const uint8_t *)path, strlen(path), &request) != STELLAR_OK ||
        request == 0) {
        fprintf(stderr, "session: open request failed\n");
        return 12;
    }
    uint32_t kind = 0;
    size_t need = 0;
    StellarStatus status = STELLAR_NO_ERROR;
    for (int spin = 0; spin < 5000000 && status == STELLAR_NO_ERROR; spin++) {
        status = stellar_runtime_poll_event(h, &kind, NULL, 0, &need);
    }
    if (status != STELLAR_OK || kind != STELLAR_EVENT_REQUEST_FAILED || need < 16 || need > 64 + STELLAR_EVENT_TEXT_MAX) {
        fprintf(stderr, "session: no failure event (status %d kind %u)\n", (int)status, kind);
        return 13;
    }
    uint8_t payload[64 + STELLAR_EVENT_TEXT_MAX];
    size_t written = 0;
    if (stellar_runtime_poll_event(h, &kind, payload, sizeof(payload), &written) != STELLAR_OK ||
        written != need) {
        fprintf(stderr, "session: poll failed\n");
        return 14;
    }
    uint32_t version = 0, failure = 0;
    uint64_t got_request = 0;
    memcpy(&version, payload, 4);
    memcpy(&got_request, payload + 4, 8);
    memcpy(&failure, payload + 12, 4);
    if (version != STELLAR_EVENT_SCHEMA_VERSION || got_request != request ||
        failure != STELLAR_FAILURE_NOT_A_REPOSITORY) {
        fprintf(stderr, "session: payload mismatch (failure %u)\n", failure);
        return 15;
    }
    if (stellar_runtime_state(h, &state) != STELLAR_OK || state.outstanding != 0 ||
        state.project_count != 0) {
        fprintf(stderr, "session: final state mismatch\n");
        return 16;
    }
    if (stellar_runtime_destroy(h) != STELLAR_OK) {
        fprintf(stderr, "session: destroy failed\n");
        return 17;
    }
    return 0;
}

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
    int session = session_check();
    if (session != 0) {
        return session;
    }
    uint64_t live = stellar_runtime_live_count();
    if (live != 0) {
        fprintf(stderr, "leak: %llu live handles\n", (unsigned long long)live);
        return 5;
    }
    printf("ok: 1000 cycles, 0 leaks, session events\n");
    return 0;
}
