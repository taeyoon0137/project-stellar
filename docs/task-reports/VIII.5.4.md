# 작업 보고서: VIII.5.4 토큰 값 평가 cache

- WBS: VIII.5.4
- 작업명: token value evaluation cache를 구현하고 dependency revision이 바뀐 token만 invalidate
- 상태: 완료 (`[x]`)
- 선행: VIII.4.6(`[~]`, Windows 비교만 ADR-0006으로 보류된 부분 완료 — lane 규칙상 충족으로 봄),
  VIII.5.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/token_cache.rs`(신규) — `TokenValueCache`, `TokenInputs`, `TokenEvalError`, `TokenValue`,
  `CacheStats`와 테스트.
- `crates/bindings/src/registry.rs` — 토큰별 값 revision(`value_revision`) 추가: 정의 추가·표현식 교체 때 갱신,
  rename은 유지. `PartialEq`를 revision과 무관한 내용 비교로 변경.
- `crates/bindings/src/lib.rs` — `token_cache` 모듈 등록.
- `docs/task-reports/VIII.5.4.md`(신규), 계획서(VIII.5.4 `[ ]`→`[x]`).

## 구현 요약

- **pull cache**: `get(token, registry, inputs)`·`get_all`이 요청 시 계산합니다. 항목은 계산 당시의 토큰 값 revision,
  참조 토큰마다 그 값이 마지막으로 바뀐 시점(`changed_at`), 토큰이 아닌 입력(환경 입력 등)마다 읽은 값을 기록합니다.
- **무효화**: 요청마다 dependency를 먼저 확인(명시적 stack, 비재귀)하고 기록과 모두 같으면 평가 없이 재사용합니다.
  다시 평가한 결과가 이전과 같으면 `changed_at`을 유지해(early cutoff) 상위 토큰도 재평가하지 않습니다. 없는 토큰
  dependency는 표식으로 기록해 그 토큰이 생기면 무효화됩니다.
- **오류**: 없는 토큰, 메모리 상태 cycle(cycle 위 토큰 전부 `Cycle`, 요청 순서와 무관), 컴파일 실패, 평가 실패,
  실패한 토큰을 실제로 읽은 경우 `Dependency`(선택되지 않은 branch는 읽지 않음, VIII.4.3 short-circuit과 일치).
  cache 값은 이번 확인 session에서 검증된 것만 읽어 오래된 값을 쓰지 않습니다.
- **평가 한도**: VIII.4.5 `EvalLimits`(기본값, 값 bytes 상한 포함)를 그대로 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (15 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **무관 token 변경 시 평가 count 0**(`unrelated_token_change_evaluates_nothing`): A·B=A+A·C·D=C+C에서 C 변경 후 B 요청 평가 0, D 요청은 C·D 2회, 변경 없는 전체 재요청 0 | 통과 |
| rename 후 revision 유지·평가 0, 같은 값이 나오는 표현식 교체 시 early cutoff(B 재평가 없음)(`rename_keeps_and_equal_result_cuts_off`) | 통과 |
| 환경 입력(dark) 변경 시 그 입력을 읽는 토큰만 1회 재평가(`environment_input_change_invalidates_only_readers`) | 통과 |
| 메모리 상태 cycle 경로 정확·해소 후 정상, 없는 토큰, 삭제·재추가 시 무효화, 실패 토큰 미선택 branch는 정상(`errors_cycles_and_missing_tokens_are_reported`) | 통과 |

## 차이(작업 지시와 다른 점)

- 행의 "dependency revision"을 토큰 정의 revision + dependency 값의 `changed_at`으로 구현했습니다. 간접 dependency의
  값 변화도 이 방식으로 전파되며, 값이 같으면 전파하지 않습니다.
- 환경 입력은 `EvaluationEnvironment`(VIII.7.2, 이후 작업) 대신 `TokenInputs` trait으로 받습니다. 입력 revision 대신
  읽은 값을 비교합니다.
- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).

## 남은 위험 / 한계

- 재사용 확인은 요청한 토큰의 dependency 하위 graph 전체를 걷습니다(평가는 없음). 대규모 graph의 push 방식 증분
  재평가는 binding scheduler(VIII.6.4)가 맡습니다.
- 다시 평가할 때마다 default expression을 다시 컴파일합니다(컴파일 결과는 cache하지 않음).
