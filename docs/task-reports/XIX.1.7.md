# 작업 보고서: XIX.1.7 stellar-mcp 쪽 IPC의 데이터 교환 전 상대 인증

- WBS: XIX.1.7
- 작업명: `stellar-mcp` 쪽 IPC가 데이터 교환 전에 연결 상대를 XIX.1.6과 같은 기준으로 인증(ADR-0005 §5). 실패 시 종료 코드는 XXIV.1.2 표를 따름
- 상태: 완료 (`[x]`)
- 선행: XIX.1.6(`[x]`, 같은 lane), XXIV.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Rust + macOS socket option·Security framework, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `crates/local-ipc/src/handshake.rs` — 브리지 측 `bridge_handshake`·`connect`가 `PeerPolicy`로 앱을 인증한 뒤에만 HELLO를 보냄, `HandshakeError::Unauthenticated`, uid 주입 경로는 테스트 전용으로 축소.
- `tools/stellar-mcp/src/attach.rs`(신규) — `--attach` 연결 흐름(registry 탐색 → 선택 → 인증·handshake)과 실패별 종료 상태(`AttachError::exit`).
- `tools/stellar-mcp/src/exit.rs`(신규) — XXIV.1.2 표와 같은 값·이름의 종료 상태.
- `tools/stellar-mcp/src/main.rs` — `--attach [--instance <id>]` 실행 방식, 종료 코드를 `exit` 상수로 통일, 도움말.
- `tools/stellar-mcp/src/lib.rs` — module 등록.
- `crates/local-ipc/src/listener.rs` — 테스트의 `connect` 호출만 새 서명으로 변경(프로덕션 코드 변경 없음).
- `tools/stellar-mcp/tests/attach_auth.rs`(신규) — 실제 실행 파일로 인증 실패·종료 코드·성공 경로 검증, 종료 코드 표 일치 검사.
- `docs/task-reports/XIX.1.7.md`(신규), 계획서(XIX.1.7 `[ ]`→`[x]`).

## 구현 요약

- **같은 기준**: 브리지는 앱 측(XIX.1.6)과 같은 `stellar_local_ipc::auth::PeerPolicy::for_current_process()`와 같은 `authenticate`를 씁니다. 같은 OS 사용자(`getpeereid`, `LOCAL_PEERTOKEN` audit token)와 코드 서명 신원(`SecCodeCopyGuestWithAttributes` + `kSecGuestAttributeAudit`, `SecCodeCheckValidity`, 서명 빌드는 같은 Team ID, 개발 빌드는 유효 서명이면서 Apple platform binary 아님)입니다. 근거 문서는 XIX.1.6 보고서에 기록했습니다. connect한 쪽이 받는 자격 증명은 상대가 `listen(2)`한 시점의 것입니다(`getpeereid(3)`).
- **데이터 교환 0**: 인증은 socket option과 Security framework 조회만 하며, 통과한 뒤에만 client nonce를 만들고 HELLO를 보냅니다. 실패하면 아무것도 쓰지 않고 닫습니다. 인증 중 앱이 먼저 닫아 자격 증명을 읽을 수 없으면(`ENOTCONN`) 응답 없는 종료(`Closed`)로 분류합니다.
- **실행 흐름**: `stellar-mcp --attach [--instance <id>]`는 MCP 세션 전에 현재 사용자 registry에서 앱을 고르고 인증·handshake한 뒤 stdio MCP를 처리합니다. 연결은 MCP 세션이 끝날 때까지 유지합니다. 자동 backend 선택은 XIX.9.4 범위입니다.
- **종료 코드(XXIV.1.2 표)**

| 실패 | 종료 코드 |
|---|---|
| 앱 인증 실패·확인 불가, 다른 사용자, 안전하지 않은 registry | 77 `noperm` |
| 실행 중 앱 없음, 지정 instance 없음, 앱이 응답 없이 닫음 | 69 `unavailable` |
| 여러 앱인데 instance 미지정, 잘못된 인자·instance 형식 | 64 `usage` |
| handshake 형식·nonce 위반, 앱의 REJECT | 76 `protocol` |
| handshake 응답 시간 초과 | 75 `tempfail` |
| 입출력 오류 | 74 `ioerr` |
| 자기 인증 기준을 만들 수 없음, 부모 감시 시작 실패 | 70 `software` |
| socket 경로가 OS 한도 초과, 내장 protocol config 오류 | 78 `config` |

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc -p stellar-mcp` | local-ipc unit 42·foreign_peer 1, stellar-mcp unit 34·attach_auth 4·connection_info 4·json_copy_sync 2·no_network 3·parent_exit 4·protocol_smoke 5·protocol_version_matrix 2 passed, 0 failed |
| `cargo test -q -p stellar-mcp -p stellar-local-ipc` 24회 반복(직렬화·경합 수정 후) | 실패 0 |
| `cargo clippy -p stellar-local-ipc -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-local-ipc -p stellar-mcp -- --check` | 통과 |

| 검사(fixture) | 결과 |
|---|---|
| **인증 실패 fixture(실제 실행 파일)**: 기본 registry의 고유 instance 자리에 `/usr/bin/nc -lkU`(실제 다른 프로그램)가 endpoint를 엶 → `stellar-mcp --attach --instance <id>` 종료 코드 77, stdout 0 byte, stderr에 `(noperm)`·서명 요구 사항 사유, **nc가 받은 byte 0**(`fixture_foreign_endpoint_is_rejected_without_data_exchange`) | 통과 |
| 종료 코드: 없는 instance 69, 잘못된 instance 형식·인자 누락·추가 인자·`--attach` 없는 `--instance` 64, 모두 stdout 0 byte(`fixture_attach_failures_use_xxiv_1_2_exit_codes`) | 통과 |
| 종료 코드 표 일치: `exit::ALL` 9개 상태의 (이름, 값)이 `tools/stellar-cli/src/exit.rs`의 `code()`·`name()` 표(13행)와 같음(`exit_table_matches_xxiv_1_2`) | 통과 |
| 성공 경로: `stellar_local_ipc::listener::AppListener`(앱 측 XIX.1.6)에 실제 브리지가 인증 후 연결, MCP ping 응답, stdin EOF로 종료 코드 0, 앱 `Connected`(`authenticated_app_attach_serves_mcp_and_ends_on_eof`) | 통과 |
| 라이브러리: 엄격 기준(`anchor apple`)·다른 사용자 기준이면 `Unauthenticated`, 상대가 받은 byte 0, nonce 장부 비어 있음(`bridge_rejects_app_failing_policy_before_sending_any_data`) | 통과 |
| `attach_with`: 앱 없음 69, 0755 registry 77, 엄격 기준 77이고 앱이 받은 handshake 0, 같은 기준이면 연결(`attach::tests`) | 통과 |
| 모든 `AttachError` 변형의 종료 상태 매핑(`exit_mapping_follows_the_table`) | 통과 |
| XIX.1.3 handshake 오류 경로(echo 불일치, session nonce 재사용, REJECT, 형식 위반, timeout) 계속 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 순서: lane 목록은 XIX.1.4를 XIX.1.6·XIX.1.7보다 앞에 두었지만, XIX.1.4 검증(외부 bridge disconnect)에 실제 연결된 브리지가 필요해 XIX.1.6 → XIX.1.7 → XIX.1.4 순으로 진행했습니다. XIX.1.7의 선행(XIX.1.6, XXIV.1.2)은 모두 `[x]`입니다.
- "다른 OS 사용자" 거부는 DEV-MAC에서 다른 계정 프로세스를 만들 수 없어 기대 uid를 바꿔 라이브러리 수준에서 재현했습니다. 실행 파일 수준의 인증 실패는 실제 다른 프로그램(`nc`)과 실제 Security framework 판정으로 검증했습니다.
- 브리지 종료 코드 표는 L1 도구에 의존하지 않도록 `stellar-mcp`에 필요한 항목만 같은 값으로 두고, 소스 비교 테스트로 XXIV.1.2 표와 일치를 강제했습니다.
- 같은 lane의 선행 커밋 두 개가 이 작업의 테스트 안정성을 위해 들어갔습니다: `Test:`(부모 감시 프로세스 테스트 분리), `Fix:`(IPC 상대가 먼저 닫힌 연결·registry 동시 생성 경합). 원인은 macOS에서 socket·pipe 생성과 close-on-exec 설정 사이에 다른 thread가 띄운 자식이 fd를 물려받는 경합이며, 프로세스를 띄우는 통합 테스트 파일은 테스트를 직렬화했습니다.

## 남은 위험 / 한계

- 테스트가 현재 사용자의 기본 registry 디렉터리(`<사용자 임시 디렉터리>/stellar-ipc-<uid>`, 0700)를 만들고 남겨 둡니다. socket 파일은 고유 instance id로 만들고 지웁니다.
- 서명 빌드(같은 Team ID) 수락 경로는 서명 인증서가 필요해 XXIV.5.1·XXIX.5.1 이후 확인해야 합니다.
- 내부 IPC protocol 버전 협상(`STELLAR-IPC/1` 불일치 시 76)은 XXIX.5.2 범위입니다. 현재는 다른 표지를 형식 위반(76)으로 거부합니다.
