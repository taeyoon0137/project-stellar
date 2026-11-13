# 작업 보고서: VIII.3.2 GUI socket 타입과 연결 가능 여부 API

- WBS: VIII.3.2
- 작업명: GUI에서 허용 가능한 child socket 타입과 연결 가능 여부 API 추가
- 상태: 완료 (`[x]`)
- 선행: VIII.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/sockets.rs`(신규) — `accepted_types`, `socket_accepts`, `can_connect`, `connect`,
  `ConnectError`, 내부 `rebuild_reachable`·`path_between`, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.3.2.md`(신규), 계획서(VIII.3.2 `[ ]`→`[x]`).

## 구현 요약

- **socket 허용 타입**: `accepted_types`는 socket block의 다른 socket에 연결된 값 타입을 고정한 채 후보 타입을 넣어
  VIII.2.4 `infer_node`로 국소 규칙을 확인합니다. 후보는 enum이 아닌 12개 값 타입과 AST에 등장한 enum 타입의
  `Required`/`Optional` 형태이며 결과는 정렬됩니다. `socket_accepts`는 특정 타입 하나를 판단합니다.
- **연결 가능 여부**: `can_connect(ast, refs, socket, source)`는 순서대로 없는 block·socket·source, cycle 생성(source에서
  block으로 닿는 경로 → `[block, …, block]` 경로 보고), source 타입 미확정, 국소 타입 불일치, 연결 후 **다른 node에 새
  type 오류**(결과 타입 변화가 상위로 전파되는 경우)를 `ConnectError`로 거부합니다.
- **적용**: `connect`는 socket의 child를 source로 바꾸고 root에서 닿지 않게 된 node를 제거한 새 AST를 만듭니다(구조 검증 포함).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (77 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **잘못된 socket 연결 거부**(`invalid_socket_connections_are_rejected`): `Length + Color`·if 조건에 Length → `TypeMismatch`(위반 slot 정확), 자기 자신·조상을 자손 socket에 연결 → `WouldCreateCycle`(경로 정확), 없는 socket·block·source 거부; 정상 공유 연결은 통과하고 적용 결과 type 오류 없음 | 통과 |
| 하류 type 파손 거부(`connection_that_breaks_downstream_types_is_rejected`): `len ÷ ?`에 Length는 국소적으로 허용되지만 상위 if branch 불일치로 `IntroducesErrors`(node = if, slot = Else) | 통과 |
| 허용 타입이 형제 socket을 반영(`accepted_types_reflect_sibling_sockets`): `Length + ?`→Length, if 조건→Boolean, if else→Length·Optional<Length>, `Length × ?`→Number, `? × Number`→Number·4 단위, clamp 인자→Angle | 통과 |
| 연결 후 도달 불가 node 제거(`connect_prunes_unreachable_nodes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 기존 오류가 있는 AST에서는 "연결 후 오류 node 집합이 연결 전의 부분집합"일 때만 허용합니다. 연쇄 억제(VIII.2.4) 때문에
  기존 오류를 고치는 연결이 상위 node의 새 오류를 드러내면 거부될 수 있습니다(보수적 판단).

## 남은 위험 / 한계

- 후보 enum 타입은 AST에 이미 등장한 enum만 포함합니다. 프로젝트 전체 enum registry와의 연결은 이후 UI 작업(Phase XII)에서 필요합니다.
- 매 호출마다 전체 type check를 수행합니다. 대형 표현식에서 GUI 반응성이 필요하면 캐시가 필요합니다.
