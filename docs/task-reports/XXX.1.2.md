# 작업 보고서: XXX.1.2 State·Pulse socket과 속성 포트 타입

- WBS: XXX.1.2
- 작업명: 프로토타입 socket 종류 State·Pulse, 직접 연결 거부와 변환 node 경유 허용, CSS 값 문법별 포트 타입 집합.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 2, ADR-0025 결정 1.2·2.2
- 선행: VIII.3.2, IX.5.2, IX.11.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/prototype/{mod,socket}.rs`(신규), `port_golden.txt`(신규): socket 종류, 포트 타입, 연결 검사, 전체 포트 golden.
- `crates/document/src/lib.rs`: `prototype` module 등록.
- `docs/adr/ADR-0094-prototype-motion-core.md`(신규).

## 구현 요약

- `property_port(대상 종류, key)`가 registry·layout longhand 문법을 `ExpressionType` 집합으로 바꿉니다. 합집합 문법은 keyword Enum + Length + Percentage처럼 구성 타입별 집합입니다(ADR-0094 결정 2).
- payload 크기 key(`width` 0x0108)는 layout longhand `width` 문법을 씁니다. keyword Enum 타입 번호는 `0xC000_0000 | longhand 번호`입니다.
- 대응이 없는 문법(`<track-size>`, `<transform-list>`, grid line, `aspect-ratio`, 값 문법 없음)과 묶음 key, 편집기 전용 필드는 포트로 노출하지 않고 진단 코드를 냅니다.
- `check_state_connection`은 식의 정적 타입만 보고 그 타입을 저장 variant로 돌려줍니다. `check_socket_kinds`는 State·Pulse 직접 연결을 거부합니다.
- 그래프 편집 plan(XXX.2.3 commit)이 이 검사를 써서 종류가 다른 연결을 거부하고, 토글·카운터 변환 node 경유 연결을 허용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo xtask validate-adr --base edfea9db` | 통과. |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `prototype::socket::tests::port_types_match_golden`: 모든 node 종류·modifier의 포트 golden.
- `width_accepts_length_and_percentage_but_not_color`, `unmapped_grammar_is_diagnosed`, `state_and_pulse_do_not_connect_directly`, `union_keywords_parse_with_layout_grammar`.
- `crates/prototype/tests/graph_plan.rs::socket_kinds_and_port_types_are_enforced`: plan 수준의 직접 연결 거부·변환 node 경유·width Length·Percentage 허용·Color 거부.
- `crates/prototype/tests/graph_plan.rs::connection_check_does_not_depend_on_stored_value_variant`: width binding을 Length로 저장한 문서와 Percentage로 바꿔 저장한 문서에서 같은 연결 plan(Length·Percentage 허용, Color 거부) 결과가 같음.

## 차이(작업 지시와 다른 점)

- "expression socket 연결 가능 API 확장"은 `stellar-expressions::sockets`를 고치지 않고, 그 위의 속성 포트 검사(`check_state_connection`)를 document에 두었습니다. expression 내부 socket 검사는 기존 `can_connect`를 그대로 씁니다.

## 남은 위험 / 한계

- `calc()` 길이·백분율 혼합(`<length-percentage>`)은 속성 모델 O3 결정 전이라 받지 않습니다.
