# 작업 보고서: XIX.1.2 로컬 IPC endpoint registry

- WBS: XIX.1.2
- 작업명: 브리지가 실행 중 Stellar 앱을 찾는 로컬 IPC endpoint registry, network port 없음
- 상태: 완료 (`[x]`)
- 선행: XIX.1.1(같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + macOS Unix domain socket, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `tools/stellar-mcp/src/ipc/mod.rs`(신규) — IPC module(macOS 전용, `cfg(target_os = "macos")`).
- `tools/stellar-mcp/src/ipc/os.rs`(신규) — 최소 `extern "C"`(`geteuid`, `confstr`)로 현재 uid와
  사용자 임시 디렉터리(`_CS_DARWIN_USER_TEMP_DIR`)를 얻는 안전 wrapper.
- `tools/stellar-mcp/src/ipc/registry.rs`(신규) — `Registry`·`InstanceId`·`Endpoint`·`Discovery`·
  `RegistryError`, 디렉터리 보안 검사와 탐색·선택.
- `tools/stellar-mcp/src/ipc/test_support.rs`(신규, `cfg(test)`) — 앱 측 등록 test double.
- `tools/stellar-mcp/src/lib.rs` — `ipc` module 등록.
- `tools/stellar-mcp/tests/no_network.rs`(신규) — network API 미사용·의존성 없음 검사.
- `docs/task-reports/XIX.1.2.md`(신규), 계획서(XIX.1.2 `[ ]`→`[x]`).

## 구현 요약

- **위치(환경 변수 무관)**: registry 디렉터리는 `confstr(_CS_DARWIN_USER_TEMP_DIR)`가 주는 사용자
  임시 디렉터리 아래 `stellar-ipc-<uid>`입니다. MCP client가 환경 변수를 비우고 브리지를 실행해도
  GUI 앱과 같은 경로를 얻습니다. 기본 위치의 socket 경로는 macOS `sun_path` 한도(103 byte) 안이며,
  한도를 넘는 경로는 `PathTooLong`으로 거부합니다.
- **디렉터리 신뢰 조건**: symlink 아님, 디렉터리, 현재 사용자(effective uid) 소유, 권한 정확히 0700.
  하나라도 어기면 `Insecure`(이유 포함)로 실패하고 안의 어떤 항목도 신뢰하지 않습니다. 디렉터리가
  없으면 오류가 아니라 "앱 없음"입니다.
- **endpoint 형식**: 앱 instance마다 `<소문자 16진수 16자>.sock` Unix domain socket. 이름 규칙 밖,
  socket이 아닌 항목(일반 파일·symlink), 다른 사용자 소유 항목은 무시하고 개수만 보고합니다.
- **탐색**: 후보마다 데이터를 보내지 않는 연결을 한 번 시도해 listener가 있으면 live, 연결 거부면
  stale(비정상 종료 잔재)로 분류합니다. 결과는 `NoApp`·`One`·`Many`(instance id 순)이며, 브리지는
  registry를 수정하지 않습니다.
- **선택**: 하나면 자동 선택, 여러 개면 `Ambiguous`(후보 목록)로 instance 지정을 요구, 지정한 id가
  없으면 `NotFound`입니다.
- **network 없음**: TCP·UDP·`std::net` API를 쓰지 않음을 소스 검사로 강제하고, 의존성이 비어 있음을
  확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 34, no_network 2, protocol_smoke 5, protocol_version_matrix 2 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사(fixture) | 결과 |
|---|---|
| **앱 없음**: registry 없음·빈 registry(`fixture_no_app_when_registry_missing_or_empty`), stale socket·이름 맞는 일반 파일·규칙 밖 socket·socket symlink·기타 파일만 있음 → NoApp, stale 1·ignored 4(`fixture_no_app_when_only_stale_or_foreign_entries`) | 통과 |
| **한 개**: live endpoint 1개 → One, 자동 선택, 없는 id 지정은 NotFound(`fixture_one_app_is_selected_automatically`) | 통과 |
| **여러 개**: live 3개 + stale 1개 → Many(id 순), 자동 선택은 Ambiguous(후보 목록), id 지정 선택 성공(`fixture_many_apps_require_explicit_instance`) | 통과 |
| 0755 권한·다른 소유자·symlink 디렉터리·파일 경로는 live endpoint가 있어도 Insecure(`insecure_registry_directories_are_not_trusted`) | 통과 |
| socket 경로 한도 초과 거부(`socket_paths_beyond_os_limit_are_rejected`) | 통과 |
| 기본 위치 = 사용자 임시 디렉터리/`stellar-ipc-<uid>`, 한도 안(`default_registry_is_per_user_and_env_independent`) | 통과 |
| `geteuid`가 이 프로세스가 만든 파일 소유자와 같음, 사용자 임시 디렉터리는 절대 경로·본인 소유·group/other 권한 없음(`ipc::os::tests`) | 통과 |
| 소스에 TCP·UDP·`std::net`·SocketAddr 없음, Unix domain socket 사용, 의존성 없음(`no_network`) | 통과 |

## 차이(작업 지시와 다른 점)

- 앱 측(endpoint 등록)은 아직 없으므로 브리지 측 탐색·선택만 라이브러리로 구현하고, 앱 등록은
  registry 계약을 따르는 test double(`ipc/test_support.rs`, 테스트 전용)로 대신했습니다. 앱 구현은
  module 문서의 "앱 측 계약"을 따라야 합니다.
- registry 위치를 `$TMPDIR`가 아니라 `confstr(_CS_DARWIN_USER_TEMP_DIR)`로 얻습니다. 환경을 비운
  client 실행에서도 같은 위치를 쓰기 위해서이며, 이를 위해 `geteuid`·`confstr`를 최소 `extern "C"`로
  선언했습니다(libc crate 없음).
- 탐색은 아직 실행 파일의 MCP 흐름에 연결하지 않았습니다. bridge session 연결은 XIX.1.3(handshake)·
  XIX.1.4(session 수명) 범위입니다.

## 남은 위험 / 한계

- liveness 확인은 연결 한 번이라 탐색과 실제 연결 사이에 앱이 종료될 수 있습니다(연결 단계에서 다시
  실패 처리 필요). 앱은 데이터 없이 닫히는 탐색 연결을 정상으로 무시해야 합니다.
- 앱이 App Sandbox로 배포되면 사용자 임시 디렉터리가 container 안으로 바뀌어 위치 계약을 다시
  정해야 합니다(현재 앱은 sandbox 미사용).
- Windows는 보류(ADR-0006)이며 `ipc` module은 macOS에서만 빌드됩니다.
