# 작업 보고서: IX.11.28 속성 조회 결과의 값 출처

- WBS: IX.11.28
- 작업명: 속성 조회 결과(`stellar query`, MCP 읽기, 속성 탭 snapshot)가 값마다 출처 `explicit` 또는
  `default(<세트 이름>)`를 돌려주게 합니다(ADR-0082 결정 5).
- 상태: 부분 완료 (`[~]`) — `stellar query`와 MCP 읽기는 끝났습니다. 속성 탭 snapshot은 UI 재설계 때 같은 함수를
  씁니다(2026-11-23 사용자 방향: 지금은 엔진·기능만).
- 결정: ADR-0087(신규) 결정 1·3, ADR-0082 결정 5, ADR-0086 결정 7·11
- 선행: IX.11.25, IX.11.27, XXIV.4.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/snapshot.rs`: 열 때 활성 세트를 한 번 평탄화(`style_defaults`, `style_defaults_name`), 문서 모델
  거울 채우기. 세트 오류는 열기를 막지 않습니다.
- `crates/ops/src/apply.rs`: transaction이 바꾼 거울을 generation 저장 뒤 `stellar.yaml` 한 줄로 씁니다(준비는 저장
  전, 실패하면 `Rejected`). IX.11.27의 "세트 변경 뒤 저장 diff가 한 줄"을 ops 경로에서 만족시키는 연결입니다.
- `crates/ops/src/query.rs`: `node_layout`, `NodeLayout`, `LayoutProperty`, `LayoutSource`(고정 크기 enum),
  `LayoutUnavailable`, `NodeDetail::layout`.
- `tools/stellar-cli/src/views.rs`: `node.layout` JSON(`views::node_layout`).
- `crates/mcp-core/src/headless/operations.rs`: `document.copy_css` 결과의 `layout`.
- `tools/stellar-cli/tests/fixtures/results/query-node.json`: golden에 `layout` 필드.
- `tools/stellar-cli/tests/layout_provenance.rs`, `tools/stellar-mcp/tests/layout_provenance.rs`,
  `tools/stellar-mcp/tests/fixtures/layout-provenance.json`(신규): CLI·MCP 공유 golden.

## 구현 요약

- 해석은 문서 crate `style_defaults::resolve` 하나, 값 표기는 `layout_record::format_value` 하나입니다. 조회는 연
  snapshot의 세트를 씁니다.
- 출처는 `LayoutSource { Explicit, Default }`이고 세트 이름은 결과에 한 번(`style_defaults`)입니다. 표기는
  `NodeLayout::source_label`입니다.
- JSON 모양은 CLI와 MCP가 같습니다: `{style_defaults, class, values: [{longhand, value, source}]}`. 상자가 없는
  node는 `null`, 세트를 풀지 못하면 `{error: {code, message}}`(고정 문구)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib layout_provenance` | 통과(3). 명시 값 `explicit`, 생략 값 `default(css)`, 세트를 `react-native`로 바꾸면 `default(react-native)`, 세트 값과 같은 명시 값은 `explicit`, `UnsetProperty`(canonical apply) 뒤 `default(react-native)`, 없는 세트는 node 결과의 `StyleDefaults` 오류이고 조회는 성공. |
| `cargo test -p stellar-ops --lib style_defaults_mirror` | 통과(3). 열 때 거울 = 매니페스트 선택, 잘못된 세트 파일이 문서 읽기를 막지 않음, canonical `SetStyleDefaults` 쓰기가 `stellar.yaml` 한 줄, 되돌리는 command 하나가 원래 bytes, dry-run은 쓰지 않음, 없는 세트는 `Rejected { index: 0 }`이고 파일 그대로, 사용자 세트 선택. |
| `cargo test -p stellar-cli --test layout_provenance` | 통과. `stellar query … node`의 `node.layout`이 공유 golden과 같음. |
| `cargo test -p stellar-mcp --test layout_provenance` | 통과. MCP `document.copy_css`의 `layout`이 같은 golden과 같음, 실행 전후 파일 hash 같음. |
| `cargo test -p stellar-cli --test output_fixtures` | 통과(26). `query-node.json` golden에 `layout` 필드(`STELLAR_CLI_BLESS=1`로 갱신 후 검토). |
| `cargo test -p stellar-project -p stellar-ops -p stellar-cli -p stellar-mcp -p stellar-mcp-core` | 통과. |

## 차이(작업 지시와 다른 점)

- "세트 이름은 세트 표 index로 줄여 씀"은 값마다 index를 두는 대신 결과에 세트 이름을 한 번 두는 것으로
  구현했습니다. `default` 출처의 세트는 언제나 활성 세트 하나라 index가 필요 없습니다(ADR-0087 결정 3).
- headless MCP에는 node 속성을 읽는 tool이 따로 없어 기존 `document.copy_css` 결과에 `layout`을 더했습니다. tool
  목록과 schema는 바꾸지 않았습니다.
- 속성 탭 snapshot은 UI 재설계 때 합니다.

## 남은 위험 / 한계

- MCP 세션 cache의 최신 판정이 사용자 세트 파일 변경을 보지 않습니다. 같은 세션에서 세트 파일만 바뀌면 `layout`이
  옛 세트로 남을 수 있습니다(`ops::trusted` 소유 후속).
- `stellar style-defaults resolve`는 같은 공개 함수를 직접 부릅니다. `node_layout`으로 옮기는 일이 남았습니다(CLI
  소유 후속).
- 앱 runtime 열기·저장 연결은 ADR-0087 결정 4 사용법대로 runtime 작업이 합니다.
