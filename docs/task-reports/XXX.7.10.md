# 작업 보고서: XXX.7.10 Context Semantic Diff·Merge

- WBS: XXX.7.10
- 작업명: ContextKey·provision·requirement·Slot 공개 목록의 StableId 단위 diff와 전용 merge conflict.
- 상태: 부분 완료 (`[~]` 후보) — 투영·역투영·전용 conflict와 검증 기준 시험은 있지만 제품 snapshot 조립(`stellar diff`·compare·merge)이 아직 Context를 넣지 않습니다(아래, 통합 검토에서 확인). 보고 당시 기록: 완료(`[x]` 후보).
- 결정: ADR-0099 결정 10, 아키텍처 27.1
- 선행: XXX.7.8, XIV.3.7, XIV.10.5

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/semantic-diff/src/context.rs`(신규, `build::context`): 투영·역투영, 속성 key.
- `crates/semantic-diff/src/{build,snapshot}.rs`: 하위 module 등록, `ObjectKind::ContextKey`(0x0801)·`ContextProvision`(0x0802),
  `SnapshotError::ContextOwnerMissing`.
- `crates/semantic-merge/src/context.rs`(신규, merge 하위 module), `crates/semantic-merge/src/{merge,model}.rs`: conflict
  11·12와 호출.
- `crates/ops/src/diff.rs`: 새 객체 종류 이름.
- `crates/document/src/context/{key,provision}.rs`: scope·owner bytes 함수.
- `tests/integration/tests/context_semantic.rs`(신규).

## 구현 요약

- key·provision은 snapshot 객체, requirement·Slot 공개 목록은 Component·Slot 객체의 IX.12.1 예약 key(0xE705·0xE716) 속성입니다.
  Context가 없으면 snapshot이 바뀌지 않습니다.
- provision 삭제 대 provider 하위 트리의 같은 key 소비 추가: 삭제를 받아들이지 않고 provision 객체 conflict(11).
- key 타입 변경 대 provision 값 변경·추가: key 객체 변경 전체와 provision 값 변경을 받아들이지 않고 conflict(12).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff -p stellar-semantic-merge` | 통과(conformance corpus·determinism 포함) |
| `cargo test -p stellar-integration-tests --test context_semantic` | 통과(3개) |

시험:

- `context_objects_project_round_trip_and_diff_bytes_are_deterministic`: 역투영 round-trip, diff bytes 결정성, StableId 단위 변경.
- `provision_removal_versus_new_consumption_is_a_dedicated_conflict`: subtree 안 소비 conflict, subtree 밖·다른 key는 conflict 아님.
- `key_type_change_versus_provision_value_change_is_a_dedicated_conflict`: 두 객체 conflict, merged는 base 타입·값, 결과 bytes 결정성.

## 차이(작업 지시와 다른 점)

- 소비는 snapshot의 expression node 등 값 bytes에 key StableId가 새로 나타나는 것으로 판정합니다(기존 drilling conflict와 같은
  보수적 scan). 소비 객체가 scene tree에 놓이지 않았으면 같은 문서 안에서 conflict로 봅니다.

## 남은 위험 / 한계

- 제품 snapshot 조립(`stellar_ops::diff::snapshot_of`, `stellar_ops::compare`의 `document_builder`·HEAD 쪽 builder)은 레코드만
  넣고 `SnapshotBuilder::add_context`를 부르지 않습니다. 그래서 지금 `stellar diff`·compare·merge 결과에는 Context 변경이
  나타나지 않습니다. `add_context`는 Component·Slot 객체를 먼저 요구하므로 컴포넌트 library 투영 뒤에 불러야 합니다(소유 밖,
  후속).
- merge 결과를 문서에 쓰는 경로(ops·git 연결)에서 `context_from_snapshot`을 부르는 일은 하지 않았습니다(소유 밖, 후속).
- 이 lane은 conflict kind 11·12를 더했습니다. 다른 lane이 같은 번호를 쓰면 통합 때 맞춰야 합니다.

## 후속 보완 (2026-11-24, Context 제품 경로 연결)

- 결정: ADR-0114 결정 2.
- 제품 snapshot 조립이 Context를 투영합니다. 문서 범위(`stellar_ops::diff::snapshot_of`)는 key·provision을, 프로젝트 범위
  (compare의 `stellar diff`·HEAD 비교·project merge, `stellar apply` 결과 diff)는 컴포넌트 library 뒤에 `add_context`로
  requirement·Slot 공개 목록까지 넣습니다(library에 없는 대상의 항목은 넣지 않음).
- 병합 결과를 ContextSet으로 되돌립니다(`document_from_snapshot` = `document_records` + `context_from_snapshot`): 창
  reconciliation(`merge_document`), Git stage(`serialize_candidate`), 병합 해결 저장(`serialize_merged_generation`), Merge
  Composer 결과(`write_documents`)가 context-set chunk를 씁니다.
- 시험: `compare::tests::project_diff_and_merge_carry_the_document_context`(diff에 context-key·context-provision, 서로 다른 key
  추가 자동 병합, 같은 key 이름 충돌), `merge_apply::tests::document::context_changes_merge_and_conflict_like_other_objects`,
  `candidate::tests::staging_a_context_change_writes_the_context_chunk`(stage한 key가 index generation에 들어감).
- 남은 위험: 문서 범위 경로(창 reconciliation, Git stage, 병합 해결 저장, Composer)는 requirement·Slot 공개 목록이 있는 문서를
  무손실 검사로 거부합니다. 그 경로에 컴포넌트 library를 넣는 후속이 필요합니다. agent 외부 변경 분석과 attached 미리보기
  diff(`mcp-core`)는 레코드만 비교합니다(소유 밖).
- 검토 보완: 프로젝트 범위 투영(`add_contexts`, HEAD 단일 컴포넌트 비교)은 여러 문서가 같은 library 컴포넌트·Slot에
  requirement·공개 목록을 두면 같은 속성을 두 번 넣어 투영 전체가 실패했습니다. 그 Component·Slot 객체에 합집합 하나로
  붙이도록 고쳤습니다(`diff::tests::requirements_of_one_library_component_from_two_documents_project_as_one_union`).
