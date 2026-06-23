# 작업 보고서: I.2.7 레코드 집합 검증

- WBS: I.2.7
- 작업명: `validate_collection` — task ID 중복, 허용 상태, title mismatch 기본 검사
- 상태: 완료 (`[x]`)
- 선행: I.2.6 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/validate_collection.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.7.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.7 `[ ]` → `[x]`)

## 구현 요약

- `validate_collection(&[TaskStatusRecord]) -> Vec<CollectionError>`.
- 코드: `duplicate-id`(같은 ID 재등장), `title-mismatch`(같은 ID인데 title 다름).
- 허용 상태는 `TaskState` 타입과 decoder(I.2.5)에서 강제되므로, "unknown status"는 decode 경계 테스트로 확인.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 고유 ID 집합 | 오류 0 |
| 중복 ID | `duplicate-id` |
| 같은 ID·다른 title | `title-mismatch` |
| unknown status(decode 경유) | `unknown-status` 거부 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 30 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "허용하지 않는 상태" 거부는 타입(enum)·decoder 경계에서 이미 강제되어, collection 단계에서는 ID·title에 집중했습니다(I.2.5 보고서와 동일한 layering).
