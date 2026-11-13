# 작업 보고서: XVI.6.5 Semantic Diff/apply/inverse fuzz target

- WBS: XVI.6.5
- 작업명: Semantic Diff/apply/inverse fuzz target 추가(valid A/B에서 apply(diff)==B)
- 상태: 완료 (`[x]`)
- 선행: XIV.3.10 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, `cargo test`).

## 변경 파일 목록

- `crates/semantic-diff/tests/semantic_diff_fuzz.rs`(신규) — 결정론적 fuzz harness, `#[test]` 4개.
- `docs/task-reports/XVI.6.5.md`(신규), 계획서(XVI.6.5 `[ ]`→`[x]`).

## 구현 요약

XVI.6.1·XVI.6.2와 같은 방식(고정 seed `#[test]`, 새 의존성·nightly 없음)입니다. 기본 반복 400,
`STELLAR_SEMANTIC_FUZZ_ITERATIONS`로 늘릴 수 있습니다. 모든 대상은 `catch_unwind`로 panic을
잡고, 실패 입력은 `CARGO_TARGET_TMPDIR/semantic-diff-fuzz/`에 hex로 저장한 뒤 실패합니다.

1. **유효 A/B**(`valid_pairs_apply_and_inverse_reproduce`): A = random valid document(VI.10.5),
   B = A에 visible·이름·Frame 크기·OrderKey·parent 이동·leaf 삭제·node 추가 편집을 무작위로
   쌓되 **문서 validator(VI.10.1)를 통과한 편집만** 남긴 문서, 그리고 무관한 valid document.
   `apply(diff(A,B), A) == B`, `apply(inverse(diff), B) == A`, diff·snapshot canonical bytes
   round-trip을 확인합니다.
2. **decoder**(`malformed_bytes_never_panic_and_accepted_inputs_are_canonical`): 유효 diff·snapshot
   bytes를 bit 뒤집기·절단·삽입·삭제·`u32::MAX` 덮어쓰기·경계 byte로 변형해 decode합니다. panic
   없이 Ok 또는 구조화 오류만 허용하고, 수용한 입력은 재인코딩하면 byte-identical이어야 합니다.
3. **stale·외부 적용**(`stale_and_foreign_diffs_never_panic`): diff를 무관한 snapshot, 이미 적용된
   target, 역방향 base에, 변형 후 수용된 diff를 원래·무관 snapshot에 적용 — panic 없이 Ok 또는
   구조화 오류(`ApplyError`)만 허용합니다.
4. **결정론**(`campaign_is_deterministic`): 같은 seed는 같은 diff bytes·변형 입력을 만듭니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 45 passed, fuzz 4 passed, 0 failed |
| `cargo test -p stellar-semantic-diff --test semantic_diff_fuzz -- --nocapture` (기본 400) | 4 passed: valid 쌍 800(비어 있지 않은 diff 746) 전부 재현, 변형 입력 3,200(수용 192·거부 3,008) panic 0·수용 입력 canonical, stale 적용 구조화 오류 1,170·panic 0 |
| `STELLAR_SEMANTIC_FUZZ_ITERATIONS=3000 cargo test -p stellar-semantic-diff --release --test semantic_diff_fuzz -- --nocapture` | 4 passed: valid 쌍 6,000(비어 있지 않은 diff 5,582) 전부 재현, 변형 입력 24,000(수용 1,261·거부 22,739) panic 0, stale 적용 구조화 오류 8,727·panic 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

- 발견된 결함: 없음(실패 입력 0).

## 차이(작업 지시와 다른 점)

- libFuzzer·cargo-fuzz 대신 결정론적 seed `#[test]` harness로 구현했습니다(lane 지시, 새 의존성
  없음). 장기 nightly 실행은 반복 수 환경 변수로 늘립니다.
- 할당량·hang 계측(XVI.6.2의 계측 할당자·watchdog)은 넣지 않았습니다. decoder는 개수 필드를
  남은 bytes로 상한 검사하고 재귀가 없어(XIV.3.8) 큰 입력에서도 선형입니다.

## 남은 위험 / 한계

- scene ComponentInstance·컴포넌트 정의·expression·variant 입력은 random document 생성기에
  없어 이 harness의 유효 A/B 범위 밖입니다(단위 테스트로만 검증). 생성기가 확장되면 추가합니다.
