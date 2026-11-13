# 작업 보고서: XVIII.1.2 프로젝트 창별 터미널 세션 registry

- WBS: XVIII.1.2
- 작업명: 프로젝트 창당 여러 terminal session registry 구현
- 상태: 완료 (`[x]`)
- 선행: XVIII.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/registry.rs`(신규) — `TerminalRegistry`, `TerminalSession`, `RegistryLimits`, `RegistryError`.
- `crates/terminal/src/lib.rs` — `registry` 모듈 추가.
- `docs/task-reports/XVIII.1.2.md`(신규), 계획서(XVIII.1.2 `[ ]`→`[x]`).

## 구현 요약

- **창 귀속**: registry는 core-types `WindowSessionId`(프로젝트 창 하나)에 귀속되고, 그 창의 세션
  여러 개를 `BTreeMap<TerminalSessionId, TerminalSession>`에 둡니다. 다른 창 registry의 세션은
  조회·닫기·handle 해석이 모두 실패합니다.
- **한도**: `RegistryLimits::max_sessions`(기본 32). 상한에 도달하면 `LimitReached`, 세션을 닫으면
  다시 만들 수 있습니다.
- **식별자**: registry마다 `SessionIdAllocator`를 하나 가지며 순번을 재사용하지 않습니다.
  `resolve_handle`은 이 registry가 발급했고 **아직 열린** 세션만 돌려줍니다.
- **create/close**: `create(profile)`은 profile을 복사해 `NotStarted` 세션을 만들고, `close(id)`는
  세션을 제거하고 제거 직전 상태를 돌려줍니다. 세션 목록은 발급 순서입니다. 프로세스 연결은
  XVIII.1.4(시작 gate)와 XVIII.2.x(PTY)에서 이어 붙입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 19 passed, 0 failed |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 20 session create/close × 5회: 매회 registry·내부 map 비움, 닫은 id 조회·handle 해석·재닫기 실패, id 재사용 0(`twenty_sessions_create_and_close_without_leaks`) | 통과 |
| 세션 상한·닫은 뒤 재생성(`session_limit_is_enforced_and_freed_by_close`) | 통과 |
| 창별 registry 격리(`registries_of_different_windows_do_not_share_sessions`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 이 단계의 "누수 없음"은 세션 record 수준입니다. 실제 PTY fd·child process 누수 검사는 PTY가 붙는
  XVIII.2.4에서 20 session start/close fixture로 다시 검증합니다.
