# 차단 보고서: XV.6.4 linked worktree path도 ProjectProbe로 허용

## WBS

- WBS: XV.6.4
- 작업명: linked worktree path도 ProjectProbe로 허용
- 상태: `[!]`
- 선행: XIV.1.2(`[x]`), XV.6.2(`[~]`)

## 실패 선행조건

- 엄격 선행 XV.6.2가 `[~]`입니다.
- XV.6.2(`[~]`)는 local override 대상의 "일반 repo" 판정을 실제 `ProjectProbe`로 하는 project crate 연결이 없어 부분 완료입니다(현재 테스트는 표 기반 probe fake).
- 이 lane(deps_wiring)의 허용 경로는 `crates/dependencies/**`와 project crate의 manifest·lockfile·YAML writer 파일뿐이라 probe·adapter 파일을 만들거나 고칠 수 없습니다.

## 재현 명령

- `grep -E '^\| XV\.6\.2 \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- XV.6.2 연결(실제 `probe_project`)이 먼저 필요합니다. `probe_project`는 gitfile(`GitLink`)을 이미 구분하므로 연결 후 linked worktree fixture를 추가할 수 있습니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.6.2 `[x]` 후 착수합니다(`git worktree add` fixture를 local override 대상으로 검증).
