# 작업 보고서: XXIX.2.2 구독 CLI 테스트 대역

- WBS: XXIX.2.2
- 작업명: 구독 CLI 실행 경로를 네트워크와 실제 계정 없이 재현하는 테스트 대역
- 상태: `[x]` 완료
- 선행: XXIX.2.1
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/test-cli/main.rs`(신규, 실행 파일 `stellar-agent-host-test-cli`), `crates/agent-host/Cargo.toml`(`[[bin]]`)
- `crates/agent-host/tests/support/mod.rs`(대역 설치, 대역 전용 profile, `FixtureVerifier`, 디렉터리 snapshot)
- `crates/agent-host/tests/test_double.rs`, `crates/agent-host/tests/fixtures/profiles/test-double-{claude,codex}.json`

## 구현 요약

- **방식**: 테스트 전용 실행 파일을 임시 디렉터리에 CLI 이름(`claude`, `codex`)으로 복사하고, 옆에 `<exe>.scenario`를 둡니다. 대역은 자기 경로로 scenario를 찾고, 인자로 단계(`--version` → version, `auth status`·`login status` → status, 그 밖 → session, `--hang` → 하위 프로세스용 대기)를 고릅니다.
- **명령**: 출력 재생(`out`·`outfile`·`err`), 입력 경계 조각(`chunk`), 대기(`sleep`·`hang`), 대량 출력(`flood`), stdin 읽기·기록, 환경 이름·cwd·인자 기록, 실행 표시(`mark`), 흔적 시험 파일(`touch`), 하위 프로세스(`spawn-child`), 종료 코드(`exit`). XXIX.2.4\~XXIX.2.13과 XXIX.6 시나리오(세션 시작·중지·재연결 입력·디코드 실패·한도 오류·흔적)를 재현합니다.
- **환경 측정 전제**: 대역은 library를 쓰지 않아 framework를 link하지 않습니다(`otool -L`: `libSystem`만). CoreFoundation이 스스로 더하는 `__CF_USER_TEXT_ENCODING` 같은 변수가 없어 child 환경 이름 집합을 정확히 잽니다. 빈 환경으로 실행하면 대역이 보는 환경도 빈 것을 self-test로 확인합니다.
- **서명**: 대역은 공급자 서명을 가질 수 없으므로 `CodeVerifier`를 테스트가 주입합니다(`FixtureVerifier`, 결과 고정·호출 수 셈). 실제 Apple 확인 수단으로 대역을 확인하면 거부됨을 따로 검사합니다(XXIX.2.3).
- 대역 전용 profile fixture는 공식 profile을 복사해 활성·Team ID `TESTDBL001`·약관 `allowed`로 둔 테스트 파일이며 앱 번들에 넣지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| 대역 self-test(`tests/test_double.rs` 3개, bin 단위 test 2개): 단계 선택, escape, 출력 byte 재생, 종료 코드, scenario 없음 97·형식 오류 98, 빈 환경 유지 | 통과 |
| 네트워크 사용 0: 대역 소스에 `std::net`·socket·FFI 0(`test_double_uses_no_network_or_library`), crate 전체 network API 0 | 통과 |
| 실제 계정 사용 0: 대역은 HOME·CLI 설정을 읽지 않고, 테스트는 가짜 HOME(임시 디렉터리)만 넘김 | 통과(구조 + inventory `no_cli_credential_location_access`) |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 방식은 작업이 정하도록 되어 있었고 ADR-0092 결정 2에 기록했습니다.

## 남은 위험 / 한계

- 대역은 workspace bin이므로 `cargo build --workspace`에 함께 빌드됩니다. 앱 번들 스크립트는 helper를 이름으로 고르므로 들어가지 않지만, 번들 구성 검사(XXIV.5)가 생기면 이 이름을 제외 목록에 넣어야 합니다.
