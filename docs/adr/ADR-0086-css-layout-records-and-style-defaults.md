# ADR-0086: CSS layout longhand를 node별 희소 record로 저장하고 기본값 세트로 생략값을 푼다

## Status

Accepted (2026-11-23).

- IX.11.6(영속 layout record)의 저장 위치·key·형식 결정과, ADR-0082(기본값 세트)의 구현 결정을 함께 기록합니다.
- ADR-0082 본문은 고치지 않습니다. 이 문서가 구현 단계의 보완입니다.
- ADR-0077(텍스트 원본 v4)의 "모든 키 필수" 규칙을 결정 6으로 개정합니다.

## Context

- ADR-0074 결정 6과 ADR-0076은 레이아웃 입력을 CSS longhand로 정했습니다. 그러나 문서 모델에는 레이아웃 longhand를
  저장할 곳이 없었습니다.
  - Frame payload는 크기·transform·clip·배경·테두리·radius만 담습니다.
  - Frame이 아닌 node에는 위치 필드가 없습니다.
- ADR-0082는 "원본에는 명시 값만 쓰고, 명시하지 않은 longhand는 활성 기본값 세트로 푼다"고 정했습니다. 그러려면
  모델이 longhand마다 명시와 미설정을 구분해야 합니다.
- 제약이 세 가지 있습니다.
  - crate 그래프는 `stellar-document → stellar-layout` 의존을 허용하지 않습니다(`tools/xtask/src/crate_graph.rs`).
    문서 쪽 해석 결과를 엔진 타입으로 직접 만들 수 없습니다.
  - `NodeRecord`·`FramePayload`·`InsertNode`는 여러 crate에서 struct literal로 만듭니다(약 160곳). 필드를 더하면
    이 lane 밖의 파일을 대량으로 고쳐야 합니다.
  - 아직 어떤 제품 경로도 문서에서 레이아웃 입력을 만들지 않습니다. scene·runtime은 Frame transform·size를 직접
    그립니다.

## Decision

1. **저장 위치: node별 희소 layout record.**
   - `stellar_document::layout_record::LayoutRecord`는 node 하나의 명시 longhand와 값의 정렬된 map입니다.
   - `DocumentRecords::layout_records`에 modifier stack처럼 payload와 분리한 목록으로 둡니다. 빈 record는 저장하지
     않습니다.
   - item longhand(`position`, inset, 크기, `flex-*`, `align-self`·`justify-self`, `grid-*-start`·`-end`,
     `box-sizing`)는 상자를 가진 모든 node 종류에 붙습니다. container longhand(`display`, `padding-*`,
     `flex-direction`, `flex-wrap`, 정렬, gap, `grid-template-*`, `grid-auto-*`)는 Frame에만 붙습니다.
   - validator가 위반을 거부합니다. Fragment의 record, Frame이 아닌 node의 container longhand, 소유 node 없음,
     중복, 빈 record가 대상입니다.
   - lane 계획은 container longhand를 Frame payload 확장에 두려 했습니다. 그러나 다음 이유로 한 record에 둡니다.
     - 명시·미설정 구분에는 어차피 희소 map이 필요합니다.
     - item과 container가 같은 형식과 같은 해석 함수를 씁니다.
     - payload struct를 바꾸지 않아 다른 crate의 struct literal이 깨지지 않습니다.
2. **key(ADR-0024 결정 14).**
   - item longhand는 node 공통 범위 `0x0010`–`0x0025`입니다. `width`·`height`는 `0x0016`·`0x0017`입니다.
   - container longhand는 Frame 범위 `0x0110`–`0x0121`입니다.
   - Frame·Rectangle·Ellipse·Polygon·Image의 `width`·`height`는 payload 크기가 저장 위치입니다.
     - key는 IX.11.22·IX.11.3이 할당한 payload key(`0x0108`·`0x0109`, `0x0301`·`0x0302`, `0x0602`·`0x0603`)입니다.
     - 이 종류의 record에는 `width`·`height`를 둘 수 없습니다. 같은 값을 두 곳에 두지 않기 위해서입니다.
     - 그래서 이 종류의 크기는 지금 `<length>`만 됩니다. `auto`·백분율 크기는 payload 크기를 희소하게 바꾸는
       schema step(IX.11.21 묶음)에서 엽니다.
   - Text·Path·Line·ComponentInstance는 record의 `width`·`height`를 씁니다. node 종류마다 `width`·`height` key는
     하나씩입니다(`LayoutLonghand::key_for`).
3. **화면 위치는 바꾸지 않습니다.** record는 레이아웃 입력에만 쓰입니다. 기존 payload(Ellipse·Polygon bounds 원점,
   Frame transform 이동 성분)와 Transform modifier는 그대로입니다. 위치·transform 분리 migration(IX.11.9)이
   들어오기 전까지 record를 더해도 기존 렌더 결과는 같습니다.
4. **값과 형식.**
   - 값은 정규형 `LayoutValue`입니다(keyword, px 길이, 백분율, 수, grid line, span, 비율, track 목록).
   - CSS 텍스트의 수 표기는 `css::value::format_length` 등 기존 formatter를 씁니다. 해석의 수 부분도
     `css::value::parse`를 거칩니다. 그래서 `var()`, 계산식, 전역 keyword, 지원하지 않는 단위는 기존 값 parser와
     같은 code로 거부됩니다.
   - chunk 형식은 subtree 끝의 선택 절입니다. record 수 `u32`(1 이상) 뒤에 길이 접두 record를 소유 node 순서로
     씁니다. record가 없는 subtree는 이 절을 쓰지 않습니다. 그래서 기존 문서의 bytes와 storage golden이 그대로입니다.
   - 이 절을 모르는 옛 reader는 trailing bytes로 거부합니다. 조용히 잃지 않습니다.
   - schema version은 올리지 않습니다. IX.11.6·IX.11.9·IX.11.10·IX.11.11의 schema 변경은 공개 포맷 전에 1→2 한 번으로
     묶는다는 IX.11.21 규칙을 따릅니다.
5. **runtime 모델.**
   - `DocumentModel`은 record를 StableId로 보관합니다. node를 지워도 record는 남아 있다가, 같은 id의 node가
     돌아오면(삭제 undo) 다시 보입니다.
   - 조회(`layout_record`)와 영속 레코드(`to_records`)에는 살아 있는 node의 record만 나옵니다.
6. **텍스트 원본 v4 개정(ADR-0077).**
   - 문서 node는 `payload` 뒤에 선택 `layout` mapping을 둡니다. 한 줄이 longhand 하나이고, longhand 번호 순서이며,
     명시한 longhand만 씁니다. 명시 값이 없으면 키를 쓰지 않습니다. 읽을 때 없는 longhand는 미설정입니다.
   - Frame payload에서 CSS initial value가 곧 모델 기본값인 키는 읽을 때 생략을 허용합니다. `transform`,
     `background-color`, 테두리 열두 줄, `border-*-radius`, `layout-mode`, `clipping`이 해당합니다.
   - 쓰기는 payload 키를 모두 씁니다. payload 값은 모델에서 언제나 명시 값이기 때문입니다. `width`·`height`는
     생략할 수 없습니다.
   - 기존 v4 파일은 그대로 읽힙니다.
7. **기본값 세트 표와 해석 함수(ADR-0082 결정 1·2).**
   - 표 원본은 `stellar_document::css::style_defaults::builtin_entries`입니다. 문서 표는
     `docs/spec/style-defaults.md`이고 test가 둘이 같은지 확인합니다.
   - registry 이름 조회 함수(`css::registry`)는 다른 lane이 고치는 중이라, 세트 표는 `style_defaults` 파일에 둡니다.
     longhand 이름은 `layout_record`의 표 하나가 정하고 CSS longhand inventory(`css::mapping`)에 있어야 합니다.
   - 구획은 `frame`(Frame·ComponentInstance), `text`, `image`, `shape`(Shape·Path)입니다.
   - `css`는 CSS initial value입니다. 예외는 `display: block`(`div` UA)과 `justify-items: normal`(`legacy`와 같은
     결과)입니다.
   - `react-native`는 `css`를 물려받고 다음 값만 바꿉니다.
     - 모든 구획: `position: relative`, `box-sizing: border-box`, `flex-shrink: 0`, `min-width`·`min-height: 0px`
     - frame 구획: `display: flex`, `flex-direction: column`, `align-content: flex-start`
   - 해석 함수는 `style_defaults::resolve(set, payload, record)` 하나입니다.
     - 결과는 40개 longhand의 값과 출처(`explicit`·`default(<세트>)`)입니다.
     - payload 크기는 명시 값입니다. node 종류에 붙지 않는 longhand는 언제나 세트 값입니다.
   - 세트는 열 때 한 번 평탄화합니다(`flatten`). `extends` 사슬을 풀어 [구획][longhand] 고정 배열로 둡니다.
8. **엔진 연결.**
   - crate 그래프 때문에 해석 결과를 엔진 타입으로 넘기지 못합니다. 그래서 정규 CSS 텍스트 선언 목록
     (`ResolvedLayout::declarations`)을 넘깁니다.
   - `stellar_layout::declared::style_from_declarations`가 이 목록을 `LayoutStyle`로 1:1 옮깁니다. longhand가 하나라도
     빠지거나 겹치면 거부합니다. 엔진이 생략값을 채우는 경로는 없습니다.
   - 엔진 쪽 해석은 문서 formatter가 쓰는 정규형만 받습니다. 두 쪽이 같은 정규형인지는 공유 fixture
     (`crates/layout/tests/fixtures/declared_values.txt`)로 고정합니다.
     - 문서 test가 그 파일을 생성·검사합니다.
     - 엔진 test는 모든 줄을 받아야 합니다.
   - `LayoutStyle::default()`는 테스트·fixture·벤치 생성자로 남깁니다. 문서 경로는 이 함수를 쓰지 않습니다.
     `box-sizing` 값은 ADR-0076 엔진의 고정 값이던 `border-box`라 기존 fixture의 뜻이 바뀌지 않습니다.
   - 엔진에 `box-sizing`을 더합니다.
     - `content-box`는 `width`·`height`·`min-*`·`max-*`·`flex-basis`의 길이·백분율에 padding을 더해 border box로
       씁니다.
     - padding이 있는 `content-box` 상자의 `aspect-ratio`는 `Unsupported`로 거부합니다. 비율 변환이 border box
       기준이라 CSS와 같은 결과를 보장하지 못하기 때문입니다.
     - `flex-shrink`의 축소 비율은 CSS Flexbox §9.7대로 inner flex base size(padding 제외)로 셉니다. content-box
       fixture에서 드러난 기존 차이를 고칩니다.
   - `css` 세트 결과는 기준 브라우저 Chrome 157의 content-box fixture 10개(IX.11.8 corpus)와 같습니다.
9. **선택과 사용자 세트(ADR-0082 결정 3·4).**
   - `stellar.yaml`의 선택 최상위 키는 `style_defaults: <세트 이름>`입니다. 키가 없으면 `css`입니다.
   - 사용자 세트 파일 `design/style-defaults/<name>.stellar.yaml`의 형식은 다음과 같습니다.
     - `extends`(필수)
     - longhand 한 줄씩
     - 선택 구획 `frame`·`text`·`image`·`shape`
   - 거부 규칙: 순환, 없는 세트, 내장 이름, 이름 형식, 모르는 longhand·구획, 중복, 문법 오류, `var()`. 사슬은
     최대 16단계입니다.
10. **command(ADR-0079 대역 이어 받기, codec version 3).**
    - `SetStyleDefaults`(`0x0091`)는 활성 세트 이름을 바꿉니다. 역은 이전 이름으로 바꾸는 같은 command입니다.
    - `UnsetProperty`(`0x0092`)는 node의 명시 longhand 하나를 지웁니다. 역은 `SetLayoutProperty`입니다.
    - `SetLayoutProperty`(`0x0093`)는 명시 longhand 하나를 둡니다. 역은 이전 상태(값 또는 미설정)로 되돌리는
      `SetLayoutProperty` 또는 `UnsetProperty`입니다.
      - lane 계획은 command 두 개를 예정했습니다. 그러나 `UnsetProperty`의 역에는 값을 되돌리는 command가 필요합니다.
      - 기존 `SetProperty`의 값 타입(`TypedValue`)은 track 목록 같은 layout 값을 표현하지 못합니다.
      - 그래서 세 번째 번호를 씁니다.
    - 활성 세트 이름은 `DocumentModel`에 거울로 둡니다. 원본은 `stellar.yaml`이고, 문서를 여는 쪽이 넣어 주어야
      합니다. 이 거울이 있어야 command가 undo 한 번으로 되돌아갑니다.
    - 아직 거울을 채우고 되쓰는 경로가 없습니다. 문서를 여는 경로(`ops::snapshot`, runtime)는 거울을 `css`로 두고,
      저장 경로는 거울을 `stellar.yaml`에 쓰지 않습니다. 그래서 지금 `SetStyleDefaults`를 실행하는 제품 경로는 두지
      않습니다. CLI `style-defaults set`은 command 없이 `stellar.yaml` 한 줄을 직접 바꿉니다(결정 11). 연결은
      Consequences의 후속 작업입니다.
11. **조회 출처(ADR-0082 결정 5).**
    - 출처는 `stellar style-defaults resolve <프로젝트> <문서> <node>`가 longhand마다 돌려줍니다.
    - 세트 목록·내용·선택은 `stellar style-defaults list|show|set`입니다.
    - MCP 읽기 도구와 `stellar query` 기존 출력의 출처 열은 소유 crate(`mcp-core`, `ops::query`) 몫입니다. 이번 범위에서
      하지 않습니다.
12. **codegen 차이 출력(ADR-0082 결정 6).**
    - 출력은 이번에 하지 않고 codegen-css lane에 넘깁니다.
    - 해석 쪽은 `style_defaults::differences(active, target, 구획)`을 제공합니다. 이 함수는 대상 고유 기본값 세트와
      다른 longhand와 값을 돌려줍니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| `NodeRecord`에 공통 layout 필드, Frame payload에 container 필드 | 채택하지 않음. 명시·미설정 구분에 희소 map이 필요하고, struct literal 약 160곳이 깨집니다 |
| record에 `width`·`height`를 두고 payload 크기와 함께 씀 | 채택하지 않음. 같은 값이 두 곳에 생깁니다. payload 크기를 희소하게 바꾸는 schema step에서 한 곳으로 옮깁니다 |
| record 값을 CSS 텍스트로 저장 | 채택하지 않음. ADR-0024는 chunk에 CSS 텍스트를 저장하지 않습니다. 정규 typed 값과 binary tag를 씁니다 |
| crate 그래프에 `document → layout`을 더해 엔진 타입으로 직접 변환 | 채택하지 않음. 문서 crate가 엔진에 묶이고 crate 정책을 바꿔야 합니다. 정규 텍스트 선언 목록과 공유 fixture로 같은 정규형을 고정합니다 |
| 엔진 `LayoutStyle::default()`를 지움 | 지금은 채택하지 않음. 엔진·codegen·벤치 test 수백 곳이 생성자로 씁니다. 문서 경로가 이 함수를 쓰지 않는 것으로 하드코딩 해석을 없앱니다 |
| `UnsetProperty`의 역을 `RestoreNodeRecords`로 | 채택하지 않음. 그 command는 node 레코드만 되돌리고 layout record를 모릅니다 |

## Consequences

- 원본에는 명시한 layout longhand만 남습니다. 세트를 바꾸면 명시하지 않은 모든 node의 배치 입력이 바뀝니다.
- 엔진·codegen·CLI가 생략값 표를 따로 갖지 않습니다. 해석 함수 하나와 표 하나입니다.
- 한계가 있습니다. 후속 작업이 필요합니다.
  - `InsertNode`·`DuplicateSubtree`·`Paste`는 layout record를 옮기지 않습니다(commands crate의 node 작업 파일).
    - 복제·붙여넣기한 node는 명시 layout 값을 잃습니다.
    - 저장된 history journal에서 다시 연 삭제 undo도 layout record를 되살리지 못합니다.
  - semantic diff·merge snapshot은 아직 layout record를 투영하지 않습니다(semantic-diff lane).
    - Git 상태·stage에서 `layout` 줄만 바뀐 문서는 의미 변경이 없는 것으로 보입니다.
    - 외부 변경 반영(`ApplyExternalSnapshot`, ADR-0059)은 바뀐 `layout` 줄을 모델에 옮기지 않습니다. 앱이 그 뒤에
      저장하면 외부에서 고친 layout 값을 앱 모델 값으로 덮어쓸 수 있습니다.
  - 컴포넌트 원본(ADR-0061)과 컴포넌트 정의 node에는 layout record가 없습니다. 컴포넌트 root의 `display` 등을 둘 수
    없습니다.
  - `stellar apply`의 CommandTransaction JSON과 `stellar css paste`는 layout longhand를 받지 않습니다(`ops::apply`,
    `ops::css`). CLI·MCP로 layout 값을 바꾸는 길은 텍스트 원본 `layout` 줄을 직접 고치는 것뿐입니다.
  - 활성 세트 거울을 열 때 채우고 저장할 때 되쓰는 연결이 없습니다(결정 10).
  - scene·runtime은 아직 레이아웃 결과로 그리지 않습니다. 위치는 여전히 Frame transform입니다.

## Compatibility

- 기존 chunk·텍스트 원본은 그대로 읽히고, record가 없으면 bytes가 같습니다.
- record가 있는 chunk는 옛 앱이 trailing bytes로 거부합니다. `layout` 키가 있는 v4 파일은 옛 앱이 모르는 키로
  거부합니다.
- `stellar.yaml`의 `style_defaults` 키는 옛 앱이 모르는 키로 거부합니다(ADR-0082 Compatibility).
- command codec version이 3이 됩니다. version 2 journal은 그대로 읽힙니다.
- FFI 변경은 없습니다.

## Performance

- 해석은 (구획, longhand 순번) 고정 배열 조회입니다. record 조회는 node당 `BTreeMap` 하나입니다.
- layout 입력 canonical bytes에 `box-sizing` 한 byte가 더해집니다. layout corpus golden의 입력 hash만 바뀌고 결과
  digest는 그대로입니다.

## Security

- 사용자 세트 파일과 `layout` mapping은 값만 담습니다. 제한 YAML loader의 크기·깊이 제한과 경로 guard를 그대로
  씁니다.
- 값 해석은 기존 CSS 값 parser의 외부 리소스·`var()`·계산식 거부를 그대로 씁니다. track 목록은 256개까지입니다.

## Migration

- 데이터 migration은 없습니다. 기존 원본의 생략값은 `css` 세트 값과 같습니다.
  - 예외: 엔진 기본 `box-sizing`이 `border-box`이던 시기의 결과와 `css` 세트(`content-box`)는 padding이 있는
    상자에서 다릅니다.
  - 지금까지 문서에서 레이아웃 입력을 만드는 제품 경로가 없었으므로 저장된 결과는 바뀌지 않습니다.
- Frame 위치·transform 분리(IX.11.9)와 border longhand 분리(IX.11.10)는 IX.11.21 묶음 step으로 넣습니다.
