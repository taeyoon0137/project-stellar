# 작업 보고서: XVI.6.7 command sequence 장기 fuzz target

- WBS: XVI.6.7
- 작업명: command sequence fuzz target을 장기 nightly job으로 추가(crash seed 자동 artifact 저장)
- 상태: 완료 (`[x]`) — 2026-11-23 plan-revision이 행 지시의 "장기 nightly job"을 재판정의 개정 제안대로 장기 실행 진입점으로 고쳐 계획서에 반영했습니다. 재판정 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 결정론적 seed harness, 장기 실행 진입점, crash seed artifact 저장은 구현·검증했으나 **nightly CI job 등록은 하지 않았습니다**
  (아래 "차이" 참조). 선행 VII.6.3도 `[~]`입니다.
- 선행: VII.6.3(`[~]` — CI fuzz 등록만 미완, harness `fuzz_one`과 로컬 smoke는 통과)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). nightly CI 실행 증거 없음.

## 변경 파일 목록

- `crates/commands/src/sequence_fuzz.rs`(신규, `#![cfg(any(test, feature = "test-support"))]`, `#[doc(hidden)]`) — `run_sequence`,
  `run_campaign`, `SequenceFuzzConfig`(`smoke`, `from_env`), `SequenceStats`, `SequenceFuzzReport`, `CrashArtifact`, 테스트 3건 + `#[ignore]` nightly
  진입점 1건.
- `crates/commands/src/lib.rs` — 모듈 등록(test-support feature 뒤).
- `docs/task-reports/XVI.6.7.md`(신규), 계획서(XVI.6.7 `[ ]`→`[~]`).

## 구현 요약

- **seed 하나 = 결정론적 sequence**(`run_sequence(seed, steps)`): random valid document(VI.10.5)에서 시작해 단계마다 요청 1~3개짜리 새
  transaction, canonical bytes를 bit flip·절단해 디코딩된 transaction, undo(역 transaction 재실행), redo(정방향 재실행) 중 하나를 고릅니다.
  외부 fuzz 엔진·crate 없이 SplitMix64와 VII.6.1 생성기만 씁니다.
- **불변식(어기면 panic)**: 성공 실행 뒤 문서 전체 validator(VI.10.1) 통과와 revision +1, 거부된 transaction은 semantic snapshot·revision 불변
  (VII.1.6 원자성), undo는 직전 snapshot·redo는 직후 snapshot으로 복귀, 끝에서 전체 undo = 초기 snapshot·전체 redo = 최종 snapshot.
- **장기 실행**(`run_campaign`): seed 범위와 시간 예산(새 seed 시작 중단)을 받아 seed마다 `catch_unwind`로 panic을 잡고 다음 seed로 진행합니다.
  실패한 seed는 artifact 파일 `command-sequence-seed-<seed 16진수>.txt`(seed, 단계 수, panic 메시지, 재현 명령)로 저장합니다. seed와 단계 수만으로
  실행이 완전히 재현되므로 입력 bytes를 따로 저장하지 않습니다.
- **nightly 진입점**: `#[ignore]` test `sequence_fuzz_nightly`가 환경 변수(`STELLAR_SEQUENCE_FUZZ_FIRST_SEED`·`_SEEDS`·`_STEPS`·`_SECONDS`·
  `_ARTIFACT_DIR`)를 읽습니다. 기본값은 seed 100,000개·32단계·3,600초, artifact는 workspace `target/fuzz-artifacts/command-sequence`(Git 제외
  경로)입니다. crash가 있으면 artifact를 저장한 뒤 실패합니다. 실행 명령:
  `cargo test -p stellar-commands sequence_fuzz_nightly -- --ignored --nocapture`(panic 정책상 `--release` test는 쓰지 않음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands -p stellar-history` | 완료 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 82 passed·1 ignored(nightly)·doctest 3 passed, history unit 29 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-document workspace_production_code_does_not_use_test_support`(VI.10.4 검사) | 통과 |
| `STELLAR_SEQUENCE_FUZZ_FIRST_SEED=1000 STELLAR_SEQUENCE_FUZZ_SEEDS=200 STELLAR_SEQUENCE_FUZZ_STEPS=48 STELLAR_SEQUENCE_FUZZ_SECONDS=120 STELLAR_SEQUENCE_FUZZ_ARTIFACT_DIR=<scratch> cargo test -p stellar-commands sequence_fuzz_nightly -- --ignored --nocapture` | 통과(94.90s): seed 200개, crash 0, committed 4,745·rejected 505·변조 디코딩 899·undo 1,679·redo 202·최종 undo 깊이 3,345 |

| 검사 | 결과 |
|---|---|
| smoke 48 seed × 32단계 crash 0, committed 707·rejected 81·변조 디코딩 122·undo 263·redo 37·최종 undo 깊이 493(`smoke_command_sequence_fuzz_is_crash_free`) | 통과 |
| 같은 seed 같은 통계(seed 3·41·977), 다른 seed 다른 경로(`same_seed_replays_identically`) | 통과 |
| 주입 실패 runner: seed 10..16 중 13만 crash, 나머지 계속 진행, 중첩 디렉터리에 artifact 생성·내용 일치, artifact에서 seed·단계 수를 읽어 같은 실패 재현, 디렉터리 없으면 보고만, 시간 예산 0이면 seed 0개(`crash_seed_is_saved_as_artifact_and_replays`) | 통과 |
| 실제 결함 검출 확인(커밋하지 않은 임시 변경): RestoreNodeRecords root 종류 수정을 끄고 seed 1000..1200 × 48단계 nightly 실행 → crash 8건, artifact 8개 생성(예: seed 1042 step 5 "root … Frame이 아니라 fragment"), 그 artifact의 재현 명령(seed 1042, 1개)으로 같은 crash 재현. 수정 복원 후 같은 범위 crash 0 | 확인 |

## 차이(작업 지시와 다른 점)

- **nightly CI job 미등록**: 작업 지시는 "장기 nightly job으로 추가"이지만, 이 lane은 CI 파일을 수정할 수 없고 개정 기록(2026-11-14 "제품 테스트 실행
  위치")상 제품 crate 테스트는 CI에 추가하지 않습니다. nightly job이 부를 진입점·환경 변수·artifact 경로까지 준비했으나 실제 nightly 실행 증거가
  없으므로 `[~]`입니다. job을 등록하면 위 명령을 실행하고 `STELLAR_SEQUENCE_FUZZ_ARTIFACT_DIR`(또는 기본 `target/fuzz-artifacts/command-sequence`)를
  artifact로 업로드하면 됩니다.
- **선행 판단**: 행 선행 VII.6.3은 `[~]`(CI fuzz 등록만 미완)입니다. orchestration 지시가 이 작업을 명시적으로 배정해 구현했지만, 엄격한 행 선행 충족은
  아닙니다. VII.6.3이 `[x]`가 될 때 이 행의 상태도 다시 판정해야 합니다.
- fuzz 엔진 대신 결정론적 seed harness를 썼습니다(외부 의존성 금지). 입력 bytes 대신 seed가 재현 단위입니다.

## 남은 위험 / 한계

- coverage 유도(feedback) fuzzing이 아니므로 깊은 경로 탐색은 seed 수와 단계 수에 비례합니다. 실제로 RestoreNodeRecords root 결함은 smoke 48 seed에는
  나타나지 않고 200 seed 범위에서 8건 나타났습니다 — 장기 실행이 필요한 이유입니다.
- 변조는 transaction bytes 단위(bit flip·절단)이며 구조 인지(structure-aware) 변이는 아닙니다.
- 단계마다 semantic snapshot을 만들어 debug 빌드에서 seed당 약 0.3~0.5초입니다. nightly 기본 3,600초 예산이면 약 1만 seed 안팎입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 결정론적 seed harness, 장기 실행 진입점, crash seed artifact 저장·재현은 코드와 실제 결함 검출 실험으로 확인했습니다. 행 지시의 "장기 nightly job"은 제품 test를 CI에 넣지 않는 계획서 개정(2026-11-14)과 충돌해 실행할 job이 없습니다. `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/commands/src/sequence_fuzz.rs`: `run_sequence`, `run_campaign`(seed별 `catch_unwind`, artifact `command-sequence-seed-<seed>.txt`), `#[ignore]` `sequence_fuzz_nightly`(환경 변수로 범위·시간·artifact 경로).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-commands` | 123 passed, 4 ignored |

### 남은 조건과 finding

- 계획 개정 제안(통합 세션 판단): CI 정책에 맞춰 "장기 nightly job으로 추가"를 "장기 실행 진입점(`cargo test -p stellar-commands sequence_fuzz_nightly -- --ignored`)과 crash artifact 경로를 제공"으로 바꾸거나, 로컬 정기 실행 수단을 정합니다. 문구가 바뀌면 코드 변경 없이 `[x]`입니다.
- 선행 VII.6.3은 이번에 `[x]`로 판정했습니다.
