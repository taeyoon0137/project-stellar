# 작업 보고서: XVIII.1.1 터미널 세션 기본 타입

- WBS: XVIII.1.1
- 작업명: `TerminalSessionId`, profile, cwd, environment allowlist, process state 정의 — project 저장·recovery snapshot 제외
- 상태: 완료 (`[x]`)
- 선행: G8(gate 수준 선행 — 사용자 지시 2026-11-14에 따라 완화, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/lib.rs` — 모듈 구성, 저장 비대상 원칙 문서, 의존성 고정 테스트.
- `crates/terminal/src/id.rs`(신규) — `TerminalSessionId`, `SessionIdAllocator`, `SessionIdExhausted`.
- `crates/terminal/src/profile.rs`(신규) — `ProfileName`, `ArgvCommand`, `ProfileCommand`, `TerminalProfile`.
- `crates/terminal/src/env.rs`(신규) — `EnvName`, `EnvironmentPolicy`, `DEFAULT_INHERITED_ENV`.
- `crates/terminal/src/cwd.rs`(신규) — `CwdSelection`, `ExplicitCwd`(문법 검사).
- `crates/terminal/src/state.rs`(신규) — `TerminalProcessState`, `TerminalExitStatus`, `TerminationReason`, `StartFailure`.
- `docs/task-reports/XVIII.1.1.md`(신규), 계획서(XVIII.1.1 `[ ]`→`[x]`).

## 구현 요약

- **식별자(세션 메모리 전용)**: `TerminalSessionId`는 발급기 epoch와 순번을 품습니다. epoch는
  프로세스마다 OS 난수로 seed되는 `RandomState`에서 얻은 기준값에 프로세스 안 단조 counter를 더해
  만듭니다. `to_handle()`의 `u64`는 FFI·UI 경계용이며, 되돌리기(`decode`)는 발급한 allocator만
  합니다. 다른 registry나 재실행 뒤 registry는 같은 handle을 해석하지 않고, 미발급 순번·순번 0도
  거부합니다. 직렬화 trait과 문자열 parser는 두지 않았습니다. 순번은 재사용하지 않고 `u32` 공간을
  다 쓰면 `SessionIdExhausted`입니다.
- **profile**: 명령은 argv 배열(`ArgvCommand`: 절대 경로 프로그램 + 인자)이며 셸 문자열로 저장·해석하지
  않습니다. NUL byte, 인자 256개 초과, 인자·경로 32 KiB 초과를 거부합니다. `ProfileCommand`는
  `DefaultShell`(XVIII.2.2에서 설정 셸로 해석)과 `Argv`입니다. 이름은 trim 후 1~64자, 제어 문자 금지.
- **환경 허용 목록**: 실행 시 환경을 모두 지우고 상속 허용 목록(이름만, 값은 실행 시점 앱 환경)과
  명시 지정(이름=값)만 전달하는 정책입니다. 명시 지정이 상속을 이깁니다. 이름은
  `[A-Za-z_][A-Za-z0-9_]*`, 항목 256개·이름 256 byte·값 32 KiB 상한, 값 NUL 금지. 결과는 이름 순.
  기본 상속 목록은 HOME·LANG·LC_ALL·LC_CTYPE·LOGNAME·PATH·TMPDIR·TZ·USER입니다.
- **cwd**: `ProjectRoot`·`WorktreeRoot(path)`·`Explicit(ExplicitCwd)`. 명시 입력은
  `ExplicitCwd::from_user_input`으로만 만들며 비어 있음·NUL·1024 byte 초과·상대 경로를 거부합니다.
  존재·디렉터리 검사와 preset은 XVIII.1.3 범위입니다.
- **process state**: `NotStarted`·`Running{pid}`·`Finished{status, reason, remaining_processes}`·
  `StartFailed{failure}`와 고정 코드 문자열.
- **저장 제외**: crate 의존성이 `stellar-core-types` 하나뿐이고 storage·project·document·serde가 없음을
  테스트(`terminal_state_has_no_persistence_dependency`)로 고정했습니다. 따라서 터미널 타입은 project
  저장·recovery snapshot 경로에 들어갈 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 16 passed, 0 failed |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| handle round-trip은 발급 allocator 안에서만 성공, 다른 allocator·미발급 순번 거부(`handle_round_trip_is_limited_to_the_issuing_allocator`) | 통과 |
| 순번 재사용 없음·소진 오류(`ids_are_unique_and_never_reused`, `exhausted_allocator_reports_error`) | 통과 |
| 허용 목록 밖 변수 미전달·명시 지정 우선·상한(`env::tests::*`) | 통과 |
| argv 원문 보존·상대 경로·NUL·상한 거부(`profile::tests::*`) | 통과 |
| 저장 경로 의존성 0(`terminal_state_has_no_persistence_dependency`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 G8(초기 릴리스 gate)은 gate 수준 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 필요한
  코드(core-types)는 이미 있습니다.

## 남은 위험 / 한계

- epoch는 32bit라 서로 다른 실행의 epoch가 우연히 같을 확률이 0은 아닙니다. handle은 저장하지 않는
  것이 원칙이고, 해석은 살아 있는 세션 범위에서만 의미가 있습니다(XVIII.1.2 registry).
