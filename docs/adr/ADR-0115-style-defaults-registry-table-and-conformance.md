# ADR-0115: 기본값 세트 표를 registry longhand 전체로 넓히고, conformance fixture로 고정하며, modifier 삽입 순서를 하나로 합친다

## Status

Accepted (2026-11-23).

- ADR-0082(기본값 세트), ADR-0086(layout record와 해석 함수), ADR-0087(거울·출처)의 후속입니다. 그 문서들의 본문은
  고치지 않고 이 문서가 보완합니다.
- 계획서 행 IX.11.24·IX.11.27·IX.11.29·IX.11.31과, IX.11.15·IX.11.18이 남긴 modifier 삽입 순서 이중 규칙을 다룹니다.

## Context

- ADR-0086 결정 7은 세트 표를 layout longhand 40개에만 두었습니다. 당시 registry 이름 조회(`css::registry`)를 다른
  작업이 고치고 있었기 때문입니다. IX.11.24 완료 기준은 "모든 registry longhand에 두 세트의 값이 있음"입니다.
  - registry에는 layout longhand 말고도 CSS longhand인 payload 항목이 있습니다(`background-color`, 테두리 열두 개,
    `border-*-radius`, `transform`, `font-*`, `line-height`, `letter-spacing`, `stroke-linecap`, `fill-rule`,
    `object-fit`, `image-rendering`).
  - 이 값들은 지금 모델에서 언제나 명시 값입니다. 그래서 해석 함수는 이 표를 읽을 일이 없습니다. 그러나 IX.11.10·
    IX.11.11 보고서가 적었듯, payload를 희소 저장으로 바꾸는 schema step과 codegen 차이 출력에는 같은 값이 필요합니다.
- IX.11.31은 `css` 세트를 기준 브라우저 계산값으로, `react-native` 세트를 RN 문서·Yoga 동작으로 고정하고 fixture마다
  출처를 적으라고 합니다. 기존 표의 근거는 표 안 주석뿐이라, 표를 바꾸어도 실패하는 test가 없었습니다.
- 새 modifier를 넣을 위치 규칙이 둘이었습니다.
  - 선언 블록 붙여넣기(`commands::modifier_ops::css_insert_order`): CSS 고정 단계(속성 모델 9.1) 경계에 넣습니다.
  - 속성 패널(`commands::inspector` `paint_order`): Transform은 맨 앞, Fill은 앞쪽 Transform 뒤, 나머지는 맨 뒤.
  - 두 규칙은 같은 stack에서 다른 위치를 냅니다. 예: Stroke가 있는 node에 Fill을 넣으면 붙여넣기는 Stroke 뒤,
    속성 패널은 Stroke 앞입니다. 같은 편집이 입력 경로에 따라 다르게 그려집니다.
- CLI `stellar style-defaults set`은 `stellar.yaml`을 직접 바꾸었고 base revision을 보지 않았습니다(ADR-0086 결정 11).
  IX.11.29는 ops 쓰기 경로(base revision 검사, 교차 프로세스 저장 잠금)를 요구합니다.
  - `ops::apply`에 canonical `SetStyleDefaults` 하나를 넣으면 문서 레코드가 그대로여도 generation을 새로 씁니다
    (`design/current`, 새 `design/generations/*.starmanifest`). 그러면 Git diff가 `stellar.yaml` 한 줄이 아니라
    세 파일이 됩니다. ADR-0082 결정 4("Git diff에서 한 줄")와 ADR-0087 결정 2의 서술("세트만 바꾸는 apply는
    `stellar.yaml` 한 줄이 바뀝니다")에 어긋납니다. `ops::apply`는 이 작업의 소유 파일이 아닙니다.
- 제품 schema 1→2 묶음 step(IX.11.9 위치·transform, IX.11.10 테두리, IX.11.11 Text style 표)은 변환 함수가 있지만
  `PRODUCT_STEPS`에 없습니다.
  - scene·runtime은 아직 layout 결과로 그리지 않습니다(`crates/scene`은 `stellar-layout`에 의존하지 않음). 위치는
    Frame transform 이동 성분입니다.
  - 그래서 위치 step을 켜면 옮긴 Frame이 원점에 그려집니다.
  - Frame payload의 4변 테두리 타입 교체(`crates/document/src/frame.rs`)도 아직 없습니다.

## Decision

1. **payload longhand 기본값 표(IX.11.24).**
   - 표가 다루는 longhand는 layout longhand 40개(`LayoutLonghand::ALL`)와 payload longhand입니다. payload longhand는
     registry 항목 가운데 다음을 모두 만족하는 항목입니다(`style_defaults::payload_longhands`).
     - 범위가 node 종류 payload(Frame·Shape·Path·Text·Image)입니다. 공통 필드(`visible`·`locked`·`name`)와 modifier는
       longhand가 아닙니다. modifier는 stack 구조이고, 없으면 효과가 없습니다.
     - 묶음 key가 아니고, 매핑 상태가 `stellar-only`가 아니며, 이름이 그 항목의 CSS longhand 이름입니다.
     - 이름이 layout longhand가 아닙니다. payload 크기 `width`·`height`는 layout 표의 값을 씁니다.
   - 표 원본은 `style_defaults::payload_entries`이고 줄마다 (세트, registry 범위, key, 정규 값, 근거)입니다. 구획은
     registry 범위가 정합니다(Frame→`frame`, Shape·Path→`shape`, Text→`text`, Image→`image`).
   - 값은 registry 값 문법 formatter의 정규 표기입니다. 문법 parser가 받지 않는 값은 registry 매핑의 값 변형 규칙에
     같은 선언이 있을 때만 받습니다(`line-height: normal`). 검사 함수는 `check_payload_value`입니다.
   - 근거 두 개를 더합니다. `css-computed`(저자 스타일이 없을 때의 계산값이 initial과 다름: `border-*-width` 0,
     `border-*-color` 검정, `letter-spacing` 0)와 `react-native-source`(RN 구현 기본값, 문서 표에는 없음)입니다.
   - `react-native`가 바꾸는 payload 값은 `border-*-style: solid`, `font-family: system-ui`, `font-size: 14px`,
     `object-fit: cover`입니다. 나머지는 `css` 값을 물려받습니다.
   - 사용자 세트는 payload longhand를 두지 않습니다. 사용자 세트 codec(`crates/project` `manifest_style_defaults`)은
     layout longhand만 받고, payload 값은 사슬의 내장 세트 값입니다(`FlatSet::payload_value`). payload를 희소 저장으로
     바꿀 때 사용자 세트 key로 엽니다.
   - 해석 함수 `resolve`는 바뀌지 않습니다. payload 값이 언제나 명시 값이기 때문입니다.
   - 문서 표는 `docs/spec/style-defaults.md` 2.4절이고 `payload_snapshot_text()`와 같아야 합니다.
2. **conformance fixture(IX.11.31).**
   - fixture는 `crates/document/src/css/style_defaults_conformance.txt`입니다. 한 줄이 (세트, 구획, longhand, 기대값,
     관측값, 출처)이고, 두 세트 × 모든 구획 × 모든 layout longhand와 두 세트 × 모든 payload longhand가 정확히 한
     번씩 나옵니다.
   - `css` 줄의 관측값은 Chrome 157 headless에서 저자 스타일 없는 요소의 `computedStyleMap` 계산값입니다. 측정기는
     `style_defaults_conformance.py`입니다. 구획 대응은 `frame`=`div`, `text`=`span`, `image`=`img`,
     `shape`=`svg rect`(`fill-rule`은 `svg path`)입니다.
     - 관측값과 기대값이 같으면 출처는 `chrome:`입니다. 다르면 출처가 이유를 적습니다.
     - `stellar-mapping:`: Text·Image·Shape의 `display`는 `block`입니다. Stellar에는 inline formatting context가
       없습니다(ADR-0076 결정 3).
     - `css-computed:`: `border-*-width`(Typed OM은 `medium` 3px을 보이지만 style이 `none`이면 계산값 0),
       `letter-spacing`(`normal`은 0).
     - `user-agent:`: `font-family`는 OS 언어 설정이 정합니다(측정 환경은 한국어 시스템 글꼴). 이식 가능한 generic
       family `serif`를 둡니다.
   - `react-native` 줄은 관측값이 `-`이고 출처 접두사가 `rn-docs:`·`rn-source:`·`yoga:`·`inherits:` 중 하나입니다.
     `inherits:` 줄의 기대값은 `css` 값과 같아야 합니다.
   - test(`conformance_mismatches`)는 다음을 봅니다. 기대값이 registry 표 값과 같은지, 모든 줄에 출처가 있는지,
     빠지거나 겹친 줄이 없는지, 관측값과 출처가 맞는지입니다. 표 값을 하나 바꾸면 실패합니다.
   - RN 값은 Yoga를 실행해 얻지 않았습니다. 테스트 의존성에 Yoga를 넣지 않습니다. 출처는 문서·구현 기본값이고,
     실행 fixture는 후속입니다.
3. **modifier 삽입 순위 하나(IX.11.15·IX.11.18 후속).**
   - 새 modifier의 위치는 `modifier_ops::css_insert_order` 하나가 정합니다. 선언 블록 붙여넣기와 속성 패널이 모두 이
     함수를 쓰고, 속성 패널의 `paint_order`는 지웁니다.
   - 순위는 Transform(0) → Boolean·StrokeExpand·Corner(1) → Fill(2) → Stroke(3) → Shadow(4) → Blur(5) → Clip(6) →
     Mask(7) → Opacity(8) → Blend(9)입니다. 같거나 낮은 순위인 마지막 항목 뒤, 높은 순위인 첫 항목 앞에 넣습니다.
   - 근거는 다음과 같습니다.
     - 순위는 CSS 고정 단계(transform → paint → filter → clip-path → mask → opacity → mix-blend-mode)와 같은 방향으로만
       커집니다. 그래서 CSS 순서를 지키던 stack은 넣은 뒤에도 Web export에서 wrapper가 생기지 않습니다(IX.11.18).
     - paint 단계 안은 렌더 순서입니다. 도형을 바꾸는 modifier가 먼저, Fill이 Stroke 아래입니다. 속성 패널이 지키던
       Transform → Fill → Stroke가 그대로입니다.
     - Shadow는 앞 결과의 그림자이고 `filter` 목록에서 Blur 앞입니다.
   - 속성 패널이 맨 뒤에 붙이던 modifier(Shadow·Opacity 등)는 이제 순위 자리에 들어갑니다. 지금 속성 패널이 넣는
     modifier는 Transform·Fill·Stroke뿐이라 기존 결과는 같습니다.
4. **CLI `style-defaults set`(IX.11.29).**
   - `set <경로> <세트> [--base <revision>] [--dry-run]`입니다.
   - 순서는 ops 쓰기 경로와 같습니다. 교차 프로세스 저장 잠금(XXIV.4.5)을 기다리지 않고 잡고, 프로젝트를 다시 연 뒤
     base revision을 봅니다. 다르면 `base-revision-mismatch`입니다. `--base`가 없으면 지금 revision을 씁니다.
   - 쓰기는 ADR-0087 결정 2의 두 단계(`prepare_selection(root, 연 시점 이름, 세트)` → `commit`)입니다. 준비가 세트
     평탄화와 연 뒤 선택 변경을 거부하고, commit은 읽은 bytes hash가 그대로일 때만 씁니다. `SetStyleDefaults`의 영속
     결과가 `stellar.yaml` 한 줄이라 generation은 쓰지 않습니다.
   - `ops::apply`를 거치지 않는 이유는 Context의 generation 재기록 때문입니다. 문서 레코드가 바뀌지 않는 transaction의
     generation 저장을 건너뛰는 일은 `ops::apply` 소유 작업입니다. 그 뒤에는 CLI가 `ops::apply`의 canonical
     `SetStyleDefaults` 하나로 바꿀 수 있고, 결과 bytes는 같습니다.
   - `resolve`는 `ops::query::node_layout` 결과를 그대로 냅니다(ADR-0087 Consequences가 남긴 CLI 이중 경로 제거).
     `stellar query … node`의 `layout.values`와 같습니다.
   - `show`는 layout 표(`values`)와 payload 표(`payload`: `scope`, `key`, `class`, `longhand`, `value`)를 냅니다.
5. **제품 schema 1→2 step은 아직 등록하지 않습니다.**
   - 등록 조건은 두 가지입니다. scene·runtime이 layout 결과 위치로 그려야 하고(ADR-0086 Consequences, IX.11.25), Frame
     payload가 4변 테두리 타입으로 바뀌어야 합니다(IX.11.10).
   - 그 전에 켜면 옮긴 Frame이 원점에 그려지고, 테두리 step 결과를 저장할 payload 형식이 없습니다. 테스트가 통과해도
     제품 결과가 틀립니다.
   - 조건이 채워지면 세 step을 한 1→2 step으로 묶어 `PRODUCT_STEPS`에 넣고 `PRODUCT_SCHEMA_VERSION`을 2로 올립니다
     (ADR-0077 결정 4).

## Alternatives

| 대안 | 판단 |
|---|---|
| layout longhand를 `registry::entries()`에 넣음 | 채택하지 않음. 이름 조회(`entries_named`, `key_of`)를 쓰는 곳이 19곳이고, `width`처럼 범위마다 key가 다른 이름의 결과가 바뀝니다. 표는 registry 항목을 key로 참조합니다 |
| payload 표를 사용자 세트 key로도 받음 | 지금은 채택하지 않음. payload 값은 언제나 명시라 사용자 세트 값이 쓰일 곳이 없고, 사용자 세트 codec은 이 작업 소유 밖입니다 |
| fixture를 표에서 생성 | 채택하지 않음. 표를 바꾸면 fixture도 같이 바뀌어 실패하지 않습니다. 관측값은 브라우저에서, 기대값과 이유는 손으로 적습니다 |
| conformance test가 브라우저를 실행 | 채택하지 않음. 테스트 의존성이 됩니다. IX.11.8과 같이 측정 결과를 정적 파일로 둡니다 |
| 속성 패널 규칙(Transform 맨 앞, 나머지 맨 뒤)으로 합침 | 채택하지 않음. Opacity 뒤에 Shadow를 붙이는 등 CSS 순서를 깨 Web export wrapper가 생깁니다 |
| CLI `set`을 `ops::apply`로 | 지금은 채택하지 않음. 문서 내용이 같아도 generation 파일 두 개가 바뀝니다. `ops::apply`가 고쳐지면 바꿉니다 |
| 1→2 step을 지금 등록 | 채택하지 않음. Decision 5의 화면 위치 회귀가 생깁니다 |

## Consequences

- 모든 registry CSS longhand가 두 세트의 값을 갖습니다. payload 희소 저장, codegen 차이 출력, 조회가 같은 표를 씁니다.
- 표 값을 바꾸면 conformance test가 실패하고, 바꾼 이유를 fixture 출처에 적어야 합니다.
- 붙여넣기와 속성 패널이 같은 stack에 같은 위치로 modifier를 넣습니다.
- `stellar style-defaults set`이 base revision과 저장 잠금을 지키고, 바뀌는 파일은 `stellar.yaml` 하나, 한 줄입니다.
- 한계가 있습니다.
  - `ops::apply`의 mirror-only transaction이 generation을 새로 씁니다(소유 밖). 앱·`stellar apply`로 세트만 바꾸면
    Git diff가 세 파일입니다.
  - layout·scene 엔진의 생략값 해석(IX.11.25)은 이 작업 소유 밖입니다. `crates/layout`은 이미 풀린 선언 목록을
    받지만(ADR-0086 결정 8), scene·runtime은 layout 결과로 그리지 않습니다.
  - RN 값의 Yoga 실행 fixture가 없습니다.
  - codegen 차이 출력(`style_defaults::differences`)은 layout longhand만 봅니다. payload 차이(RN `font-size` 등)를
    내는 일은 codegen 소유 작업입니다.

## Compatibility

- 파일 포맷 변경은 없습니다. `stellar.yaml`, 사용자 세트 형식, chunk, command codec이 그대로입니다.
- 공개 API가 늘었습니다: `style_defaults::{payload_longhands, scope_class, payload_entries, PayloadEntry,
  check_payload_value, payload_snapshot_text, CONFORMANCE_FIXTURE, conformance_mismatches}`, `FlatSet::payload_value`,
  `Basis::{CssComputed, ReactNativeSource, ALL, from_name}`.
- CLI: `style-defaults set`에 `--base`·`--dry-run`과 결과 필드 `mode`·`base_revision`·`revision`이 더해졌습니다.
  `show` 결과에 `payload`가 더해졌습니다. 기존 필드는 그대로입니다. 없는 세트 오류 code도 그대로입니다.
- FFI 변경은 없습니다.

## Performance

- payload 표 조회는 사슬 길이(최대 내장 세트 두 개) × 표 줄 수(40)의 선형 탐색이고, 조회·CLI 경로에서만 씁니다.
  해석 함수(`resolve`)의 비용은 그대로입니다.
- 삽입 순위 계산은 기존과 같은 stack 한 번 훑기입니다.

## Security

- 측정기(`style_defaults_conformance.py`)는 개발 도구이고 빌드·테스트가 실행하지 않습니다. 표준 라이브러리만 쓰고
  임시 디렉터리의 HTML 한 장만 엽니다.
- CLI `set`은 기존 `stellar.yaml` 쓰기 guard(정규 파일, 읽은 hash 확인)를 그대로 씁니다.

## Migration

- 데이터 migration은 없습니다.
- 제품 schema 1→2 step 등록은 Decision 5의 조건이 채워진 뒤에 합니다.
