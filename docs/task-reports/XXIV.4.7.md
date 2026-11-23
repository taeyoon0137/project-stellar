# 작업 보고서: XXIV.4.7 `stellar`의 AI 무관성 inventory

- WBS: XXIV.4.7
- 작업명: `stellar`에 AI·MCP·모델·에이전트 관련 명령과 해당 crate로의 직접·전이 의존이 없음을 검사하는 inventory test 추가
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XXIV.4.4(`[~]`, generation metadata writer 종류 미기록)가 `[x]`가 아닙니다. 선행 XXIX.1.1은 `[x]`입니다. 이전 기록: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XXIX.1.1이 `[~]`(crate 의존 그래프 문서 절, 이 lane 소유 밖)라서 `[x]`로 올리지 않습니다. XXIV.4.3·XXIV.4.4는 ledger-verification 재판정·원장 상태대로입니다. 행 범위는 모두 구현했습니다.
- 선행: XXIV.4.3(ledger-verification 재판정 `[x]`, 원장 미반영), XXIV.4.4 `[x]`, XXIX.1.1 `[~]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust xtask, 로컬 검증).

## 변경 파일 목록

- `tools/xtask/src/crate_graph/ai_inventory.rs`(신규) — 명령 inventory·도달 검사 `check_cli_ai_inventory`, 낱말 판정 `ai_term`, 도움말 해석 `help_commands`, test 3개.
- `tools/xtask/src/crate_graph/reach.rs` — `reach_paths`(실제 graph의 도달 crate와 최단 경로, 기존 BFS 규칙 재사용).
- `tools/xtask/src/crate_graph.rs` — 모듈 등록과 문서.
- `tools/xtask/src/commands.rs` — `validate-crate-graph`가 이 inventory를 함께 검사.

## 구현 요약

- **명령 inventory**: `stellar --help` golden(`tools/stellar-cli/tests/fixtures/help.txt`)의 `명령:` 절 명령 이름을 읽습니다. 이 golden은 CLI test(`headless_help`)가 실제 도움말과 byte 비교하므로 실제 명령 목록과 같습니다.
- **전이 도달 금지**: `stellar-cli`에서 시작해 모든 표(dev 포함)의 직접 의존, 그 뒤 normal·build 의존을 따라가는 도달 집합(XXIX.1.1 `FORBIDDEN_REACH`와 같은 규칙)을 구하고, 도달 crate 이름에 AI 관련 낱말이 있으면 경로와 함께 거부합니다.
- **낱말**: `ai`·`mcp`·`agent`·`agents`·`model`·`models`·`llm`을 이름의 `-`·`_` 조각 단위로 비교합니다. 이미 있는 crate만 세는 ADR-0005 §8 금지 목록과 달리, 이름에 그 낱말이 들어간 crate가 새로 생겨도 잡습니다.
- `cargo xtask validate-crate-graph`가 기존 정책 검사와 함께 실행합니다(위반이면 65).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo xtask validate-crate-graph` | 종료 0 |
| `cargo test -p xtask` | lib 194 passed, 통합 3+2 passed |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| help 명령 inventory와 전이 도달 금지 검사 통과: 실제 도움말 명령에 AI 낱말 없음, `stellar-cli`는 `stellar-ops`에 도달하고 workspace의 AI 관련 crate(`stellar-mcp-core`, `stellar-mcp`, `stellar-agent-protocol`) 어디에도 도달하지 않음(`real_cli_has_no_ai_command_or_reach`) | 통과 |
| `mcp-serve`·`agent` 명령과 `stellar-cli -> stellar-ops -> stellar-model-runner` 전이 의존을 거부하고, 빌드되지 않는 dev 의존의 dev 의존은 세지 않음(`ai_command_and_transitive_ai_crate_fail`) | 통과 |
| 조각 단위 판정(`email`·`maintain`·`stellar-terminal`은 통과)(`term_matching_is_by_name_segment`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 검사 대상은 workspace 내부 crate입니다. 외부 crate(crates.io)의 AI 관련 의존은 `cargo xtask validate-dependencies`(승인 목록)가 막습니다.
- 명령 inventory는 도움말 golden을 근거로 합니다. 도움말에 없는 숨은 명령은 CLI 쪽 정책(도움말 golden과 명령 표 일치 test)에 기댑니다.
