# 작업 보고서: XII.1.5 worker result 출처와 stale 거부

- WBS: XII.1.5
- 작업명: worker result에 source revision/hash를 포함하고 stale result를 적용하지 않음
- 상태: 완료 (`[x]`)
- 선행: XII.1.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`, 2026-11-14 로컬 Rust 검증.
- branch base 및 비교한 원격 `main`: `eac4531bd38f3c5dcac2c0e2743d8f2108b1ff3e`.

## 변경 파일 목록

- `crates/runtime/src/worker_result.rs`, `worker_result/tests.rs`: 입력 캡처·revision/hash·결과 channel·제출·gate와 판정 표 검사.
- `crates/runtime/tests/worker_rapid_edit.rs`, `tests/fixtures/worker_rapid_edit_v1.txt`: 결과 도착 순서 fixture, 3,000번 편집 stress, 실패 종료 및 포화 재제출 회귀 검사.
- `crates/runtime/src/lib.rs`: 모듈 등록.
- 이 보고서와 구현 계획·`planning/task-status.yaml`: 검증된 작업 상태 기록. 원장은 구현 커밋 이후 `cargo xtask generate-ledger`로 생성합니다.

## 구현 요약

- `CapturedInput`이 실제 계산 입력에서 128비트 hash를 계산하고 `DocumentRevision`과 묶습니다. `try_submit_sourced`는 그 출처를 결과까지 유지하며, 제출 거부 시 원래 job을 반환합니다.
- 결과 값은 `WorkerResultGate::admit`을 통과해야 꺼낼 수 있습니다. 기대 key·입력 hash·revision 구간을 검사하고 중복 결과도 버립니다.
- key 입력이 바뀌지 않은 문서 revision 증가는 기존 job을 유지합니다. 다른 node의 연속 편집 때문에 결과가 계속 폐기되는 문제를 막습니다. A → B → A 복귀는 새 revision 구간으로 취급합니다.
- gate는 문서 session별로 두고 key로 작업 종류를 구분합니다. 입력은 계산 중 불변이고 hash에 모든 계산 의존성이 있어야 합니다. reload로 revision을 초기화하면 이전 channel과 gate를 함께 버립니다.
- 결과 대기는 역할 없는 thread에서만 허용합니다. document·UI·render·worker thread는 `try_recv` 또는 `drain`을 씁니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime` | 종료 0, unit 57·app_runtime 8·command_queue_overflow 2·worker_pool_priority 8·worker_rapid_edit 4·doctest 11 통과 |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 종료 0, 경고 0 |
| `cargo fmt -p stellar-runtime -p stellar-renderer-core -- --check` | 종료 0 |
| `git diff --check` | 종료 0 |
| `cargo xtask generate-ledger` | 종료 0, 1,145개 작업 재생성·내부 검증 통과 |
| `cargo xtask sync-plan --check` · `cargo xtask validate-task-status` | 모두 종료 0 |
| `cargo xtask verify-task XII.1.4` · `cargo xtask verify-task XII.1.5` | 모두 종료 0, 현행 선행 검증 |
| `cargo xtask validate-task-report docs/task-reports/XII.1.4.md` · `cargo xtask validate-task-report docs/task-reports/XII.1.5.md` | 모두 종료 0 |
| `cargo xtask validate-adr <branch base 대비 변경 경로 전체>` · `cargo xtask validate-crate-graph` | 모두 종료 0 |

- 7개 rapid edit fixture의 apply 8·stale 11이 기대 판정과 일치합니다. 모든 적용 값은 적용 시점 현재 입력과 같습니다.
- 16개 key·3,000번 편집 stress에서 stale apply가 없고, 편집 종료 후 모든 key가 최신 결과로 수렴합니다.
- 포화 재제출 검사: r5 입력은 거부 후에도 r5 hash·revision·key를 보존합니다. r6 편집 후 재제출한 r5 결과는 stale로 버리고 r6 결과만 적용합니다.
- fixture 판정을 의도적으로 틀리게 한 회귀 검사에서 대기 job의 release sender가 먼저 해제되어 panic unwind가 join에서 멈추지 않습니다.
- 처리량·p99 benchmark는 수행하지 않았습니다. 출처 보존·적용 판정의 기능 검증입니다.

## 차이(작업 지시와 다른 점)

- `3892c5e3..ee2f41e5` 중 `a74430e2`의 결과 출처·gate와 `59be2255`의 fixture drop 순서 수정을 선별했습니다. 기존 결과 도착 fixture를 유지하고 포화 재제출·실패 종료 회귀 검사를 추가했습니다.
- XI.6.3의 scene revision 기반 tessellation guard와 X.7.5의 layout guard를 대체하지 않습니다. 실제 document thread에서 각 결과를 받는 배선은 후속 작업입니다.
- 이전 runtime autosave·edit_session과 XIII.1.2 보고서·상태 변경은 제외했습니다. main의 구현과 완료 기록을 유지합니다. 저장 영수증의 future revision·취소된 preview 처리 같은 session 전용 방어 규칙은 해당 저장 배선의 책임이며 이 gate가 디스크 current pointer를 전환하지 않습니다.
- WBS 설명·선행은 현행 그대로 유지합니다.

## 남은 위험 / 한계

- 소비자는 입력 변경마다 `expect` 또는 삭제 시 `forget`을 호출해야 합니다. 이를 빠뜨리거나 입력에 외부 가변 상태를 숨기면 gate만으로 stale을 판별할 수 없습니다.
- 입력 hash 충돌은 방어하지 않습니다. hash는 freshness 확인용이며 보안 인증 수단이 아닙니다.
- job panic이면 결과가 오지 않습니다. 현재 API는 다음 입력 변경에서 재제출하며 오류 결과·명시적 retry 정책은 아직 없습니다.
- 결과 channel은 unbounded입니다. 실제 앱 연결 시 소비 속도·메모리 예산을 검증하고 배압 정책을 정해야 합니다. 실행 전 stale job 취소도 아직 없습니다.
- 저장 직렬화·chunk write·base revision 확인·current pointer 전환(XIII.1.4), 통합 thread 종료(XII.1.6)는 미완료 상태로 남깁니다.
- macOS 변경이 없어 smoke는 실행하지 않았으며 전체 통합 검사는 별도 담당 범위입니다.

원장 재생성 후 비교에서 XII.1.4·XII.1.5 두 record만 달라졌고 WBS 설명·선행은 원문과 동일합니다. XIII.1.2 완료 record·보고서·project 구현 및 기존 runtime 배선·의존성은 branch base와 동일합니다.

종료 전 `git ls-remote origin refs/heads/main` 재확인 결과 원격은 `218a671cbd636aad0e8b997ef67ef80295e13f49`였습니다. base 이후 추가된 커밋은 수동 CI 검사 설정이며, 이 작업의 runtime·tessellation 파일·WBS 행·원장에는 변화가 없습니다. 해당 설정은 이 lane에 가져오지 않았고 현재 브랜치의 선별 패치만 통합 대상으로 제공합니다.
