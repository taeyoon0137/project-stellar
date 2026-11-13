# 작업 보고서: XIV.10.4 같은 속성 다른 값 conflict

- WBS: XIV.10.4
- 작업명: same property different value conflict를 생성하고 base/ours/theirs 값을 모두 보존
- 상태: 완료 (`[x]`)
- 선행: XIV.10.2 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/model.rs` — `PropertyValues`, `SemanticConflict::property_values`.
- `crates/semantic-merge/src/merge.rs` — conflict record 정확성 fixture.
- `docs/task-reports/XIV.10.4.md`(신규), 계획서(XIV.10.4 `[ ]`→`[x]`).

## 구현 요약

- **생성**: 양쪽이 같은 `StableId + PropertyPath`를 canonical 비교로 다른 값으로 바꾸면(한쪽
  속성 삭제 포함) `property-value` conflict 하나를 만듭니다(XIV.10.2 엔진). 대상은 그 속성
  경로이고 세 상태는 각 snapshot에서 읽은 값(`SideState::Value`, 없음은 `None`)입니다.
- **candidate**: conflict 경로는 base 값을 유지하고, 같은 객체의 다른 경로와 다른 객체 변경은
  계속 자동 병합합니다.
- **조회 API**: `property_values()`가 속성 conflict의 경로·base·ours·theirs 값을 돌려줍니다
  (해결 모델 XIV.12의 ChooseBase/Ours/Theirs 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 15 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 11.name(a/x/y)·12.name(b/삭제/z) conflict 2개, 세 값·대상·출처 정확; candidate의 conflict 경로는 base, 11.visible·10.name은 병합; bytes round-trip 후 동일(`different_values_conflict_preserves_all_three_values`) | 통과 |
| `property_values` 조회, 객체 대상은 `None`(`property_values_accessor`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 값 타입이 바뀐 경우의 세분(type-change-vs-value-change)은 XIV.10.5에서 분리합니다.
