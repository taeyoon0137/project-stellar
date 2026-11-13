# 작업 보고서: XIV.3.7 variant·binding·expression AST의 StableId node 단위 diff

- WBS: XIV.3.7
- 작업명: variant·binding·expression AST는 StableId node 단위로 diff하고 전체 blob 변경으로 축약하지 않음
- 상태: 완료 (`[x]`)
- 선행: VIII.2.1 `[x]`, IX.7.3 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/keys.rs` — `EXPR_PAYLOAD`·`EXPR_CHILDREN`(0xEAxx),
  `VARIANT_PROPERTY_PATCH`·`VARIANT_STRUCTURE_PATCH`·`VARIANT_SLOT_CHANGE`(0xEBxx).
- `crates/semantic-diff/src/build.rs` — `ExpressionNodeEntry`, `VariantEntry`,
  `add_expression_nodes`, `add_variant`.
- `docs/task-reports/XIV.3.7.md`(신규), 계획서(XIV.3.7 `[ ]`→`[x]`).

## 구현 요약

- **expression AST**: node마다 `ExpressionNode` 객체 하나(parent = AST owner)이고 속성은 payload
  canonical bytes와 typed child ref 목록(slot code + 대상 id, slot 순서) 두 개입니다. AST가
  DAG여도 공유 node는 객체 하나입니다. operator 하나 변경은 그 node의 payload 속성 변경 하나,
  child 교체는 그 node의 child ref 변경 하나입니다.
- **variant**: `VariantOverrideSet`을 항목 단위로 펼칩니다 — property patch는 대상
  `PropertyPath` canonical bytes hex segment마다 TypedValue, 구조 patch는 node id segment마다,
  slot change는 `인스턴스/slot` segment마다 그 항목 하나만 담은 set의 canonical 인코딩입니다.
- **binding**: binding의 expression은 binding을 owner로 하는 AST node 객체로 같은 경로를 탑니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 34 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 3-node AST에서 root operator 한 개 변경 → 변경 1개(그 node의 payload), child 순서 교체 → 1개(`ast_operator_change_is_one_node_change`) | 통과 |
| variant property patch 하나 값 변경 → 1개, 구조 patch 하나 Delete→Move → 1개(그 node segment)(`variant_patch_change_is_one_entry_change`) | 통과 |

## 차이(작업 지시와 다른 점)

- semantic-diff는 expressions crate에 의존할 수 없어(II.2.8 그래프) AST node를 caller가 옮겨
  담는 `ExpressionNodeEntry`(canonical payload bytes + child ref)로 받습니다. fixture의 payload
  bytes는 expressions codec 출력이 아니라 고정 테스트 bytes입니다. 실제 AST 변환 연결은 binding
  모델(VIII.6)·Status 통합에서 caller가 수행합니다.
- binding 모델(VIII.6.1)이 아직 없어 binding 자체의 target/scope 속성 diff는 대상이 없습니다.

## 남은 위험 / 한계

- node payload 내부(예: Match arm 값 목록)는 한 속성이므로 그 안의 값 하나 변경도 그 node의
  변경 하나입니다(node 단위 요구는 충족).
