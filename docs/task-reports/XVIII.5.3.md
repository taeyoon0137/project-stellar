# 작업 보고서: XVIII.5.3 worktree는 시스템 Git, 터미널은 cwd preset만

- WBS: XVIII.5.3
- 작업명: 일반 Git worktree 생성·사용은 시스템 Git 기능으로 남기고 terminal cwd preset만 제공. agent 전용 ShadowWorktree 타입을 만들지 않음
- 상태: 완료 (`[x]`)
- 선행: XVIII.1.3 `[x]`, XIV.1.2 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 시스템 Git·macOS PTY 통합 테스트, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/tests/worktree_type_inventory.rs`(신규) — type inventory 검사, 시스템 Git worktree → cwd preset 통합 테스트.
- `crates/terminal/src/cwd.rs` — 모듈 문서에 worktree 책임 경계(XVIII.5.3) 명시.
- `docs/task-reports/XVIII.5.3.md`(신규), 계획서(XVIII.5.3 `[ ]`→`[x]`).

## 구현 요약

- **책임 경계**: worktree 생성·삭제·전환은 시스템 Git(외부 터미널 또는 Git UI)이 하고, 터미널은 호출자가
  시스템 Git으로 얻어 공급한 worktree root 목록을 cwd preset(`CwdContext::presets`, XVIII.1.3)으로만
  제공합니다. 외부 CLI 에이전트 세션(XVIII.5.1)도 같은 preset·같은 profile 경로를 씁니다. 새 제품 코드는
  없고, 이 경계를 검사로 고정했습니다.
- **type inventory**(`worktree_type_inventory.rs`):
  - workspace의 Rust(`crates/`, `tools/`)와 Swift(`apps/`) 타입 선언 중 이름에 `Worktree`가 있고
    `Shadow`·`Agent`·`Sandbox`·`Ephemeral`이 함께 있는 agent 전용 worktree 타입이 0개입니다. 검사가 실제
    파일을 읽었는지 일반 Git worktree 타입이 1개 이상 있는지도 확인합니다(현재 `stellar-git-porcelain`의
    `WorktreeState`·`WorktreeKind`·`WorktreeEntry`).
  - terminal crate 소스에는 worktree 타입 선언, `"git"` 실행, `worktree add`/`worktree remove`,
    `git-porcelain` 참조가 없습니다.
  - 위반 fixture(`ShadowWorktree`·`AgentWorktreeLease` struct/enum, Swift class)는 검출하고, 일반 Git
    타입·enum variant는 검출하지 않습니다. fixture 이름은 실행 중에 만들어 이 테스트 파일도 workspace
    검사 대상에 그대로 남깁니다.
- **시스템 Git 통합**: 임시 저장소에서 시스템 Git으로 `worktree add`한 linked worktree를
  `git worktree list --porcelain` 결과로 받아 `CwdContext`에 넣으면 preset이 [프로젝트 루트, linked
  worktree]가 되고, 그 preset으로 시작한 세션의 `pwd -P`가 linked worktree 경로입니다. 실행 전후 worktree
  목록은 같습니다(터미널이 Git 상태를 바꾸지 않음). 사용자 Git 설정 영향을 막으려고
  `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`로 실행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | unit 114 passed / `no_cli_specific_code` 2 passed / `worktree_type_inventory` 4 passed / doctest 2 passed, 0 failed |
| `cargo test -p stellar-terminal` 10회 반복 | 실패 0, 반복 뒤 `pgrep -fl "sleep 300"` 결과 없음 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |
| `./scripts/macos/smoke.sh` | 실행하지 않음(앱 셸 변경 없음) |

| 검사 | 결과 |
|---|---|
| **type inventory**: workspace agent 전용 worktree 타입 0개, 일반 Git worktree 타입은 읽힘(`workspace_has_no_agent_only_worktree_types`) | 통과 |
| terminal crate: worktree 타입 선언·Git 실행·worktree 생성/삭제 명령·git-porcelain 참조 0건(`terminal_crate_does_not_run_git_or_define_worktree_types`) | 통과 |
| 위반 fixture 검출, 일반 타입·variant 비검출(`detector_flags_agent_only_worktree_types`) | 통과 |
| 시스템 Git linked worktree → cwd preset → 그 cwd에서 세션 시작, worktree 목록 불변(`system_git_worktrees_become_cwd_presets`) | 통과 |

## 차이(작업 지시와 다른 점)

- 검사는 workspace 전체 타입을 보지만 terminal crate의 통합 테스트로 두었습니다(이 lane의 변경 범위가
  `crates/terminal/**`). MCP·agent model 쪽 전용 inventory는 XIX.8.1이 따로 다룹니다.

## 남은 위험 / 한계

- 이름 기반 검사입니다. 다른 이름으로 agent 전용 worktree 개념을 만드는 경우는 리뷰와 XIX.8.1 검사로
  막아야 합니다.
- 시스템 Git 통합 테스트는 개발 장비에 `git`이 있어야 합니다(없으면 실패로 드러남).
- 앱 셸이 시스템 Git(XIV.1.2)으로 worktree 목록을 얻어 `CwdContext`에 넣는 연결은 터미널 view(XVIII.4.3)
  작업에서 합니다.
