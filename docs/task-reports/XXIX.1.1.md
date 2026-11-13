# 작업 보고서: XXIX.1.1 crate graph 검사기 확장

- WBS: XXIX.1.1
- 작업명: crate graph 검사기가 target·build·dev 의존 표와 `tools/stellar-*` manifest를 읽고 ADR-0005 §8 전이 도달 금지 규칙을 적용
- 상태: 부분 완료 (`[~]`) — 검사기 구현과 검증은 끝났으나, 작업 지시의 "crate 의존 그래프 문서도 함께 갱신" 부분(`docs/architecture/crate-dependency-graph.md`)은 이 lane의 수정 허용 경로 밖이라 반영하지 못했습니다. 반영할 문안은 아래 "남은 위험 / 한계"에 있습니다.
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust xtask, 로컬 검증).

## 변경 파일 목록

- `tools/xtask/src/crate_graph/manifest.rs` (신규) — 제한 TOML manifest 해석기와 workspace scan.
- `tools/xtask/src/crate_graph/reach.rs` (신규) — 모든 표의 직접 edge 검사, ADR-0005 §8 전이 도달 금지 규칙(`FORBIDDEN_REACH`)과 반복 BFS.
- `tools/xtask/src/crate_graph.rs` (수정) — 모듈 문서, 하위 모듈 선언, `check_workspace_graph` 재노출, `tool_edges` test helper를 새 해석기로 교체. `allowed_edges()` 표는 바꾸지 않았습니다.
- `tools/xtask/src/commands.rs` (수정) — `validate-crate-graph`가 새 scan·검사를 사용, 이전 `[dependencies]` 전용 문자열 분할 함수 3개 삭제, 실제 workspace 종료 코드 test 추가.
- `docs/task-reports/XXIX.1.1.md`(신규), 계획서(XXIX.1.1 `[ ]`→`[~]`).

## 구현 요약

- **manifest 해석(`manifest.rs`)**: 문자 단위 lexer가 basic·literal·여러 줄 문자열, 주석, 배열·inline table 괄호 깊이를 추적해 문장 단위로 나눈 뒤, 표 머리글 경로 + dotted key 경로로 위치를 판정합니다.
  - 읽는 표: `[dependencies]`, `[build-dependencies]`, `[dev-dependencies]`(밑줄 표기 포함), `[target.<spec>.*]`, `[dependencies.<name>]` 하위 표, `<name>.path = ...` dotted key.
  - `package = "..."` 이름 변경과 루트 `[workspace.dependencies]`를 상속하는 `workspace = true`를 실제 package 이름으로 해석합니다.
  - `[[bin]]` 같은 array table, `[features]`, 문자열·주석 안의 `stellar-*` 텍스트는 의존으로 세지 않습니다.
  - 닫히지 않은 문자열·괄호, 머리글 뒤 잔여 내용, 잘못된 key, 문자열이 아닌 `name`·`package`는 오류로 거부합니다(fail closed). manifest당 256 KiB 상한, 재귀 없음.
  - 검사 대상: `crates/*/Cargo.toml`(디렉터리 필수)과 `tools/stellar-*/Cargo.toml`. `tools/xtask`와 `benchmarks/fixtures`는 ADR-0019대로 제외합니다. package 이름 누락·중복은 오류입니다.
- **정책 검사(`reach.rs`)**:
  - 검사 대상 package가 정책 표에 없으면 의존이 없어도 `unknown-crate`입니다(이전에는 edge가 있을 때만 보고).
  - 모든 표의 내부 edge를 `allowed_edges()`로 검사하고, 위반 메시지에 선언된 표를 적습니다(예: `([target.'cfg(unix)'.dependencies])`).
  - 전이 도달: ADR-0005 §8 "전이적으로도 도달 금지" 열을 `FORBIDDEN_REACH`로 인코딩했습니다(ops, cli, local-ipc, agent-protocol, mcp, agent-host). 아직 없는 crate의 규칙은 그 crate가 생기는 순간부터 적용됩니다. ffi는 ADR대로 inventory로 보완하므로 규칙이 없습니다.
  - 도달 집합은 crate 자신의 모든 표(dev 포함) 의존에서 시작하고, 그 뒤로는 normal·build 의존(target 한정 포함)만 따라갑니다. 의존 crate의 dev 의존은 빌드되지 않으므로 따라가지 않습니다. 방문 집합을 쓰는 반복 BFS로 순환에서도 끝나며, 위반에는 최단 경로를 적습니다.
  - 정책 자체 검사 test: 금지 대상이 허용 직접 edge나 허용 정책 graph의 전이 도달에 들어 있지 않음, 허용 정책 graph가 비순환임.
- 기존 `check_crate_graph(&[(String, String)])` API와 그 test는 그대로 두었습니다(다른 lane이 `allowed_edges()`에 항목을 추가해도 충돌하지 않도록 정책 표는 건드리지 않음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask` | lib 147 passed, 0 failed(통합 test 2+2 passed), 종료 코드 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0, 종료 코드 0 |
| `cargo fmt -p xtask -- --check` | 통과, 종료 코드 0 |
| `cargo xtask validate-crate-graph` (현재 workspace) | 종료 코드 0 |
| 기준 커밋 `78a574a` xtask를 scratch 복사본에서 빌드해 위반 fixture(`crates/document`에 `[target.'cfg(unix)'.dependencies] stellar-ffi`, `tools/stellar-cli`에 `[dev-dependencies] stellar-runtime`) 대상 실행 | 이전 검사기 종료 코드 0(두 위반 모두 놓침) |
| 같은 scratch 복사본에 새 `tools/xtask/src`를 넣어 실행 | 종료 코드 65, 두 위반을 표 표기와 함께 보고 |

| 검사 | 결과 |
|---|---|
| target·build·dev 표 위반 fixture 거부(`target_build_and_dev_table_violations_are_rejected`) | 통과 |
| 전이 위반 fixture 거부와 경로 보고(`transitive_reach_violation_is_rejected_with_path`, `reach_ban_applies_even_when_every_direct_edge_is_allowed`) | 통과 |
| dev 의존 전이 규칙(`dev_dependencies_count_for_the_owner_but_do_not_propagate`), 순환 종료(`reach_search_terminates_on_cycles`) | 통과 |
| manifest 해석: 모든 표·하위 표·dotted key·이름 변경·workspace 상속, 문자열·주석·array table 무시, 잘못된 구문 거부 | 통과 |
| 정책 자체 일관성(`bans_never_contradict_the_allowed_policy`, `allowed_policy_is_acyclic`) | 통과 |
| 현재 workspace 통과(`current_workspace_passes`, `validate_crate_graph_on_real_workspace`, `real_workspace_scan_covers_crates_and_tools`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- crate 의존 그래프 문서(`docs/architecture/crate-dependency-graph.md`) 갱신을 하지 않았습니다. 이 실행의 수정 허용 경로가 `tools/xtask/src/**`(task_report 제외)로 제한되어 있기 때문이며, 그래서 상태를 `[~]`로 두었습니다.
- 전이 도달 금지 규칙을 별도 설정 파일이 아니라 `crate_graph/reach.rs`의 상수로 두었습니다. 허용 edge 표(`allowed_edges()`)와 같은 방식이고, 다른 lane이 허용 표를 동시에 고쳐도 병합이 단순하도록 파일을 나눴습니다.

## 남은 위험 / 한계

- **후속(이 행을 `[x]`로 올리기 위한 남은 일)**: `docs/architecture/crate-dependency-graph.md`에 다음을 반영해야 합니다. 첫 문단의 "허용된 **직접** 의존 방향"을 "허용 직접 의존과 전이 도달 금지"로 바꾸고, 다음 두 절을 추가합니다.
  - "검사 범위": `crates/*`와 `tools/stellar-*` manifest의 `[dependencies]`·`[build-dependencies]`·`[dev-dependencies]`·`[target.<spec>.*]` 표(하위 표·dotted key·`package` 이름 변경·`workspace = true` 포함)를 모두 같은 허용 표로 검사합니다. `tools/xtask`와 `benchmarks/fixtures`는 제외합니다(ADR-0019). 정책 표에 없는 package는 의존이 없어도 거부합니다.
  - "전이 도달 금지(ADR-0005 §8)": ops → local-ipc·agent-protocol·mcp·agent-host·ffi, cli → local-ipc·agent-protocol·mcp·agent-host, local-ipc → document·commands·ops, agent-protocol → commands·document·ops·agent-host, mcp → agent-host·ffi, agent-host → document·commands·history·storage·ops. 도달은 자신의 모든 표에서 시작해 normal·build 의존만 따라갑니다. 정책 위치는 `tools/xtask/src/crate_graph/reach.rs`입니다.
  - 기준 문서의 허용 표에는 이미 `allowed_edges()`에 있는 `stellar-cli`, `stellar-mcp` 행이 빠져 있습니다(이 작업 전부터 있던 차이).
- manifest 해석기는 Cargo가 쓰는 TOML의 부분 집합입니다. 해석할 수 없는 구문은 거부하므로 위반을 통과시키지는 않지만, 유효한 TOML이라도 지원하지 않는 형태(예: 따옴표 key 안의 escape된 `"`와 `=`)는 오류가 날 수 있습니다.
- 내부 crate 판정은 package 이름 `stellar-` prefix 기준입니다. prefix가 다른 내부 crate가 생기면 정책과 함께 판정 기준을 바꿔야 합니다.
- `stellar-mcp` 전이 금지 규칙은 package 이름으로 적용됩니다. XIX.9.1에서 library(`crates/mcp`)와 실행 파일(`tools/stellar-mcp`)의 package 이름이 나뉘면 규칙 대상 이름도 함께 맞춰야 합니다.
