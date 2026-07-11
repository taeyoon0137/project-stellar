# 작업 보고서: VI.2.2 공통 Node column 확장

- WBS: VI.2.2
- 작업명: 공통 Node column에 StableId, kind, parent, OrderKey, visible, locked, display name
- 상태: 완료 (`[x]`)
- 선행: III.6.2, VI.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/node_columns.rs` — `NodeFields`/`NodeColumns`에 `kind`
  (VI.2.1), `visible`, `locked`, `display_name` column 추가, setter 3종 추가,
  column invariant 검사 확장.
- `crates/document/src/snapshot.rs` — 테스트 helper 갱신.
- `docs/task-reports/VI.2.2.md`(신규), 계획서(VI.2.2 `[ ]`→`[x]`).

## 구현 요약

- **SoA 유지**: 새 필드 4종이 각각 독립 column(`Vec`)으로 추가되고, 삽입·재사용
  경로 모두에서 모든 column이 같은 길이를 유지합니다(III.6.2 invariant). 10,000
  회 무작위 삽입·삭제 invariant 테스트가 7개 column 전체로 확장되었습니다.
- **setter**: `set_visible`/`set_locked`/`set_display_name` — stale 핸들 거부.
  표시 이름 변경은 stable id에 영향이 없습니다(VI.1.3 정책과 일관).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 34 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 확장 필드 round-trip(`extended_common_fields_round_trip`) | 통과 |
| column invariant(7 column, 10,000회 연산)(`columns_aligned_after_insert_delete`) | 통과 |
| 새 setter 갱신·stale 거부(`new_setters_update_and_reject_stale`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- parent-child 삽입 validator(VI.2.3)와 OrderKey 정렬 자식 조회(VI.2.4)가
  후속입니다.
