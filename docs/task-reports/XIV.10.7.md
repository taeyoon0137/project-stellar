# 작업 보고서: XIV.10.7 전용 conflict 분류

- WBS: XIV.10.7
- 작업명: component property 삭제 대 instance override 추가, token 타입 변경 대 값 변경, drilling chain break를 전용 conflict로 분류
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증과 선행 판정은 `[x]` 조건을 충족합니다. 보고 당시 기록: 완료 (`[x]`)
- 선행: IX.1.3, IX.4.6, VIII.5.5
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/model.rs` — `ConflictKind` code 7·8·9와 이름.
- `crates/semantic-merge/src/merge.rs` — `token_type_conflicts`, `interface_use_conflicts`, `uses_member`·`property_use`.
- `crates/semantic-diff/src/validate.rs` — `override_member` 공개(같은 override 경로 해석 재사용).
- `crates/semantic-merge/tests/common/mod.rs`, `crates/semantic-merge/tests/dedicated_conflicts.rs`(신규 fixture corpus).

## 구현 요약

- **token-type-vs-value**: 한쪽이 `TOKEN_TYPE`, 다른 쪽이 `TOKEN_VALUE`·`TOKEN_EXPRESSION`을 바꿨고 두 결과 token의
  (타입, 값, expression)이 다르면 token 객체 conflict입니다. candidate는 base token을 유지하고 세 token 상태를 모두 담습니다.
- **component-property-removed-vs-override / drilling-chain-break**: 한쪽이 `COMPONENT_PROPERTIES`에서 뺀 Property를 그
  Property를 목록에 둔 다른 쪽이 새로 사용하면, 목록 삭제를 보류하고 목록 속성 conflict로 바꿉니다. 사용이 scene 인스턴스
  override면 7, 정의 tree 안 nested 인스턴스 override이거나 값 bytes가 그 Property id를 담으면 9입니다. 사용 변경은 받아들이므로
  candidate는 interface 규칙을 지킵니다. 양쪽이 모두 목록을 바꿔 생긴 목록 conflict는 종류만 바꿉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | lib 28 passed, dedicated_conflicts 3 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| corpus 11개(전용 세 종류 + 전용이 아닌 이웃 경우), 각 사례 좌우 교환까지 종류·대상 일치(`dedicated_conflict_corpus_passes`) | 통과 |
| token conflict의 candidate = base token, 세 side 객체 기록 | 통과 |
| Property 삭제 보류·override 수용 후 새 snapshot 위반 0 | 통과 |

## 차이(작업 지시와 다른 점)

- drilling binding(IX.4)은 영속 레코드·semantic snapshot에 아직 없으므로, snapshot에 보이는 link(정의 tree nested 인스턴스
  override, 값이 Property id를 담는 binding)만 drilling chain으로 판정합니다(ADR-0081 결정 1).

## 남은 위험 / 한계

- Slot 삭제 대 Slot override는 전용 종류가 아니라 결과 검증(XIV.10.8)의 `invalid-result`로 잡습니다.
- token conflict는 token 객체 전체를 보류하므로 같은 token의 독립 이름 변경도 함께 보류됩니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.10.7)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `6c416359` Feat: 전용 semantic conflict 세 종류 분류
