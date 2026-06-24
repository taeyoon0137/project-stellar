# 작업 보고서: I.2.12 task-status ledger 생성

- WBS: I.2.12
- 작업명: 계획서 추출로 `planning/task-status.yaml` 생성(일회성 subcommand)
- 상태: 완료 (`[x]`)
- 선행: I.2.8, I.2.9, I.2.10, I.2.11 (모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/generate.rs` (신규, 빌드·검증·git 수집·파일 생성)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `tools/xtask/src/main.rs` (`generate-ledger` 서브커맨드 연결)
- `planning/task-status.yaml` (신규, 생성물 — 950 tasks)
- `docs/task-reports/I.2.12.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.12 `[ ]` → `[x]`)

## 구현 요약

- `cargo xtask generate-ledger`: 계획서 → `extract_wbs`(I.2.10) → required_environments 파싱(검증 열 `[ENV]` 태그, 없으면 DEV-MAC) → `git log`에서 `Task(<ID>):` 커밋 SHA·시각 수집 → 레코드 구성 → 전체 validator → `write_ledger`(I.2.11) → 파일 기록.
- 상태 매핑: 계획 `[x]`→verified, `[~]`→reported_done, `[ ]`→not_started, `[!]`→blocked.
- verified 작업: report·commit·verified_at·환경 evidence 채움. reported_done: report. not_started: 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo xtask generate-ledger` | 950 tasks 생성, 종료 0 |
| record·collection·graph·evidence validator | 모두 통과(통과 못하면 파일 미기록) |
| decoder round-trip | 통과 |
| ledger ID 집합 == 계획 추출 ID 집합 | 동일(추출 결과로 구성) |
| 끝 newline / `planning/` 추적 | 정상 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 52 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 계획 표에 title 열이 없어 title은 `구현 지시` 열을 사용했습니다(I.2.10과 동일). required_environments는 검증 열의 `[ENV]` 태그에서 도출했습니다.
- 재생성은 동일 git 이력에서 결정론적입니다(commit SHA·committer date 기반).
