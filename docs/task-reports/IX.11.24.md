# 작업 보고서: IX.11.24 생략값 기본값 세트 표를 registry longhand별 항목으로

- WBS: IX.11.24
- 작업명: 생략값 기본값 세트 표를 속성 registry(ADR-0075)의 longhand별 항목으로 둠(ADR-0082 결정 2)
- 상태: 완료 (`[x]`) — 완료 기준을 모두 확인했습니다. 선행 IX.11.3이 `[~]`라 계획서 표기는 통합 때 선행 판정을 따릅니다.
- 결정: ADR-0115 결정 1(신규), ADR-0086 결정 7, ADR-0082 결정 2
- 선행: IX.11.2, IX.11.3, IX.11.14

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/css/style_defaults.rs`
  - `payload_longhands()`: registry 항목 가운데 기본값 세트가 값을 두는 payload longhand(node 종류 payload 범위, 묶음
    key 아님, `stellar-only` 아님, 이름이 자기 CSS longhand, layout longhand 이름 아님).
  - `payload_entries()`·`PayloadEntry`: 내장 세트 payload 표(registry 범위·key별 한 줄, 정규 값, 근거).
  - `FlatSet::payload_value(scope, key)`, `scope_class`, `check_payload_value`, `payload_snapshot_text()`.
  - 근거 `Basis::CssComputed`·`Basis::ReactNativeSource`, `Basis::ALL`·`from_name`.
  - test 3개 추가, 문서 표 비교 test에 payload 표를 더함.
- `docs/spec/style-defaults.md`: 1장 범위, 2.4절 payload longhand 표와 근거.

## 구현 요약

- 세트 표는 layout longhand 40개(기존)와 payload longhand 33개입니다. 두 표를 합치면 Stellar가 node에 저장하는 CSS
  longhand 전체입니다. CSS longhand 하나가 아닌 Stellar 단일 값(`visible`, Frame `clipping`, Path `path-data`)은
  표에 없습니다(ADR-0115 결정 1, 통합 검토에서 명시).
- payload 값은 지금 모델에서 언제나 명시 값이라 해석 함수 `resolve`는 바뀌지 않습니다. 표는 payload 희소 저장 step,
  codegen, 조회가 같은 값을 쓰도록 registry key별로 한 곳에 둡니다.
- `react-native`가 바꾸는 payload 값은 `border-*-style: solid`, `font-family: system-ui`, `font-size: 14px`,
  `object-fit: cover`입니다. 나머지는 `=`(물려받음)입니다.
- 사용자 세트는 payload longhand를 두지 않습니다. payload 값은 사슬의 내장 세트 값입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::style_defaults` | 통과(11). 모든 payload longhand에 두 세트 값 존재, 모든 값이 registry 값 문법 parser로 읽히고 정규 표기와 같음, 표 행 중복 없음, 문서 표 두 개와 snapshot 일치. |
| `cargo test -p stellar-document --lib css::` | 통과(52). |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| 모든 registry longhand에 두 세트의 값이 있음(물려받음 표시 포함) | layout 40개 × 구획 4개(기존 test), payload 33개(`every_registry_longhand_has_values_in_both_builtin_sets`). 문서 표의 `=`가 물려받음 |
| 문서 표와 registry 표의 비교 test 통과 | `spec_document_table_matches_snapshot`(layout 표, payload 표) |
| 표 값이 registry 값 문법 parser로 읽힘 | layout은 `LayoutLonghand::parse`(기존), payload는 `check_payload_value`(registry 문법 parser + 정규 표기, 값 변형 규칙) |

## 차이(작업 지시와 다른 점)

- layout longhand는 `registry::entries()`에 넣지 않았습니다. 이름 조회를 쓰는 곳(19곳)의 결과가 바뀌기 때문입니다.
  layout 표는 기존대로 `LayoutLonghand`(node 공통·Frame 범위 key)가 항목이고, payload 표는 registry 항목을 key로
  참조합니다(ADR-0115 Alternatives).
- `background-color`의 `css` 값은 `transparent`의 정규 표기 `#00000000`으로 적었습니다.

## 남은 위험 / 한계

- `font-family`의 `css` 값 `serif`는 브라우저 계산값이 아니라 이식 가능한 generic family입니다. 실제 기본 글꼴은 OS 언어
  설정이 정합니다(IX.11.31 fixture에 이유 기록).
- RN 값 일부는 RN 구현 기본값(`react-native-source`)이고 공식 문서 표에는 없습니다.
- codegen 차이 출력(`differences`)은 layout longhand만 봅니다. payload 차이 출력은 codegen 소유 작업입니다.
