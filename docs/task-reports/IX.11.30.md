# 작업 보고서: IX.11.30 codegen의 기본값 세트 차이 출력

## WBS

- WBS: IX.11.30
- 작업명: 대상 고유 기본값과 활성 기본값 세트의 차이를 명시 선언으로 내기(Web 특이도 0 기반 규칙, RN 스타일 객체)
- 상태: 완료 (`[x]`) — 완료 기준 세 가지(세트 두 개 × 대상별 golden, 같은 세트·대상에서 추가 출력 0, 저자 규칙이
  기반 규칙을 이기는 특이도 fixture)를 test로 확인했습니다.
- 결정: ADR-0082 결정 6, ADR-0086 결정 12, ADR-0089
- 선행: IX.11.25(해석 함수 `style_defaults::resolve_kind`·`differences`는 이미 들어와 있습니다)

## 변경 파일

- `docs/adr/ADR-0089-css-layout-export-and-style-default-differences.md`(신규) — 배치·차이·border 보정·RN 낮춤 결정.
- `crates/codegen/src/web/layout.rs`(신규) — 특이도 0 기반 규칙(`:where(#<root>, #<root> *)`, 구획 class 규칙) 조립.
- `crates/codegen/src/react_native.rs`(신규) — CSS layout longhand → RN 스타일 항목 낮춤 규칙(Target Profile 상태)과
  `StyleSheet.create` writer.
- `crates/codegen/src/web/frame.rs`(수정) — 고정 Free Frame 선언을 layout 부분과 paint 부분으로 나눔(기존 출력 bytes
  동일).
- `crates/codegen/src/web/check.rs`(수정) — HTML 전역 속성 `class`를 정적 검사 허용 속성에 더함.
- `crates/codegen/src/lib.rs`, `crates/codegen/src/web/mod.rs`(수정) — 모듈 등록.
- `crates/ops/src/export_css.rs`(신규) — 요소별 layout 계획(`plan_element`), 기반 규칙, RN 출력
  (`export_react_native`).
- `crates/ops/src/export_css/fixture.rs`·`tests.rs`(신규) — 고정 문서와 cascade 계산 대조 test.
- `crates/ops/src/web_frame.rs`(수정) — 기존 export-web walk가 계획을 부르고 결과에 `format`·`style_defaults`·
  `styles`·`base_rules`를 싣게 함.
- `crates/ops/src/lib.rs`(수정) — 모듈 등록.
- `tools/stellar-cli/src/export_styles.rs`(신규), `cli.rs`·`lib.rs`·`tests/fixtures/help.txt`(명령 등록) — `stellar
  export-styles`.
- `tools/stellar-cli/src/web_frame.rs`(수정) — 결과 `format`을 투영 값 그대로 내고 입력 오류 분류를 공용 함수로 뺌.
- `tools/stellar-cli/tests/export_styles.rs`, `tools/stellar-cli/tests/export_fixtures/*`(신규) — 2×2 golden과
  record 대조 CLI test.
- `crates/mcp-core/src/headless/operations.rs`, `crates/agent-protocol/src/catalog/tools.rs`(수정) — MCP
  `document.export_web_frame`의 결과 확장과 선택 입력 `target`.
- `tools/stellar-mcp/tests/headless_tools.rs`(수정), `tools/stellar-mcp/tests/fixtures/catalog.json`·
  `trust-boundary.jsonl`(golden 갱신) — tool 설명·schema 변경 반영과 `target` test.
- `docs/task-reports/IX.11.30.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (54 통과) |
| `cargo test -p stellar-ops --lib` | 0 (89 통과, export_css 7 포함) |
| `cargo test -p stellar-cli` | 0 (export_styles 3, web_frame 7 포함 전부 통과) |
| `cargo test -p stellar-mcp-core -p stellar-mcp -p stellar-agent-protocol` | 0 (catalog·trust-boundary golden 갱신 후) |
| `cargo clippy -p stellar-codegen -p stellar-ops -p stellar-cli -p stellar-mcp-core -p stellar-mcp --all-targets --features stellar-ops/test-support -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo xtask validate-adr --base 33ba9b2f` | 0 |
| `cargo xtask generate-ffi-header --check` | 0 |
| `STELLAR_CHECK_BASE=33ba9b2f cargo xtask check-core` | 65 (test 단계: git-porcelain `index_publish` 시간 의존 test 2개 실패, 아래 참고) |
| `cargo test -p stellar-git-porcelain --lib withheld_input_timeout -- --test-threads=1` | 첫 실행 101, 두 번째 0 |
| `cargo test --workspace --locked --no-fail-fast` | 101 (stellar-runtime `unsolicited_view_changes_coalesce…` 1개만 실패, 나머지 170개 test 묶음 통과) |
| `cargo test -p stellar-runtime --lib unsolicited_view_changes_coalesce` | 0 (단독 3회) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 별도 artifact는 없습니다. golden은 `tools/stellar-cli/tests/export_fixtures/`에 있습니다.

## 테스트

- 세트 두 개 × 대상 두 개 golden: `two_sets_by_two_targets_match_goldens_and_leave_project_bytes_unchanged`
  (`css-web.html`, `react-native-web.html`, `css-react-native.js`, `react-native-react-native.js`). 실제 `stellar`
  subprocess 출력이고, 반복 stdout이 같고, 프로젝트 파일 hash가 그대로입니다.
- 추가 출력 0: `same_set_as_target_adds_no_default_declarations`
  - `css` 세트 → Web: 기반 규칙이 없고, 요소 규칙에는 명시·배치·보정·숨김 값만 나옵니다.
  - `react-native` 세트 → RN: 명시 값과 배치 항목만 나옵니다.
  - CLI test는 `css` 세트의 `base_rules`가 빈 문자열인지 확인합니다.
- 특이도: `author_rule_beats_zero_specificity_base_rule`
  - `react-native` 세트 기반 규칙의 `flex-direction: column`(특이도 0)보다 Frame 12의 명시 `flex-direction: row`
    (`#id`, 1-0-0)가 계산값입니다.
  - 명시하지 않은 Frame 11은 기반 규칙 값 `column`을 받습니다.
- 브라우저 없는 계산 대조: `computed_cascade_matches_document_resolution_for_both_sets`
  - 출력 `<style>`의 규칙을 특이도·순서로 cascade하고, 맞는 선언이 없으면 대상 브라우저 기본값(`css` 세트)을 씁니다.
  - 그 계산값이 요소의 40개 longhand 모두에서 문서 해석 값(활성 세트)과 같습니다. 배치 규칙 값만 ADR-0089 결정 4의
    기대값으로 바꿉니다.
  - 두 세트, 세 View에서 확인했습니다.
- RN 차이: `react_native_output_states_css_set_differences`
  - `css` 세트에서 `position: 'static'`, `boxSizing: 'content-box'`, `flexShrink: 1`이 나옵니다.
  - block 부모 자식에는 flex item 속성이 없습니다.
  - `react-native` 세트에서 같은 Frame은 크기만 남습니다.
- export-web CSS ↔ record: `export_web_rules_carry_the_resolved_explicit_values`. `style-defaults resolve`의 명시 값이
  export-web 요소 규칙에 같은 `name: value`로 있습니다(두 세트, 명시 값 15개 이상).
- check-core는 통과하지 못했습니다.
  - 실패: `stellar-git-porcelain`의 `a_foreign_git_lock_is_never_mistaken_for_our_childs_lock`(Timeout)과
    `withheld_input_timeout_cancel_and_rejection_leave_index_exactly_unchanged`. 다른 작업 여러 개가 동시에 돌던
    부하 상태였습니다.
  - workspace 전체 실행에서는 그 둘이 통과했고 `stellar-runtime` socket test 하나가 `WouldBlock`으로 실패했습니다.
    단독 실행은 통과했습니다.
  - 이 diff는 git-porcelain·runtime을 바꾸지 않았습니다. 부하에 따른 간헐 실패로 보지만, 부하 없는 환경에서
    check-core를 다시 돌려 확인하지는 못했습니다.
- 기존 고정 Free Frame golden(`tools/stellar-cli/tests/fixtures/web/view-a.html`·`view-b.html`)은 바뀌지 않았습니다.

## 성능

- 해당 없음. 요청당 세트 평탄화 1회와 node당 40개 고정 배열 조회입니다. 측정하지 않았습니다.

## 차이

- ADR-0082 예시는 root class(`.<root>`)였습니다. 구현은 root 요소 id(`:where(#<root>, #<root> *)`)를 씁니다.
  `:where()` 안이라 특이도가 같고 새 속성이 필요 없습니다(ADR-0089 결정 5).
- 구획마다 값이 다른 차이(`react-native`의 Frame `display: flex` 등)는 `stellar-<구획>` class를 붙인 두 번째 기반 규칙에
  둡니다.
- RN 대상 출력은 layout longhand만 냅니다. 요소 구조·paint는 Phase XXII RN·Android adapter 몫입니다.
- 작업 정의 owns 밖 파일을 고쳤습니다. 기존 export 경로를 확장하라는 작업 정의를 따른 것입니다(이중 경로 금지).
  - `crates/ops/src/web_frame.rs`
  - `tools/stellar-cli/src/web_frame.rs`
  - MCP 두 파일
  - `crates/codegen/src/web/check.rs`
  - stellar-mcp test·golden

## 남은 위험

- 흐름 배치(`flow`·`inset`)는 캔버스와 다를 수 있습니다. 캔버스가 아직 CSS 레이아웃 결과로 그리지 않기
  때문입니다(SD.2). `layout-not-on-canvas` 진단으로 알립니다.
- IX.11.9 전 연결 규칙(Free 자식의 transform 이동 → `left`·`top`)은 IX.11.9 migration이 들어오면 바꿔야 합니다.
- Stellar border 보정(ADR-0089 결정 6)은 padding이 border보다 작은 흐름 부모에서 근사입니다
  (`approximated-border-padding`). IX.11.10 뒤 지웁니다.
- `docs/spec/web-frame-export.md`에 `css-layout-html` 형식 설명이 없습니다. 소유 밖이라 고치지 않았습니다.
- 브라우저에서 직접 연 확인은 하지 않았습니다. 계산 대조는 cascade 계산값 비교이고, 같은 값의 배치가 Chrome과 같은지는
  layout 엔진 corpus(IX.11.8)에 기댑니다.
