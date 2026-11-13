# 작업 보고서: VI.10.5 random valid document generator와 1,000 seed property test

- WBS: VI.10.5
- 작업명: random valid document generator, 1,000 seed round-trip property test(CI smoke)
- 상태: 완료 (`[x]`) — 1,000 seed property test는 workspace 통합 검증(`cargo xtask check-core`의 `cargo test --workspace`)에 포함됩니다. 제품 테스트는 CI에 추가하지 않기로 했습니다(2026-11-14).
- 선행: VI.10.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/test_support/random_document.rs`(신규) — 자체 PRNG `SplitMix64`,
  `random_valid_document(seed)`, property test.
- `crates/document/src/test_support/mod.rs` — 모듈 등록.
- `docs/task-reports/VI.10.5.md`(신규), 계획서(VI.10.5 `[ ]`→`[x]`).

## 구현 요약

- **PRNG**: 외부 crate 없이 SplitMix64를 직접 구현했습니다(출력 golden 고정).
- **유효성을 구성으로 보장**: id는 문서별 단조 counter를 timestamp에 넣어 유일·비NIL, 부모는 먼저
  만든 Frame·Fragment 중 depth < 64, 형제 OrderKey는 정렬된 형제 key 사이 `between`으로 유일,
  modifier 참조는 먼저 만든 node만(참조 graph DAG) 또는 NIL·없는 id(미해결 진단), Fragment에는
  stack 없음, Frame clip은 Path node 또는 없는 id. float은 1/4·1/8 격자 값이라 정확히 표현됩니다.
- **분포**: 페이지 1~3·View 0~3, View당 node 0~24(1/16 확률로 depth 64 chain), 모든 node 종류·
  Shape 4종·modifier 12종(비활성·무작위 stack 위치), 다국어·emoji·제어 문자 이름/텍스트, 무작위
  runtime churn(dummy insert/remove)으로 핸들 배치도 흔듭니다.
- **property test**(`smoke_thousand_seed_round_trip_property`, seed 0..1000): 생성 문서 validator
  통과 → 같은 seed 재생성 레코드 동일(비결정성) → 저장 bytes(문서 정의 + View subtree payload,
  chunk serializer와 같은 codec·분할) 디코드·재결합·`from_records` 후 레코드와 semantic snapshot
  (VI.10.4) 동일(데이터 손실) → 재로드 문서의 저장 bytes byte-identical. 모든 단계가 panic 없이
  끝나야 통과합니다. 분포 회귀 방지로 종류 커버리지·depth 64 문서 수·미해결 참조 수도 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document random_document` | 3 passed, 0 failed(6.01s, debug) |
| `cargo test -p stellar-document` | 189 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 1,000 seed: panic 0, 데이터 손실 0(레코드·snapshot 동일), 비결정성 0(재생성·재인코딩 동일)(`smoke_thousand_seed_round_trip_property`) | 통과 |
| 커버리지: ComponentInstance 외 node 종류 전부, Shape 4종, modifier 12종, depth 64 문서 > 10, 미해결 참조 > 100, node > 10,000 | 통과 |
| 다른 seed → 다른 문서, PRNG 출력 golden(`different_seeds_produce_different_documents`, `prng_sequence_is_fixed`) | 통과 |

## 차이(작업 지시와 다른 점)

- **CI 등록**: property test는 `cargo test -p stellar-document`에 포함된 smoke 테스트(이름
  `smoke_` 접두, debug 약 6초)로 두었습니다. 현재 CI(`.github/workflows/bootstrap.yml`)는 xtask
  테스트만 실행하고 crate 테스트를 실행하지 않으며(III.5.1 보고서와 같은 상태), 이번 작업 범위는
  CI 파일 수정을 금지하므로 workflow에 job을 추가하지 않았습니다.
- round-trip은 document crate 범위(VI.10.5 허용 경로)에서 chunk serializer와 같은 codec·subtree
  분할 경로로 수행합니다. storage chunk 파일 왕복은 VI.10.3의 100개 golden 저장 테스트가 덮습니다.

## 남은 위험 / 한계

- CI가 crate 테스트를 실행하도록 workflow를 확장하는 작업이 별도로 필요합니다(이 smoke 포함).
- ComponentInstance는 payload 스키마(Phase IX) 전이라 생성 대상이 아닙니다.
- 상태 확정(2026-11-14): 제품 테스트는 CI 대신 로컬 통합 검증으로 실행하기로 해 행 문구를 맞추고 `[x]`로 확정했습니다.
