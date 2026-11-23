# 작업 보고서: XVI.2.2 core-only replay runner

- WBS: XVI.2.2
- 작업명: core-only replay runner로 UI 없이 command/layout/scene 단계까지 재생
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 선행 미충족(계획 1.2 규칙 4)과 남은 범위: 선행 XVI.2.1(`[~]`)이 `[x]`가 아니고, 재생 layout 변환이 Free node만 따로 다루는 경로라 문서 → layout 입력 연결로 바꿔야 합니다(아래 "남은 위험"). 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 6·9
- 선행: XVI.2.1(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `tools/xtask/src/replay/runner.rs`(신규), `replay/layout.rs`(신규), `replay/tests.rs`(신규).
- `tools/xtask/src/replay.rs` — 결과 요약(`summarize`·`build_result`), 환경 값.
- `docs/task-reports/XVI.2.2.md`(신규).

## 구현 요약

- fixture를 다시 만들어 content hash를 확인합니다. 그 뒤 event마다 다음을 실행합니다.
  - canvas hit test(transient handle 먼저)와 상호작용 상태 기계
  - 이동 preview 주입
  - 뗌 `CommandTransaction` 하나
  - 증분 layout·layout rect 무효화
  - scene commit·공간 인덱스 갱신
  - render graph compile·batching
- T1\~T4는 T0에 실제 경과를 더한 값입니다.
- layout 단계는 Free node를 absolute box로 옮긴 입력(ADR-0076 Free 대응)으로 `IncrementalLayout`을 돌립니다.
- 측정 중 찾은 병목 두 곳을 고쳤습니다.
  - absolute node 변경의 부모 relayout(레이아웃 commit `Fix: inset이 정해진 absolute node 변경은 부모를 다시 배치하지 않음`)
  - 편집마다 전체 tree를 걷던 scene source 색인(runtime commit `Fix: 구조가 그대로인 change set은 scene source 색인을 다시 만들지 않음`)

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| `cargo run --release -p xtask -- replay record drag --out <scratch>`(pan·resize도 같음) | 62 event 파일 3개 |
| `cargo run --release -p xtask -- replay run pan.replay drag.replay resize.replay --gate` | 결과 JSON 기록. 부하에 따라 gate 통과(종료 0) 또는 Drag p99 초과(종료 65) |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `replaying_twice_gives_the_same_semantics_and_trace_count`: 3 scenario 모두 두 번 실행해 trace 수·scene semantic hash·document revision·scene build 수가 같음, T0 = event 시각.
- `drag_commits_once_and_previews_keep_the_document_revision`.

## 성능

- 2만 가시 node fixture(release, 개발 장비, 다른 lane 동시 실행)를 3회 잰 범위입니다.
  - Pan: p50 3.1ms, p99 3.5\~8.6ms
  - Drag: p50 3.6ms, p99 5.4\~11.7ms
  - Resize: p50 3.4ms, p99 5.3\~6.8ms
- scene build p99는 0.6\~1.2ms입니다. 확정 layout 구간은 약 1.6ms(수정 전 약 6.0ms)입니다.
- render graph compile·batching이 frame당 약 3ms(약 2만 2천 node 전체 compile)로 지배 비용이며 §6.2 예산(목표 0.75ms)을 넘습니다. 세부는 ADR-0084 Performance입니다.

## 차이(작업 지시와 다른 점)

- 계획의 'command/layout/scene 단계까지'보다 넓게 render graph compile까지 실행합니다. lane 결정 2가 그렇게 정했습니다.

## 남은 위험 / 한계

- 재생 layout 변환은 Free node만 다룹니다. 문서 → layout 입력 연결이 생기면 그 경로로 바꿔야 합니다(이중 경로 정리).
- render graph compile이 증분이 아니라 예산을 넘습니다(renderer-core 후속).
- 부하가 큰 공유 장비에서 잰 값이라 p99 변동이 큽니다.

## 후속 보완 (2026-11-23, render graph 증분 compile)

- 변경 파일: `tools/xtask/src/replay/runner.rs` — 재생 내내 `FrameCompiler` 하나로 render graph를 compile합니다(ADR-0090 결정 7). 이전에는 frame마다 snapshot 전체를 compile했습니다.
- 측정(release, 2만 가시 node, 개발 장비 Apple M3 Max, load average 23\~48, 이전 commit build와 번갈아 3회):

  | scenario | render graph p50 이전 → 이후 | T0→T5 p50 이전 → 이후 |
  |---|---|---|
  | Pan | 3.34\~3.76ms → 1.14\~1.33ms | 3.48\~3.85ms → 1.18\~1.54ms |
  | Drag | 3.43\~4.08ms → 0.41\~0.44ms | 3.98\~4.71ms → 0.66\~0.70ms |
  | Resize | 3.42\~3.90ms → 0.41\~0.45ms | 3.96\~4.43ms → 0.67\~0.78ms |

- 마지막 scene semantic hash는 이전과 같습니다. 세부와 p99는 ADR-0090 Performance입니다.
- 검증: `cargo test -p xtask replay` 7 passed. `cargo xtask replay run`(release) 3 scenario 결과 JSON 생성, gate 통과.
- 남은 한계: Pan은 frame마다 viewport가 바뀌어 전체 compile이며 §6.2 Render Graph 목표(0.75ms)를 넘습니다(ADR-0090 Consequences). 기준 장비(M1 16GB) 측정이 남았습니다.
