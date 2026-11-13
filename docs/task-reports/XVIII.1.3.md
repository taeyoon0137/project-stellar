# 작업 보고서: XVIII.1.3 터미널 cwd preset과 검증

- WBS: XVIII.1.3
- 작업명: terminal cwd를 project root 또는 사용자가 선택한 worktree root preset으로 제공하고 임의 cwd도 명시 입력으로 허용
- 상태: 완료 (`[x]`)
- 선행: XIV.1.2, XVIII.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/cwd.rs` — `CwdContext`, `CwdPreset`, `CwdBinding`, `ResolvedCwd`, 실행 시점 검증, `CwdError` 확장.
- `crates/terminal/src/test_support.rs`(신규, test 전용) — canonical 임시 디렉터리.
- `crates/terminal/src/lib.rs` — `test_support` 모듈, 모듈 설명.
- `docs/task-reports/XVIII.1.3.md`(신규), 계획서(XVIII.1.3 `[ ]`→`[x]`).

## 구현 요약

- **preset 기준**: `CwdContext::new(project_root, worktree_roots)`. worktree root 목록은 호출자가 시스템
  Git으로 얻어 공급합니다(XIV.1.2의 `git rev-parse --show-toplevel` 등 — 이 crate는 core-types에만
  의존하므로 Git을 실행하거나 `.git` 구조로 경로를 추측하지 않습니다). 모든 경로를 canonical로
  바꾸고 worktree root는 정렬·중복 제거, 256개 상한입니다.
- **preset 목록**: 프로젝트 루트가 먼저, 그다음 worktree root(프로젝트 루트와 같은 경로 제외).
  `CwdPreset::selection()`으로 그대로 세션 선택이 됩니다.
- **임의 cwd**: `ExplicitCwd::from_user_input`(XVIII.1.1)으로 명시 입력한 경로만 허용합니다.
- **실행 시점 검증**(`CwdContext::resolve`): 문법(비어 있음·NUL·1024 byte·절대 경로) → `canonicalize`
  (없음 `NotFound`, 그 밖 I/O `Inaccessible{kind}`) → symlink 해석 뒤 길이 재검사 → 디렉터리 여부
  (`NotDirectory`). worktree 선택은 공급된 root와 canonical 경로가 일치해야 하며(`UnknownWorktree`),
  worktree 내부 하위 디렉터리는 명시 입력으로만 허용합니다. 결과 `ResolvedCwd`는 canonical 경로와
  귀속(`ProjectRoot`·`Worktree`·`Unbound`)을 가지며 header metadata(XVIII.4.5)가 사용합니다.
  context를 만든 뒤 프로젝트 루트가 사라지면 resolve 시점에 `NotFound`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 25 passed, 0 failed |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| cwd validation fixture | 결과 |
|---|---|
| preset 순서·중복 제거·프로젝트 루트 제외, preset 선택 해석(`presets_list_project_root_then_worktree_roots`) | 통과 |
| 공급되지 않은 worktree·worktree 하위 디렉터리 거부, 명시 입력은 허용·`Unbound`(`worktree_selection_must_be_a_supplied_root`) | 통과 |
| 없는 경로 `NotFound`, 파일 `NotDirectory`(`explicit_cwd_must_be_an_existing_directory`) | 통과 |
| symlink root·명시 입력의 canonical 해석(`symlinked_directories_resolve_to_canonical_path`) | 통과 |
| 상대 경로·없는 root·root 수 상한(`context_rejects_invalid_roots`) | 통과 |
| context 생성 뒤 삭제된 프로젝트 루트 감지(`removed_project_root_is_detected_at_resolve_time`) | 통과 |
| 명시 입력 문법(빈 값·상대·NUL·길이, `explicit_cwd_syntax_is_checked`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. worktree root 목록을 이 crate가 직접 조회하지 않고 호출자 공급으로 받는 것은 crate 의존
  정책(ADR-0019: core-types만 허용)에 따른 경계입니다.

## 남은 위험 / 한계

- 검증과 실제 `chdir` 사이에 디렉터리가 사라지는 경쟁은 막을 수 없습니다. 그 경우 child 생성이
  실패(`StartFailed`)로 보고됩니다(XVIII.2.1).
- 프로젝트 창이 worktree 목록을 언제 갱신해 `CwdContext`를 다시 만드는지는 앱 셸 연결 범위입니다.
