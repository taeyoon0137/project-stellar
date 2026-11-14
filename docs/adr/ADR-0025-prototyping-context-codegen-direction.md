# ADR-0025: CSS 기반 속성 위의 노드 프로토타이핑·Component Context·코드 생성 방향

## Status

Accepted (2026-11-14, 방향 결정). 제품 소유자 방향 P4(2026-11-14)를 기록합니다. 같은 날 교차 검토를
반영해 Component Context를 로드맵으로 옮기고(결정 6), 상태 어휘 대응(결정 1.3), slot 콘텐츠 해석
(결정 3.11), Web Context lowering 규칙(결정 4.3)을 더했습니다. 저장 형식의 tag 번호, numeric
registry 값, 공개 API 시그니처는 이 ADR에서 고정하지 않습니다. 각 구현 행이 착수할 때 별도 ADR로
고정합니다(계획 2장 검토 항목 5). Context 번호 예약은 IX.12.1의 ADR이 맡습니다.

## Context

제품 소유자는 2026-11-14에 다음 방향을 정했습니다.

- 인스펙터에서 편집하는 디자인 속성은 기본적으로 CSS 기반입니다(P3). CSS 속성 어휘, 값 문법, 레이아웃
  의미(box model, Flexbox, Grid, positioning, typography, background·border, effect, transform,
  opacity·blend)를 따릅니다. CSS 기반 속성 어휘와 저장 표현 자체는 이 ADR의 범위가 아니며
  ADR-0024와 `docs/design/property_model.md`(이하 "속성 모델")가 정합니다.
- 이 속성들은 나중에 노드 기반 프로토타이핑으로 연결됩니다. 업계에서 Origami 계열 도구가 쓰는 patch·node
  그래프처럼, 그래프가 속성과 인터랙션을 움직입니다(P4).
- 컴포넌트는 property drilling과 함께 context(트리 아래로 값을 공급·소비)를 지원합니다(P4).
- 디자인은 실제 코드로 변환됩니다(P4).

현재 코드와 계획의 상태는 다음과 같습니다.

- 노드 그래프의 기반은 일부 있습니다. 표현식 AST를 GUI block·edge로 읽는 `ExpressionGraphView`, socket
  타입 연결 검사, block 편집 command plan, 오류→block·socket 매핑이 구현되어 있습니다(VIII.3.1\~VIII.3.4).
  다만 범위는 속성 하나의 값 식 안의 dataflow입니다. 이벤트·상태 흐름은 선언형 상태 머신(IX.5)에, 의미
  계약은 Control Contract(IX.6)에 따로 있습니다. 시간축(애니메이션·전환) 모델은 아키텍처와 계획 어디에도
  없고 `Duration` 값 타입만 있습니다.
- property drilling은 일부 구현되었습니다. nested instance 직접 수정 금지(IX.4.2), nested state 직접
  쓰기 금지(IX.5.6), orphan fallback(VIII.8)은 `[x]`입니다. 경계별 Property chain 필수화와
  DrillingPlan(IX.4.1, IX.4.3\~IX.4.6)은 `[ ]`입니다.
- context는 모델과 계획에 없습니다. 아키텍처는 의존성 DAG(§12.8), Value Binding 연결원 "상위 계층
  전달값"과 출처 경로의 Provider(§14.2), merge conflict(§27.1)에서 Context를 언급합니다. 그런데
  §14.3과 §15.1은 "여러 경계를 통과하면 모든 경계에 Property가 존재해야 함", "컴포넌트 경계를 건너는
  암묵적 참조 금지"를 요구합니다. 업계 프레임워크(React Context, SwiftUI Environment, Compose
  CompositionLocal, CSS custom property 상속)처럼 경계를 암묵적으로 넘는 공급·소비는 이 규칙과
  충돌합니다. ADR-0024 결정 11도 상속형 공급(context)을 범위 밖으로 두고 별도 결정으로 남겼습니다.
- 코드 생성(Phase XXII)은 전부 `[ ]`입니다. 계획에는 속성 단위 매핑 규칙(modifier stack 선형화, 블렌드
  합성 색공간 차이, 자유 배치 anchor→CSS inset 변환)이 없습니다. 현재 매핑 근거는 Control export hint
  (IX.6.7)와 SVG 매핑표뿐입니다.

제약은 다음과 같습니다.

- 결정론(§3.1): 같은 입력은 같은 결과를 냅니다. 표현식은 현재 시각과 난수를 쓰지 않습니다(§14.1).
- 단일 writer: 문서 변경은 DocumentThread만 하고(XII.1.1), 모든 편집은 CommandTransaction을 거칩니다.
  에이전트 편집도 같은 경로를 씁니다(§31.4).
- 문서에는 임의 JavaScript를 넣지 않습니다(§14.1, §15.3). 플러그인 런타임(Phase XXI)은 문서 의미와
  분리된 별도 경계입니다.
- 프리뷰 런타임 상태는 프로젝트에 저장하지 않습니다(§16, IX.5.4).
- 생성 코드는 단방향입니다(§4.3, §17.3).

## Decision

### 1. 공유 기반: CSS 기반 속성

인스펙터의 CSS 기반 속성 하나는 다음 세 곳에서 같은 식별자로 쓰입니다.

1. **값 연결 대상.** 기존 `ValueBinding` target(PropertyPath)입니다. 고정값, 토큰, Component
   Property, Context, 조건식이 모두 이 대상에 연결됩니다(§14.2).
2. **노드 그래프 포트.** 프로토타입 그래프에서 속성은 타입 있는 입력 포트로 나타납니다. 포트 하나는
   registry longhand 하나이고, 포트의 값 타입은 registry 항목의 CSS 값 문법입니다(ADR-0024 결정 13).
   그래프의 연결 검사는 그 값 문법을 `ExpressionType`으로 바꿔서 합니다. 구성 타입 하나의 대응은 한
   가지이며 속성 모델 2장·8.1절의 Stellar 타입↔CSS 값 타입 표를 그대로 씁니다(예: `<length>`→Length,
   `<time>`→Duration, keyword→속성별 Enum). `ExpressionType`이 없는 Stellar 타입에 대응하는 값
   문법(Grid track `<track-size>` 등)의 속성은 포트로 노출하지 않고 진단합니다(XXX.1.2).
   - **합집합 문법.** `width`(`auto | <length> | <percentage> | fit-content`)처럼 구성 타입이 둘 이상인
     문법은 구성 타입마다 위 대응으로 바꾼 `ExpressionType`의 집합이 됩니다. keyword 구성 요소는 그
     속성의 keyword만 담은 Enum 하나로 모읍니다. 포트와 binding 대상은 이 집합에 속한 타입의 식만
     받고, 연결된 식의 정적 타입이 저장 값의 variant를 정합니다. 저장된 값 variant에 따라 포트 타입을
     바꾸지 않으므로 값을 바꿔도 연결 검사 결과가 달라지지 않습니다. 대응이 없는 구성 타입은 집합에서
     빼고, 집합이 비면 포트로 노출하지 않습니다. 길이·백분율을 섞은 `calc()`는 구성 타입이 아니며
     `<length-percentage>` 값 타입(속성 모델 15장 O3)이 정해지기 전에는 받지 않습니다. `MotionSpec`
     보간은 같은 구성 타입 사이에서만 하고, 시작값과 끝값의 타입이 다르면 진단합니다(XXX.1.1).
     Color 보간은 저장된 색공간(sRGB 또는 Display P3)의 encoded 채널을 alpha로 곱한(premultiplied) 값에서
     IEEE-754 기본 연산만으로 선형 보간하고, 결과 alpha로 다시 나눕니다(결과 alpha가 0이면 채널 0). 이는
     CSS Color 4의 `srgb`·`display-p3` 보간 공간과 같습니다. 선형 광 공간이나 Oklab 보간은 `pow`·`cbrt`
     같은 libm 초월함수가 필요해 ADR-0006 기준으로 플랫폼 의존 결과가 되고 결정 2.7과 맞지 않으므로 쓰지
     않습니다. 시작값과 끝값의 색공간이 다르면 변환에 같은 초월함수가 필요하므로 보간하지 않고
     진단합니다(XXX.1.1). Web 출력에서 이 보간 공간을 지정할 수 없으면 Approximated입니다(XXII.1.7).
     `height`, inset longhand(`top` 등), `flex-basis`, `min-*`, `max-*`도 같은 규칙을 씁니다.
3. **코드 생성 매핑 단위.** Target Profile capability registry(IX.10.5)는 CSS 기반 속성마다 profile
   kind별로 Native·Approximated·Unsupported·Deprecated를 기록합니다. Web 항목은 IX.11.19 matrix와
   IX.11.18 modifier stack 선형화 계획(ADR-0024)이 정하고, 코드 생성은 그 matrix에 Apple·Android
   lowering 규칙(XXII.1.4)과 Context·`MotionSpec` 항목(XXII.1.7)만 더합니다. Web adapter는 같은 이름의 CSS
   속성으로 출력하고, Apple·Android adapter는 이 registry를 거쳐 SwiftUI·Compose 표현으로 낮춥니다.
   registry 항목의 CSS 매핑 상태(속성 모델 1.1절)와 Web profile capability 상태는 다음처럼
   대응합니다.
   - `exact`, `converted` → Native. `converted`의 변환 규칙은 matrix 항목에 둡니다.
   - `approximated` → Approximated.
   - `stellar-only` → 구조 변환·path 굽기·래스터화 규칙이 있으면 Approximated, 출력이 없으면
     Unsupported.
   - Deprecated는 CSS 매핑 상태에서 나오지 않습니다. 프로젝트가 고정한 Web profile 버전(IX.10.4)이
     정합니다.
   - `initial-differs`·`subset` 표시는 상태를 바꾸지 않습니다.
   - Apple·Android profile의 상태는 CSS 매핑 상태에서 유도하지 않고 lowering 규칙(CSS 기반 속성은
     XXII.1.4, Context·`MotionSpec`은 XXII.1.7)이 정합니다. 초기 capability matrix(IX.11.19)는 Web
     profile 항목만 등록합니다.

속성 이름과 값 문법은 CSS를 따르지만, 저장 원본은 CSS 문자열이 아니라 타입 있는 속성 record입니다.
CSS 문자열을 저장하지 않으므로 parser를 문서 열기 경로에 두지 않고, 타깃별로 낮추는 과정에서 손실을
진단할 수 있습니다.

### 2. 노드 기반 프로토타이핑

1. **새 원본을 만들지 않는 projection.** 프로토타입 그래프는 `ExpressionGraphView`(VIII.3.1)처럼
   읽기 전용 projection입니다. 그래프의 node·edge는 다음 기존 원본에서 만듭니다.
   - 값 흐름: `ValueBinding`과 Expression AST(VIII.2, VIII.6)
   - 이벤트·상태 흐름: 상태 변수, EventKind, Transition, action(IX.5)
   - 의미 계약: Control Contract의 이벤트·상태(IX.6)
   - Context 읽기(이 ADR의 3절)
   - 시간축: 새 `MotionSpec`(아래 3번)
2. **신호 종류를 타입으로 구분합니다.** socket은 상태(State)와 펄스(Pulse) 두 종류입니다. 상태 socket은
   시간에 걸쳐 유지되는 TypedValue이고 expression으로 낮춥니다. 펄스 socket은 한 번 발생하는 사건이고
   EventKind 또는 semantic emit으로 낮춥니다. 두 종류를 바로 연결할 수 없으며, 변환은 등록된 변환
   node(상태 변화→펄스, 펄스→상태 토글 등)만 합니다.
3. **시간축은 `MotionSpec`으로 저장합니다.** 대상 PropertyPath 집합, 시작 조건(상태 변수 변화), 지속
   시간, 지연, timing function을 담습니다. timing function은 닫힌 집합입니다. CSS 어휘인 linear,
   cubic-bezier, steps에 spring(질량, 강성, 감쇠, 초기 속도)을 더합니다. 업계 도구의 progress·보간
   패턴은 `MotionSpec`의 시작값→끝값 보간으로 표현합니다.
4. **편집은 command plan입니다.** node 추가, 연결, 해제, 삭제는 기존 binding·expression·transition
   command와 `MotionSpec` command로 낮춘 command plan입니다. 사용자 동작 하나는 CommandTransaction
   하나이고 창별 Undo 1회로 되돌립니다.
5. **컴포넌트 경계는 공개 인터페이스로만 넘습니다.** 다른 컴포넌트의 내부 포트에 직접 연결하지 않습니다.
   업계 도구의 "포트 공개"는 Stellar에서 공개 ComponentProperty·Event·Context 요구를 만드는 것이고,
   DrillingPlan(IX.4.3)과 같은 방식으로 계획·미리보기·원자 적용합니다.
6. **그래프 배치 위치는 기본적으로 저장하지 않습니다.** 기본 배치는 결정론적 자동 배치입니다. 사용자가
   고정한 위치만 희소 배치 hint(StableId→좌표)로 저장하고, Semantic Diff에서는 표현 전용 항목으로
   분류합니다.
7. **결정론적 평가.** 프리뷰 세션이 논리 시계(고정 tick)를 공급합니다. 값은 (상태, 논리 tick, 입력
   이벤트 로그)의 순수 함수이고, wall clock은 문서 평가에 들어가지 않습니다. 셸은 디스플레이 프레임을
   tick으로 대응시키기만 합니다. 같은 이벤트 로그와 tick 열은 같은 값 trace를 냅니다. timing function
   평가는 IEEE-754 기본 연산(사칙연산·`sqrt`)만 씁니다. spring은 고정 step 적분으로 계산하고,
   `exp`·`cos` 같은 libm 초월함수가 필요한 닫힌 식은 쓰지 않습니다(ADR-0006). tick 안의 펄스는
   정해진 순서로 처리하고, 상태 변경은 IX.5.5처럼 원자적으로 확정합니다. 펄스 연쇄에는 dispatch 깊이
   상한을 두고, 넘으면 진단합니다.
8. **프리뷰는 문서를 쓰지 않습니다.** 프리뷰 상태, 진행 중 motion, 입력 로그는 세션 메모리에만 있습니다
   (IX.5.4와 같음). 프리뷰를 조작한 뒤에도 save bytes는 같습니다.
9. **reduced motion.** 환경 key `reduced_motion`(VIII.7.1)이 켜져 있으면 `MotionSpec`이 정의한 대체
   동작(기본은 즉시 전환)을 씁니다.
10. **포인터 제스처 이벤트.** hover, pointer down·up, drag, scroll, long press는 현재 EventKind(IX.5.2)에
    없습니다. EventKind 확장은 numeric registry 변경이므로 구현 행에서 별도 ADR로 고정합니다. 제스처
    판정 임계값(long press 시간 등)은 논리 tick 단위로 registry에 고정합니다. 현재 EventKind payload는
    `TypeSpec` 하나이고(`crates/document/src/component/event.rs`의 `payload_type`) `TypedValue`에는 2D
    값이 없어(`crates/core-types/src/typed_value.rs`) drag·scroll 변위를 그대로 실을 수 없습니다. 그
    ADR은 2D 값 타입 추가와 축별 이벤트 중 하나를 정하고, drag 끝 이벤트에 spring 초기 속도로 쓸 축별
    release velocity를 싣습니다(XXX.1.4).
11. **그래프 편집기 화면.** 노드 그래프 편집 면은 표준 AppKit 구성요소로 만들 수 없는 디자인 도구 고유
    요소입니다. macOS 셸 디자인 가이드 1.2 원칙 2·3에 따라, 이 ADR은 그래프 편집 면을 캔버스와 같은
    방식(코어 렌더러 scene)으로 그리는 것을 승인합니다. 포트 목록, 속성 편집, 메뉴, 툴바, 다이얼로그는
    네이티브 컨트롤을 씁니다. 배치는 Xcode 같은 macOS 개발 도구의 편집기 영역 분할(캔버스와 그래프를 나란히)을
    기준점으로 삼되, 다른 제품의 UI 문구와 시각 요소는 옮기지 않습니다.

### 3. Component Context

Context는 경계를 암묵적으로 넘는 참조가 아닙니다. **선언된 인터페이스**입니다. ADR-0024 결정 11이
범위 밖으로 남긴 상속형 공급을 이 절이 선언형으로 정합니다. 아키텍처 §14.3의 "경계를 넘는 값은 공개
Component Property로만", "여러 경계를 통과하면 모든 경계에 Property"는 Property 값에만 적용하도록
좁히고, Context는 아래 3번의 `ContextRequirement` 선언으로 경계를 넘습니다(§14.5). 구현은 로드맵
XXX.7이 맡습니다(결정 6).

1. **`ContextKey`.** StableId, 이름, TypeSpec, 필수 default expression을 가진 resource입니다.
   default는 ComponentProperty처럼 닫힌 식이어야 합니다(IX.1.2). scope는 토큰처럼 프로젝트·라이브러리·
   문서이고, 참조는 StableId로 합니다(VIII.5.2).
2. **제공(provide).** `ContextProvision`은 node 하나에 붙어 key 하나에 값 식 하나를 연결합니다. 효과
   범위는 그 node의 하위 트리이며, instance expansion view(IX.2.2)를 따라 펼친 트리를 기준으로 합니다.
   slot 콘텐츠에는 11번 규칙이 따로 적용됩니다. 상위 ComponentDefinition은 자기 트리 안의 instance
   node에 provision을 붙일 수 있습니다. 이것은 자기 scope 안의 편집이므로 캡슐화를 침범하지 않습니다.
3. **소비 선언(require).** ComponentDefinition 안에서 Context를 읽으려면 그 정의의 공개 인터페이스에
   `ContextRequirement`(key 참조)를 선언해야 합니다. Property처럼 인스펙터와 라이브러리 정보에
   나타납니다. 그래서 컴포넌트가 무엇을 받는지는 명시적이고(§15.1 유지), 중간 컴포넌트는 key를 다시
   선언하지 않아도 됩니다(drilling과 다른 점). ViewDefinition 최상위 트리는 컴포넌트 경계가 아니므로
   선언 없이 읽습니다.
4. **소비(consume).** `ValueBinding` source expression이 Context 참조를 읽습니다. ReferenceTarget에
   Context 종류를 추가해야 하며, 이것은 저장 형식 변경이므로 구현 행에서 별도 ADR로 고정합니다.
5. **해석 규칙.** 소비 지점에서 펼친 트리를 위로 올라가며 가장 가까운 provision이 이깁니다(slot
   콘텐츠는 11번). 없으면 `ContextKey` default를 씁니다. 안쪽 provision이 바깥 provision을 그 하위
   트리에서 가립니다(shadowing). provision의 값 식이 같은 key를 읽으면 바깥(조상) 값으로 해석합니다. 서로 다른 key가 같은 node나
   조상·자손 관계에서 서로를 읽어 만드는 cycle은 binding cycle detector(VIII.6.3)가 저장 전에 거부합니다.
6. **override와 결합.** provision 값은 PropertyPath로 주소를 가지므로 View Variant(IX.7.3)와 Target
   Profile override(IX.10.7)가 기존 우선순위 규칙대로 덮어쓸 수 있습니다.
7. **진단.** 선언 없는 소비(오류), 타입 불일치(오류), 존재하지 않는 key의 provision·Slot 공개 목록
   항목(orphan), 쓰지 않는 선언(경고), 기본값으로 해석됨(정보), shadowing(정보)을 냅니다. `ContextKey`를 삭제하면 소비 binding은
   삭제하지 않고 OrphanedBinding(VIII.8.1)이 됩니다. provision만 삭제되면 default로 해석되며 orphan이
   아닙니다.
8. **출처 표시.** provenance chain(VIII.6.5)에 provision node StableId와 key를 기록합니다. 인스펙터는
   "이 값을 제공한 조상"으로 이동하는 경로를 보여 줍니다. 업계 프레임워크 문서가 지적하는 암묵적 의존성의
   추적 어려움을 이 표시로 보완합니다.
9. **UI 통합.** 사용자 UI에서는 Context를 별도 기능처럼 나누지 않습니다(§14.2). Value Binding 선택기에
   Context key가 연결원 종류 하나로 나타납니다.
10. **drilling과의 역할 분담.** 특정 컴포넌트의 파라미터는 명시적 Property drilling(IX.4)을 씁니다. 하위
    트리 범위의 개념(테마, 밀도, 방향, 강조 색 등)에만 Context를 씁니다. 업계 프레임워크 지침과 같은
    방향입니다.
11. **slot 콘텐츠의 해석 기준.** slot 콘텐츠는 렌더 트리가 아니라 **작성 scope** 기준으로 해석합니다.
    slot 콘텐츠는 그것을 작성한 소유자(instance를 둔 정의 또는 View)의 node이고, scene에서 부모도
    instance입니다. slot host 아래에 놓이는 것은 expansion view의 투영뿐입니다
    (`crates/document/src/component/expansion.rs`의 `ExpandedNodeId::Owned`). 그래서 slot 콘텐츠 안
    소비 지점에서 위로 올라가다 slot host에 이르면 컴포넌트 내부 조상을 건너뛰고 instance node로 이어
    올라갑니다.
    - 컴포넌트 내부 provision을 slot 콘텐츠에 보이려면 ComponentDefinition이 그 Slot의 공개
      인터페이스에 **공개 Context key 목록**을 선언해야 합니다. 목록에 있는 key만 slot host부터
      컴포넌트 내부 조상 provision을 찾고, 내부에 provision이 없으면 instance node로 이어 올라갑니다.
      공개 목록은 Property처럼 인스펙터와 라이브러리 정보에 나타납니다.
    - 정의 안에서 작성한 slot 콘텐츠의 소비는 3번 규칙대로 그 콘텐츠를 작성한 정의의
      `ContextRequirement`가 필요합니다. View 최상위에서 작성한 콘텐츠는 선언이 필요 없습니다.
    - 이 규칙에서는 View 최상위에서 작성한 콘텐츠가 컴포넌트 내부 provision을 읽는 경로가 공개 목록
      하나뿐입니다. 공개 목록은 컴포넌트의 명시 선언이므로 §15.1의 명시성과 캡슐화가 유지됩니다.
    - 렌더 트리로 값을 전달하는 타깃으로 낮출 때는 slot 삽입 위치에서 공개하지 않은 key를 instance
      바깥 값으로 다시 공급해 같은 결과를 냅니다(4.3).

### 4. 실제 코드 변환

1. CodegenIR은 CSS 기반 속성 record(IX.11.6 layout record, IX.11.11 Typography record, ADR-0024)를
   XXII.1.1에서, 상태 머신과 `MotionSpec`을 XXII.1.5에서, Context 선언·provision·소비를 XXII.1.6에서
   입력으로 받습니다.
2. 속성 매핑은 capability registry 한 곳에서 관리합니다. Web 매핑과 modifier stack 선형화는 IX.11.19
   matrix와 IX.11.18 계획을 그대로 쓰고 다시 정의하지 않습니다. 코드 생성은 그 matrix에 Apple·Android
   lowering 규칙(XXII.1.4)과 Context·`MotionSpec` 항목(XXII.1.7)만 더합니다. 속성×TargetProfile마다
   결정 1.3의 상태와 낮추는 규칙을 기록하고, 1:1이 아닌 항목(modifier 적용 순서, 블렌드 합성 색공간
   차이, 자유 배치·constraint 변환, 기본값 차이)은 규칙과 진단을 함께 둡니다.
3. Context는 타깃별로 낮춥니다. Web은 CSS로 표현 가능한 스타일 값이면 등록된 custom property 상속으로,
   그 밖의 값은 생성 코드의 context 구조로 출력합니다. Apple은 Environment 값, Android는
   CompositionLocal로 출력합니다. 세부 규칙은 다음과 같습니다(XXII.2.5, XXII.3.4, XXII.4.3).
   - 등록 속성(`@property`)의 `inherits`는 토큰과 같은 정책을 씁니다(속성 모델 10장, `inherits: true`).
     정책은 한 곳에서만 정하고 Context가 따로 정하지 않습니다.
   - ContextKey의 CSS 이름은 토큰 이름 파생 규칙(IX.11.16)으로 만들고, 같은 export scope의 토큰
     이름과 한 이름 공간에서 충돌을 검사·진단합니다.
   - 같은 key를 다시 읽는 provision(3.5)을 `--k: f(var(--k))`처럼 내보내면 CSS에서 자기 참조 cycle이
     되어 guaranteed-invalid 값이 됩니다. 그래서 그 key의 provision마다 provision 깊이를 붙인 이름의
     custom property에 값을 쓰고 key 이름으로 다시 공급하며, 값 식 안의 같은 key 읽기는 한 단계 바깥
     깊이 이름으로 바꿉니다. 깊이가 instance 위치마다 달라 규칙 하나로 낼 수 없으면 그 key를 생성
     코드 context 구조로 내보내고 정보 진단을 냅니다.
   - Web CSS 상속, Apple Environment, Android CompositionLocal은 모두 렌더 트리로 값을 전달합니다.
     그래서 slot 삽입 위치에서 Slot이 공개하지 않은 key를 instance 바깥 값으로 다시 공급해 결정
     3.11과 같은 결과를 냅니다.
4. 상태 머신과 `MotionSpec`은 타깃별로 낮춥니다. Web은 CSS transition·animation으로 표현 가능한
   motion만 CSS로 내보내고, 상태 전환은 생성 코드로 내보냅니다. Apple·Android는 각 dialect의 상태·
   animation API로 내보냅니다. spring처럼 표현할 수 없는 항목은 근사 또는 미지원 진단을 냅니다.
5. 생성 코드에 스크립트가 들어가는 것은 출력물의 일이고, 문서에는 실행 코드를 넣지 않는다는 원칙과
   충돌하지 않습니다. 생성 코드는 다시 가져오지 않습니다(XXII.1.3, XXII.5.3).

### 5. 경계

- 결정론: 그래프 projection, 자동 배치, 평가, 코드 생성 모두 같은 입력에서 byte 동일 결과를 냅니다.
- 단일 writer: 그래프 편집, Context 편집, MotionSpec 편집은 모두 DocumentThread의 CommandTransaction입니다.
  프리뷰 런타임은 문서를 쓰지 않습니다.
- 닫힌 node 집합: 프로토타입 node 종류는 registry에 등록된 것만 있습니다. 문서에 JavaScript, 파일·
  네트워크 접근, 사용자 정의 코드 node를 두지 않습니다. 확장이 필요하면 Phase XXI 플러그인 경계에서
  다루고 문서 의미에는 넣지 않습니다.
- 캡슐화: nested instance 내부 직접 연결·편집 금지(IX.4.2, IX.5.6)를 그대로 지킵니다.

### 6. 릴리스 범위

**결정(2026-11-14 자율 판단):** 제품 소유자 지시(P4: 노드 프로토타이핑·drilling·context·코드 변환은
"추후" 지원)에 따라 Component Context 기능은 로드맵에 둡니다. 처음 안은 저장 형식을 파일 포맷 공개
(Phase XXVIII) 전에 고정해야 한다는 이유로 Context 전체를 초기 릴리스(Phase IX)에 넣었습니다. 하지만
Phase XXVIII 자체가 로드맵이고, 포맷 공개 전에 필요한 것은 나중에 새 record·variant를 더할 번호 공간을
예약해 두는 일뿐입니다. 그래서 초기 릴리스에는 번호 예약만 둡니다.

- **초기 릴리스(Phase IX):** Component Context 저장 번호와 확장 위치의 예약만 둡니다(IX.12.1).
  ReferenceTarget Context tag, `ContextKey`·`ContextProvision` record의 chunk kind 또는 record kind,
  ComponentDefinition의 `ContextRequirement` 목록과 Slot 공개 Context key 목록의 확장 위치가
  대상입니다. 기능 동작과 기존 저장 bytes는 바꾸지 않습니다. 파일 포맷 동결(XXVIII.3.1)은 이 예약을
  선행으로 둡니다.
- **로드맵(Phase XXX):** Component Context의 모델, validator, 해석, command, 출처 조회 API, macOS
  인스펙터 연결(XXX.7). 노드 기반 프로토타이핑 편집기의 모델(`MotionSpec`, 배치 hint, 제스처 이벤트),
  그래프 projection·편집, 결정론적 평가, 프리뷰, macOS UI(XXX.1\~XXX.6).
- **로드맵(Phase XXII 조정):** CSS 기반 속성 registry, Context, 상태 머신, `MotionSpec`을 소비하는 코드
  생성. CSS 기반 속성·레이아웃을 낮추는 행(XXII.1.1\~XXII.1.4, XXII.2.1\~XXII.2.4, XXII.3.1\~XXII.3.3,
  XXII.4.1, XXII.4.2)은 Phase XXX를 기다리지 않습니다. 상태 머신·`MotionSpec`·Context를 낮추는 행
  (XXII.1.5\~XXII.1.7, XXII.2.5, XXII.2.6, XXII.3.4, XXII.4.3)과 XXII.5는 `MotionSpec` record(XXX.1.1)와
  Context 해석기(XXX.7.4) 뒤에 진행합니다. 로드맵 Phase 번호는 진행 순서가 아닙니다.

## Alternatives

- **프로토타입 그래프를 독립 원본으로 저장:** 같은 의미가 binding·상태 머신과 그래프 두 곳에 생겨
  Semantic Diff·Merge에서 충돌이 두 번 나고 동기화 규칙이 필요합니다. 탈락했습니다.
- **CSS custom property처럼 암묵적으로 상속되는 Context:** 컴포넌트가 무엇에 의존하는지 인터페이스에
  나타나지 않아 §15.1 "암묵적 참조 금지"와 충돌하고, 라이브러리 사용자가 의존성을 알 수 없습니다.
  탈락했습니다.
- **Context 없이 drilling만 제공:** 테마·밀도처럼 트리 전체에 걸친 값을 모든 경계에 Property로 만들어야
  해서 10단계 nesting에서 중간 Property가 폭증합니다. 제품 방향(P4)과도 맞지 않아 탈락했습니다.
- **Component Context 전체를 초기 릴리스(Phase IX)에 포함:** 제품 소유자가 "추후" 지원한다고 한 범위를
  초기 릴리스로 넓히고, 파일 포맷 공개(Phase XXVIII)도 로드맵이라 근거가 되지 않습니다. 번호 예약만으로
  나중에 호환되게 더할 수 있어 탈락했습니다.
- **slot 콘텐츠를 렌더 트리 기준으로 해석:** View 최상위에서 작성한 콘텐츠가 컴포넌트 내부 provision을
  선언 없이 읽게 되고, 컴포넌트 내부 provision을 바꾸면 바깥에서 작성한 콘텐츠의 결과가 인터페이스
  변경 없이 바뀝니다. §15.1의 명시성·캡슐화와 충돌해 탈락했습니다. 렌더 트리 기준이 필요한 key는 Slot
  공개 목록으로 명시합니다(결정 3.11).
- **spring을 닫힌 식으로 평가:** `exp`·`cos` 같은 libm 초월함수 결과는 ADR-0006 기준으로 플랫폼
  의존이라 reference corpus bytes 동일을 보장할 수 없습니다. 탈락했습니다.
- **프로토타입 동작을 JavaScript로 작성:** 결정론, 닫힌 표현식, Semantic Diff 원칙(§14.1)과 충돌합니다.
  탈락했습니다.
- **wall clock 기반 애니메이션 평가:** 같은 입력에서 결과가 달라져 재현 fixture를 만들 수 없습니다.
  탈락했습니다.
- **CSS 문자열을 그대로 저장:** 문서 열기 경로에 CSS parser가 필요하고, 타입 검사와 속성 단위 merge가
  어려워집니다. 탈락했습니다.

## Consequences

- 긍정: 속성 하나가 값 연결, 그래프 포트, 코드 생성 매핑에서 같은 식별자를 써서 기능 간 번역 계층이
  줄어듭니다.
- 긍정: 프로토타입 그래프가 projection이므로 기존 Semantic Diff·Merge·Undo·에이전트 편집 경로를 그대로
  씁니다.
- 긍정: Context가 인터페이스에 선언되므로 라이브러리 컴포넌트의 의존성이 보이고, 출처 표시로 디버깅할
  수 있습니다.
- 부정: Context 도입으로 ReferenceTarget, provenance, merge conflict 종류가 늘어납니다. 초기 릴리스에는
  번호 예약(IX.12.1)만 들어가지만, 예약한 번호는 Context 구현 전까지 다른 용도로 쓸 수 없습니다.
- 부정: Slot 인터페이스에 공개 Context key 목록이 늘고, 렌더 트리로 값을 전달하는 타깃으로 낮출 때 slot
  삽입 위치의 재공급 코드가 생깁니다.
- 부정: EventKind 확장과 `MotionSpec`은 numeric registry·저장 형식 변경이라 각각 ADR이 더 필요합니다.
- 부정: spring timing function처럼 Web CSS로 표현할 수 없는 항목은 근사 진단이 생깁니다.

## Compatibility

- 이 ADR 자체는 파일 포맷, 공개 API, FFI를 바꾸지 않습니다.
- 구현 시 바뀌는 것: ReferenceTarget에 Context 종류 추가, `ContextKey`·`ContextProvision` record 추가,
  ComponentDefinition의 `ContextRequirement` 목록과 Slot의 공개 Context key 목록 추가(번호는 IX.12.1
  예약, 구현은 XXX.7), `MotionSpec`·배치 hint record와 EventKind 확장(XXX.1). 새 record·variant이거나
  기존 record에 붙는 목록이며 목록이 비면 지금과 같은 의미입니다. 기존 record 의미는 바꾸지 않습니다.
  각 형식은 착수 시 ADR과 storage schema version 정책(V.8)을 따릅니다.
- 아키텍처 §14.3은 좁힙니다. "경계를 넘는 값은 공개 Component Property로만"과 "여러 경계를 통과하면
  모든 경계에 Property"는 Property 값에만 적용하고, 선언된 `ContextRequirement`(§14.5)를 명시 예외로
  둡니다. §15.1의 "컴포넌트 경계를 건너는 암묵적 참조 금지"는 그대로 유지합니다. Context는 선언된
  인터페이스이므로 암묵적 참조가 아닙니다.

## Performance

- Context 해석은 binding graph의 node로 들어가며, 해석 결과를 바꾸는 변경(provision·default 변경,
  소비 node나 조상의 이동, instance 교체, slot 콘텐츠·Slot 공개 목록 변경, View Variant 구조 patch)에서
  영향받는 소비 binding만 다시 평가합니다(VIII.6.4, XXX.7.6). 해석용 조상 탐색과 slot 경계 건너뛰기는
  런타임 index로 캐시하고 매 평가마다 트리를 다시 걷지 않습니다.
- 프로토타입 프리뷰는 tick마다 활성 motion과 영향받는 binding만 평가합니다. 편집 인터랙션 예산(§6)은
  프리뷰 중에도 유지하며 REF-MAC 게이트로 확인합니다.
- 그래프 projection은 열린 그래프만 만들고 저장하지 않습니다.

## Security

- 문서에 실행 코드를 넣지 않으므로 신뢰되지 않은 프로젝트(§37.1)를 열어도 프로토타입 그래프가 파일·
  네트워크·프로세스에 접근하지 않습니다.
- dispatch 깊이 상한과 evaluator instruction budget(VIII.4.5)이 악의적인 펄스 연쇄·거대 그래프를 제한합니다.
- 생성 코드에 들어가는 식별자와 문자열은 XXII.5.1의 escape 규칙을 따릅니다.

## Migration

- 기존 문서에는 Context·MotionSpec record가 없으므로 데이터 마이그레이션이 필요 없습니다.
- 계획서: Phase IX에는 IX.12.1(Context 저장 번호 예약)만 추가합니다. 로드맵 Phase XXX에 노드 기반
  프로토타이핑(XXX.1\~XXX.6)과 Component Context(XXX.7)를 추가합니다. 착수 전인 Phase XXII 행(XXII.1.1,
  XXII.2.2, XXII.2.3, XXII.3.2, XXII.4.2)을 CSS 기반 속성 모델(IX.11)을 소비하도록 그 자리에서 고치며
  새 행을 추가합니다. XXVIII.3.1은 IX.12.1을 선행으로 둡니다.
- 아키텍처 문서: §4.2 로드맵 범위, §14.3 경계 규칙의 적용 범위, §14.5 Component Context, §17.3 코드
  생성 입력을 갱신합니다. §4.1 초기 범위에는 Context를 넣지 않습니다.
