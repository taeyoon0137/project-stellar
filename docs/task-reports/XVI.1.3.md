# 작업 보고서: XVI.1.3 stage duration과 Logical Interaction Latency

- WBS: XVI.1.3
- 작업명: stage별 duration과 총 Logical Interaction Latency 계산(GPU timestamp 분리)
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.1.2(`[~]`, 앱·GPU 경로 없음)가 `[x]`가 아닙니다. 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 8·12
- 선행: XVI.1.2(이 lane), II.3.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/interaction.rs` — `StageSpan`, `CompletedTrace::span_nanos`·`logical_latency_nanos`, `GpuTiming`.
- `docs/task-reports/XVI.1.3.md`(신규).

## 구현 요약

- 구간은 5개입니다(Input·Command·Layout·Scene·Present). Logical Interaction Latency는 T5 − T0입니다.
- GPU timing은 Metal timestamp용 별도 domain입니다. `gpu_nanos`로만 내고 CPU 구간에 더하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `manual_clock_reference_durations`: ManualClock 고정 timeline에서 구간 5개, latency 4.45ms, 구간 합 = latency, GPU 3.5ms가 따로 나오고 역순 GPU 거부.

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- D3D12 timestamp는 ADR-0006(Windows 보류)으로 정의하지 않았습니다.

## 2026-11-23 interaction-trace 후속: clock domain을 타입으로 강제 (ADR-0096 결정 2)

- 연결 지점 probe는 CPU stage 시각을 `ProbeStamp`로만 받습니다. `ProbeStamp`는 probe 시계 handle(`Stamper`)만 만들 수
  있어 GPU timestamp나 운영체제 시각을 stage에 넣을 수 없습니다(compile_fail doctest).
- GPU 실행 시간은 `set_gpu`로 따로 남기고 Logical Interaction Latency에 더하지 않습니다. 병합된 event도 같은 GPU 시간을
  따로 받습니다.

| 테스트 | 확인 |
|---|---|
| `gpu_time_stays_in_its_own_clock_domain` | latency 5ms는 CPU 구간 합과 같고 GPU 3ms는 따로, 역순 GPU 거부, 병합 event도 같음 |
| `ProbeStamp` compile_fail doctest | 숫자로 `ProbeStamp`를 만들 수 없음 |

### 근거 커밋

- `d25a69c9` Feat: 상호작용 trace T0\~T5 ID·stage·구간·dimension 추가 (XVI.1.1\~XVI.1.4)
- 이 커밋(`Task(XVI.1.3)`): 연결 지점의 clock domain 검사.
