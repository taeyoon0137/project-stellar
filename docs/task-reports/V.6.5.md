# 작업 보고서: V.6.5 작은 인접 chunk idle 병합 planner

- WBS: V.6.5
- 작업명: 작은 인접 chunk를 idle 작업에서만 병합하는 planner(저장과 동시 실행 금지, 결정론, 의미 보존)
- 상태: 완료 (`[x]`)
- 선행: V.6.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/merge.rs`(신규) — `plan_chunk_merges`, `MergePlan`, `ChunkAbsorption`,
  `StaleMergePlan`, `IdleMergeWindow`, `begin_idle_merge`, `run_idle_merge`, `IdleMergeOutcome`.
- `crates/storage/src/gc/scheduler.rs` — 저장 시작 번호(`save_epoch`)를 추적하고
  `GcGuard::save_started_since()`를 추가했습니다. 기존 GC 동작은 그대로입니다.
- `crates/storage/src/chunk/mod.rs` — `merge` 모듈 등록.
- `docs/adr/ADR-0012-chunk-split-threshold.md` — "V.6.5 구현 기록" 추가(병합 기준, 순서, 저장 배타).
  보호 경로 변경과 같은 커밋입니다.
- `config/performance-budgets.yaml` — `storage:` 절 주석에 병합 상한 설명 한 줄 추가(값 변경 없음).
- `docs/task-reports/V.6.5.md`(신규), 계획서(V.6.5 `[ ]`→`[x]`).

## 구현 요약

- **병합 기준**: 새 수치를 두지 않고 승인 임계치(`storage.chunk_split_threshold_bytes`)를 씁니다.
  자식 chunk를 부모 chunk에 흡수한 결과 payload가 `≤ threshold`일 때만 병합합니다. 병합 결과가
  분할 기준을 넘지 않으므로 다음 저장의 split이 되돌리거나 다시 나누지 않습니다. 임계치를 넘는
  chunk는 병합으로 커지지 않습니다.
- **인접과 순서(결정론)**: 인접은 chunk tree의 부모–자식 관계입니다(chunk는 root 하나의 연결
  subtree이므로 형제 chunk는 부모로 흡수되는 방식으로 합쳐집니다). chunk root의 입력 역순으로
  순회하며 흡수하고, 흡수가 있었으면 바뀐 layout으로 다시 순회해 더 흡수할 것이 없을 때 멈춥니다.
  흡수 chain(c → p → g)은 최종 대상으로 펼쳐 `remap_manifest`(V.3.4)로 내보냅니다.
- **의미 보존**: chunk root 집합만 바꾸고 노드·부모·순서·노드 payload는 입력 그대로입니다.
- **저장과의 배타**: GC scheduler(V.7.4)의 idle 유지보수 guard(`GcGuard`)를 GC와 공유합니다.
  활성 저장이 있거나 idle threshold 전이거나 GC·다른 병합이 실행 중이면 시작하지 않습니다
  (`NotStarted`). 창이 열린 뒤 저장이 하나라도 시작되면 끝났더라도 결과를 폐기합니다
  (`Superseded`). 저장을 막거나 기다리게 하지 않습니다. 받아들인 계획은 적용 전에
  `check_base`로 기준 layout이 현재 layout과 같은지 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-storage` | 적용 |
| `cargo test -p stellar-storage` | 226 passed(신규 14: merge 13 + scheduler 1) + 통합 1 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo test -q -p stellar-storage --lib -- concurrent_saves` 30회 반복 | 30/30 통과 |
| `cargo test -q -p stellar-storage --lib` 8회 반복 | 8/8 통과 |
| `cargo check -p stellar-project -p stellar-assets -p xtask --all-targets` | 통과(의존 crate 영향 없음) |
| `cargo test -p stellar-core-types color` | 17 passed |
| `cargo xtask validate-adr <이 커밋 경로>` | exit 0 |
| `cargo xtask sync-plan --check` | exit 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 무작위 문서 40개 × 임계치 3종: 모든 의미 경계를 root로 둔 layout과 병합 후 layout을 실제 payload로 인코딩한 뒤 payload만으로 복원한 의미 snapshot이 원본과 같음, payload 길이 = 계획 크기, 하위 chunk 참조가 정확히 한 번씩(`merge_keeps_semantic_snapshot_identical`) | 통과 |
| 부모 흡수 상한, 승인 임계치에서 결과 == T는 병합·T+1은 미병합, chain 최종 대상과 remap 검증, oversized chunk 불변 | 통과 |
| 무작위 tree 60개 × 2 정책 × 2 시작 layout: 불변식, 결정론, 최대성(더 흡수 불가), split 고정점(진동 없음), 재병합 noop | 통과 |
| 저장 중 미시작, idle threshold 전 미시작, GC와 상호 배타, 계획 중 저장 시작 시 폐기, stale layout 감지 | 통과 |
| 저장 스레드 4개 × 200회와 병합 반복: 창이 열린 뒤 저장이 시작됐으면 항상 `Superseded`, 아니면 항상 `Ready`, 겹침 발생 확인(`concurrent_saves_never_overlap_an_accepted_merge`) | 통과 |
| `GcGuard::save_started_since`(`guard_reports_saves_started_after_it`), 기존 GC scheduler 테스트 | 통과 |

## 차이(작업 지시와 다른 점)

- 병합에 쓸 별도 scheduler를 만들지 않고 `GcScheduler`를 재사용했습니다. 저장 파이프라인이
  `begin_save` 하나만 호출하면 GC와 병합이 함께 보호되게 하기 위해서이며, 이를 위해
  `gc/scheduler.rs`에 저장 시작 번호와 `GcGuard::save_started_since()`를 추가했습니다(추가 변경만,
  기존 API 유지).

## 남은 위험 / 한계

- planner와 idle 창은 storage 인터페이스입니다. 실제 idle 감지, 병합 결과를 rechunk 전용 저장으로
  기록하는 일(다음 저장의 이전 layout으로 넘기기), 문서 serializer 연결은 셸·런타임·project 계층의
  후속 작업입니다.
- remap manifest는 이번 병합 한 번의 재매핑만 만듭니다. 흡수된 root가 나중에 다시 분리되어 같은
  ChunkId를 되찾는 경우를 포함한 세대 간 remap 누적·정리 정책은 정하지 않았습니다.
- 의미 snapshot 검사는 storage 테스트용 인코딩으로 수행했습니다. 실제 문서 모델 기준 검사(XIV.3.5
  rechunk-only semantic diff 0)는 해당 작업에서 다시 해야 합니다.
- ADR-0011의 GC scheduler 설명(V.7.4)에는 병합 guard 공유가 아직 없습니다. 이번 허용 경로가 아니라
  ADR-0012 구현 기록에만 적었습니다.
