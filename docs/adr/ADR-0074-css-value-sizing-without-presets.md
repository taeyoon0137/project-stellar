# ADR-0074: 크기·정렬·간격은 CSS 값으로만 편집하고 레이아웃 엔진 입력도 CSS 값으로 다시 모델링

## Status

Accepted (2026-11-15). 제품 책임자 지시(2026-11-15 15:00, UX 참조 범위)를 기록합니다.

- ADR-0024 결정 7의 두 문단을 대체합니다. "`SizingMode`(Fixed·Fill·HugContent·Percentage)는 인스펙터 preset"
  문단과 "Constraint는 Stellar 확장" 문단입니다. ADR-0024의 나머지(CSS longhand 어휘, 값 문법, 레이아웃 의미는
  CSS를 따름, box-sizing, 위치와 transform 분리)는 그대로입니다.
- ADR-0024 Consequences의 "CSS가 직접 표현하지 않는 디자인 도구 개념(채움, 내용 맞춤)은 preset 계층이 필요합니다"도
  이 ADR로 폐기합니다.

## Context

- 제품 책임자는 Sketch·Figma의 조작 관례(도구 단축키, 선택·핸들, 캔버스 이동·확대, 레이어 다루기)만 참고하고, 두
  도구의 UI 기조와 설계 개념은 쓰지 않도록 지시했습니다. 금지 대상으로 명시한 것은 Auto Layout, 고정·채움·내용 맞춤
  크기 방식, 캔버스 padding·gap 간격 핸들, 3×3 정렬 격자 위젯, 그들 방식의 constraints·variant 패널입니다. 레이아웃은
  CSS 의미와 CSS 값 문법을 그대로 드러냅니다(UX 명세 1장 참조 범위, P4, 3.4, 6.4).
- 그런데 설계 문서와 계획에는 이 개념이 남아 있었습니다.
  - ADR-0024 결정 7: `SizingMode`를 인스펙터 preset으로 두고, preset과 longhand의 대응을 속성 모델 4.3절에 둠.
  - `docs/design/property_model.md` 4.3절: preset ↔ longhand 표, longhand가 표와 맞지 않으면 "Custom" 표시. 12장:
    "sizing은 preset", `justify-content`·`align-items`를 함께 바꾸는 3×3 위치 컨트롤, 캔버스 padding·gap 핸들.
  - `docs/design/ui/canvas_editing.md`: 원칙 3의 padding 핸들, 5.2절의 흐름 자식 Fixed·HugContent preset 전환과 변
    핸들 더블클릭, 7장 핸들 표의 padding·gap 행, 8장 간격·자리 바꾸기 핸들, 10.3절 padding·gap 핸들, 10.4절 크기 방식.
  - 계획: IX.11.7(sizing preset 변환 함수), IX.11.17·XII.10.9(preset 문구), XII.10.22(간격·자리 바꾸기 핸들),
    XII.10.25(padding·gap 캔버스 핸들), XII.10.43(padding 핸들 성능 scenario).
- 레이아웃 엔진(`crates/layout`)의 입력도 같은 개념입니다. 배치 방식은 Free·Stack·Grid·Overlay·Constraint이고 축 크기는
  `SizingMode::{Fixed, Fill, HugContent, Percentage}`입니다(`crates/layout/src/sizing.rs`). 지금 이 crate를 쓰는 곳은
  `crates/codegen` 하나이고(`crates/codegen/Cargo.toml`), UI·CLI·MCP·저장 형식에는 노출되지 않습니다. 영속 layout
  record(IX.11.6)는 아직 없습니다.

## Decision

1. **크기는 CSS 값을 그대로 입력·표시합니다.** `width`·`height`는 `<length>`(`320px`), `<percentage>`(`100%`),
   `auto`, `fit-content`, `min-content`, `max-content` 같은 CSS 값입니다. flex item은 `flex-grow`·`flex-shrink`·
   `flex-basis`·`align-self`를 각각 한 행으로 편집합니다(longhand 한 속성 = 한 줄, D2). preset 계층, longhand에서
   preset을 거꾸로 판정하는 일, "Custom" 표시는 두지 않습니다. 화면·문서·CLI·MCP 출력에 고정·채움·내용 맞춤
   (Fixed·Fill·Hug) 같은 크기 방식 이름을 쓰지 않습니다. 선언값과 계산값이 다르면(예: `flex-grow`가 남은 공간을 채움)
   속성 탭이 계산값을 함께 보여 줍니다(WBS UX.8.2).
2. **정렬은 longhand마다 CSS 키워드 하나를 고릅니다.** `justify-content`·`align-items`·`align-content`·
   `justify-items`·`align-self`는 각각 한 행입니다. 두 값을 함께 바꾸는 위치 격자 위젯(3×3)은 두지 않습니다.
3. **padding·gap은 속성 탭의 CSS 값으로만 편집합니다.** 캔버스 padding·gap 핸들과, 등간격 다중 선택의 간격·자리
   바꾸기 핸들은 두지 않습니다. 캔버스 직접 조작은 크기·위치·반경·순서와 흐름 컨테이너 안 삽입 위치까지입니다.
4. **캔버스 크기 핸들은 크기 방식을 바꾸지 않습니다.** 흐름 자식에서도 핸들은 `width`·`height`에 px 값을 쓰고
   `flex-*`·`align-self`는 바꾸지 않습니다. 변 핸들 더블클릭으로 `fit-content`로 바꾸는 숨은 전환도 두지 않습니다.
5. **부모 기준 고정은 CSS positioning으로만 표현합니다.** `position: absolute`와 inset(`left`·`right`·`top`·`bottom`)
   이 그 어휘입니다. 별도 Constraint 방식이나 constraints 패널은 두지 않습니다. 형제·baseline 기준 배치는 CSS anchor
   positioning이 Web Target Profile capability로 확정되기 전에는 범위 밖입니다.
6. **레이아웃 엔진 입력을 CSS 값으로 다시 모델링합니다.** 엔진 입력은 `display: flex | grid`, `flex-direction`·
   `flex-wrap`, `flex-*` longhand, `width`·`height`의 keyword·`%`·px, `justify-*`·`align-*`, `position`과 inset,
   `grid-template-*`·`grid-*-start`·`-end`입니다. Stack·Overlay·Constraint·`SizingMode`·`FlexItem` 같은 엔진 고유
   어휘는 엔진을 CSS 의미에 맞추는 행(IX.11.4 Flexbox, IX.11.5 Grid·positioning)에서 CSS 값 입력으로 바꾸고, 영속
   layout record(IX.11.6)는 CSS longhand만 저장합니다. 그때까지 기존 타입은 `crates/layout` 안의 내부 표현이고, UI·
   사용자 문서·CLI·MCP·저장 형식에 노출하지 않습니다. 속성 모델 4장의 엔진 값 ↔ CSS longhand 표는 이 다시 모델링의
   변환 근거로만 남깁니다.
7. **문서와 계획 정리.** 속성 모델 4.3절은 엔진 내부 값의 변환 표로 바꾸고, 12장의 preset·3×3·캔버스 간격 핸들
   문단을 이 결정으로 고칩니다. 캔버스 편집 명세의 해당 절(원칙 3, 5.2, 7장, 8장, 10.3, 10.4, Undo·성능 표)도 고칩니다.
   계획 행의 개정(IX.11.7 삭제, IX.11.17·XII.10.9 문구, XII.10.22·XII.10.25 삭제, XII.10.43의 padding 핸들 scenario
   삭제)은 계획서를 직접 고치지 않고 UX WBS의 UX.0.3이 합니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| preset을 두고 선택지 문구만 CSS 값으로 보임(ADR-0024 결정 7 원안) | 채택하지 않음. 보이는 문구가 CSS 값이어도 longhand 여러 개를 한 번에 쓰고 역판정·Custom 상태를 가지는 크기 방식 계층이 그대로 남습니다. 사용자가 금지한 개념입니다 |
| 엔진 입력은 지금 어휘를 영구 내부 표현으로 두고 UI·저장만 CSS로 | 채택하지 않음. 영속 record(CSS longhand)와 엔진 입력 사이에 두 번째 모델과 변환 계층이 영구히 생기고, 엔진 진단·문서가 엔진 어휘로 쓰입니다 |
| 캔버스 padding·gap 핸들을 CSS longhand만 바꾸게 유지 | 채택하지 않음. 사용자가 금지한 간격 핸들 개념이고, 같은 값을 속성 탭에서 CSS 값으로 정확히 편집할 수 있습니다 |

## Consequences

- 흐름 자식의 크기를 바꾸려면 속성 탭에서 CSS 값을 입력·고릅니다. 한 번에 여러 longhand를 바꾸는 바로가기는 없습니다.
- 엔진을 CSS 의미에 맞추는 IX.11.4·IX.11.5의 범위가 엔진 입력 모델 교체까지 넓어집니다. IX.11.6 record는 CSS
  longhand 그대로라 엔진과 record 사이에 크기 방식 변환이 없습니다.
- 계획 IX.11.7은 할 일이 없어지고, XII.10.22·XII.10.25와 그 검증·성능 scenario가 빠집니다(UX.0.3).
- 기존 엔진 테스트와 codegen은 내부 표현을 바꿀 때 함께 바뀝니다.

## Compatibility

- 파일 바이트, 공개 API, FFI를 바꾸지 않습니다. 영속 layout record가 아직 없어 저장 형식 영향이 없습니다.

## Performance

- 영향 없음. 엔진 입력 모델 교체의 성능은 IX.11.4·IX.11.5에서 기존 layout 예산(아키텍처 §6)으로 확인합니다.

## Security

- 영향 없음.

## Migration

- 저장된 layout 데이터가 없으므로 문서 migration은 없습니다. 엔진 내부 표현 교체는 IX.11.4·IX.11.5에서 엔진 테스트와
  함께 합니다.
