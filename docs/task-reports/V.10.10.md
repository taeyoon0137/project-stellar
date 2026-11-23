# 작업 보고서: V.10.10 node → 파일·줄 범위

## WBS

- WBS: V.10.10
- 등록: 2026-11-23 plan-revision이 임시 번호 TS-MAP를 이 ID로 계획서에 등록하고 보고서를 `docs/task-reports/pending-wbs/TS-MAP.md`에서 옮김
- 작업명: node StableId → 텍스트 원본 파일·줄 범위 core API와 CLI `stellar query --source-span`
- 상태: 완료 (`[x]`) — 2026-11-24 `text-source-everywhere` lane: core API·CLI(앞선 lane)에 더해 MCP `document.source_span`의 성공 경로를 텍스트 원본 fixture로 확인했고, span이 가리키는 줄의 값이 원본에서 읽은 조회 값과 같습니다. 선행 V.10.4 판정은 통합 때 `generate-ledger`로 정합니다. 처음 보고 당시: 완료 (`[x]`), 2026-11-23 plan-revision 판정: 부분 완료(`Task(V.10.10)` 커밋 없음, 선행 `[~]`).
- 선행: V.10.4 `[~]`
- 구현 커밋: `ae935137`(span table), `199fee64`(ops·CLI). 결정: [ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md) 결정 6.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/project/src/document_io/source_v4/span.rs`(신규): `SourceSpan`, `SpanTable`, `node_spans`.
- `crates/project/src/document_io/source_v4/project.rs`: `encode`가 span table을 돌려주고 `open_dir`가 디스크 bytes 기준 표를 돌려줍니다.
- `crates/ops/src/text_source.rs`: `source_span`.
- `tools/stellar-cli/src/text_source.rs`: `query_source_span`. `tools/stellar-cli/src/cli.rs`: `query ... --source-span` 경로 등록.
- `tools/stellar-cli/tests/text_project.rs`(신규).

## 구현 요약

- span은 codec이 쓴 텍스트(또는 디스크 bytes)의 최상위 `nodes` 목록 항목 줄 범위(1부터, 끝 포함)입니다. canonical writer가 문자열을 한 줄로 escape하므로 줄 스캔이 정확합니다. `$ref` 항목은 node가 아니라 건너뜁니다.
- `stellar query <design 폴더> --source-span <node>`는 `{node, file, start_line, end_line}`을 돌려주고, 없는 node는 66(`source-span-not-found`)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib span` | 통과 |
| `cargo test -p stellar-project --test text_project` | 디스크 `open_dir` span = `encode` span |
| `cargo test -p stellar-cli --test text_project` | 2 passed: span이 가리키는 줄이 그 node 항목이고, 그 안의 width가 `stellar query`의 generation 값과 같음 |

## 테스트

- 위 명령 표의 테스트가 모두 통과했습니다. 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다. 열기·저장 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## 차이

- 작업 지시와 다른 점은 없습니다.

## 남은 위험

- 손으로 고친 파일이 canonical 들여쓰기(`  -`, `    id:`)를 따르지 않으면 그 node의 span을 찾지 못합니다. check는 문법만 보고 줄 형태를 강제하지 않습니다.

## 후속: MCP 성공 경로 (2026-11-24, ADR-0113)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp-core --locked --lib text_source_tests` | 0 — 1 passed |

| 시험 | 확인 |
|---|---|
| `reads_follow_a_source_only_edit_and_source_span_points_into_the_view_file`(`crates/mcp-core/src/headless/operations.rs`) | `stellar text-source adopt`와 같은 전환 뒤, View 원본만 바꾸면 MCP 문서 page가 원본 값을 읽고(snapshot 다시 엶), `document.source_span`의 파일이 그 View 파일이고 줄 범위 안에 그 node id와 바뀐 이름이 있음 |
