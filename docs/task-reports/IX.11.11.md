# 작업 보고서: IX.11.11 Typography record와 Text style longhand

## WBS

- WBS: IX.11.11
- 작업명: Typography record를 CSS longhand로 정의하고 `TextStyle.typography` 참조 대상으로 연결
- 상태: 부분 완료 (`[~]`) — record 정의·상속·style 표 저장과 longhand 단위 merge는 앞선 작업(2026-11-14, `86a2e820`·`894f2918`·`9904a4c6`)에 있었고, 이번에 registry key·문서 주석·진단·옛 문서 migration step·ADR을 더했습니다. step 등록은 남았습니다(아래 "차이").
- 결정: [ADR-0075](../adr/ADR-0075-css-property-registry.md) 개정 결정 11.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/document/src/css/typography.rs`(신규): 일곱 longhand key(0x0510\~0x0516), 이름, CSS 매핑, registry Text 범위 항목, `line-height` 저장 값의 CSS 의미 문서 주석.
- `crates/document/src/css/{registry,mod}.rs`, `registry_snapshot.txt`: Text 범위에 일곱 항목.
- `crates/document/src/text_styles.rs`: `keys::LONGHANDS`를 css::typography에서 다시 내보냄(0x0501\~0x0507 → 0x0510\~0x0516), `diagnostics`(`typography-unrecognized`·`missing-font`·`typography-missing`), `ResolvedTextStyle::letter_spacing`, 상속·진단·initial test.
- `crates/document/src/text.rs`: `LineHeight` 문서 주석을 CSS 의미로.
- `crates/project/src/migration/steps/{mod,text_styles}.rs`, `text_styles.golden`(신규): 표가 빈 옛 문서의 기본 style 표 step과 golden.
- `docs/design/property_model.md`: 7장 Typography key 표, `TextStyle.typography`·`letter_spacing` 행.
- `docs/adr/ADR-0075-css-property-registry.md`: 개정 결정 11.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib text_styles` | 4 passed(생략값 = CSS initial, run→문단→문서 기본 상속과 letter-spacing, 불투명 blob 진단과 bytes 보존, missing font 진단) |
| `cargo test -p stellar-document --lib css::` | 통과(typography key 대역·중복 0, registry snapshot, 속성 모델 표 일치) |
| `cargo test -p stellar-project --lib steps::text_styles` | 2 passed(옛 Text 문서 기본 표 golden, 기존 표 유지와 누락 index 보고) |
| `cargo test -p stellar-semantic-diff -p stellar-semantic-merge -p stellar-document -p stellar-project` | 실패 0 |
| `cargo test -p stellar-ops --test typography_merge` | 2 passed(같은 Text의 `font-size`와 `letter-spacing`을 양쪽에서 바꾼 3-way merge conflict 0) |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| record round-trip | 기존 STTY v1 golden·round-trip test(core-types) |
| `line-height` 저장 값의 CSS 의미 정의가 record 문서 주석과 ADR에 있음 | `text::LineHeight`, `css::typography`, ADR-0075 결정 11 |
| 같은 diff의 ADR, `cargo xtask validate-adr` | ADR-0075 개정. 보호 경로 변경 없음 |
| 생략값이 CSS initial value와 같은지 test | 추가 |
| missing font diagnostic fixture | 추가(문서 단계). 시스템 글꼴 누락은 text 엔진 X.8.3 |
| 옛 Text payload migration golden | `text_styles.golden` |
| run→문단→문서 기본 상속 순서 fixture | 추가 |
| 불투명 blob 진단과 bytes 보존 fixture | 추가 |
| 서로 다른 style longhand 3-way merge conflict 0 | 기존 `crates/ops/tests/typography_merge.rs` 통과 |
| 갱신한 diff golden 목록 | semantic snapshot의 typography 경로 key가 0x0510\~0x0516으로 바뀌었습니다. 저장된 diff golden 중 typography 경로를 담은 파일은 없어 갱신한 golden은 없습니다(`registry_snapshot.txt`만 갱신) |

## 차이

- 기본 style 표 step은 `PRODUCT_STEPS`에 등록하지 않았습니다(IX.11.9·IX.11.10과 같은 1→2 묶음).
- 텍스트 원본의 typography 값 표기(`crates/project/src/document_io/text_codec/document_font.rs`)는 자체 px·수 formatter를 씁니다. `css::value`로 합치는 일은 그 파일 소유 작업입니다(이 lane 소유 밖, 새 이중 경로는 만들지 않음).
- 기본값 세트(`css::style_defaults`)에 Text 구획 typography longhand 항목이 없습니다. typography는 resource 값이 언제나 명시 값이라 지금은 필요 없습니다.
  - (2026-11-23 style-defaults-finish 갱신) 세트 표에 typography 일곱 longhand의 `css`·`react-native` 값이 들어갔습니다(IX.11.24, ADR-0115 결정 1). 생략값 test의 CSS initial value와 같고(`font-size`는 UA 16px), conformance fixture(IX.11.31)가 고정합니다.
  - step 등록 조건은 ADR-0115 결정 5입니다.

## 남은 위험

- semantic snapshot typography 경로 key가 바뀌어, 이 변경 전 빌드로 만든 snapshot과 비교하면 경로가 다릅니다. snapshot은 저장하지 않아 파일 영향은 없습니다.
