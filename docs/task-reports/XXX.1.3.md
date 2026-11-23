# 작업 보고서: XXX.1.3 프로토타입 node 종류 registry

- WBS: XXX.1.3
- 작업명: 속성 출력·expression block·상태 변수·이벤트 원천·Transition·semantic emit·Context 읽기·MotionSpec·변환 node만 있는 닫힌 registry.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.3이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0094 결정 3
- 선행: XXX.1.1, XXX.1.2, XXX.7.3, IX.11.2, IX.11.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/prototype/node_kind.rs`(신규).
- `crates/document/src/prototype/mod.rs`: module 등록.

## 구현 요약

- `PrototypeNodeKind`(tag 0x01\~0x09)와 `ConversionKind`(0x01\~0x03)가 닫힌 enum입니다. 실행 코드·파일·네트워크 node는 variant가 없어 표현할 수 없습니다.
- 종류마다 socket 형태(이름·종류·방향·반복)를 고정합니다. 모르는 tag와 변환이 아닌 종류의 변환 tag는 `from_tag`가 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `registry_is_complete_and_round_trips`, `unknown_serialized_kind_is_rejected`, `only_conversion_nodes_change_socket_kind`.

## 차이(작업 지시와 다른 점)

- 선행 XXX.7.3(Context)은 아직 없습니다. Context 읽기 종류는 registry에 있지만 projection은 Context 원본이 생긴 뒤 만듭니다(ADR-0094 결정 3).

## 남은 위험 / 한계

- ContextRead node의 projection·편집은 XXX.7.4 이후에 연결해야 합니다.
