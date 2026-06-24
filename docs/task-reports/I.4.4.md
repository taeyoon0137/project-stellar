# 작업 보고서: I.4.4 Bootstrap CI 정책 검사

- WBS: I.4.4
- 작업명: Bootstrap CI에 sync-plan·Phase I validator·commit subject·ADR 검사 추가
- 상태: `[~]` (워크플로·스크립트 작성 및 로컬 동등 검증 완료. 실제 CI 실행·Windows job·fixture-branch 실패 검증은 CI에서 확보 — 미완료)
- 선행: I.4.2, I.4.3 (`[x]`)

## 필수 실행 환경

- `CI-MAC`, `CI-WIN`.

## 변경 파일 목록

- `.github/workflows/bootstrap.yml` (plan-consistency·commit-subject·ADR step 추가, job당 13 step)
- `scripts/ci/check-commit-subjects.sh` (신규)
- `scripts/ci/check-adr-policy.sh` (신규)
- `docs/task-reports/I.4.4.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.4.4 `[ ]` → `[~]`)

## 구현 요약

- 각 job에 추가: `sync-plan --check`, `validate-task-status`, `validate-versions`, `validate-dependencies`, commit-subject 검사, ADR 정책 검사.
- commit-subject: 기준 commit 이후 non-merge commit을 `Task(<ID>):` 전용으로 검사(예외 prefix 없음). 범위는 PR base 또는 `BASELINE_REF`.
- ADR: 변경 경로를 `validate-adr`로 검사.

## 실행한 명령 / 검증 (`DEV-MAC` 로컬 동등)

| 검사 | 결과 |
|---|---|
| YAML 문법(ruby) | OK, 2 job × 13 step |
| `check-commit-subjects.sh` (Task 커밋 범위) | 통과 |
| `check-adr-policy.sh` (비보호 변경) | 통과 |
| sync-plan/validate-task-status/versions/dependencies | 전부 통과 |

## 미완료(연기) — `[~]` 사유

- 실제 GitHub Actions 양 runner 성공, Windows job, 그리고 task ledger·commit subject·dependency·ADR 정책을 각각 깨뜨린 fixture branch에서 해당 job 실패는 push 후 CI에서만 확인 가능. 확보 후 `[x]`로 승격.

## 남은 위험 / 후속

- commit-subject 검사는 `Task(...)` 전용이라 부트스트랩의 `Docs:`/`Chore:` 유지보수 커밋과 충돌합니다. CI 도입 시 `BASELINE_REF`를 유지보수 커밋 이후로 설정해야 합니다(I.2.17 보고서와 동일 이슈).

## 남은 경고 수

- 0
