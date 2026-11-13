# 작업 보고서: VIII.5.1 토큰 모델

- WBS: VIII.5.1
- 작업명: `TokenSet`, `TokenDefinition`, `TokenName`, `TokenType`, default expression 정의(참조는 StableId)
- 상태: 완료 (`[x]`)
- 선행: VIII.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/Cargo.toml` — `stellar-expressions` 의존 추가(1c461c2에서 허용된 edge `bindings → expressions`).
- `crates/bindings/src/lib.rs` — crate 설명 갱신, `token` 모듈 등록.
- `crates/bindings/src/token.rs`(신규) — `TokenName`, `TokenType`, `TokenDefinition`, `TokenSet`, `TokenSetId`,
  `TokenTypes`·`TokenReferenceTypes`, `check_expression`, 오류 타입 3종과 테스트.
- `Cargo.lock` — 자동 갱신(`cargo test -p stellar-bindings` 실행 시 `stellar-bindings` 의존 목록에
  `stellar-expressions` 추가). 내부 path 의존이며 외부 crate 추가는 없습니다.
- `docs/task-reports/VIII.5.1.md`(신규), 계획서(VIII.5.1 `[ ]`→`[x]`).

## 구현 요약

- **식별과 참조**: 토큰은 `TokenRef`(StableId)로 식별합니다. 다른 표현식은 `ReferenceTarget::Token(TokenRef)`로
  가리키며 이름은 참조에 쓰지 않습니다. 따라서 rename은 이름 색인만 바꾸고 참조 표현식은 bytes까지 그대로입니다.
- **`TokenName`**: 1~256 bytes UTF-8, 제어 문자·앞뒤 공백 거부. 한 `TokenSet` 안에서 유일합니다(insert·rename이
  중복을 거부). scope 사이의 같은 이름 규칙은 VIII.5.2 소관입니다.
- **`TokenType`**: 비-nullable `ExpressionType`. 참조하는 표현식에는 `Required(T)`로 보입니다. default expression의
  결과 타입은 정확히 `Required(T)`여야 하며(`Optional` 결과·다른 타입 거부), 표현식 내부 type 오류는 VIII.2.4의
  `TypeError`(오류 node StableId 포함)를 그대로 전달합니다.
- **`TokenSet`**: ID 색인(BTreeMap)과 이름 색인을 함께 유지합니다. insert/remove/get/by_name/rename/
  replace_expression/referrers. 검증이 필요한 변경(표현식 교체)은 command(VIII.5.5)가 검사합니다.
- **type 검사 입력**: `TokenReferenceTypes`가 토큰 참조는 토큰 타입으로, 나머지 참조는 호출자 제공자로 해석합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (3 unit passed, 0 failed, doctest 0) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **rename 후 참조 유지**(`references_survive_rename`): `spacing.double = spacing.base × 2`에서 base를 rename해도 double의 canonical AST bytes 동일, 참조 ID·referrers 동일, 새 이름 조회 성공·옛 이름 조회 실패, 평가값 16px 동일, type 검사 통과 | 통과 |
| 이름 검증·set 내 이름/ID 유일성·rename 충돌·제거 시 이름 색인 정리(`names_are_validated_and_unique_within_a_set`) | 통과 |
| default expression 결과 타입 불일치·nullable 결과·모르는 토큰 참조(오류 node 정확) 거부(`default_expression_type_must_match_token_type`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다. 구현 전 실패 상태는 "타입이 없어 컴파일 불가"이며 별도 실행
  기록은 남기지 않았습니다.
- 행에 직렬화 형식 지시가 없어 토큰 저장 형식은 정의하지 않았습니다(규칙 1.2.3).

## 남은 위험 / 한계

- 토큰 default expression이 참조할 수 있는 대상 종류를 모델에서 제한하지 않았습니다. scope 가시성은 VIII.5.2,
  cycle은 VIII.5.3, 종류별 dependency는 VIII.6.2에서 다룹니다.
- 토큰 저장 형식·library export(XV.5.1)는 후속 작업입니다.
