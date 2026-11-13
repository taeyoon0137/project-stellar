# 작업 보고서: VIII.5.5 토큰 command

- WBS: VIII.5.5
- 작업명: token 생성·삭제·rename·expression 변경 command, inbound 참조가 있는 삭제는 orphan plan 없이는 거부
- 상태: 완료 (`[x]`)
- 선행: VIII.5.1(`[x]`, 이 lane), Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/token_commands.rs`(신규) — `TokenCommand`(Create·Delete·Rename·SetExpression),
  `TokenOrphanPlan`, `TokenCommandError`와 테스트.
- `crates/bindings/src/registry.rs` — `TokenTombstone`(orphan 기록), `ReferenceIssue::Orphaned`, tombstone 조회·추가·제거,
  `TokenTypes`가 tombstone의 마지막 타입을 돌려줌, 동등성에 tombstone 포함.
- `crates/bindings/src/token_graph.rs` — graph가 tombstone 대상 간선을 포함(재생성 cycle 예측), `validate_for_save`가
  orphan 참조로는 저장을 막지 않음.
- `crates/bindings/src/token_cache.rs` — orphan 토큰을 읽으면 `TokenEvalError::Orphaned`.
- `crates/bindings/src/lib.rs` — `token_commands` 모듈 등록.
- `docs/task-reports/VIII.5.5.md`(신규), 계획서(VIII.5.5 `[ ]`→`[x]`).

(프로덕션 파일 5개 — 규칙 1.2.13 한도 이내)

## 구현 요약

- **command**: 닫힌 enum. `validate(registry, others)`는 registry를 바꾸지 않고, `apply`는 검증 후 적용하며 inverse
  command를 돌려줍니다. 실패 시 registry 불변.
  - Create ↔ Delete, Rename ↔ Rename(이전 이름), SetExpression ↔ SetExpression(이전 표현식).
- **새 표현식 검사**(Create·SetExpression): 자기 참조·cycle(VIII.5.3 `cycle_if_references`), 참조 토큰 존재·scope
  가시성(VIII.5.2), 삭제된 토큰 참조 금지, default expression type(결과 = `Required(token type)`).
- **ref policy(삭제)**: inbound 참조가 있으면 orphan plan 필수, plan의 참조 목록은 현재 inbound 참조와 정확히
  같아야 합니다(오래된 plan 거부). plan 삭제는 참조 표현식을 바꾸지 않고 `TokenTombstone`(ID·scope·이름·마지막
  타입)을 남깁니다(§14.4 자동 삭제 금지). 참조하는 토큰은 계속 마지막 타입으로 type 검사되며, 평가 시 `Orphaned`
  오류, 저장 검증은 통과합니다. 같은 ID를 같은 scope·이름·타입으로 다시 만들면(undo) 기록이 사라지고 참조가 다시
  연결됩니다.
- **읽기 전용 scope**: 라이브러리 토큰 생성·삭제·rename·expression 변경은 `ReadOnlyScope`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (19 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **apply/inverse**(`create_rename_and_set_expression_round_trip`): Create·Rename·SetExpression 각각 forward → inverse = 원래 상태, inverse의 inverse = forward 상태 | 통과 |
| **ref policy**(`delete_with_inbound_references_requires_exact_orphan_plan`): plan 없는 삭제 `InboundReferences`, 다른 plan `StaleOrphanPlan`, 정확한 plan 삭제 후 참조 ID 보존·tombstone·`Orphaned` issue·저장 검증 통과·평가 `Orphaned`, 삭제 토큰 신규 참조 거부, 다른 이름 재생성 거부, 참조 없는 삭제는 plan 불필요, 모두 inverse 왕복 | 통과 |
| 거부 시 registry 불변: type mismatch, cycle(`[base, double, base]`), 자기 참조, 보이지 않는 scope, dangling, 이름·ID 중복, 라이브러리 읽기 전용 3종, 없는 scope(`invalid_commands_are_rejected_without_changes`) | 통과 |
| command 5개 순차 적용 → 전체 undo = 초기 상태, 전체 redo = 최종 상태(`command_sequence_undo_restores_initial_state`) | 통과 |

## 차이(작업 지시와 다른 점)

- **Phase VII 선행 완화**: 행의 선행 "Phase VII"(command 기반: CommandId·origin·base revision·transaction, VII.1.x)는
  base에서 모두 `[ ]`이고 `stellar-bindings`는 `stellar-commands`에 의존할 수 없습니다(허용 edge 밖). 그래서 VIII.3.3과
  같은 방식으로 토큰 전용 closed enum command(validate/apply/inverse)를 이 crate에 두었습니다. Phase VII가 구현되면 그
  command enum으로 옮기고 base revision·precondition을 붙이는 재작업이 필요합니다.
- binding이 토큰을 참조하는 inbound 참조는 binding store(VIII.6.x)가 아직 없어 이 command의 삭제 검사에 포함하지
  않았습니다. 토큰 간 참조만 검사합니다.
- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).

## 남은 위험 / 한계

- Create·SetExpression 검증이 매번 토큰 graph 전체를 만듭니다(O(토큰 수)). 대규모 registry에서는 증분 graph가 필요할
  수 있습니다.
- tombstone은 메모리 registry 상태이며 저장 형식은 아직 없습니다(토큰 저장 형식 정의 시 포함 필요).
