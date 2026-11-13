# 작업 보고서: VIII.3.4 AST 검증 오류의 GUI block·socket 매핑

- WBS: VIII.3.4
- 작업명: AST validation 오류를 GUI block ID와 socket ID로 매핑
- 상태: 완료 (`[x]`)
- 선행: VIII.3.1(`[x]`), VIII.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/gui_diagnostics.rs`(신규) — `GuiDiagnostic`, `DiagnosticSource`, `map_ast_error`,
  `map_type_error`, `map_type_errors`, `diagnose`, 매핑 표 문서, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.3.4.md`(신규), 계획서(VIII.3.4 `[ ]`→`[x]`).

## 구현 요약

- **구조 오류**: 중복 ID·root 없음·도달 불가 → 그 node block(socket 없음); dangling child·중복/불필요/누락 slot → 부모
  block과 그 slot의 socket; cycle `[a, b, …, a]` → block `a`와 `a`에서 `b`로 가는 slot의 socket.
- **type 오류**: 위반 node block과 위반 피연산자 slot의 socket(VIII.2.4 `TypeError::slot`, pattern 위반은 그 arm slot).
  피연산자 위치가 없는 오류(미지 참조 등)는 socket 없음.
- `diagnose(root, nodes, refs)`는 GUI 편집 중 node 목록을 검증해 구조 오류가 있으면 그 하나, 없으면 type 오류 전부를
  (type checker 순서대로) 돌려줍니다. block ID = node StableId, socket ID = (block, child slot)으로 VIII.3.1 projection과 같습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (87 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **오류 위치 fixture 정확**(`error_location_fixtures_are_exact`): 구조 7건(중복 ID, root 없음, dangling child→Rhs, 누락 slot→Rhs, 불필요 slot→Then, 도달 불가, cycle→`[3,4,3]`의 block 3·Operand)과 type 5건(Length+Color→Rhs, 중첩 if 조건→Condition, clamp 인자 2, match pattern 1→Arm(1), 미지 참조→socket 없음)의 출처·block·socket이 기대와 일치 | 통과 |
| type 오류 socket이 graph view의 실제 입력 socket(`type_error_sockets_exist_in_graph_view`) | 통과 |
| 다중 오류 순서 매핑·유효 AST 진단 없음·표시 문자열(`multiple_type_errors_map_in_order_and_valid_ast_has_none`) | 통과 |

## 차이(작업 지시와 다른 점)

- 구조 오류는 `ExpressionAst::new`가 첫 오류에서 멈추므로 한 번에 하나만 보고합니다. type 오류는 모두 보고합니다.

## 남은 위험 / 한계

- 메시지는 진단용 한국어 문자열입니다. 앱 UI 지역화 문자열과 오류 code 체계 연결은 Phase XII UI 작업에서 필요합니다.
