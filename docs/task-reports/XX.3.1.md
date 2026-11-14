# 작업 보고서: XX.3.1 Scene primitive → SVG element mapping table

- WBS: XX.3.1
- 작업명: Scene primitive→SVG element mapping table 정의 — Frame/Shape/Path/Image/clip/gradient 기본 지원 명시
- 상태: 완료 (`[x]`)
- 선행: XX.1.2 (`[x]`, 이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/svg/mod.rs`(신규) — SVG 변환 모듈.
- `crates/export/src/svg/mapping.rs`(신규) — `SvgFeature`(34개), `SvgSupport`, `SVG_MAPPING`,
  `node_features`, `first_unsupported`.
- `crates/export/src/svg/format.rs`(신규) — 숫자·색·transform·path data·base64·safe id 표기.
- `crates/export/src/svg/element.rs`(신규) — `SvgGeometry`와 요소별 표기(fill, stroke, clipPath,
  그룹, 배경, image).
- `crates/export/src/lib.rs` — 모듈 선언.
- `docs/task-reports/XX.3.1.md`(신규), 계획서(XX.3.1 `[ ]`→`[x]`).

## 구현 요약

- **mapping table**: scene이 표현하는 기능을 `SvgFeature`로 열거하고, row마다 scene 원천(어느
  document node·scene 값), 대응 SVG 요소·속성, 지원 상태를 기록합니다.
  - vector: Frame 상자(`<rect>`/원호 `<path>`), Shape·Path(`<path d>`, even-odd), Image(`<svg viewBox
    preserveAspectRatio>` + data URI `\<image>`, Fill/Contain/Cover/None, crop, nearest sampling),
    단색 fill, sRGB(`rgb(%)`)·wide gamut(CSS Color 4 `color()`)·반투명(`*-opacity`), Center·Inside
    stroke(Inside = 두 배 폭 + 자기 기하 clipPath), Round·Square cap, Rect·둥근 Rect·Path clip과 중첩
    clip chain, group opacity, world transform(`matrix`), 숨김(생략).
  - 후속 구현: Text → glyph outline(XX.3.2), Outside stroke → mask(XX.3.4).
  - 부분 래스터화: drop shadow·layer blur(XX.4.2, 렌더러 blur 수식과 SVG filter 반경 의미 차이),
    Normal 외 blend(XX.4.1, backdrop 의존 범위), image 색공간 override(XX.4.2).
  - scene 미지원: linear·radial gradient — scene `Paint`에 아직 없어 대응 요소(`<linearGradient>`·
    `<radialGradient>`, userSpaceOnUse, stop 시각 순서)만 명시합니다.
- **분류**: `node_features`가 scene enum을 wildcard 없이 match하므로 scene variant가 추가되면 표를
  갱신하기 전까지 컴파일되지 않습니다.
- **좌표계 규칙**: node 그룹은 world 좌표(변환 없음), 내용 요소가 world transform을 직접 가지며,
  조상 clip은 clip 자체 transform으로 그린 clipPath를 그룹에 겁니다.
- **표기 결정론**: 숫자는 f32 최단 round-trip 십진(지수 없음, `-0`→`0`), 계산 결과가 비유한이면
  typed error, 속성 순서 고정, id는 `[A-Za-z][A-Za-z0-9_-]*` ≤128 byte만 허용(escape 불필요).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 37 passed, 0 failed (이 작업 신규 18) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 표가 모든 기능을 선언 순서로 한 번씩 포함(`mapping_table_covers_every_feature_once_in_order`) | 통과 |
| Frame/Shape/Path/Image/clip 기본 기능은 vector, 비 vector 목록·gradient 대응 요소 고정(`required_basic_support_is_vector`) | 통과 |
| scene의 모든 variant 조합 fixture가 gradient(scene 미지원)를 뺀 모든 row에 도달(`every_scene_variant_maps_to_a_table_row`) | 통과 |
| node별 분류 정확, Text는 첫 미지원 기능(`classification_is_exact_per_node`, `basic_fixture_is_fully_vector`) | 통과 |
| 요소 표기 golden: fill·stroke(Center/Inside, Outside 거부)·clipPath chain·그룹·배경·image fit 4종·crop·sampling | 통과 |
| 숫자 round-trip·지수 없음, RFC 4648 base64 vector, safe id 규칙 | 통과 |

## 차이(작업 지시와 다른 점)

- gradient는 scene `Paint`에 variant가 없어 "기본 지원"을 대응 요소 명시(`NotInScene`)로 했습니다.
  scene에 gradient가 추가되면 분류 match가 컴파일 오류를 내므로 그때 vector row로 바꿉니다.
- `ImageFit::None`의 "원본 크기"는 scene·document 문서에 단위가 정의되어 있지 않아, FitMode 집합이
  대응하는 CSS `object-fit: none`의 의미(image pixel 1 = 단위 1, 중앙 정렬, 상자 밖 자름)로
  기록했습니다.
- 문서 수준 writer(그룹 순회, id, clip 정의 공유)는 SVG id 규칙과 함께 XX.3.3에서 구현합니다.

## 남은 위험 / 한계

- stroke join은 XI.6.2에서 아직 정의되지 않아 SVG 기본 miter(limit 4)에 맡깁니다. XI.6.2가 다른
  join을 정하면 표와 표기를 함께 바꿔야 합니다.
- Inside stroke를 열린 path에 쓰면 clip 영역은 path를 닫은 채우기 영역입니다(scene builder는 Inside를
  닫힌 상자형 content에만 씁니다).
- wide gamut 색은 CSS Color 4를 지원하지 않는 SVG 렌더러에서 표시되지 않을 수 있습니다.
- 선형 알파 합성(아키텍처 §10.3)과 일반 SVG 렌더러의 sRGB 합성 차이 허용 오차는 XX.3.4(선행
  XI.10.4 미완료)의 browser render fixture가 고정해야 합니다.
