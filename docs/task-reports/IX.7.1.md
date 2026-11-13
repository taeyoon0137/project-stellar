# 작업 보고서: IX.7.1 ViewVariable

- WBS: IX.7.1
- 작업명: ViewVariable에 StableId, type, default value, allowed enum values 정의
- 상태: 완료 (`[x]`)
- 선행: VI.1.2(`[x]`), VIII.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/mod.rs`(신규) — View 변수·Variant 모듈.
- `crates/document/src/view_variant/variable.rs`(신규) — `ViewVariableId`, `VariableValue`(값·`ExplicitNull`),
  `ViewVariable`, `ViewVariableSet`(View 소유 변수 모음, codec), `ViewVariableError`, 테스트 fixture.
- `crates/document/src/lib.rs` — `pub mod view_variant;` 등록.
- `docs/task-reports/IX.7.1.md`(신규), 계획서(IX.7.1 `[ ]`→`[x]`).

## 구현 요약

- **변수**: StableId, 이름(Property와 같은 규칙, View 안에서 유일), 타입(`TypeSpec` — nullable 변수만 실제 null
  값 `ExplicitNull` 허용), default 값, 허용 enum 값.
- **enum 규칙**: enum 타입 변수는 허용 값 목록이 비어 있지 않고 모두 같은 enum 타입이며 중복이 없어야 합니다.
  default가 그 목록 밖이면 `DefaultNotAllowed`로 거부합니다. enum이 아닌 변수는 허용 목록을 가질 수 없습니다.
- **default 타입**: default가 선언 타입과 맞지 않거나(다른 enum 타입 포함), nullable이 아닌 변수에
  `ExplicitNull`이면 `DefaultTypeMismatch`.
- `ExplicitNull`(실제 null 값)은 Variant 조건의 "제한 없음"(`Any`, IX.7.2)과 다른 값으로 별도 tag(0x02)를 씁니다.
- **저장**: `ViewVariableSet`은 View StableId + 변수(id 오름차순, 허용 값 오름차순) canonical bytes이며 디코딩이
  생성 규칙을 다시 검사합니다. ViewDefinition(VI.1) 저장 형식은 바꾸지 않았습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 306 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 허용 목록 밖 default → `DefaultNotAllowed`, 다른 enum 타입·비nullable ExplicitNull default 거부, 빈·혼합·중복 허용 목록 거부, 비enum 변수의 허용 목록 거부(`invalid_enum_default_is_rejected`) | 통과(거부) |
| 변수 모음 round-trip byte-identical, 절단 거부, id·이름 중복 거부(`variable_set_round_trips_and_rejects_duplicates`) | 통과 |

## 차이(작업 지시와 다른 점)

- View 변수 모음을 ViewDefinition 저장 형식에 넣지 않고 View StableId로 묶은 별도 resource로 정의했습니다. 기존
  문서 chunk(VI.10.2) 형식과 golden hash를 바꾸지 않기 위해서입니다. 저장 chunk 연결은 project 계층의 몫입니다.

## 남은 위험 / 한계

- 테스트가 쓰는 enum 타입 ID(0x0D01·0x0D02)는 fixture 값입니다. 실제 enum 타입 정의(이름·variant 이름)는 token·
  enum registry 쪽 후속입니다.
