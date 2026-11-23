# 작업 보고서: IX.11.6 레이아웃 longhand 영속 record와 기본값 세트 연결

- WBS: IX.11.6
- 작업명: 레이아웃 container·item 속성을 CSS longhand로 저장하는 영속 record. 같은 lane에서 기본값 세트(ADR-0082,
  신규 SD.1–SD.8)도 구현했습니다.
- 상태: 부분 완료 (`[~]`) — record·key·형식·codec·검증·해석·엔진 연결은 끝났습니다. `cargo xtask test-migrations`는
  제품 step이 아직 없어 새로 검증할 내용이 없습니다. 복제·붙여넣기 command가 record를 옮기지 않는 한계가 남았습니다.
- 결정: ADR-0086(신규), ADR-0082, ADR-0076, ADR-0077 개정(ADR-0086 결정 6)
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

- `InsertNode`·`DuplicateSubtree`·`Paste`가 layout record를 옮기지 않습니다. 복제한 node는 명시 값을 잃습니다.
- semantic diff·merge snapshot에 layout record 투영이 없습니다.
- payload 크기 종류의 `auto`·백분율 크기는 아직 저장할 수 없습니다.
- RN 세트 값은 Yoga로 실행한 fixture가 없습니다(문서 근거와 동등성 test만).
- 기본값 세트 codegen 출력(SD.7)은 codegen-css lane 몫입니다. MCP 출처 표시와 `stellar query` 출처 열도 남았습니다.
