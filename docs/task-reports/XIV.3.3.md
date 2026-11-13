# 작업 보고서: XIV.3.3 StableId 전용 object match

- WBS: XIV.3.3
- 작업명: object match는 StableId로만 수행하고 이름·geometry 유사도로 rename 추정하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.3.1 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/diff.rs`(신규) — `diff`, `object_provenance`.
- `crates/semantic-diff/src/lib.rs` — `diff` module 선언.
- `docs/task-reports/XIV.3.3.md`(신규), 계획서(XIV.3.3 `[ ]`→`[x]`).

## 구현 요약

- **대응**: 두 snapshot을 StableId 오름차순으로 한 번 merge-join합니다(객체 수에 선형).
  한쪽에만 있는 id는 RemoveObject/AddObject(token은 RemoveToken/AddToken)이고, 변경에는 객체
  전체가 들어갑니다. 이름·geometry·내용 유사도는 전혀 보지 않습니다.
- **동일 id·다른 정체성**: 같은 StableId라도 종류(`ObjectKind`)나 범위(`Origin`)가 다르면 속성
  단위 비교가 불가능한 다른 객체로 보고 삭제 + 추가를 냅니다(같은 id에서 삭제가 먼저).
- **출처**: 기본 출처를 객체 범위에서 유도합니다(Project/Document/ComponentDefinition).
- 같은 정체성 객체의 속성·위치 비교는 XIV.3.4·XIV.3.5 범위입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 22 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 같은 이름·같은 크기·같은 위치, 다른 id → RemoveObject + AddObject, rename 추정 없음(`same_name_different_id_is_add_and_remove`) | 통과 |
| 같은 id·다른 node 종류 → 삭제 + 추가(`same_id_different_kind_is_replace`) | 통과 |
| 동일 snapshot 변경 0, 전체 추가·삭제와 StableId 오름차순(`identical_snapshots_have_no_changes`, `whole_snapshot_add_and_remove`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 사용자가 복사·붙여넣기로 새 id를 만든 경우 의도상 "이동"이라도 삭제 + 추가로 보입니다.
  지시대로 추정하지 않는 것이 정책입니다.
