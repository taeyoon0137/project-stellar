# 작업 보고서: XI.4.6 RenderGraph compile trace

- WBS: XI.4.6
- 작업명: RenderGraph compile 결과의 CPU 시간, pass count, draw count, batch count trace
- 상태: 완료 (`[x]`)
- 선행: XI.4.5 (이 lane에서 `2b7e065`로 `[x]`), II.3.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/graph_trace.rs`(신규) — `compile_frame_traced`, `RenderGraphCompileTrace`,
  `TraceClock`, `MonotonicClock`, `TraceSink`, `CompiledFrame`.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.6.md`(신규), 계획서(XI.4.6 `[ ]`→`[x]`).

## 구현 요약

- **측정 범위**: `compile_frame_traced`가 frame compile(effect 경계·clip·분류·graph 검증)과 batching을 한
  번에 실행하고, 앞뒤 시각으로 CPU 시간을 잽니다. 끝 시각은 아키텍처 §6.1 T4(Scene Snapshot·Render Graph
  갱신 완료)의 후보입니다. 시각은 `TraceClock`으로 주입하며, 제품은 `Instant` 기반 `MonotonicClock`,
  테스트는 고정 시계를 씁니다. 시계가 뒤로 가도 CPU 시간은 0으로 포화합니다.
- **trace 내용**: event `render_graph.compile`과 scene revision, 시작·끝 시각, CPU ns, pass 수(graph 실행
  순서 길이), draw 수, batch 수, layer 수. field는 모두 정수이고 이름은 정적 문자열이라 node 이름·text·
  경로 같은 프로젝트 내용이 들어갈 수 없습니다(II.3.2 telemetry 규칙). `fields()`는 고정 순서의
  (정적 이름, 값) 목록, `to_line()`은 고정 형식 한 줄입니다.
- **diagnostics 연결**: renderer-core는 crate 의존 정책(II.2.8)상 `stellar-diagnostics`를 쓸 수 없어
  `TraceSink` trait로 내보냅니다. runtime이 `fields()`로 diagnostics event를 만듭니다. compile이 실패하면
  trace를 기록하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 95 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| trace fixture(8 node, scene revision 42, 고정 시계 1000→1750): trace 값(cpu 750, pass 3, draw 8, batch 5, layer 1)과 sink 기록, 고정 형식 줄 golden, 반환 plan·batch와 일치(`trace_fixture_matches_golden`) | 통과 |
| field 이름·순서 고정, 시계 역행 시 CPU 0, 빈 frame trace(pass 1·draw 0·batch 0)(`trace_fields_are_static_names_and_numbers`) | 통과 |
| 단조 시계(`monotonic_clock_never_goes_backwards`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.4.5는 이 lane에서 완료, II.3.2는 `[x]`입니다. II.3.2의 diagnostics 타입은 crate 의존 정책상 직접
  쓰지 않고 `TraceSink`로 분리했습니다. 완화 적용은 없습니다.
- layer 수와 scene revision, 시작·끝 시각을 추가로 기록했습니다(T4 연결과 offscreen 비용 추적용).

## 남은 위험 / 한계

- T0~T5 통합 trace와 누락 stage 검증은 XVI.1.2 범위입니다. 이 작업은 T4 후보 시각까지 제공합니다.
- CPU 시간은 compile을 호출한 thread의 경과 시간(wall clock)입니다. thread 선점은 구분하지 않습니다.
