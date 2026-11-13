# 작업 보고서: VIII.7.3 환경 입력 reverse index

- WBS: VIII.7.3
- 작업명: environment 변경 시 해당 key를 참조한 expression만 invalidate하는 reverse index
- 상태: 완료 (`[x]`)
- 선행: VIII.6.4(`[x]`, 이 lane), VIII.7.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/environment_index.rs`(신규) — `EnvironmentIndex`(key → 직접 참조 producer), `EnvironmentSources`
  (환경 node는 registry·`EvaluationEnvironment`에서), `BindingEvaluator::apply_environment`와 테스트.
- `crates/bindings/src/lib.rs` — `environment_index` 모듈 등록.
- `docs/task-reports/VIII.7.3.md`(신규), 계획서(VIII.7.3 `[ ]`→`[x]`).

## 구현 요약

- **reverse index**: 통합 graph의 환경 node마다 그 key를 직접 참조하는 표현식(producer node: 토큰·binding)을 번호 순으로
  모읍니다.
- **무효화**: `apply_environment(index, previous, next)`가 `changed_keys`(VIII.7.2: 평가 값이 실제로 바뀐 key만)를 구해
  index에 있는(표현식이 참조하는) key만 scheduler(VIII.6.4) source 변경으로 넘깁니다. 그 key의 reader와 dependents만
  위상 순서로 재평가되고, 다른 key만 읽는 표현식은 평가하지 않습니다.
- **source 제공**: `EnvironmentSources`가 환경 node 타입을 key registry(VIII.7.1)에서, 값을 환경(명시 값 또는 기본값)에서
  주고 나머지 source는 다른 제공자에 맡깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (39 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **color scheme 변경 시 viewport-only expression 미평가**(`color_scheme_change_does_not_evaluate_viewport_only_expressions`): light→dark에서 평가 `[p1]`만(p2 viewport·p3 dependent 0), viewport 변경은 `[p2, p3]`, 참조 없는 key·기본값과 같은 명시 값은 평가 0, 값 갱신 정확 | 통과 |
| reverse index reader 정확, 참조 없는 key는 비어 있음, 기본 환경 값으로 초기 평가(`reverse_index_lists_direct_readers`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- 토큰 pull cache(VIII.5.4)의 환경 무효화는 이미 읽은 입력 값 비교로 동작하므로 이 index는 push scheduler 경로에만
  연결했습니다.

## 남은 위험 / 한계

- index는 graph에서 만들므로 graph를 다시 만들면 index도 다시 만들어야 합니다.
