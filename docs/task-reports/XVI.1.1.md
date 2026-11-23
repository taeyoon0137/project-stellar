# 작업 보고서: XVI.1.1 InteractionTraceId와 T0

- WBS: XVI.1.1
- 작업명: `InteractionTraceId`를 pointer/key event마다 생성하고 T0 입력 수신 시각 기록
- 상태: 부분 완료 (`[~]`)
- 결정: ADR-0084 결정 8
- 선행: XII.3.5(`[~]`, 앱 경로 미연결)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/interaction.rs`(신규).
- `crates/diagnostics/src/lib.rs` — 모듈 등록.
- `tools/xtask/src/replay/runner.rs` — core replay가 event마다 trace를 열고 stage를 기록.
- `docs/task-reports/XVI.1.1.md`(신규).

## 구현 요약

- `InteractionTraceIds::issue`가 1부터 단조 증가 ID를 발급합니다. `InteractionTrace::begin`이 T0을 기록한 trace를 엽니다.
- core replay는 재생 event마다 trace 하나를 열고, T0을 ManualClock timeline 시각으로 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `every_event_gets_its_own_trace_id_with_t0`: event 5개에 고유 ID 5개와 T0.
- `replaying_twice_gives_the_same_semantics_and_trace_count`: 재생 event 수 = trace 수, T0 = event 시각.

## 성능

- ID 발급은 정수 증가 1회입니다.

## 차이(작업 지시와 다른 점)

- 앱 입력 경로(FFI receipt, ADR-0029)에서 ID를 발급하지 않습니다. runtime은 diagnostics에 의존할 수 없고(crate graph 정책), UI는 재설계 전입니다.

## 남은 위험 / 한계

- 앱 경로가 없어 `[~]`입니다. 앱 연결 때 receipt의 T0 후보(event_time·received) 중 어느 값을 T0로 쓸지 정해야 합니다.

## 2026-11-23 interaction-trace 후속: 앱 연결 지점 (ADR-0096 결정 1·2)

- 상태는 그대로 부분 완료(`[~]`)입니다. 앱 입력 경로 연결은 UI 재설계 때 하고, 이번에는 그때 쓸 연결 지점 API를
  `crates/diagnostics/src/latency_probe.rs`에 두었습니다.
- T0은 Stellar 수신 시각입니다. 운영체제 event timestamp는 T0에 쓰지 않습니다(ADR-0096 결정 1). 위 "남은 위험"의 T0
  후보 결정이 이것으로 닫힙니다.
- `InteractionProbe::begin`은 event마다 새 `InteractionTraceId`를 발급하고 probe 시계로 T0을 찍습니다. `begin_at`은 수신
  스레드가 `Stamper`로 먼저 찍은 시각을 씁니다.
- 이벤트 병합(`coalesce`)된 event도 자기 trace ID·T0을 유지하고, 살아남은 event의 T1\~T5를 받아 store에 들어갑니다.

| 테스트 | 확인 |
|---|---|
| `begin_issues_one_id_per_event_with_receipt_t0` | event 4개에 고유 ID 4개, `begin_at`의 T0 = 수신 시각 |
| `coalesced_events_keep_their_own_id_and_t0` | 병합 event의 latency가 자기 수신부터(5µs·7µs), T1 뒤 병합 거부 |

### 근거 커밋

- `d25a69c9` Feat: 상호작용 trace T0\~T5 ID·stage·구간·dimension 추가 (XVI.1.1\~XVI.1.4)
- 이 커밋(`Task(XVI.1.1)`): 앱 연결 지점 probe.

### 남은 범위

- 앱 입력 경로(AppKit 입력 → DocumentThread)에서 `InteractionProbe::begin`을 부르는 일(UI 재설계 때). runtime은
  diagnostics에 의존하지 않으므로 shell 또는 앱 진단 계층이 probe를 갖습니다.
