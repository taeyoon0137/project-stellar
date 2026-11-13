# 작업 보고서: XI.1.6 제출 전 오래된 snapshot 감지

- WBS: XI.1.6
- 작업명: renderer가 제출 전 오래된 snapshot을 감지해 폐기하는 revision check
- 상태: 완료 (`[x]`)
- 선행: XI.1.3 (이 lane에서 완료: `a4347ad`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/freshness.rs`(신규) — `LatestSceneRevision`(공유 atomic 최신 세대: `publish`,
  `latest`, `check`, `check_handle`), `PresentCheck::{Present, Discard}`.
- `crates/scene/src/lib.rs` — `freshness` 모듈 등록.
- `docs/task-reports/XI.1.6.md`(신규), 계획서(XI.1.6 `[ ]`→`[x]`).

## 구현 요약

- **최신 세대 공유**: document thread와 render thread가 `Arc<AtomicU64>` 하나를 공유합니다. document
  thread는 snapshot을 넘기기 전에 `publish`(단조 `fetch_max`, 늦게 온 작은 값 무시)하고, render
  thread는 GPU 제출 직전에 `check`합니다. lock이 없고 document thread는 기다리지 않습니다.
- **판정**: 후보 세대 < 알려진 최신 세대면 `Discard { stale, latest }`, 그 외에는 `Present`입니다.
  세대 번호는 XI.1.3의 `SceneRevision`(1부터, 0 = 없음)입니다.
- present 대기열 깊이 제한(≤1)과 frame drop 정책은 frame scheduler(XI.5.5)가 이 판정을 써서 구현합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 44 passed, doctest 2 passed, 0 failed |
| `cargo test -q -p stellar-scene rapid_100` ×5 | 5회 모두 2 passed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 결정론적 interleaving: 100세대(구조 공유)를 frame 하나 encode하는 동안 3세대씩 더 publish → 오래된 frame 25개 모두 폐기, present 후보는 세대 100 하나(`rapid_100_revisions_present_only_latest_scripted`) | 통과 |
| 실제 thread: document thread가 100세대를 쉬지 않고 publish·전달, renderer는 최신만 골라 느리게 encode 후 check → 마지막 present는 세대 100, present 순서 엄격 증가, 오래된 세대는 Discard(`rapid_100_revisions_present_only_latest_threaded`) | 통과 |
| `publish` 단조성(작은 값 무시), 복제본 값 공유, 경계 판정(`publish_is_monotonic_and_check_compares_against_latest`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `check`와 실제 GPU submit 사이에 새 세대가 publish되면 그 frame은 한 세대 늦은 상태로 제출될 수
  있습니다(판정 시점 기준 최신만 보장). 제출 후 present 예약 단계의 추가 drop은 XI.5.5 범위입니다.
- 계약상 document thread는 handoff **전에** publish해야 합니다. 이 순서는 RenderThread 통합(XII.1.3)
  에서 지켜야 합니다.
