# 작업 보고서: XIX.1.3 bridge↔app IPC session nonce와 OS 사용자 검증

- WBS: XIX.1.3
- 작업명: bridge↔app IPC에 session nonce와 OS 사용자 identity 검증 적용
- 상태: 완료 (`[x]`)
- 선행: XIX.1.2(같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + macOS Unix domain socket·`getpeereid`, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `tools/stellar-mcp/src/ipc/os.rs` — 최소 `extern "C"` `getpeereid` 선언과 `peer_uid` 안전 wrapper.
- `tools/stellar-mcp/src/ipc/nonce.rs`(신규) — 128 bit `Nonce`(OS CSPRNG), 엄격 hex 표기, 재사용을
  거부하는 `NonceLedger`(상한 있는 기억).
- `tools/stellar-mcp/src/ipc/handshake.rs`(신규) — `STELLAR-IPC/1` HELLO/WELCOME/REJECT 메시지,
  `bridge_handshake`·`connect`, `HandshakeError`, `Session`.
- `tools/stellar-mcp/src/ipc/test_support.rs` — 앱 측 handshake test double(`spawn_app`, 동작 9종).
- `tools/stellar-mcp/src/ipc/mod.rs`, `src/lib.rs` — module 등록·문서.
- `docs/task-reports/XIX.1.3.md`(신규), 계획서(XIX.1.3 `[ ]`→`[x]`).

## 구현 요약

- **OS 사용자 검증(데이터 전송 전)**: 연결 직후 커널 peer 자격 증명(`getpeereid`)으로 상대 effective
  uid를 얻어 브리지의 effective uid와 비교합니다. 다르면 `PeerUserMismatch`, 확인할 수 없으면
  `PeerIdentity`로 실패하며, 두 경우 모두 **어떤 byte도 쓰지 않고 nonce도 만들지 않은 채** 연결을
  닫습니다. libc crate 없이 `getpeereid` 하나만 `extern "C"`로 선언했습니다.
- **session nonce**: 브리지는 OS CSPRNG(`/dev/urandom`)로 128 bit client nonce를 만들어 HELLO로
  보내고, 앱은 WELCOME으로 client nonce를 echo하고 새 session nonce를 줍니다. 브리지는 echo 불일치
  (`NonceMismatch`), 이미 본 session nonce 또는 client nonce와 같은 session nonce(`NonceReused`),
  REJECT(`Rejected{reason}`), 형식 위반·128 byte 초과 줄(`Malformed`), 무응답(`TimedOut`, 기본 5초),
  응답 없는 종료(`Closed`)를 모두 거부합니다.
- **재사용 거부**: `NonceLedger`가 프로세스가 본 nonce를 기억합니다(기본 최근 4096개). 앱 측
  계약은 본 client nonce를 기억해 재전송 HELLO에 `REJECT nonce-reused`로 답하는 것입니다.
- **메시지 형식**: ASCII 한 줄, 공백 하나 구분, 추가 field·대문자 hex·다른 protocol 표지·CR 거부.
  다음 데이터를 미리 읽지 않도록 byte 단위로 읽습니다.
- **앱 측**: 앱이 아직 없어 앱 동작은 계약을 따르는 test double로 구현했습니다(accept 직후 peer uid
  확인 → 다르면 쓰지 않고 닫음, client nonce 재사용 거부, HELLO 없는 탐색 연결 무시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 47, no_network 2, protocol_smoke 5, protocol_version_matrix 2 passed, 0 failed |
| `cargo test -q -p stellar-mcp --lib ipc` 5회 반복 | 매회 23 passed, 0 failed(thread·socket 경쟁 없음) |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **다른 사용자 앱 거부(브리지 측)**: 상대 uid가 다르게 보고됨·실제 `getpeereid` uid가 기대와 다름·자격 증명 확인 실패 → 각각 `PeerUserMismatch`/`PeerIdentity`, 상대가 받은 byte 0, nonce 미생성(`bridge_rejects_other_user_app_before_sending_any_data`) | 통과 |
| **다른 사용자 브리지 거부(앱 측 계약)**: 앱이 데이터 없이 닫고 브리지는 `Closed`(`app_rejects_other_user_bridge_before_sending_any_data`) | 통과 |
| **재사용 client nonce 거부**: 같은 HELLO 재전송 → 첫 연결 WELCOME, 둘째 `REJECT nonce-reused`(`app_rejects_reused_client_nonce`), 브리지는 REJECT를 `Rejected`로 반환(`bridge_maps_app_rejection`) | 통과 |
| **재사용 session nonce 거부**: 같은 session nonce를 다시 주는 앱 → 둘째 연결 `NonceReused`, 새 장부에서는 통과(장부가 근거)(`bridge_rejects_reused_session_nonce`) | 통과 |
| echo 불일치 `NonceMismatch`, client nonce를 session으로 되돌림 `NonceReused`(`bridge_rejects_wrong_echo_and_echoed_client_as_session`) | 통과 |
| 같은 사용자 정상 handshake: registry 탐색(생존 확인 연결) 후 연결, 양측 nonce 일치, peer uid = 현재 uid(`same_user_handshake_establishes_session`) | 통과 |
| 형식 위반·긴 줄 `Malformed`, 응답 없이 닫음 `Closed`, 무응답 100 ms timeout `TimedOut`(`malformed_silent_or_closing_apps_fail_cleanly`) | 통과 |
| 메시지 round-trip과 형식 위반 9종 거부(`messages_round_trip_and_reject_malformed_lines`) | 통과 |
| nonce 1000개 중복 없음, 장부 재사용 거부·상한(`ipc::nonce::tests`), 같은 사용자 socketpair의 `getpeereid` = 현재 uid(`peer_uid_of_same_user_socket_pair_is_current_uid`) | 통과 |
| XIX.1.2 network 미사용 검사(`no_network`) 계속 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 앱 측(Swift 셸/코어)이 아직 없으므로 브리지 측 검증을 라이브러리로 구현하고, 앱 측
  handshake는 계약을 따르는 test double(`ipc/test_support.rs`, 테스트 전용)로 대신했습니다. 앱의
  client nonce 재사용 거부는 이 double로 검증했습니다.
- "다른 사용자"는 테스트에서 다른 OS 계정 프로세스를 만들 수 없어, peer uid 공급 함수를 주입하거나
  기대 uid를 바꿔 재현했습니다. 실제 `getpeereid` 경로는 같은 사용자 연결에서 검증했습니다.
- handshake는 아직 실행 파일의 MCP 흐름에 연결하지 않았습니다(한 bridge = 한 session, 종료 시 권한
  폐기는 XIX.1.4).

## 남은 위험 / 한계

- 같은 OS 사용자로 실행되는 악성 프로세스는 이 검증으로 막을 수 없습니다(설계상 신뢰 경계는 OS
  사용자). 프로젝트 신뢰·MCP 권한은 앱이 최종 검증합니다(XIX.3).
- 실제 앱 구현은 module 문서의 앱 측 계약(accept 직후 uid 확인, client nonce 장부, session마다 새
  nonce, 탐색 연결 무시)을 따라야 하며, 앱 쪽 crate 배치는 crate graph 정책 검토가 필요합니다.
- 장부는 최근 4096개 nonce만 기억합니다. 128 bit 무작위 값이라 우연한 충돌은 무시할 수준입니다.
