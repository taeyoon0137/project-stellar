# 작업 보고서: XVI.6.4 expression AST·typechecker·evaluator fuzz target

- WBS: XVI.6.4
- 작업명: expression AST/typechecker/evaluator fuzz target 추가(instruction budget 작동, panic 0)
- 상태: 완료 (`[x]`)
- 선행: VIII.4.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, debug 기본 실행 + release 확장 실행).

## 변경 파일 목록

- `crates/expressions/tests/expression_fuzz.rs`(신규) — 결정론적 fuzz harness(타입 인지 AST 생성기, 구조·bytes 변형,
  고정 stress 입력, 판정), `#[test]` 3개.
- `crates/expressions/src/codec.rs` — fuzz 발견 수정: `write_node`가 같은 slot이 두 번 이상 있는 node를
  `CodecError::NonCanonical`로 거부(decoder가 거부할 bytes를 만들지 않음). 회귀 단위 테스트 1개 추가.
- `crates/expressions/src/logic.rs` — fuzz 발견 수정: `check_match` pattern 중복 검사를 선형 재탐색(arm 수의 제곱)에서
  `BTreeSet`(O(n log n))으로 변경. 오류 위치·종류는 같습니다.
- `crates/expressions/src/ast.rs` — fuzz 발견 수정: `ExprNode::child`를 정렬된 child 목록의 이진 탐색으로 변경(같은 slot이
  여럿이면 이전과 같이 정렬상 첫 child). type 검사·컴파일·AST 평가가 arm마다 선형 탐색하던 제곱 비용을 없앱니다.
- `docs/task-reports/XVI.6.4.md`(신규), 계획서(XVI.6.4 `[ ]`→`[x]`).

## 구현 요약

### harness (`tests/expression_fuzz.rs`)

XVI.6.1·XVI.6.2·XVI.6.5와 같은 방식(고정 seed SplitMix64 `#[test]`, 새 의존성·nightly 없음, `catch_unwind`, worker
thread 기본 2 MiB stack, 입력당 시간 상한 + 전체 watchdog, 발견 입력은 `CARGO_TARGET_TMPDIR/expression-fuzz/`에 저장 후
실패)입니다. 기본 반복 1,200, `STELLAR_EXPRESSION_FUZZ_ITERATIONS`로 늘립니다.

- **입력**
  1. 타입 인지 생성기: 9개 값 타입(Boolean·Number·String·Enum·단위 4종·Color)과 Optional을 목표로 node 종류 7·이항
     13·단항 2·함수 16을 모두 만들고, 완성된 subtree를 재사용해 DAG 공유를 만듭니다. 참조 입력에는 값 누락·null·
     선언과 다른 타입의 값이 섞이고, 약 1/4은 일부러 타입을 어긋나게 만든 위치를 포함합니다.
  2. 구조 변형(생성 AST당 3개): child 대상 변경(cycle·도달 불가), slot 변경·삭제·추가, payload 교체, ID 중복, root
     변경, 없는 child 대상.
  3. canonical AST bytes 변형(생성 AST당 6개): bit 뒤집기·경계 byte·삽입·구간 삭제·`u16`/`u32` 경계값·절단·구간 복제.
  4. 고정 stress: 왼쪽 중첩(전체 규모 60만 단계 = 명령 120만 개), 부호 반전 사슬, 300단계 공유 block 중첩, 7만 단계
     오른쪽 중첩, `if` 중첩, arm이 많은 `match`, 공유 `concat` 증폭. release 빌드(또는
     `STELLAR_EXPRESSION_FUZZ_STRESS=full`)는 전체 규모, debug 기본 실행은 축소 규모(왼쪽 중첩 6만 단계 + budget 10만,
     부호 반전 3만, `if` 3만, `match` 2만 arm)입니다.
- **판정**
  - panic 0, hang 0. 구조: 생성 AST 검증 통과, canonical round-trip 동일, 수용한 변형 bytes의 재인코딩이 입력과
    byte-identical, 직렬화에 성공한 node는 같은 node로 복원, 검증된 AST는 cycle 없음.
  - typechecker: 컴파일 성공 ⇔ type 오류 0, program 결과 타입 = root 추론 타입, 타입을 지켜 만든 AST는 통과,
    검증된 program 평가에서 방어 오류(`OperandTypeMismatch`·`MalformedProgram`·`IllTyped`) 0, 성공 값은 추론 타입과 일치.
  - evaluator: 한도 오류가 아니면 bytecode 결과 = AST 기준 결과(오류 node 포함).
  - **instruction budget**: 성공한 평가의 명령 수 `n`에 대해 budget `n`은 같은 결과·같은 명령 수, budget `n − 1`은
    `InstructionBudgetExceeded { budget: n − 1 }`. stack 한도도 peak / peak − 1로 같은 검사. 무작위 작은 한도(명령
    0\~399, frame 1\~8, stack 1\~16, 값 0\~4,095 bytes)에서 두 evaluator의 결과는 기본 한도 결과 또는 설정 값을 보고하는
    한도 오류뿐입니다. trace는 항상 기본 한도 이하입니다.

### fuzz 발견과 수정

| 발견 | 입력 | 수정 전 결과 | 수정 |
|---|---|---|---|
| node 직렬화 비대칭 | 같은 slot이 두 번 있는 node(구조 변형) | 기본 캠페인 발견 130건(저장 파일로 확인한 128건이 모두 이 유형, 2건은 같은 실행의 stress 발견 파일에 덮어써져 미확인): `ExprNode::to_bytes` 성공 → `from_bytes`가 `NonCanonical`로 거부. 수정 후 같은 seed 재실행 발견 0 | `write_node`가 중복 slot node를 `NonCanonical`로 거부 |
| `match` arm 수 제곱 비용 | 2만 arm `match`(stress) | debug에서 검증·type 검사·컴파일·평가 19.34 s(pattern 중복 선형 재탐색 + arm마다 `child` 선형 탐색) | `BTreeSet` 중복 검사 + `child` 이진 탐색 → 같은 입력 debug 0.32 s, 6만 arm release 0.11 s |

나머지 3건은 harness 쪽 판정 오류였습니다(작은 budget에서 budget 오류 대신 더 먼저 나는 `CallDepthExceeded`·
`StringTooLong`를 실패로 본 것, debug 60만 node stress가 시간 상한 60 s를 넘은 것). 작은 한도 판정을 "기본 결과 또는
설정한 한도 오류"로 바로잡고 debug stress 규모를 줄였습니다. 60만 node 입력은 debug에서 검증·cycle 검사만 수십 초가
걸리지만(측정: `ExpressionAst::new` 22.8 s, release 0.38 s) 단계별로 선형이며 hang이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions --test expression_fuzz -- --nocapture` (수정 전 harness 첫 실행) | 종료 코드 101: 캠페인 발견 130건(위 표), stress 발견 3건(harness 판정 2 + 시간 1), stress `match` 2만 arm 19.34 s |
| `cargo test -p stellar-expressions --test expression_fuzz -- --nocapture` (수정 후) | 종료 코드 0, 3 passed |
| `cargo test -p stellar-expressions` | 종료 코드 0 (unit 129 + `evaluation_memory_bound` 3 + `evaluator_determinism` 3 + `evaluator_golden_corpus` 3 + `expression_fuzz` 3 + doctest 3 passed, 0 failed) |
| `STELLAR_EXPRESSION_FUZZ_ITERATIONS=30000 cargo test -p stellar-expressions --release --test expression_fuzz -- --nocapture` | 종료 코드 0, 3 passed(26.12 s) |
| `cargo test -p stellar-bindings` | 종료 코드 0 (unit 51 + `fallback_golden_corpus` 2 + `token_change_benchmark` 3 passed) — 하위 crate 영향 확인 |
| `cargo test -p stellar-document` | 종료 코드 0 (unit 316 + 그 밖 통합·doctest 11 passed) — expressions를 쓰는 하위 crate 영향 확인 |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 캠페인 | 입력 | 결과 |
|---|---|---|
| debug 기본(1,200회) | 12,000(생성 1,200: 타입 준수 1,018 / 구조 변형 3,600: 수용 265·거부 3,335 / bytes 변형 7,200: 수용 225·거부 6,975) | 발견 **0**. 컴파일 1,434·type 거부 256, 평가 성공 953·오류 481, budget 경계 검사 953건 통과, 무작위 한도 평가 8,604건(한도 오류 1,047·기본 결과 7,557) |
| release 확장(30,000회) | 300,000(생성 30,000: 타입 준수 25,501 / 구조 수용 6,264·거부 83,736 / bytes 수용 5,683·거부 174,317) | 발견 **0**. 컴파일 35,499·type 거부 6,448, 평가 성공 23,256·오류 12,243, budget 경계 검사 23,256건 통과, 무작위 한도 평가 212,994건(한도 오류 26,745), 입력당 최대 205 ms |

| stress(release 전체 규모) | 결과 |
|---|---|
| 왼쪽 중첩 60만 단계, 기본 한도 | `InstructionBudgetExceeded { budget: 1000000 }` (debug 축소: 6만 단계·budget 10만 → `budget: 100000`) |
| 300단계 공유 block 중첩 | `CallDepthExceeded { limit: 256 }` |
| 7만 단계 오른쪽 중첩 | `StackLimitExceeded { limit: 65536 }` |
| 공유 `concat` 24단계(64 B × 2^24) | `StringTooLong` |
| 부호 반전 15만 / `if` 10만 / `match` 6만 arm | 정상 값(명령 150,002 / 300,002 / 60,004), stack overflow 없음 |

## 차이(작업 지시와 다른 점)

- "fuzz target"을 cargo-fuzz/libFuzzer가 아닌 결정론적 seed `#[test]` harness로 구현했습니다(lane 지시, 새 의존성·
  nightly 없음). coverage-guided가 아닙니다.
- 검증 기준을 만족시키려고 fuzz가 찾은 결함을 VIII.2.2(node codec)·VIII.1.3(match type 규칙)·VIII.2.1(child 조회)
  코드에서 수정했습니다. 동작 변화는 중복 slot node 직렬화 거부뿐이며, 검증된 AST의 bytes·type 결과·평가 결과는 같습니다
  (VIII.4.6·VIII.9.3 golden 무변경 통과).
- 할당량 계측(XVI.6.1·XVI.6.2의 계측 할당자)은 넣지 않았습니다. 평가 메모리 상한은 VIII.4.5 `evaluation_memory_bound`
  테스트가 계측합니다.

## 남은 위험 / 한계

- coverage-guided가 아니므로 발견력이 libFuzzer보다 낮습니다. 장기 실행과 crash artifact 보관은 XVI.6.7(nightly) 범위입니다.
- AST 기준 evaluator의 한도 의미(처리 작업 수)는 bytecode와 달라 한도 오류끼리는 결과 동일성을 비교하지 않습니다.
- 60만 node 규모의 검증·cycle 검사는 debug에서 수십 초입니다(release 1 s 미만). 문서 열기 경로에서 이런 크기의 표현식을
  허용할지(node 수 상한)는 별도 정책 결정이 필요합니다.
- 시간 상한은 wall-clock 기준(생성 입력 30 s, stress 240 s)이며, 작업 중 기기 부하(load average 약 45)에서 측정한
  값입니다. watchdog가 hang을 실패로 보고해도 worker thread는 프로세스가 끝날 때까지 남습니다.
