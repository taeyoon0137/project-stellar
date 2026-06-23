# 저장소 기준선 (Repository Baseline)

- WBS: I.1.1
- 기준일: 2026-06-23
- 목적: 현재 저장소의 실제 상태(루트, HEAD, dirty 여부)를 재검증해 기록합니다.

> 보안 원칙에 따라 절대 경로는 기록하지 않습니다. 경로는 저장소 상대 경로로만 표기합니다.

## 실행 명령과 결과

| 명령 | 종료 코드 | 결과 요약 |
|---|---|---|
| `git rev-parse --show-toplevel` | 0 | 저장소 최상위 디렉터리 확인됨 |
| `git status --porcelain=v2` | 0 | tracked 변경 없음, untracked 1건 |
| `git log -1 --format=%H` | 0 | HEAD 커밋 해시 확인됨 |

## 저장소 상태

- 저장소 루트: `project-stellar` (저장소 최상위, 상대 경로 기준 `.`)
- HEAD: `59b1416be23fd187dafd7dfed79834264011aa94`
- dirty 여부:
  - tracked 변경: 없음 (clean)
  - untracked: `.vscode/` 1건 — 작업 환경 디렉터리이며 Git 추적 대상이 아닙니다(`.gitignore`에 포함하지 않는 정책).
