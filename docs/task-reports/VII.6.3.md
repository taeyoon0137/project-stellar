# 작업 보고서: VII.6.3 command 실행 fuzz target

- WBS: VII.6.3
- 작업명: command 실행 중 panic·out-of-bounds가 없는 fuzz target 추가
- 상태: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — fuzz target과 로컬 smoke는 통과하지만, 검증 기준의 "CI fuzz smoke duration"은 CI에 등록하지 않았습니다
  (CI 파일 수정은 이번 작업 범위 밖).
- 선행: VII.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). CI 등록 증거 없음.

## 변경 파일 목록

- `crates/commands/src/fuzz.rs`(신규, `#[doc(hidden)]` 테스트 전용) — `fuzz_one`, `FuzzStats`, `SMOKE_INPUTS`, smoke test.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.6.3.md`(신규), 계획서(VII.6.3 `[ ]`→`[~]`).

## 구현 요약

- **외부 엔진 없는 결정론적 harness** `fuzz_one(input)`: 외부 crate(libFuzzer 등)를 쓰지 않습니다.
  1. 임의 bytes(전체와 7바이트 간격 suffix)를 command·요청·transaction·선택 event로 디코딩합니다(거부는 정상).
  2. 입력 seed의 random valid document 위에서 생성기(VII.6.1) command의 canonical bytes를 입력 bytes로 bit flip·덮어쓰기·절단해 디코딩되면 실행합니다.
  3. 유효 후보 실행과 직전 성공 command의 역 실행을 섞습니다.
- **판정**: 어떤 경로에서도 panic·out-of-bounds가 없어야 하고, 실행이 성공하면 문서가 전체 validator(VI.10.1)를 통과해야 합니다(불변식 위반은
  panic으로 드러남). transaction guard 재생 실패는 debug 빌드 `debug_assert`로 드러납니다.
- **smoke 범위**: 결정론적 corpus `SMOKE_INPUTS = 600`개(길이 1..160) + 빈 입력·`0xFF`·`0x00` 극단값. 실행·변조 디코딩·거부 횟수 하한으로
  smoke가 실제 경로를 지나는지 확인합니다. 장기 nightly(XVI.6.7)나 외부 엔진 연결은 같은 `fuzz_one`을 호출하면 됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 77 passed, doctest 3 passed, 0 failed(10.25s) |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |
| `cargo test -p stellar-commands smoke_fuzz`(입력 1,500개로 늘린 임시 확장 실행) | 통과(26.94s), 이후 smoke 기본값 600으로 고정 |

| 검사 | 결과 |
|---|---|
| 603개 입력: panic 0, 실행 성공 4,786·변조 디코딩 2,974·거부 2,442, 성공 실행마다 문서 validator 통과(`smoke_fuzz_command_execution_is_panic_free`) | 통과 |

## 차이(작업 지시와 다른 점)

- **CI 등록 미완**: 현재 CI(`.github/workflows`)는 crate 테스트를 실행하지 않고, 이번 lane은 CI 파일을 수정할 수 없어 "CI fuzz smoke duration"
  증거가 없습니다. 그래서 `[~]`로 표기합니다. smoke는 `cargo test -p stellar-commands`에 포함되어 있어 CI job이 crate 테스트를 돌리면 함께
  실행됩니다.
- fuzz 엔진 대신 결정론적 corpus를 썼습니다(외부 의존성 금지).

## 남은 위험 / 한계

- coverage 유도(feedback) fuzzing이 아니므로 깊은 경로 탐색은 제한적입니다. nightly 장기 실행·crash seed artifact 저장은 XVI.6.7의 몫입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다.
- 근거: fuzz target은 panic·out-of-bounds 없음을 검사하고, 성공 실행마다 문서 validator를 통과시킵니다. 이전 `[~]` 사유인 "CI 미등록"은 계획서 개정(2026-11-14 "제품 테스트 실행 위치": 행 지시의 CI smoke는 통합 시 로컬 `cargo xtask check-core`를 뜻함)으로 사라졌습니다. smoke는 고정 corpus 603개 입력으로 정해져 있고 `cargo test --workspace`에 포함됩니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/commands/src/fuzz.rs` `fuzz_one`(82행): 임의 bytes를 command·요청·transaction·선택 event로 디코딩하고, 변조 command를 실행·역 실행합니다. `SMOKE_INPUTS = 600`(144행) + 극단값 3개, `smoke_fuzz_command_execution_is_panic_free`(151행).
- `tools/xtask/src/check_core.rs`: `cargo test --workspace --locked`를 실행하므로 smoke가 통합 검증에 들어 있습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-commands` | 123 passed, 0 failed, 4 ignored(장기 실행 진입점) |

### 남은 조건과 finding

- coverage 유도 fuzzing은 아닙니다(이전 보고서 한계). 이 판정으로 XVI.6.7의 선행이 풀립니다.
