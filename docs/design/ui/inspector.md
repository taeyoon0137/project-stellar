# 인스펙터 UI 명세

- 상태: 초안(2026-11-14), Stub 데이터 기준 구현
- 범위: macOS 셸 오른쪽 인스펙터 pane의 내용(선택 종류별 속성 섹션, modifier stack, Value Binding 표시 자리)
- 코드: `apps/macos/StellarMac/UI/Inspector/`
- 상위 기준: `docs/design/macos-shell-design.md`(이하 "가이드"), `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`(이하 "아키텍처")

근거 표기는 가이드 0.1절을 따릅니다. `[가이드 n]`은 가이드의 절, `[아키텍처 §n]`은 아키텍처 명세의 절, `[HIG <주제>]`는 Apple Human Interface Guidelines의 해당 주제, `[계획 n]`은 구현 계획의 해당 항목입니다.

이 문서는 가이드·아키텍처·계획을 바꾸지 않습니다. 어긋나면 그 문서들이 우선합니다. [가이드 0.2]

---

## 1. 역할과 위치

- 인스펙터는 선택 대상의 속성, Value Binding, Component Property·Slot, modifier stack을 보여 주고 편집 명령을 코어로 보냅니다. 문서 의미·검증·Undo 그룹화는 코어가 맡습니다. [가이드 1.2 원칙 4, 6.3] [아키텍처 §8.1, §8.2]
- 진입점은 `InspectorViewController`입니다. 통합(XII.9.1) 시 `NSSplitViewItem(inspectorWithViewController:)`에 넣습니다. pane 재질은 시스템이 정하므로 인스펙터는 배경을 그리지 않습니다. [가이드 2.3, 3.2]
- 데이터는 `InspectorDataSource`가 주는 값 snapshot으로만 받고, 편집은 `InspectorCommand`로만 보냅니다(7장). 코어 연결 전에는 `InspectorStubDataSource`를 씁니다. 이름의 `Stub`은 의도적 placeholder 표시입니다. [계획 1.2 규칙 6]
- 앱 UI 문자열은 기존 셸과 같이 영어로 둡니다. 현지화는 이 작업 범위 밖입니다.

## 2. 화면 구성

위에서 아래로 다음 순서입니다.

| 영역 | 내용 | 구현 |
|---|---|---|
| 선택 요약 | 종류 아이콘, 레이어 이름(다중 선택은 "N Layers"), 종류 이름 | `InspectorHeaderView` |
| 잠금·읽기 전용 안내 | 잠금이면 안내 + Unlock 버튼, 읽기 전용이면 이유 | `InspectorBannerView` |
| 속성 섹션 | 선택 종류별 섹션(2.1절). 섹션마다 disclosure로 접기·펼치기 | `InspectorSectionView` + `InspectorRowsView` |
| Modifier 섹션 | modifier 목록(`NSTableView`) + 추가·삭제 + 선택 항목 파라미터 | `InspectorModifierStackController` |

- 한 줄(행)은 "레이블 열 + 필드 컨트롤 + 진단 아이콘(있을 때) + Value Binding 표시"입니다. 여러 값이 한 줄에 놓이면(예: W·H) 짧은 레이블을 앞에 붙입니다. [HIG Layout]
- 섹션은 disclosure 버튼과 제목 클릭으로 접고 폅니다. 접힘 상태는 창별 UI 상태이며 문서·Git에 넣지 않습니다. [가이드 2.6, 6.3] [HIG Disclosure controls]

### 2.1 선택 종류별 섹션

| 선택 | 섹션(순서) | 근거 |
|---|---|---|
| 선택 없음·로딩·오류 | 상태 화면(3장) | [가이드 7.1] |
| Frame | Layer · Frame · Appearance · Modifiers | `frame.rs`, `frame_metadata.rs` |
| Fragment | Layer · Fragment(안내 + Convert to Frame). Modifier 섹션 없음 | `fragment.rs`, `modifier.rs` `can_own_modifiers` [아키텍처 §12.4] |
| Shape(Rectangle·Ellipse·Line·Polygon) | Layer · 도형 섹션(제목은 도형 종류) · Modifiers | `shape.rs` [아키텍처 §12.2] |
| Path | Layer · Path · Modifiers | `path.rs` |
| Text | Layer · Text · Character · Paragraph · Modifiers | `text.rs` |
| Image | Layer · Image · Modifiers | `image.rs` |
| ComponentInstance | Layer · Component · Properties · Slots · Modifiers | `component/instance.rs`, `component/slot.rs` [아키텍처 §15] |
| 같은 종류 다중 선택 | 단일 선택과 같은 섹션, 값이 다르면 혼합(4장). Modifier 섹션은 안내만 | [아키텍처 §12.6] |
| 다른 종류 다중 선택 | Layer · Modifiers(안내만) | 공통 필드만 편집 |

섹션 구성은 `InspectorPropertyCatalog`가 코드 원본입니다. 코어 adapter도 같은 카탈로그 템플릿에 값만 채웁니다(7.3절).

## 3. 상태

### 3.1 내용 상태(`InspectorContentState`)

| 상태 | 화면 | 동작 |
|---|---|---|
| `noSelection` | 아이콘 `cursorarrow.rays`, "No Selection", 선택 방법 안내 | 없음 |
| `loading` | 작은 spinning `NSProgressIndicator`, "Loading Properties…" | 없음 |
| `failed(message)` | 아이콘 `exclamationmark.triangle`, "Can’t Show Properties", 메시지 | "Try Again" → `.reload` |
| `ready` | 2장의 페이지 | — |

- 가이드 7.1은 빈 상태에 다음 동작 버튼을 두라고 합니다. 선택은 캔버스·레이어 목록에서 하므로 인스펙터 안에는 실행할 동작이 없어, 선택 없음 화면은 안내만 둡니다. 통합 시 이 판단을 다시 확인합니다. [가이드 7.1] [HIG Writing]

### 3.2 편집 가능 상태(`InspectorEditability`)

| 상태 | 표시 | 편집 |
|---|---|---|
| `editable` | 안내 없음 | 필드별 `isEditable`을 따름 |
| `locked` | `lock.fill` + "This layer is locked…" + Unlock 버튼 | 코어가 필드별 `isEditable=false`로 보냄. 이름·표시·잠금 필드와 Unlock은 편집 가능 |
| `readOnly(reason)` | `pencil.slash` + 이유(예: 신뢰 전 제한 모드) | 셸이 모든 필드·modifier 조작을 강제로 비활성화(Value Binding 표시는 보기만 가능) |

- 비활성 컨트롤은 숨기지 않고 비활성으로 보입니다. [가이드 8.1 "숨기지 않고 비활성화"] [아키텍처 §24.3]

### 3.3 그 밖의 상태

- 필드 값 없음(`.none`): 읽기 전용 값은 "—", 입력 필드는 빈 값 + placeholder "—".
- 진단(`InspectorRow.issue`): 행 끝에 `exclamationmark.triangle.fill` 아이콘, 툴팁·VoiceOver에 메시지. 미해결 참조(clip Path, 누락 asset)를 invalid가 아니라 진단으로 보여 줍니다. [아키텍처 §14.4] (`frame.rs` `resolve_clip`, `image.rs` `resolve_asset`)
- 의존 필드: 켜고 끄는 optional 값(Frame background·border, Text fill)은 스위치가 꺼지면 색·두께 필드가 비활성입니다. 판단은 코어가 `isEditable`로 보냅니다.

## 4. 혼합 값(다중 선택)

선택 대상의 값이 서로 다르면 snapshot 값이 `.mixed`입니다. 컨트롤별 표시는 다음과 같습니다. [HIG Toggles, HIG Pop-up buttons]

| 컨트롤 | 혼합 표시 | 편집 시 |
|---|---|---|
| 수치 `NSTextField` | 빈 값 + placeholder "Mixed" | 입력값을 모두에 설정. 화살표 키·stepper는 `.adjustNumber`(node별 상대 조정) |
| 텍스트 `NSTextField` | 빈 값 + placeholder "Mixed" | 빈 채로 확정하면 명령 없음 |
| `NSSwitch` | off + 옆에 혼합 표시 "–"(툴팁 "Mixed"), 접근성 값 "Mixed" | 누르면 모두 on |
| `NSColorWell` | 기본 색 + 옆에 "–"(툴팁 "Mixed"), 접근성 값 "Mixed" | 고른 색을 모두에 설정 |
| `NSPopUpButton` | 맨 위에 비활성 "Mixed" 항목을 넣고 선택 | 고른 항목을 모두에 설정 |
| `NSSegmentedControl` | 선택 없음, 접근성 값 "Mixed" | 고른 항목을 모두에 설정 |
| 읽기 전용 | "Mixed" | — |

- `NSSwitch`와 `NSColorWell`은 혼합 상태를 직접 그릴 수 없어 옆에 macOS 혼합 상태 관례인 대시를 둡니다. 좁은 폭(300pt)에서 "Mixed" 글자를 두면 Border 행이 넘쳐 대시를 택했습니다.

## 5. 속성 → 컨트롤 대응표

문서 모델(`crates/document/src`)의 편집 가능한 속성과 컨트롤의 대응입니다. "경로"는 `InspectorPropertyID.path`, "key"는 문서 모델 `PropertyKey`(정해진 것만)입니다. 단위는 표시 단위이며, 표시 값 = 모델 값 × 배율입니다. "B"는 Value Binding 대상(행 끝 표시)입니다. [아키텍처 §11.2, §14.2] [가이드 5.1, 5.2]

### 5.1 공통 레이어(`records.rs` `NodeRecord`)

| 속성 | 경로 | 모델 | 컨트롤 | 단위·범위 | B |
|---|---|---|---|---|---|
| 이름 | `layer.name` | `display_name: String` | `NSTextField`(한 줄) | — | 아니오 |
| 표시 | `layer.visible` | `visible: bool` | `NSSwitch` | — | 예 |
| 잠금 | `layer.locked` | `locked: bool` | `NSSwitch` | — | 아니오 |

### 5.2 Frame(`frame.rs` `FramePayload`, `frame_metadata.rs`)

| 속성 | 경로 | 모델(key) | 컨트롤 | 단위·범위 | B |
|---|---|---|---|---|---|
| 위치 X·Y | `frame.transform.x/y` | `transform: Transform2D`의 이동(0x0102) | 수치 필드 ×2 | px | 예 |
| 크기 W·H | `frame.size.width/height` | `size: Size2`(0x0101) | 수치 필드 ×2 | px, ≥ 0 | 예 |
| 회전 | `frame.transform.rotation` | `transform` 회전 성분(0x0102) | 수치 필드 + `NSStepper` | °, 소수 1자리 | 예 |
| 좌우·상하 반전 | `frame.transform.flipHorizontal/Vertical` | `mirror_horizontally/vertically`(0x0102) | 아이콘 `NSButton` ×2 | — | 아니오 |
| Clip | `frame.clipping` | `Clipping` Disabled/FrameBounds/Path(0x0103) | `NSPopUpButton`(None·Frame Bounds·Path) | Path 미해결이면 진단 | 예 |
| Layout | `frame.layoutMode` | `layout_mode: Option<StableId>`(0x0107) | `NSPopUpButton`(None + 문서 layout 정의) | — | 예 |
| 배경 사용 | `frame.background.enabled` | `background: Option<Color>`(0x0104) | `NSSwitch` | — | 아니오 |
| 배경 색 | `frame.background.color` | 같음 | `NSColorWell`(minimal, 알파) | sRGB·Display P3 | 예 |
| 테두리 사용·색·두께 | `frame.border.enabled/color/thickness` | `border: Option<Border>`(0x0105) | `NSSwitch` + `NSColorWell` + 수치 필드 | px, ≥ 0 | 색·두께 예 |
| 모서리 4개 | `frame.cornerRadius.topLeft…bottomRight` | `CornerRadius`(0x0106) | 수치 필드 ×4(2행, TL·TR·BL·BR) | px, ≥ 0 | 예 |

- `transform`은 아핀 행렬입니다. 위치·회전 값은 코어가 분해해 보내고, 편집 명령을 받아 다시 합성합니다. 셸은 행렬을 계산하지 않습니다. 기울임(skew)은 편집하지 않습니다. [가이드 1.2 원칙 4]
- 음수 크기는 거부합니다(반전은 transform). 셸은 하한 0으로 clamp만 하고 최종 검증은 코어가 합니다. (`frame.rs` VI.3.3)

### 5.3 Fragment(`fragment.rs`)

| 속성 | 경로 | 모델 | 컨트롤 | 비고 |
|---|---|---|---|---|
| Frame으로 변환 | `fragment.convertToFrame` | VI.4.4 변환 command | `NSButton` | Fragment는 시각·layout 속성과 modifier가 없음(안내 문구) [아키텍처 §12.4] |

### 5.4 Shape(`shape.rs` `ShapeGeometry`)

| 도형 | 속성 | 경로 | 모델 | 컨트롤 | 단위·범위 |
|---|---|---|---|---|---|
| Rectangle | 크기 W·H | `shape.rectangle.size.*` | `RectangleParams.size` | 수치 필드 ×2 | px, ≥ 0 |
| Rectangle | 모서리 4개 | `shape.rectangle.cornerRadius.*` | `RectangleParams.corner_radius`(원본 값, 평가 시 clamp) | 수치 필드 ×4 | px, ≥ 0 |
| Ellipse | 위치 X·Y, 크기 W·H | `shape.ellipse.bounds.*` | `EllipseParams.bounds: Rect` | 수치 필드 ×4 | px |
| Line | 시작 X·Y, 끝 X·Y | `shape.line.start.*`, `shape.line.end.*` | `LineParams.start/end: Vec2` | 수치 필드 ×4 | px |
| Line | 끝 모양 | `shape.line.cap` | `LineCap` Butt/Round/Square | `NSSegmentedControl` | — |
| Polygon | 위치·크기 | `shape.polygon.bounds.*` | `PolygonParams.bounds` | 수치 필드 ×4 | px |
| Polygon | 변 수 | `shape.polygon.sides` | `sides: u32` | 수치 필드 + `NSStepper` | 정수 3–1000 |
| Polygon | 회전 | `shape.polygon.rotation` | `rotation_degrees` | 수치 필드 + `NSStepper` | ° |
| Polygon | 모서리 | `shape.polygon.cornerRadius` | `corner_radius` | 수치 필드 + `NSStepper` | px, ≥ 0 |
| 공통 | Path로 변환 | `shape.convertToPath` | `plan_shape_to_path`(명시적 파괴 변환) | `NSButton` | [아키텍처 §12.6] |

- 도형 종류 자체는 바꾸지 않습니다(문서 모델에 종류 변경 command가 없음). Rectangle payload에는 위치가 없어 위치 행을 두지 않습니다.
- Shape·Path·Text 속성은 문서 모델에 `PropertyKey`가 아직 없어 key 없이 경로만 둡니다(9장).

### 5.5 Path(`path.rs`)

| 속성 | 경로 | 모델 | 컨트롤 |
|---|---|---|---|
| 채우기 규칙 | `path.winding` | `WindingRule` NonZero/EvenOdd | `NSSegmentedControl` |
| 점 수·subpath 수 | `path.pointCount`, `path.subpathCount` | `IdentifiedPath`, `PathData::subpaths` | 읽기 전용 |
| 경계 X·Y·W·H | `path.bounds.*` | `path_bounds` | 읽기 전용 |

- 점·핸들 편집은 캔버스에서 합니다(안내 문구). [가이드 1.2 원칙 2]

### 5.6 Text(`text.rs`)

| 속성 | 경로 | 모델 | 컨트롤 | 단위·범위 | B |
|---|---|---|---|---|---|
| 내용 | `text.content` | `TextPayload.content: UserText` | 읽기 전용 + "Edit on Canvas" 버튼 | — | 아니오 |
| Typography | `text.style.typography` | `TextStyle.typography: Option<StableId>` | `NSPopUpButton`(Default + 문서 typography) | — | 예 |
| 글자 채우기 | `text.style.fill.enabled/color` | `TextStyle.fill: Option<Color>`(없으면 상속) | `NSSwitch` + `NSColorWell` | — | 색 예 |
| OpenType | `text.style.features` | `features`, `variations` | 읽기 전용 요약 | — | 아니오 |
| 정렬 | `text.paragraph.alignment` | `TextAlignment` Start/Center/End/Justify | `NSSegmentedControl`(아이콘) | — | 예 |
| 줄 높이 방식·값 | `text.paragraph.lineHeight.mode/value` | `LineHeight` Auto/Multiplier/Fixed | `NSPopUpButton` + 수치 필드 | ×(0–10) 또는 px(0–10000), Auto면 값 비활성 | 예 |
| 자간 | `text.paragraph.letterSpacing` | `letter_spacing_em` | 수치 필드 + `NSStepper` | em, −1–10 | 예 |
| 줄바꿈 | `text.paragraph.wrapping` | `WrappingMode` Wrap/NoWrap | `NSSegmentedControl` | — | 예 |

- 캔버스 Text 내용은 인스펙터 텍스트 필드로 편집하지 않습니다. 공통 editing session이 맡습니다. [아키텍처 §11.1] [가이드 5.2]
- 문자 style은 선택 범위의 style run 기준입니다. 범위가 여러 run에 걸치면 혼합입니다.

### 5.7 Image(`image.rs` `ImagePayload`)

| 속성 | 경로 | 모델(key) | 컨트롤 | 단위·범위 |
|---|---|---|---|---|
| Asset | `image.asset` | `asset: AssetRef`(0x0601) | 읽기 전용 + 누락 진단 | — |
| 크기 W·H | `image.size.*` | `size: Size2` | 수치 필드 ×2 | px, ≥ 0 |
| Fit | `image.fit` | `FitMode` Fill/Contain/Cover/None | `NSPopUpButton` | — |
| Crop X·Y·W·H | `image.crop.*` | `CropRect`(정규화 0–1) | 수치 필드 ×4 | %(×100), 0–100 |
| Sampling | `image.sampling` | `SamplingMode` Linear/Nearest | `NSSegmentedControl` | — |
| 색공간 | `image.colorSpace` | `color_space_override: Option<ColorSpace>` | `NSPopUpButton`(Asset Default·sRGB·Display P3) | — |

- Crop 범위 위반(x+w > 1 등)은 코어가 거부합니다(`CropRect::from_components`). 셸은 각 값 0–1 clamp만 합니다.

### 5.8 ComponentInstance(`component/instance.rs`, `definition.rs`, `slot.rs`)

| 속성 | 경로 | 모델(key) | 컨트롤 |
|---|---|---|---|
| 정의 | `component.definition` | `ComponentRef`(0x0A01) | 읽기 전용 |
| Detach | `component.detach` | `detach_instance`(IX.2.6) | `NSButton` |
| 공개 Property | `component.property.<id>` | `property_overrides`(0x0A02), 값 `TypedValue` | 아래 표 |
| 공개 Slot | `component.slot.<id>` | `slot_overrides`(0x0A03), `SlotCardinality` | 읽기 전용 요약(개수·범위) |

`TypedValue` → 컨트롤:

| TypedValue | 컨트롤 | 단위 |
|---|---|---|
| Boolean | `NSSwitch` | — |
| Number | 수치 필드 | 없음 |
| String | `NSTextField` | — |
| Enum | `NSPopUpButton` | — |
| Length | 수치 필드 | px |
| Percentage | 수치 필드 | % |
| Angle | 수치 필드 + `NSStepper` | ° |
| Duration | 수치 필드 | ms |
| Color | `NSColorWell` | — |
| Gradient·Shadow·Typography·ResourceRef | 읽기 전용 요약(전용 편집기는 후속) | — |

- 인스턴스 편집은 공개 Property·허용 Override·공개 Slot으로 제한하고 자유 편집은 Detach로만 엽니다. [아키텍처 §15.1, §15.2] [가이드 6.3]

### 5.9 Modifier(`modifier.rs`, `modifier_metadata.rs`)

| 종류 | 파라미터 | 경로(`modifier.<id>.…`) | key | 컨트롤 | 단위·범위 |
|---|---|---|---|---|---|
| 공통 | 활성 | (목록 행 체크박스) | 0x1001 | 체크박스 | — |
| 공통 | 순서 | (목록 재정렬) | 0x1002 | 드래그·키보드 | — |
| Transform | Offset X·Y, 회전, Scale X·Y | `transform.*` | 0x1011 | 수치 필드(회전은 + `NSStepper`) | px, °, % |
| Boolean | 연산 | `boolean.op` | 0x1021 | `NSPopUpButton`(Union·Subtract·Intersect·Exclude) | — |
| Boolean | 피연산 node | `boolean.operands` | 0x1022 | 읽기 전용 | — |
| Stroke Expand | 폭 | `strokeExpand.width` | 0x1031 | 수치 필드 + `NSStepper` | px, ≥ 0 |
| Corner | 반경 | `corner.radius` | 0x1041 | 수치 필드 + `NSStepper` | px, ≥ 0 |
| Mask·Clip | 원본 node | `mask.source`, `clip.source` | 0x1051, 0x1061 | 읽기 전용 | — |
| Fill | 색 | `fill.color` | 0x1071 | `NSColorWell` | — |
| Stroke | 색, 두께 | `stroke.color`, `stroke.width` | 0x1081, 0x1082 | `NSColorWell`, 수치 필드 + `NSStepper` | px, ≥ 0 |
| Shadow | 색, Offset X·Y, Blur | `shadow.*` | 0x1091–0x1093 | `NSColorWell`, 수치 필드(Blur는 + `NSStepper`) | px, Blur ≥ 0 |
| Blur | 반경 | `blur.radius` | 0x10A1 | 수치 필드 + `NSStepper` | px, ≥ 0 |
| Blend | 모드 | `blend.mode` | 0x10B1 | `NSPopUpButton`(Normal·Multiply·Screen·Overlay·Darken·Lighten) | — |
| Opacity | 값 | `opacity.value` | 0x10C1 | 수치 필드 + `NSStepper` | %(×100), 0–100 |

- 목록 행은 활성 체크박스, 종류 아이콘, 종류 이름, 코어가 만든 요약 문자열입니다. 행을 고르면 목록 아래에 그 modifier의 파라미터 행이 나옵니다.
- 추가 메뉴의 새 항목 기본값은 `ModifierPayload::default_for`를 따릅니다. 참조형(Boolean·Mask·Clip)은 대상 지정 전까지 미해결입니다. [아키텍처 §12.6]
- 다중 선택에서는 stack을 편집하지 않고 "Select a single layer…" 안내만 둡니다.

## 6. 상호작용과 단축키

### 6.1 편집 → 명령

| 조작 | 명령 | 단계 |
|---|---|---|
| 수치·텍스트 필드에서 Return 또는 포커스 이동 | `.setValue` | `.commit` |
| 수치 필드 화살표 위·아래 | `.setValue`(단일), `.adjustNumber`(혼합) | `.continuous` |
| `NSStepper` 누름·자동 반복 | 같음 | `.continuous` |
| `NSColorWell`·색 패널 변경 | `.setValue` | `.continuous` |
| switch·popup·segmented | `.setValue` | `.commit` |
| 명령 버튼(반전, 변환, Detach, Edit on Canvas) | `.perform` | — |
| Value Binding 표시 클릭 | `.showBinding`(placeholder) | — |
| modifier 추가·삭제·활성·이동 | `.addModifier`·`.removeModifier`·`.setModifierEnabled`·`.moveModifier` | — |
| 오류 화면 Try Again | `.reload` | — |

- `.continuous`는 같은 속성의 연속 입력을 코어 `HistoryGroupingPolicy`가 한 Undo로 묶으라는 표시입니다. [아키텍처 §23.3]
- 수치 입력은 표시 단위로 받고 `displayScale`로 나눠 모델 단위로 보냅니다. 범위 밖 값은 규격 하한·상한으로 clamp하고, 숫자가 아니면 경고음 후 이전 값으로 되돌립니다.
- 모든 명령은 근거가 된 snapshot 세대(`basedOn`)와 함께 갑니다. 선택이 바뀌는 snapshot(`selectionToken` 변경)을 반영하기 전에 진행 중인 텍스트 편집을 이전 세대로 확정하고, 색 패널에 연결된 색 well을 끊습니다. 새 선택에 이전 입력이 잘못 적용되지 않게 하기 위함입니다.

### 6.2 키보드

| 키 | 동작 | 근거 |
|---|---|---|
| Tab·Shift-Tab | 인스펙터 안 컨트롤을 화면 순서로 이동(접힌 섹션 본문 제외) | [가이드 8.2] [HIG Keyboard] |
| Return | 필드 값 확정 | [HIG Text fields] |
| Escape | 편집 중 입력을 되돌림(포커스 유지) | [HIG Keyboard] |
| ↑·↓ | 수치 한 단계 증감(`step`) | 디자인 도구 관례 |
| Shift-↑·Shift-↓ | 열 단계 증감 | 디자인 도구 관례 |
| Space | disclosure·switch·체크박스 전환(시스템 동작), modifier 목록에서는 선택 항목 활성 전환 | [HIG Keyboard] |
| Delete·Forward Delete | modifier 목록에서 선택 항목 삭제 | [HIG Lists and tables] |
| Command-Option-↑·↓ | modifier 목록에서 선택 항목 위·아래로 이동 | [가이드 8.1 커스텀 단축키 규칙] |

- 인스펙터 표시·숨김(Show Inspector)은 메뉴·툴바 통합(XII.6, XII.9.1)이 맡습니다. [가이드 2.3, 8.1]
- `InspectorViewController.keyViewChain`은 인스펙터 안의 순서만 잇습니다. 창의 key view loop에 앞뒤를 잇는 일은 통합에서 합니다.

## 7. 데이터 경계(프로토콜)

### 7.1 프로토콜

```swift
protocol InspectorDataSource: AnyObject {
    var snapshot: InspectorSnapshot { get }
    var snapshotHandler: ((InspectorSnapshot) -> Void)? { get set }
    func submit(_ command: InspectorCommand, basedOn generation: UInt64)
}
```

- 모든 호출과 콜백은 메인 스레드입니다. 구현은 무거운 질의·해석을 메인 스레드에서 하지 않고 준비된 snapshot만 넘깁니다. [아키텍처 §6.3, §9.2] [가이드 9장]
- snapshot(`InspectorSnapshot`)은 plain value입니다: 세대 번호, 선택 토큰, 내용 상태, 선택 요약, 편집 가능 상태, 섹션 → 행 → 필드(속성 id, 레이블, 컨트롤 종류, 값, 편집 가능, binding 상태), modifier stack. Rust 타입을 담지 않습니다. [아키텍처 §8.2]
- 숫자 tag가 있는 enum(`InspectorNodeKind`, `InspectorShapeKind`, `InspectorModifierKind`, `InspectorColor.Space`)은 문서 모델 저장 tag와 같은 값입니다.
- 명령(`InspectorCommand`)은 셸 → 코어 단방향입니다(6.1절).

### 7.2 세대와 선택 토큰

- `generation`은 단조 증가합니다. 셸은 현재보다 오래된 snapshot을 버리고, 한 런 루프에 여러 개가 오면 마지막 것만 반영합니다. [가이드 9장]
- `selectionToken`은 선택 집합이 바뀔 때마다 코어가 새로 발급합니다. 같은 이름의 다른 레이어를 골라도 바뀝니다.
- 코어는 `basedOn` 세대의 선택 대상에 명령을 적용하거나, 만료된 세대면 거부하고 현재 상태 snapshot을 다시 보냅니다. 명령마다 결과 snapshot을 보내야 셸의 낙관적 표시가 바로잡힙니다.

### 7.3 코어 adapter가 할 일

`InspectorPropertyCatalog`의 섹션·행 템플릿에 `InspectorFieldResolver`로 값·편집 가능 여부·binding 상태를 채웁니다. 동적 popup 선택지(layout 정의, typography 정의)와 모드에 따른 수치 규격(줄 높이)은 `InspectorFieldState.control`로 바꿉니다. Component Property·Slot 섹션은 정의마다 다르므로 `componentProperty(id:name:kind:)`, `componentSlot(id:name:)`으로 만듭니다. `InspectorStubModel`이 같은 방식의 예시입니다.

## 8. 접근성·외형·성능

### 8.1 접근성

- 인스펙터 루트는 group "Inspector", 각 섹션은 섹션 제목 group, 각 행은 행 레이블 group입니다. 행 안 보조 레이블(짧은 레이블, 단위, 혼합 대시)은 접근성 요소에서 뺍니다. [가이드 8.3] [HIG Accessibility]
- 모든 조작 컨트롤은 접근성 레이블이 있습니다(필드 전체 이름, 예: "Width", "Top Left Radius"). 수치 필드는 단위를 help로 읽습니다. 아이콘만 있는 버튼(반전, 추가·삭제, binding 표시)은 레이블과 툴팁이 함께 있습니다. [가이드 5.4, 8.1]
- 혼합 값은 접근성 값 "Mixed"(switch·색 well·segmented) 또는 placeholder "Mixed"(필드)로 읽힙니다.
- modifier 목록 행은 "종류, 요약, Enabled/Disabled" 한 요소로 읽히고, 활성 체크박스는 "Enable <종류>"로 따로 조작할 수 있습니다. [가이드 8.3]
- 디자인 문서의 접근성 의미 트리 preview는 이 작업 범위 밖입니다(XII.8.3). [아키텍처 §18]

### 8.2 외형·시스템 설정

- 색은 시스템 의미 색만 씁니다: `labelColor`, `secondaryLabelColor`, `tertiaryLabelColor`, `disabledControlTextColor`, `controlAccentColor`(binding 연결 표시), 시스템 색 `systemOrange`(진단·끊어진 binding 경고). 하드코딩 RGB는 앱 크롬에 없습니다. 색 well의 색과 Stub 예시 색은 사용자 디자인 콘텐츠 값입니다. [가이드 4.2]
- 글꼴은 `NSFont.systemFontSize(for:)`·텍스트 스타일로 지정하고 pt 값을 쓰지 않습니다. 수치 필드는 고정폭 숫자를 씁니다(열린 결정 O7 제안, 10장). [가이드 4.1]
- Light·Dark: 모든 색이 동적 색이라 외형 전환을 시스템이 반영합니다. 검사는 두 외형 모두에서 배치를 확인합니다.
- 투명도 줄이기: 인스펙터는 반투명 표면을 직접 그리지 않으므로 대응할 대상이 없습니다(pane 재질은 시스템). [가이드 8.4]
- 대비 높이기: 표준 컨트롤과 의미 색이 시스템 설정을 따릅니다. 직접 그리는 선·표면이 없습니다. [가이드 8.4]
- 동작 줄이기: 섹션 접기·펼치기 애니메이션은 `accessibilityDisplayShouldReduceMotion`을 매번 읽어 켜져 있으면 즉시 전환합니다. [가이드 8.4, 8.5]

### 8.3 성능

메인 스레드 단일 점유 2ms 상한을 기준으로 설계했습니다. [아키텍처 §6.3] [가이드 9장]

- 같은 구성의 snapshot은 바뀐 행만 갱신합니다(`InspectorRow` 값 비교). modifier stack도 값이 같으면 다시 그리지 않습니다.
- 구성(섹션 id 목록)별 페이지(`InspectorPageView`)를 창 안에 숨겨 두고 재사용합니다(최대 4개, 넘치면 다음 런 루프에서 가장 오래 안 쓴 페이지 제거). 선택 종류가 바뀌어도 컨트롤을 다시 만들지 않습니다.
- 페이지 외곽은 autoresizing으로 루트를 따릅니다. 모든 페이지를 루트 가장자리 제약으로 묶으면 루트 폭 변수를 공유해, 새 페이지를 넣을 때 숨은 페이지 제약까지 다시 풀었습니다.
- `NSButton` 편의 생성자(생성 시 `sizeToFit`)를 쓰지 않습니다.

측정(로컬 Debug 빌드, Stub, 폭 300pt, `InspectorUICheck` 중앙값, 반영 + 배치):

| 경우 | 시간 |
|---|---|
| 값 하나 변경 | 약 0.6ms |
| 이미 만든 구성으로 선택 종류 전환 | 약 1.5ms |
| 처음 보는 구성의 페이지 생성(창당 구성별 1회) | 약 70–90ms |

- 첫 페이지 생성은 2ms 상한을 넘습니다. macOS 26에서 `NSPopUpButton`·`NSSegmentedControl`(내부 SwiftUI 호스팅)·`NSButton` 크기 계산이 각 0.1–1ms이고, 창의 제약 엔진 최적화 비용이 창 전체 제약 수에 비례하기 때문입니다. 숨은 페이지가 많을수록 새 페이지 생성이 느려져 상한을 4개로 두었습니다. 후속 대응은 9장에 둡니다.
- 실제 판정은 Release 빌드와 통합 창에서 XVI.1 계측으로 합니다.

## 9. 코어 연결과 후속 작업

| 항목 | 내용 | 관련 |
|---|---|---|
| FFI adapter | `stellar_runtime_poll_event`로 받은 선택 snapshot을 `InspectorSnapshot`으로 옮기고 명령을 Command로 보내는 `InspectorDataSource` 구현 | ADR-0009 [아키텍처 §8.2] |
| PropertyKey | Shape·Path·Text·Image(asset 외) 속성의 key가 문서 모델에 없음. metadata registry(VI.3.5·VI.9.7 방식)가 생기면 `coreKey`를 채움 | `frame_metadata.rs`, `modifier_metadata.rs` |
| Transform 분해 | Frame·Transform modifier의 행렬 ↔ 위치·회전·배율 분해·합성 규칙 | [아키텍처 §12.3, §12.6] |
| Value Binding UI | binding 표시를 눌렀을 때의 편집 UI, 출처 경로 표시. 지금은 표시만 하는 placeholder | [아키텍처 §14.2, §14.4] [가이드 6.3] |
| Component Property 선언 | 정의의 Property 타입·기본값 선언이 문서 모델에 아직 없음(`ComponentDefinition`은 id 목록). override 표시와 기본값 되돌리기 | [아키텍처 §15.1] |
| View Variant | Variant별 값 편집 | [아키텍처 §16] |
| 접근성 의미 preview | 디자인 문서 role·label 등 읽기 | XII.8.3 [아키텍처 §18] |
| Text style run | 선택 범위 기준 문자 style, OpenType feature·variation 편집 | `text.rs` VI.7.2 |
| Undo 그룹화 | `.commit`·`.continuous` 단계를 `HistoryGroupingPolicy`에 연결 | [아키텍처 §23.3] |
| 첫 페이지 생성 비용 | 행 `NSStackView`를 직접 제약으로 바꿔 제약 수 줄이기, 유휴 시간 분할 예열, Release 측정 | [아키텍처 §6.3] XVI.1 |
| 셸 통합 | `NSSplitViewController` 3열 인스펙터 자리, Show Inspector 메뉴·툴바, 창 key view loop 연결, 섹션 접힘 상태 복원, scroll edge effect(macOS 26.1 이상) | XII.9.1, XII.6 [가이드 2.3, 2.6] |
| 검사 연결 | `main.swift`에 `--inspector-ui-check` 분기를 두고 `scripts/macos/smoke.sh`에서 실행 | 10장 |

- 가이드 10.4에 따라 인스펙터 내용(Value Binding UI, 출처 경로, Component·Variant 편집)은 계획에 전용 WBS가 없습니다. 이 작업의 WBS 대응은 통합에서 정합니다. [가이드 10.4]

## 10. 수치 후보와 열린 결정

가이드 0.2절에 따라 수치는 구현 작업에서 정하고 가이드 12장에 기록합니다. 아래는 이 구현의 후보값이며, 통합 시 근거와 함께 가이드 12장에 옮깁니다(`InspectorMetrics`).

| 항목 | 값 | 근거 |
|---|---|---|
| 패널 바깥 여백 | 12pt | HIG Layout 여백 관례, 300pt 폭에서 두 값 행 배치 확인 |
| 행 간격 | 6pt | 같음 |
| 컨트롤 묶음 간격 | 4pt | 같음 |
| 행 레이블 열 | 88pt | "Letter Spacing"이 small 크기에서 잘리지 않는 폭 |
| 수치 필드 선호 최소 폭 | 36pt(우선순위 480) | 네 자리 수 표시 |
| 컨트롤 크기 | small | 가이드 4.4 "밀도 높은 인스펙터는 small·regular" |
| 대표 폭 | 300pt | 검사 기준 폭 |

- 열린 결정 O7(수치 필드 고정폭 숫자): 인스펙터는 고정폭 숫자(`monospacedDigitSystemFont`)를 제안합니다. 자릿수 정렬이 쉬워지고, 값이 바뀔 때 폭이 흔들리지 않습니다. [가이드 11장 O7]
- `prefersCompactControlSizeMetrics` 사용 여부는 측정하지 않았습니다. 통합에서 정합니다. [가이드 4.4]
- 고정값 속성에도 binding 표시(점선 원)를 항상 보일지, 연결된 속성에만 보일지는 Value Binding UI 작업에서 정합니다. 지금은 항상 보입니다.

## 11. 검사

`InspectorUICheck.run()`(통합 시 `scripts/macos/smoke.sh`에 연결)이 화면 밖 창에서 다음을 확인합니다.

- 사용한 SF Symbols 이름이 모두 시스템에 있음(현재 41개).
- Stub 17개 장면 × Light·Dark: Auto Layout 모호성 없음, 내용이 인스펙터 폭을 넓히지 않음, 컨트롤이 패널 여백 안에 있음, 크기 0 컨트롤 없음, 조작 컨트롤의 접근성 레이블, 장면별 섹션 구성, key view 순서 연결.
- 상호작용: 수치 확정·clamp·Escape 되돌리기·화살표 키, stepper, switch와 의존 필드, popup·동적 선택지, 색 well, segmented, 줄 높이 Auto, disclosure와 key view 제외, 오래된 세대 무시, 선택 종류 전환, modifier 선택·배율 변환·추가·활성·이동(Command-Option-↑)·삭제, 혼합 값 상대 조정(Shift-↑), Fragment 변환, 잠금 → 안내 → Unlock, 페이지 재사용 상한·재생성·접힘 상태 공유, 편집 중 선택 변경 시 이전 세대로 확정.
- Stub 한계: 혼합 값 상대 조정을 적용하지 않음, Value Binding 요청에 snapshot을 내지 않음. [계획 1.2 규칙 6]
- 반영 시간 참고값(8.3절)을 출력합니다. 판정 기준은 아닙니다.
- 화면 밖에서 대비 높이기·투명도 줄이기 설정을 강제할 수 없어, 이 두 설정의 실제 화면은 검사하지 않습니다. XII.9.4 적합성 보고서에서 확인합니다.

## 12. SF Symbols

| 용도 | 이름 |
|---|---|
| 상태 | `cursorarrow.rays`, `exclamationmark.triangle`, `lock.fill`, `pencil.slash`, `exclamationmark.triangle.fill`, `square.stack.3d.up` |
| node 종류 | `rectangle.dashed`(Frame), `square.on.square.dashed`(Fragment), `rectangle`, `circle`, `line.diagonal`, `pentagon`, `scribble.variable`(Path), `textformat`(Text), `photo`(Image), `diamond`(ComponentInstance) |
| 명령 | `plus`, `minus`, `arrow.left.and.right`(좌우 반전), `arrow.up.and.down`(상하 반전), `text.alignleft`, `text.aligncenter`, `text.alignright`, `text.justify` |
| Value Binding | `circle.dashed`(고정값), `link.circle.fill`(토큰), `arrow.down.right.circle.fill`(Component Property), `function`(식), `exclamationmark.circle.fill`(끊어짐) |
| Modifier | `arrow.up.and.down.and.arrow.left.and.right`(Transform), `square.on.circle`(Boolean), `lineweight`(Stroke Expand), `rectangle.roundedtop`(Corner), `theatermasks`(Mask), `scissors`(Clip), `drop.fill`(Fill), `pencil.line`(Stroke), `shadow`(Shadow), `aqi.medium`(Blur), `circle.lefthalf.filled`(Blend), `circle.dotted`(Opacity) |

이미지 asset은 쓰지 않습니다. [가이드 5.4] [AGENTS.md 변경 정책]
