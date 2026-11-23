# 작업 보고서: XXIX.2.5 구독 CLI 연결 상태 판정

- WBS: XXIX.2.5
- 작업명: 각 CLI 공식 문서가 제공하는 방법으로만 연결 상태를 판정하고, 판정할 수 없으면 사용 불가로 처리
- 상태: `[x]` 완료
- 선행: XXIX.2.3, XXIX.2.4
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/status.rs`(신규): `ConnectionState`, `judge`, `probe_connection`, `spawn_probe`
- `crates/agent-host/tests/connection_status.rs`

## 구현 요약

- **방법**: profile의 `status_command`와 `status_rule`만 씁니다. Claude Code CLI reference는 `claude auth status`가 "로그인했으면 종료 코드 0, 아니면 1"이라고 문서화합니다. 그래서 규칙은 `exit-code`(0 = 연결됨, 1 = 연결 안 됨, 그 밖 = 판정 불가)입니다.
- **출력 비보관**: 상태 명령 stdout 보관 상한이 0이고 stderr는 버립니다. CLI가 보고한 계정 식별 정보(메일·조직 등)가 Stellar 메모리·저장·로그·진단에 들어오지 않습니다(XVI.8). 판정 결과는 enum뿐입니다.
- **자격 증명 위치 접근 0**: CLI 설정 디렉터리, 인증 파일, Keychain을 읽거나 열지 않습니다(inventory).
- **fail closed**: 실행 실패, timeout, signal 종료, 문서에 없는 종료 코드, 실행 파일 변경은 `Undeterminable`이며 사용 불가입니다.
- **스레드**: 판정은 프로세스를 실행하므로 UI·Document 스레드 밖에서 합니다. `spawn_probe`는 worker 스레드에서 판정해 channel로 돌려줍니다.
- **상태 정의**: `Connected`, `NotConnected`(사용자가 CLI 공식 절차로 직접 로그인해야 함, Stellar는 로그인 흐름을 제공하지 않음), `Undeterminable(code)`.
- **판정 시점**: 세션 실행 전 확인(XXIX.2.7) 때마다, 그리고 설정 화면의 상태 새로 고침(XXIX.4.8)입니다. 결과를 캐시하지 않습니다.

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
| 테스트 대역 상태 fixture: 종료 0 → Connected, 1 → NotConnected(둘 다 계정 문자열 출력) | 통과 |
| 판정 불가 fixture: 종료 2·127, hang(timeout 300 ms) → Undeterminable, 사용 불가 | 통과 |
| 연결 안 됨이면 실행 전 확인 거부, 세션 실행 0 | 통과 |
| 진단·거부 이유·오류 코드에 계정 식별 문자열 0 | 통과 |
| CLI 자격 증명 저장 위치 접근 inventory 0(`no_cli_credential_location_access`) | 통과 |
| 호출 스레드 비차단(`spawn_probe`) | 통과 |
| 판정 뒤 scratch 정리 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- Codex CLI의 `codex login status` 종료 코드 의미는 이번 실행에서 공식 문서로 확인하지 못했습니다. Codex profile은 비활성이라 실행되지 않습니다.

## 남은 위험 / 한계

- Codex profile을 활성화하기 전에 상태 판정 방법을 공식 문서로 확인해 `status_rule`을 정해야 합니다. 규칙이 문서와 다르면 새 `StatusRule` variant가 필요합니다.
