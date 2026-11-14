# ADR-0025: CSS 기반 속성 위의 노드 프로토타이핑·Component Context·코드 생성 방향

## Status

Accepted (2026-11-14, 방향 결정). 제품 소유자 방향 P4(2026-11-14)를 기록합니다. 저장 형식의 tag 번호,
numeric registry 값, 공개 API 시그니처는 이 ADR에서 고정하지 않습니다. 각 구현 행이 착수할 때 별도
ADR로 고정합니다(계획 2장 검토 항목 5).

## Context

제품 소유자는 2026-11-14에 다음 방향을 정했습니다.

- 인스펙터에서 편집하는 디자인 속성은 기본적으로 CSS 기반입니다(P3). CSS 속성 어휘, 값 문법, 레이아웃
  의미(box model, Flexbox, Grid, positioning, typography, background·border, effect, transform,
  opacity·blend)를 따릅니다. CSS 기반 속성 어휘와 저장 표현 자체는 이 ADR의 범위가 아닙니다.
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
  충돌합니다.
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
2. **노드 그래프 포트.** 프로토타입 그래프에서 속성은 타입 있는 입력 포트로 나타납니다. 포트 타입은 속성의
   `ExpressionType`입니다.
3. **코드 생성 매핑 단위.** Target Profile capability registry(IX.10.5)는 CSS 기반 속성마다
   Native·Approximated·Unsupported·Deprecated를 기록합니다. Web adapter는 같은 이름의 CSS 속성으로
   출력하고, Apple·Android adapter는 이 registry를 거쳐 SwiftUI·Compose 표현으로 낮춥니다.

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
   tick으로 대응시키기만 합니다. 같은 이벤트 로그와 tick 열은 같은 값 trace를 냅니다. tick 안의 펄스는
   정해진 순서로 처리하고, 상태 변경은 IX.5.5처럼 원자적으로 확정합니다. 펄스 연쇄에는 dispatch 깊이
   상한을 두고, 넘으면 진단합니다.
8. **프리뷰는 문서를 쓰지 않습니다.** 프리뷰 상태, 진행 중 motion, 입력 로그는 세션 메모리에만 있습니다
   (IX.5.4와 같음). 프리뷰를 조작한 뒤에도 save bytes는 같습니다.
9. **reduced motion.** 환경 key `reduced_motion`(VIII.7.1)이 켜져 있으면 `MotionSpec`이 정의한 대체
   동작(기본은 즉시 전환)을 씁니다.
10. **포인터 제스처 이벤트.** hover, pointer down·up, drag, scroll, long press는 현재 EventKind(IX.5.2)에
    없습니다. EventKind 확장은 numeric registry 변경이므로 구현 행에서 별도 ADR로 고정합니다. 제스처
    판정 임계값(long press 시간 등)은 논리 tick 단위로 registry에 고정합니다.
11. **그래프 편집기 화면.** 노드 그래프 편집 면은 표준 AppKit 구성요소로 만들 수 없는 디자인 도구 고유
    요소입니다. macOS 셸 디자인 가이드 1.2 원칙 2·3에 따라, 이 ADR은 그래프 편집 면을 캔버스와 같은
    방식(코어 렌더러 scene)으로 그리는 것을 승인합니다. 포트 목록, 속성 편집, 메뉴, 툴바, 다이얼로그는
    네이티브 컨트롤을 씁니다. 배치는 Xcode 같은 macOS 개발 도구의 편집기 영역 분할(캔버스와 그래프를 나란히)을
    기준점으로 삼되, 다른 제품의 UI 문구와 시각 요소는 옮기지 않습니다.

### 3. Component Context

Context는 경계를 암묵적으로 넘는 참조가 아닙니다. **선언된 인터페이스**입니다.

1. **`ContextKey`.** StableId, 이름, TypeSpec, 필수 default expression을 가진 resource입니다.
   default는 ComponentProperty처럼 닫힌 식이어야 합니다(IX.1.2). scope는 토큰처럼 프로젝트·라이브러리·
   문서이고, 참조는 StableId로 합니다(VIII.5.2).
2. **제공(provide).** `ContextProvision`은 node 하나에 붙어 key 하나에 값 식 하나를 연결합니다. 효과
   범위는 그 node의 하위 트리이며, instance expansion view(IX.2.2)를 따라 펼친 트리를 기준으로 합니다.
   상위 ComponentDefinition은 자기 트리 안의 instance node에 provision을 붙일 수 있습니다. 이것은
   자기 scope 안의 편집이므로 캡슐화를 침범하지 않습니다.
3. **소비 선언(require).** ComponentDefinition 안에서 Context를 읽으려면 그 정의의 공개 인터페이스에
   `ContextRequirement`(key 참조)를 선언해야 합니다. Property처럼 인스펙터와 라이브러리 정보에
   나타납니다. 그래서 컴포넌트가 무엇을 받는지는 명시적이고(§15.1 유지), 중간 컴포넌트는 key를 다시
   선언하지 않아도 됩니다(drilling과 다른 점). ViewDefinition 최상위 트리는 컴포넌트 경계가 아니므로
   선언 없이 읽습니다.
4. **소비(consume).** `ValueBinding` source expression이 Context 참조를 읽습니다. ReferenceTarget에
   Context 종류를 추가해야 하며, 이것은 저장 형식 변경이므로 구현 행에서 별도 ADR로 고정합니다.
5. **해석 규칙.** 소비 지점에서 펼친 트리를 위로 올라가며 가장 가까운 provision이 이깁니다. 없으면
   `ContextKey` default를 씁니다. 안쪽 provision이 바깥 provision을 그 하위 트리에서 가립니다(shadowing).
   provision의 값 식이 같은 key를 읽으면 바깥(조상) 값으로 해석합니다. 서로 다른 key가 같은 node나
   조상·자손 관계에서 서로를 읽어 만드는 cycle은 binding cycle detector(VIII.6.3)가 저장 전에 거부합니다.
6. **override와 결합.** provision 값은 PropertyPath로 주소를 가지므로 View Variant(IX.7.3)와 Target
   Profile override(IX.10.7)가 기존 우선순위 규칙대로 덮어쓸 수 있습니다.
7. **진단.** 선언 없는 소비(오류), 타입 불일치(오류), 존재하지 않는 key의 provision(orphan), 쓰지 않는
   선언(경고), 기본값으로 해석됨(정보), shadowing(정보)을 냅니다. `ContextKey`를 삭제하면 소비 binding은
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

### 4. 실제 코드 변환

1. CodegenIR(XXII.1.1)은 CSS 기반 속성 record, Context 선언·provision·소비, 상태 머신, `MotionSpec`을
   입력으로 받습니다.
2. 속성 매핑은 capability registry 한 곳에서 관리합니다. 속성×TargetProfile마다 Native·Approximated·
   Unsupported와 낮추는 규칙을 기록합니다. modifier stack 선형화, 블렌드 합성 색공간 차이, 자유 배치
   anchor→CSS inset 변환처럼 1:1이 아닌 항목은 규칙과 진단을 함께 둡니다.
3. Context는 타깃별로 낮춥니다. Web은 CSS로 표현 가능한 스타일 값이면 등록된 custom property 상속으로,
   그 밖의 값은 생성 코드의 context 구조로 출력합니다. Apple은 Environment 값, Android는
   CompositionLocal로 출력합니다.
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

- **초기 릴리스(Phase IX):** Component Context의 모델, validator, 해석, command, 인스펙터용 출처 조회
  API(IX.12). Context는 문서 모델과 저장 형식에 들어가므로 파일 포맷 공개(Phase XXVIII) 전에 고정합니다.
  인스펙터 화면 연결은 Phase XII 이후 행입니다.
- **로드맵(Phase XXX):** 노드 기반 프로토타이핑 편집기의 모델(`MotionSpec`, 배치 hint, 제스처 이벤트),
  그래프 projection·편집, 결정론적 평가, 프리뷰, macOS UI.
- **로드맵(Phase XXII 조정):** CSS 기반 속성 registry, Context, 상태 머신, `MotionSpec`을 소비하는 코드
  생성.

## Alternatives

- **프로토타입 그래프를 독립 원본으로 저장:** 같은 의미가 binding·상태 머신과 그래프 두 곳에 생겨
  Semantic Diff·Merge에서 충돌이 두 번 나고 동기화 규칙이 필요합니다. 탈락했습니다.
- **CSS custom property처럼 암묵적으로 상속되는 Context:** 컴포넌트가 무엇에 의존하는지 인터페이스에
  나타나지 않아 §15.1 "암묵적 참조 금지"와 충돌하고, 라이브러리 사용자가 의존성을 알 수 없습니다.
  탈락했습니다.
- **Context 없이 drilling만 제공:** 테마·밀도처럼 트리 전체에 걸친 값을 모든 경계에 Property로 만들어야
  해서 10단계 nesting에서 중간 Property가 폭증합니다. 제품 방향(P4)과도 맞지 않아 탈락했습니다.
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
- 부정: Context 도입으로 ReferenceTarget, provenance, merge conflict 종류가 늘어 Phase IX 작업량이
  늘어납니다.
- 부정: EventKind 확장과 `MotionSpec`은 numeric registry·저장 형식 변경이라 각각 ADR이 더 필요합니다.
- 부정: spring timing function처럼 Web CSS로 표현할 수 없는 항목은 근사 진단이 생깁니다.

## Compatibility

- 이 ADR 자체는 파일 포맷, 공개 API, FFI를 바꾸지 않습니다.
- 구현 시 바뀌는 것: ReferenceTarget에 Context 종류 추가, `ContextKey`·`ContextProvision`·
  `ContextRequirement` record 추가(IX.12), `MotionSpec`·배치 hint record와 EventKind 확장(Phase XXX).
  모두 새 record 또는 새 variant 추가이며, 기존 record 의미는 바꾸지 않습니다. 각 형식은 착수 시 ADR과
  storage schema version 정책(V.8)을 따릅니다.
- 아키텍처 §14.3·§15.1의 "암묵적 참조 금지"는 유지됩니다. Context는 선언된 인터페이스이므로 예외가
  아니며, §14.5에 규칙을 추가합니다.

## Performance

- Context 해석은 binding graph의 node로 들어가며, provision 변경 시 영향받는 소비 binding만 다시
  평가합니다(VIII.6.4). 해석용 조상 탐색은 런타임 index로 캐시하고 매 평가마다 트리를 다시 걷지 않습니다.
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
- 계획서: Phase IX에 IX.12 Component Context, 로드맵 Phase XXX 노드 기반 프로토타이핑을 추가하고, 착수
  전인 Phase XXII 행(XXII.1.1, XXII.2.2)을 CSS 기반 속성 모델을 소비하도록 그 자리에서 고치며 새 행을
  추가합니다.
- 아키텍처 문서: §4.1·§4.2 범위, §14.5 Component Context, §17.3 코드 생성 입력을 갱신합니다.
