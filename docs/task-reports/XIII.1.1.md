# 작업 보고서: XIII.1.1 AutosavePolicy와 config 읽기

- WBS: XIII.1.1
- 작업명: `AutosavePolicy`에 debounce, 최대 지연, 앱 비활성화 flush, 창 닫기 flush를 정의하고 값은
  `config/performance-budgets.yaml`에서 읽음
- 상태: 완료 (`[x]`) — 2026-11-23 plan-revision이 선행 "Phase XII, V.5.4"를 재판정의 재작성안대로 "V.5.4"로 좁혀 계획서에 반영했습니다. 재판정 기록: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 정책 타입·config 읽기·거부 규칙 구현과 검증 완료. `config/performance-budgets.yaml`에
  넣을 **수치는 아직 정해지지 않아** 기록하지 않았습니다(아래 "차이").
- 선행: V.5.4(2026-11-23 개정으로 "Phase XII"를 뺌. 이전 기록: Phase XII(완화 — 아래 "차이"), V.5.4)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/autosave.rs` (신규: `AutosavePolicy`, `FlushPolicy`, `AutosaveConfigError`,
  `AUTOSAVE_SECTION`, 테스트)
- `crates/project/src/lib.rs` (`pub mod autosave;` 추가)
- `docs/task-reports/XIII.1.1.md`(신규), 계획서(XIII.1.1 `[ ]`→`[~]`).
- `config/performance-budgets.yaml`·ADR-0013은 **변경·생성하지 않았습니다**(아래 "차이").

## 구현 요약

- `AutosavePolicy { debounce, max_delay, app_deactivate_flush, window_close_flush }`. flush 둘은 기술
  아키텍처 22.1이 요구하는 동작이라 끌 수 없고 제한 시간(`FlushPolicy { timeout }`, XIII.1.6에서 사용)만
  설정합니다.
- `AutosavePolicy::from_budgets_yaml`/`load`는 제한 YAML(IV.1)로 `performance-budgets.yaml`을 읽어
  `autosave` 절의 `debounce_ms`·`max_delay_ms`·`app_deactivate_flush_timeout_ms`·
  `window_close_flush_timeout_ms`(밀리초 정수)를 해석합니다.
- **거부 규칙**: 절·키 누락은 기본값으로 대체하지 않고 오류(`MissingSection`·`MissingKey`), 알 수 없는 키,
  정수가 아니거나 음수·소수·`u32` 초과·비스칼라 값(`InvalidValue`), 0인 시간과 `max_delay_ms <
  debounce_ms`(`OutOfRange`)를 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 245 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **config 누락 거부**: 절 없음·최상위 비매핑·절 비매핑·키 4개 각각 누락·파일 없음·문법 오류(`missing_config_is_rejected_without_defaults`) | 통과 |
| **범위 밖 값 거부**: 0 debounce·0 timeout·`max_delay < debounce`·음수·소수·`u32` 초과·비스칼라·알 수 없는 키(`out_of_range_values_are_rejected`) | 통과 |
| 정상 절 해석과 `max_delay == debounce` 경계 허용(`valid_section_is_read`, 테스트 전용 값) | 통과 |
| 저장소 `config/performance-budgets.yaml`은 제한 YAML로 읽히며 현재 `autosave` 절이 없어 `MissingSection`(다른 오류·기본값 대체 없음)(`repository_budget_file_is_well_formed`) | 통과 |

## 차이(작업 지시와 다른 점)

- **수치 미기록**: 작업 지시는 값을 `config/performance-budgets.yaml`에서 읽게 하고, 이 파일 변경에는 근거가
  기술 아키텍처에 있는 ADR(ADR-0013)이 필요합니다. 그러나 기술 아키텍처 22.1은 "Debounce 수치는 사용성·SSD
  쓰기·복구 손실 범위 벤치마크로 확정"이라 하고 42장 2번이 "자동 저장 debounce 시간"을 구현 검증 후
  확정할 값으로 둡니다. 근거 있는 수치가 없으므로 config 키와 ADR-0013을 추가하지 않았고, 그 결과 실제
  config에서는 정책을 읽지 못하는 상태(`MissingSection`)라 `[~]`로 둡니다. 벤치마크로 값을 정하면 ADR-0013과
  함께 `autosave` 절을 추가하는 것만 남습니다(코드 변경 불필요).
- 선행 "Phase XII"는 사용자 지시(2026-11-14)에 따라 완화했습니다. 정책 타입과 config 읽기는 셸·runtime
  thread 코드가 필요하지 않습니다.
- 앱 비활성화·창 닫기 flush를 켜고 끄는 키는 두지 않았습니다(아키텍처 필수 동작).

## 남은 위험 / 한계

- 수치 결정(벤치마크) 전까지 자동 저장 scheduler(XIII.1.3~)는 실제 config로 시작할 수 없습니다.
- 테스트의 700/5000/2000/3000ms는 형식 검증용 입력일 뿐 제품 값이 아닙니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: 이전 `[~]` 사유(config 수치 미기록)는 해소되었습니다. `config/performance-budgets.yaml`에 `autosave` 절이 ADR-0072와 함께 있고 저장소 파일을 실제로 읽는 test가 있습니다. 남은 사유는 선행 "Phase XII"(`[ ]` 61행)뿐입니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/project/src/autosave.rs` `AutosavePolicy::from_budgets_yaml`·`load`: 누락 절·키는 기본값 없이 거부, 0 이하·`max_delay_ms < debounce_ms`·범위 밖은 `OutOfRange`. `repository_budget_file_is_well_formed`가 저장소 config를 읽습니다.
- `config/performance-budgets.yaml` `autosave`: debounce 1000, max delay 10000, 앱 비활성화 flush 2000, 창 닫기 flush 10000(ms). 근거 ADR-0072.
- `crates/runtime/src/app/autosave.rs`가 같은 파일을 `include_str!`로 읽습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-project` | 392 passed, 0 failed, 1 ignored |

### 남은 조건과 finding

- 선행 재작성안(통합 세션 판단): 정책 타입과 config 읽기는 셸·창 코드가 필요 없으므로 "Phase XII, V.5.4"를 "V.5.4"로 바꿉니다. 그러면 이 행은 `[x]`이고, XIII.1.3(선행 XIII.1.1, XII.1.1 `[x]`)이 착수 가능해집니다.
- XIII.1.3은 `[ ]`이지만 지연 자동 저장 trigger가 이미 들어와 있습니다(`b9392f8b`, `crates/project/src/autosave/schedule.rs`). XIII.1.3 착수 때 이 코드를 기준으로 판정해야 합니다.
