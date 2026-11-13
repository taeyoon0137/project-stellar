# 작업 보고서: XVIII.4.5 일반·worktree 터미널 header metadata

- WBS: XVIII.4.5
- 작업명: 일반 terminal과 특정 worktree terminal의 header metadata를 분리 표시할 data model 추가
- 상태: 완료 (`[x]`)
- 선행: XVIII.1.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/header.rs`(신규) — `TerminalHeader`, `HeaderScope`, `CwdWarning`.
- `crates/terminal/src/registry.rs` — `TerminalRegistry::header`.
- `crates/terminal/src/lib.rs` — `header` 모듈.
- `docs/task-reports/XVIII.4.5.md`(신규), 계획서(XVIII.4.5 `[ ]`→`[x]`).

## 구현 요약

- **범위 분리**: `HeaderScope::Project{root}`(프로젝트 루트에서 시작한 일반 터미널),
  `Worktree{root}`(사용자가 고른 worktree 귀속), `Custom`(명시 입력 cwd, 귀속 없음). 시작 뒤에는
  XVIII.1.3의 `ResolvedCwd` 귀속을, 시작 전에는 profile cwd 선택을 씁니다. `is_worktree()`와 고정
  코드(`project`·`worktree`·`custom`)로 UI가 두 종류를 다르게 표시할 수 있습니다.
- **header 값**: 제목(profile 이름), 범위, 시작 cwd, 현재 cwd(호출자 관찰값, 없으면 시작 cwd; symlink는
  해석), 현재 cwd를 담은 가장 깊은 worktree root, 경고 목록.
- **잘못된 cwd 경고**(`CwdWarning`):
  - `InsideOtherWorktree`: 현재 cwd를 담은 가장 깊은 worktree가 귀속 worktree(프로젝트 터미널은
    프로젝트 루트가 속한 worktree)와 다름 — 프로젝트 폴더 안에 있는 linked worktree로 이동한 경우 포함.
  - `OutsideBinding`: 어떤 다른 worktree에도 있지 않으면서 귀속 root 밖으로 이동.
  - `BindingMissing`: 귀속 worktree가 창의 현재 worktree 목록에서 사라짐(삭제·이동 뒤 다시 만든
    `CwdContext`).
  - 명시 cwd(`Custom`) 터미널은 귀속이 없어 이동 경고를 내지 않습니다.
- worktree 판정은 `CwdContext`의 root 목록(시스템 Git이 보고한 값)으로만 하고 셸 출력을 해석하지
  않습니다. `TerminalRegistry::header(id, context, observed_cwd)`가 세션에서 header를 만듭니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 61 passed, 0 failed |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 일반(project)·worktree·custom 범위 분리와 코드(`general_and_worktree_terminals_have_distinct_scopes`) | 통과 |
| wrong cwd session 경고: worktree→다른 worktree, worktree→밖, project→내부 linked worktree, project→밖은 경고; 같은 root 하위 이동·custom은 경고 없음(`wrong_cwd_sessions_are_warned`) | 통과 |
| 귀속 worktree 목록 제거 시 `BindingMissing`(`removed_worktree_binding_is_warned`) | 통과 |
| 시작 전 header는 선택으로 범위 판단(`header_before_start_uses_selection`) | 통과 |
| registry header: 일반·worktree 세션 구분, 이동 경고, 닫은 세션 `None`(`registry_header_separates_general_and_worktree_sessions`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. header를 실제로 그리는 UI와, 셸의 현재 cwd를 관찰하는 방법(예: emulator가 받는 작업 디렉터리
  보고, 또는 foreground 프로세스 cwd 조회)은 이 data model의 입력으로 남겼습니다. emulator(XVIII.4.1~4.3)는
  선행 차단 상태입니다.

## 남은 위험 / 한계

- 현재 cwd 관찰값을 주지 않으면 경고는 시작 cwd 기준(귀속 누락 여부)만 판단합니다.
