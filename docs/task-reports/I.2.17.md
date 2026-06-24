# 작업 보고서: I.2.17 commit subject validator

- WBS: I.2.17
- 작업명: `tools/xtask/src/commit_message.rs` — `Task(<WBS-ID>):` 전용 validator
- 상태: 완료 (`[x]`)
- 선행: I.2.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/commit_message.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod commit_message` 추가)
- `docs/task-reports/I.2.17.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.17 `[ ]` → `[x]`)

## 구현 요약

- `validate_commit_subject(subject) -> Option<&str>` (None = 통과).
- 허용: `Task(<WBS-ID>): <비어 있지 않은 제목>`. 오류 코드: `not-task-prefix`, `malformed-id`, `bad-separator`, `empty-title`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| Roman ID 3개 | 통과 |
| lowercase·Arabic·Docs·Chore·빈 제목·malformed·merge·revert | 각 오류 코드 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 63 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점 / 후속

- 본 validator는 Task 전용으로 엄격합니다. I.4.4의 CI commit-subject 검사가 "기준 commit 이후 모든 non-merge commit"에 이를 적용하면, 부트스트랩 과정의 `Docs:`/`Chore:` 유지보수 커밋과 충돌합니다. 기준 commit 경계 설정 또는 적용 범위는 I.4.4(CI) 단계에서 확정해야 합니다(현재 `.githooks/commit-msg`·AGENTS.md는 유지보수 prefix를 허용).
