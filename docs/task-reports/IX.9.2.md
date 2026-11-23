# 작업 보고서: IX.9.2 10단계 nested component property drilling fixture

## WBS

- WBS: IX.9.2
- 작업명: 10단계 nested component property drilling fixture
- 상태: 부분 완료 (`[~]`) — fixture와 출처 chain 완결성은 확인했지만, drilled 값을 계산해 value provenance 기록을 내는 제품 evaluator가
  없어 검증 열("최종 value provenance가 전체 chain 포함")을 provenance 기록으로 확인하지 못했습니다.
- 선행: IX.4.5(`[x]`)
- 결정: ADR-0119 결정 8

## 변경 파일

- `tests/integration/tests/library_nested_drilling.rs`(신규) — 10단계 nested component drilling fixture와 시험.
- `docs/task-reports/IX.9.2.md`(신규).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --test library_nested_drilling` | 1 passed |
| `cargo clippy -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(DEV-MAC 로컬 시험)

## 테스트

| 검사 | 결과 |
|---|---|
| C0 ⊃ … ⊃ C10 중첩(정의 tree마다 다음 컴포넌트 instance), C10 공개 `label`을 제품 `DrillingPlan::compute`로 노출 계획: 컴포넌트 11개, 새 property 10개, link 10개, 같은 입력 같은 계획, preview 컴포넌트 경로·instance 경로·이름·default | 통과 |
| `apply_drilling_plan` 한 transaction 적용: property 10개·link 10개 생성 | 통과 |
| 출처 chain: scene C0 instance override(노출 property) → 경계마다 제품 `DrillingSet::link_into`로 link(owner property → nested instance property)를 따라 C10 `label`까지 property 11개·경계 10개가 빠짐없이 이어지고, 컴포넌트 순서가 계획 경로와 같으며 orphan 없음, `DrillingSet::validate` 통과(`ten_level_drilling_chain_reaches_the_innermost_property`) | 통과 |

## 성능

- 해당 없음.

## 차이

- 계획서 검증 열의 "value provenance"는 바인딩 provenance(VIII.6.5, `BindingEvaluator::provenance`) 같은 값 출처 기록을 뜻합니다. drilling
  link는 바인딩 graph에 들어가지 않고, nested instance property 값을 link로 계산하는 제품 evaluator도 없습니다(document
  `property_cache`는 instance override·default만 봄). 시험 안에서 값 계산·provenance를 새로 만들면 이중 판정이므로, 제품 drilling 조회만으로
  출처 chain의 완결성을 확인했습니다.

## 남은 위험

- 후속(document·bindings 소유): drilling link를 따라 nested instance property 값을 계산하고 그 출처(scene override → 경계별 property chain)를
  provenance로 내는 제품 evaluator. 생기면 이 fixture에 최종 값·provenance 단계 비교를 더하고 `[x]`로 올립니다.
