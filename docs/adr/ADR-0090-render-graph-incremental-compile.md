# ADR-0090: render graph 증분 compile

## Status

Accepted (2026-11-23, lane render-incremental 구현 기록)

## Context

ADR-0084는 scene을 증분으로 만들게 했지만 render graph compile(XI.4.2\~XI.4.5)은 frame마다 snapshot 전체를 처리했습니다.
2만 가시 node 재생(`cargo xtask replay`)에서 compile·batching CPU 시간은 frame당 약 3.3ms였습니다. 아키텍처 §6.2 Render
Graph 예산(목표 0.75ms, 상한 1.25ms)을 넘었고, 상호작용 지연의 대부분이 이 구간이었습니다(ADR-0084 Performance).

같은 측정에서 비용은 두 곳에 나뉘었습니다.

- 효과 경계 계획(`plan_boundaries`, XI.4.4): node 전체를 한 번 더 걸으며 unit 목록을 만들었습니다. 약 1.3ms.
- draw 수집: node마다 clip stack을 처음부터 분석하고(`analyze_clip`, 목록 할당 포함), 분류를 계산했습니다. 약 1.3ms.

제약은 다음과 같습니다.

- compile 결과(`FramePlan`)의 의미는 바꾸지 않습니다. 증분 경로와 한 번짜리 compile은 같은 결과여야 합니다.
- renderer-core는 scene에만 의존합니다(crate graph). trace field(`render_graph.compile` 8개)는 diagnostics schema와
  맞춰 두었으므로 바꾸지 않습니다.
- Metal·Swift 코드는 바꾸지 않습니다(사용자 방향 2026-11-23).

## Decision

1. **`FrameCompiler`.** `stellar_renderer_core::pass_compiler::FrameCompiler`가 frame 사이 재사용 상태를 들고 있습니다.
   `compile_frame`은 상태를 남기지 않는 `FrameCompiler`로 같은 함수를 실행합니다. 그래서 compile 구현은 하나입니다.
   `graph_trace::compile_frame_traced_with`는 compiler를 받아 같은 trace를 남깁니다.
2. **재사용 단위: 효과 없는 subtree의 draw 구간.**
   - 합성 효과가 있는 표시 node(효과 node)가 subtree에 없으면, 그 subtree의 draw는 한 target에 연속으로 쌓입니다.
   - 그 구간은 subtree 자신과 문맥만으로 정해집니다. 문맥은 target 안 clip 시작 depth와 바깥 효과를 접은
     opacity·blend입니다.
   - key는 `(subtree root Arc 주소, clip 시작 depth, 접은 opacity bit, blend)`입니다. 다음 frame에서 같은 key를 만나면
     subtree를 내려가지 않고 구간을 붙입니다.
   - scene 증분 build는 바뀌지 않은 subtree를 같은 `Arc`로 공유합니다(ADR-0084 결정 3). 그래서 주소가 같으면 내용이
     같습니다. 항목은 `Arc`를 붙잡아 주소가 다른 node에 다시 쓰이지 않게 합니다.
   - 자식 없는 node는 구간을 남기지 않습니다. 다시 계산하는 비용이 조회 비용과 비슷하기 때문입니다.
   - 효과 node가 있는 subtree는 node 단위로 걷습니다. 그 안의 효과 없는 자식 subtree는 재사용합니다. layer id는
     pre-order 시작 순서라서, 효과 node를 건너뛰면 뒤쪽 layer 번호가 달라지기 때문입니다.
3. **효과 경계 결정은 필요할 때 subtree만.** 효과 node의 결정(XI.4.4)은 그 subtree의 unit만으로 정해집니다.
   compile은 바깥 효과 node를 처음 만날 때 `offscreen::plan_subtree_boundaries`로 그 subtree만 계획합니다.
   `plan_boundaries`도 root마다 같은 함수를 부릅니다. 같은 효과 node(같은 `Arc`)의 결정 목록은 다음 frame에 재사용합니다.
   효과 node가 없는 scene은 경계 계획 비용이 0입니다.
4. **clip 분석 memo(한 compile 안).**
   - clip stack은 구조 공유 목록입니다. 형제는 같은 stack을, 자식은 부모 stack에 clip 하나를 더한 stack을 씁니다.
   - `(stack 저장소 주소, 시작 depth)`별로 clip 결정과 scissor 정확 여부를 memo합니다.
   - memo가 없으면 `ClipAnalysisMemo`가 기록이 있는 바깥 prefix부터 안쪽 clip만 더 분석합니다. 한 단계 함수
     (`ClipAnalysis::push_clip`)를 `analyze_clip`과 함께 쓰므로 결과가 같습니다.
   - stack 깊이가 32를 넘으면 prefix memo 없이 바로 분석합니다(항목마다 복합 clip 목록을 복제하는 제곱 비용 방지).
   - 주소는 stack이 살아 있을 때만 식별자이므로 memo는 compile 한 번 동안만 씁니다.
   - `ClipPlan::Stencil`·`Mask`의 clip 목록은 `Arc<[usize]>`로 바꿨습니다. 재사용한 draw를 복제할 때 할당하지 않습니다.
5. **무효화.**
   - 재사용은 frame 문맥이 직전 compile과 bit 단위로 같을 때만 합니다. 문맥은 viewport 크기·world → device 변환,
     가시 후보 z 목록, 효과 경계 방식, clip capability입니다. 하나라도 다르면 그 frame은 전체 compile입니다.
   - 다음 frame용 상태는 문맥이 직전과 같은 frame에서만 남깁니다. Pan·Zoom처럼 frame마다 문맥이 바뀌는 동안에는 남겨도
     쓰이지 않기 때문입니다. 그래서 문맥이 바뀐 뒤 첫 frame과 그다음 frame은 전체 compile이고, 세 번째 frame부터
     재사용합니다.
   - 상태에는 직전 frame에서 쓰거나 만든 항목만 남습니다(frame 번호 표시 후 정리). 그래서 frame이 이어져도 계속
     늘지 않습니다. 다만 기록하는 frame은 자식 있는 subtree마다 자기 draw 구간을 따로 복사하므로, 그 frame의 상태는
     draw 수 × subtree 깊이에 비례합니다. 다음 재사용 frame에서 쓰이지 않은 안쪽 구간이 정리되면 바깥 구간만 남습니다.
   - compile이 실패하면 상태를 모두 버립니다.
6. **재사용 집계.** `FrameCompiler::last_reuse()`가 `CompileReuse`를 돌려줍니다. 값은 들어간 node 수, 재사용한 subtree·draw
   수, 재사용한 경계 결정 수, 문맥 변경 여부입니다. trace schema는 바꾸지 않았으므로 trace에는 넣지 않습니다.
7. **재생 runner.** `cargo xtask replay`의 core runner(XVI.2.2)는 `FrameCompiler` 하나를 재생 내내 씁니다. 앱 render
   thread도 같은 방식으로 쓰면 됩니다. 연결은 UI 재설계 때 합니다.

## Alternatives

- **draw 목록을 world 좌표로 두고 viewport를 GPU uniform으로만 적용:** Pan·Zoom에서도 재사용됩니다. 하지만 opaque 분류
  (device pixel 정렬), scissor 결정, layer 접기의 pixel 영역 판정이 모두 device 좌표에서 정해집니다. pass compiler와
  backend 계약을 다시 설계해야 해서 이번 범위에서 탈락했습니다.
- **정수 pixel 이동이면 이전 결과를 평행 이동:** f32 계산은 평행 이동에 대해 bit 단위로 같지 않습니다. 한 번짜리
  compile과 결과가 달라질 수 있어 탈락했습니다.
- **node마다 결과를 저장:** 바뀌지 않은 subtree도 node 수만큼 조회해야 합니다. subtree 단위 재사용보다 느려서
  탈락했습니다.
- **효과 node가 있는 subtree도 통째로 재사용:** layer id를 상대 번호로 바꾸고 합성 draw·layer 목록을 다시 매겨야 합니다.
  효과 node는 드물고, 그 안의 효과 없는 subtree는 이미 재사용하므로 탈락했습니다.

## Consequences

- 단일 node 편집 frame은 바뀐 경로의 node와 그 형제 subtree 수만큼만 처리합니다. 2만 node fixture의 drag는 frame마다
  약 4,000개(최상위 card 수)를 방문하고, 나머지 구간은 그대로 붙입니다.
- 남은 O(draw 수) 비용이 있습니다. draw 순위 매기기, opaque·transparent 분리·정렬, batching입니다. 6,900 draw에서 약
  0.2ms입니다.
- Pan·Zoom은 frame마다 전체 compile입니다. clip memo와 지연 경계 계획 덕분에 이전보다 약 3배 빠르지만, §6.2 목표
  0.75ms보다는 느립니다(아래 측정). 남은 비용은 node 수에 비례하는 수집 순회입니다. 목표까지 줄이려면 viewport에
  독립인 draw 표현(대안 1)이 필요합니다. 이 결정은 renderer backend 계약을 바꾸므로 Metal 경로 재설계와 함께 합니다.

## Compatibility

- 저장 포맷, C ABI, FFI 헤더는 바뀌지 않습니다.
- 공개 API를 추가합니다.
  - `pass_compiler::{FrameCompiler, CompileReuse}`
  - `graph_trace::compile_frame_traced_with`
  - `offscreen::plan_subtree_boundaries`
  - `stellar_scene::stack::ClipStackRef::{storage_addr, outer}`
- `ClipPlan::Stencil`·`ClipPlan::Mask`의 `clips` field 타입이 `Vec<usize>`에서 `Arc<[usize]>`로 바뀝니다. renderer-core
  밖 사용처는 없습니다.
- `compile_frame`, `compile_frame_traced`, `plan_boundaries`의 결과는 바뀌지 않습니다.

## Performance

`cargo xtask replay`(release)를 2만 가시 node fixture(seed 1)에서 돌렸습니다. scenario는 Pan·Drag·Resize이고 각 62
event입니다. 측정 장비는 개발 장비(Apple M3 Max)입니다. 다른 작업이 동시에 돌아 load average가 23\~48이었습니다.
같은 부하에서 이전 commit(33ba9b2f) build와 이번 build를 번갈아 3회씩 쟀습니다. 기준 장비(M1 16GB) 판정이 아닙니다.

render graph compile·batching CPU 시간(처음 2 frame 제외, frame당):

| scenario | 이전 p50 | 이후 p50 | 이전 최소 | 이후 최소 |
|---|---:|---:|---:|---:|
| Pan | 3.34\~3.76ms | 1.14\~1.33ms | 3.09ms | 1.08ms |
| Drag | 3.43\~4.08ms | 0.41\~0.44ms | 3.12ms | 0.37ms |
| Resize | 3.42\~3.90ms | 0.41\~0.45ms | 3.08ms | 0.36ms |

상호작용 지연(T0→T5, 결과 JSON):

| scenario | 이전 p50 | 이후 p50 | 이전 p99 | 이후 p99 |
|---|---:|---:|---:|---:|
| Pan | 3.48\~3.85ms | 1.18\~1.54ms | 3.81\~19.00ms | 2.01\~4.52ms |
| Drag | 3.98\~4.71ms | 0.66\~0.70ms | 5.43\~10.53ms | 2.27\~3.47ms |
| Resize | 3.96\~4.43ms | 0.67\~0.78ms | 5.83\~7.91ms | 2.31\~18.81ms |

- 마지막 scene semantic hash는 이전·이후 모두 같습니다(Pan `987bdd63…`, Drag `fb619097…`, Resize `5cac6d3c…`).
- Drag·Resize의 render graph 구간은 §6.2 목표(0.75ms) 안입니다.
- Pan은 상한(1.25ms) 근처이고 목표를 넘습니다(Consequences).
- p99는 부하 때문에 회차마다 크게 흔들렸습니다(Resize 18.81ms는 한 회차 값). 기준 장비에서 다시 재야 합니다.

## Security

- 재사용 상태는 프로세스 메모리 안 scene 참조와 숫자뿐입니다. trace·결과에 새 값을 넣지 않습니다.
- cache key의 주소는 프로세스 밖으로 나가지 않습니다.

## Migration

- 없음. 저장 데이터와 ABI에 영향이 없습니다. frame마다 compile하는 호출자는 `compile_frame` 대신 `FrameCompiler`를
  들고 `compile`을 부르면 됩니다.
