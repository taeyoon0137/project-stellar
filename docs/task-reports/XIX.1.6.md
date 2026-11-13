# 작업 보고서: XIX.1.6 앱 쪽 IPC의 데이터 교환 전 상대 인증

- WBS: XIX.1.6
- 작업명: 앱 쪽 IPC가 데이터 교환 전에 연결 상대를 인증(ADR-0005 §5, 아키텍처 37 신뢰 경계). 인증은 UI·Document 스레드 밖에서 수행
- 상태: 완료 (`[x]`)
- 선행: XIX.1.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Rust + macOS socket option·libbsm·Security framework, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `crates/local-ipc/src/peer.rs`(신규) — `getpeereid` euid + `LOCAL_PEERTOKEN` audit token(`PeerCredentials`, `AuditToken`, `CredentialError`).
- `crates/local-ipc/src/code_identity.rs`(신규) — Security framework 코드 서명 검사(`CodeRule`, `TeamId`, `check_peer_code`, `own_team_id`), 최소 CoreFoundation·Security `extern "C"`와 CF 소유 wrapper.
- `crates/local-ipc/src/auth.rs`(신규) — 양쪽 공통 인증 기준 `PeerPolicy`와 `authenticate`(`AuthError`).
- `crates/local-ipc/src/listener.rs`(신규) — 앱 측 endpoint(`AppEndpoint`)와 IPC I/O 스레드(`AppListener`, `ListenerEvent`, 앱 측 handshake).
- `crates/local-ipc/src/lib.rs` — module 등록.
- `docs/task-reports/XIX.1.6.md`(신규), 계획서(XIX.1.6 `[ ]`→`[x]`).

## 인증 기준과 방법(Apple 공식 문서 근거)

모두 만족해야 하며, 하나라도 확인할 수 없으면 거부합니다(fail closed). 앱과 브리지가 같은 `PeerPolicy`·`authenticate`를 씁니다(XIX.1.7).

1. **같은 OS 사용자**
   - `getpeereid(3)`(macOS man page): Unix-domain peer의 effective uid. "the credentials returned to the listen(2) caller are those of its peer at the time it called connect(2) … there is no way for either side to influence the credentials returned to its peer". 자신의 `geteuid()`와 같아야 합니다.
   - `getsockopt(SOL_LOCAL, LOCAL_PEERTOKEN)`(macOS SDK `<sys/un.h>` "retrieve peer audit token")으로 상대 audit token을 얻고, `<bsm/libbsm.h>`가 권하는 accessor `audit_token_to_euid`로 읽은 euid가 `getpeereid` 값과 같아야 합니다. pid는 `audit_token_to_pid`로 얻습니다.
2. **코드 서명 신원**
   - `SecCodeCopyGuestWithAttributes`(host `NULL` = "the code signing root of trust (currently, the system kernel)")에 `kSecGuestAttributeAudit`(`SecCode.h`, audit token CFData)로 상대 실행 코드를 찾습니다. pid가 아니라 audit token(pid version 포함)을 쓰므로 pid 재사용 경쟁이 없습니다.
   - `SecRequirementCreateWithString`으로 요구 사항을 만들고 `SecCodeCheckValidity`("Performs dynamic validation of signed code … validates the code against a code requirement")로 검증합니다. `errSecCSReqFailed`·`errSecCSUnsigned`·기타 OSStatus는 모두 거부입니다.
   - 규칙은 자기 서명에서 정합니다(`SecCodeCopySelf` + `SecCodeCopySigningInformation(kSecCSSigningInformation)`의 `kSecCodeInfoTeamIdentifier`).
     - 자기 Team ID가 있으면(Developer ID·Mac App Store 서명 빌드): `anchor apple generic and certificate leaf[subject.OU] = "<TeamID>"`. TN3127 "Inside Code Signing: Requirements"는 `certificate leaf[subject.OU]`가 Developer ID 인증서의 Team ID 위치라고 설명합니다. Team ID는 `[A-Z0-9]{10}`만 받아 요구 사항 문자열 주입을 막습니다.
     - 없으면(개발 빌드, ad-hoc·linker 서명): `!anchor apple`. 유효한 서명이 있고 Apple platform binary가 아닌 코드만 받습니다. 시스템 도구(`nc` 등)로 만든 가짜 endpoint·bridge는 거부하지만 같은 사용자가 직접 서명한 실행 파일은 막지 못합니다. 개발 빌드의 실제 신뢰 경계는 OS 사용자입니다.
3. **순서와 데이터 교환**: accept 직후 1 → 2를 수행하고, 통과한 뒤에만 HELLO를 읽고 WELCOME을 씁니다. 인증 함수는 socket option과 Security framework 조회만 하고 stream에서 읽거나 쓰지 않습니다(`credentials_do_not_consume_stream_data`). 실패하면 읽지도 쓰지도 않고 닫습니다.
4. **스레드(ADR-0005 §6)**: 인증과 handshake는 전용 IPC I/O 스레드 `stellar-ipc-io`에서만 실행됩니다. 앱(UI) 스레드는 이벤트 채널만 읽고, 각 이벤트에 처리 스레드 id가 들어 있습니다. `stellar-local-ipc`는 crate graph 정책상 core-types만 허용되어 document·commands·runtime에 도달할 수 없으므로 Document 스레드를 호출할 수단이 없습니다.
5. **탐색 연결**: registry 생존 확인처럼 연결 직후 닫힌 상대는 `LOCAL_PEERTOKEN`이 `ENOTCONN`을 돌려주므로, 데이터 교환이 불가능한 종료 연결로 보고 이벤트 없이 버립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc` | unit 42 passed, 0 failed |
| `cargo test -q -p stellar-local-ipc` 5회 반복 | 매회 42 passed, 0 failed |
| `cargo clippy -p stellar-local-ipc -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-local-ipc -- --check` | 통과 |
| `cargo test -p stellar-mcp` | 회귀 없음(unit 33, 통합 18 passed) |
| `codesign -dv target/debug/stellar-mcp`, `codesign -v -R='!anchor apple'` (참고) | 개발 빌드는 `adhoc,linker-signed`, Team ID 없음, 개발 규칙 만족. `/usr/bin/nc`는 개발 규칙 불만족 |

| 검사(fixture) | 결과 |
|---|---|
| **인증 실패 fixture — 다른 사용자**: 기대 uid를 현재+1로 둔 앱에 연결해 HELLO를 먼저 보냄 → 앱이 보낸 byte 0, `Rejected(UserMismatch)`, 연결 전달 0(`fixture_auth_failure_exchanges_no_data`) | 통과 |
| **인증 실패 fixture — 코드 규칙**: 요구 사항 `anchor apple`인 앱에 테스트 프로세스(ad-hoc)가 연결 → Security framework가 실제로 `RequirementFailed`, 앱이 보낸 byte 0(같은 테스트) | 통과 |
| **인증 실패 fixture — 실제 다른 프로그램**: 현재 프로세스 기준(개발 규칙) 앱에 `/usr/bin/nc -U`가 연결 → `Rejected(Code(RequirementFailed))`, nc가 받은 byte 0(`fixture_foreign_program_is_rejected_by_current_process_policy`) | 통과 |
| **UI·Document 스레드 호출 0**: 모든 이벤트의 처리 스레드 = `AppListener::io_thread()`(`stellar-ipc-io`) ≠ 호출(테스트) 스레드(위 fixture와 `authenticated_bridge_completes_handshake_on_io_thread`), crate graph상 document·commands 도달 불가(`local_ipc_depends_only_on_core_types`) | 통과 |
| 인증 통과 bridge(`handshake::connect`)와 handshake 완료, 양측 nonce 일치(`authenticated_bridge_completes_handshake_on_io_thread`) | 통과 |
| 인증 뒤 재사용 client nonce에 `REJECT nonce-reused`(`reused_client_nonce_is_rejected_after_authentication`) | 통과 |
| 탐색 연결은 이벤트 없음(`probe_connections_are_ignored`) | 통과 |
| endpoint: registry 0700 생성, 같은 instance 중복 `InUse`, drop 시 socket 삭제, 잔재 socket 재사용, 0755 registry 거부(`bind_creates_private_registry_and_cleans_up`, `insecure_registry_is_not_used`) | 통과 |
| 자기 프로세스는 개발 규칙 만족, `anchor apple`·임의 Team ID 규칙은 실제로 불만족(`own_process_satisfies_development_rule_and_fails_stricter_rules`), 테스트 실행 파일 Team ID 없음(`test_binary_is_development_signed_without_team`) | 통과 |
| Team ID 형식 엄격 검사, 요구 사항 문자열 컴파일·잘못된 문자열 거부, audit token 32 byte 배치, socketpair 상대 = 자기 pid·uid | 통과 |

## 차이(작업 지시와 다른 점)

- 앱 runtime(`crates/runtime`)은 다른 작업 lane의 범위라 이 작업에서 고치지 않았습니다. 앱 쪽 IPC는 runtime이 그대로 쓸 수 있는 `stellar-local-ipc`의 `AppEndpoint`·`AppListener`로 구현했고, runtime이 이를 시작하는 연결은 후속 작업입니다.
- "다른 OS 사용자" 거부는 DEV-MAC에서 다른 계정 프로세스를 만들 수 없어 기대 uid를 바꿔 재현했습니다. 대신 코드 서명 거부는 실제 Security framework 판정과 실제 다른 프로그램(`/usr/bin/nc`)으로 검증했습니다.
- XIX.1.3의 test double(`test_support.rs`)은 handshake 오류 경로 테스트용으로 남겼습니다.

## 후속 변경(같은 lane, `Fix:` 커밋)

- `fixture_foreign_program_is_rejected_by_current_process_policy`를 `crates/local-ipc/tests/foreign_peer.rs`(별도 test 실행 파일)로 옮겼습니다. macOS에서 socket 생성과 close-on-exec 설정 사이에 띄운 `nc`가 다른 unit test의 listener fd를 물려받아 stale 판정이 흔들린 것을 확인했기 때문입니다. unit test는 41개, 통합 1개가 됩니다.
- 앱 측 I/O 스레드가 다음을 이벤트 없이 닫도록 고쳤습니다: timeout 설정 중 상대가 닫은 연결(`setsockopt` `EINVAL` → `Closed`), 인증 중 상대 프로세스가 이미 끝난 연결(`errSecCSNoSuchCode`). 둘 다 데이터를 주고받을 수 없는 연결이며, 짧게 실행된 브리지의 registry 탐색 연결에서 발생했습니다.
- 두 앱 instance가 동시에 registry 디렉터리를 만들 때 `AlreadyExists`를 오류로 보던 경합을 고쳤습니다(이후 소유자·권한·symlink 검사는 그대로).

## 남은 위험 / 한계

- 서명 빌드 규칙(같은 Team ID)의 수락 경로는 서명 인증서가 없는 DEV-MAC에서 실행할 수 없습니다. 요구 사항 문자열 컴파일과 ad-hoc 코드 거부만 검증했고, 수락 경로는 서명 helper 작업(XXIV.5.1, XXIX.5.1)에서 확인해야 합니다.
- 개발 규칙은 같은 사용자가 직접 서명한 실행 파일을 막지 못합니다(위 기준 2).
- 인증은 연결마다 Security framework를 호출합니다. I/O 스레드 안에서 순차 처리하므로 동시 연결이 많으면 대기가 생길 수 있습니다(브리지는 세션당 연결 하나).
- `docs/architecture/crate-dependency-graph.md`에 `stellar-local-ipc` 항목 추가는 이 lane의 허용 경로 밖이라 하지 않았습니다(XIX.9.1 범위).
