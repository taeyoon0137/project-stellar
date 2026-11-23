# ADR-0096: 상호작용 지연 측정 연결 지점과 성능 게이트 시나리오

## Status

Accepted

## Context

ADR-0084 결정 8\~11이 상호작용 trace(T0\~T5), ring buffer store, 입력 재생 파일(`stellar-replay v1`), core replay
runner, 예산 config, analyzer와 release gate를 정했습니다. 다음 일이 남아 있었습니다.

- 앱 입력 경로(AppKit 입력 → DocumentThread → render thread)가 trace를 쓸 API가 없었습니다. 앱 연결은 UI 재설계 때
  하지만, 그때 쓸 연결 지점은 지금 정해야 core replay와 앱이 같은 규칙을 씁니다. T0을 운영체제 event 시각으로 할지
  Stellar 수신 시각으로 할지도 정하지 않았습니다(XVI.1.1 보고서).
- 계획 XVI.2.5는 아키텍처 §38.1 시나리오 13개(Pan, Zoom, Drag, Resize, Rotate, Hover, Select, Text Edit, Token
  Propagation, Component Update, Variant Change, Undo, Redo)를 각각 1,000 sample 이상 재생하라고 합니다. 재생 형식과
  runner는 pan·drag·resize만 다뤘고 key 입력이 없었습니다.
- 계획 XVI.2.6은 warm·cold cache setup을 나누고 결과 metadata에 적으며, setup이 실제 hit·miss와 맞는지 확인하라고
  합니다. 재생 형식·결과 schema에 cache 정보가 없었습니다.
- release gate(XVI.4.4)는 실행에 넣은 scenario만 판정했습니다. 빠진 required scenario, sample이 너무 적은 scenario,
  cache setup이 실제와 다른 결과도 통과할 수 있었습니다.
- trace dimension의 상호작용 종류는 `undo_redo`·`token_edit`처럼 시나리오와 1:1이 아니었습니다.

## Decision

1. **T0은 Stellar 수신 시각입니다(XVI.1.1).** 아키텍처 §6.1의 T0 정의("운영체제 입력 이벤트를 Stellar가 수신")를
   따릅니다. 운영체제 event timestamp(XII.3.1이 monotonic domain으로 바꾼 값)는 T0에 쓰지 않습니다. 운영체제
   전달 지연은 Stellar가 통제할 수 없고, 다른 시계에서 옮긴 값을 CPU stage와 섞으면 clock domain 오차가 구간에
   들어가기 때문입니다.
2. **앱 연결 지점 `stellar_diagnostics::latency_probe::InteractionProbe`(XVI.1.1\~XVI.1.5).**
   - `begin`은 event마다 새 `InteractionTraceId`를 발급하고 probe 시계로 T0을 찍습니다. `begin_at`은 수신 스레드가
     먼저 찍은 시각을 씁니다.
   - stage 시각은 `ProbeStamp`로만 받습니다. `ProbeStamp`는 probe 시계 handle(`Stamper`)만 만들 수 있어 GPU
     timestamp나 운영체제 시각을 stage에 넣을 수 없습니다(clock domain을 타입으로 강제, XVI.1.3). GPU 실행 시간은
     `set_gpu`로 따로 남기고 Logical Interaction Latency에 더하지 않습니다(ADR-0084 결정 8).
   - 이벤트 병합은 `coalesce(merged, into)`입니다. 병합된 event도 자기 trace ID와 T0을 유지하고 T1\~T5는 살아남은
     event의 시각을 받습니다. 그래서 각 event의 latency는 자기 수신부터 그 event를 반영한 frame까지입니다. T1 뒤에는
     병합할 수 없습니다.
   - `finish`는 validator(XVI.1.2)를 통과한 trace와 병합된 trace를 ring buffer store(XVI.1.5)에 넣습니다. 빠진
     stage가 있으면 오류이고 trace는 진행 중으로 남습니다.
   - 진행 중 trace는 256개까지입니다. 넘으면 가장 오래된 진행 중 trace를 버리고 셉니다(memory bound).
   - frame 없이 취소된 event(`cancel`)는 latency가 없으므로 sample이 아니며 따로 셉니다. 측정한 sample을 빼는 일이
     아닙니다(XVI.4.3).
   - runtime은 diagnostics에 의존하지 않습니다(ADR-0084 결정 8). 앱 연결 때 shell 또는 앱 진단 계층이 probe를 갖고
     runtime 경계에서 stage 시각을 받습니다.
3. **시나리오 목록 `stellar_diagnostics::latency_scenario::LatencyScenario`(XVI.2.5).**
   - §38.1 시나리오 13개의 닫힌 목록입니다. 모두 required입니다. 재생 파일의 `scenario` 값과 같은 목록이라 xtask의
     `ReplayScenario`는 이 타입의 별칭입니다(목록을 두 곳에 두지 않음).
   - trace dimension `InteractionKind`를 시나리오와 1:1로 맞춥니다. `token_edit`은 `token_propagation`으로, `undo_redo`는
     `undo`·`redo`로 나누고 `component_update`·`variant_change`를 더합니다. 단일 속성 변경(`property_edit`)은 시나리오가
     아니지만 dimension으로 남깁니다. 결과 JSON의 `kind`만으로 시나리오를 구분합니다.
   - core replay(UI 없이 command → layout → scene → render graph)가 실행하는 시나리오는 10개입니다. Pan(Hand),
     Zoom(Zoom 도구 scrub), Drag, Resize, Rotate(회전 영역), Hover(누르지 않은 이동마다 hit test), Select(card 클릭),
     Variant Change(View Variant 교체), Undo, Redo(history 재생)입니다.
   - 다음 세 시나리오는 엔진 경로가 문서 → scene에 아직 이어지지 않아 `CoreReplay::NotConnected`입니다. 재생 파일을
     만들지 않고, release gate에서는 sample 0개라 실패합니다. 연결 경로를 core replay 전용으로 따로 만들면 이중
     경로가 되므로 만들지 않습니다.
     - Text Edit: Text node의 scene 입력(text layout hook)이 runtime crate 밖에 공개되지 않았습니다.
     - Token Propagation: binding 평가 결과를 문서 node 값과 scene에 쓰는 경로가 없습니다.
     - Component Update: 컴포넌트 정의 편집을 command change set으로 내는 경로가 없습니다.
   - Variant Change는 해석한 `VariantOverrideSet`을 runtime `scene_edits`의 variant option으로 넘기고, 이전·다음 Variant가
     건드리는 node를 Subtree 무효화로 다시 펼칩니다(ADR-0084 결정 5). 표준 재생의 짝수 단계는 card 10개 중 1개를
     숨기는 Variant, 홀수 단계는 Variant 없음입니다.
   - Undo·Redo는 `stellar_history::History`로 기록한 편집을 되돌리거나 다시 적용합니다. 확정 command도 history에
     기록합니다(제품 편집과 같은 경로).
4. **입력 재생 파일 v2(XVI.2.1).** `stellar-replay v2`는 v1에 다음을 더합니다.
   - `cache warm|cold`: 측정 시작 cache 상태(필수).
   - `measure-from <n>`: 앞 `n`개 event는 setup이고 sample이 아닙니다(예: Undo가 되돌릴 편집 만들기). 파일에 실행 전에
     정해지는 값이라 측정값을 보고 sample을 고르는 일이 아닙니다.
   - event `key <key> <modifier>`, `text <scalar hex>`, `commit <n>`. modifier는 `-` 또는 `cmd`·`ctrl`·`alt`·`shift`를
     이 순서로 `+`로 이은 값이고, 표기 하나가 값 하나입니다. `commit <n>`은 패널·View 조작의 확정 단계입니다. UI
     재설계 전이라 패널 입력을 확정 단계로만 적고, 단계의 뜻은 시나리오가 정합니다.
   - v1 파일은 계속 읽습니다(cache warm, measure-from 0). 쓰기는 v2만 합니다.
5. **cache setup(XVI.2.6).**
   - core replay의 cache는 증분 scene build의 node 결과와 그 위의 공간 인덱스·증분 layout입니다. hit은 이전 결과를 그대로
     쓴 node(`shared`), miss는 다시 평가한 node(`rebuilt`)입니다. GPU 쪽 cache(glyph atlas XI.6.5, texture cache XI.7.3)는
     core replay에 없어 macOS full replay(XVI.2.3)가 같은 형식으로 더합니다.
   - `warm`은 문서를 연 뒤 만든 결과와 setup event가 남긴 결과를 그대로 두고 측정합니다. `cold`는 측정 첫 event에서
     그 결과를 버리고 scene·공간 인덱스·layout을 처음부터 만듭니다. 측정 첫 interaction의 cold 비용을 보는
     정의입니다. 단일 interaction 상한(통제 가능 max 10ms)이 이 비용을 판정합니다.
   - `check_cache_setup` 판정: cold는 측정 시작 build가 처음부터 만든 build이고 hit 0, miss 1 이상입니다. warm은 측정 구간에
     처음부터 만든 build가 없고, 측정 시작 build가 있으면 hit 1 이상입니다. 측정 중 편집이 만든 miss는 setup과 무관해
     판정하지 않습니다.
   - 결과 schema를 `stellar.performance-result.v2`로 올립니다. scenario마다 `cache`(상태, 측정 구간 hit·miss·처음부터 만든
     build 수, 첫 build 값, 일치 여부)와 `setup_events`를 남깁니다. 허용 key 목록(privacy schema)에 이 key를 더합니다.
6. **release gate 보강(XVI.4.1, XVI.4.4).**
   - `config/performance-budgets.yaml` `interaction:` 절에 `scenario_min_samples: 1000`을 더합니다(계획 XVI.2.5의 1,000).
     p99를 nearest-rank로 셀 때 상위 1%가 10개 이상이 되는 크기입니다. reader는 0을 거부합니다.
   - required scenario는 판정 sample이 `scenario_min_samples` 이상이어야 합니다(`too_few_samples`).
   - 선언한 cache setup과 관측이 다르면 실패입니다(`cache_setup`).
   - `cargo xtask replay run --gate`는 §38.1 시나리오 13개 각각의 warm·cold 결과가 모두 있어야 통과합니다. 실행에 없는
     조합은 sample 0개 결과로 넣어 `no_samples`로 실패합니다. 판정에서 빼지 않습니다.

## Alternatives

- **T0 = 운영체제 event timestamp**: 운영체제 전달 지연까지 보지만, Stellar가 통제할 수 없는 구간이 들어가 통제 가능
  max 판정이 흔들리고 clock domain 변환 오차가 섞입니다. 탈락.
- **병합된 event의 trace를 버림**: 병합된 event의 실제 지연(먼저 받은 event가 더 오래 기다림)이 sample에서 사라집니다.
  탈락.
- **core replay 전용으로 Text·Token·Component 경로를 만들기**: 제품 경로와 다른 두 번째 변환이 생깁니다(이중 경로).
  탈락. 제품 경로가 연결되면 그 경로로 runner를 넓힙니다.
- **cold = 모든 sample 직전에 cache 비우기**: 매 interaction이 문서 전체 build가 되어 문서를 연 뒤의 실제 상호작용을
  대표하지 않습니다. 탈락.
- **cache로 texture cache를 core replay에서 흉내 내기**: synthetic fixture image에는 asset 내용이 없어 content hash key를
  만들 수 없습니다. 가짜 key는 판정을 흐립니다. 탈락.
- **누락 scenario를 gate에서 무시**: release gate가 부분 실행으로 통과할 수 있습니다. 탈락.

## Consequences

- 앱 연결 때 shell은 `InteractionProbe` 하나로 T0\~T5를 찍고 store에 넣습니다. core replay와 같은 trace 형식입니다.
- release gate는 Text Edit·Token Propagation·Component Update 경로가 이어지고 macOS full replay가 생길 때까지 통과하지
  않습니다. 이는 아직 판정할 수 없는 상태를 그대로 드러내는 결과입니다.
- 결과 JSON v1을 읽는 도구는 없습니다(이 저장소에서 결과는 측정 기록용 파일).
- `InteractionKind`의 이름이 바뀌어 trace 한 줄 형식의 `kind` 값이 달라집니다. 이 값을 저장하거나 읽는 다른 경로는
  없습니다.

## Compatibility

- 재생 파일: v1 읽기 유지, 쓰기는 v2.
- 결과 schema: v1 → v2(새 key 추가, 기존 key 유지).
- FFI·문서 파일 포맷 변경 없음.

## Performance

- probe의 stage 기록은 진행 중 trace 수(상한 256)에 대한 선형 검색 한 번과 시계 읽기 한 번입니다.
- cold 측정의 첫 event는 문서 전체 scene build를 포함합니다. 측정값은 작업 보고서(XVI.2.6)에 남깁니다.
- `scenario_min_samples`는 예산이 아니라 판정 조건입니다. 단계 예산 수치는 바꾸지 않았습니다.

## Security

- trace·결과에는 닫힌 enum 이름과 숫자만 들어갑니다. 새 cache metadata도 숫자와 고정 이름입니다(privacy schema 검사).
- 재생 파일의 `text` event는 Unicode scalar 하나이고 제어 문자를 거부합니다. 재생 파일은 프로젝트 밖에 두며 사용자
  문서를 가리키지 않습니다.

## Migration

- 기존 v1 재생 파일은 그대로 읽힙니다. 새로 기록하려면 `cargo xtask replay record`·`record-suite`를 다시 실행합니다.
