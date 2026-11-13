# 작업 보고서: VIII.2.3 AST cycle detector

- WBS: VIII.2.3
- 작업명: AST는 DAG 참조를 허용하되 cycle을 금지 — cycle detector 구현
- 상태: 완료 (`[x]`)
- 선행: VIII.2.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/cycle.rs`(신규) — `find_cycle`(반복 3색 DFS), 단위 테스트.
- `crates/expressions/src/ast.rs` — `AstError::Cycle { path }`, `ExpressionAst::new`의 마지막 단계로 cycle 거부, 모듈 문서.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.2.3.md`(신규), 계획서(VIII.2.3 `[ ]`→`[x]`).

## 구현 요약

- **검출**: root에서 시작하는 반복(비재귀) 3색 DFS입니다. 방문 중(Open) node로 되돌아가는 간선을 만나면
  stack에서 그 node부터의 경로를 잘라 `[a, …, a]`로 닫아 반환합니다. child를 slot 순서로 방문하므로 node
  삽입 순서와 무관하게 같은 경로를 보고합니다. 완료(Done) node 재방문은 DAG 공유이므로 허용합니다.
- **거부**: `ExpressionAst::new`가 도달성 검사 뒤 cycle을 검사해 `AstError::Cycle`로 거부합니다. 따라서
  검증된 `ExpressionAst`는 항상 비순환 DAG입니다.
- 각 node는 한 번만 Open/Done 처리되어 시간은 node·간선 수에 선형입니다(공유가 많은 DAG도 경로 수와 무관).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (55 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **direct cycle fixture 실패**(`direct_cycle_fixtures_fail`): 자기 자신 피연산자 `[1,1]`, Rhs 자기 참조 `[1,1]`, 상호 참조 `[1,2,1]` | 통과 |
| **indirect cycle fixture 실패**(`indirect_cycle_fixtures_fail`): root 아래 3단 cycle `[2,3,4,2]`, 공유 node 경유 root 복귀 `[1,5,1]` | 통과 |
| DAG 공유(diamond, 같은 child 양 slot)는 cycle 아님(`dag_sharing_is_not_a_cycle`) | 통과 |
| 깊이 100,000 chain 통과·끝에서 닫히는 cycle 검출(경로 길이 100,001), 64단 diamond chain 선형 처리(`deep_graphs_do_not_overflow_the_stack`) | 통과 |
| 삽입 순서와 무관한 결정적 경로(`detection_is_deterministic`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 표현식 사이의 cycle(토큰 A의 식이 토큰 B를, B가 A를 참조)은 이 detector 범위가 아닙니다. token dependency
  graph(VIII.5.3)와 binding cycle detector(VIII.6.3) 소관입니다.
