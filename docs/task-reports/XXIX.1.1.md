# 작업 보고서: XXIX.1.1 crate graph 검사기 확장

- WBS: XXIX.1.1
- 작업명: crate graph 검사기가 target·build·dev 의존 표와 `tools/stellar-*` manifest를 읽고 ADR-0005 §8 전이 도달 금지 규칙을 적용
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 검토: `FORBIDDEN_REACH`에 ADR-0005 §8의 `stellar-runtime` → mcp-core 전이 도달 금지 규칙이 없습니다(아래 "검토 보완" 절). 문서 기록: 남은 문서 절을 2026-11-23 plan-revision이 `docs/architecture/crate-dependency-graph.md`에 반영했습니다(아래 "문서 반영" 절). 재판정 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 검사기 구현과 검증은 끝났으나, 작업 지시의 "crate 의존 그래프 문서도 함께 갱신" 부분(`docs/architecture/crate-dependency-graph.md`)은 이 lane의 수정 허용 경로 밖이라 반영하지 못했습니다. 반영할 문안은 아래 "남은 위험 / 한계"에 있습니다.
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
  - 읽는 표: `[dependencies]`, `[build-dependencies]`, `[dev-dependencies]`(밑줄 표기 포함), `[target.<spec>.*]`, `[dependencies.<name>]` 하위 표, `<name>.path = ...` dotted key. 의존 표나 그 상위 표를 inline table로 쓴 형태(`dev-dependencies = { .. }`, `[target.'cfg(unix)']` 아래 `dependencies = { .. }`, `[target]` 아래 `'cfg(unix)' = { .. }`, `target = { .. }`, `workspace = { dependencies = { .. } }`)도 펼쳐 읽습니다(리뷰 후속 수정).
  - `package = "..."` 이름 변경과 루트 `[workspace.dependencies]`를 상속하는 `workspace = true`를 실제 package 이름으로 해석합니다.
  - basic string escape는 Cargo 1.96의 TOML 해석기와 같게 풉니다. TOML 1.0의 `\b \t \n \f \r \" \\ \uXXXX \UXXXXXXXX`와 TOML 1.1의 `\e`·`\xHH`이고, 따옴표 key 안의 escape된 `"`와 `=`도 읽습니다. 그 밖의 escape는 거부합니다(리뷰 후속 수정).
  - `[[bin]]` 같은 array table, `[features]`, 문자열·주석 안의 `stellar-*` 텍스트는 의존으로 세지 않습니다.
  - 닫히지 않은 문자열·괄호, 머리글 뒤 잔여 내용, 값 하나 뒤에 다른 내용이 붙은 문장, 의존 표 위치에 표가 아닌 값, 잘못된 key, 문자열이 아닌 `name`·`package`·`path`는 오류로 거부합니다(fail closed). manifest당 256 KiB 상한, 재귀 없음.
  - 검사 대상: `crates/*/Cargo.toml`(디렉터리 필수)과 `tools/stellar-*/Cargo.toml`에 더해, Cargo workspace member 전체를 Cargo 규칙대로 모읍니다(리뷰 후속 수정). 루트 `[workspace].members`(`dir/*` glob만 펼치고 그 밖의 glob은 거부), 루트가 package면 루트 자신, member의 모든 의존 표(dev·build·target 한정·optional 포함)와 `workspace = true` 상속의 path 의존 중 workspace 루트 안에 있는 자동 member(재귀)이고, `[workspace].exclude`를 Cargo처럼 적용합니다. `stellar-`로 시작하지 않는 자동 member도 검사하므로 그런 crate를 거친 금지 edge는 정책에 없는 crate로 거부됩니다. `tools/xtask`와 `benchmarks/fixtures`는 ADR-0019대로, crate 사이 종단 시나리오 테스트 crate `tests/integration`(dev 의존만 있는 개발용 crate, 9B 통합에서 추가)도 graph 검사에서 제외하되, 그 path 의존으로 생기는 자동 member는 검사합니다. 루트 안 path 의존의 manifest가 없으면 `missing-path-dependency`, package 이름 누락·중복은 오류입니다. 검사 대상이 `cargo metadata --no-deps`의 member 목록(제외 대상 제외)과 같음을 실제 workspace와 fixture test로 확인합니다.
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
  - "검사 범위": `crates/*`와 `tools/stellar-*` manifest, 그리고 Cargo workspace member 전체(루트 `[workspace].members`, path 의존으로 생기는 자동 member, root package, `[workspace].exclude` 반영)의 `[dependencies]`·`[build-dependencies]`·`[dev-dependencies]`·`[target.<spec>.*]` 표(하위 표·dotted key·inline table 형태·`package` 이름 변경·`workspace = true` 포함)를 모두 같은 허용 표로 검사합니다. `tools/xtask`와 `benchmarks/fixtures`(ADR-0019), 종단 시나리오 테스트 crate `tests/integration`은 제외합니다. 정책 표에 없는 package는 `stellar-` 이름이 아니어도, 의존이 없어도 거부합니다.
  - "전이 도달 금지(ADR-0005 §8)": ops → local-ipc·agent-protocol·mcp·agent-host·ffi, cli → local-ipc·agent-protocol·mcp·agent-host, local-ipc → document·commands·ops, agent-protocol → commands·document·ops·agent-host, mcp → agent-host·ffi, agent-host → document·commands·history·storage·ops. 도달은 자신의 모든 표에서 시작해 normal·build 의존만 따라갑니다. 정책 위치는 `tools/xtask/src/crate_graph/reach.rs`입니다.
  - 기준 문서의 허용 표에는 이미 `allowed_edges()`에 있는 `stellar-cli`, `stellar-mcp` 행이 빠져 있습니다(이 작업 전부터 있던 차이).
- manifest 해석기는 Cargo가 쓰는 TOML의 부분 집합입니다. 해석할 수 없는 구문은 거부하므로 위반을 통과시키지는 않지만, Cargo가 받는 manifest라도 지원하지 않는 형태(예: `dir/*` 밖의 members glob)는 오류가 날 수 있습니다.
- 내부 edge 판정은 의존 이름이 `stellar-`로 시작하거나 workspace member 이름인 경우입니다. 외부 crate와 이름이 같은 workspace member가 생기면 그 외부 의존도 내부 edge로 보고 거부할 수 있습니다(통과 쪽으로는 틀리지 않음).
- workspace member 판정은 Cargo 규칙을 파일 기준으로 다시 구현한 것입니다(`cargo metadata`를 실행하지 않음). 경로는 글자 단위로 정리하고 symlink는 풀지 않으며, workspace 루트 밖에 있으면서 `package.workspace`로 이 workspace를 가리키는 package는 다루지 않습니다.
- `stellar-mcp` 전이 금지 규칙은 package 이름으로 적용됩니다. XIX.9.1에서 library(`crates/mcp`)와 실행 파일(`tools/stellar-mcp`)의 package 이름이 나뉘면 규칙 대상 이름도 함께 맞춰야 합니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 검사기 코드는 행 지시와 검증 열을 모두 만족합니다. 행 지시의 "crate 의존 그래프 문서도 함께 갱신"이 아직 반영되지 않아 `[~]`를 유지합니다. 문서는 이 lane 소유 경로 밖이라 고치지 않았고, 아래 문안을 통합 세션에 넘깁니다. 문서를 반영하면 코드 변경 없이 `[x]`이고, 그러면 XIX.9.1의 선행(XIX.1.3 `[x]`, XXIV.4.2 이번 `[x]` 판정, XXIX.1.1)이 모두 풀립니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `tools/xtask/src/crate_graph/manifest.rs`: `[dependencies]`·`[build-dependencies]`·`[dev-dependencies]`·`[target.<spec>.*]`·하위 표·dotted key·inline table·`package` 이름 변경·`workspace = true`를 읽고, 해석할 수 없는 구문은 거부합니다(fail closed).
- `tools/xtask/src/crate_graph/reach.rs` `FORBIDDEN_REACH`: mcp-core → agent-host·runtime·ffi, ops → local-ipc·agent-protocol·mcp-core·mcp·agent-host·ffi, cli → local-ipc·agent-protocol·mcp-core·mcp·agent-host, local-ipc → document·commands·ops, agent-protocol → commands·document·ops·agent-host, mcp → agent-host·ffi, agent-host → document·commands·history·storage·ops. 도달은 자기 모든 표에서 시작해 normal·build 의존만 따라가는 반복 BFS입니다.
- `docs/architecture/crate-dependency-graph.md` 첫 문단은 여전히 "허용된 **직접** 의존 방향"만 말하고, 검사 범위·전이 도달 금지 절이 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p xtask` | lib 188 passed, 통합 3+2 passed, 0 failed |
| `cargo xtask validate-crate-graph` | 종료 코드 0(현재 workspace 통과) |

### 남은 조건과 finding

- 통합 세션 반영 문안(`docs/architecture/crate-dependency-graph.md`): 첫 문단을 "허용 직접 의존과 전이 도달 금지"로 바꾸고 "검사 범위" 절(`crates/*`·`tools/stellar-*`와 Cargo workspace member 전체의 모든 의존 표, `tools/xtask`·`benchmarks/fixtures`·`tests/integration` 제외, 정책 표에 없는 package 거부)과 "전이 도달 금지(ADR-0005 §8)" 절(위 `FORBIDDEN_REACH` 목록, 정책 위치 `tools/xtask/src/crate_graph/reach.rs`)을 추가합니다. 이전 보고서의 문안에서 mcp-core 규칙(agent-host·runtime·ffi)이 생긴 것만 다릅니다.
- 결함은 없습니다. 이 행은 문서 한 절 때문에 `[~]`입니다.

## 문서 반영 (2026-11-23, plan-revision)

- 판정(문서 반영 당시): 완료 (`[x]`). 재판정의 남은 조건이었던 문서 절을 반영했고, 코드는 바꾸지 않았습니다. 이 판정은 아래 "검토 보완" 절로 `[~]`가 되었습니다.
- `docs/architecture/crate-dependency-graph.md`:
  - 첫 문단을 "허용 직접 의존과 전이 도달 금지 규칙"으로 바꾸고 두 정책 위치(`allowed_edges()`, `FORBIDDEN_REACH`)를 적었습니다.
  - "검사 범위" 절: `crates/*`·`tools/stellar-*`와 Cargo workspace member 전체, 모든 의존 표와 그 표기 형태, 제외 crate(`tools/xtask`, `benchmarks/fixtures`, `tests/integration`), 정책 표에 없는 package 거부.
  - "전이 도달 금지(ADR-0005 §8)" 절: 현재 `FORBIDDEN_REACH` 일곱 규칙(mcp-core 포함)과 도달 집합 계산 규칙.
  - 허용 표의 `stellar-ops` 행과 Mermaid에 `allowed_edges()`에 이미 있던 git-porcelain 간선(ADR-0080)을 더했습니다. 이 밖의 표·Mermaid 간선이 `allowed_edges()`와 같은지 스크립트로 대조했습니다.
- 이전 보고서가 적은 "`stellar-cli`, `stellar-mcp` 행 누락"은 지금 문서에 두 행이 있어 해소된 상태입니다.
- 실행한 검증: `cargo xtask validate-crate-graph` 종료 코드 0, `cargo xtask validate-task-report docs/task-reports/XXIX.1.1.md` 종료 코드 0.

## 검토 보완 (2026-11-23, plan-revision 검토)

- 판정: 부분 완료 (`[~]`).
- 근거: ADR-0005 §8 crate 배치 표의 "전이적으로도 도달 금지" 열에는 `stellar-runtime` 행(mcp-core)이 있습니다. `tools/xtask/src/crate_graph/reach.rs`의 `FORBIDDEN_REACH`에는 runtime 규칙이 없고, 주석과 crate 의존 그래프 문서는 이 상수를 그 열의 인코딩이라고 적습니다. 행 지시의 "지정한 crate 쌍 사이의 전이 도달 금지" 가운데 한 쌍이 검사되지 않으므로 남은 범위입니다. 앞의 재판정("결함은 없습니다")은 이 차이를 보지 못했습니다.
- 지금 상태: `cargo tree -p stellar-runtime -e normal,build,dev`에 `stellar-mcp-core`가 없어 현재 위반은 없습니다. runtime 허용 의존을 넓히는 XXIX.3.1 전에 막아야 하는 규칙입니다.
- `[x]` 조건: `FORBIDDEN_REACH`에 `stellar-runtime` → `stellar-mcp-core` 규칙과 위반 fixture를 더하고, 문서의 "전이 도달 금지" 절 표에 그 행을 넣습니다. 이 검토는 문서만 고칠 수 있어 코드는 바꾸지 않았습니다.
- 영향: 선행 XXIX.1.1이 `[x]`가 아니므로 XIX.9.1은 `[!]`로 되돌렸습니다(계획 1.2 규칙 4).
