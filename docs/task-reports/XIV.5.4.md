# 작업 보고서: XIV.5.4 승인된 선택으로 CandidateIndexSnapshot 생성

- WBS: XIV.5.4
- 작업명: caller가 dependencies 포함을 승인한 뒤 Index snapshot에 selected+required changes를 적용해 CandidateIndexSnapshot 생성
- 상태: 완료 (`[x]`)
- 선행: XIV.5.3(`[x]`, 이 lane), XIV.3.9(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/records.rs`(신규) — `document_records`(snapshot → `DocumentRecords` 역투영), `DocumentProjection`, `ProjectionError`/`ProjectionFailure`, `documents`.
- `crates/semantic-diff/src/stage.rs` — `CandidateIndexSnapshot`, `build_stage_candidate`, `build_candidate`(crate 내부, unstage 공용), `StageError` 확장(ApprovalRequired·UnexpectedApproval·Apply·CandidateInvalid·Projection·DocumentInvalid).
- `crates/semantic-diff/src/lib.rs` — `pub mod records;`.
- `docs/task-reports/XIV.5.4.md`(신규), 계획서(XIV.5.4 `[ ]`→`[x]`).

## 구현 요약

- **역투영**: `document_records(snapshot, document)`는 `SnapshotBuilder`의 문서 투영을 거꾸로 돌려 문서
  정의(페이지·View 순서는 목록 속성), scene node(Frame·Fragment·Shape 4종·Path·Text·Image), modifier stack
  (12종 payload, OrderKey 순)을 복원합니다. 값은 문서 crate 생성자(크기·radius·crop·polygon·border 등)를 다시
  거쳐 범위 밖 값은 `ProjectionError`로 거부합니다(고쳐 넣지 않음). scene ComponentInstance는 레코드가
  표현하지 못해 `instances`로 따로 돌려줍니다. 결과는 `sort_canonical` 순서입니다.
- **candidate**: `build_stage_candidate(index, working, request)`는 계획(XIV.5.3)의 필수 변경을
  `request.approved`가 모두 포함하는지 확인하고(아니면 `ApprovalRequired`, 필수가 아닌 승인은
  `UnexpectedApproval`), closure 변경을 Index에 적용합니다. 그 결과가 semantic 무결성 위반을 새로 만들면
  `CandidateInvalid`, 변경이 닿은 문서를 역투영해 문서 validator(VI.10.1)가 Index에 없던 위반을 내면
  `DocumentInvalid`입니다. 통과한 candidate는 snapshot, 적용 diff, 검증된 문서 레코드를 담습니다. 입력
  snapshot은 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 74 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **candidate가 document validator 통과**: 레코드 기반 fixture에서 새 rect 21 선택 → required 4개; 미승인·일부 승인 → `ApprovalRequired`, 필수 아닌 승인 → `UnexpectedApproval`; 승인 후 candidate에 페이지 4·View 5·frame 20·rect 21 포함, 미선택 modifier 60·rect 11 크기 미포함, 적용 변경 5개, 역투영 레코드가 `validate_document` 통과·페이지 2개; 입력 snapshot 불변(`approved_selection_builds_a_candidate_that_passes_the_document_validator`) | 통과 |
| semantic은 통과하지만 Shape 아래 자식(문서 validator parent 규칙 위반) → `DocumentInvalid(문서 1)`, unstage 요청 거부(`candidate_failing_the_document_validator_is_rejected`) | 통과 |
| random 20쌍: 모든 unstaged 변경 stage → candidate == WorkingTree, 필수 변경 0(`staging_every_change_reproduces_the_working_tree`) | 통과 |
| 역투영: HEAD fixture와 random 200 문서 `document_records(build(R)) == R`(canonical 정렬)·재build 동일(`projection_round_trips_records`), 없는 문서·없는 페이지·형식 오류·음수 크기 typed error(`missing_or_malformed_objects_are_typed_errors`) | 통과 |

## 차이(작업 지시와 다른 점)

- 문서 validator는 candidate에서 변경이 닿은 문서만 실행합니다(닿지 않은 문서는 Index와 동일). 기준 Index
  문서에 이미 있던 위반은 이 선택이 만든 것이 아니므로 제외합니다.
- 컴포넌트 정의 library·token·asset은 `DocumentRecords` 범위 밖이라 semantic validator(XIV.5.3)만 검사합니다.

## 남은 위험 / 한계

- scene ComponentInstance node는 문서 레코드가 표현하지 못해(문서 crate 한계) 문서 validator 대상이 아닙니다.
- candidate를 Git index에 쓰려면 파일 직렬화(XIV.5.5)와 index 기록(XIV.6)이 필요합니다.
