# 작업 보고서: XII.1.4 worker pool class·priority와 저장·GC 실행 상한

- WBS: XII.1.4
- 작업명: worker pool을 text shaping, tessellation, image decode, save serialization class로 나누거나 priority tag로 구분
- 상태: 완료 (`[x]`)
- 선행: X.7.5 (`[x]`), XI.6.3 (`[x]`), XI.7.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`, 2026-11-14 로컬 Rust 검증.
- 비교 기준: 원격 `main`과 branch base가 `eac4531bd38f3c5dcac2c0e2743d8f2108b1ff3e`로 동일함을 `git ls-remote origin refs/heads/main`으로 확인했습니다.

## 변경 파일 목록

- `crates/runtime/src/worker_pool.rs`, `worker_pool/tests.rs`: 공통 pool·설정·통계·종료와 단위 테스트.
- `crates/runtime/tests/worker_pool_priority.rs`: 실제 worker의 우선순위·포화·저장·GC 상한 검사.
- `crates/runtime/src/lib.rs`: 모듈 등록.
- `crates/renderer-core/src/tessellation_worker.rs`: 별도 tessellation pool과의 관계 주석.
- 이 보고서와 구현 계획·`planning/task-status.yaml`: 검증된 작업 상태 기록. 원장은 구현 커밋 이후 `cargo xtask generate-ledger`로 생성합니다.

## 구현 요약

- 6개 class(text shaping, tessellation, image decode, save serialization, cache generation, GC)와 3개 priority를 제공합니다. class의 기본 priority를 job별로 바꿀 수 있습니다.
- `Interactive` → `Normal` → `Background` 순서, lane 내부 FIFO입니다. 각 lane이 포화되면 기존 job을 버리지 않고 새 payload를 반환합니다.
- 저장(`Normal`)과 GC·캐시(`Background`)의 합산 동시 실행 상한은 기본 worker 수 - 1입니다(worker 1개면 1). background 자체 상한도 별도로 적용하여 interactive worker 한 자리를 남깁니다.
- worker는 이름과 `ThreadRole::Worker`를 가지며 이미지 decode scope를 얻을 수 있습니다. 역할이 있는 thread의 `wait_idle`은 거부합니다.
- 종료 시 미시작 job을 폐기하고 실행 중 job에 취소 표시를 전달합니다. job이 pool의 마지막 owner를 놓으면 현재 worker를 자기 join하지 않고 job 반환 후 종료합니다. 다른 worker는 join하고 공유 상태를 회수합니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib last_pool_owner_can_be_dropped_inside_its_worker` (수정 전) | 종료 101, 자기 join panic 재현 |
| `cargo test -p stellar-runtime` (수정 후) | 종료 0, unit 53·app_runtime 8·command_queue_overflow 2·worker_pool_priority 8·doctest 11 통과 |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 종료 0, 경고 0 |
| `cargo fmt -p stellar-runtime -p stellar-renderer-core -- --check` | 종료 0 |
| `git diff --check` | 종료 0 |
| `cargo xtask generate-ledger` | 종료 0, 1,145개 작업 재생성·내부 검증 통과 |
| `cargo xtask sync-plan --check` · `cargo xtask validate-task-status` | 모두 종료 0 |
| `cargo xtask verify-task XII.1.4` · `cargo xtask verify-task XII.1.5` | 모두 종료 0, 현행 선행 검증 |
| `cargo xtask validate-task-report docs/task-reports/XII.1.4.md` · `cargo xtask validate-task-report docs/task-reports/XII.1.5.md` | 모두 종료 0 |
| `cargo xtask validate-adr <branch base 대비 변경 경로 전체>` · `cargo xtask validate-crate-graph` | 모두 종료 0 |

- 긴 저장 2개와 GC 1개를 worker 2개에 제출해도 shaping은 저장 완료 전에 실행됩니다. 최대 non-interactive 실행 수는 1입니다.
- 4개 생산자의 혼합 job 400개 모두 완료되고 background·합산 실행 상한을 지킵니다.
- 새 종료 회귀 검사는 worker 안에서 마지막 owner를 drop한 뒤 job 반환과 공유 상태 회수를 확인합니다.
- 처리량·지연 예산 benchmark는 수행하지 않았습니다. 상한과 실행 순서의 기능 검증입니다.

## 차이(작업 지시와 다른 점)

- `3892c5e3..ee2f41e5` 중 `bc9a5f93`의 pool, `a7eff257`의 저장·GC 합산 상한, `4caecba4`의 별도 tessellation pool 설명만 선별했습니다. 추가로 자기 join 오류를 고쳤습니다.
- 기존 main의 XIII.1.2(`stellar-project::autosave::dirty`)를 유지합니다. 이전 runtime `autosave`·`edit_session` 구현 및 그 전용 의존성은 가져오지 않았습니다. 기존 `plan_save`는 preview 중 캡처를 거부합니다.
- WBS의 설명·검증 기준·선행은 바꾸지 않습니다. class와 priority 제공이 이 행의 완료 범위이며 실제 앱 배선을 완료한 것으로 표시하지 않습니다.

## 남은 위험 / 한계

- XI.6.3 `AsyncTessellation`은 별도 pool을 소유합니다. 공통 pool로 이전하려면 renderer-core의 job 제출 추상화가 필요하며, 이전 전 두 pool을 함께 쓰면 thread 예산을 함께 정해야 합니다. `for_available_parallelism`은 별도 tessellation thread를 세지 않습니다.
- 실제 text shaping·decode·저장 경로와 앱 runtime의 pool 소유·종료 배선은 미연결입니다. 저장 직렬화·chunk write·stale current pointer 차단은 XIII.1.4, 앱 전체 종료 순서는 XII.1.6 후속 범위입니다. 이 행에서 그 상태를 올리지 않습니다.
- 실행 중인 job은 선점하지 않습니다. 취소를 무시하고 끝나지 않는 job은 종료를 지연시킵니다. worker 1개 또는 합산 상한을 worker 수로 올린 설정에는 interactive 전용 자리가 없습니다.
- 엄격한 우선순위 때문에 interactive 입력이 계속되면 background 작업이 늦어질 수 있습니다. 실제 저장 지연 측정은 XIII.1.4 연결 후 필요합니다.
- macOS 앱 파일은 바꾸지 않아 `scripts/macos/smoke.sh`는 실행하지 않았습니다. 전체 통합 검사는 별도 담당 범위입니다.

원장 재생성 후 비교에서 XII.1.4·XII.1.5 두 record만 달라졌고 WBS 설명·선행은 원문과 동일합니다. XIII.1.2 완료 record·보고서·project 구현 및 기존 runtime 배선·의존성은 branch base와 동일합니다.

종료 전 `git ls-remote origin refs/heads/main` 재확인 결과 원격은 `218a671cbd636aad0e8b997ef67ef80295e13f49`였습니다. base 이후 추가된 커밋은 수동 CI 검사 설정이며, 이 작업의 runtime·tessellation 파일·WBS 행·원장에는 변화가 없습니다. 해당 설정은 이 lane에 가져오지 않았고 현재 브랜치의 선별 패치만 통합 대상으로 제공합니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U034 수용(아키텍처 §9.2와 맞고 자기 join panic을 고침).
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |
| `tests/worker_pool_priority.rs` (workspace 실행에 포함) | 0 — 8 통과 |

- 검증 기준(interactive 작업이 background GC보다 먼저 실행): `worker_pool_priority` 8개와 `worker_pool::tests::last_pool_owner_can_be_dropped_inside_its_worker` 통과.
- 판정: `[x]` 유지.
