# 작업 보고서: IX.11.6 레이아웃 longhand 영속 record와 기본값 세트 연결

- WBS: IX.11.6
- 작업명: 레이아웃 container·item 속성을 CSS longhand로 저장하는 영속 record. 같은 lane에서 기본값 세트(ADR-0082,
  신규 SD.1–SD.8)도 구현했습니다.
- 상태: 완료 (`[x]`) — record·key·형식·codec·검증·해석·엔진 연결(첫 lane)에 더해, 후속 lane(layout-wiring)이
  편집·동기화 경로가 record를 잃지 않게 연결했습니다(아래 "후속: 편집·동기화 경로 연결"). `cargo xtask test-migrations`는
  통과합니다(제품 step이 없어 새로 검증한 step은 없음).
- 결정: ADR-0086(신규, 2026-11-23 layout-wiring 개정 포함), ADR-0082, ADR-0076, ADR-0077 개정(ADR-0086 결정 6)
- 선행: IX.11.4, IX.11.5, IX.11.21, IX.11.22(main에 통합됨)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). UA box-sizing 기대값 생성만 로컬 Google Chrome 157.0.8103.79(headless)를
  썼습니다.

## 변경 파일 목록

- `crates/document/src/layout_record.rs`(신규): longhand 40개, key, 정규 값 해석·표기, binary 형식, 검증.
- `crates/document/src/{records,model,record_codec,validate,lib}.rs`: 희소 record 목록, 모델 보관, subtree 선택 절,
  validator 위반.
- `crates/document/src/css/style_defaults.rs`(신규): 내장 세트 표, 사용자 세트 평탄화, `resolve`, `differences`.
- `crates/document/src/css/mapping.rs`: CSS longhand inventory에 `grid-auto-columns` 한 줄.
- `crates/layout/src/declared.rs`(신규), `crates/layout/src/{style,style_codec,engine,flex,input,property,dirty}.rs`:
  선언 목록 → `LayoutStyle`, `box-sizing`.
- `crates/layout/tests/css/*`, `crates/layout/tests/fixtures/*`: content-box fixture 10개, UA box-sizing 기대값, 공유
  정규 값 fixture, corpus 입력 hash 갱신.
- `crates/project/src/document_io/source_v4/*`, `component_source.rs`, `text_codec.rs`: v4 `layout` mapping, Frame
  키 생략 읽기.
- `crates/project/src/{manifest,manifest_style_defaults}.rs`: `style_defaults` 키와 사용자 세트 파일.
- `crates/commands/src/style_defaults.rs`(신규)와 등록(`kind`·`codec`·`command`·`edit`·`test_support`·`fuzz`·
  `property_tests`): command 세 개.
- `tools/stellar-cli/src/style_defaults.rs`(신규), `crates/ops/src/lib.rs`: `stellar style-defaults`.
- `tests/integration/tests/style_defaults_layout.rs`(신규): 종단 적합성.
- `docs/adr/ADR-0086-css-layout-records-and-style-defaults.md`, `docs/spec/style-defaults.md`(신규).

## 구현 요약

- record는 node별 명시 longhand만 담는 희소 map입니다. 빈 record는 저장하지 않습니다. item longhand는 모든 상자
  node에, container longhand는 Frame에만 붙습니다.
- Frame·Rectangle·Ellipse·Polygon·Image의 `width`·`height`는 payload 크기(IX.11.22·IX.11.3 key)입니다. 그 밖의
  종류는 `0x0016`·`0x0017`입니다. node 종류마다 key가 하나입니다(`LayoutLonghand::key_for`).
- chunk는 subtree 끝 선택 절이라 record가 없으면 bytes가 예전과 같습니다.
- 생략값은 `style_defaults::resolve`가 활성 세트로 풉니다. 엔진은 `declared::style_from_declarations`로 모든
  longhand가 있는 목록만 받습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(427). record golden·왕복·모든 상자 종류 item record·subtree bytes· validator·세트 표·해석·문서 표 대조. |
| `cargo test -p stellar-layout` | 통과. `css_conformance` 267 fixture(content-box 10개 포함) 일치. |
| `cargo test -p stellar-integration-tests --test style_defaults_layout` | 통과. 문서 parser·`css` 세트·엔진 결과가 Chrome UA box-sizing 기대값 267개와 모두 일치(건너뜀 0). `react-native` 세트 = `css` 세트 + 명시 차이. |
| `cargo test -p stellar-project`, `-p stellar-commands`, `-p stellar-cli` | 통과. |
| `cargo xtask validate-adr --base d2bf97b3`, `cargo xtask generate-ffi-header --check` | 통과. |

## 차이(작업 지시와 다른 점)

- container longhand는 Frame payload 확장이 아니라 같은 희소 record에 둡니다(ADR-0086 결정 1).
- Ellipse 원점·Transform modifier가 있는 Rectangle의 화면 위치 보존은 payload·node 필드가 바뀌지 않음으로
  확인했습니다. scene이 아직 layout 결과를 읽지 않기 때문입니다.
- command를 세 개(`0x0091`–`0x0093`) 더했습니다(ADR-0086 결정 10).

## 남은 위험 / 한계

- (후속에서 해결) `InsertNode`·`DuplicateSubtree`·`Paste`가 layout record를 옮기지 않던 한계와 semantic diff·merge
  투영이 없던 한계는 아래 후속 절에서 닫았습니다.
- payload 크기 종류의 `auto`·백분율 크기는 아직 저장할 수 없습니다.
- RN 세트 값은 Yoga로 실행한 fixture가 없습니다(문서 근거와 동등성 test만).
- 기본값 세트 codegen 출력(SD.7)은 codegen-css lane 몫입니다. MCP 출처 표시와 `stellar query` 출처 열도 남았습니다.

## 후속: 편집·동기화 경로 연결 (lane layout-wiring, 2026-11-23)

첫 lane이 남긴 한계 가운데 record가 편집·동기화 경로에서 빠지는 문제를 닫았습니다. 결정은 ADR-0086의
"개정(2026-11-23, layout-wiring)" 절에 있습니다.

### 바꾼 것

- `crates/commands/src/node_ops.rs`: `InsertNode.layout_records`를 더했습니다. payload 끝의 선택 절이고 검증은
  `check_records`입니다. `DeleteSubtree`의 역이 subtree record를 담습니다. 같은 id를 다시 넣으면 남은 옛 record를 지웁니다.
- `crates/commands/src/duplicate.rs`, `paste.rs`: 복제와 클립보드 붙여넣기가 record 소유 node를 새 id로 옮깁니다.
- `crates/commands/src/snapshot_apply.rs`: `RecordChange::Layout`(tag 7)을 더했습니다. 외부에서 고친 `layout` 줄이 모델에
  반영됩니다.
- `crates/commands/src/codec.rs`: command codec version을 4로 올렸습니다. 절·tag가 없으면 bytes는 version 3과 같습니다.
- `crates/semantic-diff/src/{build,records,snapshot,keys}.rs`: 명시 longhand마다 node 객체 속성 하나로 투영·역투영합니다.
  경로는 record key, 값은 정규 CSS 값 텍스트입니다. `crates/semantic-merge/src/merge.rs`: longhand별 병합·conflict test.
- `crates/ops/src/{apply,mcp_command_input,css}.rs`, `schemas/command-transaction.schema.json`,
  `tools/stellar-mcp/tests/fixtures/*`:
  - `set-layout-property`·`unset-property` 필드 형식을 더했습니다.
  - `ops::css::paste_plan`이 속성 패널 행이 없는 layout longhand를 record로 받습니다.
  - 복사가 명시 record 값을 함께 냅니다.
  - 선언 블록 상한 `MAX_DECLARATION_BYTES`를 두었습니다.
- `tools/stellar-cli/src/apply.rs`: command object 해석을 `stellar_ops::mcp_command_input::command_input`으로 바꿨습니다.
- `crates/mcp-core/src/proposal.rs`: 소스 비교 test를 지우고 layout 입력 test를 더했습니다.
- `tools/stellar-cli/src/css.rs`, `crates/mcp-core/src/headless/operations.rs`: 공용 상한 상수를 씁니다.
- 소유 밖 최소 수정: 필드 추가로 컴파일이 깨진 `InsertNode` struct literal에 `layout_records: Vec::new()` 한 줄씩
  넣었습니다. 대상은 `crates/commands/src/convert.rs` test와 `tests/integration/tests/`의 `chunk_layout.rs`,
  `component_library.rs`, `project_lifecycle.rs`입니다.

### 완료 기준 대조

| 기준 | 근거 |
|---|---|
| ADR 존재 | ADR-0086과 그 개정 절 |
| record round-trip·canonical bytes golden | 첫 lane `stellar-document` test. 후속에서 command codec·semantic snapshot 왕복을 더함 |
| `width`·`height` key가 종류마다 하나 | `LayoutLonghand::key_for`, 첫 lane test |
| 상자 node 종류별 item record round-trip | 첫 lane test, 후속 `InsertNode` 검증 test(Text record 크기) |
| bounds 원점·Transform modifier 화면 위치 불변 | record는 layout 입력에만 쓰이고 payload·modifier를 바꾸지 않음(ADR-0086 결정 3) |
| unknown field 정책 | 모르는 longhand 번호·trailing bytes 거부 test, v4 모르는 키 거부 |
| `cargo xtask test-migrations` | 통과(fixture 2개) |

### 후속 lane에서 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands` | 통과(179, 새 test 9: 삭제 역의 record·journal 재열기 복원, 옛 bytes 유지, canonical 절, 삽입 검증, 옛 record 정리, 복제, 클립보드, 외부 변경 2) |
| `cargo test -p stellar-semantic-diff`, `-p stellar-semantic-merge` | 통과(longhand별 투영·왕복·거부, longhand별 병합·conflict) |
| `cargo test -p stellar-ops -p stellar-cli -p stellar-mcp-core -p stellar-mcp` | 통과(apply 두기·지우기·거부, css 붙여넣기·복사, CLI·MCP 해석). MCP fixture `catalog.json`·`trust-boundary.jsonl`은 `STELLAR_MCP_BLESS=1`로 다시 만듦(schema 변경분) |
| `cargo test -p stellar-history` | 통과(codec version 4에서 journal 읽기) |
| `cargo xtask generate-command-schema` | schema 다시 생성 |
| `cargo xtask validate-adr --base 33ba9b2f`, `cargo xtask generate-ffi-header --check`, `cargo xtask test-migrations` | 통과 |
| `cargo test --workspace --locked --no-fail-fast` | 이 lane 변경과 무관한 시간 의존 test 3개만 실패: `stellar-ffi` `unpolled_requests_are_bounded`, `stellar-local-ipc` `slow_authenticated_peer_does_not_block_other_handshakes`, `stellar-project` `restricted_yaml_fuzz_campaign_has_no_findings`(3s 상한을 3.2s로 넘음). 셋 다 단독 실행에서 통과했고 이 lane은 세 crate를 바꾸지 않았습니다 |
| `STELLAR_CHECK_BASE=33ba9b2f cargo xtask check-core`(샌드박스 해제) | 두 번 실행했고 두 번 다 test 단계에서 위 시간 의존 test(`stellar-ffi`, `stellar-local-ipc`)로 실패했습니다. fmt·clippy 단계는 통과했습니다. 전체 통과는 확인하지 못했습니다 |

### 후속 뒤에도 남은 한계

- View 삭제(`RemoveView`)의 역 `AddView`는 View tree의 record를 담지 않습니다(`commands/src/page_view.rs`, 소유 밖).
  저장된 journal로 다시 연 뒤의 View 삭제 undo는 record를 잃습니다.
- `stellar query` node 속성 목록(`ops::query::node_detail`)은 layout 속성을 내지 않습니다. IX.11.28과 함께 처리합니다.
- `inset` 붙여넣기는 `top`·`left`가 위치 transform, `right`·`bottom`이 record로 나뉩니다(IX.11.9 전까지).
- 컴포넌트 원본·정의 node의 record, `SetStyleDefaults` 거울 연결, payload 크기 종류의 `auto`·백분율은 그대로 남습니다.
