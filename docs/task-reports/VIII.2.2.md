# 작업 보고서: VIII.2.2 Literal·Reference·Unary·Binary·Call·If·Match AST node

- WBS: VIII.2.2
- 작업명: Literal, Reference, Unary, Binary, Call, If, Match AST node 구현
- 상태: 완료 (`[x]`)
- 선행: VIII.2.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/ast.rs` — `Literal`(값/null), `EnvironmentKey`, `ReferenceTarget`(token·property·
  component property·환경 입력), `NodePayload`, `ExprNode`의 payload 보관과 node별 생성자, `ExpressionAst::referenced_targets`,
  payload 기반 slot 검증(Call 인자 수 = signature arity, Match arm 수 = pattern 수), 테스트 추가.
- `crates/expressions/src/codec.rs`(신규) — node binary 형식 `ExprNode::to_bytes`/`from_bytes`, `CodecError`,
  `value_type_tag`, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.2.2.md`(신규), 계획서(VIII.2.2 `[ ]`→`[x]`).

## 구현 요약

- **7개 node**: `ExprNode::literal`/`null`(Optional\<T>의 값 없음), `reference`, `unary`, `binary`, `call`,
  `if_else`, `match_on`. kind는 payload에서 파생되고 child는 VIII.2.1의 typed slot으로 연결합니다.
  참조는 이름이 아니라 `TokenRef`·`PropertyPath`·Component Property `StableId`·환경 입력 numeric key로 가리킵니다.
- **구조 검증 강화**: Call은 `Argument(0..arity)`를 정확히, Match는 pattern 수만큼 `Arm(i)`(최소 1개)와
  `Scrutinee`·`Default`를 요구합니다. 타입 검사는 VIII.2.4 소관입니다.
- **node 직렬화**: `id[16] ++ op[3](VIII.1.5) ++ payload ++ child_count:u32 LE ++ (slot tag u8, slot index u16 LE,
  target[16])*`. 리터럴·pattern은 `TypedValue` 안정 bytes(III.3.5)를 `u32` LE 길이 blob으로 담고, null 리터럴
  타입은 TypedValue type tag(+ enum 타입 ID u32 LE)로 씁니다. 디코더는 미지 연산·tag·slot, 과대 길이(할당 전
  거부), 잉여 bytes, 비정렬 child, 재직렬화 결과가 다른 값 bytes(예: `-0.0`), 비-canonical PropertyPath를
  거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (50 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **각 node round-trip**(`each_node_kind_round_trips`): 7개 kind 전부를 덮는 20개 fixture(값 리터럴 4종, null 2종, 참조 4종(key/index PropertyPath 포함), 단항 2, 이항 2, 호출 2, if, match 2)가 decode(encode(n)) = n, 재직렬화 bytes 동일 | 통과 |
| layout golden(`layout_golden`), 값 타입 tag = TypedValue tag(`value_type_tags_match_typed_value_tags`) | 통과 |
| 모든 prefix 절단·잉여 byte 오류(`every_truncation_is_an_error`) | 통과 |
| 미지 리터럴·참조·타입·연산·slot tag, 역순 child, 과대 길이, `-0.0`, NaN 거부(`malformed_payloads_are_rejected`) | 통과 |
| 결정적 byte mutation 4,000건 panic 없음·성공 시 canonical(`byte_mutations_never_panic`) | 통과 |
| node별 생성자·kind·op·참조 목록(`constructors_build_each_node_kind`), Call arity·Match arm 수 구조 검증(`call_arity_and_match_arms_are_structural`) | 통과 |

## 차이(작업 지시와 다른 점)

- VIII.2.1 테스트의 node 생성 부분을 payload 생성자로 바꿨습니다(`NodeKind` 인자 → `NodePayload`). 검증
  단언(assert)은 바꾸지 않았고 모두 통과합니다.
- 환경 입력 참조는 numeric key(`EnvironmentKey`)만 담습니다. key registry와 기본값 정책은 VIII.7.1~VIII.7.2 소관입니다.

## 남은 위험 / 한계

- `PropertyPath` canonical bytes(III.2.2)는 정수를 big-endian으로 기록합니다. node 형식은 그 bytes를 blob으로
  그대로 감싸므로 그 부분만 LE 규칙의 예외입니다(core 형식 보존).
- 전체 AST의 canonical 직렬화(node 순서 = child slot 순서)는 VIII.2.5에서 구현합니다.
