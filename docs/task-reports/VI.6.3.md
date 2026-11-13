# 작업 보고서: VI.6.3 path point·handle StableId

- WBS: VI.6.3
- 작업명: Path point와 handle에 StableId를 부여해 개별 point diff 가능하게 함
- 상태: 완료 (`[x]`)
- 선행: VI.6.1, III.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/path.rs` — `PathPointId`, `PathHandleId`, `CommandIds`,
  `IdentifiedPath`(`new`/`assign`/`point_ids`/`insert_line_point`),
  `PathError::{IdShapeMismatch, DuplicateId, IndexOutOfRange}`.
- `docs/task-reports/VI.6.3.md`(신규), 계획서(VI.6.3 `[ ]`→`[x]`).

## 구현 요약

- **id 구성**: 끝점이 있는 command(MoveTo·LineTo·CubicTo)는 point id, CubicTo는 두
  handle id를 추가로 가지며 Close에는 id가 없습니다. 서로 다른 newtype이라 point와
  handle id가 섞이지 않습니다.
- **불변식**: id 목록 길이 = command 수, 구성이 종류와 일치, 모든 id 유일 — 위반은
  생성 시 거부합니다.
- **편집**: `insert_line_point`는 기존 point·handle id를 그대로 두고 새 point만 새
  id를 받습니다. 결과가 subpath 규칙(VI.6.2)이나 유일성을 어기면 거부하고 path는
  변경하지 않습니다. semantic diff(Phase XIV)가 point 단위로 대응할 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 103 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| point 삽입 후 기존 point·handle id 유지(`inserting_a_point_keeps_existing_ids`) | 통과 |
| 규칙 위반 삽입·범위 밖·중복 id 거부 시 불변(`invalid_insertions_are_rejected_without_change`) | 통과 |
| id 구성 불일치 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- id를 포함한 저장 인코딩은 document→chunk serializer(VI.10.2)에서 정의합니다.
