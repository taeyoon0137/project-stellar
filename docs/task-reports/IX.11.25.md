# 작업 보고서: IX.11.25 명시하지 않은 longhand를 활성 세트로 푸는 해석 함수 하나

- WBS: IX.11.25
- 작업명: 명시하지 않은 longhand를 활성 기본값 세트로 푸는 해석 함수 하나를 layout·scene·조회·codegen이 모두 쓰게 함
  (ADR-0082 결정 1)
- 상태: 부분 완료 (`[~]`) — 해석 함수는 하나이고 조회·codegen·native 레이아웃 입력이 그 함수를 씁니다. scene·runtime이
  레이아웃 결과로 그리지 않고, `react-native` 세트의 배치 결과 fixture가 없습니다(아래 "남은 위험").
- 결정: ADR-0086 결정 7·8, ADR-0087 결정 3, ADR-0115
- 선행: IX.11.24, IX.11.6, IX.11.4, IX.11.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- 이 보고서만 추가했습니다. 해석 함수의 코드는 앞선 기본값 세트 작업이 넣었고, 이번 작업은 그 함수를 쓰지 않던 CLI
  `style-defaults resolve`를 `ops::query::node_layout`으로 옮겼습니다(IX.11.29 커밋).

## 확인한 코드 위치

- 해석 함수: `crates/document/src/css/style_defaults.rs` `resolve`·`resolve_kind`. 활성 세트는 열 때 한 번
  `flatten`으로 [구획][longhand] 고정 배열로 평탄화합니다(`FlatSet`).
- 조회: `crates/ops/src/query.rs` `node_layout`·`node_detail`(CLI `query … node`의 `layout`, MCP `document.copy_css`의
  `layout`, CLI `style-defaults resolve`).
- codegen: `crates/ops/src/export_css.rs`(`resolve_kind`로 푼 값과 `differences`로 대상 기본값 차이), native 출력
  `crates/ops/src/export_native.rs`(`ResolvedLayout::declarations`).
- 레이아웃 엔진 입력: `crates/layout/src/declared.rs` `style_from_declarations`가 40개 longhand가 모두 있는 선언 목록만
  받습니다. 빠지거나 겹치면 거부하므로 엔진이 생략값을 채우는 경로가 없습니다(ADR-0086 결정 8).
- 원본: 텍스트 원본 v4 `layout` mapping과 chunk layout record는 명시 longhand만 씁니다(ADR-0086 결정 4·6).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::style_defaults` | 통과(13). 명시 값은 세트 값과 같아도 `explicit`, 생략은 `default(<세트>)` |
| `cargo test -p stellar-layout` | 통과(lib 112, css_conformance 4, layout_corpus 5, component 2·2). `css` 세트 선언 목록이 엔진 initial과 같은 값(`css_set_maps_to_initial_values_with_content_box`) |
| `cargo test -p stellar-cli --test style_defaults` | 통과(5). `resolve`와 `query node`가 같은 해석 결과 |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| `css` 세트에서 기존 layout·scene golden 불변 | layout golden 통과. scene은 레이아웃 결과로 그리지 않아 해석 함수를 거치지 않음(미충족, 소유 밖) |
| `react-native` 세트에서 같은 문서의 flex 기본 배치가 RN 기대값과 같음 | fixture 없음(미충족). 세트 표 값은 IX.11.31 fixture로 고정 |
| 엔진 crate에 생략값 상수 0건 | 문서 경로는 상수를 쓰지 않음. `LayoutStyle::default()`는 test·fixture·bench 생성자로 남음(ADR-0086 Alternatives). scene은 해당 없음 |
| 세트 값과 같은 명시 값이 명시로 남음 | `resolve_marks_explicit_and_default_sources`, CLI test |
| 키를 생략한 원본의 왕복 | 텍스트 원본 v4 `layout` mapping 왕복(ADR-0086 결정 6, `crates/project` 소유 test) |

## 차이(작업 지시와 다른 점)

- 이 작업의 소유 경로에는 `crates/layout`·`crates/scene`·`crates/runtime`이 없습니다. 엔진 연결과 scene이 레이아웃 결과로
  그리는 일은 하지 않았습니다.

## 남은 위험 / 한계

- scene·runtime이 레이아웃 결과로 그리게 되면(ADR-0086 Consequences) 이 행의 scene 기준과 제품 schema 1→2 step 등록
  (ADR-0115 결정 5)이 함께 풀립니다.
- `react-native` 세트로 푼 문서의 배치 결과 fixture(Yoga 기대값)가 필요합니다.
