# 작업 보고서: IX.1.2 ComponentProperty

- WBS: IX.1.2
- 작업명: `ComponentProperty`(StableId, 이름, type, 필수 default expression, 공개 여부) 정의, default 없는 property 생성 불가
- 상태: 완료 (`[x]`)
- 선행: IX.1.1(`[x]`), VIII.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/Cargo.toml` — `stellar-expressions` path 의존 추가(crate 그래프 1c461c2가 허용한 edge).
- `Cargo.lock` — 위 의존 추가에 따른 `stellar-document` 의존 목록 1줄 자동 갱신.
- `crates/document/src/component/typed_expr.rs`(신규) — `CheckedExpression`(선언 타입에 대해 검사된
  표현식), `assignable`, `ExpressionCheckError`, `TypeSpec`·AST canonical bytes codec 도우미.
- `crates/document/src/component/property.rs`(신규) — `ComponentProperty`, `PropertyVisibility`,
  `ComponentPropertyError`, `validate_property_name`, `ComponentLibrary::add_property`/`validate_properties`.
- `crates/document/src/component/definition.rs` — `ComponentEntry`에 property record map,
  `ComponentDefinitionError::Property`, library encode/decode에 property record 추가, 기존 round-trip 테스트를
  record 필수 규칙에 맞게 보강.
- `crates/document/src/component/mod.rs` — 모듈 등록, 모듈 문서의 "표현식은 의존 대상 아님" 문구 갱신.
- `docs/task-reports/IX.1.2.md`(신규), 계획서(IX.1.2 `[ ]`→`[x]`).

## 구현 요약

- **필수 default**: `ComponentProperty::new(id, name, value_type, default, visibility)`는 default AST를
  필수 인자로 받습니다. 저장 형식에서도 default bytes가 비면(node 없는 AST) `Codec(EmptyAst)`로 거부해,
  어떤 경로로도 default 없는 property를 만들 수 없습니다.
- **type 검사**: `CheckedExpression`이 bytecode compiler(VIII.4.1)의 type 검사를 거쳐 root 추론 타입이 선언
  타입에 대입 가능한지 확인합니다(`T`→`Optional<T>` 허용, 반대 금지, `Enum`은 enum 타입 ID까지 비교).
  맞지 않으면 `DefaultTypeMismatch { expected, found }`입니다.
- **닫힌 default**: default는 참조 없는 표현식이어야 합니다(`DefaultNotClosed`). orphaned binding의 fallback이
  "대상 Property의 선언된 기본값"(기술 아키텍처 14.4)이므로, 기본값 자체가 끊길 수 있는 원천에 묶이면 안
  됩니다. 평가 오류(0으로 나누기 등)도 "유효한 기본값 필수" 규칙으로 거부하고, 평가 값은 저장하지 않는
  캐시(`default_value`)로 보관합니다.
- **이름**: 비어 있지 않음, 256 bytes 이하, 제어 문자 없음. 식별은 StableId만 사용하고, 한 컴포넌트 안
  이름 중복은 `add_property`가 거부합니다(IX.4.4 이름 충돌 제안의 전제).
- **library 저장**: entry마다 slot record 뒤에 property record(id 오름차순)를 기록합니다. 디코딩은 각 record를
  생성 규칙으로 재검증하고, 정의에 선언된 property마다 record가 있는지(`MissingRecord`) 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 254 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과(출력 없음) |

| 검사 | 결과 |
|---|---|
| String property에 Number default, 다른 enum 타입, 필수 타입에 null default 거부(`default_type_mismatch_is_rejected`) | 통과 |
| 참조 default 거부, 0으로 나누기 default 거부, 계산식 default 평가 캐시(`default_must_be_closed_and_evaluable`) | 통과 |
| 빈 default bytes·조작된 타입 불일치 record·미지 visibility tag 디코딩 거부, round-trip byte-identical(`property_round_trips_and_default_is_required_in_bytes`) | 통과 |
| 미선언·중복 record·중복 이름 거부, 선언 property record 누락 시 library 디코딩 거부(`library_registers_declared_properties_with_unique_names`) | 통과 |

## 차이(작업 지시와 다른 점)

- default를 닫힌 표현식으로 제한했습니다(위 "닫힌 default"). token·환경에 따라 달라지는 값은 property에 거는
  binding(VIII.6)으로 표현합니다.
- `Cargo.lock`은 허용 경로 밖이지만 의존 추가에 따른 자동 갱신 1줄뿐입니다.

## 남은 위험 / 한계

- property 생성·rename·default 변경·삭제 command(IX.1.3)는 선행 VIII.8.1 미완으로 이 lane에서 차단입니다.
  `renamed`/`with_default`/`with_visibility`는 검증된 사본을 만드는 데까지만 제공합니다.
- library 저장 형식에 property record가 추가됐습니다. 아직 project loader가 ComponentLibrary chunk를 쓰지
  않아 기존 저장 파일 영향은 없습니다.
