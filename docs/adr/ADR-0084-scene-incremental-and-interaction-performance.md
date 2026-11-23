# ADR-0084: 증분 scene build와 상호작용 성능 계측 경로

## Status

Accepted (2026-11-23, lane scene-incremental-perf 구현 기록)

## Context

입력 성능 계약(아키텍처 §6)은 T0\~T5 단계마다 예산이 있고, 판정 기준은 p50≤5ms, p95≤7ms, p99≤9ms, 통제 가능 max≤10ms입니다.
이 lane 전에는 다음이 비어 있었습니다.

- scene은 전체 build만 있었습니다(XI.2.2). 무효화를 scene dirty로 바꾸는 규칙(XI.2.1)이 없었고, dirty subtree만 다시 만드는
  경로(XI.2.5)도 없었습니다.
- 상호작용 preview를 scene에 주입하는 경로가 없어 XII.3.7이 차단 상태였습니다.
- Boolean modifier는 저장만 되고 geometry 평가가 없었습니다(XII.10.30).
- T0\~T5 trace는 runtime에 T0·T1 후보만 있었고(ADR-0029), 예산 config, analyzer, release gate, 재생 runner가 없었습니다.
- CSS 레이아웃 엔진(ADR-0076)을 통합한 뒤, 200,201 node 합성 문서의 전체 배치가 이전 엔진보다 약 2.3배 느렸습니다(0.75s 대 0.33s).

제약도 있습니다.

- scene은 core-types에만 의존합니다(II.2.8). runtime은 diagnostics와 layout에 직접 의존할 수 없습니다(crate graph 정책).
- 사용자 방향(2026-11-23)에 따라 셸 FFI와 Swift UI는 바꾸지 않습니다. 기능은 Rust 테스트·CLI로 검증합니다.

## Decision

1. **레이아웃 전체 배치 회귀.** 원인은 node당 배치 계산이 약 7.4회(1,480,201회)였던 데 있습니다. 결과가 같은 요청이
   따로 계산되었습니다. 세로 가용 공간의 min-content와 max-content가 구분되었고, 자식 없는 node의 세로 가용 공간과
   px 크기 축의 가용 공간도 따로 계산되었습니다. 여기에 요청 전체를 hash하는 memo map과 `Prepared` 복제 비용이
   더해졌습니다. 고친 내용은 다음과 같습니다. 입력 타입과 생략값 해석은 바꾸지 않았습니다.
   - `Engine::normalize`가 결과에 쓰이지 않는 요청 값을 정규형으로 바꿉니다.
     - 세로 `MinContent`는 `MaxContent`로 바꿉니다. 모든 배치 함수가 세로 가용 공간을 `.definite()`로만 읽기 때문입니다.
     - 자식 없는 node의 세로 가용 공간을 지웁니다.
     - px 크기이고 keyword min·max가 없는 축의 가용 공간을 지웁니다. 가로 축은 너비 질의 여부만 남깁니다.
   - in-run memo를 node별 목록으로 바꿉니다. 전체 계산은 node 수만큼 미리 두고, 증분 계산은 방문한 node만 담습니다.
   - 배치 함수가 `Prepared`를 빌려 씁니다.
   - 결과: 같은 부하에서 번갈아 3회 잰 값입니다.

     | 엔진 | 전체 배치 | 증분 세션 생성 | 단일 leaf edit p50 |
     |---|---:|---:|---:|
     | 이전 엔진 | 0.33s | 0.95s | 202µs |
     | 통합 직후 | 0.70s | 1.30s | 260µs |
     | 수정 후 | 0.26s | 0.91s | 192µs |

     계산 수는 700,201회입니다. CSS 적합성 fixture 257개가 그대로 통과합니다.
   - 예산: 전체 배치는 상호작용 동기 경로가 아니므로(문서 열기·구조 교체) 따로 상한을 두지 않습니다. 회귀 감시로
     `bench` 테스트가 node당 계산 4회 이하를 검사합니다(결정적 집계). 증분 edit는 §6.2 증분 레이아웃 예산(1.00ms,
     상한 1.75ms) 안입니다.
2. **scene dirty set(XI.2.1).** scene 쪽 무효화 값 `SceneInvalidation`을 node별 `Paint < Geometry < Subtree`로 바꿉니다.
   - 문서 무효화 class가 paint만이면 Paint입니다.
   - geometry·layout class, layout rect 변경, 잠금 전환(문서 class는 비어 있지만 hit metadata가 바뀜), 구조 변경은
     Geometry입니다.
   - View Variant·컴포넌트 정의 교체는 Subtree입니다.
   - 다른 node geometry를 읽는 node(Boolean 결과)는 `SceneDependents`로 전파합니다.
   - Paint와 Geometry의 구분은 집계와 Subtree 강제에만 씁니다. builder는 dirty node의 spec을 항상 다시 평가하고,
     자손에게 가는 영향은 scope 비교로 판단합니다. 그래서 Frame transform처럼 문서 class가 paint인 변경도 자손 world
     값이 맞습니다.
3. **증분 scene build(XI.2.5).**
   - 입력은 key별 builder 입력 tree `SceneSource`입니다.
   - `IncrementalScene`은 node마다 이전 결과·받은 부모 scope(world·clip·숨김·잠금)·시작 z-order를 기억합니다.
     - 자신·자손이 깨끗하고 scope와 z가 같으면 `Arc`를 그대로 씁니다.
     - z만 다르면 content·paint·clip을 공유한 채 z만 다시 매깁니다.
   - z-order는 기존 builder와 같은 pre-order 연속 번호입니다. 그래서 증분 결과와 전체 build 결과의 semantic hash가
     같습니다. 간격을 둔 z 번호는 canonical bytes가 build 순서에 따라 달라지므로 채택하지 않았습니다.
   - source 편집은 바뀐 node·자식·조상 chain만 검사하고, 실패하면 편집 전으로 되돌립니다.
   - runtime `canvas::scene_source`가 문서를 source로 바꿉니다. node 종류별 입력은 캔버스 행과 같은 함수로 만듭니다.
     command change set은 subtree 단위로 다시 변환한 뒤 달라진 node만 편집으로 냅니다.
4. **scene build trace(XI.2.6).** `SceneBuildTrace`는 정적 event `scene.build`와 숫자 field 13개만 냅니다. field는
   revision·시각·node 수·rebuilt·copied·renumbered·shared·dirty 종류별 수·full입니다.
   `stellar_diagnostics::trace_schema`가 같은 이름 목록으로 검사합니다. 두 목록이 같은지는 xtask replay 테스트가
   확인합니다. scene은 diagnostics에 의존할 수 없기 때문입니다.
5. **ComponentInstance와 View Variant(XI.2.4).**
   - 인스턴스는 문서에 복제하지 않고 expansion view(IX.2.2)의 가상 subtree를 펼칩니다.
     - 인스턴스 key는 scene node가 없는 Fragment입니다.
     - 가상 node key는 FNV-1a 128로 만든 version 8 id입니다. 입력은 (인스턴스 chain, 정의 node)입니다.
     - scene `source`는 바깥 인스턴스입니다(hit test가 인스턴스를 고름). 캔버스 행(component.rs)과 같은 규칙입니다.
   - View Variant는 해석한 `VariantOverrideSet`을 받습니다.
     - scene 입력에 바로 닿는 표시 여부 patch만 적용합니다.
     - 다른 property·구조·slot patch는 셉니다. 문서 계층에 `TypedValue → payload` 쓰기 함수가 없기 때문입니다. 따로
       해석하면 이중 경로가 됩니다.
     - Variant 교체는 Subtree 무효화입니다.
6. **transient preview(XII.3.7).**
   - `IncrementalScene::set_preview`는 key별 대체 입력으로 snapshot을 만듭니다. document revision은 그대로 두고 scene
     revision만 올립니다.
   - preview는 source·history에 들어가지 않습니다.
   - pointer up에서 호출자가 `CommandTransaction` 하나를 확정하고 source 편집을 반영한 뒤 preview를 지웁니다. 취소는
     preview만 지웁니다.
   - 앱(AppKit) 입력 연결은 UI 재설계 때 합니다. 지금은 core replay가 같은 순서를 구동합니다.
7. **Boolean geometry(XII.10.30).**
   - operand node는 결과에 흡수되어 단독 표시하지 않습니다. 결과는 host의 paint로 칠합니다.
   - operand 순서는 host geometry 다음에 참조 operand를 문서 pre-order(OrderKey 순)로 둡니다. 참조 목록의 저장 순서는
     쓰지 않습니다. Subtract는 첫 operand(host)에서 나머지의 합집합을 뺍니다.
   - 평가 순서는 다음과 같습니다.
     1. 1/64 px 정수 격자로 선분화합니다.
     2. `i128` orientation으로 교차 분할합니다.
     3. 선분 양쪽의 operand별 winding으로 경계 선분만 남깁니다.
     4. 남은 선분을 NonZero loop로 잇습니다.
   - 같은 입력은 bit 단위로 같은 결과입니다.
   - 좌표 ±32,768 px, 선분 20,000개를 넘으면 거부합니다. scene은 그때 host geometry를 그리고 실패 수를 셉니다.
   - host의 다른 modifier는 Boolean 항목을 뺀 stack으로 평가합니다.
8. **상호작용 trace(XVI.1.1\~XVI.1.5).** `stellar_diagnostics::interaction`이 담당합니다.
   - event마다 `InteractionTraceId`를 발급하고 T0을 기록합니다.
   - T1\~T5는 순서대로 한 번씩만 기록합니다. validator는 빠진 stage를 오류로 표시합니다.
   - 구간은 5개입니다. Logical Interaction Latency는 T5 − T0입니다.
   - GPU timing은 별도 clock domain이라 더하지 않습니다.
   - dimension은 kind·document bucket·visible bucket·cache 상태의 닫힌 enum만 허용합니다(문자열 field 없음).
   - 로컬 store는 고정 용량 ring buffer입니다. 기본 4,096개이고 오래된 것부터 지웁니다.
   - runtime은 diagnostics에 의존할 수 없습니다(crate graph). 그래서 앱 경로(ADR-0029의 receipt)를 이 trace로 잇는
     일은 crate graph 결정과 UI 재설계 때 합니다.
9. **입력 재생(XVI.2.1, XVI.2.2, XXIV.3.3).**
   - 재생 파일은 프로젝트 밖에 두는 줄 단위 텍스트(`stellar-replay v1`)입니다. 담는 값은 다음과 같습니다.
     - synthetic fixture 명세와 content hash
     - viewport
     - ManualClock 시각이 붙은 pointer event 목록
     - scenario
   - core replay runner(`cargo xtask replay`)는 UI 없이 문서 command → layout → scene → render graph encode를 실행합니다.
     - T0은 재생 시각입니다.
     - T1\~T4는 T0에 각 단계의 실제 경과를 더한 값입니다.
     - core에는 GPU 제출이 없어 T5 = T4입니다.
   - layout 단계는 Free Frame을 `position: absolute` box로 옮긴 입력(ADR-0076의 Free 대응)으로 `IncrementalLayout`을
     돌립니다. 문서 → layout 입력 연결은 아직 없어 재생 전용 변환입니다.
   - 같은 재생은 trace 수와 scene semantic hash가 같아야 합니다.
10. **예산 config(XVI.4.1).**
    - `config/performance-budgets.yaml`의 `interaction:` 절에 아키텍처 §6.2\~§6.4 값을 정수 µs로 옮깁니다. 단계 목표·상한
      9행, 합계, p50·p95·p99, 통제 가능 max, GPU p99, present 대기열 깊이, 스레드 점유, hover 목표·상한입니다.
    - 테스트가 아키텍처 문서 표와 값을 비교합니다.
    - T 구간 대응은 다음과 같습니다. 스케줄링 여유는 합계에만 들어갑니다.

      | T 구간 | §6.2 행 |
      |---|---|
      | T0→T1 | 입력 + Hit Test |
      | T1→T2 | Command |
      | T2→T3 | Binding + 레이아웃 |
      | T3→T4 | Scene + Render Graph |
      | T4→T5 | GPU 제출·Present |
11. **analyzer·gate·결과(XVI.4.2\~XVI.4.5).**
    - 통계는 통제 가능 sample의 nearest-rank p50·p95·p99·max와 10ms 초과 수입니다.
    - 제외는 사전 정의 코드 4개(OS 선점, 장치 상실, 절전 전환, 드라이버 정지)가 있을 때만 합니다.
    - release gate는 required scenario마다 기준 4개를 검사합니다. 경계값은 통과이고 sample이 없으면 실패입니다.
    - 결과 JSON의 key는 고정입니다. 담는 값은 다음과 같습니다.
      - commit(hex)
      - hardware·OS·app build(경로·URL 문자를 막는 형식 검사)
      - synthetic fixture 식별
      - scenario 통계·gate
12. **GPU 구간.** macOS는 Metal command buffer timestamp만 씁니다. D3D12 timestamp(XI.9.5 선행)는 Windows 보류
    (ADR-0006)로 정의하지 않습니다.

## Alternatives

- **scene node에 간격 z-order:** 삽입 뒤에도 뒤쪽 node를 공유할 수 있습니다. 하지만 같은 내용의 canonical bytes가 build
  이력에 따라 달라져 semantic hash 비교가 깨지므로 탈락했습니다.
- **dirty set에 자손 전파까지 담기:** layout·binding 쪽이 scope 전파 규칙(clip·숨김·잠금)을 알아야 합니다. 이 규칙을
  builder의 scope 비교 한 곳에 두려고 탈락했습니다.
- **Boolean을 f64 다각형 clipping(Greiner–Hormann 등)으로:** 겹친 변·접점에서 결과가 입력 순서에 민감해 bit-stable
  보장이 어렵습니다. 정수 격자와 winding 분류를 택했습니다.
- **View Variant property patch를 adapter에서 해석:** 문서 속성 값 쓰기 규칙이 두 곳에 생기므로 탈락했습니다.
- **replay runner를 runtime crate에:** lane 소유 범위와 crate graph를 바꿔야 합니다. xtask는 crate graph 검사 대상이
  아니라 모든 crate를 쓸 수 있습니다.

## Consequences

- 단일 property 편집은 scene node 1개 평가와 조상 path copying만 합니다. 바뀌지 않은 resource 참조·paint stack은 공유합니다.
- 구조 변경은 뒤쪽 node의 z만 다시 매기므로 O(뒤쪽 node 수)의 node 할당이 남습니다.
- 앱 경로는 아직 이 scene·trace를 쓰지 않습니다. 대상은 셸 입력 → T0\~T5와 캔버스 행 대신 snapshot 렌더입니다.
  UI 재설계 때 연결합니다.
- 재생 layout 변환은 Free Frame만 다룹니다. layout Frame은 문서 → layout 입력 연결 뒤 같은 runner로 측정합니다.

## Compatibility

- 저장 포맷, C ABI, FFI 헤더는 바뀌지 않습니다.
- 공개 API를 추가합니다.
  - `stellar_scene::{dirty, source, incremental, boolean}`
  - `stellar_diagnostics::{interaction, trace_store, trace_schema, budget, analyzer, result}`
  - `stellar_runtime::canvas::scene_source`
  - `cargo xtask replay`
- `config/performance-budgets.yaml`에 `interaction:` 절을 더합니다. 기존 reader는 다른 절을 건너뜁니다.

## Performance

- 결정 1의 수치를 참조합니다.
- 증분 scene의 수치는 `cargo xtask replay` 결과 JSON과 작업 보고서(XVI.2.2)에 기록합니다. 대상은 2만 가시 node
  fixture의 Pan·Drag·Resize입니다.
- 이 결과는 개발 장비에서 잰 값입니다. 기준 장비(M1 16GB) 판정이 아닙니다.

## Security

- trace·결과에는 정적 이름과 숫자, 형식을 검사한 환경 문자열만 들어갑니다. 프로젝트 내용·이름·경로는 들어가지 않습니다.
- 재생 파일은 synthetic fixture만 가리킵니다.

## Migration

- 없음. 저장 데이터와 ABI에 영향이 없습니다.
