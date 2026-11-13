# 작업 보고서: X.7.5 layout worker result guard

- WBS: X.7.5
- 작업명: layout result가 DocumentRevision과 input hash에 맞지 않으면 폐기하는 worker result guard 구현
- 상태: 완료 (`[x]`)
- 선행: II.3.4(`[x]`), X.1.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/guard.rs`(신규) — `LayoutInput::input_hash`, `LayoutJob`, `LayoutWorkerResult`, `StaleReason`, `GuardOutcome`, `GuardError`, `GuardStats`, `LayoutResultGuard`, 테스트.
- `crates/layout/src/lib.rs` — `pub mod guard;`.
- `docs/task-reports/X.7.5.md`(신규), 계획서(X.7.5 `[ ]`→`[x]`).

## 구현 요약

- **job 식별**: `LayoutJob { revision: DocumentRevision, input_hash: LayoutInputHash }`. 입력 hash는 입력 canonical
  bytes를 X.1.4 `LayoutInputHasher`(FNV-1a 64 + 독립 check hash)로 계산합니다.
- **guard**: 현재 기대 job을 보관하고 `submit`은 결과 job의 revision과 입력 hash가 **둘 다** 같을 때만 적용합니다.
  revision 불일치(과거·미래)·hash 불일치(같은 revision 다른 입력)·이미 적용한 job의 중복은 `Discarded(이유)`이며
  적용 상태를 바꾸지 않습니다. 적용할 때마다 `LayoutRevision`이 1 증가합니다.
- **revision 역행 금지**: `expect`로 기대 revision을 이전 값으로 되돌리면 `RevisionRegressed` 오류입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 117 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 늦은 이전 revision 결과·같은 revision 다른 입력 결과·미래 revision 결과 폐기, 적용 0 확인 후 맞는 결과만 적용, 중복 폐기, 집계 applied 1/discarded 4(`stale_revision_and_hash_results_are_discarded`) | 통과 |
| 결정적 PRNG로 문서 변경·작업 시작·늦은 도착 400단계 교차: stale 적용 0, 적용된 결과는 항상 현재 입력의 계산 결과, 적용·폐기 모두 발생(`interleaved_workers_never_apply_stale_results`) | 통과 |
| 기대 revision 역행 오류, 입력 hash 결정성·값 변화 반영(`expected_revision_cannot_regress_and_hash_is_canonical`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 worker pool(스레드)은 이 crate 범위가 아니라 guard 판정 로직과 결정적 교차 시뮬레이션으로 검증했습니다.

## 남은 위험 / 한계

- 입력 hash는 입력 전체 canonical bytes 기준입니다. 부분(서브트리) 결과 guard는 증분 레이아웃(X.7.2~X.7.3) 이후 소관입니다.
