//! CSS layout longhand → Compose layout 규칙 (XXII.4.2, ADR-0107 결정 3·4).
//!
//! [`COMPOSE_RULES`]는 layout longhand 40개 각각의 Android profile 규칙입니다. XXII.1.4 capability matrix의 Android 열
//! (CSS layout longhand 부분)이며 생성기는 이 규칙대로만 낮추고 1:1이 아닌 경우를
//! [`LayoutFinding`](crate::apple::LayoutFinding)으로 냅니다. 단위: CSS `px` 1은 `1.dp`입니다.

/// Compose 규칙(longhand, 규칙).
pub const COMPOSE_RULES: [(&str, &str); 40] = [
    ("position", "static·relative는 흐름 item, absolute는 부모 `Box` 위 `Modifier.align(Alignment.TopStart).offset`. 양쪽 inset이 모두 있거나 모두 auto면 Approximated"),
    ("box-sizing", "border-box는 크기 modifier 뒤 `padding`, content-box는 `padding` 뒤 크기 modifier(Native)"),
    ("top", "relative는 `Modifier.offset(y =)`, absolute는 위쪽 정렬 + offset. `%`는 Unsupported"),
    ("right", "relative는 `offset(x = -값)`, absolute는 `Alignment.*End` 정렬 + offset. `%`는 Unsupported"),
    ("bottom", "relative는 `offset(y = -값)`, absolute는 `Alignment.Bottom*` 정렬 + offset. `%`는 Unsupported"),
    ("left", "relative는 `offset(x =)`, absolute는 시작 정렬 + offset. `%`는 Unsupported"),
    ("width", "`<length>`는 `width(dp)`, 채움은 `fillMaxWidth()`, min-content·max-content는 `width(IntrinsicSize.Min|Max)`, `%`는 `fillMaxWidth(비율)`(Approximated), fit-content는 자연 크기(Approximated)"),
    ("height", "width와 같은 규칙(세로)"),
    ("min-width", "`<length>`는 `widthIn(min =)`, auto는 플랫폼 최소 크기(root에 한 번 Approximated), 그 밖은 Unsupported"),
    ("min-height", "min-width와 같은 규칙(세로)"),
    ("max-width", "`<length>`는 `widthIn(max =)`, none은 출력 없음, 그 밖은 Unsupported"),
    ("max-height", "max-width와 같은 규칙(세로)"),
    ("aspect-ratio", "`Modifier.aspectRatio(w / h)`(Approximated: 제약 안 맞춤)"),
    ("flex-grow", "`Modifier.weight(grow)`. 성장 형제가 둘 이상이면 모두 `flex-basis: 0`·main 크기 auto일 때 Native, 아니면 Approximated(콘텐츠 크기를 빼지 않고 비율 분배). 부모 main 축이 확정이 아니면 효과 없음"),
    ("flex-shrink", "1은 순차 측정 압축(root에 한 번 Approximated), 그 밖은 Approximated"),
    ("flex-basis", "grow 0의 `<length>`는 main 크기 modifier, auto·content는 출력 없음, `%`는 Unsupported"),
    ("align-self", "`Modifier.align(Alignment.*)`, row의 baseline은 `alignByBaseline()`, column의 baseline은 Approximated"),
    ("justify-self", "grid 칸 `Box(contentAlignment =)`, flex·block은 효과 없음"),
    ("grid-column-start", "모든 item이 명시 line이거나 모두 자동 배치면 행 `Row`의 칸, 섞이면 Unsupported(문서 순서 자동 배치)"),
    ("grid-column-end", "열 span은 span한 track 합(고정 폭 + gap 또는 weight 합). gap이 있는 fr span은 Approximated"),
    ("grid-row-start", "grid-column-start와 같은 규칙"),
    ("grid-row-end", "행 span은 Unsupported(span 1로 냄)"),
    ("display", "block은 `Column` + 폭 채움, flex는 `Row`·`Column`(wrap은 `FlowRow`·`FlowColumn`), grid는 행 `Column` 안 칸 `Row`, none은 출력 없음"),
    ("padding-top", "`Modifier.padding(top =)`"),
    ("padding-right", "`Modifier.padding(end =)`"),
    ("padding-bottom", "`Modifier.padding(bottom =)`"),
    ("padding-left", "`Modifier.padding(start =)`"),
    ("flex-direction", "row는 `Row`, column은 `Column`. reverse는 자식 출력 순서를 뒤집어 접근성 순회 순서도 바뀌므로 Approximated"),
    ("flex-wrap", "wrap은 `FlowRow`·`FlowColumn`(Native), wrap-reverse는 Unsupported(wrap으로 냄)"),
    ("justify-content", "`Arrangement.Start|Center|End`(gap이 있으면 `spacedBy(gap, alignment)`), space-between·around·evenly는 `Arrangement.Space*`(gap이 있으면 Approximated)"),
    ("align-content", "nowrap flex는 효과 없음, wrap은 Flow의 cross arrangement, grid는 start·center·end를 행 묶음 arrangement로"),
    ("align-items", "`verticalAlignment`·`horizontalAlignment`, stretch·normal은 자식 `fillMaxHeight()`·`fillMaxWidth()` + 확정이 아닌 container `height|width(IntrinsicSize.Max)`, row baseline은 자식 `alignByBaseline()`"),
    ("justify-items", "grid 칸 `Box(contentAlignment =)` 가로 성분·채움, flex·block은 효과 없음"),
    ("row-gap", "column `Arrangement.spacedBy`, Flow·grid의 행 간격"),
    ("column-gap", "row `Arrangement.spacedBy`, Flow·grid의 열 간격"),
    ("grid-template-columns", "`<length>`는 칸 `width(dp)`, `minmax(0, fr)`는 `weight(fr)`(비율 그대로), 최소 auto fr은 Approximated, 콘텐츠 track은 행마다 폭이 달라 Approximated, `%`·섞인 minmax는 Unsupported"),
    ("grid-template-rows", "`<length>`는 행 `height(dp)`, fr은 Approximated, 콘텐츠 track은 자연 크기"),
    ("grid-auto-columns", "암시 열에 grid-template-columns 규칙"),
    ("grid-auto-rows", "암시 행에 grid-template-rows 규칙"),
    ("grid-auto-flow", "row·column 자동 배치 순서를 칸 좌표로(Native)"),
];
