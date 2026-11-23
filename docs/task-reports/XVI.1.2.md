# 작업 보고서: XVI.1.2 T1\~T5 stage 기록과 validator

- WBS: XVI.1.2
- 작업명: T1 hit test, T2 command commit, T3 binding/layout, T4 scene/render graph, T5 submit/present 예약을 같은 trace에 기록
- 상태: 부분 완료 (`[~]`)
- 결정: ADR-0084 결정 8·9
- 선행: VII.1.4(`[x]`), X.7.3(`[x]`), XI.2.6(이 lane), XI.8.5, XI.9.5(D3D12, ADR-0006으로 제외)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/interaction.rs`(신규).
- `crates/diagnostics/src/lib.rs` — 모듈 등록.
- `tools/xtask/src/replay/runner.rs` — core replay가 event마다 trace를 열고 stage를 기록.
- `docs/task-reports/XVI.1.2.md`(신규).

## 구현 요약

- `mark`는 앞 stage가 있어야 하고 한 번만, 시각은 감소하지 않아야 합니다. `validate`는 빠진 stage를 `MissingStage`로 표시하고 완료 trace를 만듭니다.
- core replay는 각 stage를 표의 단계 뒤에 기록합니다. 단계는 hit test·상태 기계, command, 증분 layout, scene·render graph입니다. GPU 제출이 없어 T5 = T4입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `validator_reports_missing_stages`: 누락·건너뜀·역순·중복 거부, 전부 있으면 통과.
- 재생 trace 186개(3 scenario × 62)가 모두 validator를 통과.

## 성능

- stage 기록은 시계 읽기 1회입니다.

## 차이(작업 지시와 다른 점)

- T5(GPU 제출·Present 예약)는 core에 GPU 제출이 없어 T4와 같은 시각입니다. Metal 제출 시각 연결(XI.8.5)은 앱 경로 몫입니다.

## 남은 위험 / 한계

- 앱·GPU 경로가 없어 `[~]`입니다.

## 2026-11-23 interaction-trace 후속: 연결 지점의 stage 기록 (ADR-0096 결정 2)

- 상태는 그대로 부분 완료(`[~]`)입니다. 앱·Metal 경로에서 T1\~T5를 찍는 연결은 UI 재설계 때 합니다.
- `InteractionProbe::mark`(probe 시계)와 `mark_at`(다른 스레드가 찍은 `ProbeStamp`)으로 T1\~T5를 같은 trace에 기록합니다.
  `finish`가 validator를 부르고, 빠진 stage가 있으면 오류를 돌려주며 trace는 진행 중으로 남습니다. 통과한 trace만
  store에 들어갑니다. frame 없이 취소된 event(`cancel`)는 sample이 아닙니다.

| 테스트 | 확인 |
|---|---|
| `stages_are_recorded_once_in_order_and_validated` | 누락(T3)·건너뜀·중복·역순 거부, 실패한 finish 뒤 진행 중 유지, 끝낸 trace 기록 거부, 취소는 store 밖 |

### 근거 커밋

- `d25a69c9` Feat: 상호작용 trace T0\~T5 ID·stage·구간·dimension 추가 (XVI.1.1\~XVI.1.4)
- `2e574ac5` Feat: UI 없이 command·layout·scene·render graph를 재생하는 core replay runner 추가 (XVI.2.2)
- 이 커밋(`Task(XVI.1.2)`): 연결 지점 probe의 stage 기록 검사.
