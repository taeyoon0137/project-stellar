# ADR-0082: 생략한 스타일 속성은 CSS 기본값으로 풀고, 프로젝트가 기본값 세트를 바꿀 수 있음

## Status

Accepted (2026-11-23). 제품 책임자 지시(2026-11-23)를 기록합니다. 지시 내용은 다음과 같습니다. "RN, CSS와 같은
프레임워크들을 쓸 수 있으니, 스타일링 기본값은 CSS로 하되 기본값을 프리셋처럼 바꿀 수 있도록 하자."

- ADR-0074 결정 6의 "생략값은 CSS initial value" 원칙을 보완합니다. 생략값은 여전히 CSS 값입니다. 다만 어떤 값으로
  푸는지를 고정 표가 아니라 프로젝트가 고른 기본값 세트가 정합니다.
- ADR-0074가 금지한 크기 방식 preset(고정·채움·내용 맞춤)과는 다른 개념입니다.
  - 크기 방식 preset은 longhand 여러 개를 한 번에 쓰고, longhand에서 preset을 거꾸로 판정합니다.
  - 기본값 세트는 어떤 속성도 쓰지 않습니다. 값을 정하지 않은 longhand가 무엇으로 풀리는지만 정합니다.
  - 이 문서는 혼동을 피하려고 "preset" 대신 "기본값 세트(style defaults)"라고 부릅니다.

## Context

- 같은 CSS longhand 어휘라도 대상 프레임워크마다 생략값이 다릅니다.
  - 브라우저에서 `div`는 `display: block`, `flex-direction: row`, `flex-shrink: 1`, `min-width: auto`,
    `box-sizing: content-box`, `position: static`입니다.
  - React Native(Yoga)의 View는 `display: flex`, `flex-direction: column`, `flex-shrink: 0`, `align-content: flex-start`,
    `min-width: 0`, `box-sizing: border-box`, `position: relative`처럼 동작합니다.
- 생략값을 CSS 하나로 고정하면 RN을 쓰는 팀은 모든 Frame에 `display: flex; flex-direction: column`을 명시해야 합니다.
  - 그러면 원본 파일에 같은 줄이 반복되고, 실제 코드와도 멀어집니다. RN 코드에는 그 줄이 없습니다.
  - 핵심 가치인 "실제 구현에 가까운 디자인"에 어긋납니다.
- 반대로 생략값을 프레임워크별로 엔진에 박아 두면 엔진과 codegen이 대상 수만큼 갈라집니다.
- Target Profile(`crates/target-profile`, IX.10)은 "무엇을 생성할 수 있는가"(capability)를 다룹니다. "생략한 값이
  무엇인가"는 다루지 않습니다. 한 프로젝트가 RN 기본값으로 디자인하면서 Web으로도 export할 수 있으므로, 두 축은
  서로 독립입니다.

## Decision

1. **값을 정하지 않은 longhand의 해석.** 원본(텍스트 원본, component source, chunk)은 사용자가 명시한 longhand만
   기록합니다. 명시하지 않은 longhand는 활성 기본값 세트의 값으로 풉니다.
   - 레이아웃, scene, 속성 탭, CLI·MCP 출력, codegen이 모두 같은 해석 함수 하나를 씁니다.
   - 엔진(`crates/layout`, `crates/scene`)에는 생략값을 하드코딩하지 않습니다. 엔진은 풀린 값만 입력으로 받습니다.
2. **내장 세트 두 개.**
   - `css`: 기본 세트입니다. 저자 스타일이 하나도 없을 때 브라우저가 계산하는 값입니다. 곧 CSS initial value에, 노드가
     대응하는 HTML 요소의 UA stylesheet 값을 더한 것입니다. Frame은 `div`이므로 `display: block`입니다.
   - `react-native`: React Native(Yoga) View·Text·Image의 기본 동작을 CSS longhand 값으로 적은 세트입니다.
   - 세트 표의 원본은 속성 registry(ADR-0075)의 longhand별 항목입니다. `docs/spec/style-defaults.md`가 표 전체를
     문서로 둡니다. RN 값은 RN 공식 문서와 Yoga 동작 fixture로 확인합니다. 확인하지 못한 항목은 `css` 값을 그대로
     물려받는다고 명시합니다.
3. **사용자 세트.** 프로젝트는 `design/style-defaults/<name>.stellar.yaml`에 세트를 정의할 수 있습니다.
   - 형식은 `extends: <세트 이름>` 한 줄과 `<css-longhand>: <CSS 값>` 줄(D2, 한 longhand가 한 줄)입니다.
   - 노드 종류별 구획(`frame:`, `text:`, `image:`, `shape:`)을 둘 수 있습니다.
   - `extends` 사슬은 내장 세트에서 끝나야 합니다. 순환, 모르는 longhand, 문법 오류, 토큰 참조(`var()`)는 거부합니다.
     기본값은 값 자체여야 하기 때문입니다.
4. **선택.** `stellar.yaml`의 `style_defaults: <세트 이름>`이 프로젝트 전체의 활성 세트를 정합니다. 키가 없으면 `css`입니다.
   - 세트를 바꾸는 것은 영속 command(`SetStyleDefaults`)입니다. Undo 한 번으로 되돌아가고, Git diff에서는 한 줄로 보입니다.
   - 사용자 세트 파일 편집은 일반 텍스트 원본 편집과 같은 경로(외부 변경 반영, ADR-0059)를 탑니다.
   - 문서·Page·컴포넌트 단위로 세트를 따로 두지 않습니다. 이유는 Alternatives를 봅니다.
5. **출처 표시.** 속성 조회 결과(`stellar query`, MCP, 속성 탭 snapshot)는 값마다 출처를 돌려줍니다.
   - 출처는 `explicit`(명시)이거나 `default(<세트 이름>)`(기본값)입니다.
   - 명시한 값이 세트 값과 같아도 명시로 남습니다. 저장 시 자동으로 지우지 않습니다.
   - 명시를 지우는 일(값 초기화)은 별도 command인 `UnsetProperty`입니다.
6. **codegen.** 생성기는 대상의 고유 기본값과 활성 세트의 차이를 명시 선언으로 냅니다.
   - Web export에서 활성 세트가 `react-native`이면, 내보내는 root 범위에 특이도가 0인 기반 규칙 하나를 둡니다
     (`:where(.<root>, .<root> *) { display: flex; flex-direction: column; … }`). 그래서 저자 규칙이 언제나 이깁니다.
   - RN export에서 활성 세트가 `css`이면, 차이가 나는 값(`flexDirection: 'row'`, `flexShrink: 1` 등)을 스타일 객체에
     명시합니다.
   - 세트와 대상 기본값이 같으면 아무것도 더하지 않습니다.
   - 따라서 어떤 세트로 디자인해도 결과 렌더링은 대상에서 같습니다.
7. **상속 규칙은 바꾸지 않습니다.** 세트는 초기값만 정합니다. 상속(CSS의 `color`·`font-*` 상속)은 CSS 규칙대로 둡니다.
   - RN의 "Text 밖으로 텍스트 스타일이 상속되지 않음"은 이번 범위에서 모델링하지 않습니다.
   - RN export가 필요한 곳에 값을 명시해 맞춥니다(결정 6).
8. **소유와 순서.** 이 결정은 별도 lane `style-defaults`로 구현합니다.
   - 선행 조건은 속성 registry(ADR-0075)와 CSS 레이아웃 엔진 재모델(ADR-0076)이 main에 통합되는 것입니다.
   - 그 lane이 할 일은 다음과 같습니다.
     - registry에 세트 표를 둡니다.
     - 해석 함수를 만들고 layout·scene에 연결합니다.
     - `stellar.yaml` 키, 사용자 세트 파일 codec, `SetStyleDefaults`·`UnsetProperty` command를 만듭니다.
     - CLI(`stellar style-defaults list|show|set`)와 MCP 읽기, codegen 차이 출력, conformance fixture를 만듭니다.
   - 셸 FFI는 UI 재설계 때 정합니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| 생략값을 CSS initial value로 고정(ADR-0074 원안) | 채택하지 않음. RN 팀의 원본에 같은 줄이 반복되고 실제 코드와 멀어집니다 |
| Target Profile이 기본값도 정함 | 채택하지 않음. RN 기본값으로 디자인하고 Web으로 export하는 경우처럼 두 축이 독립입니다. 묶으면 export 대상을 바꿀 때 디자인이 바뀝니다 |
| 새 노드를 만들 때 세트 값을 명시 속성으로 써 넣음 | 채택하지 않음. 원본에 반복 줄이 생기고, 세트를 바꿔도 기존 노드에 반영되지 않습니다 |
| 문서·Page·컴포넌트마다 세트를 따로 둠 | 지금은 채택하지 않음. 같은 컴포넌트가 놓인 곳에 따라 달리 풀려 결정성이 깨집니다. 여러 프레임워크용 라이브러리를 한 저장소에 두는 요구가 생기면 컴포넌트 라이브러리 단위로 다시 검토합니다 |
| 이름을 "preset"으로 함 | 채택하지 않음. ADR-0074가 금지한 크기 방식 preset과 혼동됩니다 |

## Consequences

- 원본에는 사용자가 정한 값만 남습니다. RN 팀과 Web 팀 모두 실제 코드와 같은 모양의 원본을 갖습니다.
- 세트를 바꾸면 명시하지 않은 모든 노드의 배치가 바뀝니다. 이것은 의도한 동작입니다. 그래서 세트 변경은
  undo 가능한 command이고 Git diff에서 한 줄로 보입니다.
- 엔진과 codegen이 생략값 표를 각자 갖지 않으므로, 이중 경로가 생기지 않습니다.
- 출처 표시가 생겨 조회 응답이 조금 커집니다. 출처는 고정 크기 enum이고 세트 이름은 세트 표 index로 줄여 씁니다.
- RN 세트 표가 실제 RN 동작과 어긋나면 디자인과 코드가 달라집니다. 그래서 conformance fixture로 표를 고정합니다.

## Compatibility

- 기존 프로젝트는 `style_defaults` 키가 없으므로 `css` 세트로 열립니다. 생략값 해석은 ADR-0074 원안과 같습니다.
- `stellar.yaml`에 새 선택 키가 생깁니다. 이 키를 모르는 옛 앱은 기존 매니페스트 규칙대로 모르는 키를 거부합니다.
  그래서 세트를 바꾼 프로젝트는 옛 앱에서 열리지 않으며, 진단에 필요한 앱 버전을 적습니다.
- 새 command kind 두 개(`SetStyleDefaults`, `UnsetProperty`)는 persistent-commands 대역 끝에 이어서 번호를 받습니다.
- FFI 변경은 없습니다.

## Performance

- 해석은 (노드 종류, longhand id) → 값 표 조회입니다. 활성 세트는 열 때와 바꿀 때 한 번 평탄화합니다.
  `extends` 사슬을 미리 풀어 둔 고정 배열입니다.
- 세트를 바꾸면 모든 노드의 layout invalidation이 한 번 일어납니다. 이것은 전체 재배치 예산 안에서 처리합니다.

## Security

- 사용자 세트 파일은 값만 담습니다. 실행 가능한 내용은 없습니다. 기존 텍스트 원본과 같은 크기·깊이 제한과 경로
  guard를 적용합니다.

## Migration

- 데이터 migration은 필요 없습니다. 기존 원본의 생략값은 `css` 세트 값과 같습니다.
- 구현 단계에서 layout 엔진의 하드코딩된 initial value를 세트 해석 함수로 옮깁니다. 하드코딩 값은 ADR-0076 lane이
  넣은 것입니다.
