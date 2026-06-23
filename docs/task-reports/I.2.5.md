# 작업 보고서: I.2.5 TaskStatus decoder

- WBS: I.2.5
- 작업명: 제한 YAML 값 → `TaskStatusRecord` 디코더 (구조·형식 검증)
- 상태: 완료 (`[x]`)
- 선행: I.2.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/decode.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.5.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.5 `[ ]` → `[x]`)

## 구현 요약

- `decode_collection(&YamlValue) -> Result<Vec<TaskStatusRecord>, DecodeError>`.
- top-level은 `tasks` sequence 하나만 허용. 거부 코드: `top-level-not-mapping`, `unknown-top-key`, `missing-tasks`, `tasks-not-sequence`, `task-not-mapping`, `unknown-key`, `duplicate-key`, `type-mismatch`, `unknown-status`, `unknown-environment`, `missing-status`, `missing-field`.
- task key 화이트리스트(10개), evidence key 화이트리스트(3개). 알 수 없는/중복 key, scalar/sequence 형식 불일치 거부.
- 상태 규칙·선행 graph는 검사하지 않음(I.2.7, I.2.8).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 full record fixture | 디코드 일치 |
| top-level/tasks/task 형식 위반 | 각 코드 반환 |
| unknown-key / duplicate-key / type-mismatch | 각 코드 반환 |
| unknown-status / unknown-environment | 각 코드 반환 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 24 passed |

## 작업 지시와 다른 점

- `TaskStatusRecord.status`가 타입(enum)이라(I.2.1) 상태·환경 토큰 파싱을 decode 단계에서 수행하며 알 수 없는 토큰을 여기서 거부합니다. 따라서 I.2.7의 "unknown status" 거부는 본 decode 경계에서 강제됩니다(I.2.7은 중복 ID·title 검사에 집중).

## 남은 경고 수

- 0
