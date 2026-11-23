# ADR-0094: 노드 기반 프로토타이핑·MotionSpec 코어

## Status

Accepted (2026-11-23). ADR-0025 결정 2(노드 기반 프로토타이핑)가 구현 행에 미룬 저장 형식, numeric registry 값,
공개 API를 고정합니다. 대상 행은 XXX.1.1\~XXX.1.6, XXX.2.1\~XXX.2.5, XXX.3.1\~XXX.3.6과 XXX.6.3의 엔진 부분입니다.
UI(XXX.4, XXX.5)와 셸 FFI는 범위 밖입니다(2026-11-23 사용자 방향: 엔진·기능 먼저).

## Context

- ADR-0025 결정 2는 프로토타입 그래프를 새 원본이 아닌 projection으로, 시간축을 `MotionSpec`으로, 평가를 논리 시계의
  순수 함수로 정했습니다. tag 번호·저장 형식·API는 구현 행에서 정하기로 했습니다.
- 속성 어휘는 CSS longhand입니다(ADR-0024, ADR-0074, ADR-0075). Auto Layout·Fixed/Fill/Hug 같은 비CSS 개념을 만들지
  않습니다. 레이아웃 longhand와 문법은 ADR-0086의 layout record가 정합니다.
- 텍스트 원본은 ADR-0077 v4 규칙(한 줄 = 영속 값 하나, CSS 값 정규형, 정규형이 아닌 입력 거부)을 씁니다.
- 결정론(§3.1)과 ADR-0006: 평가 경로는 IEEE-754 기본 연산만 쓰고 libm 초월함수를 쓰지 않습니다.
- Component Context(XXX.7)는 아직 없습니다. Context 읽기 node·ContextRequirement 공개·Context 진단은 그 행들이 원본을
  만든 뒤에 연결합니다.
- 이 작업의 소유 범위는 `crates/document/src/{motion,prototype}`, 새 crate `crates/prototype`, `crates/commands/src/{motion,prototype}*`
  입니다. `crates/document/src/component/event.rs`의 EventKind 확장(XXX.1.4)은 행 정의가 요구하는 같은 파일의 추가이므로
  여기서 함께 합니다.

## Decision

### 1. `MotionSpec` record (XXX.1.1)

1. **CSS `transition` 어휘.** `MotionSpec`은 대상 속성 목록(`transition-property`), 지속 시간(`transition-duration`),
   지연(`transition-delay`), timing function(`transition-timing-function`)을 가집니다. 시작 조건만 CSS와 다릅니다. CSS는
   "계산 값이 바뀜"이지만 Stellar는 상태 변수 하나의 값 조건입니다.
   - `Becomes { variable, value }`: 값이 `value`가 아니다가 `value`가 되는 tick.
   - `Changes { variable }`: 값이 바뀌는 모든 tick.
2. **소유자.** `Component(ComponentRef)`(대상은 정의 tree node) 또는 `View(StableId)`(대상은 View 문서 node). 대상
   경로는 key 하나·tail 없는 `PropertyPath`이고, 중복은 거부합니다. 대상 수 상한은 64입니다.
3. **보간 대상 타입.** Number, Length, Percentage, Angle, Duration, Color만 보간합니다. Boolean·String·Enum·Gradient·
   Shadow·Typography·ResourceRef는 보간하지 않습니다. 생성·변경 시 대상 포트 타입 집합(결정 2)에 보간할 수 있는 타입이
   하나도 없으면 거부합니다(`display` keyword, `visible` 등).
4. **끝값 규칙.** 보간은 같은 타입 끝값 사이에서만 합니다. 합집합 포트(`width`의 keyword·Length·Percentage)에서 끝값
   타입이 다르거나, Color 끝값의 색공간(sRGB·Display P3)이 다르면 보간하지 않고 끝값으로 바로 바꾼 뒤 진단합니다
   (`motion-endpoint-type`, `motion-color-space-mismatch`). Color는 ADR-0025 결정 1.2대로 저장 색공간의 encoded 채널을
   alpha로 곱해(premultiplied) 선형 보간하고 결과 alpha로 나눕니다. 결과 alpha가 0이면 채널은 0입니다. 진행률이 [0, 1]을
   벗어나면(overshoot) alpha는 [0, 1]로 자릅니다.
5. **parameter 범위.** 지속 시간 `0..=3_600_000ms`, 지연 절댓값 `3_600_000ms` 이하(음수는 중간부터 시작, CSS와 같음).
   `cubic-bezier`의 x1·x2는 `[0, 1]`, y1·y2 절댓값은 1000 이하. `steps` 개수는 1\~10000(`jump-none`은 2 이상).
   `spring`은 질량 > 0, 강성 > 0, 감쇠 ≥ 0, 강성/질량 ≤ 10⁶, 감쇠/질량 ≤ 1000, 초기 속도 절댓값 ≤ 1000, 지속 시간 ≤
   10000ms입니다. 강성·감쇠 상한은 1ms 고정 step 적분이 안정한 범위입니다. 모든 값은 유한해야 하고 `-0`은 `0`으로 씁니다.
6. **timing function(닫힌 집합).** `linear`, `cubic-bezier(x1, y1, x2, y2)`, `steps(n, jump-start|jump-end|jump-none|jump-both)`,
   `spring(mass stiffness damping velocity)`. CSS 이름 별칭(`ease`, `start`)은 저장하지 않고 정규 함수형만 씁니다.
   spring은 CSS에 없는 Stellar 값이며, Web 출력에서는 `linear()` 근사가 필요합니다(XXII.1.7, Approximated).
7. **reduced motion 대체 동작(닫힌 집합).** `instant`(기본, 끝값으로 즉시)와 `play`(그대로 재생: 움직임 없는 색·불투명도
   전환용).

### 2. socket 종류와 속성 포트 타입 (XXX.1.2)

1. socket 종류는 State(tag 0x01)와 Pulse(0x02)입니다. 같은 종류끼리만 바로 연결하고, 종류를 바꾸는 것은 등록된 변환
   node(결정 3)뿐입니다.
2. 속성 입력 포트의 State 타입은 CSS 값 문법을 `ExpressionType`으로 바꾼 집합입니다.

   | 문법 | 타입 집합 |
   |---|---|
   | `<length>` | Length |
   | `<percentage>` | Percentage |
   | `<angle>` | Angle |
   | `<time>` | Duration |
   | `<number>`, `<alpha-value>`, `<integer>` | Number |
   | `<color>` | Color |
   | keyword | Enum(그 longhand keyword) |
   | boolean(Stellar) | Boolean |
   | string(Stellar) | String |
   | layout `auto \| <length> \| <percentage>`(inset) | Enum(`auto`), Length, Percentage |
   | layout 크기(`width`·`height`·`min-*`) | Enum(`auto min-content max-content fit-content`), Length, Percentage |
   | layout `max-*` | Enum(`none min-content max-content fit-content`), Length, Percentage |
   | layout `flex-basis` | Enum(`auto content`), Length, Percentage |
   | layout gap | Enum(`normal`), Length |
   | `<transform-list>`, `border`·`border-radius` shorthand, `<track-size>`, grid line, `aspect-ratio`, 값 문법 없음 | 노출 안 함(`port-unmapped-grammar`, `port-no-value-grammar`) |

3. 포트 하나는 CSS longhand 하나입니다. payload 크기 key(`width` 0x0108 등)는 저장 위치가 달라도 layout longhand
   `width`의 문법을 씁니다. 같은 값을 다른 CSS 속성으로 바꿔 쓰는 항목(`visible` → `display`)은 자기 문법을 씁니다.
   Frame·Image의 record key `width`·`height`(0x0016·0x0017)는 payload가 저장 위치이므로 포트가 아닙니다.
4. keyword Enum 타입 번호는 `0xC000_0000 | longhand 번호`입니다. layout longhand는 `LayoutLonghand::id`, 그 밖은 registry
   key 값입니다. variant는 keyword 목록 index입니다. `0xE000_xxxx`(환경 입력 enum)와 겹치지 않습니다.
5. 묶음 key(`size`, `border-radius` 묶음 등)는 나뉜 longhand 포트로만 연결합니다(`port-bundle-key`). node 공통의
   편집기 전용 필드(`locked`, `name`)는 포트가 아닙니다(`port-editor-metadata`).
6. 연결 검사는 식의 정적 타입이 집합에 속하는지만 봅니다. nullable 식은 거부합니다. 통과한 정적 타입이 binding의 기대
   타입(저장 값 variant)입니다. 저장된 값 variant로 포트 타입을 바꾸지 않습니다.
7. 전체 포트 표는 `crates/document/src/prototype/port_golden.txt` golden으로 고정합니다.

### 3. node 종류 registry (XXX.1.3)

| kind | tag | socket |
|---|---|---|
| PropertyOutput | 0x01 | in `value`(State), in `motion`(State) |
| ExpressionBlock | 0x02 | in `operand`(State, slot별), in `reference`(State), out `result`(State) |
| StateVariable | 0x03 | in `write`(Pulse, transition별), out `value`(State) |
| EventSource | 0x04 | out `event`(Pulse, 종류·nested action별), out `payload`(State, 종류별) |
| Transition | 0x05 | in `trigger`(Pulse), in `condition`(State), in `set-value`(State, action별), out `set`(Pulse), out `emit`(Pulse) |
| SemanticEmit | 0x06 | in `emit`(Pulse, transition별), out `event`(Pulse) |
| ContextRead | 0x07 | out `value`(State) |
| MotionSpec | 0x08 | in `start`(State), out `drive`(State, 대상별) |
| Conversion | 0x09 + 변환 tag | StateChangeToPulse 0x01(State→Pulse), PulseToggle 0x02·PulseCounter 0x03(Pulse→State) |

닫힌 enum이므로 사용자 정의 코드·JavaScript·파일·네트워크 node는 표현할 수 없고, 모르는 tag는 읽기에서 거부합니다.
ContextRead는 Context 원본(XXX.7.4)이 생기기 전에는 projection에 나타나지 않습니다. StateChangeToPulse는 지금 상태 머신
(IX.5)에 상태 변화 trigger가 없어 낮출 곳이 없으므로 편집 plan이 `Unsupported`로 거부합니다. 토글은 `상태 = not 상태`,
카운터는 `상태 = 상태 + 1` transition으로 낮춥니다.

### 4. 편집 command와 transaction (XXX.1.6, XXX.2.3, XXX.2.4)

1. `MotionCommand`(`Create`·`Replace`·`Delete`·`Restore`)는 document 계층 closed enum command입니다(Component Property
   command와 같은 방식). `Create`·`Replace`는 대상·시작 조건을 검사하고, `Restore`(inverse 전용)는 검사 없이 되살립니다.
2. 대상 node·속성이 삭제되어도 MotionSpec을 지우지 않습니다. 남은 spec은 `motion-orphan-target`으로 진단합니다.
3. 그래프 편집 하나는 `PrototypeTransaction` 하나입니다. step은 binding command(VIII.6.6), MotionSpec command, Component
   Property command(IX.1.3), 상태 머신 교체(IX.5, 이전 상태 머신이 inverse), 배치 hint 고정입니다. 실패하면 적용 전
   snapshot으로 되돌리고, 성공하면 inverse transaction 하나(Undo 1회)를 돌려줍니다. Phase VII `CommandTransaction`
   journal에 이 step을 영속 command로 등록하는 일은 영속 command registry(ADR-0079)가 맡습니다.
4. 포트 공개는 계획(미리보기 목록 + transaction)을 먼저 만들고 하나의 transaction으로 적용합니다. 속성 입력 공개는
   공개 Component Property 생성 + 대상 binding을 그 Property 참조로 바꾸기이고, 이벤트 공개는 semantic action 선언 +
   그 이벤트에 action을 emit하는 transition입니다. ContextRequirement 공개는 XXX.7.8 전까지 `Unsupported`입니다.
5. 다른 컴포넌트 정의 안(nested instance 내부)의 포트에 직접 잇는 edge는 `NestedInternalPort`로 거부합니다.

### 5. 저장 형식 (XXX.1.1, XXX.1.5)

1. **MotionSet binary.** 빈 모음은 0 bytes. 그 밖은 `version u16(=1) ++ count u32 ++ spec*`(id 오름차순, 정수·f32 LE).
   spec은 `id[16] ++ owner tag u8(component 0x01, view 0x02) ++ owner id[16] ++ target count u16 ++ (root[16] ++ key u32)* ++
   start tag u8(becomes 0x01, changes 0x02) ++ variable[16] ++ (becomes: value len u16 ++ TypedValue bytes) ++ duration f32 ++
   delay f32 ++ timing tag u8(linear 0x01, cubic-bezier 0x02, steps 0x03, spring 0x04) ++ timing payload ++ reduced-motion tag
   u8(instant 0x01, play 0x02)`입니다. step 위치 tag는 jump-start 0x01, jump-end 0x02, jump-none 0x03, jump-both 0x04입니다.
   모르는 tag·버전, 비오름차순, 남는 bytes, 비유한·`-0` f32, 검증 실패를 거부합니다.
2. **배치 hint binary.** 빈 모음은 0 bytes. 그 밖은 `count u32 ++ (id[16] ++ x f32 ++ y f32)*`(id 오름차순). 좌표 절댓값은
   10⁷ 이하입니다. hint 없는 그래프의 저장 bytes에는 배치 정보가 없습니다.
3. **프로토타입 텍스트 원본 v1.** 소유자 하나의 MotionSpec과 hint를 한 파일에 씁니다. 형식은
   `docs/spec`에 별도 문서를 두지 않고 `crates/document/src/prototype/source.rs` module 문서와 golden
   (`source_golden.txt`)이 정합니다. 첫 줄 `stellar-prototype: 1`, `owner: component|view <id>`, `motion <id>` 아래
   두 칸 들여쓴 `transition-property`·`transition-duration`·`transition-delay`·`transition-timing-function`·`start`·
   `reduced-motion` 여섯 줄, 그 뒤 `hint <id> <x> <y>`입니다. 대상은 `<root>/<key 소문자 hex 네 자리>`입니다. 값은 문서 CSS
   값 formatter·parser 정규형이고, 다시 쓴 결과가 입력과 같지 않으면 거부합니다. MotionSpec과 hint가 모두 없으면 파일을
   쓰지 않습니다. 프로젝트 안 경로는 `design/prototype/<owner StableId>.stellar-prototype`로 정하고, 열기·저장 연결은
   텍스트 원본 codec(ADR-0077)과 journal(ADR-0078) 작업이 합니다.
4. **Semantic Diff 분류.** hint 변경은 표현 전용(`Presentation`)이고 MotionSpec 변경은 의미 변경입니다.
   `diff_hints`·`apply_hint_changes`로 hint 변경만 골라 적용(stage)할 수 있습니다. semantic-diff crate의 변경 목록에 이
   분류를 싣는 일은 semantic-diff 작업이 합니다.

### 6. 프리뷰 평가 (XXX.3.1\~XXX.3.6)

1. **논리 시계.** `PreviewClock`은 초당 120 tick 고정입니다. tick → ms는 `tick × 1000 / 120`(f64)이고, 디스플레이 프레임은
   정수 연산 `frame × 120 / display_hz`(내림)로 tick에 대응시킵니다. 평가 경로에는 시스템 시각·파일·네트워크·프로세스·
   환경 변수 접근이 없습니다(inventory test).
2. **timing evaluator.** 모든 중간 계산은 f64 사칙연산·비교·`floor`(정확한 연산)이고 결과 값은 저장 타입 f32로 한 번
   반올림합니다. `cubic-bezier`는 x(t) = p를 64회 고정 이분법으로 풀고, `steps`는 CSS Easing 1 공식(before flag 없음),
   `spring`은 끝값까지 거리 1을 1ms 고정 step semi-implicit Euler로 적분하고 step 사이는 선형 보간하며 지속 시간 끝에서
   정확히 1입니다. 지연 동안은 시작값을 유지합니다. reference corpus(`timing_corpus.txt`)가 bits를 고정합니다.
3. **tick 처리 순서.** (1) 입력 이벤트: 입력 순서대로 제스처 판정, 그 뒤 원천 StableId 순으로 tick 끝 판정. (2) 펄스
   dispatch: 너비 우선, 작업 사본에 하고 끝나면 한꺼번에 commit. emit된 action은 `PulseRouter`(기본: nested instance →
   부모의 `NestedAction`)가 다음 펄스로 잇습니다. 연쇄 깊이가 상한(기본 16)을 넘으면 진단하고 그 tick의 상태 변화를 모두
   버립니다. (3) MotionSpec 시작·재지정: 시작 조건이 성립한 spec(id 순)의 대상마다 지금 표시 중인 값을 시작값으로 잡습니다
   (진행 중이면 그 현재 값에서 재지정). 같은 대상을 여러 spec이 잡으면 id가 앞선 spec이 이깁니다. (4) binding 재평가:
   바뀐 상태 변수에 영향받는 binding만 scheduler(VIII.6.4)로 재평가하고 그 값을 (3)의 끝값으로 씁니다. (5) 보간: 진행 중인
   motion만 표본을 뽑습니다. 상태 변화가 없는 tick은 binding을 평가하지 않습니다.
4. **값의 뜻.** MotionSpec의 보간 값은 대상 속성의 표시 값입니다. 다른 binding이 그 속성을 읽으면 끝값(binding 결과)을
   읽습니다. CSS에서 transition 중인 값이 다른 속성의 계산에 쓰이지 않는 것과 같습니다.
5. **reduced motion.** 환경 key `reduced_motion`(VIII.7.1)이 켜지면 대체 동작이 `instant`인 spec은 끝값으로 바로 바꿉니다.
   그 key를 읽는 binding은 scheduler로 재평가합니다.
6. **세션 메모리.** 프리뷰 상태, 진행 중 motion, 입력 로그, 값 trace는 `PreviewSession` 안에만 있습니다. 세션은 문서
   원본을 공유 참조로만 읽으므로 저장 bytes를 바꿀 수 없습니다.

### 7. 포인터 제스처 EventKind (XXX.1.4)

1. 2D 값 타입을 더하지 않고 **축별 이벤트**를 씁니다. `TypedValue`·`ExpressionType`(core-types·expressions 보호 경로)을
   바꾸지 않아도 되고, 기존 Length 식·binding이 축 값을 그대로 씁니다.
2. 새 tag: PointerEnter 0x09, PointerLeave 0x0A, PointerDown 0x0B, PointerUp 0x0C, DragX 0x0D, DragY 0x0E, DragEndX 0x0F,
   DragEndY 0x10, ScrollX 0x11, ScrollY 0x12, LongPress 0x13. Drag는 drag 시작점에서의 누적 변위(Length px), DragEnd는 release
   velocity(Length, px/tick), Scroll은 그 입력의 변위입니다. 같은 tick 안에서는 x 다음 y입니다. 기존 0x01\~0x08은 바뀌지
   않습니다.
3. 판정 임계값: drag 시작은 누른 위치에서 한 축이라도 4px 이상, long press는 drag 없이 60 tick(500ms), release velocity
   창은 4 tick입니다. velocity는 창 안 tick 각각의 변위(그 tick 끝 위치 − 앞 tick 끝 위치, 입력 없는 tick은 0)를 f32로
   차례로 더해 tick 수로 나눈 평균이고, drag가 짧으면 누른 다음 tick부터 있는 tick 수로 나눕니다.
4. drag·long press 없이 뗀 탭은 `pointerUp` 다음 `activate`를 냅니다.

## Alternatives

- **2D 값 타입 추가(drag·scroll payload).** core-types `TypedValue`와 expressions `ExpressionType`의 tag를 늘려야 하고,
  모든 codec·연산 규칙·CSS 값 대응을 함께 바꿔야 합니다. 축별 이벤트로 같은 표현력을 얻으므로 택하지 않았습니다.
- **MotionSpec에 끝값을 직접 저장.** CSS `@keyframes`에 가깝지만 상태·binding이 이미 끝값을 정하므로 같은 값이 두 곳에
  생깁니다. 이중 경로를 피하려고 끝값은 binding 결과로 둡니다. keyframe 애니메이션(`animation-*`)은 후속 결정입니다.
- **닫힌 식 spring(감쇠 진동 공식).** `exp`·`cos`가 필요해 ADR-0006 기준 플랫폼 의존 결과가 되므로 고정 step 적분을
  씁니다.
- **cubic-bezier Newton 반복.** 기울기가 0인 곳에서 발산할 수 있어 고정 이분법을 씁니다(64회, 결정론).
- **Phase VII CommandKind에 MotionSpec command 등록.** 영속 command registry(ADR-0079)를 함께 바꾸는 전역 작업이라 이
  범위 밖입니다. document 계층 command + `PrototypeTransaction`으로 같은 원자성·inverse 규칙을 지킵니다.
- **새 crate 없이 document에 projection.** projection·프리뷰는 binding(`stellar-bindings`)을 읽어야 하는데 document는
  bindings에 의존하지 않습니다. `stellar-prototype`(core-types, expressions, document, bindings)을 새로 둡니다.

## Consequences

- 긍정: 프로토타입 그래프·MotionSpec·프리뷰가 UI 없이 Rust test로 검증됩니다. 저장 원본은 CSS 어휘이고 텍스트 원본에
  사람이 읽는 `transition-*` 줄로 남아 Git 협업과 AI 편집(CLI·MCP 연결 후)에 맞습니다.
- 부정: Context 관련 부분(ContextRead projection, ContextRequirement 공개, Context 진단)은 XXX.7 뒤로 남습니다. 상태 변화→펄스
  변환은 상태 머신에 trigger가 생기기 전까지 쓸 수 없습니다. MotionSpec·hint 원본 파일의 프로젝트 열기·저장 연결, Semantic
  Diff 분류 연결, CLI·MCP 명령은 후속입니다. 프리뷰 binding 평가는 루트 인스턴스 정의의 binding만 다룹니다.
- projection의 node StableId는 원본 StableId입니다. 서로 다른 원본이 같은 StableId를 쓰지 않는다는 문서 전역 유일성을
  전제합니다.

## Compatibility

- EventKind에 tag 0x09\~0x13을 더합니다. 기존 tag와 저장 bytes는 그대로입니다. 이 tag를 쓴 상태 머신은 이전 빌드에서
  읽으면 모르는 tag로 거부됩니다(앞으로 호환 없음, 새 기능 저장 형식과 같은 규칙).
- MotionSet·배치 hint·프로토타입 텍스트 원본은 새 형식(v1)이며 기존 파일에 영향이 없습니다.
- 보호 경로(core-types, storage, ffi, schemas, performance budget)는 바꾸지 않습니다. 셸 FFI를 더하지 않습니다.
- crate graph에 `stellar-prototype`(→ core-types, expressions, document, bindings)를 더합니다.

## Performance

- 상태 변화가 없는 tick은 binding 평가 0, 진행 중인 motion 수에 비례한 보간만 합니다.
- spring 표본 하나는 경과 ms만큼 step을 적분합니다(지속 시간 상한 10000ms → 최대 10⁴ step). 프리뷰 세션 전용 경로이며
  성능 예산 표(`config/performance-budgets.yaml`)는 바꾸지 않습니다.

## Security

- node 종류가 닫힌 집합이라 문서에 실행 코드·파일·네트워크 접근을 넣을 수 없습니다. inventory test가 엔진 소스에 시스템
  시각·파일·네트워크·프로세스·환경 변수 접근이 없음을 확인합니다.
- 텍스트 원본은 1 MiB 상한이고 모르는 줄을 거부합니다.

## Migration

- 기존 데이터 migration은 없습니다. 새 EventKind와 원본 형식은 추가만 합니다.
