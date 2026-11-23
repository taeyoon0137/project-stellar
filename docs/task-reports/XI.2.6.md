# 작업 보고서: XI.2.6 scene build trace

- WBS: XI.2.6
- 작업명: scene build duration과 rebuilt node count를 diagnostics trace에 기록(내용·이름 없음)
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 4
- 선행: II.3.2(`[x]`), XI.2.5(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/incremental.rs` — `SceneBuildTrace`(정적 event, 숫자 field 13개, `MonotonicClock` 주입).
- `crates/diagnostics/src/trace_schema.rs`(신규) — `scene.build`·`render_graph.compile` schema와 `validate`.
- `tools/xtask/src/replay/runner.rs` — 재생의 모든 scene·render graph trace를 schema로 검사.
- `docs/task-reports/XI.2.6.md`(신규).

## 구현 요약

- 갱신마다 다음 field를 정수로 냅니다.
  - revision·시작·끝·duration
  - node 수·rebuilt·copied·renumbered·shared
  - dirty 종류별 수·full
- event 이름은 정적 문자열이라 이름·text·경로를 담을 수 없습니다.
- scene은 diagnostics에 의존할 수 없습니다. 그래서 diagnostics schema 목록과 같은지 xtask 테스트가 확인하고, core replay가 모든 record를 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `build_trace_has_fixed_numeric_fields`(scene): 고정 줄 형식과 field 이름.
- `scene_build_record_passes_and_drift_fails`(diagnostics): 이름 변경·개수·모르는 event 거부.
- `trace_schemas_match_the_emitting_crates`(xtask): scene·render graph field 목록이 schema와 같음.

## 성능

- trace 한 건은 시계 읽기 2회입니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 앱 runtime은 diagnostics에 의존할 수 없어(crate graph) 앱 경로의 trace 저장은 UI 재설계 때 연결합니다.
