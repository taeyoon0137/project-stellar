# 작업 보고서: VIII.1.5 닫힌 표현식 연산 집합

- WBS: VIII.1.5
- 작업명: 파일·네트워크·시간·난수·반복·재귀를 표현식 모델에 넣을 수 없게 closed AST 연산 집합 유지
- 상태: 완료 (`[x]`)
- 선행: VIII.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/ops.rs`(신규) — `NodeKind`(7개), `UnaryOp`(2개), `BinaryOp`(13개)·`BinaryOpClass`,
  `ExpressionOp`(kind + operator/함수 ID), 고정 3바이트 연산 직렬화 `encode`/`decode`, `ClosedOpError`,
  연산별 type rule dispatch(`UnaryOp::check`, `BinaryOp::check`), 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.1.5.md`(신규), 계획서(VIII.1.5 `[ ]`→`[x]`).

## 구현 요약

- **닫힌 집합**: node kind는 Literal·Reference·Unary·Binary·Call·If·Match 7개뿐이고, Call은
  `FunctionId` registry(VIII.1.4)의 순수 함수만 가리킵니다. 모든 enum은 `#[non_exhaustive]`가 아니며 문자열·
  사용자 정의 payload variant가 없어 파일·네트워크·Git·터미널·현재 시각·난수 연산, 반복 kind, 사용자 정의
  함수(재귀 수단)를 **표현할 수 없습니다**. AST 참조 cycle 금지는 VIII.2.3에서 구현합니다.
- **안정 tag**: node kind `0x01~0x07`, 단항 `0x01~0x02`, 이항 산술 `0x01~0x05`·비교 `0x10~0x15`·논리
  `0x20~0x21`, 함수 ID는 VIII.1.4 값. golden test로 고정했습니다.
- **연산 직렬화**: `kind u8 ++ operand u16 LE` 고정 3바이트(canonical primitive 규칙 R1·R2). 디코더는 미등록
  kind·operator·함수 ID, operand가 없어야 하는 kind(Literal·Reference·If·Match)의 0이 아닌 operand, 짧은
  입력·잉여 bytes를 `ClosedOpError`로 거부합니다. AST serializer(VIII.2.5)가 이 디코더를 그대로 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (35 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **serialized malicious unknown op 거부**(`serialized_malicious_unknown_ops_are_rejected`): 빈/짧은 입력, 잉여 byte, kind 0·가상 반복 kind `0x08`·`0xFF`, 가상 파일·시각/난수 함수 ID `0x0F00`·`0xFFFF`, 함수 ID 0, 미등록 단항·이항 tag, Literal·If·Reference에 숨긴 operand — 15건 모두 정확한 오류 | 통과 |
| 3바이트 공간 전수(256 × 65,536)에서 디코드 성공 집합 = 닫힌 연산 35개(`only_registered_encodings_decode`) | 통과 |
| 모든 연산 encode/decode round-trip·인코딩 유일(`every_op_round_trips`) | 통과 |
| 연산 이름에 금지 능력 단어(file·read·net·fetch·git·terminal·exec·eval·time·now·random·loop·while·for·recur·lambda 등) 없음, kind 7개 고정(`no_forbidden_capability_is_representable`) | 통과 |
| tag golden(`tags_are_stable_golden`), 연산별 type rule dispatch(`op_type_checks_dispatch_to_rule_tables`) | 통과 |

## 차이(작업 지시와 다른 점)

- 이 작업은 AST node(VIII.2.1~VIII.2.2)보다 먼저라서, "serialized op"를 node 연산의 고정 3바이트
  직렬화로 정의하고 그 디코더의 거부를 검증했습니다. 전체 AST bytes 수준의 unknown op 거부는 이 디코더를
  재사용하는 canonical AST decoder(VIII.2.5)에서 추가로 검증합니다.

## 남은 위험 / 한계

- 금지 단어 검사는 이름 기반 보조 검사입니다. 실질적 보장은 enum이 닫혀 있다는 구조와 전수 디코드 검사입니다.
- 새 연산·함수를 추가하면 golden과 전수 검사 기대 개수를 함께 갱신해야 합니다(의도적 변경 강제).
